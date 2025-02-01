use std::net::SocketAddr;

use anyhow::Result;
use rabbit_digger::RabbitDigger;

use crate::config::ConfigManager;

mod handlers;
mod routes;
pub mod v2;

pub struct ApiServer {
    pub rabbit_digger: RabbitDigger,
    pub config_manager: ConfigManager,
    pub access_token: Option<String>,
    pub web_ui: Option<String>,
}

impl ApiServer {
    pub async fn run(self, bind: &str) -> Result<SocketAddr> {
        let app = self.routes().await?;

        let server = axum::Server::bind(&bind.parse()?).serve(app.into_make_service());
        let local_addr = server.local_addr();
        tokio::spawn(server);

        Ok(local_addr)
    }
}
