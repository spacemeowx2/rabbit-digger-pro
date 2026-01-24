use std::io;

use super::wrapper::{Cipher, WrapAddress, WrapSSTcp, WrapSSUdp};
use rd_interface::{
    async_trait, prelude::*, registry::NetRef, Address, Error, INet, IntoDyn, Net, Result,
    TcpStream, UdpSocket,
};
use shadowsocks::{
    config::{ServerConfig, ServerType},
    context::{Context, SharedContext},
    ProxyClientStream,
};

#[rd_config]
#[derive(Debug, Clone)]
pub struct SSNetConfig {
    pub(crate) server: Address,
    #[serde(skip_serializing_if = "rd_interface::config::detailed_field")]
    pub(crate) password: String,
    #[serde(default)]
    pub(crate) udp: bool,

    pub(crate) cipher: Cipher,

    #[serde(default)]
    pub(crate) net: NetRef,
}

pub struct SSNet {
    context: SharedContext,
    cfg: ServerConfig,
    addr: Address,
    udp: bool,
    net: Net,
}

impl SSNet {
    pub fn new(config: SSNetConfig) -> Result<SSNet> {
        let cfg = ServerConfig::new(
            (config.server.host(), config.server.port()),
            config.password,
            config.cipher.into(),
        )
        .map_err(rd_interface::error::map_other)?;

        Ok(SSNet {
            context: Context::new_shared(ServerType::Local),
            addr: config.server.clone(),
            cfg,
            udp: config.udp,
            net: config.net.value_cloned(),
        })
    }
}

#[async_trait]
impl rd_interface::TcpConnect for SSNet {
    async fn tcp_connect(
        &self,
        ctx: &mut rd_interface::Context,
        addr: &Address,
    ) -> Result<TcpStream> {
        let stream = self.net.tcp_connect(ctx, &self.addr).await?;

        let client = ProxyClientStream::from_stream(
            self.context.clone(),
            stream,
            &self.cfg,
            WrapAddress(addr.clone()),
        );
        Ok(WrapSSTcp(client).into_dyn())
    }
}

#[async_trait]
impl rd_interface::UdpBind for SSNet {
    async fn udp_bind(
        &self,
        ctx: &mut rd_interface::Context,
        _addr: &Address,
    ) -> Result<UdpSocket> {
        if !self.udp {
            return Err(Error::NotEnabled);
        }

        let server_addr = self
            .net
            .lookup_host(&self.addr)
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::AddrNotAvailable, "Failed to lookup domain")
            })?;

        let socket = self
            .net
            .udp_bind(ctx, &Address::from(server_addr).to_any_addr_port()?)
            .await?;
        let udp = WrapSSUdp::new(socket, &self.cfg, server_addr);
        Ok(udp.into_dyn())
    }
}

impl INet for SSNet {
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

        let ss = SSNet::new(SSNetConfig {
            server: "127.0.0.1:1234".into_address().unwrap(),
            password: "password".to_string(),
            udp: false,
            cipher: Cipher::AES_128_CCM,
            net: NetRef::new_with_value("test".into(), net),
        })
        .unwrap()
        .into_dyn();

        assert_net_provider(
            &ss,
            ProviderCapability {
                tcp_connect: true,
                udp_bind: true,
                ..Default::default()
            },
        );
    }
}
