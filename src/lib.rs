extern crate core;

use std::{sync::Arc, time::Duration};

use clap::Parser;
use log::info;

use crate::{
  cdn::{
    receipt::{ReceiptService, ReceiptServiceImpl},
    CdnService, CdnServiceImpl,
  },
  index::{IndexService, IndexServiceImpl},
  redis::{RedisService, RedisServiceImpl},
  rtsp::{TypewriterService, TypewriterServiceImpl},
};

pub mod cdn;
pub mod forward;
pub mod http;
pub mod index;
pub mod redis;
pub mod rtsp;
pub mod types;

pub type Result<T> = anyhow::Result<T>;

pub const MY_VERSION_ID: u32 = 1;

#[derive(Debug, Parser, Clone)]
pub struct AppOpts {
  #[clap(short = 'r', long, env, default_value = "redis://127.0.0.1:6379")]
  pub redis_url: String,
  #[clap(short = 'x', long, env, default_value = "true")]
  pub no_auth: bool,
  #[clap(short = 'v', long, env, default_value = "./pypydance-song")]
  pub video_path: String,
  #[clap(long, env, default_value = "./udondance-song")]
  pub video_path_ud: String,
  #[clap(long, env, default_value = "./pypydance-cache")]
  pub cache_path_jd: String,
  #[clap(long, env, default_value = "./wannadance-cache")]
  pub cache_path_ud: String,
  #[clap(long, env, default_value = "ud-play.kiva.moe")]
  pub cache_upstream_ud_oversea: String,
  #[clap(long, env, default_value = "ud-nya.kiva.moe")]
  pub cache_upstream_ud_domestic: String,
  #[clap(short = 'l', long, env, default_value = "0.0.0.0:80")]
  pub listen: String,
  #[clap(short = '3', long, env)]
  pub builtin_l3_listen: Option<String>,
  #[clap(long, env, default_value = "jd-orig.kiva.moe:443")]
  pub builtin_l3_forward: String,
  #[clap(long, env, default_value = "ud-orig.kiva.moe:443")]
  pub builtin_l3_forward_ud: String,
  #[clap(short = 'w', long, env, default_value = "0.0.0.0:7991")]
  pub rtsp_listen: String,
  #[clap(long, env, default_value = "false")]
  pub rtsp_enable: bool,
  #[clap(long, env, default_value = "5")]
  pub receipt_max_per_user_per_sender: usize,
  #[clap(long, env, default_value = "300")]
  pub receipt_default_expire_seconds: u64,

  #[clap(long, env, value_delimiter = ',')]
  pub admin_src_host: Option<Vec<String>>,
}

#[derive(Debug)]
pub struct AppServiceImpl {
  pub opts: AppOpts,
  pub redis: Option<RedisService>,
  pub typewriter: TypewriterService,
  pub cdn_jd: CdnService,
  pub cdn_ud: CdnService,
  pub index: IndexService,
  pub receipt: ReceiptService,
}

pub type AppService = Arc<AppServiceImpl>;

impl AppServiceImpl {
  pub async fn new(opts: AppOpts) -> Result<AppService> {
    let redis = match opts.no_auth {
      true => {
        info!("Authentication disabled");
        None
      }
      false => {
        info!("Authentication enabled: using {}", opts.redis_url);
        Some(RedisServiceImpl::new(opts.redis_url.clone()).await?)
      }
    };
    let cdn_jd = CdnServiceImpl::new(opts.video_path.clone(), opts.cache_path_jd.clone(), redis.clone());
    let cdn_ud = CdnServiceImpl::new(opts.video_path_ud.clone(), opts.cache_path_ud.clone(), redis.clone());
    let typewriter = Arc::new(TypewriterServiceImpl::default());
    let index = IndexServiceImpl::new(opts.video_path.clone()).await?;
    let receipt = ReceiptServiceImpl::new(
      opts.receipt_max_per_user_per_sender,
      Duration::from_secs(opts.receipt_default_expire_seconds),
    )
    .await?;
    Ok(Arc::new(AppServiceImpl {
      opts,
      redis,
      cdn_jd,
      cdn_ud,
      typewriter,
      index,
      receipt,
    }))
  }
}

pub fn my_git_hash() -> String {
  option_env!("VERGEN_GIT_SHA").map(|x| x[..8].to_string()).unwrap_or_else(|| "0".to_string())
}