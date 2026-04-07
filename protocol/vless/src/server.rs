use std::sync::{atomic::AtomicBool, Arc};

use crate::common::{
    build_server_config, ensure_rustls_provider_installed, read_request_header, relay_stream_udp,
    write_response_header, NormalizedFlow, UserId, VisionStream, COMMAND_MUX, COMMAND_TCP,
    FLOW_VISION, MUX_DOMAIN, MUX_PORT,
};
use crate::reality::{accept_reality_stream, RealityServerConfig};
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

    #[serde(default)]
    pub(crate) tls_cert: String,
    #[serde(default)]
    pub(crate) tls_key: String,

    #[serde(default)]
    pub(crate) reality_server_name: Option<String>,
    #[serde(default)]
    pub(crate) reality_private_key: Option<String>,
    #[serde(default)]
    pub(crate) reality_short_id: Option<String>,

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
    acceptor: ServerAcceptor,
    udp: bool,
    listen: Net,
    net: Net,
}

#[derive(Clone)]
enum ServerAcceptor {
    Tls(TlsAcceptor),
    Reality(Arc<RealityServerConfig>),
}

impl VlessServer {
    pub fn new(config: VlessServerConfig) -> Result<Self> {
        ensure_rustls_provider_installed();

        let acceptor = match config.reality_private_key.clone() {
            Some(private_key) => ServerAcceptor::Reality(Arc::new(RealityServerConfig {
                server_name: config.reality_server_name.clone().ok_or_else(|| {
                    rd_interface::Error::other(
                        "reality_server_name is required when reality_private_key is set",
                    )
                })?,
                private_key,
                short_id: config.reality_short_id.clone(),
                max_time_diff: None,
            })),
            None => ServerAcceptor::Tls(TlsAcceptor::from(Arc::new(build_server_config(
                &config.tls_cert,
                &config.tls_key,
            )?))),
        };
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

    async fn serve_vless_stream<S>(
        user_id: UserId,
        flow: NormalizedFlow,
        udp: bool,
        net: Net,
        mut stream: S,
        peer: std::net::SocketAddr,
        shared_read_raw: Option<Arc<AtomicBool>>,
    ) -> Result<()>
    where
        S: rd_interface::AsyncRead + rd_interface::AsyncWrite + Unpin + Send + 'static,
    {
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
                    ctx.connect_tcp(
                        VisionStream::new_with_shared(stream, &user_id, shared_read_raw),
                        outbound,
                    )
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
                relay_stream_udp(
                    VisionStream::new_with_shared(stream, &user_id, shared_read_raw),
                    net,
                )
                .await?;
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

    async fn serve_connection(
        acceptor: ServerAcceptor,
        user_id: UserId,
        flow: NormalizedFlow,
        udp: bool,
        net: Net,
        socket: rd_interface::TcpStream,
        peer: std::net::SocketAddr,
    ) -> Result<()> {
        match acceptor {
            ServerAcceptor::Tls(acceptor) => {
                let stream = acceptor
                    .accept(socket)
                    .await
                    .map_err(rd_interface::error::map_other)?;
                Self::serve_vless_stream(user_id, flow, udp, net, stream, peer, None).await
            }
            ServerAcceptor::Reality(cfg) => {
                let shared_read_raw = flow.is_vision().then(|| Arc::new(AtomicBool::new(false)));
                let stream = accept_reality_stream(socket, &cfg, shared_read_raw.clone()).await?;
                Self::serve_vless_stream(user_id, flow, udp, net, stream, peer, shared_read_raw)
                    .await
            }
        }
    }
}

#[async_trait]
impl IServer for VlessServer {
    async fn start(&self, _ctx: &rd_interface::EngineContext) -> Result<()> {
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
