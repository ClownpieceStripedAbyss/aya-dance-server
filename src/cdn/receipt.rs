use std::{collections::HashMap, sync::Arc, time::Duration};

use anyhow::anyhow;
use itertools::{Either, Itertools};
use serde_derive::{Deserialize, Serialize};

use crate::{
  types::{timedmap, timedmap::TimedMap, SongId, UuidString},
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
  pub created_at: i64,
  pub expires_at: i64,
  pub song_id: Option<SongId>,
  pub song_url: Option<String>,
  pub sender: Option<UserId>,
  pub message: Option<String>,
}

#[derive(Debug)]
pub struct ReceiptServiceImpl {
  /// TimedMap is thread-safe, since it uses a RwLock internally.
  receipts: Arc<TimedMap<ReceiptId, Receipt>>,
  max_receipts_per_user_per_sender: usize,
  default_expire: Duration,
}

pub type ReceiptService = Arc<ReceiptServiceImpl>;

impl ReceiptServiceImpl {
  pub async fn new(
    max_receipts_per_user_per_sender: usize,
    default_expire: Duration,
  ) -> Result<ReceiptService> {
    let receipts = Arc::new(TimedMap::new());
    let _canceller = timedmap::tokio_cleaner(receipts.clone(), Duration::from_secs(60));
    Ok(Arc::new(ReceiptServiceImpl {
      receipts,
      max_receipts_per_user_per_sender,
      default_expire,
    }))
  }
}

impl ReceiptServiceImpl {
  pub async fn receipts(&self, room_id: RoomId) -> Vec<Receipt> {
    self
      .receipts
      .snapshot::<HashMap<_, _>>()
      .await
      .into_iter()
      .filter(|(_, receipt)| receipt.room_id == room_id)
      .map(|(_, receipt)| receipt)
      .sorted_by(|a, b| match (a.sender.as_ref(), b.sender.as_ref()) {
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (Some(_), None) => std::cmp::Ordering::Less,
        _ => a.created_at.cmp(&b.created_at),
      })
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
    let snapshots = self.receipts(room_id.clone()).await;
    let user_receipts = snapshots
      .iter()
      .filter(|r| r.target == target)
      .cloned()
      .collect::<Vec<_>>();

    let per_sender = user_receipts
      .into_iter()
      .sorted_by_key(|r| r.sender.clone())
      .chunk_by(|r| r.sender.clone())
      .into_iter()
      .map(|(sender, receipts)| (sender, receipts.collect::<Vec<_>>()))
      .collect::<HashMap<_, _>>();
    if let Some(from_this_sender) = per_sender.get(&sender) {
      if from_this_sender.len() >= self.max_receipts_per_user_per_sender {
        return Err(anyhow!(
          "Sender {:?} already reached the maximum number of receipts to target {}",
          sender,
          target
        ));
      }
      match song.as_ref() {
        Either::Left(song_id) if from_this_sender.iter().any(|r| r.song_id == Some(*song_id)) => {
          return Err(anyhow!(
            "Sender {:?} already sent a receipt with song id {} to target {}",
            sender,
            song_id,
            target
          ));
        }
        Either::Right(song_url)
          if from_this_sender
            .iter()
            .any(|r| r.song_url.as_ref() == Some(song_url)) =>
        {
          return Err(anyhow!(
            "Sender {:?} already sent a receipt with song url {} to target {}",
            sender,
            song_url,
            target
          ));
        }
        _ => (),
      }
    }

    let (song_id, song_url) = match song {
      Either::Left(id) => (Some(id), None),
      Either::Right(url) => (None, Some(url)),
    };

    let uuid = {
      let uuids = snapshots.iter().map(|r| &r.receipt_id).collect::<Vec<_>>();
      let mut uuid = uuid::Uuid::new_v4().to_string();
      while uuids.contains(&&uuid) {
        uuid = uuid::Uuid::new_v4().to_string();
      }
      uuid
    };
    let valid_duration = self.default_expire.clone();
    let created_at = chrono::Utc::now();
    let expires_at = created_at + valid_duration;
    let receipt = Receipt {
      receipt_id: uuid.clone(),
      room_id,
      created_at: created_at.timestamp(),
      expires_at: expires_at.timestamp(),
      song_id,
      song_url,
      sender,
      target,
      message,
    };
    self
      .receipts
      .insert(uuid, receipt.clone(), valid_duration)
      .await;
    Ok(receipt)
  }
}
