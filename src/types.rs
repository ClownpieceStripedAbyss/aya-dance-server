use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct Song {
    pub id: i32,
    pub category: i32,
    pub name: String,
}
