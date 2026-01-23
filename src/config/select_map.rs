use std::collections::HashMap;

use anyhow::Result;
use rabbit_digger::Config;
use serde::{Deserialize, Serialize};

use crate::storage::Storage;

#[derive(Debug, Serialize, Deserialize)]
pub struct SelectMap(HashMap<String, String>);

impl SelectMap {
    pub async fn from_cache(id: &str, cache: &dyn Storage) -> Result<SelectMap> {
        let select_map = cache
            .get(id)
            .await?
            .map(|i| serde_json::from_str(&i.content).unwrap_or_default())
            .unwrap_or_default();
        Ok(SelectMap(select_map))
    }
    pub async fn write_cache(&self, id: &str, cache: &dyn Storage) -> Result<()> {
        cache.set(id, &serde_json::to_string(&self.0)?).await
    }
    pub async fn apply_config(&self, config: &mut Config) {
        for (net, selected) in &self.0 {
            if let Some(n) = config.net.get_mut(net) {
                if n.net_type == "select" {
                    if let Some(o) = n.opt.as_object_mut() {
                        if o.get("list")
                            .into_iter()
                            .filter_map(|v| v.as_array())
                            .flatten()
                            .flat_map(|v| v.as_str())
                            .any(|i| i == selected)
                        {
                            o.insert("selected".to_string(), selected.to_string().into());
                        } else {
                            tracing::info!("The selected({}/{}) in the select map is not in the list, skip overriding.", selected, net);
                        }
                    }
                }
            }
        }
    }
    pub fn insert(&mut self, key: String, value: String) -> Option<String> {
        self.0.insert(key, value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::MemoryCache;
    use rabbit_digger::config::{Config, Net};
    use serde_json::json;

    #[tokio::test]
    async fn test_from_cache_invalid_json_is_empty() {
        let cache = MemoryCache::new().await.unwrap();
        cache.set("id", "not-json").await.unwrap();

        let m = SelectMap::from_cache("id", &cache).await.unwrap();
        assert!(m.0.is_empty());
    }

    #[tokio::test]
    async fn test_apply_config_overrides_when_selected_in_list() {
        let mut cfg = Config::default();
        cfg.net.insert(
            "sel".to_string(),
            Net::new(
                "select",
                json!({
                    "list": ["a", "b"],
                    "selected": "a"
                }),
            ),
        );

        let mut m = SelectMap(HashMap::new());
        m.insert("sel".to_string(), "b".to_string());
        m.apply_config(&mut cfg).await;

        let opt = cfg.net.get("sel").unwrap().opt.as_object().unwrap();
        assert_eq!(opt.get("selected").unwrap().as_str().unwrap(), "b");
    }

    #[tokio::test]
    async fn test_apply_config_skips_when_selected_not_in_list() {
        let mut cfg = Config::default();
        cfg.net.insert(
            "sel".to_string(),
            Net::new(
                "select",
                json!({
                    "list": ["a"],
                    "selected": "a"
                }),
            ),
        );

        let mut m = SelectMap(HashMap::new());
        m.insert("sel".to_string(), "b".to_string());
        m.apply_config(&mut cfg).await;

        let opt = cfg.net.get("sel").unwrap().opt.as_object().unwrap();
        assert_eq!(opt.get("selected").unwrap().as_str().unwrap(), "a");
    }
}
