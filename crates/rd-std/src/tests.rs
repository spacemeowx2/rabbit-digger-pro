pub use self::net::TestNet;
use crate::builtin;
use rd_interface::{Context, IntoAddress, Net, ReadBuf, Registry};
use std::time::Duration;
use tokio::{
    io::{self, AsyncReadExt, AsyncWriteExt},
    task::yield_now,
    time::timeout,
};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

mod channel;
mod net;

pub fn get_registry() -> Registry {
    let mut registry = Registry::new();
    builtin::init(&mut registry).unwrap();
    registry
}

pub async fn spawn_echo_server_udp(net: &Net, addr: impl IntoAddress) {
    let mut udp = net
        .udp_bind(&mut Context::new(), &addr.into_address().unwrap())
        .await
        .unwrap();
    tokio::spawn(async move {
        let vec = &mut vec![0; 4096];
        loop {
            let mut buf = ReadBuf::new(vec);
            let addr = udp.recv_from(&mut buf).await.unwrap();
            udp.send_to(buf.filled(), &addr.into()).await.unwrap();
        }
    });
    yield_now().await;
}

pub async fn assert_echo_udp(net: &Net, addr: impl IntoAddress) {
    let target_addr = addr.into_address().unwrap();
    let mut socket = net
        .udp_bind(&mut Context::new(), &"0.0.0.0:0".parse().unwrap())
        .await
        .unwrap();

    socket.send_to(b"hello", &target_addr).await.unwrap();
    let buf = &mut vec![0; 4096];
    let mut buf = ReadBuf::new(buf);
    timeout(DEFAULT_TIMEOUT, socket.recv_from(&mut buf))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(buf.filled(), b"hello");
}

pub async fn spawn_echo_server(net: &Net, addr: impl IntoAddress) {
    let listener = net
        .tcp_bind(&mut Context::new(), &addr.into_address().unwrap())
        .await
        .unwrap();
    tokio::spawn(async move {
        loop {
            let (tcp, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let (mut rx, mut tx) = io::split(tcp);
                io::copy(&mut rx, &mut tx).await.unwrap();
            });
        }
    });
    yield_now().await;
}

pub async fn assert_echo(net: &Net, addr: impl IntoAddress) {
    const BUF: &[u8] = b"asdfasdfasdfasj12312313123";
    let mut tcp = net
        .tcp_connect(&mut Context::new(), &addr.into_address().unwrap())
        .await
        .unwrap();
    tcp.write_all(BUF).await.unwrap();

    let mut buf = [0u8; BUF.len()];
    timeout(DEFAULT_TIMEOUT, tcp.read_exact(&mut buf))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(buf, BUF);
}

#[derive(Default, Debug)]
pub struct ProviderCapability {
    pub tcp_connect: bool,
    pub tcp_bind: bool,
    pub udp_bind: bool,
    pub lookup_host: bool,
}

pub fn assert_net_provider(net: &Net, capability: ProviderCapability) {
    assert_eq!(net.provide_tcp_connect().is_some(), capability.tcp_connect);
    assert_eq!(net.provide_tcp_bind().is_some(), capability.tcp_bind);
    assert_eq!(net.provide_udp_bind().is_some(), capability.udp_bind);
    assert_eq!(net.provide_lookup_host().is_some(), capability.lookup_host);
}
