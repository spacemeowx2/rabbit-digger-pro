use std::{net::SocketAddr, time::Duration};

use rd_interface::{config::NetRef, Context, IntoAddress, IntoDyn, Net};
use rd_std::builtin::local::{LocalNet, LocalNetConfig};
use tempfile::TempDir;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, UdpSocket},
    time::timeout,
};

use crate::{
    client::{HysteriaNet, HysteriaNetConfig},
    server::{create_endpoint, serve_endpoint, HysteriaServerConfig},
};

fn write_test_cert(dir: &TempDir) -> (String, String, String) {
    let rcgen::CertifiedKey { cert, key_pair } =
        rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let cert_pem = cert.pem();
    let key_pem = key_pair.serialize_pem();

    let cert_path = dir.path().join("cert.pem");
    let key_path = dir.path().join("key.pem");
    std::fs::write(&cert_path, cert_pem.as_bytes()).unwrap();
    std::fs::write(&key_path, key_pem.as_bytes()).unwrap();

    (
        cert_path.to_string_lossy().to_string(),
        key_path.to_string_lossy().to_string(),
        cert_path.to_string_lossy().to_string(),
    )
}

fn local_net() -> Net {
    LocalNet::new(LocalNetConfig::default()).into_dyn()
}

#[tokio::test]
async fn test_hy2_server_client_tcp() {
    let dir = TempDir::new().unwrap();
    let (cert_path, key_path, ca_path) = write_test_cert(&dir);

    let outbound = local_net();

    let server_cfg = HysteriaServerConfig {
        bind: "127.0.0.1:0".into_address().unwrap(),
        tls_cert: cert_path,
        tls_key: key_path,
        auth: "test-password".to_string(),
        udp: false,
        salamander: None,
        net: NetRef::new_with_value("out".into(), outbound.clone()),
    };

    let endpoint = create_endpoint(&server_cfg).unwrap();
    let server_addr = endpoint.local_addr().unwrap();
    let server_task = tokio::spawn(serve_endpoint(
        endpoint,
        server_cfg.clone(),
        outbound.clone(),
    ));

    let echo = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let echo_addr = echo.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (mut sock, _) = echo.accept().await.unwrap();
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                loop {
                    let n = match sock.read(&mut buf).await {
                        Ok(0) => return,
                        Ok(n) => n,
                        Err(_) => return,
                    };
                    if sock.write_all(&buf[..n]).await.is_err() {
                        return;
                    }
                }
            });
        }
    });

    let client_net = HysteriaNet::new(HysteriaNetConfig {
        server: server_addr.into(),
        auth: "test-password".to_string(),
        server_name: Some("localhost".to_string()),
        ca_pem: Some(ca_path),
        bind: None,
        salamander: None,
        udp: false,
        padding: false,
        cc_rx: 0,
        net: NetRef::new_with_value("out".into(), outbound.clone()),
    })
    .unwrap()
    .into_dyn();

    let mut ctx = Context::new();
    let mut tcp = client_net
        .tcp_connect(&mut ctx, &echo_addr.to_string().into_address().unwrap())
        .await
        .unwrap();

    tcp.write_all(b"hello").await.unwrap();
    let mut buf = [0u8; 5];
    timeout(Duration::from_secs(5), tcp.read_exact(&mut buf))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&buf, b"hello");

    server_task.abort();
}

#[tokio::test]
async fn test_hy2_server_client_udp() {
    let dir = TempDir::new().unwrap();
    let (cert_path, key_path, ca_path) = write_test_cert(&dir);

    let outbound = local_net();

    let server_cfg = HysteriaServerConfig {
        bind: "127.0.0.1:0".into_address().unwrap(),
        tls_cert: cert_path,
        tls_key: key_path,
        auth: "test-password".to_string(),
        udp: true,
        salamander: None,
        net: NetRef::new_with_value("out".into(), outbound.clone()),
    };

    let endpoint = create_endpoint(&server_cfg).unwrap();
    let server_addr = endpoint.local_addr().unwrap();
    let server_task = tokio::spawn(serve_endpoint(
        endpoint,
        server_cfg.clone(),
        outbound.clone(),
    ));

    let echo = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let echo_addr: SocketAddr = echo.local_addr().unwrap();
    tokio::spawn(async move {
        let mut buf = [0u8; 2048];
        loop {
            let (n, from) = echo.recv_from(&mut buf).await.unwrap();
            let _ = echo.send_to(&buf[..n], from).await;
        }
    });

    let client_net = HysteriaNet::new(HysteriaNetConfig {
        server: server_addr.into(),
        auth: "test-password".to_string(),
        server_name: Some("localhost".to_string()),
        ca_pem: Some(ca_path),
        bind: None,
        salamander: None,
        udp: true,
        padding: false,
        cc_rx: 0,
        net: NetRef::new_with_value("out".into(), outbound.clone()),
    })
    .unwrap()
    .into_dyn();

    let mut ctx = Context::new();
    let mut udp = client_net
        .udp_bind(&mut ctx, &"0.0.0.0:0".into_address().unwrap())
        .await
        .unwrap();

    udp.send_to(b"ping", &echo_addr.into()).await.unwrap();
    let mut buf = vec![0u8; 64];
    let mut rb = rd_interface::ReadBuf::new(&mut buf);
    let _from = timeout(Duration::from_secs(5), udp.recv_from(&mut rb))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(rb.filled(), b"ping");

    server_task.abort();
}

#[tokio::test]
async fn test_hy2_server_client_tcp_error_response() {
    let dir = TempDir::new().unwrap();
    let (cert_path, key_path, ca_path) = write_test_cert(&dir);

    let outbound = local_net();

    let server_cfg = HysteriaServerConfig {
        bind: "127.0.0.1:0".into_address().unwrap(),
        tls_cert: cert_path,
        tls_key: key_path,
        auth: "test-password".to_string(),
        udp: false,
        salamander: None,
        net: NetRef::new_with_value("out".into(), outbound.clone()),
    };

    let endpoint = create_endpoint(&server_cfg).unwrap();
    let server_addr = endpoint.local_addr().unwrap();
    let server_task = tokio::spawn(serve_endpoint(
        endpoint,
        server_cfg.clone(),
        outbound.clone(),
    ));

    let client_net = HysteriaNet::new(HysteriaNetConfig {
        server: server_addr.into(),
        auth: "test-password".to_string(),
        server_name: Some("localhost".to_string()),
        ca_pem: Some(ca_path),
        bind: None,
        salamander: None,
        udp: false,
        padding: false,
        cc_rx: 0,
        net: NetRef::new_with_value("out".into(), outbound.clone()),
    })
    .unwrap()
    .into_dyn();

    let mut ctx = Context::new();
    let bad = "127.0.0.1:1".into_address().unwrap();
    let r = timeout(
        Duration::from_secs(2),
        client_net.tcp_connect(&mut ctx, &bad),
    )
    .await;
    assert!(r.is_ok());
    assert!(r.unwrap().is_err());

    server_task.abort();
}

#[tokio::test]
async fn test_hy2_server_client_udp_fragmentation() {
    let dir = TempDir::new().unwrap();
    let (cert_path, key_path, ca_path) = write_test_cert(&dir);

    let outbound = local_net();

    let server_cfg = HysteriaServerConfig {
        bind: "127.0.0.1:0".into_address().unwrap(),
        tls_cert: cert_path,
        tls_key: key_path,
        auth: "test-password".to_string(),
        udp: true,
        salamander: None,
        net: NetRef::new_with_value("out".into(), outbound.clone()),
    };

    let endpoint = create_endpoint(&server_cfg).unwrap();
    let server_addr = endpoint.local_addr().unwrap();
    let server_task = tokio::spawn(serve_endpoint(
        endpoint,
        server_cfg.clone(),
        outbound.clone(),
    ));

    let echo = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let echo_addr: SocketAddr = echo.local_addr().unwrap();
    tokio::spawn(async move {
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let (n, from) = echo.recv_from(&mut buf).await.unwrap();
            let _ = echo.send_to(&buf[..n], from).await;
        }
    });

    let client_net = HysteriaNet::new(HysteriaNetConfig {
        server: server_addr.into(),
        auth: "test-password".to_string(),
        server_name: Some("localhost".to_string()),
        ca_pem: Some(ca_path),
        bind: None,
        salamander: None,
        udp: true,
        padding: false,
        cc_rx: 0,
        net: NetRef::new_with_value("out".into(), outbound.clone()),
    })
    .unwrap()
    .into_dyn();

    let mut ctx = Context::new();
    let mut udp = client_net
        .udp_bind(&mut ctx, &"0.0.0.0:0".into_address().unwrap())
        .await
        .unwrap();

    let payload = vec![0x42u8; 6_000];
    let mut ok = false;
    for _ in 0..10 {
        udp.send_to(&payload, &echo_addr.into()).await.unwrap();
        let mut buf = vec![0u8; 10_000];
        let mut rb = rd_interface::ReadBuf::new(&mut buf);
        match timeout(Duration::from_secs(1), udp.recv_from(&mut rb)).await {
            Ok(Ok(_from)) => {
                if rb.filled() == payload.as_slice() {
                    ok = true;
                    break;
                }
            }
            _ => continue,
        }
    }
    assert!(ok, "did not receive fragmented UDP echo after retries");

    server_task.abort();
}
