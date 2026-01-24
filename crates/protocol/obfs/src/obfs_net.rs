use std::net::SocketAddr;

use crate::{Obfs, ObfsType};
use rd_interface::{
    async_trait, prelude::*, registry::NetRef, Address, Arc, Context, INet, ITcpListener, IntoDyn,
    Net, Result, TcpListener, TcpStream,
};

type BoxObfs = Arc<dyn Obfs + Send + Sync + 'static>;

#[rd_config]
#[derive(Debug)]
pub struct ObfsNetConfig {
    #[serde(default)]
    pub net: NetRef,
    #[serde(flatten)]
    pub obfs_type: ObfsType,
}

pub struct ObfsNet {
    net: Net,
    obfs: Arc<ObfsType>,
}

impl ObfsNet {
    pub fn new(config: ObfsNetConfig) -> Result<Self> {
        Ok(ObfsNet {
            net: config.net.value_cloned(),
            obfs: Arc::new(config.obfs_type),
        })
    }
}

#[async_trait]
impl rd_interface::TcpConnect for ObfsNet {
    async fn tcp_connect(&self, ctx: &mut Context, addr: &Address) -> Result<TcpStream> {
        let tcp = self.net.tcp_connect(ctx, addr).await?;
        Ok(self.obfs.tcp_connect(tcp, ctx, addr)?)
    }
}

#[async_trait]
impl rd_interface::TcpBind for ObfsNet {
    async fn tcp_bind(&self, ctx: &mut Context, addr: &Address) -> Result<TcpListener> {
        let listener = self.net.tcp_bind(ctx, addr).await?;
        Ok(ObfsTcpListener(listener, self.obfs.clone()).into_dyn())
    }
}

impl INet for ObfsNet {
    fn provide_tcp_connect(&self) -> Option<&dyn rd_interface::TcpConnect> {
        Some(self)
    }

    fn provide_tcp_bind(&self) -> Option<&dyn rd_interface::TcpBind> {
        Some(self)
    }
}

struct ObfsTcpListener(TcpListener, BoxObfs);

#[async_trait]
impl ITcpListener for ObfsTcpListener {
    async fn accept(&self) -> Result<(TcpStream, SocketAddr)> {
        let (tcp, addr) = self.0.accept().await?;
        Ok((self.1.tcp_accept(tcp, addr)?, addr))
    }

    async fn local_addr(&self) -> Result<SocketAddr> {
        self.0.local_addr().await
    }
}

#[cfg(test)]
mod tests {
    use rd_std::tests::{assert_net_provider, ProviderCapability, TestNet};

    use super::*;

    #[test]
    fn test_provider() {
        let net = TestNet::new().into_dyn();

        let obfs = ObfsNet::new(ObfsNetConfig {
            net: NetRef::new_with_value("test".into(), net.clone()),
            obfs_type: ObfsType::Plain(Default::default()),
        })
        .unwrap()
        .into_dyn();

        assert_net_provider(
            &obfs,
            ProviderCapability {
                tcp_connect: true,
                tcp_bind: true,
                ..Default::default()
            },
        );
    }
}
