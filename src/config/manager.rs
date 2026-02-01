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
    use futures::StreamExt;
    use std::time::Duration;

    fn minimal_yaml() -> String {
        r#"
id: test
net: {}
server: {}
"#
        .to_string()
    }

    #[tokio::test]
    async fn test_config_stream_yields_config_and_sets_id_from_cache_key() {
        let mgr = ConfigManager::new().await.unwrap();
        let s = mgr
            .config_stream(ImportSource::Text(minimal_yaml()))
            .await
            .unwrap();
        futures::pin_mut!(s);

        let cfg = s.next().await.unwrap().unwrap();
        assert_eq!(cfg.id, "text");
    }

    #[tokio::test]
    async fn test_config_stream_from_sources_no_source_is_error() {
        let mgr = ConfigManager::new().await.unwrap();
        let r = mgr
            .config_stream_from_sources(futures::stream::empty())
            .await;
        assert!(r.is_err());
    }

    #[tokio::test]
    async fn test_inner_wait_source_poll_returns_immediately_when_no_timestamps() {
        let mgr = ConfigManager::new().await.unwrap();
        let source = ImportSource::new_poll("http://example.invalid".to_string(), Some(0));

        let t = tokio::time::timeout(Duration::from_secs(1), mgr.inner.wait_source(&source, &[]))
            .await
            .unwrap();
        t.unwrap();
    }

    #[tokio::test]
    async fn test_deserialize_config_applies_merge_import() {
        let mgr = ConfigManager::new().await.unwrap();
        let source = ImportSource::Text(
            r#"
id: base
net: {}
server: {}
import:
  - type: merge
    source:
      text: |
        id: other
        net:
          n1:
            type: alias
            net: local
        server: {}
"#
            .to_string(),
        );

        let (cfg, _imports) = mgr
            .inner
            .deserialize_config_from_source(&source)
            .await
            .unwrap();
        assert!(cfg.net.contains_key("n1"));
    }

    #[tokio::test]
    async fn test_deserialize_config_import_error_has_context() {
        let mgr = ConfigManager::new().await.unwrap();
        let source = ImportSource::Text(
            r#"
id: base
net: {}
server: {}
import:
  - type: merge
    source:
      text: "not-yaml: ["
"#
            .to_string(),
        );

        let err = mgr
            .inner
            .deserialize_config_from_source(&source)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("applying import"));
    }
}
