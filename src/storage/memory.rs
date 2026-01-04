use std::{collections::HashMap, time::SystemTime};

use super::{Storage, StorageItem, StorageKey};
use anyhow::Result;
use parking_lot::RwLock;
use rd_interface::async_trait;

pub struct MemoryCache {
    cache: RwLock<HashMap<String, StorageItem>>,
}

impl MemoryCache {
    #[allow(dead_code)]
    pub async fn new() -> Result<Self> {
        Ok(MemoryCache {
            cache: RwLock::new(HashMap::new()),
        })
    }
}

#[async_trait]
impl Storage for MemoryCache {
    async fn get_updated_at(&self, key: &str) -> Result<Option<SystemTime>> {
        Ok(self.cache.read().get(key).map(|item| item.updated_at))
    }

    async fn get(&self, key: &str) -> Result<Option<StorageItem>> {
        Ok(self.cache.read().get(key).cloned())
    }

    async fn set(&self, key: &str, value: &str) -> Result<()> {
        self.cache.write().insert(
            key.to_string(),
            StorageItem {
                updated_at: SystemTime::now(),
                content: value.to_string(),
            },
        );
        Ok(())
    }

    async fn keys(&self) -> Result<Vec<StorageKey>> {
        Ok(self
            .cache
            .read()
            .iter()
            .map(|(key, i)| StorageKey {
                key: key.to_string(),
                updated_at: i.updated_at,
            })
            .collect())
    }

    async fn remove(&self, key: &str) -> Result<()> {
        self.cache.write().remove(key);
        Ok(())
    }

    async fn clear(&self) -> Result<()> {
        self.cache.write().clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    #[tokio::test]
    async fn test_memory_cache_set_get() {
        let cache = MemoryCache::new().await.unwrap();
        cache.set("key1", "value1").await.unwrap();
        let item = cache.get("key1").await.unwrap().unwrap();
        assert_eq!(item.content, "value1");
    }

    #[tokio::test]
    async fn test_memory_cache_get_nonexistent() {
        let cache = MemoryCache::new().await.unwrap();
        let result = cache.get("nonexistent").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_memory_cache_remove() {
        let cache = MemoryCache::new().await.unwrap();
        cache.set("key1", "value1").await.unwrap();
        assert!(cache.get("key1").await.unwrap().is_some());
        cache.remove("key1").await.unwrap();
        assert!(cache.get("key1").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_memory_cache_keys() {
        let cache = MemoryCache::new().await.unwrap();
        cache.set("key1", "value1").await.unwrap();
        cache.set("key2", "value2").await.unwrap();
        let keys = cache.keys().await.unwrap();
        assert_eq!(keys.len(), 2);
    }

    #[tokio::test]
    async fn test_memory_cache_clear() {
        let cache = MemoryCache::new().await.unwrap();
        cache.set("key1", "value1").await.unwrap();
        cache.set("key2", "value2").await.unwrap();
        cache.clear().await.unwrap();
        assert_eq!(cache.keys().await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn test_memory_cache_get_updated_at() {
        let cache = MemoryCache::new().await.unwrap();
        let before = SystemTime::now();
        cache.set("key1", "value1").await.unwrap();
        let after = SystemTime::now();
        let updated_at = cache.get_updated_at("key1").await.unwrap().unwrap();
        assert!(updated_at >= before && updated_at <= after);
    }
}
