use std::sync::Arc;

use crate::{
    deserialize_config,
    storage::{FileStorage, FolderType, Storage},
};

use super::{importer::get_importer, select_map::SelectMap, Import, ImportSource};
use anyhow::{Context, Result};
use async_stream::stream;
use futures::{stream::FuturesUnordered, Stream, StreamExt};
use rabbit_digger::Config;
use tokio::select;

const CFG_MGR_PREFIX: &str = "cfg_mgr";
const SELECT_PREFIX: &str = "select";

struct Inner {
    file_cache: FileStorage,
    select_storage: FileStorage,
}

#[derive(Clone)]
pub struct ConfigManager {
    inner: Arc<Inner>,
}

impl ConfigManager {
    pub async fn new() -> Result<Self> {
        let file_cache = FileStorage::new(FolderType::Cache, CFG_MGR_PREFIX).await?;
        let select_storage = FileStorage::new(FolderType::Data, SELECT_PREFIX).await?;

        let mgr = ConfigManager {
            inner: Arc::new(Inner {
                file_cache,
                select_storage,
            }),
        };

        Ok(mgr)
    }
    pub async fn config_stream(
        &self,
        source: ImportSource,
    ) -> Result<impl Stream<Item = Result<Config>>> {
        let inner = self.inner.clone();

        Ok(stream! {
            loop {
                let (config, import) = inner.deserialize_config_from_source(&source).await?;
                yield Ok(config);
                inner.wait_source(&source, &import).await?;
            }
        })
    }
    pub async fn config_stream_from_sources(
        &self,
        sources: impl Stream<Item = ImportSource>,
    ) -> Result<impl Stream<Item = Result<Config>>> {
        let inner = self.inner.clone();
        let mut sources = Box::pin(sources);
        let mut source = match sources.next().await {
            Some(s) => s,
            None => return Err(anyhow::anyhow!("no source")),
        };

        Ok(stream! {
            loop {
                let (config, import) = inner.deserialize_config_from_source(&source).await?;
                yield Ok(config);
                let r = select! {
                    r = inner.wait_source(&source, &import) => r,
                    r = sources.next() => {
                        source = match r {
                            Some(s) => s,
                            None => break,
                        };
                        Ok(())
                    }
                };
                r?;
            }
        })
    }
    pub fn select_storage(&self) -> &dyn Storage {
        &self.inner.select_storage
    }
}

impl Import {
    async fn apply(&self, config: &mut Config, cache: &dyn Storage) -> Result<()> {
        let mut importer = get_importer(self)?;
        let content = self.source.get_content(cache).await?;
        importer.process(config, &content, cache).await?;
        Ok(())
    }
}

impl Inner {
    async fn deserialize_config_from_source(
        &self,
        source: &ImportSource,
    ) -> Result<(Config, Vec<Import>)> {
        let mut config = deserialize_config(&source.get_content(&self.file_cache).await?)?;
        config.config.id = source.cache_key();

        let imports = config.import;

        for i in &imports {
            i.apply(&mut config.config, &self.file_cache)
                .await
                .context(format!("applying import: {i:?}"))?;
        }
        let mut config = config.config;

        // restore patch
        SelectMap::from_cache(&config.id, &self.select_storage)
            .await?
            .apply_config(&mut config)
            .await;

        Ok((config, imports))
    }

    async fn wait_source(&self, cfg_src: &ImportSource, imports: &[Import]) -> Result<()> {
        let mut events = FuturesUnordered::new();
        events.push(cfg_src.wait(&self.file_cache));
        for i in imports {
            events.push(i.source.wait(&self.file_cache));
        }
        events.next().await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_config_manager_new() {
        let result = ConfigManager::new().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_config_manager_clone() {
        let mgr = ConfigManager::new().await.unwrap();
        let _ = mgr.clone();
    }
}
