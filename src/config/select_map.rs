use std::collections::HashMap;

use anyhow::Result;
use rabbit_digger::Config;
use serde::{Deserialize, Serialize};

use crate::storage::Storage;

#[derive(Debug, Serialize, Deserialize, Default)]
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

    #[test]
    fn test_select_map_insert() {
        let mut map = SelectMap(HashMap::new());
        let old_value = map.insert("net1".to_string(), "selected1".to_string());
        assert!(old_value.is_none());
        let old_value = map.insert("net1".to_string(), "selected2".to_string());
        assert_eq!(old_value, Some("selected1".to_string()));
    }

    #[test]
    fn test_select_map_default() {
        let map = SelectMap::default();
        assert!(map.0.is_empty());
    }

    #[tokio::test]
    async fn test_select_map_from_cache_empty() {
        let cache = crate::storage::MemoryCache::new().await.unwrap();
        let map = SelectMap::from_cache("test_id", &cache).await.unwrap();
        assert!(map.0.is_empty());
    }

    #[tokio::test]
    async fn test_select_map_write_cache() {
        let cache = crate::storage::MemoryCache::new().await.unwrap();
        let mut map = SelectMap(HashMap::new());
        map.insert("net1".to_string(), "selected1".to_string());
        map.write_cache("test_id", &cache).await.unwrap();

        let loaded = SelectMap::from_cache("test_id", &cache).await.unwrap();
        assert_eq!(loaded.0.get("net1"), Some(&"selected1".to_string()));
    }

    #[tokio::test]
    async fn test_select_map_apply_config() {
        use rabbit_digger::config::Net;
        use rd_interface::Value;
        use serde_json::json;

        let cache = crate::storage::MemoryCache::new().await.unwrap();
        let mut map = SelectMap(HashMap::new());
        map.insert("net_select".to_string(), "net2".to_string());

        let mut config = rabbit_digger::Config::default();
        let net_value: Net = serde_json::from_value(json!({
            "type": "select",
            "list": ["net1", "net2"],
            "selected": "net1"
        }))
        .unwrap();
        config.net.insert("net_select".to_string(), net_value);

        map.apply_config(&mut config).await;

        let net_config = config.net.get("net_select").unwrap();
        assert_eq!(
            net_config.opt.get("selected"),
            Some(&Value::String("net2".to_string()))
        );
    }

    #[tokio::test]
    async fn test_select_map_apply_config_invalid() {
        use rabbit_digger::config::Net;
        use rd_interface::Value;
        use serde_json::json;

        let cache = crate::storage::MemoryCache::new().await.unwrap();
        let mut map = SelectMap(HashMap::new());
        map.insert("net_select".to_string(), "net_invalid".to_string());

        let mut config = rabbit_digger::Config::default();
        let net_value: Net = serde_json::from_value(json!({
            "type": "select",
            "list": ["net1", "net2"],
            "selected": "net1"
        }))
        .unwrap();
        config.net.insert("net_select".to_string(), net_value);

        map.apply_config(&mut config).await;

        let net_config = config.net.get("net_select").unwrap();
        assert_eq!(
            net_config.opt.get("selected"),
            Some(&Value::String("net1".to_string()))
        );
    }
}
