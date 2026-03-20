use std::sync::Arc;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

use crate::storage::{FileStorage, FolderType, Storage};

use super::{PolicyActionRecord, PolicySuggestion, POLICY_SCHEMA_VERSION};

const POLICY_STATE_KEY: &str = "policy_state";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedPolicyState {
    pub version: u32,
    pub suggestions: Vec<PolicySuggestion>,
    pub actions: Vec<PolicyActionRecord>,
}

impl PersistedPolicyState {
    pub fn new(suggestions: Vec<PolicySuggestion>, actions: Vec<PolicyActionRecord>) -> Self {
        Self {
            version: POLICY_SCHEMA_VERSION,
            suggestions,
            actions,
        }
    }
}

impl Default for PersistedPolicyState {
    fn default() -> Self {
        Self::new(vec![], vec![])
    }
}

#[derive(Clone)]
pub struct PolicyStore {
    storage: Arc<dyn Storage>,
}

impl PolicyStore {
    pub async fn new_data() -> Result<Self> {
        Ok(Self {
            storage: Arc::new(FileStorage::new(FolderType::Data, "policy").await?),
        })
    }

    #[cfg(test)]
    pub fn new(storage: Arc<dyn Storage>) -> Self {
        Self { storage }
    }

    pub async fn load(&self) -> Result<PersistedPolicyState> {
        let Some(item) = self.storage.get(POLICY_STATE_KEY).await? else {
            return Ok(PersistedPolicyState::default());
        };
        let persisted: PersistedPolicyState = serde_json::from_str(&item.content)?;
        if persisted.version != POLICY_SCHEMA_VERSION {
            return Err(anyhow!(
                "unsupported policy store version {}, expected {}",
                persisted.version,
                POLICY_SCHEMA_VERSION
            ));
        }
        Ok(persisted)
    }

    pub async fn save(&self, state: &PersistedPolicyState) -> Result<()> {
        self.storage
            .set(POLICY_STATE_KEY, &serde_json::to_string(state)?)
            .await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::storage::MemoryCache;

    use super::*;

    #[tokio::test]
    async fn test_policy_store_roundtrip_and_version() {
        let cache = Arc::new(MemoryCache::new().await.unwrap());
        let store = PolicyStore::new(cache.clone());
        let state = PersistedPolicyState::default();

        store.save(&state).await.unwrap();
        let loaded = store.load().await.unwrap();
        assert_eq!(loaded.version, POLICY_SCHEMA_VERSION);
    }

    #[tokio::test]
    async fn test_policy_store_rejects_wrong_version() {
        let cache = Arc::new(MemoryCache::new().await.unwrap());
        cache
            .set(
                POLICY_STATE_KEY,
                r#"{"version":999,"suggestions":[],"actions":[]}"#,
            )
            .await
            .unwrap();
        let store = PolicyStore::new(cache);
        assert!(store.load().await.is_err());
    }
}
