use super::*;
use crate::tests::{
    assert_echo, assert_echo_udp, get_registry, spawn_echo_server, spawn_echo_server_udp, TestNet,
};
use rd_interface::IntoAddress;
use rd_interface::{IServer, IntoDyn};
use std::time::Duration;
use tokio::time::sleep;

#[test]
fn test_socks5_smoke() {
    let mut registry = get_registry();
    super::init(&mut registry).unwrap();
}

#[tokio::test]
async fn test_socks5_server_client() {
    let local = TestNet::new().into_dyn();
    spawn_echo_server(&local, "127.0.0.1:26666").await;
    spawn_echo_server_udp(&local, "127.0.0.1:26666").await;

    let server = server::Socks5::new(
        local.clone(),
        local.clone(),
        "127.0.0.1:16666".into_address().unwrap(),
    );
    tokio::spawn(async move { server.start().await });

    sleep(Duration::from_secs(1)).await;

    let client =
        client::Socks5Client::new(local, "127.0.0.1:16666".into_address().unwrap()).into_dyn();

    assert_echo(&client, "127.0.0.1:26666").await;
    assert_echo_udp(&client, "127.0.0.1:26666").await;
}
