pub use self::{importer::get_importer_registry, manager::ConfigManager, select_map::SelectMap};
use anyhow::{anyhow, Context, Result};
use futures::{Future, StreamExt};
use notify_stream::{notify::RecursiveMode, notify_stream};
use rabbit_digger::Config;
use rd_interface::{
    prelude::*,
    rd_config,
    schemars::{schema::SchemaObject, schema_for},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    future::pending,
    path::PathBuf,
    time::{Duration, SystemTime},
};
use tokio::{fs::read_to_string, sync::OnceCell, time::sleep};

use crate::{
    storage::{FileStorage, FolderType, Storage},
    util::DebounceStreamExt,
};

mod importer;
mod manager;
mod select_map;

static CONFIG_STORAGE: OnceCell<FileStorage> = OnceCell::const_new();
const POLL_VISIT_PREFIX: &str = "poll_visit";

#[rd_config]
#[derive(Debug, Clone)]
pub struct ImportUrl {
    pub url: String,
    pub interval: Option<u64>,
}

#[rd_config]
#[derive(Debug, Clone)]
pub struct ImportStorage {
    pub folder: String,
    pub key: String,
}

#[rd_config]
#[derive(Debug, Clone)]
#[serde(rename_all = "lowercase")]
pub enum ImportSource {
    Path(PathBuf),
    Poll(ImportUrl),
    Storage(ImportStorage),
    Text(String),
}

async fn config_storage() -> &'static FileStorage {
    CONFIG_STORAGE
        .get_or_init(|| async {
            FileStorage::new(FolderType::Cache, POLL_VISIT_PREFIX)
                .await
                .unwrap()
        })
        .await
}

async fn fetch(url: &str) -> Result<String> {
    let content = reqwest::get(url)
        .await
        .context("reqwest::get")?
        .text()
        .await
        .context("text")?;

    Ok(content)
}

async fn retry<F, Fut, E, R>(times: usize, f: F) -> Result<R, E>
where
    Fut: Future<Output = Result<R, E>>,
    F: Fn() -> Fut,
    E: std::fmt::Debug,
{
    let mut last_err = match f().await {
        Ok(r) => return Ok(r),
        Err(e) => e,
    };
    for i in 1..times {
        tracing::debug!("retry {}: {:?}", i, last_err);
        last_err = match f().await {
            Ok(r) => return Ok(r),
            Err(e) => e,
        }
    }

    Err(last_err)
}

async fn read_from_path(path: impl AsRef<std::path::Path>) -> Result<String> {
    let content = read_to_string(path).await?;

    // Remove BOM
    if content.starts_with('\u{feff}') {
        return Ok(content[3..].to_string());
    }

    Ok(content)
}

impl ImportSource {
    pub fn new_path(path: PathBuf) -> Self {
        ImportSource::Path(path)
    }
    pub fn new_poll(url: String, interval: Option<u64>) -> Self {
        ImportSource::Poll(ImportUrl { url, interval })
    }
    pub fn cache_key(&self) -> String {
        match self {
            ImportSource::Path(path) => format!("path:{path:?}"),
            ImportSource::Poll(url) => format!("poll:{}", url.url),
            ImportSource::Storage(storage) => format!("storage:{}:{}", storage.folder, storage.key),
            ImportSource::Text(_) => "text".to_string(),
        }
    }
    pub async fn get_content(&self, cache: &dyn Storage) -> Result<String> {
        let key = self.cache_key();
        let content = cache.get(&key).await?;

        if let Some(content) = content.and_then(|c| {
            // Only use cached content if it's not empty and not expired
            if c.content.is_empty() {
                None
            } else {
                self.get_expire_duration()
                    .map(|d| SystemTime::now() < c.updated_at + d)
                    .unwrap_or(true)
                    .then_some(c.content)
            }
        }) {
            return Ok(content);
        }

        Ok(match self {
            ImportSource::Path(path) => read_from_path(path).await?,
            ImportSource::Poll(ImportUrl { url, .. }) => {
                tracing::info!("Fetching {}", url);
                let content = match retry(3, || fetch(url)).await {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::warn!("Failed to fetch {}: {:?}, try to use cache", url, e);
                        // Don't set cache, let it expired
                        return Ok(cache.get(&key).await?.ok_or(e)?.content);
                    }
                };
                tracing::info!("Done");
                // Only cache non-empty content to avoid caching error responses
                if !content.is_empty() {
                    cache.set(&key, &content).await?;
                }
                content
            }
            ImportSource::Storage(ImportStorage { folder, key }) => {
                let storage = FileStorage::new(FolderType::Data, folder).await?;
                let item = storage
                    .get(key)
                    .await?
                    .ok_or_else(|| anyhow!("Not found"))?;
                item.content
            }
            ImportSource::Text(content) => content.to_string(),
        })
    }
    fn get_expire_duration(&self) -> Option<Duration> {
        match self {
            ImportSource::Path(_) => None,
            ImportSource::Poll(ImportUrl { interval, .. }) => interval.map(Duration::from_secs),
            ImportSource::Storage(_) => None,
            ImportSource::Text(_) => None,
        }
    }
    pub async fn wait(&self, cache: &dyn Storage) -> Result<()> {
        match self {
            ImportSource::Path(path) => {
                let mut stream = notify_stream(path, RecursiveMode::NonRecursive)?
                    .debounce(Duration::from_millis(100));
                stream.next().await;
            }
            ImportSource::Poll(ImportUrl { interval, .. }) => {
                let visited_at = config_storage()
                    .await
                    .get_updated_at(&self.cache_key())
                    .await
                    .unwrap();
                let updated_at = cache.get_updated_at(&self.cache_key()).await?;
                let time = match (visited_at, updated_at) {
                    (Some(a), Some(b)) => Some(a.max(b)),
                    (Some(t), None) | (None, Some(t)) => Some(t),
                    _ => None,
                };
                match (time, interval) {
                    (None, _) => {}
                    (Some(_), None) => pending().await,
                    (Some(time), Some(interval)) => {
                        let expired_at = time + Duration::from_secs(*interval);
                        let tts = expired_at
                            .duration_since(SystemTime::now())
                            .unwrap_or(Duration::ZERO);
                        sleep(tts).await
                    }
                }
            }
            ImportSource::Storage(ImportStorage { folder, key }) => {
                let storage = FileStorage::new(FolderType::Data, folder).await?;
                let path = storage
                    .get_path(key)
                    .await?
                    .ok_or_else(|| anyhow!("Not found"))?;

                let mut stream = notify_stream(path, RecursiveMode::NonRecursive)?
                    .debounce(Duration::from_millis(100));
                stream.next().await;
            }
            ImportSource::Text(_) => {
                pending::<()>().await;
            }
        };
        Ok(())
    }
}

#[rd_config]
#[derive(Debug, Clone)]
pub struct Import {
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub format: String,
    pub(super) source: ImportSource,
    #[serde(flatten)]
    pub opt: Value,
}

impl Import {
    // Append fields other than opt to a schema
    pub(crate) fn append_schema(mut schema: SchemaObject) -> SchemaObject {
        let properties = &mut schema.object().properties;
        properties.insert(
            "name".to_string(),
            schema_for!(Option<String>).schema.into(),
        );
        properties.insert(
            "source".to_string(),
            schema_for!(ImportSource).schema.into(),
        );
        schema.object().required.insert("source".to_string());
        schema
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct ConfigImport {
    #[serde(default)]
    import: Vec<Import>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ConfigExt {
    #[serde(flatten)]
    config: Config,
    #[serde(default, with = "serde_yaml::with::singleton_map_recursive")]
    import: Vec<Import>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_import_source_new_path() {
        let path = PathBuf::from("/test/path");
        let source = ImportSource::new_path(path.clone());
        assert!(matches!(source, ImportSource::Path(p) if p == path));
    }

    #[test]
    fn test_import_source_new_poll() {
        let url = "https://example.com".to_string();
        let interval = Some(60);
        let source = ImportSource::new_poll(url.clone(), interval);
        assert!(matches!(source, ImportSource::Poll(p) if p.url == url && p.interval == interval));
    }

    #[test]
    fn test_import_source_cache_key() {
        let path_source = ImportSource::Path(PathBuf::from("/test"));
        assert!(path_source.cache_key().contains("path:"));

        let poll_source = ImportSource::Poll(ImportUrl {
            url: "https://example.com".to_string(),
            interval: None,
        });
        assert_eq!(poll_source.cache_key(), "poll:https://example.com");

        let storage_source = ImportSource::Storage(ImportStorage {
            folder: "test".to_string(),
            key: "key1".to_string(),
        });
        assert_eq!(storage_source.cache_key(), "storage:test:key1");

        let text_source = ImportSource::Text("test".to_string());
        assert_eq!(text_source.cache_key(), "text");
    }

    #[test]
    fn test_import_source_get_expire_duration() {
        let path_source = ImportSource::Path(PathBuf::from("/test"));
        assert!(path_source.get_expire_duration().is_none());

        let poll_source = ImportSource::Poll(ImportUrl {
            url: "https://example.com".to_string(),
            interval: Some(60),
        });
        assert_eq!(
            poll_source.get_expire_duration(),
            Some(Duration::from_secs(60))
        );

        let poll_source_none = ImportSource::Poll(ImportUrl {
            url: "https://example.com".to_string(),
            interval: None,
        });
        assert!(poll_source_none.get_expire_duration().is_none());

        let text_source = ImportSource::Text("test".to_string());
        assert!(text_source.get_expire_duration().is_none());
    }

    #[test]
    fn test_config_import_serialize() {
        let config_import = ConfigImport { import: vec![] };
        let yaml = serde_yaml::to_string(&config_import);
        assert!(yaml.is_ok());
    }

    #[test]
    fn test_config_import_deserialize() {
        let yaml = "import: []";
        let result = serde_yaml::from_str::<ConfigImport>(yaml);
        assert!(result.is_ok());
    }
}
