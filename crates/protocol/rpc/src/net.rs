use crate::types::Command;
use crate::{connection::Codec, session::ClientSession};

use self::socket::TcpListenerWrapper;

use rd_interface::{async_trait, Address, Context, INet, IntoDyn, Net, Result, TcpStream};

use socket::{TcpWrapper, UdpWrapper};
use tokio::sync::{Mutex, OnceCell};

mod socket;

pub struct RpcNet {
    net: Net,
    endpoint: Address,
    auto_reconnect: bool,

    sess: Mutex<OnceCell<Result<ClientSession>>>,
    codec: Codec,
}

impl RpcNet {
    pub fn new(net: Net, endpoint: Address, auto_reconnect: bool, codec: Codec) -> Self {
        RpcNet {
            net,
            endpoint,
            auto_reconnect,
            sess: Mutex::new(OnceCell::new()),
            codec,
        }
    }
    pub async fn get_sess(&self) -> Result<ClientSession> {
        let mut sess = self.sess.lock().await;
        Ok(loop {
            let client_sess = sess
                .get_or_init(|| ClientSession::new(&self.net, &self.endpoint, self.codec))
                .await
                .as_ref()
                .cloned();
            let client_sess = match client_sess {
                Ok(s) => s,
                Err(e) => {
                    if !self.auto_reconnect {
                        return Err(rd_interface::Error::other(e.to_string()));
                    }
                    tracing::error!("Connection error: {:?}", e);
                    *sess = OnceCell::new();
                    continue;
                }
            };
            if !self.auto_reconnect || !client_sess.is_closed() {
                break client_sess;
            } else {
                tracing::info!("reconnect to server");
                *sess = OnceCell::new();
            }
        })
    }
}

#[async_trait]
impl rd_interface::TcpConnect for RpcNet {
    async fn tcp_connect(&self, ctx: &mut Context, addr: &Address) -> Result<TcpStream> {
        let conn = self.get_sess().await?;
        let (resp, _) = conn
            .send(Command::TcpConnect(ctx.to_value(), addr.clone()), None)
            .await?
            .wait()
            .await?;

        let (obj, ctx_value) = resp.into_object_value()?;

        *ctx = Context::from_value(ctx_value)?;
        let tcp = TcpWrapper::new(conn, obj);

        Ok(tcp.into_dyn())
    }
}

#[async_trait]
impl rd_interface::TcpBind for RpcNet {
    async fn tcp_bind(
        &self,
        ctx: &mut Context,
        addr: &Address,
    ) -> Result<rd_interface::TcpListener> {
        let conn = self.get_sess().await?;
        let (resp, _) = conn
            .send(Command::TcpBind(ctx.to_value(), addr.clone()), None)
            .await?
            .wait()
            .await?;

        let (obj, ctx_value) = resp.into_object_value()?;

        *ctx = Context::from_value(ctx_value)?;
        let listener = TcpListenerWrapper::new(conn, obj);

        Ok(listener.into_dyn())
    }
}

#[async_trait]
impl rd_interface::UdpBind for RpcNet {
    async fn udp_bind(&self, ctx: &mut Context, addr: &Address) -> Result<rd_interface::UdpSocket> {
        let conn = self.get_sess().await?;
        let (resp, _) = conn
            .send(Command::UdpBind(ctx.to_value(), addr.clone()), None)
            .await?
            .wait()
            .await?;

        let (obj, ctx_value) = resp.into_object_value()?;

        *ctx = Context::from_value(ctx_value)?;
        let udp = UdpWrapper::new(conn, obj);

        Ok(udp.into_dyn())
    }
}

#[async_trait]
impl rd_interface::LookupHost for RpcNet {
    async fn lookup_host(&self, addr: &Address) -> Result<Vec<std::net::SocketAddr>> {
        let conn = self.get_sess().await?;
        let getter = conn.send(Command::LookupHost(addr.clone()), None).await?;

        let (resp, _) = getter.wait().await?;

        resp.into_value()
    }
}

impl INet for RpcNet {
    fn provide_tcp_connect(&self) -> Option<&dyn rd_interface::TcpConnect> {
        Some(self)
    }

    fn provide_tcp_bind(&self) -> Option<&dyn rd_interface::TcpBind> {
        Some(self)
    }

    fn provide_udp_bind(&self) -> Option<&dyn rd_interface::UdpBind> {
        Some(self)
    }

    fn provide_lookup_host(&self) -> Option<&dyn rd_interface::LookupHost> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use rd_interface::IntoAddress;
    use rd_std::tests::{assert_net_provider, ProviderCapability, TestNet};

    use super::*;

    #[test]
    fn test_provider() {
        let net = TestNet::new().into_dyn();

        let rpc = RpcNet::new(
            net,
            "127.0.0.1:1234".into_address().unwrap(),
            true,
            Codec::Cbor,
        )
        .into_dyn();

        assert_net_provider(
            &rpc,
            ProviderCapability {
                tcp_connect: true,
                tcp_bind: true,
                udp_bind: true,
                lookup_host: true,
            },
        );
    }
}
