use std::{
    io,
    net::SocketAddr,
    sync::{atomic::AtomicBool, Arc},
    task::Poll,
};

use crate::common::{
    build_request_header, write_request_header, NormalizedFlow, PrefixWriteStream,
    ResponseHeaderStream, UserId, VisionStream, XudpCodec, COMMAND_MUX, COMMAND_TCP, FLOW_VISION,
    MUX_DOMAIN, MUX_PORT,
};
use crate::reality::{connect_reality_stream, RealityConfig};
use bytes::Bytes;
use futures::{ready, SinkExt, StreamExt};
use rd_interface::{
    async_trait, config::NetRef, prelude::*, Address, Error, INet, IntoDyn, Net, Result, TcpStream,
    UdpSocket,
};
use tokio_rustls::{
    rustls::{
        client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
        pki_types::{CertificateDer, ServerName, UnixTime},
        ClientConfig, DigitallySignedStruct, RootCertStore,
    },
    TlsConnector,
};
use tokio_util::codec::Framed;

#[rd_config]
#[derive(Debug, Clone)]
pub struct VlessNetConfig {
    /// 下游连接所使用的 net。
    #[serde(default)]
    pub(crate) net: NetRef,

    /// hostname:port
    pub(crate) server: Address,

    /// UUID
    pub(crate) id: String,

    /// flow
    #[serde(default = "default_flow")]
    pub(crate) flow: Option<String>,

    /// sni
    #[serde(default)]
    pub(crate) sni: Option<String>,

    /// skip certificate verify
    #[serde(default)]
    pub(crate) skip_cert_verify: bool,

    /// enable udp relay
    #[serde(default)]
    pub(crate) udp: bool,

    #[serde(default)]
    pub(crate) client_fingerprint: Option<String>,

    #[serde(default)]
    pub(crate) reality_public_key: Option<String>,

    #[serde(default)]
    pub(crate) reality_short_id: Option<String>,
}

fn default_flow() -> Option<String> {
    Some(FLOW_VISION.to_string())
}

pub struct VlessNet {
    server: Address,
    user_id: UserId,
    flow: NormalizedFlow,
    udp: bool,
    transport: Transport,
}

enum Transport {
    Tls {
        net: Net,
        server_name: String,
        connector: TlsConnector,
    },
    Reality {
        net: Net,
        config: RealityConfig,
    },
}

#[derive(Debug)]
struct AllowAnyCert(Arc<dyn ServerCertVerifier>);

impl ServerCertVerifier for AllowAnyCert {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, tokio_rustls::rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, tokio_rustls::rustls::Error> {
        self.0.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, tokio_rustls::rustls::Error> {
        self.0.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<tokio_rustls::rustls::SignatureScheme> {
        self.0.supported_verify_schemes()
    }
}

impl VlessNet {
    pub fn new(config: VlessNetConfig) -> Result<Self> {
        let VlessNetConfig {
            net,
            server,
            id,
            flow,
            sni,
            skip_cert_verify,
            udp,
            client_fingerprint,
            reality_public_key,
            reality_short_id,
        } = config;

        let transport = match reality_public_key {
            Some(public_key) => Transport::Reality {
                net: net.value_cloned(),
                config: RealityConfig {
                    server_name: sni.unwrap_or_else(|| server.host()),
                    public_key,
                    short_id: reality_short_id,
                    client_fingerprint,
                },
            },
            None => {
                let server_name = sni.unwrap_or_else(|| server.host());
                let mut roots = RootCertStore::empty();
                roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
                let roots = Arc::new(roots);

                let builder = ClientConfig::builder();
                let client_config = if skip_cert_verify {
                    let verifier =
                        tokio_rustls::rustls::client::WebPkiServerVerifier::builder(roots)
                            .build()
                            .map_err(rd_interface::error::map_other)?;
                    builder
                        .dangerous()
                        .with_custom_certificate_verifier(Arc::new(AllowAnyCert(verifier)))
                        .with_no_client_auth()
                } else {
                    builder.with_root_certificates(roots).with_no_client_auth()
                };
                let mut client_config = client_config;
                client_config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
                Transport::Tls {
                    net: net.value_cloned(),
                    server_name,
                    connector: TlsConnector::from(Arc::new(client_config)),
                }
            }
        };
        Ok(Self {
            server,
            user_id: UserId::parse(&id)?,
            flow: NormalizedFlow::parse(flow)?,
            udp,
            transport,
        })
    }

    async fn connect_stream(
        &self,
        ctx: &mut rd_interface::Context,
        shared_read_raw: Option<Arc<AtomicBool>>,
    ) -> Result<TcpStream> {
        match &self.transport {
            Transport::Tls {
                net,
                server_name,
                connector,
            } => {
                let stream = net.tcp_connect(ctx, &self.server).await?;
                let server_name = ServerName::try_from(server_name.clone())
                    .map_err(rd_interface::error::map_other)?;
                let tls = connector
                    .connect(server_name, stream)
                    .await
                    .map_err(rd_interface::error::map_other)?;
                Ok(TcpStream::from(tls))
            }
            Transport::Reality { net, config } => {
                let stream = net.tcp_connect(ctx, &self.server).await?;
                let reality = connect_reality_stream(stream, config, shared_read_raw)
                    .await
                    .map(TcpStream::from)?;
                Ok(reality)
            }
        }
    }
}

#[async_trait]
impl rd_interface::TcpConnect for VlessNet {
    async fn tcp_connect(
        &self,
        ctx: &mut rd_interface::Context,
        addr: &Address,
    ) -> Result<TcpStream> {
        let shared_read_raw = self
            .flow
            .is_vision()
            .then(|| Arc::new(AtomicBool::new(false)));
        let stream = self.connect_stream(ctx, shared_read_raw.clone()).await?;
        let header = build_request_header(&self.user_id, &self.flow, COMMAND_TCP, addr)
            .map_err(rd_interface::error::map_other)?;
        let stream = ResponseHeaderStream::new(PrefixWriteStream::new(stream, header));

        let stream = if self.flow.is_vision() {
            TcpStream::from(VisionStream::new_with_shared(
                stream,
                &self.user_id,
                shared_read_raw,
            ))
        } else {
            TcpStream::from(stream)
        };
        Ok(stream)
    }
}

#[async_trait]
impl rd_interface::UdpBind for VlessNet {
    async fn udp_bind(
        &self,
        ctx: &mut rd_interface::Context,
        _addr: &Address,
    ) -> Result<UdpSocket> {
        if !self.udp {
            return Err(Error::NotEnabled);
        }
        if !self.flow.is_vision() {
            return Err(Error::other(
                "vless udp currently requires xtls-rprx-vision",
            ));
        }

        let shared_read_raw = Arc::new(AtomicBool::new(false));
        let mut stream = self
            .connect_stream(ctx, Some(shared_read_raw.clone()))
            .await?;
        write_request_header(
            &mut stream,
            &self.user_id,
            &self.flow,
            COMMAND_MUX,
            &Address::Domain(MUX_DOMAIN.to_string(), MUX_PORT),
        )
        .await?;
        let stream = VisionStream::new_with_shared(
            ResponseHeaderStream::new(stream),
            &self.user_id,
            Some(shared_read_raw),
        );
        let framed = Framed::new(stream, XudpCodec::client());
        Ok(VlessUdp {
            framed,
            flushing: false,
        }
        .into_dyn())
    }
}

impl INet for VlessNet {
    fn provide_tcp_connect(&self) -> Option<&dyn rd_interface::TcpConnect> {
        Some(self)
    }

    fn provide_udp_bind(&self) -> Option<&dyn rd_interface::UdpBind> {
        Some(self)
    }
}

struct VlessUdp<S> {
    framed: Framed<S, XudpCodec>,
    flushing: bool,
}

#[async_trait]
impl<S> rd_interface::IUdpSocket for VlessUdp<S>
where
    S: rd_interface::AsyncRead + rd_interface::AsyncWrite + Unpin + Send + Sync + 'static,
{
    async fn local_addr(&self) -> Result<SocketAddr> {
        Err(rd_interface::NOT_IMPLEMENTED)
    }

    fn poll_recv_from(
        &mut self,
        cx: &mut std::task::Context<'_>,
        buf: &mut rd_interface::ReadBuf,
    ) -> Poll<io::Result<SocketAddr>> {
        let (bytes, from) = match ready!(self.framed.poll_next_unpin(cx)) {
            Some(r) => r?,
            None => return Poll::Ready(Err(io::ErrorKind::UnexpectedEof.into())),
        };
        let from = match from {
            Address::SocketAddr(addr) => addr,
            Address::Domain(_, _) => {
                return Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "xudp response returned a domain address",
                )))
            }
        };
        let to_copy = bytes.len().min(buf.remaining());
        buf.put_slice(&bytes[..to_copy]);
        Poll::Ready(Ok(from))
    }

    fn poll_send_to(
        &mut self,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
        target: &Address,
    ) -> Poll<io::Result<usize>> {
        loop {
            if self.flushing {
                ready!(self.framed.poll_flush_unpin(cx))?;
                self.flushing = false;
                return Poll::Ready(Ok(buf.len()));
            }
            ready!(self.framed.poll_ready_unpin(cx))?;
            self.framed
                .start_send_unpin((Bytes::copy_from_slice(buf), target.clone()))?;
            self.flushing = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use rd_interface::IntoAddress;
    use rd_interface::IntoDyn;
    use rd_std::tests::{assert_net_provider, ProviderCapability, TestNet};

    use super::*;

    #[test]
    fn test_provider() {
        let net = TestNet::new().into_dyn();
        let vless = VlessNet::new(VlessNetConfig {
            net: NetRef::new_with_value("test".into(), net),
            server: "127.0.0.1:443".into_address().unwrap(),
            id: "27848739-7e61-4ea0-ba56-d8edf2587d12".to_string(),
            flow: default_flow(),
            sni: Some("localhost".to_string()),
            skip_cert_verify: true,
            udp: true,
            client_fingerprint: None,
            reality_public_key: None,
            reality_short_id: None,
        })
        .unwrap()
        .into_dyn();

        assert_net_provider(
            &vless,
            ProviderCapability {
                tcp_connect: true,
                udp_bind: true,
                ..Default::default()
            },
        );
    }
}
