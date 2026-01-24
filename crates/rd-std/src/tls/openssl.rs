use std::pin::Pin;

use super::TlsConnectorConfig;
use openssl::ssl::{SslConnector, SslMethod, SslVerifyMode};
use openssl_crate as openssl;
use rd_interface::{error::map_other, AsyncRead, AsyncWrite, Result};

pub use tokio_openssl::SslStream as TlsStream;

pub struct TlsConnector {
    connector: SslConnector,
}

impl TlsConnector {
    pub(crate) fn new(config: TlsConnectorConfig) -> Result<TlsConnector> {
        let mut builder = SslConnector::builder(SslMethod::tls()).map_err(map_other)?;

        if config.skip_cert_verify {
            builder.set_verify(SslVerifyMode::NONE);
        }

        Ok(TlsConnector {
            connector: builder.build(),
        })
    }

    pub async fn connect<IO>(&self, domain: &str, stream: IO) -> Result<TlsStream<IO>>
    where
        IO: AsyncRead + AsyncWrite + Unpin,
    {
        let ssl = self
            .connector
            .configure()
            .map_err(map_other)?
            .into_ssl(domain)
            .map_err(map_other)?;

        let mut stream = TlsStream::new(ssl, stream).map_err(map_other)?;

        Pin::new(&mut stream).connect().await.map_err(map_other)?;

        Ok(stream)
    }
}
