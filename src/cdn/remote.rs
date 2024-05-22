use std::{collections::HashMap, sync::Arc, time::Duration};

use itertools::Either;
use serde_derive::{Deserialize, Serialize};
use timedmap::TimedMap;
use tokio::sync::RwLock;

use crate::{
  types::{SongId, UserId, UuidString},
  Result,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
  pub uuid: UuidString,
  pub added_at: i64,
  pub expire_at: i64,
  pub song_id: Option<SongId>,
  pub song_url: Option<String>,
  pub sender: Option<UserId>,
  pub target: UserId,
  pub notes: Option<String>,
}

#[derive(Debug)]
pub struct ReceiptServiceImpl {
  receipts: RwLock<TimedMap<UuidString, Receipt>>,
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
  pub async fn receipts(&self) -> Vec<Receipt> {
    self
      .receipts
      .read()
      .await
      .snapshot::<HashMap<_, _>>()
      .into_iter()
      .map(|(_, receipt)| receipt)
      .collect()
  }

  pub async fn create_receipt(
    &self,
    target: UserId,
    song: Either<SongId, String>,
    sender: Option<UserId>,
    notes: Option<String>,
  ) -> Result<Receipt> {
    // TODO: maximum number of receipts per user
    let receipts = self.receipts.write().await;
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
      uuid: uuid.clone(),
      added_at: added_at.timestamp(),
      expire_at: expire_at.timestamp(),
      song_id,
      song_url,
      sender,
      target,
      notes,
    };
    receipts.insert(uuid, receipt.clone(), valid_duration);
    Ok(receipt)
  }
}
