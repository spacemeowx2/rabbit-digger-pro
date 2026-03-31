use std::time::Duration;

use rd_interface::{config::NetRef, IServer, IntoAddress, IntoDyn, Value};
use rd_std::tests::{
    assert_echo, assert_echo_udp, get_registry, spawn_echo_server, spawn_echo_server_udp, TestNet,
};
use tempfile::TempDir;
use tokio::time::sleep;

use super::*;

fn write_test_cert(dir: &TempDir) -> (String, String) {
    let rcgen::CertifiedKey { cert, key_pair } =
        rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let cert_path = dir.path().join("cert.pem");
    let key_path = dir.path().join("key.pem");
    std::fs::write(&cert_path, cert.pem()).unwrap();
    std::fs::write(&key_path, key_pair.serialize_pem()).unwrap();
    (
        cert_path.to_string_lossy().to_string(),
        key_path.to_string_lossy().to_string(),
    )
}

#[test]
fn test_vless_smoke() {
    let mut registry = get_registry();
    super::init(&mut registry).unwrap();
}

#[tokio::test]
async fn test_vless_server_client() {
    let local = TestNet::new().into_dyn();
    spawn_echo_server(&local, "127.0.0.1:26666").await;
    spawn_echo_server_udp(&local, "127.0.0.1:26666").await;

    let dir = TempDir::new().unwrap();
    let (cert_path, key_path) = write_test_cert(&dir);

    let server_addr = "127.0.0.1:16666".into_address().unwrap();
    let server_cfg = server::VlessServerConfig {
        listen: NetRef::new_with_value("local".into(), local.clone()),
        net: NetRef::new_with_value("local".into(), local.clone()),
        bind: server_addr.clone(),
        id: "27848739-7e61-4ea0-ba56-d8edf2587d12".to_string(),
        flow: Some(crate::common::FLOW_VISION.to_string()),
        tls_cert: cert_path,
        tls_key: key_path,
        udp: true,
    };
    let server = server::VlessServer::new(server_cfg).unwrap();
    tokio::spawn(async move { server.start().await });

    sleep(Duration::from_secs(1)).await;

    let client_cfg = client::VlessNetConfig {
        server: "localhost:16666".into_address().unwrap(),
        id: "27848739-7e61-4ea0-ba56-d8edf2587d12".to_string(),
        flow: Some(crate::common::FLOW_VISION.to_string()),
        sni: Some("localhost".to_string()),
        skip_cert_verify: true,
        udp: true,
        net: NetRef::new_with_value(Value::String("local".to_string()), local.clone()),
    };
    let client = client::VlessNet::new(client_cfg).unwrap().into_dyn();

    assert_echo(&client, "127.0.0.1:26666").await;
    assert_echo_udp(&client, "127.0.0.1:26666").await;
}
