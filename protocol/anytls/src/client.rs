use std::{
    io,
    net::SocketAddr,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use crate::proto::*;
use bytes::BytesMut;
use futures::ready;
use rd_interface::{
    async_trait, config::NetRef, prelude::*, Address, INet, IntoDyn, Result, TcpStream,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{mpsc, Mutex},
};
use tokio_rustls::{
    rustls::{
        client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
        crypto::ring,
        pki_types::{CertificateDer, ServerName, UnixTime},
        ClientConfig, DigitallySignedStruct, RootCertStore,
    },
    TlsConnector,
};

#[rd_config]
#[derive(Debug, Clone)]
pub struct AnyTlsNetConfig {
    #[serde(default)]
    pub(crate) net: NetRef,
    pub(crate) server: Address,
    pub(crate) password: String,
    #[serde(default)]
    pub(crate) sni: Option<String>,
    #[serde(default)]
    pub(crate) skip_cert_verify: bool,
}

pub struct AnyTlsNet {
    net: rd_interface::Net,
    server: Address,
    password: String,
    server_name: String,
    connector: TlsConnector,
    padding: Arc<Mutex<PaddingScheme>>,
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

impl AnyTlsNet {
    pub fn new(config: AnyTlsNetConfig) -> Result<Self> {
        let _ = ring::default_provider().install_default();
        let AnyTlsNetConfig {
            net,
            server,
            password,
            sni,
            skip_cert_verify,
        } = config;
        let server_name = sni.unwrap_or_else(|| server.host());
        let mut roots = RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let roots = Arc::new(roots);
        let builder = ClientConfig::builder();
        let client_config = if skip_cert_verify {
            let verifier = tokio_rustls::rustls::client::WebPkiServerVerifier::builder(roots)
                .build()
                .map_err(rd_interface::error::map_other)?;
            builder
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(AllowAnyCert(verifier)))
                .with_no_client_auth()
        } else {
            builder.with_root_certificates(roots).with_no_client_auth()
        };

        Ok(Self {
            net: net.value_cloned(),
            server,
            password,
            server_name,
            connector: TlsConnector::from(Arc::new(client_config)),
            padding: Arc::new(Mutex::new(PaddingScheme::default())),
        })
    }
}

#[async_trait]
impl rd_interface::TcpConnect for AnyTlsNet {
    async fn tcp_connect(
        &self,
        ctx: &mut rd_interface::Context,
        addr: &Address,
    ) -> Result<TcpStream> {
        let stream = self.net.tcp_connect(ctx, &self.server).await?;
        let server_name = ServerName::try_from(self.server_name.clone())
            .map_err(rd_interface::error::map_other)?;
        let mut tls = self
            .connector
            .connect(server_name, stream)
            .await
            .map_err(rd_interface::error::map_other)?;
        let padding = self.padding.lock().await.clone();
        tls.write_all(&auth_prefix(&self.password, &padding))
            .await?;

        let (mut reader, mut writer) = tokio::io::split(tls);
        let (write_tx, mut write_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let (read_tx, read_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let sid = 1;
        let writer_padding = self.padding.clone();

        tokio::spawn(async move {
            let mut pkt = 1u32;
            while let Some(payload) = write_rx.recv().await {
                let padding = writer_padding.lock().await.clone();
                let chunks = apply_padding(pkt, &padding, payload);
                pkt = pkt.saturating_add(1);
                for chunk in chunks {
                    if !chunk.is_empty() && writer.write_all(&chunk).await.is_err() {
                        let _ = writer.shutdown().await;
                        return;
                    }
                }
            }
            let _ = writer.shutdown().await;
        });

        let recv_write_tx = write_tx.clone();
        let reader_padding = self.padding.clone();
        tokio::spawn(async move {
            let mut header = [0u8; HEADER_LEN];
            loop {
                if reader.read_exact(&mut header).await.is_err() {
                    break;
                }
                let (cmd, got_sid, len) = decode_header(header);
                let mut data = vec![0u8; len as usize];
                if len > 0 && reader.read_exact(&mut data).await.is_err() {
                    break;
                }
                match cmd {
                    CMD_PSH if got_sid == sid => {
                        if read_tx.send(data).is_err() {
                            break;
                        }
                    }
                    CMD_FIN if got_sid == sid => break,
                    CMD_ALERT => break,
                    CMD_UPDATE_PADDING_SCHEME => {
                        if let Some(padding) = PaddingScheme::parse(&data) {
                            *reader_padding.lock().await = padding;
                        }
                    }
                    CMD_HEART_REQUEST => {
                        let _ = recv_write_tx.send(encode_frame(&Frame {
                            cmd: CMD_HEART_RESPONSE,
                            sid: got_sid,
                            data: Vec::new(),
                        }));
                    }
                    CMD_WASTE | CMD_SETTINGS | CMD_SYNACK | CMD_SERVER_SETTINGS
                    | CMD_HEART_RESPONSE => {}
                    _ => {}
                }
            }
        });

        let padding = self.padding.lock().await.clone();
        let mut initial_payload = encode_frame(&Frame {
            cmd: CMD_SETTINGS,
            sid: 0,
            data: encode_settings(&padding),
        });
        initial_payload.extend_from_slice(&encode_frame(&Frame {
            cmd: CMD_SYN,
            sid,
            data: Vec::new(),
        }));
        initial_payload.extend_from_slice(&encode_frame(&Frame {
            cmd: CMD_PSH,
            sid,
            data: encode_socks_addr(addr)?,
        }));
        write_tx
            .send(initial_payload)
            .map_err(|_| rd_interface::Error::other("anytls writer closed"))?;

        Ok(AnyTlsStream {
            sid,
            write_tx,
            read_rx,
            read_buf: BytesMut::new(),
            closed: false,
        }
        .into_dyn())
    }
}

impl INet for AnyTlsNet {
    fn provide_tcp_connect(&self) -> Option<&dyn rd_interface::TcpConnect> {
        Some(self)
    }
}

struct AnyTlsStream {
    sid: u32,
    write_tx: mpsc::UnboundedSender<Vec<u8>>,
    read_rx: mpsc::UnboundedReceiver<Vec<u8>>,
    read_buf: BytesMut,
    closed: bool,
}

#[async_trait]
impl rd_interface::ITcpStream for AnyTlsStream {
    async fn peer_addr(&self) -> Result<SocketAddr> {
        Err(rd_interface::NOT_IMPLEMENTED)
    }

    async fn local_addr(&self) -> Result<SocketAddr> {
        Err(rd_interface::NOT_IMPLEMENTED)
    }

    fn poll_read(
        &mut self,
        cx: &mut Context<'_>,
        buf: &mut rd_interface::ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.read_buf.is_empty() {
            match ready!(Pin::new(&mut self.read_rx).poll_recv(cx)) {
                Some(data) => self.read_buf.extend_from_slice(&data),
                None => return Poll::Ready(Ok(())),
            }
        }
        let n = self.read_buf.len().min(buf.remaining());
        buf.put_slice(&self.read_buf.split_to(n));
        Poll::Ready(Ok(()))
    }

    fn poll_write(&mut self, _cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        if self.closed {
            return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
        }
        if buf.is_empty() {
            return Poll::Ready(Ok(0));
        }
        let n = buf.len().min(u16::MAX as usize);
        self.write_tx
            .send(encode_frame(&Frame {
                cmd: CMD_PSH,
                sid: self.sid,
                data: buf[..n].to_vec(),
            }))
            .map_err(|_| io::Error::from(io::ErrorKind::BrokenPipe))?;
        Poll::Ready(Ok(n))
    }

    fn poll_flush(&mut self, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(&mut self, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if !self.closed {
            self.closed = true;
            let _ = self.write_tx.send(encode_frame(&Frame {
                cmd: CMD_FIN,
                sid: self.sid,
                data: Vec::new(),
            }));
        }
        Poll::Ready(Ok(()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rd_interface::{IntoAddress, IntoDyn};
    use rd_std::tests::{assert_net_provider, ProviderCapability, TestNet};

    #[test]
    fn anytls_net_provides_tcp_connect_only() {
        let net = TestNet::new().into_dyn();
        let anytls = AnyTlsNet::new(AnyTlsNetConfig {
            net: NetRef::new_with_value("test".into(), net),
            server: "127.0.0.1:443".into_address().unwrap(),
            password: "pw".to_string(),
            sni: Some("localhost".to_string()),
            skip_cert_verify: true,
        })
        .unwrap()
        .into_dyn();

        assert_net_provider(
            &anytls,
            ProviderCapability {
                tcp_connect: true,
                ..Default::default()
            },
        );
    }
}
