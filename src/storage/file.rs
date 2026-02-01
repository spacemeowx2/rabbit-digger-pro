use std::{
    collections::HashMap,
    fs::File,
    io::Write,
    path::{Path, PathBuf},
    time::SystemTime,
};

use anyhow::{Context, Result};
use dirs::{cache_dir, data_local_dir};
use fs2::FileExt;
use rd_interface::async_trait;
use serde::{Deserialize, Serialize};
use tokio::{
    fs::{create_dir_all, read_to_string, remove_file, write},
    sync::RwLock,
    task::spawn_blocking,
};
use uuid::Uuid;

pub use super::{Storage, StorageItem, StorageKey};

const PROGRAM_DIR: &str = "rabbit_digger_pro";

pub enum FolderType {
    Cache,
    Data,
}

impl FolderType {
    fn path(&self, folder: impl AsRef<Path>) -> Result<PathBuf> {
        Ok(match self {
            FolderType::Cache => cache_dir(),
            FolderType::Data => data_local_dir(),
        }
        .ok_or_else(|| anyhow::anyhow!("no cache dir"))?
        .join(PROGRAM_DIR)
        .join(folder))
    }
}

pub struct FileStorage {
    storage_dir: PathBuf,
    index_path: PathBuf,
    lock_path: PathBuf,
    lock: RwLock<()>,
}

impl FileStorage {
    pub async fn new(folder_type: FolderType, folder: impl AsRef<Path>) -> Result<Self> {
        let storage_dir = folder_type.path(folder)?;
        create_dir_all(&storage_dir)
            .await
            .context("Failed to create cache dir")?;
        let index_path = storage_dir.join("index.json");
        let lock_path = storage_dir.join("index.lock");
        File::create(&lock_path).context("create index lock")?;
        let cache = FileStorage {
            storage_dir,
            index_path: index_path.clone(),
            lock_path,
            lock: RwLock::new(()),
        };
        if tokio::fs::metadata(&index_path).await.is_ok() && cache.get_index().await.is_err() {
            tracing::warn!("Index file is corrupted, try to remove it");
            remove_file(&index_path)
                .await
                .context("Failed to remove index file")?;
        }
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

        Ok(index
            .index
            .get(key)
            .map(|item| self.storage_dir.join(&item.content)))
    }
    async fn get_index(&self) -> Result<Index> {
        let index_path = self.index_path.clone();
        let lock_path = self.lock_path.clone();
        let index = spawn_blocking(move || {
            let lock = File::open(&lock_path).context("open cache index")?;
            lock.lock_shared().context("lock cache index")?;
            let file = File::open(&index_path).context("open cache index")?;
            let result = serde_json::from_reader(&file).context("deserial cache index");
            lock.unlock().context("unlock cache index")?;
            result
        })
        .await??;
        Ok(index)
    }
    async fn set_index(&self, index: Index) -> Result<()> {
        let index_path = self.index_path.clone();
        let lock_path = self.lock_path.clone();
        spawn_blocking(move || {
            let mut lock = File::open(&lock_path).context("open cache index")?;
            lock.lock_exclusive().context("lock cache index mut")?;
            let file = File::create(&index_path).context("open cache index")?;
            let result = serde_json::to_writer(&file, &index).context("serialize cache index");
            lock.flush()?;
            lock.unlock().context("unlock cache index")?;
            result
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
        let _ = self.lock.read().await;
        let index = self.get_index().await?;
        Ok(index.index.get(key).map(|item| item.updated_at))
    }

    async fn get(&self, key: &str) -> Result<Option<StorageItem>> {
        let _ = self.lock.read().await;
        let index = self.get_index().await?;
        Ok(match index.index.get(key) {
            Some(item) => Some(StorageItem {
                updated_at: item.updated_at,
                content: read_to_string(self.storage_dir.join(&item.content)).await?,
            }),
            None => None,
        })
    }

    async fn set(&self, key: &str, value: &str) -> Result<()> {
        if value.is_empty() {
            return Ok(());
        }

        let _ = self.lock.write().await;
        let mut index = self.get_index().await?;

        let filename = index
            .index
            .get(key)
            .map(|item| item.content.clone())
            .unwrap_or_else(|| Uuid::new_v4().to_string());

        write(self.storage_dir.join(&filename), value).await?;

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
        let _ = self.lock.read().await;
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
        let _ = self.lock.write().await;
        let mut index = self.get_index().await?;

        let filename = index
            .index
            .get(key)
            .map(|item| item.content.clone())
            .unwrap_or_else(|| Uuid::new_v4().to_string());

        remove_file(self.storage_dir.join(&filename)).await.ok();

        index.index.remove(key);
        self.set_index(index).await?;

        Ok(())
    }

    async fn clear(&self) -> Result<()> {
        let _ = self.lock.write().await;
        let index = self.get_index().await?;
        for item in index.index.values() {
            remove_file(self.storage_dir.join(&item.content)).await.ok();
        }
        self.set_index(Index {
            version: 0,
            index: HashMap::new(),
        })
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_folder(prefix: &str) -> String {
        format!("{prefix}-{}", Uuid::new_v4())
    }

    #[tokio::test]
    async fn test_file_storage_set_get_remove_clear() {
        let folder = unique_folder("file_storage");
        let storage = FileStorage::new(FolderType::Cache, &folder).await.unwrap();

        // Empty value should be ignored.
        storage.set("k0", "").await.unwrap();
        assert!(storage.get("k0").await.unwrap().is_none());

        storage.set("k1", "v1").await.unwrap();
        let item = storage.get("k1").await.unwrap().unwrap();
        assert_eq!(item.content, "v1");

        let keys = storage.keys().await.unwrap();
        assert!(keys.iter().any(|k| k.key == "k1"));

        storage.remove("k1").await.unwrap();
        assert!(storage.get("k1").await.unwrap().is_none());

        storage.set("k2", "v2").await.unwrap();
        storage.clear().await.unwrap();
        assert!(storage.keys().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_file_storage_recovers_from_corrupted_index() {
        let folder = unique_folder("corrupt_index");
        let storage_dir = FolderType::Cache.path(&folder).unwrap();
        let index_path = storage_dir.join("index.json");

        // Initialize once.
        let _ = FileStorage::new(FolderType::Cache, &folder).await.unwrap();

        // Corrupt the index.
        tokio::fs::write(&index_path, b"not-json").await.unwrap();

        // Should recover by deleting and recreating the index.
        let storage = FileStorage::new(FolderType::Cache, &folder).await.unwrap();
        assert!(storage.get_index().await.is_ok());
    }
}
