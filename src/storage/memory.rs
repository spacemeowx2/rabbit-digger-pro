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

    #[tokio::test]
    async fn test_memory_cache_roundtrip() {
        let cache = MemoryCache::new().await.unwrap();
        assert!(cache.get("k").await.unwrap().is_none());

        cache.set("k", "v").await.unwrap();
        let item = cache.get("k").await.unwrap().unwrap();
        assert_eq!(item.content, "v");

        let keys = cache.keys().await.unwrap();
        assert!(keys.iter().any(|i| i.key == "k"));

        cache.remove("k").await.unwrap();
        assert!(cache.get("k").await.unwrap().is_none());

        cache.set("k2", "v2").await.unwrap();
        cache.clear().await.unwrap();
        assert!(cache.keys().await.unwrap().is_empty());
    }
}
