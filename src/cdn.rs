use std::{net::IpAddr, sync::Arc};

use anyhow::anyhow;
use log::debug;
use redis::{AsyncCommands, SetExpiry, SetOptions};
use uuid::Uuid;

use crate::{redis::RedisService, types::SongId, Result};

#[derive(Debug)]
pub struct CdnServiceImpl {
    pub video_path: String,
    pub redis: Option<RedisService>,
}

pub type CdnService = Arc<CdnServiceImpl>;

impl CdnServiceImpl {
    pub fn new(video_path: String, redis: Option<RedisService>) -> CdnService {
        Arc::new(CdnServiceImpl { video_path, redis })
    }
}

#[derive(Debug, Clone)]
pub enum CdnFetchResult {
    Hit(String),
    Miss,
}

macro_rules! redis_get {
    ($redis:expr, $k:expr, $d:expr) => {
        match $redis.get().await?.get($k).await {
            Ok(v) => v,
            Err(e) => match e.kind() {
                redis::ErrorKind::TypeError => $d,
                _ => return Err(e.into()),
            },
        }
    };
}

impl CdnServiceImpl {
    pub async fn get_video_file_path(&self, id: SongId) -> Option<String> {
        let metadata_json = format!("{}/{}/metadata.json", self.video_path, id);
        let video_mp4 = format!("{}/{}/video.mp4", self.video_path, id);
        if std::path::Path::new(&metadata_json).exists()
            && std::path::Path::new(&video_mp4).exists()
        {
            Some(video_mp4)
        } else {
            None
        }
    }

    pub async fn serve_file(
        &self,
        id: Option<SongId>,
        token: String,
        remote: IpAddr,
    ) -> Result<Option<String>> {
        debug!("serve_file: token={}, client={}", token, remote);

        // Get the song id from the token
        let id_in_token = match song_id_for_token(&token) {
            Some(id) => id,
            None => return Err(anyhow!("Invalid token")),
        };

        // If provided, the song id must match the one in the token
        match id {
            Some(id) if id != id_in_token => {
                return Err(anyhow!("Invalid token"));
            }
            _ => (),
        }

        // Check if the provided token is valid
        let (is_member, is_valid) = match &self.redis {
            None => (true, true),
            Some(redis) => {
                let redis = redis.pool.clone();
                let tokens_set = format!("cdn_token_set:{}:{}", id_in_token, remote);
                let token_valid = format!("cdn_token_valid:{}", token);
                let is_member: bool = redis
                    .get()
                    .await?
                    .sismember(&tokens_set, token.clone())
                    .await?;
                let is_valid: bool = redis_get!(redis, token_valid, false);
                if !is_member || !is_valid {
                    redis.get().await?.srem(&tokens_set, token).await?;
                }
                (is_member, is_valid)
            }
        };

        if is_member && is_valid {
            Ok(self.get_video_file_path(id_in_token).await)
        } else {
            return Err(anyhow!("Invalid token"));
        }
    }

    pub async fn serve_token(&self, id: SongId, remote: IpAddr) -> Result<CdnFetchResult> {
        debug!("serve_token: id={}, client={}", id, remote);
        let token = token_for_song_id(id);

        match self.get_video_file_path(id).await {
            // Now if the file exists, we can generate a token for the client.
            Some(_) => {
                if let Some(redis) = &self.redis {
                    let tokens_set = format!("cdn_token_set:{}:{}", id, remote);
                    let token_valid = format!("cdn_token_valid:{}", token);

                    let redis = redis.pool.clone();
                    redis.get().await?.sadd(&tokens_set, token.clone()).await?;
                    // Mark the token as valid for 10 minutes
                    redis
                        .get()
                        .await?
                        .set_options(
                            &token_valid,
                            true,
                            SetOptions::default().with_expiration(SetExpiry::EX(10 * 60)),
                        )
                        .await?;
                }

                Ok(CdnFetchResult::Hit(token))
            }
            // Otherwise, return a miss.
            None => Ok(CdnFetchResult::Miss),
        }
    }
}

fn token_for_song_id(song_id: SongId) -> String {
    let uuid = Uuid::new_v4().to_string();
    format!("{}{}", uuid, encode_song_id(song_id))
}

fn song_id_for_token(token: &str) -> Option<SongId> {
    let (uuid, song_id) = token.split_at(36);
    if Uuid::parse_str(uuid).is_ok() {
        decode_song_id(song_id)
    } else {
        None
    }
}

fn encode_song_id(song_id: SongId) -> String {
    // We use a simple encoding to avoid exposing the actual song id.
    // By converting the song id to a fixed-length string.
    format!("{:04x}", song_id)
}

fn decode_song_id(encoded: &str) -> Option<SongId> {
    SongId::from_str_radix(encoded, 16).ok()
}
