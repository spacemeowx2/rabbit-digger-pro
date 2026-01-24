use rd_interface::{
    async_trait, config::NetRef, prelude::*, registry::Builder, Address, Context, IServer, Net,
    Result, Server, TcpListener, TcpStream,
};
use tokio::io;
use tracing::instrument;

/// A echo server.
#[rd_config]
#[derive(Debug)]
pub struct EchoServerConfig {
    bind: Address,
    #[serde(default)]
    listen: NetRef,
}

pub struct EchoServer {
    listen: Net,
    bind: Address,
}

impl EchoServer {
    fn new(EchoServerConfig { bind, listen }: EchoServerConfig) -> EchoServer {
        let listen = listen.value_cloned();
        EchoServer { listen, bind }
    }
}
#[async_trait]
impl IServer for EchoServer {
    async fn start(&self) -> Result<()> {
        let listener = self
            .listen
            .tcp_bind(&mut Context::new(), &self.bind)
            .await?;
        self.serve_listener(listener).await
    }
}

impl EchoServer {
    #[instrument(err, skip(socket))]
    async fn serve_connection(socket: TcpStream) -> Result<()> {
        let (mut rx, mut tx) = io::split(socket);
        io::copy(&mut rx, &mut tx).await?;
        Ok(())
    }
    pub async fn serve_listener(&self, listener: TcpListener) -> Result<()> {
        loop {
            let (socket, _) = listener.accept().await?;
            let _ = tokio::spawn(async move {
                if let Err(e) = Self::serve_connection(socket).await {
                    tracing::error!("Error when serve_connection: {:?}", e);
                }
            });
        }
    }
}

impl Builder<Server> for EchoServer {
    const NAME: &'static str = "echo";
    type Config = EchoServerConfig;
    type Item = Self;

    fn build(cfg: Self::Config) -> Result<Self> {
        Ok(EchoServer::new(cfg))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use rd_interface::{IntoAddress, IntoDyn};
    use tokio::time::sleep;

    use super::*;
    use crate::tests::{assert_echo, TestNet};

    #[tokio::test]
    async fn test_echo_server() {
        let net = TestNet::new().into_dyn();

        let server = EchoServer {
            listen: net.clone(),
            bind: "127.0.0.1:1234".into_address().unwrap(),
        };
        tokio::spawn(async move { server.start().await.unwrap() });

        sleep(Duration::from_millis(1)).await;

        assert_echo(&net, "127.0.0.1:1234").await;
    }
}
