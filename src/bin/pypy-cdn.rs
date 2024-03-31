use std::{convert::Infallible, net::SocketAddr};

use clap::Parser;
use log::{info, warn};
use pypy_cdn::{AppOpts, AppService, AppServiceImpl, Result};
use warp::{http::StatusCode, reject::Reject, Filter, Rejection, Reply};

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    match dotenvy::dotenv() {
        Err(e) => warn!("dotenv(): failed to load .env file: {}", e),
        _ => {}
    }

    let opts = AppOpts::parse();
    let app = AppServiceImpl::new(opts)
        .await
        .expect("Failed to initialize app service");
    let server = tokio::spawn(server(app));

    tokio::select! {
        e = server => {
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

async fn server(app: AppService) -> Result<()> {
    let socket = app
        .opts
        .listen
        .parse::<SocketAddr>()
        .expect("Failed to parse listen address");

    // API Gateway: https://base-url/api/v1/videos/227.mp4
    // We need: https://base-url/api/{version}/videos/{id}.mp4

    let gateway = warp::get()
        .and(warp::path!("api" / String / "videos" / String))
        .and(with_service(&app))
        .and(warp::addr::remote())
        .and_then(
            |_version: String, id_mp4: String, app: AppService, _remote: Option<SocketAddr>| async move {
                let id = id_mp4.trim_end_matches(".mp4").parse::<u32>()
                    .map_err(|_| warp::reject::custom(CustomRejection::BadVideoId))?;
                let token = "114514".to_string();
                let location =
                    format!("{}/resources/{}.mp4?token={}", app.opts.base_url, id, token);
                Ok::<_, Rejection>(
                    warp::http::Response::builder()
                        .status(StatusCode::FOUND)
                        .header(warp::http::header::LOCATION, location.clone())
                        .body(location),
                )
            },
        );

    // Resource Gateway: https://base-url/resources/227.mp4?token=xxx
    // We need: https://base-url/resources/{id}.mp4?token=<token>
    let video = warp::get()
        .and(warp::path!("resources" / String))
        .and(warp::query::<String>())
        .and(with_service(&app))
        .and(warp::addr::remote())
        .and_then(
            |id_mp4: String, token: String, app: AppService, remote: Option<SocketAddr>| async move {
                let id = id_mp4.trim_end_matches(".mp4").parse::<u32>()
                    .map_err(|_| warp::reject::custom(CustomRejection::BadVideoId))?;
                let res = format!("{}:{}:{:?}", id, token, remote);
                Ok::<_, Rejection>(
                    warp::http::Response::builder()
                        .status(StatusCode::OK)
                        .body(res),
                )
            },
        );

    // Ok, let's run the server
    let routes = gateway.or(video).with(cors()).recover(handle_rejection);

    info!("Listening on http://{}", socket);
    warp::serve(routes).run(socket).await;

    Ok(())
}

#[derive(Debug)]
pub enum CustomRejection {
    BadVideoId,
    BadRequest,
}

impl Reject for CustomRejection {}

async fn handle_rejection(e: Rejection) -> std::result::Result<impl Reply, Infallible> {
    warn!("handle_rejection: {:?}", &e);
    Ok(warp::reply::with_status(
        format!("Oops! {:?}", e),
        StatusCode::BAD_REQUEST,
    ))
}

pub fn with_service(
    service: &AppService,
) -> impl Filter<Extract=(AppService, ), Error=Infallible> + Clone {
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
