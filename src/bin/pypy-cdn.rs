use clap::Parser;
use log::{info, warn};
use pypy_cdn::{AppOpts, AppServiceImpl};

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

  info!("pypy-cdn: starting daemon");
  info!("video path: {}", opts.video_path);

  let app = AppServiceImpl::new(opts.clone())
    .await
    .expect("Failed to initialize app service");

  let video_http = tokio::spawn(pypy_cdn::http::serve_video_http(app.clone()));
  let rtsp = tokio::spawn(pypy_cdn::rtsp::serve_rtsp_typewriter(app.clone()));
  let l3 = match &opts.builtin_l3_listen {
    Some(listen) => tokio::spawn(pypy_cdn::forward::serve_l3_forward(
      listen.clone(),
      opts.builtin_l3_forward,
    )),
    None => {
      info!("No L3 forwarding configured");
      tokio::task::spawn(async { Ok(()) })
    }
  };

  tokio::select! {
      e = l3, if opts.builtin_l3_listen.is_some() => {
          match e {
              Ok(Ok(_)) => info!("L3 Forward exited successfully"),
              Ok(Err(e)) => warn!("L3 Forward exited with error: {}", e),
              Err(e) => warn!("L3 Forward exited with error: {}", e),
          }
      },
      e = rtsp => {
          match e {
              Ok(Ok(_)) => info!("RTSP exited successfully"),
              Ok(Err(e)) => warn!("RTSP exited with error: {}", e),
              Err(e) => warn!("RTSP exited with error: {}", e),
          }
      },
      e = video_http => {
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
