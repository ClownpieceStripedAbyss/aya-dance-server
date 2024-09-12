pub mod errors;

use std::future::Future;
use std::str::FromStr;
use async_stream::__private::AsyncStream;
use futures::{Stream, StreamExt};
use once_cell::sync::OnceCell;
use reqwest::redirect::Policy;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use warp::filters::path::FullPath;
use warp::hyper::body::Bytes;
use warp::hyper::Body;
use warp::Rejection;

pub static CLIENT: OnceCell<reqwest::Client> = OnceCell::new();

pub type Uri = FullPath;
pub type QueryParameters = Option<String>;
pub type Headers = warp::http::HeaderMap;

pub async fn proxy_and_inspecting(
  proxy_uri: String,
  method: reqwest::Method,
  headers: warp::http::HeaderMap,
  body: Bytes,
  host_override: Option<String>,
  dump_file: Option<String>,
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
  response_to_reply(response, dump_file)
    .await
    .map_err(warp::reject::custom)
}

/// Converts a reqwest response into a http::Response
async fn response_to_reply(
  response: reqwest::Response,
  dump_file: Option<String>,
) -> Result<warp::http::Response<Body>, errors::Error> {
  let mut builder = warp::http::Response::builder();
  for (k, v) in response.headers().iter() {
    let kk = k.to_string();
    let vv = v.to_str().map_err(|e| errors::Error::String(e.to_string()))?.to_string();
    builder = builder.header(kk, vv);
  }
  let status = response.status();
  let mut byte_stream = response.bytes_stream();
  let body = match dump_file {
    Some(dump_file) => {
      // create parent directories if not exist
      if let Some(parent) = std::path::Path::new(&dump_file).parent() {
        if let Err(e) = tokio::fs::create_dir_all(parent).await {
          log::warn!("Failed to create parent directories for cache file {}: {}", dump_file, e);
        }
      }
      // open file for dumping
      match tokio::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .open(dump_file.clone())
        .await {
        Ok(mut file) => Body::wrap_stream(async_stream::stream! {
          loop {
            tokio::select! {
              Some(x) = byte_stream.next() => {
                match &x {
                  Ok(x) => {
                    match file.write_all(&x).await {
                      Ok(_) => log::debug!("Wrote {} bytes to cache file {}", x.len(), dump_file),
                      Err(e) => log::warn!("Failed to write to cache file {}: {}", dump_file, e),
                    }
                  }
                  _ => {}
                }
                yield x;
              }
            }
          }
        }),
        Err(e) => {
          log::warn!("Failed to open file {} for caching: {}", dump_file, e);
          Body::wrap_stream(byte_stream)
        }
      }
    }
    _ => Body::wrap_stream(byte_stream),
  };
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
