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
  pub cache_path: String,
}

pub type CdnService = Arc<CdnServiceImpl>;
pub type CdnFetchToken = UuidString;

impl CdnServiceImpl {
  pub fn new(video_path: String, cache_path: String) -> CdnService {
    Arc::new(CdnServiceImpl {
      video_path,
      cache_path,
    })
  }
}

#[derive(Debug, Clone)]
pub enum CdnFetchResult {
  Hit(CdnFetchToken),
  Miss,
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

  async fn serve_file_no_auth(&self, id: SongId) -> (String, String, bool) {
    trace!("serve_file_no_auth: id={}", id);
    self.get_video_file_path(id).await
  }

  pub async fn serve_file_auth(
    &self,
    id: Option<SongId>,
    token: String,
    remote: IpAddr,
  ) -> Result<Option<String>> {
    trace!("serve_file: token={}, client={}", token, remote);

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

    let (video, _, avail) = self.get_video_file_path(id_in_token).await;
    Ok(avail.then(|| video))
  }

  pub async fn serve_token(&self, id: SongId, remote: IpAddr) -> Result<CdnFetchResult> {
    trace!("serve_token: id={}, client={}", id, remote);
    let token = token_for_song_id(id);

    let (_, _, avail) = self.get_video_file_path(id).await;
    match avail {
      true => Ok(CdnFetchResult::Hit(token)),
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
