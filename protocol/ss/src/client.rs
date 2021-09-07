use super::wrapper::{Cipher, WrapAddress, WrapSSTcp, WrapSSUdp};
use rd_interface::{
    async_trait, prelude::*, registry::NetRef, Address, Arc, INet, IntoDyn, Result, TcpStream,
    UdpSocket, NOT_ENABLED,
};
use shadowsocks::{
    config::{ServerConfig, ServerType},
    context::Context,
    ProxyClientStream,
};

#[rd_config]
#[derive(Debug, Clone)]
pub struct SSNetConfig {
    server: Address,
    password: String,
    #[serde(default)]
    udp: bool,

    cipher: Cipher,

    #[serde(default)]
    net: NetRef,
}

pub struct SSNet {
    context: Arc<Context>,
    config: SSNetConfig,
}

impl SSNet {
    pub fn new(config: SSNetConfig) -> SSNet {
        let context = Arc::new(Context::new(ServerType::Local));
        SSNet { context, config }
    }
}

#[async_trait]
impl INet for SSNet {
    async fn tcp_connect(
        &self,
        ctx: &mut rd_interface::Context,
        addr: &Address,
    ) -> Result<TcpStream> {
        let cfg = self.config.clone();
        let stream = self.config.net.tcp_connect(ctx, &cfg.server).await?;
        let svr_cfg = ServerConfig::new(
            (cfg.server.host(), cfg.server.port()),
            cfg.password,
            cfg.cipher.into(),
        );
        let client = ProxyClientStream::from_stream(
            self.context.clone(),
            stream,
            &svr_cfg,
            WrapAddress(addr.clone()),
        );
        Ok(WrapSSTcp(client).into_dyn())
    }

    async fn udp_bind(&self, ctx: &mut rd_interface::Context, addr: &Address) -> Result<UdpSocket> {
        if !self.config.udp {
            return Err(NOT_ENABLED);
        }
        let cfg = self.config.clone();
        let socket = self
            .config
            .net
            .udp_bind(ctx, &addr.to_any_addr_port()?)
            .await?;
        let svr_cfg = ServerConfig::new(
            (cfg.server.host(), cfg.server.port()),
            cfg.password,
            cfg.cipher.into(),
        );
        let udp = WrapSSUdp::new(self.context.clone(), socket, &svr_cfg);
        Ok(udp.into_dyn())
    }
}
