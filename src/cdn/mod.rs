use std::{net::IpAddr, sync::Arc};

use anyhow::anyhow;
use log::trace;

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
  pub token_sign_secret: String,
}

pub type CdnService = Arc<CdnServiceImpl>;
pub type CdnFetchToken = UuidString;
pub type ChecksumType = String;
pub type TimestampType = i64;
pub type SignTimestampType = String;
pub type SignType = String;

impl CdnServiceImpl {
  pub fn new(
    video_path: String,
    video_override_path: String,
    cache_path: String,
    token_valid_seconds: i64,
    token_sign_secret: String,
  ) -> CdnService {
    Arc::new(CdnServiceImpl {
      video_path,
      video_override_path,
      cache_path,
      token_valid_seconds,
      token_sign_secret,
    })
  }
}

#[derive(Debug, Clone)]
pub enum CdnFetchResult {
  Hit(
    CdnFetchToken,
    ChecksumType,
    TimestampType,
    SignType,
    SignTimestampType,
  ),
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
  pub async fn get_video_file_checksum_by_cached_video(
    &self,
    cached_video: &CachedVideo,
  ) -> Result<ChecksumType> {
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
      CachedVideo::Video {
        metadata_json_file, ..
      } => std::fs::File::open(metadata_json_file)
        .map_err(|e| anyhow::anyhow!("Failed to open metadata: {:?}", e))
        .and_then(|f| {
          serde_json::from_reader::<_, aya_dance_types::Song>(f)
            .map_err(|e| anyhow::anyhow!("Failed to parse metadata: {:?}", e))
        })
        .and_then(|s| {
          s.checksum
            .ok_or_else(|| anyhow::anyhow!("No checksum in metadata"))
        }),
    }
  }

  pub async fn get_video_file_checksum_by_id(&self, id: SongId) -> Result<ChecksumType> {
    match self.get_video_file_path(id).await {
      CachedVideoFile::Available(cached_video) => {
        self
          .get_video_file_checksum_by_cached_video(&cached_video)
          .await
      }
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
    id: SongId,
    token: Option<String>,
    checksum: ChecksumType,
    remote: IpAddr,
  ) -> Result<Option<CachedVideo>> {
    match token {
      Some(token) => self.serve_file_auth(id, token, checksum, remote).await,
      None => self.serve_file_no_auth(id).await,
    }
  }

  async fn serve_file_no_auth(&self, id: SongId) -> Result<Option<CachedVideo>> {
    trace!("serve_file_no_auth: id={:?}", id);

    match self.get_video_file_path(id).await {
      CachedVideoFile::Available(video) => Ok(Some(video)),
      _ => Ok(None),
    }
  }

  async fn serve_file_auth(
    &self,
    id: SongId,
    token: String,
    checksum: ChecksumType,
    remote: IpAddr,
  ) -> Result<Option<CachedVideo>> {
    trace!("serve_file: token={}, client={}", token, remote);

    Self::verify_token(
      &token,
      &self.token_sign_secret,
      id,
      &checksum,
      self.token_valid_seconds,
    )?;

    match self.get_video_file_path(id).await {
      CachedVideoFile::Available(video) => Ok(Some(video)),
      _ => Ok(None),
    }
  }

  fn encode_token(sign: &SignType, sign_ts: &SignTimestampType) -> String {
    format!("{}-{}", sign, sign_ts)
  }

  fn decode_token(token: &str) -> Result<(SignType, SignTimestampType)> {
    let mut parts = token.split('-');
    let sign = parts.next().ok_or_else(|| anyhow!("missing sign"))?;
    let sign_ts = parts
      .next()
      .ok_or_else(|| anyhow!("missing sign timestamp"))?;
    Ok((sign.to_string(), sign_ts.to_string()))
  }

  fn generate_sign(
    secret: &str,
    id: SongId,
    checksum: &ChecksumType,
    sign_ts: &SignTimestampType,
  ) -> String {
    let sign_plain = format!("{}/v/{}-{}.mp4{}", secret, id, checksum, sign_ts);
    format!("{:x}", md5::compute(sign_plain))
  }

  fn verify_token(
    token: &str,
    secret: &str,
    id: SongId,
    checksum: &ChecksumType,
    token_valid_seconds: i64,
  ) -> Result<()> {
    let (sign, sign_ts) = Self::decode_token(token)?;
    let sign_verify = Self::generate_sign(secret, id, checksum, &sign_ts);
    if sign_verify != sign {
      return Err(anyhow!(
        "token mismatch: provided={}, wanted={}",
        sign,
        sign_verify
      ));
    }
    let provided_ts = i64::from_str_radix(&sign_ts, 16)?;
    let now = chrono::Utc::now().timestamp();
    if now - provided_ts > token_valid_seconds {
      return Err(anyhow!(
        "token expired: now={}, provided={}, diff={}, tolerance={}",
        now,
        provided_ts,
        now - provided_ts,
        token_valid_seconds
      ));
    }
    Ok(())
  }

  pub async fn serve_token(&self, id: SongId, remote: IpAddr) -> Result<CdnFetchResult> {
    trace!("serve_token: id={}, client={}", id, remote);

    match self.get_video_file_path(id).await {
      CachedVideoFile::Available(video) => {
        match self.get_video_file_checksum_by_cached_video(&video).await {
          Ok(checksum) => {
            let ts = chrono::Utc::now().timestamp();
            let sign_ts = format!("{:08X}", ts);
            let sign = Self::generate_sign(&self.token_sign_secret, id, &checksum, &sign_ts);
            let token = Self::encode_token(&sign, &sign_ts);
            Ok(CdnFetchResult::Hit(token, checksum, ts, sign, sign_ts))
          }
          Err(e) => {
            log::warn!(
              "Failed to get checksum for video file, will force a cache-miss: {}: {}",
              video.video_file(),
              e
            );
            Ok(CdnFetchResult::Miss)
          }
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
