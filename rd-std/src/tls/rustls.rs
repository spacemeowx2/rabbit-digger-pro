use std::{
    io,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use super::TlsConnectorConfig;
use futures::ready;
use rd_interface::{error::map_other, AsyncRead, AsyncWrite, Result};
use tokio::io::ReadBuf;
use tokio_rustls::rustls::{
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::ring::default_provider,
    pki_types::{CertificateDer, ServerName, UnixTime},
    ClientConfig, DigitallySignedStruct, RootCertStore,
};

pub type TlsStream<T> = PushingStream<tokio_rustls::client::TlsStream<T>>;

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
    ) -> Result<ServerCertVerified, tokio_rustls::rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, tokio_rustls::rustls::Error> {
        self.0.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, tokio_rustls::rustls::Error> {
        self.0.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<tokio_rustls::rustls::SignatureScheme> {
        self.0.supported_verify_schemes()
    }
}

pub struct TlsConnector {
    connector: tokio_rustls::TlsConnector,
}

fn ensure_rustls_provider_installed() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let _ = default_provider().install_default();
    });
}

impl TlsConnector {
    pub(crate) fn new(config: TlsConnectorConfig) -> Result<TlsConnector> {
        ensure_rustls_provider_installed();

        let mut roots = RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let roots = Arc::new(roots);
        let builder = ClientConfig::builder();
        let client_config = if config.skip_cert_verify {
            let verifier = tokio_rustls::rustls::client::WebPkiServerVerifier::builder(roots)
                .build()
                .map_err(map_other)?;
            builder
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(AllowAnyCert(verifier)))
                .with_no_client_auth()
        } else {
            builder.with_root_certificates(roots).with_no_client_auth()
        };

        let connector = tokio_rustls::TlsConnector::from(Arc::new(client_config));

        Ok(TlsConnector { connector })
    }
    pub async fn connect<IO>(&self, domain: &str, stream: IO) -> Result<TlsStream<IO>>
    where
        IO: AsyncRead + AsyncWrite + Unpin,
    {
        let server_name = ServerName::try_from(domain.to_owned()).map_err(map_other)?;
        let stream = self.connector.connect(server_name, stream).await?;
        Ok(PushingStream::new(stream.into()))
    }
}

enum State {
    Write,
    Flush(usize),
}

pub struct PushingStream<S> {
    inner: S,
    state: State,
}

impl<S> AsyncRead for PushingStream<S>
where
    S: AsyncRead + Unpin,
{
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl<S> AsyncWrite for PushingStream<S>
where
    S: AsyncWrite + Unpin,
{
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let wrote = loop {
            match self.state {
                State::Write => {
                    let wrote = ready!(Pin::new(&mut self.inner).poll_write(cx, buf))?;
                    self.state = State::Flush(wrote);
                }
                State::Flush(wrote) => {
                    ready!(Pin::new(&mut self.inner).poll_flush(cx))?;
                    self.state = State::Write;
                    break wrote;
                }
            }
        };

        Poll::Ready(Ok(wrote))
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

impl<S> PushingStream<S> {
    pub fn new(inner: S) -> Self {
        PushingStream {
            inner,
            state: State::Write,
        }
    }
}
