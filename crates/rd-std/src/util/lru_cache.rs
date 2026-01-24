use std::{
    ops::{Deref, DerefMut},
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use futures::FutureExt;
use lru_time_cache::LruCache as LruTimeCache;
use tokio::time::{sleep, Instant, Sleep};

/// A LruCache with a time-to-live expiration.
pub struct LruCache<Key, Value> {
    cache: LruTimeCache<Key, Value>,
    ttl: Duration,
    sleep: Pin<Box<Sleep>>,
}

impl<Key, Value> LruCache<Key, Value>
where
    Key: Ord + Clone,
{
    /// Constructor for time based `LruCache`.
    pub fn with_expiry_duration(time_to_live: Duration) -> LruCache<Key, Value> {
        LruCache {
            cache: LruTimeCache::with_expiry_duration(time_to_live),
            ttl: time_to_live,
            sleep: Box::pin(sleep(time_to_live)),
        }
    }

    /// Constructor for dual-feature capacity and time based `LruCache`.
    pub fn with_expiry_duration_and_capacity(
        time_to_live: Duration,
        capacity: usize,
    ) -> LruCache<Key, Value> {
        LruCache {
            cache: LruTimeCache::with_expiry_duration_and_capacity(time_to_live, capacity),
            ttl: time_to_live,
            sleep: Box::pin(sleep(time_to_live)),
        }
    }

    pub fn poll_clear_expired(&mut self, cx: &mut Context<'_>) {
        loop {
            match self.sleep.poll_unpin(cx) {
                Poll::Pending => return,
                Poll::Ready(_) => {
                    self.sleep.as_mut().reset(Instant::now() + self.ttl);

                    // removes expired connections
                    self.cache.iter();
                }
            }
        }
    }
}

impl<Key, Value> Deref for LruCache<Key, Value> {
    type Target = LruTimeCache<Key, Value>;

    fn deref(&self) -> &Self::Target {
        &self.cache
    }
}

impl<Key, Value> DerefMut for LruCache<Key, Value> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.cache
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use futures::future::poll_fn;
    use rd_interface::Arc;

    use super::*;

    #[tokio::test]
    async fn test_lru_cache() {
        let mut cache = LruCache::with_expiry_duration(Duration::from_millis(100));
        cache.insert("key1", "value1");
        cache.insert("key2", "value2");

        assert_eq!(cache.get("key1"), Some(&"value1"));
        assert_eq!(cache.get("key2"), Some(&"value2"));

        sleep(Duration::from_millis(200)).await;

        assert_eq!(cache.get("key1"), None);
        assert_eq!(cache.get("key2"), None);
    }

    #[tokio::test]
    async fn test_lru_cache_with_capacity() {
        let mut cache = LruCache::with_expiry_duration_and_capacity(Duration::from_millis(100), 2);
        cache.insert("key1", "value1");
        cache.insert("key2", "value2");
        cache.insert("key3", "value3");

        assert_eq!(cache.get("key1"), None);
        assert_eq!(cache.get("key2"), Some(&"value2"));
        assert_eq!(cache.get("key3"), Some(&"value3"));
    }

    #[tokio::test]
    async fn test_lru_cache_with_expiry_duration() {
        let mut cache = LruCache::with_expiry_duration_and_capacity(Duration::from_millis(100), 2);

        struct Value(Arc<AtomicBool>);
        impl Drop for Value {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Relaxed);
            }
        }

        let dropped = Arc::new(AtomicBool::new(false));

        cache.insert("key1", Value(dropped.clone()));

        assert_eq!(dropped.load(Ordering::Relaxed), false);

        sleep(Duration::from_millis(200)).await;
        poll_fn(|cx| {
            cache.poll_clear_expired(cx);
            Poll::Ready(())
        })
        .await;

        assert_eq!(dropped.load(Ordering::Relaxed), true);
    }
}
