use backend::*;
use rd_interface::{
    async_trait, config::NetRef, prelude::*, rd_config, registry::Builder, Address, INet, Net,
    Registry, Result, TcpStream,
};

#[cfg(feature = "rustls")]
#[path = "tls/rustls.rs"]
mod backend;

#[cfg(feature = "openssl")]
#[path = "tls/openssl.rs"]
mod backend;

#[cfg(feature = "native-tls")]
#[path = "tls/native-tls.rs"]
mod backend;

#[derive(Clone)]
pub(crate) struct TlsConnectorConfig {
    pub skip_cert_verify: bool,
}

#[rd_config]
pub struct TlsNetConfig {
    /// Dangerous, but can be used to skip certificate verification.
    #[serde(default)]
    pub skip_cert_verify: bool,

    /// Override domain with SNI
    #[serde(default)]
    pub sni: Option<String>,

    #[serde(default)]
    pub net: NetRef,
}

pub struct TlsNet {
    connector: TlsConnector,
    sni: Option<String>,
    net: Net,
}

#[async_trait]
impl rd_interface::TcpConnect for TlsNet {
    async fn tcp_connect(
        &self,
        ctx: &mut rd_interface::Context,
        addr: &Address,
    ) -> Result<TcpStream> {
        let stream = self.net.tcp_connect(ctx, addr).await?;
        let tls_stream = match &self.sni {
            Some(d) => self.connector.connect(d, stream).await?,
            None => self.connector.connect(&addr.host(), stream).await?,
        };

        Ok(TcpStream::from(tls_stream))
    }
}

impl INet for TlsNet {
    fn provide_tcp_connect(&self) -> Option<&dyn rd_interface::TcpConnect> {
        Some(self)
    }
}

impl Builder<Net> for TlsNet {
    const NAME: &'static str = "tls";

    type Config = TlsNetConfig;

    type Item = TlsNet;

    fn build(cfg: Self::Config) -> Result<Self::Item> {
        Ok(TlsNet {
            connector: TlsConnector::new(TlsConnectorConfig {
                skip_cert_verify: cfg.skip_cert_verify,
            })?,
            sni: cfg.sni,
            net: cfg.net.value_cloned(),
        })
    }
}

pub fn init(registry: &mut Registry) -> Result<()> {
    registry.add_net::<TlsNet>();
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::tests::{assert_net_provider, ProviderCapability, TestNet};
    use rd_interface::IntoDyn;

    use super::*;

    #[test]
    fn test_provider() {
        let net = TestNet::new().into_dyn();

        let tls = TlsNet {
            connector: TlsConnector::new(TlsConnectorConfig {
                skip_cert_verify: false,
            })
            .unwrap(),
            sni: None,
            net,
        }
        .into_dyn();

        assert_net_provider(
            &tls,
            ProviderCapability {
                tcp_connect: true,
                ..Default::default()
            },
        );
    }
}
