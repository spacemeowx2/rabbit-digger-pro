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

    #[test]
    fn test_rhai_builder() {
        let result = Rhai::build(Rhai {});
        assert!(result.is_ok());
    }

    #[test]
    fn test_rhai_into_dyn() {
        let rhai = Rhai::build(Rhai {}).unwrap();
        let _ = rhai.into_dyn();
    }

    #[tokio::test]
    async fn test_rhai_process_error() {
        let cache = crate::storage::MemoryCache::new().await.unwrap();
        let mut config = rabbit_digger::Config::default();
        let invalid_script = "this is not valid rhai";

        let mut rhai = Rhai::build(Rhai {}).unwrap();
        let result = rhai.process(&mut config, invalid_script, &cache).await;
        assert!(result.is_err());
    }
}
