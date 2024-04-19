extern crate core;

use std::sync::Arc;

use clap::Parser;
use log::info;

use crate::{
  cdn::{CdnService, CdnServiceImpl},
  redis::{RedisService, RedisServiceImpl},
};

pub mod cdn;
pub mod forward;
pub mod http;
pub mod redis;
pub mod rtsp;
pub mod types;

pub type Result<T> = anyhow::Result<T>;

#[derive(Debug, Parser, Clone)]
pub struct AppOpts {
  #[clap(short = 'r', long, env, default_value = "redis://127.0.0.1:6379")]
  pub redis_url: String,
  #[clap(short = 'x', long, env, default_value = "false")]
  pub no_auth: bool,
  #[clap(short = 'v', long, env, default_value = "./pypydance-song")]
  pub video_path: String,
  #[clap(short = 'l', long, env, default_value = "[::]:80")]
  pub listen: String,
  #[clap(short = '3', long, env)]
  pub builtin_l3_listen: Option<String>,
  #[clap(short = 'f', long, env, default_value = "jd-orig.kiva.moe:443")]
  pub builtin_l3_forward: String,
  #[clap(short = 'w', long, env, default_value = "[::]:7991")]
  pub rtsp_listen: String,
}

#[derive(Debug)]
pub struct AppServiceImpl {
  pub opts: AppOpts,
  pub redis: Option<RedisService>,
  pub cdn: CdnService,
}

pub type AppService = Arc<AppServiceImpl>;

impl AppServiceImpl {
  pub async fn new(opts: AppOpts) -> Result<AppService> {
    let redis = match opts.no_auth {
      true => {
        info!("Authentication disabled: skipping redis initialization");
        None
      }
      false => {
        info!("Authentication enabled: using {}", opts.redis_url);
        Some(RedisServiceImpl::new(opts.redis_url.clone()).await?)
      }
    };
    let cdn = CdnServiceImpl::new(opts.video_path.clone(), redis.clone());
    Ok(Arc::new(AppServiceImpl { opts, redis, cdn }))
  }
}
