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
    #[cfg(feature = "hysteria2")]
    registry.init_with_registry("hysteria2", hysteria2::init)?;

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
