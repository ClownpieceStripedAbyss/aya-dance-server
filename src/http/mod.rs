use std::{
  collections::HashMap,
  convert::Infallible,
  net::{IpAddr, SocketAddr},
};

use itertools::Either;
use log::{debug, info, trace, warn};
use serde_derive::Deserialize;
use serde_json::json;
use warp::{
  addr::remote, http::StatusCode, hyper, path::FullPath, reject::Reject, Filter, Rejection, Reply,
};
use warp_real_ip::get_forwarded_for;

use crate::{
  cdn::{
    proxy::{InspectingOpts, ProxyOpts},
    receipt::{RoomId, UserId},
    CdnFetchResult,
  },
  types::SongId,
  wanna::audio_compensator::CompensatorTask,
  AppService,
};

pub async fn serve_video_http(app: AppService) -> crate::Result<()> {
  let socket = app
    .opts
    .listen
    .parse::<SocketAddr>()
    .expect("Failed to parse listen address");

  let aya_root = warp::get()
    .and(warp::path!("aya-api" / String / "aya"))
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

  let aya_videos = warp::get()
    .and(warp::path!("api" / String / "videos" / String))
    .and(with_service(&app))
    .and(real_ip())
    .and_then(
      |_version: String, id_mp4: String, app: AppService, remote: Option<IpAddr>| async move {
        let remote = remote.ok_or(warp::reject::custom(CustomRejection::NoClientIP))?;
        let id = id_mp4
          .trim_end_matches(".mp4")
          .parse::<SongId>()
          .map_err(|_| warp::reject::custom(CustomRejection::BadVideoId))?;
        let serve = app
          .cdn
          .serve_token(id, remote, "".to_string())
          .await
          .map_err(|_| warp::reject::custom(CustomRejection::NoServeToken))?;
        let location = match serve {
          CdnFetchResult::Miss => {
            // Not found in our CDN, let's redirect to the original source.
            info!(
              "[MISS] Cache {} miss: redirect to https://api.udon.dance",
              id
            );
            format!("https://api.udon.dance/Api/Songs/play?id={}", id)
          }
          CdnFetchResult::Hit(token, checksum, _ts, _sign, _sign_ts) => {
            // Found in our CDN, let's redirect to the resource gateway.
            // Note: in prior versions, we used the format `{token}.mp4`,
            // which turned out it's not caching-friendly.
            format!(
              "/v/{}-{}.mp4?auth={}&t=aya&auth_key={}",
              id, checksum, token, token,
            )
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

  let aya_video_files = warp::get()
    .and(warp::path!("v" / String))
    .and(warp::path::end())
    .and(warp::query::<HashMap<String, String>>())
    .and(with_service(&app))
    .and(real_ip())
    .and(crate::cdn::range::filter_range())
    .and_then(
      |id_checksum_mp4: String,
       qs: HashMap<String, String>,
       app: AppService,
       remote: Option<IpAddr>,
       range: Option<String>| async move {
        let id_checksum = id_checksum_mp4
          .trim_end_matches(".mp4")
          .split('-')
          .collect::<Vec<&str>>();
        if id_checksum.len() != 2 {
          return Err(warp::reject::custom(CustomRejection::AreYouTryingToHackMe));
        }
        let id = id_checksum[0]
          .parse::<SongId>()
          .map_err(|_| warp::reject::custom(CustomRejection::BadVideoId))?;
        let checksum_requested = id_checksum[1].to_string();
        let remote = remote.ok_or(warp::reject::custom(CustomRejection::NoClientIP))?;
        let token = match qs.get("auth") {
          Some(token) => Some(token.clone()),
          // allow empty token if no_auth is enabled
          // None if app.opts.no_auth => None,
          // otherwise, reject
          None => {
            warn!("Missing token, id={}, client={}", id, remote);
            return Err(warp::reject::custom(CustomRejection::BadToken));
          }
        };
        let backing_cdn = match qs.get("t") {
          Some(t) if t == "wd" => &app.cdn,
          _ => &app.cdn,
        };
        let video = match backing_cdn
          .serve_file(id, token, checksum_requested, remote.clone())
          .await
        {
          Ok(Some(video_file)) => video_file,
          Ok(None) => {
            warn!(
              "Token passed but video not found, id={}, client={}",
              id, remote
            );
            return Err(warp::reject::custom(CustomRejection::AreYouTryingToHackMe));
          }
          Err(e) => {
            warn!("Bad token, id={}, client={}: {:?}", id, remote, e);
            return Err(warp::reject::custom(CustomRejection::BadToken));
          }
        };

        let video_file_path = video.video_file();
        info!("[HIT] Cache {} found: serving {}", id, video_file_path);
        serve_video_mp4(app, id, range, video_file_path, None).await
      },
    );
  //
  // let aya_song_index_get = warp::get()
  //   .and(warp::path!("aya-api" / String / "songs"))
  //   .and(with_service(&app))
  //   .and(real_ip())
  //   .and_then(
  //     |_version: String, app: AppService, remote: Option<IpAddr>| async move {
  //       let _ =
  // remote.ok_or(warp::reject::custom(CustomRejection::NoClientIP))?;       let
  // index = match app.index.get_index(false).await {         Ok(index) =>
  // index,         Err(e) => {
  //           warn!("Failed to get index: {:?}", e);
  //           return Err(warp::reject::custom(CustomRejection::IndexNotReady));
  //         }
  //       };
  //       Ok::<_, Rejection>(warp::reply::json(&index).into_response())
  //     },
  //   );
  //
  // let aya_song_index_clear = warp::delete()
  //   .and(warp::path!("aya-api" / String / "songs"))
  //   .and(with_service(&app))
  //   .and(real_ip())
  //   .and_then(
  //     |_version: String, app: AppService, remote: Option<IpAddr>| async move {
  //       let _ =
  // remote.ok_or(warp::reject::custom(CustomRejection::NoClientIP))?;       let
  // hosts = app         .opts
  //         .admin_src_host
  //         .as_ref()
  //         .ok_or(warp::reject::custom(CustomRejection::AreYouTryingToHackMe))?;
  //       for host in hosts {
  //         info!("admin src host: {}", host);
  //         // If the host is a valid IP, we will check the remote IP
  //         let ip = match host.parse::<IpAddr>() {
  //           Ok(ip) => Some(ip),
  //           // If it is a hostname? `resolve_host` needs a socket address, so
  // give it a port           Err(_) => {
  //             match
  // crate::forward::tokio_util::resolve_host(format!("{}:11451", host)).await {
  //               Ok(sock) => Some(sock.ip()),
  //               Err(e) => {
  //                 warn!(
  //                   "failed to resolve admin src host {}: {:?}, trying next
  // one",                   host, e
  //                 );
  //                 continue;
  //               }
  //             }
  //           }
  //         };
  //
  //         if ip == remote {
  //           info!(
  //             "remote IP matches admin src host: remote={:?}, admin={:?}",
  //             remote, ip
  //           );
  //           info!("admin says to rebuild the index, yes sir! admin={:?}", ip);
  //           match app.index.get_index(true).await {
  //             Ok(_) => info!("Index built"),
  //             Err(e) => warn!("Failed to clear index: {:?}", e),
  //           }
  //           return Ok::<_, Rejection>(
  //             warp::reply::json(&json!({"message": "ok"})).into_response(),
  //           );
  //         } else {
  //           warn!(
  //             "remote IP does not match admin src host: remote={:?},
  // admin={:?}",             remote, ip
  //           );
  //         }
  //       }
  //
  //       error!(
  //         "someone is trying to clear the index without permission!
  // remote={:?}",         remote
  //       );
  //       return
  // Err(warp::reject::custom(CustomRejection::AreYouTryingToHackMe));     },
  //   );
  //
  // let aya_song_index = aya_song_index_get.or(aya_song_index_clear);

  // Join them all!
  let aya = aya_root
    // .or(aya_song_index)
    .or(aya_videos)
    .or(aya_video_files);

  // http://api.udon.dance/Api/Songs/play?id=1021
  let wanna_dance_play = warp::path!("Api" / "Songs" / "play")
    .and(warp::query::<HashMap<String, String>>())
    .and(with_service(&app))
    .and(real_ip())
    .and(warp::header::headers_cloned())
    .and_then(
      |query: HashMap<String, String>,
       app: AppService,
       remote: Option<IpAddr>,
       headers: warp::http::HeaderMap| async move {
        let id = query
          .get("id")
          .ok_or(warp::reject::custom(CustomRejection::BadVideoId))?
          .parse::<SongId>()
          .map_err(|_| warp::reject::custom(CustomRejection::BadVideoId))?;
        let remote = remote.ok_or(warp::reject::custom(CustomRejection::NoClientIP))?;
        let user_agent = headers
          .get(warp::http::header::USER_AGENT)
          .ok_or(warp::reject::custom(CustomRejection::NoUserAgent))?
          .to_str()
          .map_err(|_| warp::reject::custom(CustomRejection::NoUserAgent))?;
        let serve = app
          .cdn
          .serve_token(id, remote, user_agent.to_string())
          .await
          .map_err(|_| warp::reject::custom(CustomRejection::NoServeToken))?;
        let location = match serve {
          CdnFetchResult::Miss => {
            info!(
              "[MISS] Cache {} miss: redirect to https://api.udon.dance",
              id
            );
            // Not found in our CDN, let's redirect to api.udon.dance
            format!("https://api.udon.dance/Api/Songs/play?id={}", id)
          }
          CdnFetchResult::Hit(token, checksum, _ts, _sign, _sign_ts) => {
            // Found in our CDN, let's redirect to the resource gateway.
            // Note: in prior versions, we used the format `{token}.mp4`,
            // which turned out it's not caching-friendly.
            format!(
              "/v/{}-{}.mp4?auth={}&t=wd&auth_key={}",
              id, checksum, token, token,
            )
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

  // http://api.udon.dance/Api/Songs/list
  let wanna_dance_song_list = warp::path!("Api" / "Songs" / "list")
    .and(with_service(&app))
    .and(warp::header::headers_cloned())
    .and(warp::body::bytes())
    .and_then(
      |app: AppService, _headers: warp::http::HeaderMap, body: bytes::Bytes| async move {
        info!("[HIT] /Api/Songs/list");
        let response = crate::cdn::proxy::proxy_and_inspecting(
          format!("http://{}/Api/Songs/list", app.opts.api_upstream_ud),
          reqwest::Method::GET,
          {
            let mut hdr = warp::http::HeaderMap::new();
            hdr.insert(
              warp::http::header::USER_AGENT,
              warp::http::header::HeaderValue::from_str("curl/8.7.1").unwrap(),
            );
            hdr.insert(
              warp::http::header::ACCEPT,
              warp::http::header::HeaderValue::from_str("*/*").unwrap(),
            );
            hdr.insert(
              warp::http::header::HOST,
              warp::http::header::HeaderValue::from_str("api.udon.dance").unwrap(),
            );
            hdr
          },
          body,
          ProxyOpts {
            host_override: Some("api.udon.dance".to_string()),
            user_agent_override: Some(format!(
              "WannaDanceSelfHostedCDN/{}.{}",
              crate::MY_VERSION_ID,
              crate::my_git_hash(),
            )),
            allow_304: app.opts.proxy_allow_304,
          },
          None,
        )
        .await?;

        let mut builder = warp::http::Response::builder();
        builder = builder.status(response.status());
        for (k, v) in response.headers().iter() {
          let kk = k.to_string();
          let vv = v
            .to_str()
            .map_err(|e| crate::cdn::proxy::errors::Error::String(e.to_string()))?
            .to_string();
          builder = builder.header(kk, vv);
        }
        let body_bytes = warp::hyper::body::to_bytes(response.into_body())
          .await
          .map_err(|e| crate::cdn::proxy::errors::Error::String(e.to_string()))?;
        let body_string = String::from_utf8(body_bytes.to_vec())
          .map_err(|e| crate::cdn::proxy::errors::Error::String(e.to_string()))?;
        let mut body_json = serde_json::from_str::<serde_json::Value>(&body_string)
          .map_err(|e| crate::cdn::proxy::errors::Error::String(e.to_string()))?;
        // Now inserts a dedicated field for the CDN
        if let Some(map) = body_json.as_object_mut() {
          map.insert(
            "WDSelfHostedCDN".to_string(),
            json!({
              "version": crate::MY_VERSION_ID,
              "git_hash": crate::my_git_hash(),
            }),
          );
          // Scan override dir for <id>.mp4
          let override_dir = app.opts.video_override_path_ud.clone();
          let mut override_ids = vec![];
          if let Ok(entries) = std::fs::read_dir(override_dir) {
            for entry in entries {
              if let Ok(entry) = entry {
                if let Ok(file_name) = entry.file_name().into_string() {
                  if let Ok(id) = file_name.trim_end_matches(".mp4").parse::<SongId>() {
                    override_ids.push(id);
                  }
                }
              }
            }
          }
          map.insert(
            "WDSelfHostedCDNOverride".to_string(),
            serde_json::to_value(&override_ids).unwrap(),
          );
        }
        Ok::<_, Rejection>(
          builder.body(
            serde_json::to_string_pretty(&body_json)
              .map_err(|e| crate::cdn::proxy::errors::Error::String(e.to_string()))?,
          ),
        )
      },
    );

  // https://api.udon.dance/Api/..
  let wanna_dance_other_api = warp::path!("Api" / ..)
    .and(warp::path::full())
    .and(warp::get())
    .and(with_service(&app))
    .and_then(|full: FullPath, _app: AppService| async move {
      let path = format!("{}", full.as_str());
      debug!("[Catch] GET {}", path);
      // Redirect to https://api.udon.dance
      let location = format!("https://api.udon.dance{}", path);
      Ok::<_, Rejection>(
        warp::http::Response::builder()
          .status(StatusCode::FOUND)
          .header(warp::http::header::LOCATION, location.clone())
          .body(location),
      )
    });

  // https://play.udon.dance/files/2403/1-660524b46664a.mp4?e=b03f9584f49350599d6d641d74b0b547&s=13959733
  let wanna_dance_play_cache = warp::path!("files" / String / String)
    .and(warp::path::end())
    .and(warp::query::<HashMap<String, String>>())
    .and(with_service(&app))
    .and(real_ip())
    .and(remote())
    .and(crate::cdn::range::filter_range())
    .and(warp::header::headers_cloned())
    .and(warp::body::bytes())
    .and_then(
      |date: String,
       file: String,
       query: HashMap<String, String>,
       app: AppService,
       real_ip: Option<IpAddr>,
       remote: Option<SocketAddr>,
       range: Option<String>,
       headers: warp::http::HeaderMap,
       body: bytes::Bytes| async move {
        let _real_ip = real_ip.ok_or(warp::reject::custom(CustomRejection::NoClientIP))?;
        let remote = remote.ok_or(warp::reject::custom(CustomRejection::AreYouTryingToHackMe))?;
        let id = file
          .split('-')
          .next()
          .ok_or_else(|| warp::reject::custom(CustomRejection::BadVideoId))?
          .parse::<SongId>()
          .map_err(|_| warp::reject::custom(CustomRejection::BadVideoId))?;
        let e = query
          .get("e")
          .ok_or_else(|| warp::reject::custom(CustomRejection::BadToken))?;
        let s = query
          .get("s")
          .ok_or_else(|| warp::reject::custom(CustomRejection::BadToken))?
          .parse::<u64>()
          .map_err(|_| warp::reject::custom(CustomRejection::BadToken))?;

        let (download_tmp, cache_file, metadata_json, available) = app
          .cdn
          .serve_local_cache(id, file.clone(), e.clone(), s, remote)
          .await;
        match available {
          true => {
            info!("[HIT] Cache {} found: serving {}", id, cache_file);
            serve_video_mp4(app, id, range, cache_file, Some(e.clone())).await
          }
          _ => {
            let (upstream_dns, host_override) = match headers
              .get(warp::http::header::HOST)
              .map(|x| x.to_str().ok())
              .flatten()
            {
              Some("nya.xin.moe") => (
                &app.opts.cache_upstream_ud_domestic,
                "nya.xin.moe".to_string(),
              ),
              _ => (
                &app.opts.cache_upstream_ud_oversea,
                "play.udon.dance".to_string(),
              ),
            };
            info!(
              "[MISS] Cache {} miss ({}): fetch from {} (DNS: {})",
              id, cache_file, host_override, upstream_dns,
            );
            crate::cdn::proxy::proxy_and_inspecting(
              format!(
                "http://{}/files/{}/{}?e={}&s={}",
                upstream_dns, date, file, e, s
              ),
              reqwest::Method::GET,
              headers,
              body,
              ProxyOpts {
                host_override: Some(host_override),
                user_agent_override: Some(format!(
                  "WannaDanceSelfHostedCDN/{}.{}",
                  crate::MY_VERSION_ID,
                  crate::my_git_hash(),
                )),
                allow_304: app.opts.proxy_allow_304,
              },
              Some(InspectingOpts {
                id,
                download_tmp,
                cache_file,
                metadata_json,
                etag: e.clone(),
                expected_size: s,
              }),
            )
            .await
          }
        }
      },
    );

  let wanna_dance = wanna_dance_play
    .or(wanna_dance_play_cache)
    .or(wanna_dance_song_list)
    .or(wanna_dance_other_api);

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

  // Remote receipt gateway
  let receipt_get = warp::get()
    .and(warp::path!("r" / RoomId))
    .and(with_service(&app))
    .and_then(|room_id: RoomId, app: AppService| async move {
      let receipts = app.receipt.receipts(room_id).await;
      Ok::<_, Rejection>(warp::reply::json(&receipts).into_response())
    });

  #[derive(Debug, Clone, Deserialize)]
  struct ReceiptCreate {
    target: UserId,
    id: Option<SongId>,
    url: Option<String>,
    sender: Option<UserId>,
    message: Option<String>,
  }

  let receipt_post = warp::post()
    .and(warp::path!("r" / RoomId))
    .and(warp::body::json())
    .and(with_service(&app))
    .and_then(
      |room_id: RoomId, create: ReceiptCreate, app: AppService| async move {
        debug!("create receipt: {:?}", &create);
        let song = match (create.id, create.url) {
          (Some(song_id), _) => Either::Left(song_id),
          (_, Some(song_url)) => Either::Right(song_url.trim().to_string()),
          _ => {
            return Ok(
              warp::reply::json(&json!({
                "message": "missing song id or url",
                "receipt": null,
              }))
              .into_response(),
            )
          }
        };
        let receipt = match app
          .receipt
          .create_receipt(
            room_id,
            create.target.trim().to_string(),
            song,
            create.sender.map(|s| s.trim().to_string()),
            create.message,
          )
          .await
        {
          Ok(receipt) => receipt,
          Err(e) => {
            let format = format!("create receipt failed: {:?}", e);
            return Ok(
              warp::reply::json(&json!({
                "message": format,
                "receipt": null,
              }))
              .into_response(),
            );
          }
        };
        Ok::<_, Infallible>(
          warp::reply::json(&json!({
            "message": "ok",
            "receipt": receipt,
          }))
          .into_response(),
        )
      },
    );

  let receipt = receipt_get.or(receipt_post);

  // Ok, let's run the server
  let routes = aya
    .or(wanna_dance)
    .or(typewriter)
    .or(receipt)
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
  NoUserAgent,
  NoServeToken,
  IndexNotReady,
  CacheDirNotAvailable,
  VideoExpired,
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
      "Cache-Control",
      "Authorization",
      "Access-Control-Request-Method",
      "Access-Control-Request-Headers",
    ])
    .allow_methods(vec!["GET", "POST", "OPTIONS", "PUT", "DELETE"])
}

pub fn real_ip() -> impl Filter<Extract = (Option<IpAddr>,), Error = Infallible> + Clone {
  remote().and(get_forwarded_for()).map(
    move |addr: Option<SocketAddr>, forwarded_for: Vec<IpAddr>| {
      addr.map(|addr| forwarded_for.first().copied().unwrap_or(addr.ip()))
    },
  )
}

pub async fn serve_video_mp4(
  app: AppService,
  id: SongId,
  range: Option<String>,
  video_file: String,
  md5: Option<String>,
) -> Result<warp::http::Response<hyper::body::Body>, Rejection> {
  let audio_offset = app.opts.audio_compensation;
  if (audio_offset - 0.0).abs() > f64::EPSILON {
    return match crate::wanna::audio_compensator::submit_new_compensator_task(
      app.clone(),
      CompensatorTask {
        song_id: id,
        song_md5: md5,
        input_video_path: video_file.clone(),
        audio_offset,
      },
    )
    .await
    {
      Err(e) => {
        warn!("Failed to compensate {}, serving original video: {}", id, e);
        crate::cdn::range::get_range(range, video_file.as_str(), "video/mp4").await
      }
      Ok(compensated) => {
        info!("Serving compensated {}: {}", id, compensated);
        crate::cdn::range::get_range(range, compensated.as_str(), "video/mp4").await
      }
    };
  }
  crate::cdn::range::get_range(range, video_file.as_str(), "video/mp4").await
}
