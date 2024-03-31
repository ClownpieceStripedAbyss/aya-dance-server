use std::sync::Arc;

use clap::Parser;

use crate::{
    cdn::{CdnService, CdnServiceImpl},
    redis::{RedisService, RedisServiceImpl},
};

pub mod cdn;
pub mod redis;
pub mod types;

pub type Result<T> = anyhow::Result<T>;

#[derive(Debug, Parser, Clone)]
pub struct AppOpts {
    #[clap(short = 'r', long, env, default_value = "redis://127.0.0.1:6379")]
    pub redis_url: String,
    #[clap(short = 'v', long, env, default_value = "./pypydance-song")]
    pub video_path: String,
    #[clap(short, long, env, default_value = "[::]:80")]
    pub listen: String,
}

#[derive(Debug)]
pub struct AppServiceImpl {
    pub opts: AppOpts,
    pub redis: RedisService,
    pub cdn: CdnService,
}

pub type AppService = Arc<AppServiceImpl>;

impl AppServiceImpl {
    pub async fn new(opts: AppOpts) -> Result<AppService> {
        let redis = RedisServiceImpl::new(opts.redis_url.clone()).await?;
        let cdn = CdnServiceImpl::new(opts.video_path.clone(), redis.clone());
        Ok(Arc::new(AppServiceImpl { opts, redis, cdn }))
    }
}
