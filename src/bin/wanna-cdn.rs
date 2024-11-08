use std::collections::HashMap;

use clap::Parser;
use log::{info, warn};
use wanna_cdn::{AppOpts, AppServiceImpl};

#[tokio::main]
async fn main() {
  match dotenvy::dotenv() {
    Err(e) => warn!("dotenv(): failed to load .env file: {}", e),
    _ => {}
  }

  env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
    .filter(Some("warp::server"), log::LevelFilter::Off)
    .init();

  let opts = AppOpts::parse();

  info!(
    "WannaDance: starting daemon, version {}",
    wanna_cdn::my_git_hash()
  );
  info!("video path: {}", opts.video_path_ud);

  let app = AppServiceImpl::new(opts.clone())
    .await
    .expect("Failed to initialize app service");

  let http = tokio::spawn(wanna_cdn::http::serve_video_http(app.clone()));
  let rtsp = match opts.rtsp_listen.is_some() {
    true => tokio::spawn(wanna_cdn::rtsp::serve_rtsp_typewriter(app.clone())),
    false => {
      info!("RTSP disabled");
      tokio::task::spawn(async { Ok(()) })
    }
  };
  let l4 = match (&opts.builtin_sni_listen, &opts.builtin_sni_proxy) {
    (Some(listen), Some(proxy)) if !proxy.is_empty() => {
      let mut proxy_targets = HashMap::new();
      for target_def in proxy {
        // api.udon.dance=ud-orig.kiva.moe:443
        let mut parts = target_def.splitn(2, '=');
        if let (Some(host), Some(forward_target)) = (parts.next(), parts.next()) {
          proxy_targets.insert(host.to_string(), forward_target.to_string());
        }
      }
      tokio::spawn(wanna_cdn::forward::serve_sni_proxy(
        listen.clone(),
        proxy_targets,
      ))
    }
    _ => {
      info!("No L4 forwarding configured");
      tokio::task::spawn(async { Ok(()) })
    }
  };

  tokio::select! {
      e = l4, if opts.builtin_sni_listen.is_some() => {
          match e {
              Ok(Ok(_)) => info!("L4 Forward exited successfully"),
              Ok(Err(e)) => warn!("L4 Forward exited with error: {}", e),
              Err(e) => warn!("L4 Forward exited with error: {}", e),
          }
      },
      e = rtsp, if opts.rtsp_listen.is_some() => {
          match e {
              Ok(Ok(_)) => info!("RTSP exited successfully"),
              Ok(Err(e)) => warn!("RTSP exited with error: {}", e),
              Err(e) => warn!("RTSP exited with error: {}", e),
          }
      },
      e = http => {
          match e {
              Ok(Ok(_)) => info!("Server exited successfully"),
              Ok(Err(e)) => warn!("Server exited with error: {}", e),
              Err(e) => warn!("Server exited with error: {}", e),
          }
      },
      _ = tokio::signal::ctrl_c() => {
          info!("Received Ctrl-C, shutting down...");
      }
  }

  info!("Goodbye!");
}
