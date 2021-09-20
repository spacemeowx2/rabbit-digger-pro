use std::{collections::HashMap, fs::File, path::PathBuf, time::SystemTime};

use anyhow::{Context, Result};
use dirs::cache_dir;
use fs2::FileExt;
use parking_lot::RwLock;
use rd_interface::async_trait;
use serde::{Deserialize, Serialize};
use tokio::{
    fs::{create_dir_all, read_to_string, write},
    task::spawn_blocking,
};
use uuid::Uuid;

const CACHE_DIR: &str = "rabbit_digger_pro";

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CacheItem {
    pub updated_at: SystemTime,
    pub content: String,
}
#[async_trait]
pub trait ConfigCache: Send + Sync {
    async fn get_updated_at(&self, key: &str) -> Result<Option<SystemTime>>;
    async fn get(&self, key: &str) -> Result<Option<CacheItem>>;
    async fn set(&self, key: &str, value: &str) -> Result<()>;
    async fn keys(&self) -> Result<Vec<String>>;
}

pub struct MemoryCache {
    cache: RwLock<HashMap<String, CacheItem>>,
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
impl ConfigCache for MemoryCache {
    async fn get_updated_at(&self, key: &str) -> Result<Option<SystemTime>> {
        Ok(self.cache.read().get(key).map(|item| item.updated_at))
    }
    async fn get(&self, key: &str) -> Result<Option<CacheItem>> {
        Ok(self.cache.read().get(key).cloned())
    }
    async fn set(&self, key: &str, value: &str) -> Result<()> {
        self.cache.write().insert(
            key.to_string(),
            CacheItem {
                updated_at: SystemTime::now(),
                content: value.to_string(),
            },
        );
        Ok(())
    }
    async fn keys(&self) -> Result<Vec<String>> {
        Ok(self.cache.read().keys().map(|i| i.clone()).collect())
    }
}

pub struct FileCache {
    prefix: String,
    cache_dir: PathBuf,
    index_path: PathBuf,
}

impl FileCache {
    pub async fn new(prefix: impl Into<String>) -> Result<Self> {
        let cache_dir = cache_dir()
            .ok_or_else(|| anyhow::anyhow!("no cache dir"))?
            .join(CACHE_DIR);
        create_dir_all(&cache_dir)
            .await
            .context("Failed to create cache dir")?;
        let index_path = cache_dir.join("index.json");
        let cache = FileCache {
            prefix: prefix.into(),
            cache_dir,
            index_path: index_path.clone(),
        };
        if tokio::fs::metadata(index_path).await.is_err() {
            cache
                .set_index(Index {
                    version: 0,
                    index: HashMap::new(),
                })
                .await?;
        }

        Ok(cache)
    }
    async fn get_index(&self) -> Result<Index> {
        let index_path = self.index_path.clone();
        let index = spawn_blocking(move || {
            let file = File::open(&index_path).context("open cache index")?;
            file.lock_shared().context("lock cache index")?;
            let result = serde_json::from_reader(&file).context("deserial cache index");
            file.unlock().context("unlock cache index")?;
            Result::<Index>::Ok(result?)
        })
        .await??;
        Ok(index)
    }
    async fn set_index(&self, index: Index) -> Result<()> {
        let index_path = self.index_path.clone();
        spawn_blocking(move || {
            let file = File::create(&index_path).context("open cache index")?;
            file.lock_exclusive().context("lock cache index mut")?;
            let result = serde_json::to_writer(&file, &index).context("serialize cache index");
            file.unlock().context("unlock cache index")?;
            Result::<()>::Ok(result?)
        })
        .await??;
        Ok(())
    }
}

#[derive(Deserialize, Serialize)]
struct Index {
    version: u32,
    index: HashMap<String, CacheItem>,
}

#[async_trait]
impl ConfigCache for FileCache {
    async fn get_updated_at(&self, key: &str) -> Result<Option<SystemTime>> {
        let index = self.get_index().await?;
        Ok(index
            .index
            .get(&format!("{}{}", self.prefix, key))
            .map(|item| item.updated_at))
    }
    async fn get(&self, key: &str) -> Result<Option<CacheItem>> {
        let index = self.get_index().await?;
        Ok(match index.index.get(&format!("{}{}", self.prefix, key)) {
            Some(item) => Some(CacheItem {
                updated_at: item.updated_at,
                content: read_to_string(self.cache_dir.join(&item.content)).await?,
            }),
            None => None,
        })
    }

    async fn set(&self, key: &str, value: &str) -> Result<()> {
        let key = format!("{}{}", self.prefix, key);
        let mut index = self.get_index().await?;

        let filename = index
            .index
            .get(&key)
            .map(|item| item.content.clone())
            .unwrap_or_else(|| Uuid::new_v4().to_string());

        write(self.cache_dir.join(&filename), value).await?;

        index.index.insert(
            key,
            CacheItem {
                updated_at: SystemTime::now(),
                content: filename,
            },
        );
        self.set_index(index).await?;

        Ok(())
    }
    async fn keys(&self) -> Result<Vec<String>> {
        let index = self.get_index().await?;
        Ok(index
            .index
            .keys()
            .filter(|i| i.starts_with(&self.prefix))
            .map(|i| i.clone())
            .collect())
    }
}