use std::{
  borrow::Borrow,
  collections::HashMap,
  hash::Hash,
  time::{Duration, Instant},
};

use async_trait::async_trait;
use tokio::sync::RwLock;

use crate::types::timedmap::{time::TimeSource, tokio_cleaner::Cleanup, Value};

/// Provides a hash map with expiring key-value pairs.
#[derive(Debug)]
pub struct TimedMap<K, V, TS = Instant> {
  inner: RwLock<HashMap<K, Value<V, TS>>>,
}

impl<K, V> TimedMap<K, V> {
  /// Create a new instance of [`TimedMap`] with the default
  /// [`TimeSource`] implementation [`Instant`].
  pub fn new() -> Self {
    Self::new_with_timesource()
  }
}

impl<K, V, TS> TimedMap<K, V, TS> {
  /// Create a new instance of [`TimedMap`] with a custom
  /// [`TimeSource`] implementation.
  pub fn new_with_timesource() -> Self {
    Self {
      inner: RwLock::new(HashMap::new()),
    }
  }
}

impl<K, V, TS> TimedMap<K, V, TS>
where
  K: Eq + PartialEq + Hash + Clone,
  V: Clone,
  TS: TimeSource,
{
  /// Add a new key-value pair to the map with the
  /// given lifetime.
  ///
  /// When the lifetime has passed, the key-value pair
  /// will be no more accessible.
  pub async fn insert(&self, key: K, value: V, lifetime: Duration) {
    let mut m = self.inner.write().await;
    m.insert(key, Value::new(value, lifetime));
  }

  /// Returns a copy of the value corresponding to the
  /// given key.
  ///
  /// [`None`] is returned when the values lifetime has
  /// been passed.
  ///
  /// # Behavior
  ///
  /// If the key-value pair has expired and not been
  /// cleaned up before, it will be removed from the
  /// map on next retrival try.
  pub async fn get(&self, key: &K) -> Option<V> {
    self.get_value(key).await.map(|v| v.value())
  }

  /// Returns `true` when the map contains a non-expired
  /// value for the given key.
  ///
  /// # Behavior
  ///
  /// Because this method is basically a shorthand for
  /// [get(key).is_some()](#method.get), it behaves the
  /// same on retrival of expired pairs.
  pub async fn contains(&self, key: &K) -> bool {
    self.get(key).await.is_some()
  }

  /// Removes the given key-value pair from the map and
  /// returns the value if it was previously in the map
  /// and is not expired.
  pub async fn remove<Q>(&self, key: &Q) -> Option<V>
  where
    K: Borrow<Q>,
    Q: Hash + Eq + ?Sized,
  {
    let mut m = self.inner.write().await;
    m.remove(key).and_then(|v| v.value_checked())
  }

  /// Sets the lifetime of the value coresponding to the
  /// given key to the new lifetime from now.
  ///
  /// Returns `true` if a non-expired value exists for the
  /// given key.
  pub async fn refresh(&self, key: &K, new_lifetime: Duration) -> bool {
    let Some(mut v) = self.get_value(key).await else {
      return false;
    };

    let mut m = self.inner.write().await;
    v.set_expiry(new_lifetime);
    m.insert(key.clone(), v);

    true
  }

  /// Extends the lifetime of the value coresponding to the
  /// given key to the new lifetime from now.
  ///
  /// Returns `true` if a non-expired value exists for the
  /// given key.
  pub async fn extend(&self, key: &K, added_lifetime: Duration) -> bool {
    let Some(mut v) = self.get_value(key).await else {
      return false;
    };

    let mut m = self.inner.write().await;
    v.add_expiry(added_lifetime);
    m.insert(key.clone(), v);

    true
  }

  /// Returns the number of key-value pairs in the map
  /// which have not been expired.
  pub async fn len(&self) -> usize {
    let m = self.inner.read().await;
    m.iter().filter(|(_, v)| !v.is_expired()).count()
  }

  /// Returns `true` when the map does not contain any
  /// non-expired key-value pair.
  pub async fn is_empty(&self) -> bool {
    let m = self.inner.read().await;
    m.len() == 0
  }

  /// Clears the map, removing all key-value pairs.
  pub async fn clear(&self) {
    let mut m = self.inner.write().await;
    m.clear();
  }

  /// Create a snapshot of the current state of the maps
  /// key-value entries.
  ///
  /// It does only contain all non-expired key-value pairs.
  pub async fn snapshot<B: FromIterator<(K, V)>>(&self) -> B {
    self
      .inner
      .read()
      .await
      .iter()
      .filter(|(_, v)| !v.is_expired())
      .map(|(k, v)| (k.clone(), v.value()))
      .collect()
  }

  /// Retrieves the raw [`Value`] wrapper by the given key if
  /// the key-value pair has not been expired yet.
  ///
  /// If the given key-value pair is expired and not cleaned
  /// up yet, it will be removed from the map automatically.
  pub async fn get_value<Q>(&self, key: &Q) -> Option<Value<V, TS>>
  where
    K: Borrow<Q>,
    Q: Hash + Eq + ?Sized,
  {
    let v = self.get_value_unchecked(key).await?;
    if v.is_expired() {
      self.remove(key).await;
      return None;
    }
    Some(v)
  }

  /// Retrieves the raw [`Value`] wrapper by the given key
  /// without checking expiry.
  pub async fn get_value_unchecked<Q>(&self, key: &Q) -> Option<Value<V, TS>>
  where
    K: Borrow<Q>,
    Q: Hash + Eq + ?Sized,
  {
    let m = self.inner.read().await;
    m.get(key).cloned()
  }
}

#[async_trait]
impl<K, V, TS> Cleanup for TimedMap<K, V, TS>
where
  K: Eq + PartialEq + Hash + Clone + Send + Sync,
  V: Clone + Send + Sync,
  TS: TimeSource + Send + Sync,
{
  async fn cleanup(&self) {
    let now = TS::now();

    let mut keys = vec![];
    {
      let m = self.inner.read().await;
      keys.extend(
        m.iter()
          .filter(|(_, val)| val.is_expired_at(&now))
          .map(|(key, _)| key)
          .cloned(),
      );
    }

    if keys.is_empty() {
      return;
    }

    let mut m = self.inner.write().await;
    for key in keys {
      m.remove(&key);
    }

    // TODO: Maybe shrink the map down if it exceeds a predefined
    // capacity, like
    // if m.capacity() > SOME_CAP_VAL {
    //     m.shrink_to_fit();
    // }
  }
}

impl<K, V> Default for TimedMap<K, V> {
  fn default() -> Self {
    Self {
      inner: Default::default(),
    }
  }
}
