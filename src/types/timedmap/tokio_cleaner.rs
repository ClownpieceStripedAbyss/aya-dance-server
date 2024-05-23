use std::{sync::Arc, time::Duration};

/// Cleanup defines an implementation where expired
/// elements can be removed.
pub trait Cleanup: Send + Sync {
  /// Cleanup removes all elements
  /// which have been expired.
  fn cleanup(&self);
}

pub fn _start_cleaner(m: Arc<dyn Cleanup>, interval: Duration) -> Box<dyn Fn()> {
  let job = tokio::spawn(async move {
    loop {
      tokio::time::sleep(interval).await;
      m.cleanup();
    }
  });
  Box::new(move || job.abort())
}

/// Start a new cleanup cycle on the given [`Cleanup`](crate::Cleanup)
/// implementation instance and returns a function to cancel the
/// cleanup cycle.
///
/// On each elapse, the map ich checked for expired
/// key-value pairs and removes them from the map.
pub fn start_cleaner(m: Arc<dyn Cleanup>, interval: Duration) -> Box<dyn Fn()> {
  _start_cleaner(m, interval)
}

#[cfg(test)]
mod test {
  use tokio::time;

  use super::*;
  use crate::types::timedmap::TimedMap;

  #[tokio::test]
  async fn cleanup() {
    let tm = Arc::new(TimedMap::new());
    tm.insert("a", 1, Duration::from_millis(100));
    tm.insert("b", 2, Duration::from_millis(200));

    let _ = _start_cleaner(tm.clone(), Duration::from_millis(10));

    assert!(tm.get_value_unchecked(&"a").is_some());
    assert!(tm.get_value_unchecked(&"b").is_some());

    time::sleep(Duration::from_millis(150)).await;

    assert!(tm.get_value_unchecked(&"a").is_none());
    assert!(tm.get_value_unchecked(&"b").is_some());

    time::sleep(Duration::from_millis(60)).await;
    assert!(tm.get_value_unchecked(&"a").is_none());
    assert!(tm.get_value_unchecked(&"b").is_none());
  }
}
