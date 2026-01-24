use std::{
    io::{Cursor, Write},
    time::Duration,
};

use crate::{stream::IOStream, websocket::WebSocketStream};
use rd_interface::{
    async_trait,
    prelude::*,
    registry::{Builder, NetRef},
    Address as RdAddress, Address, INet, IntoDyn, Net, Result, TcpStream, UdpSocket,
};
use rd_std::tls::{TlsNet, TlsNetConfig};
use sha2::{Digest, Sha224};
use socks5_protocol::{sync::FromIO, Address as S5Addr};
use tokio::time::timeout;

mod tcp;
mod udp;

pub struct TrojanNet {
    net: Net,
    server: RdAddress,
    password: String,
    websocket: Option<WebSocket>,
    handshake_timeout: Option<u64>,
}

impl TrojanNet {
    pub fn new_trojan(config: TrojanNetConfig) -> Result<Self> {
        let tls_config = TlsNetConfig {
            skip_cert_verify: config.skip_cert_verify,
            sni: config.sni,
            net: config.net,
        };
        let server = config.server.clone();

        let password = hex::encode(Sha224::digest(config.password.as_bytes()));
        let net = TlsNet::build(tls_config)?.into_dyn();

        Ok(TrojanNet {
            net,
            server,
            password,
            websocket: config.websocket,
            handshake_timeout: config.handshake_timeout,
        })
    }
    pub fn new_trojanc(config: TrojancNetConfig) -> Result<Self> {
        let password = hex::encode(Sha224::digest(config.password.as_bytes()));

        Ok(TrojanNet {
            net: (*config.net).clone(),
            server: config.server,
            password,
            websocket: config.websocket,
            handshake_timeout: config.handshake_timeout,
        })
    }
}

#[rd_config]
#[derive(Debug, Clone)]
pub struct WebSocket {
    host: String,
    path: String,
}

#[rd_config]
#[derive(Debug, Clone)]
pub struct TrojanNetConfig {
    #[serde(default)]
    net: NetRef,

    /// hostname:port
    server: Address,
    /// password in plain text
    #[serde(skip_serializing_if = "rd_interface::config::detailed_field")]
    password: String,

    /// sni
    #[serde(default)]
    sni: Option<String>,
    /// skip certificate verify
    #[serde(default)]
    skip_cert_verify: bool,

    /// enabled websocket support
    #[serde(default)]
    websocket: Option<WebSocket>,

    /// timeout of TLS handshake, in seconds.
    handshake_timeout: Option<u64>,
}

#[rd_config]
pub struct TrojancNetConfig {
    #[serde(default)]
    net: NetRef,

    /// hostname:port
    server: RdAddress,
    /// password in plain text
    #[serde(skip_serializing_if = "rd_interface::config::detailed_field")]
    password: String,

    /// enabled websocket support
    #[serde(default)]
    websocket: Option<WebSocket>,

    /// timeout of TLS handshake, in seconds.
    handshake_timeout: Option<u64>,
}

impl TrojanNet {
    // cmd 1 for Connect, 3 for Udp associate
    fn make_head(&self, cmd: u8, addr: S5Addr) -> Result<Vec<u8>> {
        let head = Vec::<u8>::new();
        let mut writer = Cursor::new(head);

        writer.write_all(self.password.as_bytes())?;
        writer.write_all(b"\r\n")?;
        // Connect
        writer.write_all(&[cmd])?;
        addr.write_to(&mut writer).map_err(|e| e.to_io_err())?;
        writer.write_all(b"\r\n")?;

        Ok(writer.into_inner())
    }
    async fn connect_stream(&self, ctx: &mut rd_interface::Context) -> Result<Box<dyn IOStream>> {
        let stream = self.net.tcp_connect(ctx, &self.server).await?;
        Ok(match &self.websocket {
            Some(ws) => Box::new(WebSocketStream::connect(stream, &ws.host, &ws.path).await?),
            None => Box::new(stream),
        })
    }
    async fn get_stream(&self, ctx: &mut rd_interface::Context) -> Result<Box<dyn IOStream>> {
        let timeout_sec = self.handshake_timeout.unwrap_or(10);

        timeout(Duration::from_secs(timeout_sec), self.connect_stream(ctx)).await?
    }
}

pub(crate) fn ra2sa(addr: RdAddress) -> S5Addr {
    match addr {
        RdAddress::SocketAddr(s) => S5Addr::SocketAddr(s),
        RdAddress::Domain(d, p) => S5Addr::Domain(d, p),
    }
}

#[async_trait]
impl rd_interface::TcpConnect for TrojanNet {
    async fn tcp_connect(
        &self,
        ctx: &mut rd_interface::Context,
        addr: &RdAddress,
    ) -> Result<TcpStream> {
        let stream = self.get_stream(ctx).await?;
        let head = self.make_head(1, ra2sa(addr.clone()))?;

        let tcp = tcp::TrojanTcp::new(stream, head);
        Ok(tcp.into_dyn())
    }
}

#[async_trait]
impl rd_interface::UdpBind for TrojanNet {
    async fn udp_bind(
        &self,
        ctx: &mut rd_interface::Context,
        addr: &RdAddress,
    ) -> Result<UdpSocket> {
        let stream = self.get_stream(ctx).await?;
        let head = self.make_head(3, ra2sa(addr.clone()))?;

        let udp = udp::TrojanUdp::new(stream, head);

        Ok(udp.into_dyn())
    }
}

impl INet for TrojanNet {
    fn provide_tcp_connect(&self) -> Option<&dyn rd_interface::TcpConnect> {
        Some(self)
    }

    fn provide_udp_bind(&self) -> Option<&dyn rd_interface::UdpBind> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use rd_interface::IntoAddress;
    use rd_std::tests::{assert_net_provider, ProviderCapability, TestNet};

    use super::*;

    #[test]
    fn test_provider() {
        let net = TestNet::new().into_dyn();

        let trojan = TrojanNet::new_trojan(TrojanNetConfig {
            net: NetRef::new_with_value("test".into(), net),
            server: "127.0.0.1:1234".into_address().unwrap(),
            password: "password".to_string(),
            sni: None,
            skip_cert_verify: false,
            websocket: None,
            handshake_timeout: None,
        })
        .unwrap()
        .into_dyn();

        assert_net_provider(
            &trojan,
            ProviderCapability {
                tcp_connect: true,
                udp_bind: true,
                ..Default::default()
            },
        );
    }
}
