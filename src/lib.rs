extern crate core;

use std::{sync::Arc, time::Duration};

use clap::Parser;

use crate::{
  cdn::{
    receipt::{ReceiptService, ReceiptServiceImpl},
    CdnService, CdnServiceImpl,
  },
  rtsp::{TypewriterService, TypewriterServiceImpl},
};

pub mod cdn;
pub mod ffmpeg;
pub mod forward;
pub mod http;
pub mod index;
pub mod obws;
pub mod rtsp;
pub mod types;

pub type Result<T> = anyhow::Result<T>;

pub const MY_VERSION_ID: u32 = 1;

#[derive(Debug, Parser, Clone)]
pub struct AppOpts {
  #[clap(long, env, default_value = "./wannadance-song")]
  pub video_path_ud: String,
  #[clap(long, env, default_value = "./wannadance-cache")]
  pub cache_path_ud: String,
  #[clap(long, env, default_value = "./wannadance-override")]
  pub video_override_path_ud: String,

  #[clap(long, env, default_value = "ud-play.kiva.moe")]
  pub cache_upstream_ud_oversea: String,
  #[clap(long, env, default_value = "ud-nya.kiva.moe")]
  pub cache_upstream_ud_domestic: String,
  #[clap(long, env, default_value = "ud-orig.kiva.moe")]
  pub api_upstream_ud: String,

  #[clap(short = 'l', long, env, default_value = "0.0.0.0:80")]
  pub listen: String,
  #[clap(long, env, default_value = "0.0.0.0:443")]
  pub builtin_sni_listen: Option<String>,
  #[clap(
    long,
    env,
    value_delimiter = ',',
    default_value = "api.udon.dance=ud-orig.kiva.moe:443,nya.xin.moe=ud-nya.kiva.moe:443,play.udon.dance=ud-play.kiva.moe:443"
  )]
  pub builtin_sni_proxy: Option<Vec<String>>,

  #[clap(short = 'w', long, env)]
  pub rtsp_listen: Option<String>,
  #[clap(long, env, default_value = "5")]
  pub receipt_max_per_user_per_sender: usize,
  #[clap(long, env, default_value = "300")]
  pub receipt_default_expire_seconds: u64,

  #[clap(long, env, value_delimiter = ',')]
  pub admin_src_host: Option<Vec<String>>,
  #[clap(long, env, default_value = "3600")]
  pub token_valid_seconds: i64,

  #[clap(long, env, default_value = "false")]
  pub proxy_allow_304: bool,

  #[clap(long, env, default_value = "0")]
  pub audio_compensation: f64,

  #[clap(long, env)]
  pub obws_host: Option<String>,
  #[clap(long, env, default_value = "4455")]
  pub obws_port: u16,
}

#[derive(Debug)]
pub struct AppServiceImpl {
  pub opts: AppOpts,
  pub typewriter: TypewriterService,
  pub cdn: CdnService,
  pub receipt: ReceiptService,
}

pub type AppService = Arc<AppServiceImpl>;

impl AppServiceImpl {
  pub async fn new(opts: AppOpts) -> Result<AppService> {
    let cdn = CdnServiceImpl::new(
      opts.video_path_ud.clone(),
      opts.video_override_path_ud.clone(),
      opts.cache_path_ud.clone(),
      opts.token_valid_seconds,
    );
    let typewriter = Arc::new(TypewriterServiceImpl::default());
    let receipt = ReceiptServiceImpl::new(
      opts.receipt_max_per_user_per_sender,
      Duration::from_secs(opts.receipt_default_expire_seconds),
    )
    .await?;
    Ok(Arc::new(AppServiceImpl {
      opts,
      cdn,
      typewriter,
      receipt,
    }))
  }
}

pub fn my_git_hash() -> String {
  option_env!("VERGEN_GIT_SHA")
    .map(|x| x[..8].to_string())
    .unwrap_or_else(|| "0".to_string())
}
