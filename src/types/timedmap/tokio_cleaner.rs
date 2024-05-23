use std::{sync::Arc, time::Duration};

use async_trait::async_trait;

/// Cleanup defines an implementation where expired
/// elements can be removed.
#[async_trait]
pub trait Cleanup: Send + Sync {
  /// Cleanup removes all elements
  /// which have been expired.
  async fn cleanup(&self);
}

/// Start a new cleanup cycle on the given [`Cleanup`](crate::Cleanup)
/// implementation instance and returns a function to cancel the
/// cleanup cycle.
///
/// On each elapse, the map ich checked for expired
/// key-value pairs and removes them from the map.
pub fn tokio_cleaner(m: Arc<dyn Cleanup>, interval: Duration) -> Box<dyn Fn()> {
  let job = tokio::spawn(async move {
    loop {
      tokio::time::sleep(interval).await;
      m.cleanup().await;
    }
  });
  Box::new(move || job.abort())
}
