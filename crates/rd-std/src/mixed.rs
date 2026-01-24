use std::net::SocketAddr;

use anyhow::Context as AnyhowContext;
use rd_interface::{
    async_trait, config::NetRef, prelude::*, registry::Builder, Address, Context, IServer, IntoDyn,
    Net, Registry, Result, Server, TcpStream,
};
use tracing::instrument;

use crate::{http::HttpServer, socks5::Socks5Server, util::PeekableTcpStream};

#[derive(Clone)]
struct HttpSocks5Server {
    http_server: HttpServer,
    socks5_server: Socks5Server,
}

impl HttpSocks5Server {
    fn new(listen_net: Net, net: Net) -> Self {
        Self {
            http_server: HttpServer::new(net.clone()),
            socks5_server: Socks5Server::new(listen_net, net),
        }
    }
    #[instrument(err, skip(self, socket))]
    pub async fn serve_connection(self, socket: TcpStream, addr: SocketAddr) -> anyhow::Result<()> {
        let buf = &mut [0u8; 1];
        let mut socket = PeekableTcpStream::new(socket);
        if let Err(_) = socket.peek_exact(buf).await {
            // The client has closed the connection before we could read the first byte.
            // This is not an error, so we just return.
            return Ok(());
        }
        let socket = socket.into_dyn();

        match buf[0] {
            b'\x05' => self
                .socks5_server
                .serve_connection(socket, addr)
                .await
                .context("socks5 server"),
            _ => self
                .http_server
                .serve_connection(socket, addr)
                .await
                .context("http server"),
        }
    }
}

pub struct HttpSocks5 {
    listen_net: Net,
    bind: Address,

    server: HttpSocks5Server,
}

#[async_trait]
impl IServer for HttpSocks5 {
    async fn start(&self) -> Result<()> {
        let listener = self
            .listen_net
            .tcp_bind(&mut Context::new(), &self.bind)
            .await?;

        loop {
            let (socket, addr) = listener.accept().await?;

            let server = self.server.clone();
            let _ = tokio::spawn(async move {
                if let Err(e) = server.serve_connection(socket, addr).await {
                    tracing::error!("Error when serve_connection: {:?}", e)
                }
            });
        }
    }
}

impl HttpSocks5 {
    fn new(listen_net: Net, net: Net, bind: Address) -> Self {
        HttpSocks5 {
            server: HttpSocks5Server::new(listen_net.clone(), net),
            listen_net,
            bind,
        }
    }
}

#[rd_config]
#[derive(Debug)]
pub struct MixedServerConfig {
    bind: Address,
    #[serde(default)]
    listen: NetRef,
    #[serde(default)]
    net: NetRef,
}

impl Builder<Server> for HttpSocks5 {
    const NAME: &'static str = "http+socks5";
    type Config = MixedServerConfig;
    type Item = Self;

    fn build(Self::Config { listen, net, bind }: Self::Config) -> Result<Self> {
        Ok(HttpSocks5::new(
            listen.value_cloned(),
            net.value_cloned(),
            bind,
        ))
    }
}

pub fn init(registry: &mut Registry) -> Result<()> {
    registry.add_server::<HttpSocks5>();
    Ok(())
}
