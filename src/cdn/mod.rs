use std::{net::IpAddr, sync::Arc};

use anyhow::anyhow;
use log::trace;
use uuid::Uuid;

use crate::{
  types::{SongId, UuidString},
  Result,
};

pub mod proxy;
pub mod range;
pub mod receipt;

#[derive(Debug)]
pub struct CdnServiceImpl {
  pub video_path: String,
  pub video_override_path: String,
  pub cache_path: String,
  pub token_valid_seconds: i64,
}

pub type CdnService = Arc<CdnServiceImpl>;
pub type CdnFetchToken = UuidString;
pub type ChecksumType = String;
pub type TimestampType = i64;

impl CdnServiceImpl {
  pub fn new(video_path: String, video_override_path: String, cache_path: String, token_valid_seconds: i64) -> CdnService {
    Arc::new(CdnServiceImpl {
      video_path,
      video_override_path,
      cache_path,
      token_valid_seconds,
    })
  }
}

#[derive(Debug, Clone)]
pub enum CdnFetchResult {
  Hit(CdnFetchToken, ChecksumType),
  Miss,
}

#[derive(Debug, Clone)]
pub enum CachedVideo {
  Video {
    video_file: String,
    metadata_json_file: String,
  },
  VideoOverride {
    video_file: String,
  },
}

#[derive(Debug, Clone)]
pub enum CachedVideoFile {
  Available(CachedVideo),
  Unavailable {
    video_file: String,
    metadata_json_file: String,
  },
}

impl CachedVideo {
  pub fn video_file(&self) -> String {
    match self {
      CachedVideo::Video { video_file, .. } => video_file.clone(),
      CachedVideo::VideoOverride { video_file } => video_file.clone(),
    }
  }
}

impl CdnServiceImpl {
  pub async fn get_video_file_checksum_by_cached_video(&self, cached_video: &CachedVideo) -> Result<ChecksumType> {
    match cached_video {
      CachedVideo::VideoOverride { video_file } => {
        // Now get the file's last modified time to be the checksum
        std::fs::metadata(video_file.as_str())
          .map_err(|e| anyhow::anyhow!("Failed to get metadata: {:?}", e))
          .and_then(|m| {
            m.modified()
              .map_err(|e| anyhow::anyhow!("Failed to get modified time: {:?}", e))
          })
          .and_then(|t| {
            t.duration_since(std::time::SystemTime::UNIX_EPOCH)
              .map_err(|e| anyhow::anyhow!("Failed to get duration since epoch: {:?}", e))
          })
          .and_then(|d| Ok(d.as_secs().to_string()))
          .map(|last_modified| format!("override{}", last_modified))
      }
      CachedVideo::Video { metadata_json_file, .. } => {
        std::fs::File::open(metadata_json_file)
          .map_err(|e| anyhow::anyhow!("Failed to open metadata: {:?}", e))
          .and_then(|f| {
            serde_json::from_reader::<_, aya_dance_types::Song>(f)
              .map_err(|e| anyhow::anyhow!("Failed to parse metadata: {:?}", e))
          })
          .and_then(|s| {
            s.checksum
              .ok_or_else(|| anyhow::anyhow!("No checksum in metadata"))
          })
      }
    }
  }
  
  pub async fn get_video_file_checksum_by_id(&self, id: SongId) -> Result<ChecksumType> {
    match self.get_video_file_path(id).await {
      CachedVideoFile::Available(cached_video) => self.get_video_file_checksum_by_cached_video(&cached_video).await,
      CachedVideoFile::Unavailable { .. } => Err(anyhow!("video file not found")),
    }
  }
  
  pub async fn get_video_file_path(&self, id: SongId) -> CachedVideoFile {
    let metadata_json = format!("{}/{}/metadata.json", self.video_path, id);
    let video_mp4 = format!("{}/{}/video.mp4", self.video_path, id);
    let override_mp4 = format!("{}/{}.mp4", self.video_override_path, id);

    if std::path::Path::new(&override_mp4).exists() {
      return CachedVideoFile::Available(CachedVideo::VideoOverride {
        video_file: override_mp4,
      });
    }
    if std::path::Path::new(&metadata_json).exists() && std::path::Path::new(&video_mp4).exists() {
      return CachedVideoFile::Available(CachedVideo::Video {
        video_file: video_mp4,
        metadata_json_file: metadata_json,
      });
    }
    CachedVideoFile::Unavailable {
      video_file: video_mp4,
      metadata_json_file: metadata_json,
    }
  }

  pub async fn serve_file(
    &self,
    id: Option<SongId>,
    token: Option<String>,
    remote: IpAddr,
  ) -> Result<Option<CachedVideo>> {
    match token {
      Some(token) => self.serve_file_auth(id, token, remote).await,
      None => self.serve_file_no_auth(id).await,
    }
  }

  async fn serve_file_no_auth(&self, id: Option<SongId>) -> Result<Option<CachedVideo>> {
    trace!("serve_file_no_auth: id={:?}", id);
    
    match self.get_video_file_path(id.ok_or_else(|| anyhow!("missing song id"))?).await {
      CachedVideoFile::Available(video) => Ok(Some(video)),
      _ => Ok(None),
    }
  }

  async fn serve_file_auth(
    &self,
    id: Option<SongId>,
    token: String,
    remote: IpAddr,
  ) -> Result<Option<CachedVideo>> {
    trace!("serve_file: token={}, client={}", token, remote);

    // Get the song id from the token
    let (id_in_token, ts_in_token) = match song_id_for_token(&token) {
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
    // If timestamp is provided, it must not expire
    if self.token_valid_seconds != 0 && ts_in_token + self.token_valid_seconds < chrono::Utc::now().timestamp() {
      return Err(anyhow!("token expired"));
    }

    match self.get_video_file_path(id_in_token).await {
      CachedVideoFile::Available(video) => Ok(Some(video)),
      _ => Ok(None),
    }
  }

  pub async fn serve_token(&self, id: SongId, remote: IpAddr) -> Result<CdnFetchResult> {
    trace!("serve_token: id={}, client={}", id, remote);
    let token = token_for_song_id(id);

    match self.get_video_file_path(id).await {
      CachedVideoFile::Available(video) => match self.get_video_file_checksum_by_cached_video(&video).await {
        Ok(checksum) => Ok(CdnFetchResult::Hit(token, checksum)),
        Err(e) => {
          log::warn!("Failed to get checksum for video file, will force a cache-miss: {}: {}", video.video_file(), e);
          Ok(CdnFetchResult::Miss)
        }
      }
      _ => Ok(CdnFetchResult::Miss),
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
    match self.get_video_file_path(id).await {
      // Always serve the override file if it exists
      CachedVideoFile::Available(CachedVideo::VideoOverride { video_file }) => {
        (download_tmp_file, video_file, "".to_string(), true)
      }
      // Local cache found and it's not an override, check if it's the correct file
      CachedVideoFile::Available(CachedVideo::Video {
        video_file,
        metadata_json_file,
      }) => {
        if std::fs::metadata(&video_file).map(|x| x.len()).unwrap_or(0) != size {
          return (download_tmp_file, video_file, metadata_json_file, false);
        }
        let reader = match std::fs::File::open(&metadata_json_file) {
          Ok(f) => f,
          Err(e) => {
            log::warn!("Failed to open metadata file {}: {}", metadata_json_file, e);
            return (download_tmp_file, video_file, metadata_json_file, false);
          }
        };
        let x: aya_dance_types::Song = match serde_json::from_reader(reader) {
          Ok(x) => x,
          Err(e) => {
            log::warn!(
              "Failed to parse metadata file {}: {}",
              metadata_json_file,
              e
            );
            return (download_tmp_file, video_file, metadata_json_file, false);
          }
        };
        match x.checksum {
          Some(x) if x == md5 => (download_tmp_file, video_file, metadata_json_file, true),
          _ => (download_tmp_file, video_file, metadata_json_file, false),
        }
      }
      // Video isn't found, just tell our caller to download it
      CachedVideoFile::Unavailable {
        video_file,
        metadata_json_file,
      } => (download_tmp_file, video_file, metadata_json_file, false),
    }
  }
}

fn token_for_song_id(song_id: SongId) -> String {
  let uuid = Uuid::new_v4().to_string();
  // We use a simple encoding to avoid exposing the actual song id.
  // By converting the song id to a fixed-length string.
  format!("{}{:08x}{:016x}", uuid, song_id, chrono::Utc::now().timestamp())
}

fn song_id_for_token(token: &str) -> Option<(SongId, TimestampType)> {
  if token.len() < 36 {
    return None;
  }
  let (uuid, song_id_and_ts) = token.split_at(36);
  if Uuid::parse_str(uuid).is_ok() {
    let song_id = SongId::from_str_radix(&song_id_and_ts[0..8], 16).ok()?;
    let ts = TimestampType::from_str_radix(&song_id_and_ts[8..], 16).ok()?;
    Some((song_id, ts))
  } else {
    None
  }
}
