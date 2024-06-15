use std::sync::Arc;

use aya_dance_types::songs_to_index;
pub use aya_dance_types::SongIndex;
use log::{debug, warn};
use tokio::sync::Mutex;

use crate::{types::Song, Result};

pub mod watch;

#[derive(Debug)]
pub struct IndexServiceImpl {
  pub video_path: String,
  pub index: Mutex<Option<SongIndex>>,
}

pub type IndexService = Arc<IndexServiceImpl>;

impl IndexServiceImpl {
  pub async fn new(video_path: String) -> Result<IndexService> {
    Ok(Arc::new(IndexServiceImpl {
      video_path,
      index: Default::default(),
    }))
  }
}

impl IndexServiceImpl {
  pub async fn get_index(&self, force_rebuild: bool) -> Result<SongIndex> {
    let mut index = self.index.lock().await;
    if force_rebuild {
      *index = None;
    }
    if let Some(index) = &*index {
      return Ok(index.clone());
    }
    let result = self.build_index().await?;
    *index = Some(result.clone());
    Ok(result) // implicitly drop the lock
  }

  pub async fn build_index(&self) -> Result<SongIndex> {
    debug!("Building index from {}", self.video_path);
    let path = self.video_path.clone();

    // iterate path for each subdirectory
    // for each subdirectory, read its metadata.json file,
    // parse the metadata.json file into a Song struct.
    let mut songs = vec![];

    let mut cursor = tokio::fs::read_dir(path).await?;
    while let Some(entry) = cursor.next_entry().await? {
      let path = entry.path();
      if path.is_dir() {
        let metadata_path = path.join("metadata.json");
        if metadata_path.exists() {
          let metadata = match tokio::fs::read_to_string(&metadata_path).await {
            Ok(metadata) => metadata,
            Err(e) => {
              warn!(
                "Failed to read metadata file {}: {:?}",
                metadata_path.to_str().unwrap_or("<unknown-file>"),
                e
              );
              continue;
            }
          };
          let song: Song = match serde_json::from_str(&metadata) {
            Ok(song) => song,
            Err(e) => {
              warn!(
                "Failed to parse metadata file {}: {:?}",
                metadata_path.to_str().unwrap_or("<unknown-file>"),
                e
              );
              continue;
            }
          };
          if song.id.to_string() != path.file_name().unwrap_or_default().to_string_lossy() {
            warn!(
              "Song id mismatch: {} (directory) != {} (metadata), skipping",
              path.file_name().unwrap().to_string_lossy(),
              song.id
            );
            continue;
          }
          songs.push(song);
        }
      }
    }

    Ok(songs_to_index(songs))
  }
}
