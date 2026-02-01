use crate::wrapper::Cipher;

use super::*;
use rd_interface::{config::NetRef, IServer, IntoAddress, IntoDyn, Value};
use rd_std::tests::{
    assert_echo, assert_echo_udp, get_registry, spawn_echo_server, spawn_echo_server_udp, TestNet,
};
use std::time::Duration;
use tokio::time::sleep;

#[test]
fn test_ss_smoke() {
    let mut registry = get_registry();
    super::init(&mut registry).unwrap();
}

#[tokio::test]
async fn test_ss_server_client() {
    let local = TestNet::new().into_dyn();
    spawn_echo_server(&local, "127.0.0.1:26666").await;
    spawn_echo_server_udp(&local, "127.0.0.1:26666").await;

    let server_addr = "127.0.0.1:16666".into_address().unwrap();
    let server_cfg = server::SSServerConfig {
        listen: NetRef::new_with_value("local".to_string().into(), local.clone()),
        net: NetRef::new_with_value("local".to_string().into(), local.clone()),
        bind: server_addr.clone(),
        password: "password".into(),
        udp: true,
        cipher: Cipher::AES_128_GCM,
    };
    let server = server::SSServer::new(server_cfg).unwrap();
    tokio::spawn(async move { server.start().await });

    sleep(Duration::from_secs(1)).await;

    let client_cfg = client::SSNetConfig {
        server: "localhost:16666".into_address().unwrap(),
        password: "password".into(),
        udp: true,
        cipher: Cipher::AES_128_GCM,
        net: NetRef::new_with_value(Value::String("local".to_string()), local.clone()),
    };
    let client = client::SSNet::new(client_cfg).unwrap().into_dyn();

    assert_echo(&client, "127.0.0.1:26666").await;
    assert_echo_udp(&client, "127.0.0.1:26666").await;
}
