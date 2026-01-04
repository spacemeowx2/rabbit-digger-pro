use anyhow::{Context, Result};
use config::ConfigManager;
pub use rabbit_digger;
use rabbit_digger::{RabbitDigger, Registry};
use yaml_merge_keys::merge_keys_serde;

#[cfg(feature = "api_server")]
pub mod api_server;
pub mod config;
pub mod log;
pub mod schema;
mod select;
pub mod storage;
pub mod util;

pub fn get_registry() -> Result<Registry> {
    let mut registry = Registry::new_with_builtin()?;

    #[cfg(feature = "ss")]
    registry.init_with_registry("ss", ss::init)?;
    #[cfg(feature = "trojan")]
    registry.init_with_registry("trojan", trojan::init)?;
    #[cfg(feature = "rpc")]
    registry.init_with_registry("rpc", rpc::init)?;
    #[cfg(feature = "raw")]
    registry.init_with_registry("raw", raw::init)?;
    #[cfg(feature = "obfs")]
    registry.init_with_registry("obfs", obfs::init)?;

    registry.init_with_registry("rabbit-digger-pro", select::init)?;

    Ok(registry)
}

pub fn deserialize_config(s: &str) -> Result<config::ConfigExt> {
    let raw_yaml = serde_yaml::from_str(s)?;
    let merged = merge_keys_serde(raw_yaml)?;
    Ok(serde_yaml::from_value(merged)?)
}

pub struct App {
    pub rd: RabbitDigger,
    pub cfg_mgr: ConfigManager,
}

#[derive(Default, Debug)]
pub struct ApiServer {
    pub bind: Option<String>,
    pub access_token: Option<String>,
    pub web_ui: Option<String>,
}

impl App {
    pub async fn new() -> Result<Self> {
        let rd = RabbitDigger::new(get_registry()?).await?;
        let cfg_mgr = ConfigManager::new().await?;

        Ok(Self { rd, cfg_mgr })
    }
    pub async fn run_api_server(&self, api_server: ApiServer) -> Result<()> {
        #[cfg(feature = "api_server")]
        if let Some(bind) = api_server.bind {
            api_server::ApiServer {
                rabbit_digger: self.rd.clone(),
                config_manager: self.cfg_mgr.clone(),
                access_token: api_server.access_token,
                web_ui: api_server.web_ui,
            }
            .run(&bind)
            .await
            .context("Failed to run api server.")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deserialize_config() {
        let yaml = r#"
net:
  test_net:
    type: direct
"#;
        let result = deserialize_config(yaml);
        assert!(result.is_ok());
    }

    #[test]
    fn test_deserialize_config_invalid() {
        let invalid_yaml = "invalid: yaml: [";
        let result = deserialize_config(invalid_yaml);
        assert!(result.is_err());
    }

    #[test]
    fn test_api_server_default() {
        let api_server = ApiServer::default();
        assert!(api_server.bind.is_none());
        assert!(api_server.access_token.is_none());
        assert!(api_server.web_ui.is_none());
    }

    #[tokio::test]
    async fn test_app_new() {
        let result = App::new().await;
        assert!(result.is_ok());
    }
}
