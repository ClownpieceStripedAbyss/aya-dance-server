use std::{net::IpAddr, sync::Arc};

use anyhow::anyhow;
use log::debug;
use redis::{AsyncCommands, SetExpiry, SetOptions};
use uuid::Uuid;

use crate::{
  redis::RedisService,
  types::{SongId, UuidString},
  Result,
};

pub mod proxy;
pub mod range;
pub mod receipt;

#[derive(Debug)]
pub struct CdnServiceImpl {
  pub video_path: String,
  pub cache_path: String,
  pub redis: Option<RedisService>,
}

pub type CdnService = Arc<CdnServiceImpl>;
pub type CdnFetchToken = UuidString;

impl CdnServiceImpl {
  pub fn new(video_path: String, cache_path: String, redis: Option<RedisService>) -> CdnService {
    Arc::new(CdnServiceImpl {
      video_path,
      cache_path,
      redis,
    })
  }
}

#[derive(Debug, Clone)]
pub enum CdnFetchResult {
  Hit(CdnFetchToken),
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
  pub async fn get_video_file_path(&self, id: SongId) -> (String, String, bool) {
    let metadata_json = format!("{}/{}/metadata.json", self.video_path, id);
    let video_mp4 = format!("{}/{}/video.mp4", self.video_path, id);
    let available =
      std::path::Path::new(&metadata_json).exists() && std::path::Path::new(&video_mp4).exists();
    (video_mp4, metadata_json, available)
  }

  pub async fn serve_file(
    &self,
    id: Option<SongId>,
    token: Option<String>,
    remote: IpAddr,
  ) -> Result<Option<String>> {
    match token {
      Some(token) => self.serve_file_auth(id, token, remote).await,
      None => {
        let (video, _, avail) = self
          .serve_file_no_auth(id.ok_or_else(|| anyhow!("missing song id"))?)
          .await;
        Ok(avail.then(|| video))
      }
    }
  }

  pub async fn serve_file_no_auth(&self, id: SongId) -> (String, String, bool) {
    debug!("serve_file_no_auth: id={}", id);
    self.get_video_file_path(id).await
  }

  pub async fn serve_file_auth(
    &self,
    id: Option<SongId>,
    token: String,
    remote: IpAddr,
  ) -> Result<Option<String>> {
    debug!("serve_file: token={}, client={}", token, remote);

    // Get the song id from the token
    let id_in_token = match song_id_for_token(&token) {
      Some(id) => id,
      None => return Err(anyhow!("wrong token format")),
    };

    // If provided, the song id must match the one in the token
    match id {
      Some(id) if id != id_in_token => {
        return Err(anyhow!(
          "song id mismatch: {} (id) != {} (id in token)",
          id,
          id_in_token
        ));
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
      let (video, _, avail) = self.get_video_file_path(id_in_token).await;
      Ok(avail.then(|| video))
    } else {
      Err(anyhow!("token expired"))
    }
  }

  pub async fn serve_token(&self, id: SongId, remote: IpAddr) -> Result<CdnFetchResult> {
    debug!("serve_token: id={}, client={}", id, remote);
    let token = token_for_song_id(id);

    let (_, _, avail) = self.get_video_file_path(id).await;
    match avail {
      // Now if the file exists, we can generate a token for the client.
      true => {
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
      false => Ok(CdnFetchResult::Miss),
    }
  }

  pub async fn serve_local_cache(
    &self,
    id: SongId,
    file: String,
    md5: String,
    size: u64,
    remote: std::net::SocketAddr,
  ) -> (String, String, String, bool) {
    let download_tmp_file = format!("{}/{}_{}", self.cache_path, remote.port(), file);
    let (video, metadata_json, avail) = self.get_video_file_path(id).await;
    if !avail {
      return (download_tmp_file, video, metadata_json, false);
    }
    if std::fs::metadata(&video).map(|x| x.len()).unwrap_or(0) != size {
      return (download_tmp_file, video, metadata_json, false);
    }
    let reader = match std::fs::File::open(&metadata_json) {
      Ok(f) => f,
      Err(e) => {
        log::warn!("Failed to open metadata file {}: {}", metadata_json, e);
        return (download_tmp_file, video, metadata_json, false);
      }
    };
    let x: aya_dance_types::Song = match serde_json::from_reader(reader) {
      Ok(x) => x,
      Err(e) => {
        log::warn!("Failed to parse metadata file {}: {}", metadata_json, e);
        return (download_tmp_file, video, metadata_json, false);
      }
    };
    match x.checksum {
      Some(x) if x == md5 => (download_tmp_file, video, metadata_json, true),
      _ => (download_tmp_file, video, metadata_json, false),
    }
  }
}

fn token_for_song_id(song_id: SongId) -> String {
  let uuid = Uuid::new_v4().to_string();
  format!("{}{}", uuid, encode_song_id(song_id))
}

fn song_id_for_token(token: &str) -> Option<SongId> {
  if token.len() < 36 {
    return None;
  }
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
