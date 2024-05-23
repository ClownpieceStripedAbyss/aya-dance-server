use std::{collections::HashMap, sync::Arc, time::Duration};

use itertools::Either;
use serde_derive::{Deserialize, Serialize};

use crate::{
  types::{timedmap::TimedMap, SongId, UuidString},
  Result,
};

pub type UserId = String;
pub type ReceiptId = UuidString;
pub type RoomId = String;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
  pub receipt_id: ReceiptId,
  pub room_id: RoomId,
  pub target: UserId,
  pub added_at: i64,
  pub expire_at: i64,
  pub song_id: Option<SongId>,
  pub song_url: Option<String>,
  pub sender: Option<UserId>,
  pub message: Option<String>,
}

#[derive(Debug)]
pub struct ReceiptServiceImpl {
  /// TimedMap is thread-safe, since it uses a RwLock internally.
  receipts: TimedMap<ReceiptId, Receipt>,
}

pub type ReceiptService = Arc<ReceiptServiceImpl>;

impl ReceiptServiceImpl {
  pub async fn new() -> ReceiptService {
    Arc::new(ReceiptServiceImpl {
      receipts: Default::default(),
    })
  }
}

impl ReceiptServiceImpl {
  pub async fn receipts(&self, room_id: RoomId) -> Vec<Receipt> {
    self
      .receipts
      .snapshot::<HashMap<_, _>>()
      .into_iter()
      .filter(|(_, receipt)| receipt.room_id == room_id)
      .map(|(_, receipt)| receipt)
      .collect()
  }

  pub async fn create_receipt(
    &self,
    room_id: RoomId,
    target: UserId,
    song: Either<SongId, String>,
    sender: Option<UserId>,
    message: Option<String>,
  ) -> Result<Receipt> {
    // TODO: maximum number of receipts per user
    let receipts = &self.receipts;
    let (song_id, song_url) = match song {
      Either::Left(id) => (Some(id), None),
      Either::Right(url) => (None, Some(url)),
    };
    let uuid = {
      let mut uuid = UuidString::new();
      while receipts.contains(&uuid) {
        uuid = UuidString::new();
      }
      uuid
    };
    let valid_duration = Duration::from_secs(60 * 10); // 10 minutes
    let added_at = chrono::Utc::now();
    let expire_at = added_at + valid_duration;
    let receipt = Receipt {
      receipt_id: uuid.clone(),
      room_id,
      added_at: added_at.timestamp(),
      expire_at: expire_at.timestamp(),
      song_id,
      song_url,
      sender,
      target,
      message,
    };
    receipts.insert(uuid, receipt.clone(), valid_duration);
    Ok(receipt)
  }
}
