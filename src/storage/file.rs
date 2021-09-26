use std::{
    collections::HashMap,
    fs::File,
    path::{Path, PathBuf},
    time::SystemTime,
};

use anyhow::{Context, Result};
use dirs::cache_dir;
use fs2::FileExt;
use rd_interface::async_trait;
use serde::{Deserialize, Serialize};
use tokio::{
    fs::{create_dir_all, read_to_string, remove_file, write},
    task::spawn_blocking,
};
use uuid::Uuid;

pub use super::{Storage, StorageItem, StorageKey};

const CACHE_DIR: &str = "rabbit_digger_pro";

pub struct FileStorage {
    cache_dir: PathBuf,
    index_path: PathBuf,
}

impl FileStorage {
    pub async fn new(folder: impl AsRef<Path>) -> Result<Self> {
        let cache_dir = cache_dir()
            .ok_or_else(|| anyhow::anyhow!("no cache dir"))?
            .join(CACHE_DIR)
            .join(folder);
        create_dir_all(&cache_dir)
            .await
            .context("Failed to create cache dir")?;
        let index_path = cache_dir.join("index.json");
        let cache = FileStorage {
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
    pub async fn get_path(&self, key: &str) -> Result<Option<PathBuf>> {
        let index = self.get_index().await?;

        Ok(match index.index.get(key) {
            Some(item) => Some(self.cache_dir.join(&item.content)),
            None => None,
        })
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
    index: HashMap<String, StorageItem>,
}

#[async_trait]
impl Storage for FileStorage {
    async fn get_updated_at(&self, key: &str) -> Result<Option<SystemTime>> {
        let index = self.get_index().await?;
        Ok(index.index.get(key).map(|item| item.updated_at))
    }
    async fn get(&self, key: &str) -> Result<Option<StorageItem>> {
        let index = self.get_index().await?;
        Ok(match index.index.get(key) {
            Some(item) => Some(StorageItem {
                updated_at: item.updated_at,
                content: read_to_string(self.cache_dir.join(&item.content)).await?,
            }),
            None => None,
        })
    }

    async fn set(&self, key: &str, value: &str) -> Result<()> {
        let mut index = self.get_index().await?;

        let filename = index
            .index
            .get(key)
            .map(|item| item.content.clone())
            .unwrap_or_else(|| Uuid::new_v4().to_string());

        write(self.cache_dir.join(&filename), value).await?;

        index.index.insert(
            key.to_string(),
            StorageItem {
                updated_at: SystemTime::now(),
                content: filename,
            },
        );
        self.set_index(index).await?;

        Ok(())
    }
    async fn keys(&self) -> Result<Vec<StorageKey>> {
        let index = self.get_index().await?;
        Ok(index
            .index
            .into_iter()
            .map(|(key, i)| StorageKey {
                updated_at: i.updated_at,
                key,
            })
            .collect())
    }

    async fn remove(&self, key: &str) -> Result<()> {
        let mut index = self.get_index().await?;

        let filename = index
            .index
            .get(key)
            .map(|item| item.content.clone())
            .unwrap_or_else(|| Uuid::new_v4().to_string());

        remove_file(self.cache_dir.join(&filename)).await.ok();

        index.index.remove(key);
        self.set_index(index).await?;

        Ok(())
    }
}