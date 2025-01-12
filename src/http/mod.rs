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
  ffmpeg::{ffmpeg_audio_compensation, ffmpeg_copy},
  types::SongId,
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
          .serve_token(id, remote)
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
          CdnFetchResult::Hit(token) => {
            // Found in our CDN, let's redirect to the resource gateway.
            // Note: in prior versions, we used the format `{token}.mp4`,
            // which turned out it's not caching-friendly.
            format!("/v/{}.mp4?auth={}&t=aya", id, token)
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
        let token = match qs.get("auth") {
          Some(token) => Some(token.clone()),
          // allow empty token if no_auth is enabled
          None /*if app.opts.no_auth */ => None,
          // otherwise, reject
          // None => {
          //   warn!("Missing token, id={}, client={}", id, remote);
          //   return Err(warp::reject::custom(CustomRejection::BadToken));
          // }
        };
        let backing_cdn = match qs.get("t") {
          Some(t) if t == "wd" => &app.cdn,
          _ => &app.cdn,
        };
        let video_file = match backing_cdn
          .serve_file(Some(id), token, remote.clone())
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

        info!("[HIT] Cache {} found: serving {}", id, video_file);
        serve_video_mp4(app, id, range, video_file, None).await
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
    .and_then(
      |query: HashMap<String, String>, app: AppService, remote: Option<IpAddr>| async move {
        let id = query
          .get("id")
          .ok_or(warp::reject::custom(CustomRejection::BadVideoId))?
          .parse::<SongId>()
          .map_err(|_| warp::reject::custom(CustomRejection::BadVideoId))?;
        let remote = remote.ok_or(warp::reject::custom(CustomRejection::NoClientIP))?;
        let serve = app
          .cdn
          .serve_token(id, remote)
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
          CdnFetchResult::Hit(token) => {
            // Found in our CDN, let's redirect to the resource gateway.
            // Note: in prior versions, we used the format `{token}.mp4`,
            // which turned out it's not caching-friendly.
            format!("/v/{}.mp4?auth={}&t=wd", id, token)
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

  // https://api.udon.dance/Api/..
  let wanna_dance_other_api = warp::path!("Api" / ..)
    .and(warp::path::full())
    .and(warp::get())
    .and(with_service(&app))
    .and_then(|full: FullPath, _app: AppService| async move {
      let path = format!("{}", full.as_str());
      debug!("GET {}", path);
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
  NoServeToken,
  IndexNotReady,
  CacheDirNotAvailable,
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
    let md5 = match md5 {
      Some(m) => m,
      None => match app.cdn.get_video_file_path(id).await {
        (_, metadata_json, avail) if avail => std::fs::File::open(metadata_json)
          .map_err(|e| anyhow::anyhow!("Failed to open metadata: {:?}", e))
          .and_then(|f| {
            serde_json::from_reader::<_, aya_dance_types::Song>(f)
              .map_err(|e| anyhow::anyhow!("Failed to parse metadata: {:?}", e))
          })
          .and_then(|s| {
            s.checksum
              .ok_or_else(|| anyhow::anyhow!("No checksum in metadata"))
          })
          .unwrap_or_default(),
        _ => "".to_string(),
      },
    };
    let compensated = format!(
      "{}/{}-{}-audio-offset-{}.mp4",
      app.cdn.cache_path, id, md5, audio_offset
    );
    if !std::path::Path::new(compensated.as_str()).exists() {
      if let Err(e) = std::fs::create_dir_all(app.cdn.cache_path.as_str()) {
        warn!(
          "Failed to create cache directory, serving original video: {:?}",
          e
        );
        return crate::cdn::range::get_range(range, video_file.as_str(), "video/mp4").await;
      }

      let compensated_stage1 = format!(
        "{}/{}-{}-audio-offset-{}-nocopy.mp4",
        app.cdn.cache_path, id, md5, audio_offset
      );

      let start = std::time::Instant::now();
      let stats = match ffmpeg_audio_compensation(
        video_file.as_str(),
        compensated_stage1.as_str(),
        audio_offset,
      ) {
        Ok(stats) => stats,
        Err(e) => {
          warn!(
            "Failed to compensate audio for song {}, serving original video: {:?}",
            id, e
          );
          return crate::cdn::range::get_range(range, video_file.as_str(), "video/mp4").await;
        }
      };

      info!(
        "Compensate {} (ss+aac, {:.2}s, vcopy={:.3}s, adec={:.3}s, ares={:.3}s, aenc={:.3}s)",
        id,
        start.elapsed().as_secs_f64(),
        stats.video_copy_secs,
        stats.audio_decode_secs,
        stats.audio_resample_secs,
        stats.audio_encode_secs,
      );

      let start = std::time::Instant::now();
      if let Err(e) = ffmpeg_copy(compensated_stage1.as_str(), compensated.as_str()) {
        warn!(
          "Failed to copy compensated audio for song {} (file: {}), serving original video: {:?}",
          id, compensated_stage1, e
        );
        return crate::cdn::range::get_range(range, video_file.as_str(), "video/mp4").await;
      }

      info!(
        "Compensate {} (copy,   {:.2}s)",
        id,
        start.elapsed().as_secs_f64(),
      );

      if let Err(e) = std::fs::remove_file(compensated_stage1.as_str()) {
        warn!(
          "Failed to remove temporary file {}: {:?}",
          compensated_stage1, e
        );
      }
    }
    info!("Serving compensated {}: {}", id, compensated);
    return crate::cdn::range::get_range(range, compensated.as_str(), "video/mp4").await;
  }
  crate::cdn::range::get_range(range, video_file.as_str(), "video/mp4").await
}
