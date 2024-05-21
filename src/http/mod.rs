use std::{
  collections::HashMap,
  convert::Infallible,
  net::{IpAddr, SocketAddr},
};

use hyper::{
  client::{connect::dns::GaiResolver, HttpConnector},
  Body, Client, StatusCode,
};
use hyper_tls::HttpsConnector;
use log::{debug, info, trace};
use warp::{addr::remote, path::FullPath, reject::Reject, Filter, Rejection, Reply};
use warp_real_ip::get_forwarded_for;

use crate::{cdn::CdnFetchResult, types::SongId, AppService};

pub type HttpClient = Client<HttpsConnector<HttpConnector<GaiResolver>>, Body>;

pub async fn body_to_bytes(body: Body) -> anyhow::Result<Vec<u8>> {
  let body = hyper::body::to_bytes(body).await?;
  Ok(body.into())
}

pub fn http_client() -> HttpClient {
  Client::builder().build::<_, Body>(hyper_tls::HttpsConnector::new())
}

pub async fn serve_video_http(app: AppService) -> crate::Result<()> {
  let socket = app
    .opts
    .listen
    .parse::<SocketAddr>()
    .expect("Failed to parse listen address");

  let aya_root = warp::get()
    .and(warp::path!("api" / String / "aya"))
    .and(with_service(&app))
    .and(real_ip())
    .and_then(
      |version: String, _app: AppService, remote: Option<IpAddr>| async move {
        Ok::<_, Rejection>(
          warp::http::Response::builder()
            .status(StatusCode::OK)
            .body(format!(
              "Hello {:?}, this is Aya Dance server {}!",
              remote, version
            )),
        )
      },
    );

  // Video gateway: https://base-url/api/v1/videos/{id}.mp4
  let aya_videos = warp::get()
    .and(warp::path!("api" / String / "videos" / String))
    .and(with_service(&app))
    .and(real_ip())
    .and_then(
      |version: String, id_mp4: String, app: AppService, remote: Option<IpAddr>| async move {
        let remote = remote.ok_or(warp::reject::custom(CustomRejection::NoClientIP))?;
        let id = id_mp4
          .trim_end_matches(".mp4")
          .parse::<SongId>()
          .map_err(|_| warp::reject::custom(CustomRejection::BadVideoId))?;
        let serve = app
          .cdn
          .serve_token(id, remote)
          .await
          .map_err(|_| warp::reject::custom(CustomRejection::NoServeToken))?;
        let location = match serve {
          CdnFetchResult::Miss => {
            // Not found in our CDN, let's redirect to jd.pypy.moe
            format!("https://jd.pypy.moe/api/{}/videos/{}.mp4", version, id)
          }
          CdnFetchResult::Hit(token) => {
            // Found in our CDN, let's redirect to the resource gateway.
            // Note: in prior versions, we used the format `{token}.mp4`,
            // which turned out it's not caching-friendly.
            format!("/v/{}.mp4?auth={}", id, token)
          }
        };
        Ok::<_, Rejection>(
          warp::http::Response::builder()
            .status(StatusCode::FOUND)
            .header(warp::http::header::LOCATION, location.clone())
            .body(location),
        )
      },
    );

  // Video files gateway: https://base-url/resources/{id}.mp4?auth=<token>
  let aya_video_files = warp::get()
    .and(warp::path!("v" / String))
    .and(warp::path::end())
    .and(warp::query::<HashMap<String, String>>())
    .and(with_service(&app))
    .and(real_ip())
    .and(warp_range::filter_range())
    .and_then(
      |id_mp4: String,
       qs: HashMap<String, String>,
       app: AppService,
       remote: Option<IpAddr>,
       range: Option<String>| async move {
        let id = id_mp4
          .trim_end_matches(".mp4")
          .parse::<SongId>()
          .map_err(|_| warp::reject::custom(CustomRejection::BadVideoId))?;
        let remote = remote.ok_or(warp::reject::custom(CustomRejection::NoClientIP))?;
        let token = qs
          .get("auth")
          .ok_or(warp::reject::custom(CustomRejection::BadToken))?;
        let video_file = app
          .cdn
          .serve_file(Some(id), token.clone(), remote.clone())
          .await
          .map_err(|_| warp::reject::custom(CustomRejection::BadToken))?
          // There shouldn't be a token if the file is not found, which is
          // guaranteed by the gateway.
          .ok_or(warp::reject::custom(CustomRejection::AreYouTryingToHackMe))?;

        warp_range::get_range(range, video_file.as_str(), "video/mp4").await
      },
    );

  // Song metadata index: https://base-url/api/v1/aya/songs
  let aya_song_index = warp::get()
    .and(warp::path!("api" / String / "aya" / "songs"))
    .and(with_service(&app))
    .and(real_ip())
    .and_then(
      |_version: String, _app: AppService, _remote: Option<IpAddr>| async move {
        Ok::<_, Rejection>(
          warp::http::Response::builder()
            .status(StatusCode::OK)
            .body("TODO: return a json!"),
        )
      },
    );

  // Join them all!
  let aya = aya_root
    .or(aya_song_index)
    .or(aya_videos)
    .or(aya_video_files);

  let pypy = warp::path!("api" / ..)
    .and(warp::path::full())
    .and(warp::get())
    .and(with_service(&app))
    .and_then(|full: FullPath, _app: AppService| async move {
      let path = format!("{}", full.as_str());
      debug!("GET {}", path);
      // Redirect to https://jd.pypy.moe
      let location = format!("https://jd.pypy.moe{}", path);
      Ok::<_, Rejection>(
        warp::http::Response::builder()
          .status(StatusCode::FOUND)
          .header(warp::http::header::LOCATION, location.clone())
          .body(location),
      )
    });

  // Typewriter gateway
  let typewriter = warp::get()
    .and(warp::path!("typewriter" / String))
    .and(with_service(&app))
    .and(real_ip())
    .and_then(
      |token: String, app: AppService, client: Option<IpAddr>| async move {
        let client = client.ok_or(warp::reject::custom(CustomRejection::NoClientIP))?;
        let bv = app
          .typewriter
          .read(client, token)
          .await
          .map_err(|_| warp::reject::custom(CustomRejection::BadToken))?;
        info!("Typewriter submit {} -> [{}]", client, bv);
        let location = format!("https://api.xin.moe/ov/{}", bv);
        Ok::<_, Rejection>(
          warp::http::Response::builder()
            .status(StatusCode::FOUND)
            .header(warp::http::header::LOCATION, location.clone())
            .body(location),
        )
      },
    );

  // Ok, let's run the server
  let routes = aya
    .or(pypy)
    .or(typewriter)
    .with(cors())
    .recover(handle_rejection);

  info!("Listening on http://{}", socket);
  info!("Have a good day!");
  warp::serve(routes).run(socket).await;

  Ok(())
}

#[derive(Debug)]
pub enum CustomRejection {
  BadVideoId,
  BadToken,
  AreYouTryingToHackMe,
  NoClientIP,
  NoServeToken,
}

impl Reject for CustomRejection {}

async fn handle_rejection(e: Rejection) -> Result<impl Reply, Infallible> {
  trace!("handle_rejection: {:?}", &e);
  Ok(warp::reply::with_status(
    format!("Oops! {:?}", e),
    StatusCode::BAD_REQUEST,
  ))
}

pub fn with_service(
  service: &AppService,
) -> impl Filter<Extract = (AppService,), Error = Infallible> + Clone {
  let service = service.clone();
  warp::any().map(move || service.clone())
}

pub fn cors() -> warp::cors::Builder {
  warp::cors()
    .allow_any_origin()
    .allow_headers(vec![
      "Content-Type",
      "User-Agent",
      "Sec-Fetch-Mode",
      "Referer",
      "Origin",
      "Authorization",
      "Access-Control-Request-Method",
      "Access-Control-Request-Headers",
    ])
    .allow_methods(vec!["GET"])
}

pub fn real_ip() -> impl Filter<Extract = (Option<IpAddr>,), Error = Infallible> + Clone {
  remote().and(get_forwarded_for()).map(
    move |addr: Option<SocketAddr>, forwarded_for: Vec<IpAddr>| {
      addr.map(|addr| forwarded_for.first().copied().unwrap_or(addr.ip()))
    },
  )
}
