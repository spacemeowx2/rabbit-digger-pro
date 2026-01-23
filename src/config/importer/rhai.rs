use anyhow::{anyhow, Result};
use rabbit_digger::Config;
use rd_interface::{async_trait, prelude::*, rd_config, registry::Builder, IntoDyn};
use rhai::{
    serde::{from_dynamic, to_dynamic},
    Engine, Scope,
};

use crate::storage::Storage;

use super::{BoxImporter, Importer};

#[rd_config]
#[derive(Debug)]
pub struct Rhai {}

#[async_trait]
impl Importer for Rhai {
    async fn process(
        &mut self,
        config: &mut Config,
        content: &str,
        _cache: &dyn Storage,
    ) -> Result<()> {
        let engine = Engine::new();
        let mut scope = Scope::new();
        let dyn_config = to_dynamic(&config).map_err(|e| anyhow!("to_dynamic err: {:?}", e))?;
        scope.push("config", dyn_config);

        engine
            .eval_with_scope(&mut scope, content)
            .map_err(|e| anyhow!("Failed to evaluate rhai: {:?}", e))?;

        if let Some(cfg) = scope.get_value("config") {
            *config = from_dynamic(&cfg).map_err(|e| anyhow!("from_dynamic err: {:?}", e))?;
        } else {
            return Err(anyhow!("Failed to get config from rhai"));
        }

        Ok(())
    }
}

impl Builder<BoxImporter> for Rhai {
    const NAME: &'static str = "rhai";

    type Config = Rhai;
    type Item = Rhai;

    fn build(_cfg: Self::Config) -> rd_interface::Result<Self::Item> {
        Ok(Rhai {})
    }
}

impl IntoDyn<BoxImporter> for Rhai {
    fn into_dyn(self) -> BoxImporter {
        Box::new(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_rhai_importer_can_modify_config() {
        let mut cfg = rabbit_digger::config::Config::default();
        cfg.id = "before".to_string();

        let cache = crate::storage::MemoryCache::new().await.unwrap();
        let mut importer = Rhai {};

        importer
            .process(&mut cfg, r#"config.id = "after";"#, &cache)
            .await
            .unwrap();

        assert_eq!(cfg.id, "after");
    }

    #[tokio::test]
    async fn test_rhai_importer_eval_error_is_reported() {
        let mut cfg = rabbit_digger::config::Config::default();
        let cache = crate::storage::MemoryCache::new().await.unwrap();
        let mut importer = Rhai {};

        let err = importer.process(&mut cfg, "this_is_invalid(", &cache).await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn test_rhai_importer_from_dynamic_error_is_reported() {
        let mut cfg = rabbit_digger::config::Config::default();
        let cache = crate::storage::MemoryCache::new().await.unwrap();
        let mut importer = Rhai {};

        let err = importer.process(&mut cfg, "config = 1;", &cache).await;
        assert!(err.is_err());
    }

    #[test]
    fn test_rhai_builder_and_into_dyn() {
        let r = <Rhai as rd_interface::registry::Builder<BoxImporter>>::build(Rhai {}).unwrap();
        let _dyn_imp = r.into_dyn();
    }
}
