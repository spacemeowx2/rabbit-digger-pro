use std::{io, net::SocketAddr, task::Poll};

use crate::common::{
    write_request_header, NormalizedFlow, ResponseHeaderStream, UserId, VisionStream, XudpCodec,
    COMMAND_MUX, COMMAND_TCP, FLOW_VISION, MUX_DOMAIN, MUX_PORT,
};
use crate::reality::{connect_reality_stream, RealityConfig};
use bytes::Bytes;
use futures::{ready, SinkExt, StreamExt};
use rd_interface::{
    async_trait, config::NetRef, prelude::*, registry::Builder, Address, Error, INet, IntoDyn, Net,
    Result, TcpConnect, TcpStream, UdpSocket,
};
use rd_std::tls::{TlsNet, TlsNetConfig};
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
    Tls(TlsNet),
    Reality { net: Net, config: RealityConfig },
}

impl VlessNet {
    pub fn new(config: VlessNetConfig) -> Result<Self> {
        let transport = match config.reality_public_key {
            Some(public_key) => Transport::Reality {
                net: config.net.value_cloned(),
                config: RealityConfig {
                    server_name: config.sni.unwrap_or_else(|| config.server.host()),
                    public_key,
                    short_id: config.reality_short_id,
                    client_fingerprint: config.client_fingerprint,
                },
            },
            None => Transport::Tls(TlsNet::build(TlsNetConfig {
                skip_cert_verify: config.skip_cert_verify,
                sni: config.sni,
                net: config.net,
            })?),
        };
        Ok(Self {
            server: config.server,
            user_id: UserId::parse(&config.id)?,
            flow: NormalizedFlow::parse(config.flow)?,
            udp: config.udp,
            transport,
        })
    }

    async fn connect_stream(&self, ctx: &mut rd_interface::Context) -> Result<TcpStream> {
        match &self.transport {
            Transport::Tls(tls) => tls.tcp_connect(ctx, &self.server).await,
            Transport::Reality { net, config } => {
                let stream = net.tcp_connect(ctx, &self.server).await?;
                let reality = connect_reality_stream(stream, config)
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
        let mut stream = self.connect_stream(ctx).await?;
        write_request_header(&mut stream, &self.user_id, &self.flow, COMMAND_TCP, addr).await?;
        let stream = ResponseHeaderStream::new(stream);

        let stream = if self.flow.is_vision() {
            TcpStream::from(VisionStream::new(stream, &self.user_id))
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

        let mut stream = self.connect_stream(ctx).await?;
        write_request_header(
            &mut stream,
            &self.user_id,
            &self.flow,
            COMMAND_MUX,
            &Address::Domain(MUX_DOMAIN.to_string(), MUX_PORT),
        )
        .await?;
        let stream = VisionStream::new(ResponseHeaderStream::new(stream), &self.user_id);
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
