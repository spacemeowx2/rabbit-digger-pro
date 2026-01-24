use std::{
    io,
    net::SocketAddr,
    pin::Pin,
    task::{self, Poll},
};

use futures::{ready, Future, FutureExt};
use parking_lot::Mutex;
use rd_interface::{async_trait, Address, IUdpSocket, Result, UdpSocket};
use tokio::sync::Semaphore;
use tokio_util::sync::PollSemaphore;

type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;
type Connector = Box<dyn FnOnce(&[u8], &Address) -> BoxFuture<Result<UdpSocket>> + Send>;

enum State {
    Idle {
        connector: Mutex<Option<Connector>>,
    },
    Binding {
        fut: Mutex<BoxFuture<Result<UdpSocket>>>,
    },
    Binded(UdpSocket),
}

/// A UdpConnector is a UdpSocket that lazily binds when first packet is about to send.
pub struct UdpConnector {
    semaphore: PollSemaphore,
    state: State,
}

impl UdpConnector {
    pub fn new(connector: Connector) -> Self {
        UdpConnector {
            semaphore: PollSemaphore::new(Semaphore::new(0).into()),
            state: State::Idle {
                connector: Mutex::new(Some(connector)),
            },
        }
    }
}

#[async_trait]
impl IUdpSocket for UdpConnector {
    async fn local_addr(&self) -> Result<SocketAddr> {
        match &self.state {
            State::Binded(udp) => udp.local_addr().await,
            State::Idle { .. } | State::Binding { .. } => Err(rd_interface::Error::NotFound(
                "UdpConnector is not connected".to_string(),
            )),
        }
    }

    fn poll_recv_from(
        &mut self,
        cx: &mut task::Context<'_>,
        buf: &mut rd_interface::ReadBuf,
    ) -> Poll<io::Result<SocketAddr>> {
        loop {
            match &mut self.state {
                State::Binded(udp) => return udp.poll_recv_from(cx, buf),
                State::Idle { .. } | State::Binding { .. } => {
                    ready!(self.semaphore.poll_acquire(cx));
                }
            }
        }
    }

    fn poll_send_to(
        &mut self,
        cx: &mut task::Context<'_>,
        buf: &[u8],
        target: &Address,
    ) -> Poll<io::Result<usize>> {
        loop {
            let mut result: Option<io::Result<usize>> = None;

            match &mut self.state {
                State::Binded(ref mut udp) => {
                    result = Some(ready!(udp.poll_send_to(cx, buf, target)));
                }
                State::Binding { fut, .. } => {
                    let udp = ready!(fut.get_mut().poll_unpin(cx));
                    self.semaphore.add_permits(1);
                    self.state = State::Binded(udp?)
                }
                State::Idle { connector } => {
                    let connector = connector
                        .get_mut()
                        .take()
                        .expect("connector shouldn't be None");
                    let fut = connector(buf, target);
                    self.state = State::Binding {
                        fut: Mutex::new(fut),
                    }
                }
            };

            if let Some(result) = result {
                return Poll::Ready(result);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use rd_interface::{Context, IntoAddress, IntoDyn};

    use crate::tests::{spawn_echo_server_udp, TestNet};

    use super::*;

    #[tokio::test]
    async fn test_udp_connector() {
        let net = TestNet::new().into_dyn();
        let net2 = net.clone();
        let mut udp = UdpConnector::new(Box::new(move |buf, target| {
            let buf = buf.to_vec();
            let target = target.clone();
            Box::pin(async move {
                let mut udp = net2
                    .udp_bind(&mut Context::new(), &"0.0.0.0:0".into_address().unwrap())
                    .await
                    .unwrap();

                udp.send_to(&buf, &target).await?;

                Ok(udp)
            })
        }))
        .into_dyn();

        spawn_echo_server_udp(&net, "127.0.0.1:26666").await;

        let target_addr: Address = "127.0.0.1:26666".parse().unwrap();

        udp.send_to(b"hello", &target_addr).await.unwrap();

        let mut buf = vec![0u8; 1024];
        let mut buf = rd_interface::ReadBuf::new(&mut buf);
        let addr = udp.recv_from(&mut buf).await.unwrap();

        assert_eq!(buf.filled(), b"hello");
        assert_eq!(Address::SocketAddr(addr), target_addr);
    }
}
