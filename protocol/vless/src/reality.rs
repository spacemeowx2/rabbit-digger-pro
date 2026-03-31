use std::{
    io::{self, ErrorKind, Read, Write},
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rd_interface::{error::map_other, AsyncRead, AsyncWrite, Result};
use reality::{RealityConnectionState, X25519RealityGroup};
use reality_rustls::crypto::ring::default_provider;
use reality_rustls::pki_types::ServerName;
use reality_rustls::{client::WebPkiServerVerifier, ClientConfig, ClientConnection, RootCertStore};
use tokio::io::ReadBuf;

#[derive(Clone, Debug)]
pub(crate) struct RealityConfig {
    pub server_name: String,
    pub public_key: String,
    pub short_id: Option<String>,
    pub client_fingerprint: Option<String>,
}

impl RealityConfig {
    pub fn validate_client_fingerprint(&self) -> Result<()> {
        if let Some(fp) = self.client_fingerprint.as_deref() {
            if !fp.is_empty() && !fp.eq_ignore_ascii_case("chrome") {
                return Err(rd_interface::Error::other(format!(
                    "unsupported reality client fingerprint: {fp}"
                )));
            }
        }
        Ok(())
    }

    fn decode_public_key(&self) -> io::Result<[u8; 32]> {
        let mut public_key_bytes = [0u8; 32];
        if let Ok(b) = hex::decode(&self.public_key) {
            if b.len() == 32 {
                public_key_bytes.copy_from_slice(&b);
                return Ok(public_key_bytes);
            }
        }
        let decoded = URL_SAFE_NO_PAD
            .decode(&self.public_key)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        if decoded.len() != 32 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid reality public key length",
            ));
        }
        public_key_bytes.copy_from_slice(&decoded);
        Ok(public_key_bytes)
    }

    fn decode_short_id(&self) -> io::Result<[u8; 8]> {
        let mut short_id_bytes = [0u8; 8];
        if let Some(short_id) = self.short_id.as_deref() {
            if !short_id.is_empty() {
                let padded_short_id = format!("{:0<16}", short_id);
                hex::decode_to_slice(&padded_short_id[..16], &mut short_id_bytes).map_err(|e| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("invalid reality short id: {e}"),
                    )
                })?;
            }
        }
        Ok(short_id_bytes)
    }
}

#[derive(Debug)]
struct DebugVerifier(Arc<dyn reality_rustls::client::danger::ServerCertVerifier>);

impl reality_rustls::client::danger::ServerCertVerifier for DebugVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &reality_rustls::pki_types::CertificateDer<'_>,
        intermediates: &[reality_rustls::pki_types::CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: reality_rustls::pki_types::UnixTime,
    ) -> std::result::Result<
        reality_rustls::client::danger::ServerCertVerified,
        reality_rustls::Error,
    > {
        self.0
            .verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now)
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &reality_rustls::pki_types::CertificateDer<'_>,
        dss: &reality_rustls::DigitallySignedStruct,
    ) -> std::result::Result<
        reality_rustls::client::danger::HandshakeSignatureValid,
        reality_rustls::Error,
    > {
        self.0.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &reality_rustls::pki_types::CertificateDer<'_>,
        dss: &reality_rustls::DigitallySignedStruct,
    ) -> std::result::Result<
        reality_rustls::client::danger::HandshakeSignatureValid,
        reality_rustls::Error,
    > {
        self.0.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<reality_rustls::SignatureScheme> {
        self.0.supported_verify_schemes()
    }

    fn root_hint_subjects(&self) -> Option<&[reality_rustls::DistinguishedName]> {
        self.0.root_hint_subjects()
    }
}

fn create_reality_provider() -> Arc<reality_rustls::crypto::CryptoProvider> {
    let mut provider = default_provider();
    let mut new_kx_groups = vec![];
    for group in provider.kx_groups.iter() {
        if group.name() == reality_rustls::NamedGroup::X25519 {
            new_kx_groups
                .push(&X25519RealityGroup as &'static dyn reality_rustls::crypto::SupportedKxGroup);
        } else {
            new_kx_groups.push(*group);
        }
    }
    provider.kx_groups = new_kx_groups;
    Arc::new(provider)
}

fn build_rustls_config(cfg: &RealityConfig) -> io::Result<Arc<ClientConfig>> {
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let verifier = WebPkiServerVerifier::builder(Arc::new(roots))
        .build()
        .map_err(map_other)
        .map_err(rd_interface::Error::to_io_err)?;
    let reality_state = Arc::new(RealityConnectionState::new(
        cfg.decode_public_key()?,
        cfg.decode_short_id()?,
        Arc::new(DebugVerifier(verifier)),
    ));

    let mut config = ClientConfig::builder_with_provider(create_reality_provider())
        .with_safe_default_protocol_versions()
        .unwrap()
        .dangerous()
        .with_custom_certificate_verifier(reality_state.clone())
        .with_no_client_auth();

    config.reality_callback = Some(reality_state);
    config.alpn_protocols = vec![b"h2".to_vec().into(), b"http/1.1".to_vec().into()];

    Ok(Arc::new(config))
}

struct TlsBridge<'a, 'b, S> {
    stream: Pin<&'a mut S>,
    cx: &'a mut Context<'b>,
    safe_byte_read: bool,
}

impl<'a, 'b, S: AsyncRead> Read for TlsBridge<'a, 'b, S> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let read_len = if self.safe_byte_read { 1 } else { buf.len() };
        let mut read_buf = ReadBuf::new(&mut buf[..read_len]);
        match self.stream.as_mut().poll_read(self.cx, &mut read_buf) {
            Poll::Ready(Ok(())) => Ok(read_buf.filled().len()),
            Poll::Ready(Err(e)) => Err(e),
            Poll::Pending => Err(io::Error::new(ErrorKind::WouldBlock, "WouldBlock")),
        }
    }
}

impl<'a, 'b, S: AsyncWrite> Write for TlsBridge<'a, 'b, S> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self.stream.as_mut().poll_write(self.cx, buf) {
            Poll::Ready(Ok(n)) => Ok(n),
            Poll::Ready(Err(e)) => Err(e),
            Poll::Pending => Err(io::Error::new(ErrorKind::WouldBlock, "WouldBlock")),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.stream.as_mut().poll_flush(self.cx) {
            Poll::Ready(Ok(())) => Ok(()),
            Poll::Ready(Err(e)) => Err(e),
            Poll::Pending => Err(io::Error::new(ErrorKind::WouldBlock, "WouldBlock")),
        }
    }
}

pub(crate) struct RealityStream<S> {
    conn: ClientConnection,
    stream: S,
}

impl<S: AsyncRead + AsyncWrite + Unpin> RealityStream<S> {
    pub(crate) fn new(
        config: Arc<ClientConfig>,
        name: ServerName<'static>,
        stream: S,
    ) -> io::Result<Self> {
        let conn = ClientConnection::new(config, name)
            .map_err(map_other)
            .map_err(rd_interface::Error::to_io_err)?;
        Ok(Self { conn, stream })
    }

    pub(crate) async fn perform_handshake(&mut self) -> io::Result<()> {
        std::future::poll_fn(|cx| {
            let mut progress = false;
            while self.conn.is_handshaking() {
                while self.conn.wants_write() {
                    let mut bridge = TlsBridge {
                        stream: Pin::new(&mut self.stream),
                        cx,
                        safe_byte_read: false,
                    };
                    match self.conn.write_tls(&mut bridge) {
                        Ok(n) if n > 0 => progress = true,
                        Ok(_) => break,
                        Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                        Err(e) => return Poll::Ready(Err(e)),
                    }
                }

                if self.conn.wants_read() {
                    let mut bridge = TlsBridge {
                        stream: Pin::new(&mut self.stream),
                        cx,
                        safe_byte_read: false,
                    };
                    match self.conn.read_tls(&mut bridge) {
                        Ok(0) => {
                            return Poll::Ready(Err(io::Error::new(
                                ErrorKind::UnexpectedEof,
                                "connection closed during reality handshake",
                            )));
                        }
                        Ok(_) => {
                            if let Err(e) = self.conn.process_new_packets() {
                                return Poll::Ready(Err(io::Error::new(
                                    ErrorKind::InvalidData,
                                    format!("TLS Error: {e}"),
                                )));
                            }
                            progress = true;
                        }
                        Err(e) if e.kind() == ErrorKind::WouldBlock => {}
                        Err(e) => return Poll::Ready(Err(e)),
                    }
                }

                if !progress {
                    return Poll::Pending;
                }
                progress = false;
            }
            Poll::Ready(Ok(()))
        })
        .await
    }

    fn pump_read(&mut self, cx: &mut Context<'_>) -> io::Result<usize> {
        if self.conn.wants_read() {
            let mut bridge = TlsBridge {
                stream: Pin::new(&mut self.stream),
                cx,
                safe_byte_read: true,
            };
            match self.conn.read_tls(&mut bridge) {
                Ok(0) => return Err(io::Error::new(ErrorKind::UnexpectedEof, "EOF")),
                Ok(n) => {
                    self.conn.process_new_packets().map_err(|e| {
                        io::Error::new(ErrorKind::InvalidData, format!("TLS Error: {e}"))
                    })?;
                    return Ok(n);
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => return Ok(0),
                Err(e) => return Err(e),
            }
        }
        Ok(0)
    }

    fn pump_write(&mut self, cx: &mut Context<'_>) -> io::Result<bool> {
        while self.conn.wants_write() {
            let mut bridge = TlsBridge {
                stream: Pin::new(&mut self.stream),
                cx,
                safe_byte_read: false,
            };
            match self.conn.write_tls(&mut bridge) {
                Ok(_) => {}
                Err(e) if e.kind() == ErrorKind::WouldBlock => return Ok(false),
                Err(e) => return Err(e),
            }
        }
        Ok(true)
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncRead for RealityStream<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let _ = this.pump_write(cx)?;

        loop {
            let slice = buf.initialize_unfilled();
            match this.conn.reader().read(slice) {
                Ok(n) if n > 0 => {
                    buf.advance(n);
                    return Poll::Ready(Ok(()));
                }
                _ => {
                    if this.conn.wants_read() {
                        let n = this.pump_read(cx)?;
                        if n == 0 {
                            return Poll::Pending;
                        }
                    } else if this.conn.wants_write() {
                        let _ = this.pump_write(cx)?;
                        return Poll::Pending;
                    } else {
                        return Poll::Ready(Ok(()));
                    }
                }
            }
        }
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncWrite for RealityStream<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let n = this.conn.writer().write(buf)?;
        let _ = this.pump_write(cx)?;
        Poll::Ready(Ok(n))
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        this.conn.writer().flush()?;
        if !this.pump_write(cx)? {
            return Poll::Pending;
        }
        Pin::new(&mut this.stream).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        this.conn.send_close_notify();
        let _ = this.pump_write(cx)?;
        Pin::new(&mut this.stream).poll_shutdown(cx)
    }
}

pub(crate) async fn connect_reality_stream<S>(
    stream: S,
    cfg: &RealityConfig,
) -> io::Result<RealityStream<S>>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    cfg.validate_client_fingerprint()
        .map_err(rd_interface::Error::to_io_err)?;
    let server_name = ServerName::try_from(cfg.server_name.clone())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    let config = build_rustls_config(cfg)?;
    let mut reality = RealityStream::new(config, server_name, stream)?;
    reality.perform_handshake().await?;
    Ok(reality)
}
