use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    time::Duration,
};

use rd_interface::{config::NetRef, Context, IServer, IntoAddress, IntoDyn};
use rd_std::builtin::local::{LocalNet, LocalNetConfig};
use tempfile::TempDir;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream, UdpSocket},
    process::{Child, Command},
    time::{sleep, timeout},
};

use crate::{client::VlessNetConfig, server::VlessServerConfig};

const XRAY_IO_TIMEOUT: Duration = Duration::from_secs(20);
// Keep the in-test UDP soak conservative; heavier concurrency lives in
// scripts/vless_xray_reality_e2e.sh to avoid flaky CI under workspace load.
const UDP_STRESS_CLIENTS: usize = 2;
const UDP_STRESS_PACKETS: usize = 8;
const UDP_STRESS_PACING: Duration = Duration::from_millis(5);

fn xray_bin() -> Option<PathBuf> {
    std::env::var_os("XRAY_BIN")
        .map(PathBuf::from)
        .or_else(|| {
            let brew = PathBuf::from("/opt/homebrew/opt/xray/bin/xray");
            brew.exists().then_some(brew)
        })
        .or_else(|| {
            let path = PathBuf::from("/usr/local/opt/xray/bin/xray");
            path.exists().then_some(path)
        })
}

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

async fn spawn_xray(bin: &Path, config_path: &Path, port: u16) -> Child {
    let mut child = Command::new(bin)
        .arg("run")
        .arg("-c")
        .arg(config_path)
        .spawn()
        .unwrap();
    for _ in 0..50 {
        if let Some(status) = child.try_wait().unwrap() {
            panic!("xray exited early with status {status}");
        }
        if TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
            return child;
        }
        sleep(Duration::from_millis(100)).await;
    }
    panic!("timed out waiting for xray to listen on port {port}");
}

async fn xray_x25519(bin: &Path) -> (String, String) {
    let output = Command::new(bin).arg("x25519").output().await.unwrap();
    assert!(output.status.success(), "xray x25519 failed");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let mut private = None;
    let mut public = None;
    for line in stdout.lines() {
        if let Some(value) = line.split(':').nth(1) {
            let value = value.trim().to_string();
            if line.to_ascii_lowercase().contains("private") {
                private = Some(value);
            } else if line.to_ascii_lowercase().contains("public") {
                public = Some(value);
            }
        }
    }
    (private.unwrap(), public.unwrap())
}

fn local_net() -> rd_interface::Net {
    LocalNet::new(LocalNetConfig::default()).into_dyn()
}

async fn run_rdp_udp_burst(client: rd_interface::Net, target: SocketAddr) {
    let target = rd_interface::Address::from(target);
    for client_idx in 0..UDP_STRESS_CLIENTS {
        let mut udp = client
            .clone()
            .udp_bind(&mut Context::new(), &"0.0.0.0:0".into_address().unwrap())
            .await
            .unwrap();

        let warmup = format!("rdp-warmup-{client_idx:02}").into_bytes();
        udp.send_to(&warmup, &target).await.unwrap();
        let mut warmup_buf = vec![0u8; warmup.len() + 32];
        let mut warmup_rb = rd_interface::ReadBuf::new(&mut warmup_buf);
        timeout(XRAY_IO_TIMEOUT, udp.recv_from(&mut warmup_rb))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(warmup_rb.filled(), warmup.as_slice());

        for packet_idx in 0..UDP_STRESS_PACKETS {
            let payload = format!("rdp-{client_idx:02}-{packet_idx:03}").into_bytes();
            udp.send_to(&payload, &target).await.unwrap();
            let mut buf = vec![0u8; payload.len() + 32];
            let mut rb = rd_interface::ReadBuf::new(&mut buf);
            timeout(XRAY_IO_TIMEOUT, udp.recv_from(&mut rb))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(rb.filled(), payload.as_slice());
            sleep(UDP_STRESS_PACING).await;
        }
    }
}

async fn run_udp_burst_via_xray(client_udp_addr: SocketAddr) {
    for client_idx in 0..UDP_STRESS_CLIENTS {
        let udp = UdpSocket::bind("127.0.0.1:0").await.unwrap();

        let warmup = format!("xray-warmup-{client_idx:02}").into_bytes();
        udp.send_to(&warmup, client_udp_addr).await.unwrap();
        let mut warmup_buf = [0u8; 128];
        let (warmup_n, _) = timeout(XRAY_IO_TIMEOUT, udp.recv_from(&mut warmup_buf))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&warmup_buf[..warmup_n], warmup.as_slice());

        for packet_idx in 0..UDP_STRESS_PACKETS {
            let payload = format!("xray-{client_idx:02}-{packet_idx:03}").into_bytes();
            udp.send_to(&payload, client_udp_addr).await.unwrap();
            let mut buf = [0u8; 128];
            let (n, _) = timeout(XRAY_IO_TIMEOUT, udp.recv_from(&mut buf))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(&buf[..n], payload.as_slice());
            sleep(UDP_STRESS_PACING).await;
        }
    }
}

#[tokio::test]
async fn test_xray_server_with_rdp_client_tcp_udp() {
    let Some(bin) = xray_bin() else {
        eprintln!("XRAY_BIN not set; skipping xray interop test");
        return;
    };

    let dir = TempDir::new().unwrap();
    let (cert_path, key_path) = write_test_cert(&dir);
    let uuid = "27848739-7e61-4ea0-ba56-d8edf2587d12";

    let echo_tcp = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let echo_tcp_addr = echo_tcp.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (mut sock, _) = echo_tcp.accept().await.unwrap();
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

    let echo_udp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let echo_udp_addr = echo_udp.local_addr().unwrap();
    tokio::spawn(async move {
        let mut buf = [0u8; 4096];
        loop {
            let (n, from) = echo_udp.recv_from(&mut buf).await.unwrap();
            let _ = echo_udp.send_to(&buf[..n], from).await;
        }
    });

    let server_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let server_addr = server_listener.local_addr().unwrap();
    drop(server_listener);

    let config_path = dir.path().join("server.json");
    std::fs::write(
        &config_path,
        format!(
            r#"{{
  "log": {{ "loglevel": "warning" }},
  "inbounds": [{{
    "listen": "127.0.0.1",
    "port": {server_port},
    "protocol": "vless",
    "settings": {{
      "clients": [{{ "id": "{uuid}", "flow": "xtls-rprx-vision" }}],
      "decryption": "none"
    }},
    "streamSettings": {{
      "network": "tcp",
      "security": "tls",
      "tlsSettings": {{
        "certificates": [{{ "certificateFile": "{cert_path}", "keyFile": "{key_path}" }}]
      }}
    }}
  }}],
  "outbounds": [{{ "protocol": "freedom" }}]
}}"#,
            server_port = server_addr.port(),
        ),
    )
    .unwrap();

    let mut child = spawn_xray(&bin, &config_path, server_addr.port()).await;

    let client = crate::client::VlessNet::new(VlessNetConfig {
        net: NetRef::new_with_value("out".into(), local_net()),
        server: server_addr.into(),
        id: uuid.to_string(),
        flow: Some(crate::common::FLOW_VISION.to_string()),
        sni: Some("localhost".to_string()),
        skip_cert_verify: true,
        udp: true,
        client_fingerprint: None,
        reality_public_key: None,
        reality_short_id: None,
    })
    .unwrap()
    .into_dyn();

    let mut ctx = Context::new();
    let mut tcp = client
        .tcp_connect(&mut ctx, &echo_tcp_addr.to_string().into_address().unwrap())
        .await
        .unwrap();
    tcp.write_all(b"hello").await.unwrap();
    tcp.flush().await.unwrap();
    let mut buf = [0u8; 5];
    timeout(XRAY_IO_TIMEOUT, tcp.read_exact(&mut buf))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&buf, b"hello");

    let mut udp = client
        .udp_bind(&mut Context::new(), &"0.0.0.0:0".into_address().unwrap())
        .await
        .unwrap();
    udp.send_to(b"ping", &echo_udp_addr.into()).await.unwrap();
    let mut ubuf = vec![0u8; 64];
    let mut rb = rd_interface::ReadBuf::new(&mut ubuf);
    timeout(XRAY_IO_TIMEOUT, udp.recv_from(&mut rb))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(rb.filled(), b"ping");

    let _ = child.kill().await;
}

#[tokio::test]
async fn test_xray_client_with_rdp_server_tcp_udp() {
    let Some(bin) = xray_bin() else {
        eprintln!("XRAY_BIN not set; skipping xray interop test");
        return;
    };

    let dir = TempDir::new().unwrap();
    let (cert_path, key_path) = write_test_cert(&dir);
    let uuid = "27848739-7e61-4ea0-ba56-d8edf2587d12";
    let outbound = local_net();

    let echo_tcp = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let echo_tcp_addr = echo_tcp.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (mut sock, _) = echo_tcp.accept().await.unwrap();
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

    let echo_udp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let echo_udp_addr = echo_udp.local_addr().unwrap();
    tokio::spawn(async move {
        let mut buf = [0u8; 4096];
        loop {
            let (n, from) = echo_udp.recv_from(&mut buf).await.unwrap();
            let _ = echo_udp.send_to(&buf[..n], from).await;
        }
    });

    let probe = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let server_addr = probe.local_addr().unwrap();
    drop(probe);
    let server = crate::server::VlessServer::new(VlessServerConfig {
        bind: server_addr.into(),
        id: uuid.to_string(),
        flow: Some(crate::common::FLOW_VISION.to_string()),
        tls_cert: cert_path,
        tls_key: key_path,
        reality_server_name: None,
        reality_private_key: None,
        reality_short_id: None,
        udp: true,
        net: NetRef::new_with_value("out".into(), outbound.clone()),
        listen: NetRef::new_with_value("out".into(), outbound.clone()),
    })
    .unwrap();
    let server_task = tokio::spawn(async move { server.start().await });
    sleep(Duration::from_secs(1)).await;

    let client_tcp = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client_tcp_addr = client_tcp.local_addr().unwrap();
    drop(client_tcp);
    let client_udp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let client_udp_addr = client_udp.local_addr().unwrap();
    drop(client_udp);

    let config_path = dir.path().join("client.json");
    std::fs::write(
        &config_path,
        format!(
            r#"{{
  "log": {{ "loglevel": "warning" }},
  "inbounds": [
    {{
      "listen": "127.0.0.1",
      "port": {tcp_port},
      "protocol": "dokodemo-door",
      "settings": {{
        "address": "127.0.0.1",
        "port": {echo_tcp_port},
        "network": "tcp"
      }}
    }},
    {{
      "listen": "127.0.0.1",
      "port": {udp_port},
      "protocol": "dokodemo-door",
      "settings": {{
        "address": "127.0.0.1",
        "port": {echo_udp_port},
        "network": "udp"
      }}
    }}
  ],
  "outbounds": [{{
    "protocol": "vless",
    "settings": {{
      "vnext": [{{
        "address": "127.0.0.1",
        "port": {server_port},
        "users": [{{
          "id": "{uuid}",
          "encryption": "none",
          "flow": "xtls-rprx-vision"
        }}]
      }}]
    }},
    "streamSettings": {{
      "network": "tcp",
      "security": "tls",
      "tlsSettings": {{
        "serverName": "localhost",
        "allowInsecure": true
      }}
    }}
  }}]
}}"#,
            tcp_port = client_tcp_addr.port(),
            udp_port = client_udp_addr.port(),
            echo_tcp_port = echo_tcp_addr.port(),
            echo_udp_port = echo_udp_addr.port(),
            server_port = server_addr.port(),
        ),
    )
    .unwrap();

    let mut child = spawn_xray(&bin, &config_path, client_tcp_addr.port()).await;

    let mut tcp = TcpStream::connect(client_tcp_addr).await.unwrap();
    tcp.write_all(b"hello").await.unwrap();
    let mut buf = [0u8; 5];
    timeout(XRAY_IO_TIMEOUT, tcp.read_exact(&mut buf))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&buf, b"hello");

    let udp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    udp.send_to(b"ping", client_udp_addr).await.unwrap();
    let mut ubuf = [0u8; 64];
    let (n, _) = timeout(XRAY_IO_TIMEOUT, udp.recv_from(&mut ubuf))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&ubuf[..n], b"ping");

    let _ = child.kill().await;
    server_task.abort();
}

#[tokio::test]
async fn test_xray_reality_server_with_rdp_client_tcp_udp() {
    let Some(bin) = xray_bin() else {
        eprintln!("XRAY_BIN not set; skipping xray reality interop test");
        return;
    };

    let uuid = "27848739-7e61-4ea0-ba56-d8edf2587d12";
    let dir = TempDir::new().unwrap();
    let (_cert_path, _key_path) = write_test_cert(&dir);
    let (private_key, public_key) = xray_x25519(&bin).await;

    let echo_tcp = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let echo_tcp_addr = echo_tcp.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (mut sock, _) = echo_tcp.accept().await.unwrap();
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

    let echo_udp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let echo_udp_addr = echo_udp.local_addr().unwrap();
    tokio::spawn(async move {
        let mut buf = [0u8; 4096];
        loop {
            let (n, from) = echo_udp.recv_from(&mut buf).await.unwrap();
            let _ = echo_udp.send_to(&buf[..n], from).await;
        }
    });

    let server_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let server_addr = server_listener.local_addr().unwrap();
    drop(server_listener);

    let config_path = dir.path().join("reality-server.json");
    std::fs::write(
        &config_path,
        format!(
            r#"{{
  "log": {{ "loglevel": "warning" }},
  "inbounds": [{{
    "listen": "127.0.0.1",
    "port": {server_port},
    "protocol": "vless",
    "settings": {{
      "clients": [{{ "id": "{uuid}", "flow": "xtls-rprx-vision" }}],
      "decryption": "none"
    }},
    "streamSettings": {{
      "network": "tcp",
      "security": "reality",
      "realitySettings": {{
        "show": false,
        "dest": "www.microsoft.com:443",
        "xver": 0,
        "serverNames": ["www.microsoft.com"],
        "privateKey": "{private_key}",
        "shortIds": ["00000000"]
      }}
    }}
  }}],
  "outbounds": [{{ "protocol": "freedom" }}]
}}"#,
            server_port = server_addr.port(),
        ),
    )
    .unwrap();

    let mut child = spawn_xray(&bin, &config_path, server_addr.port()).await;

    let client = crate::client::VlessNet::new(VlessNetConfig {
        net: NetRef::new_with_value("out".into(), local_net()),
        server: server_addr.into(),
        id: uuid.to_string(),
        flow: Some(crate::common::FLOW_VISION.to_string()),
        sni: Some("www.microsoft.com".to_string()),
        skip_cert_verify: false,
        udp: true,
        client_fingerprint: Some("chrome".to_string()),
        reality_public_key: Some(public_key),
        reality_short_id: Some("00000000".to_string()),
    })
    .unwrap()
    .into_dyn();

    let mut ctx = Context::new();
    let mut tcp = client
        .tcp_connect(&mut ctx, &echo_tcp_addr.to_string().into_address().unwrap())
        .await
        .unwrap();
    tcp.write_all(b"hello").await.unwrap();
    tcp.flush().await.unwrap();
    let mut buf = [0u8; 5];
    timeout(XRAY_IO_TIMEOUT, tcp.read_exact(&mut buf))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&buf, b"hello");

    let mut udp = client
        .udp_bind(&mut Context::new(), &"0.0.0.0:0".into_address().unwrap())
        .await
        .unwrap();
    udp.send_to(b"ping", &echo_udp_addr.into()).await.unwrap();
    let mut ubuf = vec![0u8; 64];
    let mut rb = rd_interface::ReadBuf::new(&mut ubuf);
    timeout(XRAY_IO_TIMEOUT, udp.recv_from(&mut rb))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(rb.filled(), b"ping");

    run_rdp_udp_burst(client.clone(), echo_udp_addr).await;

    let _ = child.kill().await;
}

#[tokio::test]
async fn test_xray_client_with_rdp_reality_server_tcp_udp() {
    let Some(bin) = xray_bin() else {
        eprintln!("XRAY_BIN not set; skipping xray reality interop test");
        return;
    };

    let uuid = "27848739-7e61-4ea0-ba56-d8edf2587d12";
    let (private_key, public_key) = xray_x25519(&bin).await;
    let outbound = local_net();

    let echo_tcp = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let echo_tcp_addr = echo_tcp.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (mut sock, _) = echo_tcp.accept().await.unwrap();
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

    let echo_udp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let echo_udp_addr = echo_udp.local_addr().unwrap();
    tokio::spawn(async move {
        let mut buf = [0u8; 4096];
        loop {
            let (n, from) = echo_udp.recv_from(&mut buf).await.unwrap();
            let _ = echo_udp.send_to(&buf[..n], from).await;
        }
    });

    let probe = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let server_addr = probe.local_addr().unwrap();
    drop(probe);
    let server = crate::server::VlessServer::new(VlessServerConfig {
        bind: server_addr.into(),
        id: uuid.to_string(),
        flow: Some(crate::common::FLOW_VISION.to_string()),
        tls_cert: String::new(),
        tls_key: String::new(),
        reality_server_name: Some("www.microsoft.com".to_string()),
        reality_private_key: Some(private_key),
        reality_short_id: Some("00000000".to_string()),
        udp: true,
        net: NetRef::new_with_value("out".into(), outbound.clone()),
        listen: NetRef::new_with_value("out".into(), outbound.clone()),
    })
    .unwrap();
    let server_task = tokio::spawn(async move { server.start().await });
    sleep(Duration::from_secs(1)).await;

    let client_tcp = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client_tcp_addr = client_tcp.local_addr().unwrap();
    drop(client_tcp);
    let client_udp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let client_udp_addr = client_udp.local_addr().unwrap();
    drop(client_udp);

    let dir = TempDir::new().unwrap();
    let config_path = dir.path().join("reality-client.json");
    std::fs::write(
        &config_path,
        format!(
            r#"{{
  "log": {{ "loglevel": "warning" }},
  "inbounds": [
    {{
      "listen": "127.0.0.1",
      "port": {tcp_port},
      "protocol": "dokodemo-door",
      "settings": {{
        "address": "127.0.0.1",
        "port": {echo_tcp_port},
        "network": "tcp"
      }}
    }},
    {{
      "listen": "127.0.0.1",
      "port": {udp_port},
      "protocol": "dokodemo-door",
      "settings": {{
        "address": "127.0.0.1",
        "port": {echo_udp_port},
        "network": "udp"
      }}
    }}
  ],
  "outbounds": [{{
    "protocol": "vless",
    "settings": {{
      "vnext": [{{
        "address": "127.0.0.1",
        "port": {server_port},
        "users": [{{
          "id": "{uuid}",
          "encryption": "none",
          "flow": "xtls-rprx-vision"
        }}]
      }}]
    }},
    "streamSettings": {{
      "network": "tcp",
      "security": "reality",
      "realitySettings": {{
        "show": false,
        "serverName": "www.microsoft.com",
        "fingerprint": "chrome",
        "publicKey": "{public_key}",
        "shortId": "00000000"
      }}
    }}
  }}]
}}"#,
            tcp_port = client_tcp_addr.port(),
            udp_port = client_udp_addr.port(),
            echo_tcp_port = echo_tcp_addr.port(),
            echo_udp_port = echo_udp_addr.port(),
            server_port = server_addr.port(),
        ),
    )
    .unwrap();

    let mut child = spawn_xray(&bin, &config_path, client_tcp_addr.port()).await;

    let mut tcp = TcpStream::connect(client_tcp_addr).await.unwrap();
    tcp.write_all(b"hello").await.unwrap();
    let mut buf = [0u8; 5];
    timeout(XRAY_IO_TIMEOUT, tcp.read_exact(&mut buf))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&buf, b"hello");

    let udp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    udp.send_to(b"ping", client_udp_addr).await.unwrap();
    let mut ubuf = [0u8; 64];
    let (n, _) = timeout(XRAY_IO_TIMEOUT, udp.recv_from(&mut ubuf))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&ubuf[..n], b"ping");

    run_udp_burst_via_xray(client_udp_addr).await;

    let _ = child.kill().await;
    server_task.abort();
}
