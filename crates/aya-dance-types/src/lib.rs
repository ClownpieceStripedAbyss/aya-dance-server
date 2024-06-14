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
  pub original_url: Vec<String>,
  pub checksum: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Category {
  pub title: String,
  pub entries: Vec<Song>,
}
