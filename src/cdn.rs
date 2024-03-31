use std::sync::Arc;

use crate::{redis::RedisService, types::Song, AppOpts};

#[derive(Debug)]
pub struct CdnServiceImpl {
    pub video_path: String,
    pub redis: RedisService,
}

pub type CdnService = Arc<CdnServiceImpl>;

impl CdnServiceImpl {
    pub fn new(video_path: String, redis: RedisService) -> CdnService {
        Arc::new(CdnServiceImpl { video_path, redis })
    }
}

#[derive(Debug, Clone)]
pub struct CdnClientInfo {
    pub client_ip: String,
}

#[derive(Debug, Clone)]
pub enum CdnFetchResult {
    Hit { token: String, video_url: String },
    RateLimited,
    Miss,
}

impl CdnServiceImpl {
    pub async fn serve(&self, song: &Song) -> CdnFetchResult {
        CdnFetchResult::Miss
    }
}
