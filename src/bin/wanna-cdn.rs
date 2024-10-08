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

  info!("WannaDance: starting daemon, version {}", wanna_cdn::my_git_hash());
  info!("video path: {}", opts.video_path);
  info!("video path: {}", opts.video_path_ud);

  let app = AppServiceImpl::new(opts.clone())
    .await
    .expect("Failed to initialize app service");

  let http = tokio::spawn(wanna_cdn::http::serve_video_http(app.clone()));
  let rtsp = match opts.rtsp_enable {
    true => tokio::spawn(wanna_cdn::rtsp::serve_rtsp_typewriter(app.clone())),
    false => {
      info!("RTSP disabled");
      tokio::task::spawn(async { Ok(()) })
    }
  };
  let l4 = match &opts.builtin_l3_listen {
    Some(listen) => tokio::spawn(wanna_cdn::forward::serve_l4_forward(
      listen.clone(),
      opts.builtin_l3_forward,
      opts.builtin_l3_forward_ud,
    )),
    None => {
      info!("No L4 forwarding configured");
      tokio::task::spawn(async { Ok(()) })
    }
  };

  tokio::select! {
      e = l4, if opts.builtin_l3_listen.is_some() => {
          match e {
              Ok(Ok(_)) => info!("L4 Forward exited successfully"),
              Ok(Err(e)) => warn!("L4 Forward exited with error: {}", e),
              Err(e) => warn!("L4 Forward exited with error: {}", e),
          }
      },
      e = rtsp, if opts.rtsp_enable => {
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
