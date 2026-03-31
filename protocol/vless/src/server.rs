use std::sync::Arc;

use crate::common::{
    build_server_config, read_request_header, relay_stream_udp, write_response_header,
    NormalizedFlow, UserId, VisionStream, COMMAND_MUX, COMMAND_TCP, FLOW_VISION, MUX_DOMAIN,
    MUX_PORT,
};
use rd_interface::{async_trait, config::NetRef, prelude::*, Address, IServer, Net, Result};
use rd_std::ContextExt;
use tokio_rustls::TlsAcceptor;

#[rd_config]
#[derive(Debug, Clone)]
pub struct VlessServerConfig {
    pub(crate) bind: Address,

    /// UUID
    pub(crate) id: String,

    /// flow
    #[serde(default = "default_flow")]
    pub(crate) flow: Option<String>,

    pub(crate) tls_cert: String,
    pub(crate) tls_key: String,

    #[serde(default)]
    pub(crate) udp: bool,

    #[serde(default)]
    pub(crate) net: NetRef,
    #[serde(default)]
    pub(crate) listen: NetRef,
}

fn default_flow() -> Option<String> {
    Some(FLOW_VISION.to_string())
}

pub struct VlessServer {
    bind: Address,
    user_id: UserId,
    flow: NormalizedFlow,
    acceptor: TlsAcceptor,
    udp: bool,
    listen: Net,
    net: Net,
}

impl VlessServer {
    pub fn new(config: VlessServerConfig) -> Result<Self> {
        let acceptor = TlsAcceptor::from(Arc::new(build_server_config(
            &config.tls_cert,
            &config.tls_key,
        )?));
        Ok(Self {
            bind: config.bind,
            user_id: UserId::parse(&config.id)?,
            flow: NormalizedFlow::parse(config.flow)?,
            acceptor,
            udp: config.udp,
            listen: config.listen.value_cloned(),
            net: config.net.value_cloned(),
        })
    }

    async fn serve_connection(
        acceptor: TlsAcceptor,
        user_id: UserId,
        flow: NormalizedFlow,
        udp: bool,
        net: Net,
        socket: rd_interface::TcpStream,
        peer: std::net::SocketAddr,
    ) -> Result<()> {
        let mut stream = acceptor
            .accept(socket)
            .await
            .map_err(rd_interface::error::map_other)?;
        let request = read_request_header(&mut stream, &user_id).await?;
        if request.flow != flow {
            return Err(rd_interface::Error::other(format!(
                "unexpected vless flow: {:?}",
                request.flow.as_deref()
            )));
        }

        match request.command {
            COMMAND_TCP => {
                let mut ctx = rd_interface::Context::from_socketaddr(peer);
                let outbound = net.tcp_connect(&mut ctx, &request.addr).await?;
                write_response_header(&mut stream).await?;
                if flow.is_vision() {
                    ctx.connect_tcp(VisionStream::new(stream, &user_id), outbound)
                        .await?;
                } else {
                    ctx.connect_tcp(stream, outbound).await?;
                }
            }
            COMMAND_MUX
                if udp
                    && flow.is_vision()
                    && matches!(&request.addr, Address::Domain(domain, port) if domain == MUX_DOMAIN && *port == MUX_PORT) =>
            {
                write_response_header(&mut stream).await?;
                relay_stream_udp(VisionStream::new(stream, &user_id), net).await?;
            }
            _ => {
                return Err(rd_interface::Error::other(format!(
                    "unsupported vless command {}",
                    request.command
                )))
            }
        }

        Ok(())
    }
}

#[async_trait]
impl IServer for VlessServer {
    async fn start(&self) -> Result<()> {
        let listener = self
            .listen
            .tcp_bind(&mut rd_interface::Context::new(), &self.bind)
            .await?;
        loop {
            let (socket, peer) = listener.accept().await?;
            let acceptor = self.acceptor.clone();
            let user_id = self.user_id.clone();
            let flow = self.flow.clone();
            let udp = self.udp;
            let net = self.net.clone();
            tokio::spawn(async move {
                if let Err(err) =
                    Self::serve_connection(acceptor, user_id, flow, udp, net, socket, peer).await
                {
                    tracing::debug!("vless connection error: {:?}", err);
                }
            });
        }
    }
}
