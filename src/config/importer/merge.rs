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

    #[test]
    fn test_merge_builder() {
        let result = Merge::build(EmptyConfig::default());
        assert!(result.is_ok());
    }

    #[test]
    fn test_merge_into_dyn() {
        let merge = Merge::build(EmptyConfig::default()).unwrap();
        let _ = merge.into_dyn();
    }

    #[tokio::test]
    async fn test_merge_process() {
        let cache = crate::storage::MemoryCache::new().await.unwrap();
        let mut config = rabbit_digger::Config::default();
        let content = r#"
net:
  test_net:
    type: direct
"#;

        let mut merge = Merge::build(EmptyConfig::default()).unwrap();
        let result = merge.process(&mut config, content, &cache).await;
        assert!(result.is_ok());
        assert!(config.net.contains_key("test_net"));
    }
}
