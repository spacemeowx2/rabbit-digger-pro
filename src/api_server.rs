use std::net::SocketAddr;

use anyhow::Result;
use rabbit_digger::RabbitDigger;
use tokio::net::TcpListener;

use crate::config::ConfigManager;

mod handlers;
mod routes;

pub struct ApiServer {
    pub rabbit_digger: RabbitDigger,
    pub config_manager: ConfigManager,
    pub access_token: Option<String>,
    pub web_ui: Option<String>,
}

impl ApiServer {
    pub async fn run(self, bind: &str) -> Result<SocketAddr> {
        let app = self.routes().await?;

        let listener = TcpListener::bind(bind).await?;
        let local_addr = listener.local_addr()?;
        tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app).await {
                tracing::error!("api server exited: {e}");
            }
        });

        Ok(local_addr)
    }
}
