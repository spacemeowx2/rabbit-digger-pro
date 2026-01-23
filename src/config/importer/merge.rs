use anyhow::Result;
use rabbit_digger::Config;
use rd_interface::{async_trait, config::EmptyConfig, registry::Builder, IntoDyn};

use crate::storage::Storage;

use super::{BoxImporter, Importer};

#[derive(Debug)]
pub struct Merge;

#[async_trait]
impl Importer for Merge {
    async fn process(
        &mut self,
        config: &mut Config,
        content: &str,
        _cache: &dyn Storage,
    ) -> Result<()> {
        let other_content: Config = serde_yaml::from_str(content)?;
        config.merge(other_content);
        Ok(())
    }
}

impl Builder<BoxImporter> for Merge {
    const NAME: &'static str = "merge";

    type Config = EmptyConfig;

    type Item = Merge;

    fn build(_config: Self::Config) -> rd_interface::Result<Self::Item> {
        Ok(Merge)
    }
}

impl IntoDyn<BoxImporter> for Merge {
    fn into_dyn(self) -> BoxImporter {
        Box::new(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_merge_importer_merges_config() {
        let mut base = rabbit_digger::config::Config::default();
        base.id = "base".to_string();

        let other = r#"
id: other
net:
  n1:
    type: alias
    net: local
server: {}
    "#;

        let cache = crate::storage::MemoryCache::new().await.unwrap();
        let mut importer = Merge;
        importer.process(&mut base, other, &cache).await.unwrap();

        assert!(base.net.contains_key("n1"));
    }

    #[test]
    fn test_merge_builder_and_into_dyn() {
        let m =
            <Merge as rd_interface::registry::Builder<BoxImporter>>::build(EmptyConfig::default())
                .unwrap();
        let _dyn_imp = m.into_dyn();
    }
}
