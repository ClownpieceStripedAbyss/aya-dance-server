use serde::{Deserialize, Serialize};

pub type SongId = u32;
pub type SongCategory = u32;

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct Song {
  pub id: SongId,
  pub category: SongCategory,
  pub name: String,
}
