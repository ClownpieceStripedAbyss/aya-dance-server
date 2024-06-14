use std::sync::Arc;

pub use aya_dance_types::SongIndex;
use itertools::Itertools;
use log::{debug, warn};
use tokio::sync::Mutex;

use crate::{
  types::{Category, Song},
  Result,
};

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
          songs.push(song);
        }
      }
    }

    // Make sure it is sorted by id
    songs.sort_by_key(|s| s.id);

    let mut pypy_categories = songs
      .clone()
      .into_iter()
      // `chunk_by` only works on sorted data.
      .sorted_by_key(|s| s.category_name.clone())
      .chunk_by(|s| s.category_name.clone())
      .into_iter()
      .map(|(category_name, songs)| Category {
        title: category_name,
        // Now, sort each group by song id
        entries: songs.sorted_by_key(|s| s.id).collect(),
      })
      .sorted_by_key(|c| c.title.clone())
      .collect::<Vec<_>>();

    let mut categories = vec![];
    categories.push(Category {
      title: "All Songs".to_string(),
      entries: songs.clone(),
    });
    categories.push(Category {
      title: "Song's Family".to_string(),
      entries: songs
        .iter()
        .filter(|s| s.title.contains("[Song]"))
        .cloned()
        .collect(),
    });
    categories.append(&mut pypy_categories);

    Ok(SongIndex {
      updated_at: chrono::Utc::now().timestamp(),
      categories,
    })
  }
}
