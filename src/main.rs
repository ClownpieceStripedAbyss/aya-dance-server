use log::{info, warn};

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    match dotenvy::dotenv() {
        Err(e) => warn!("dotenv(): failed to load .env file: {}", e),
        _ => {}
    }

    info!("Hello, world!");
}
