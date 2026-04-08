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
pub mod service;
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
    #[cfg(feature = "hysteria")]
    registry.init_with_registry("hysteria", hysteria::init)?;
    #[cfg(feature = "vless")]
    registry.init_with_registry("vless", vless::init)?;

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
pub struct ApiServerConfig {
    pub bind: Option<String>,
    pub access_token: Option<String>,
    pub web_ui: Option<String>,
    pub source_sender: Option<tokio::sync::mpsc::Sender<config::ImportSource>>,
    pub log_file_path: Option<std::path::PathBuf>,
}

impl App {
    pub async fn new() -> Result<Self> {
        let se_path = util::app_dirs::data_dir().join("side_effects.json");
        let rd = RabbitDigger::new(get_registry()?, se_path).await?;
        let cfg_mgr = ConfigManager::new().await?;

        Ok(Self { rd, cfg_mgr })
    }
    pub async fn run_api_server(&self, config: ApiServerConfig) -> Result<()> {
        #[cfg(feature = "api_server")]
        if let Some(bind) = config.bind {
            api_server::ApiServer {
                rabbit_digger: self.rd.clone(),
                config_manager: self.cfg_mgr.clone(),
                access_token: config.access_token,
                web_ui: config.web_ui,
                source_sender: config.source_sender,
                log_file_path: config.log_file_path,
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
    fn test_deserialize_config_minimal() {
        let cfg = deserialize_config(
            r#"
id: test
net: {}
server: {}
"#,
        )
        .unwrap();
        let v = serde_yaml::to_value(&cfg).unwrap();
        assert_eq!(v.get("id").and_then(|v| v.as_str()), Some("test"));
    }

    #[test]
    fn test_get_registry_smoke() {
        let registry = get_registry().unwrap();
        assert!(!registry.net().is_empty());
    }

    #[test]
    fn test_deserialize_config_invalid_is_error() {
        let err = deserialize_config("not: [valid").unwrap_err();
        let _ = err;
    }

    #[tokio::test]
    async fn test_app_new_smoke() {
        let app = App::new().await.unwrap();
        // Make sure the object is usable.
        let _ = app.rd.get_id().await;
    }
}
