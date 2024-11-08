use itertools::Itertools;
use serde::{Deserialize, Serialize};

pub type SongId = u32;
pub type CategoryId = u32;
pub type UuidString = String;

// {
//   "id": 1,
//   "category": 5,
//   "title": "2 Be Loved (Am I Ready) - Lizzo",
//   "categoryName": "FitDance",
//   "url": "https://aya-dance-cf.kiva.moe/api/v1/videos/1.mp4",
//   "urlForQuest": "",
//   "titleSpell": "2 Be Loved ( Am I Ready ) - Lizzo",
//   "playerIndex": 0,
//   "volume": 0.36,
//   "start": 0,
//   "end": 202,
//   "flip": false,
//   "skipRandom": false,
//   "originalUrl": [
//     "https://exmaple.com/"
//   ],
//   "checksum": "ef2e97e4118f146cb3d472fe48c7d9e2"
// },
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Song {
  pub id: SongId,
  pub category: CategoryId,
  pub title: String,
  #[serde(rename = "categoryName")]
  pub category_name: String,
  #[serde(rename = "titleSpell")]
  pub title_spell: String,
  #[serde(rename = "playerIndex")]
  pub player_index: u32,
  pub volume: f32,
  pub start: u32,
  pub end: u32,
  pub flip: bool,
  #[serde(rename = "skipRandom")]
  pub skip_random: bool,
  #[serde(rename = "originalUrl")]
  pub original_url: Option<Vec<String>>,
  pub checksum: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Category {
  pub title: String,
  pub entries: Vec<Song>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SongIndex {
  pub updated_at: i64,
  pub categories: Vec<Category>,
}

pub fn songs_to_index(mut songs: Vec<Song>) -> SongIndex {
  // Make sure it is sorted by id
  songs.sort_by_key(|s| s.id);

  let mut original_categories = songs
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
  categories.append(&mut original_categories);
  SongIndex {
    updated_at: chrono::Utc::now().timestamp(),
    categories,
  }
}
