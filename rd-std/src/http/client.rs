use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use hyper::{client::conn as client_conn, Body, Error, Request};
use std::net::SocketAddr;

use rd_interface::{
    async_trait, impl_async_read_write, Address, INet, ITcpStream, IntoDyn, Net, Result, TcpStream,
    NOT_IMPLEMENTED,
};

fn map_err(e: Error) -> rd_interface::Error {
    rd_interface::Error::Other(e.into())
}

pub struct HttpClient {
    server: Address,
    net: Net,
    username: Option<String>,
    password: Option<String>,
}

pub struct HttpTcpStream(TcpStream);

#[async_trait]
impl ITcpStream for HttpTcpStream {
    async fn peer_addr(&self) -> Result<SocketAddr> {
        Err(NOT_IMPLEMENTED)
    }

    async fn local_addr(&self) -> Result<SocketAddr> {
        Err(NOT_IMPLEMENTED)
    }

    impl_async_read_write!(0);
}

#[async_trait]
impl rd_interface::TcpConnect for HttpClient {
    async fn tcp_connect(
        &self,
        ctx: &mut rd_interface::Context,
        addr: &rd_interface::Address,
    ) -> Result<TcpStream> {
        let socket = self.net.tcp_connect(ctx, &self.server).await?;
        let (mut request_sender, connection) =
            client_conn::handshake(socket).await.map_err(map_err)?;

        let mut connect_req = Request::builder().method("CONNECT").uri(addr.to_string());

        // 添加认证头
        if let (Some(username), Some(password)) = (&self.username, &self.password) {
            let auth = format!("{}:{}", username, password);
            let encoded = BASE64.encode(auth);
            let auth_header = format!("Basic {}", encoded);
            connect_req = connect_req.header(hyper::http::header::PROXY_AUTHORIZATION, auth_header);
        }

        let connect_req = connect_req.body(Body::empty()).unwrap();

        let connection = connection.without_shutdown();
        let _connect_resp = request_sender.send_request(connect_req);
        let io = connection.await.map_err(map_err)?.io;
        let _connect_resp = _connect_resp.await.map_err(map_err)?;
        Ok(HttpTcpStream(io).into_dyn())
    }
}

impl INet for HttpClient {
    fn provide_tcp_connect(&self) -> Option<&dyn rd_interface::TcpConnect> {
        Some(self)
    }
}

impl HttpClient {
    pub fn new(net: Net, server: Address) -> Self {
        Self {
            server,
            net,
            username: None,
            password: None,
        }
    }

    pub fn with_auth(net: Net, server: Address, username: String, password: String) -> Self {
        Self {
            server,
            net,
            username: Some(username),
            password: Some(password),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::tests::{assert_net_provider, ProviderCapability, TestNet};
    use rd_interface::{IntoAddress, IntoDyn};

    use super::*;

    #[test]
    fn test_provider() {
        let net = TestNet::new().into_dyn();

        let http = HttpClient::new(net, "127.0.0.1:12345".into_address().unwrap()).into_dyn();

        assert_net_provider(
            &http,
            ProviderCapability {
                tcp_connect: true,
                ..Default::default()
            },
        );
    }
}
