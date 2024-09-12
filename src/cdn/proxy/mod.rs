pub mod errors;

use std::str::FromStr;
use futures::StreamExt;
use once_cell::sync::OnceCell;
use reqwest::redirect::Policy;
use warp::filters::path::FullPath;
use warp::hyper::body::Bytes;
use warp::hyper::Body;
use warp::Rejection;

pub static CLIENT: OnceCell<reqwest::Client> = OnceCell::new();

pub type Uri = FullPath;
pub type QueryParameters = Option<String>;
pub type Headers = warp::http::HeaderMap;

pub async fn proxy_to_and_forward_response(
  proxy_uri: String,
  method: reqwest::Method,
  headers: warp::http::HeaderMap,
  body: Bytes,
  host_override: Option<String>,
) -> Result<warp::http::Response<Body>, Rejection> {
  let mut hdr = reqwest::header::HeaderMap::new();
  for (k, v) in headers.iter() {
    let ks = k.as_str();
    match ks.to_lowercase().as_str() {
      "host" if host_override.is_some() => {
        hdr.insert(
          reqwest::header::HOST,
          reqwest::header::HeaderValue::from_str(host_override.as_ref().unwrap()).unwrap(),
        );
      }
      _ => {
        hdr.insert(
          reqwest::header::HeaderName::from_str(ks).unwrap(),
          reqwest::header::HeaderValue::from_str(v.to_str().unwrap()).unwrap(),
        );
      }
    }
  }
  let request = CLIENT
    .get_or_init(default_reqwest_client)
    .request(method, proxy_uri)
    .headers(hdr)
    .body(body)
    .build()
    .map_err(errors::Error::Request)
    .map_err(warp::reject::custom)?;
  let response = CLIENT
    .get_or_init(default_reqwest_client)
    .execute(request)
    .await
    .map_err(errors::Error::Request).map_err(warp::reject::custom)?;
  response_to_reply(response)
    .await
    .map_err(warp::reject::custom)
}

/// Converts a reqwest response into a http::Response
async fn response_to_reply(
  response: reqwest::Response,
) -> Result<warp::http::Response<Body>, errors::Error> {
  let mut builder = warp::http::Response::builder();
  for (k, v) in response.headers().iter() {
    let kk = k.to_string();
    let vv = v.to_str().map_err(|e| errors::Error::String(e.to_string()))?.to_string();
    builder = builder.header(kk, vv);
  }
  let status = response.status();
  let mut byte_stream = response.bytes_stream();
  let inspecting = async_stream::stream! {
    loop {
       tokio::select! {
          Some(x) = byte_stream.next() => {
             yield x;
          }
       }
    }
  };
  let body = Body::wrap_stream(inspecting);
  builder
    .status(status.as_u16())
    .body(body)
    .map_err(errors::Error::Http)
}

fn default_reqwest_client() -> reqwest::Client {
  reqwest::Client::builder()
    .redirect(Policy::none())
    .build()
    // we should panic here, it is enforce that the client is needed, and there is no error
    // handling possible on function call, better to stop execution.
    .expect("Default reqwest client couldn't build")
}
