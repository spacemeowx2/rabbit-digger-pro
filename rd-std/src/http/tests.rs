use super::*;
use crate::tests::{assert_echo, get_registry, spawn_echo_server, TestNet};
use rd_interface::IntoAddress;
use rd_interface::{IServer, IntoDyn};
use std::time::Duration;
use tokio::time::sleep;

#[test]
fn test_http_smoke() {
    let mut registry = get_registry();
    super::init(&mut registry).unwrap();
}

#[tokio::test]
async fn test_http_server_client() {
    let local = TestNet::new().into_dyn();
    spawn_echo_server(&local, "127.0.0.1:26667").await;

    let server = server::Http::new(
        local.clone(),
        local.clone(),
        "127.0.0.1:16667".into_address().unwrap(),
    );
    tokio::spawn(async move { server.start().await });

    sleep(Duration::from_secs(1)).await;

    let client =
        client::HttpClient::new(local, "127.0.0.1:16667".into_address().unwrap()).into_dyn();

    assert_echo(&client, "127.0.0.1:26667").await;
}

#[tokio::test]
async fn test_http_server_auth() {
    let local = TestNet::new().into_dyn();
    spawn_echo_server(&local, "127.0.0.1:26668").await;

    // 创建带认证的服务器
    let server = server::Http::with_auth(
        local.clone(),
        local.clone(),
        "127.0.0.1:16668".into_address().unwrap(),
        "testuser".to_string(),
        "testpass".to_string(),
    );
    tokio::spawn(async move { server.start().await });

    sleep(Duration::from_secs(1)).await;

    // 使用带认证的客户端
    let client = client::HttpClient::with_auth(
        local.clone(),
        "127.0.0.1:16668".into_address().unwrap(),
        "testuser".to_string(),
        "testpass".to_string(),
    )
    .into_dyn();

    assert_echo(&client, "127.0.0.1:26668").await;
}
