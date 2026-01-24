use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
    time::SystemTime,
};

use super::TlsConnectorConfig;
use futures::ready;
use rd_interface::{error::map_other, AsyncRead, AsyncWrite, Result};
use std::sync::Arc;
use tokio::io::ReadBuf;
use tokio_rustls::rustls::{
    client::{ServerCertVerified, ServerCertVerifier},
    Certificate, ClientConfig, OwnedTrustAnchor, RootCertStore, ServerName,
};

pub type TlsStream<T> = PushingStream<tokio_rustls::TlsStream<T>>;

struct AllowAnyCert;
impl ServerCertVerifier for AllowAnyCert {
    fn verify_server_cert(
        &self,
        _end_entity: &Certificate,
        _intermediates: &[Certificate],
        _server_name: &ServerName,
        _scts: &mut dyn Iterator<Item = &[u8]>,
        _ocsp_response: &[u8],
        _now: SystemTime,
    ) -> Result<ServerCertVerified, tokio_rustls::rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }
}

pub struct TlsConnector {
    connector: tokio_rustls::TlsConnector,
}

impl TlsConnector {
    pub(crate) fn new(config: TlsConnectorConfig) -> Result<TlsConnector> {
        let mut root_cert_store = RootCertStore::empty();
        root_cert_store.add_server_trust_anchors(webpki_roots::TLS_SERVER_ROOTS.0.iter().map(
            |ta| {
                OwnedTrustAnchor::from_subject_spki_name_constraints(
                    ta.subject,
                    ta.spki,
                    ta.name_constraints,
                )
            },
        ));
        let mut client_config = ClientConfig::builder()
            .with_safe_defaults()
            .with_root_certificates(root_cert_store)
            .with_no_client_auth();

        if config.skip_cert_verify {
            client_config
                .dangerous()
                .set_certificate_verifier(Arc::new(AllowAnyCert));
        }

        let connector = tokio_rustls::TlsConnector::from(Arc::new(client_config));

        Ok(TlsConnector { connector })
    }
    pub async fn connect<IO>(&self, domain: &str, stream: IO) -> Result<TlsStream<IO>>
    where
        IO: AsyncRead + AsyncWrite + Unpin,
    {
        let stream = self
            .connector
            .connect(ServerName::try_from(domain).map_err(map_other)?, stream)
            .await?;
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
