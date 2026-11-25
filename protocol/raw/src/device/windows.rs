use std::{
    io::{self, Write},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc,
    },
};

use crate::config::{Layer, TunTapSetup};
use once_cell::sync::OnceCell;
use rd_interface::{error::map_other, Error, Result};
use tokio::sync::mpsc::{Receiver, Sender};
use tokio_smoltcp::{
    device::{ChannelCapture, DeviceCapabilities},
    smoltcp::phy::Checksum,
};
use wintun::{Adapter, Wintun};

static WINTUN: OnceCell<Wintun> = OnceCell::new();
static TUN_ADAPTER_INDEX: AtomicU32 = AtomicU32::new(0);
pub(crate) const POOL_NAME: &'static str = "rabbit-digger-pro";
pub(crate) const DEVICE_NAME: &'static str = "rabbit digger pro";

pub(crate) fn adapter_index() -> Option<u32> {
    let index = TUN_ADAPTER_INDEX.load(Ordering::Relaxed);
    (index != 0).then_some(index)
}

fn get_wintun() -> Result<&'static Wintun> {
    WINTUN.get_or_try_init(|| {
        unsafe { wintun::load() }.map_err(|error| {
            rd_interface::Error::other(format!("Failed to load wintun.dll: {error}"))
        })
    })
}

pub fn get_tun(cfg: TunTapSetup) -> Result<ChannelCapture> {
    if let Layer::L2 = cfg.layer {
        return Err(Error::Other("On windows only support tun".into()));
    }

    if !crate::route_setup::is_elevated().map_err(|error| Error::Other(error.into()))? {
        return Err(Error::Other(
            "TUN mode on Windows requires running as administrator".into(),
        ));
    }

    let wintun = get_wintun()?;
    let adapter = match Adapter::open(wintun, POOL_NAME) {
        Ok(a) => a,
        Err(_) => Adapter::create(wintun, POOL_NAME, DEVICE_NAME, None)
            .map_err(|_| rd_interface::Error::other("Failed to create wintun"))?,
    };
    let s1 = Arc::new(
        adapter
            .start_session(wintun::MAX_RING_CAPACITY)
            .map_err(|_| rd_interface::Error::other("Failed to create wintun session"))?,
    );
    let s2 = s1.clone();

    let wintun_adapter_index = adapter
        .get_adapter_index()
        .map_err(|_| rd_interface::Error::other("Failed to get adapter index"))?;
    TUN_ADAPTER_INDEX.store(wintun_adapter_index, Ordering::Relaxed);

    let prefix_len = cfg.destination_addr.prefix_len();

    run_netsh(format!("set interface {} metric=250", wintun_adapter_index))?;
    run_powershell(&format!(
        "$ErrorActionPreference='Stop'; \
         Remove-NetIPAddress -InterfaceIndex {index} -AddressFamily IPv4 -Confirm:$false -ErrorAction SilentlyContinue; \
         New-NetIPAddress -InterfaceIndex {index} -IPAddress {addr} -PrefixLength {prefix} -ErrorAction Stop",
        index = wintun_adapter_index,
        addr = cfg.addr,
        prefix = prefix_len,
    ))?;

    let recv = move |tx: Sender<io::Result<Vec<u8>>>| loop {
        let p = match s1.receive_blocking().map(|p| p.bytes().to_vec()) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("Wintun recv error: {:?}", e);
                break;
            }
        };
        tx.blocking_send(Ok(p)).unwrap();
    };
    let send = move |mut rx: Receiver<Vec<u8>>| {
        while let Some(pkt) = rx.blocking_recv() {
            let mut p = match s2.allocate_send_packet(pkt.len() as u16) {
                Ok(p) => p,
                Err(_) => {
                    eprintln!("Wintun send error");
                    break;
                }
            };
            p.bytes_mut().copy_from_slice(&pkt);
            s2.send_packet(p);
        }
    };

    let mut caps = DeviceCapabilities::default();
    caps.medium = cfg.layer.into();
    caps.max_transmission_unit = cfg.mtu;
    caps.checksum.ipv4 = Checksum::Tx;
    caps.checksum.tcp = Checksum::Tx;
    caps.checksum.udp = Checksum::Tx;
    caps.checksum.icmpv4 = Checksum::Tx;
    caps.checksum.icmpv6 = Checksum::Tx;

    let dev = ChannelCapture::new(recv, send, caps);

    Ok(dev)
}

fn run_netsh(cmd_str: String) -> Result<()> {
    let mut cmd = Command::new("netsh");
    cmd.arg("interface").arg("ip").args(cmd_str.split(" "));
    let output = cmd.stdout(Stdio::inherit()).output().map_err(map_other)?;
    let status = output.status;
    let stdout = output.stdout;
    let stderr = output.stderr;

    if !status.success() || (!stdout.is_empty() && stdout != b"Ok.") {
        tracing::error!(
            "Running process: {:?} failed! Status: {:?}",
            cmd_str,
            status,
        );
        io::stderr().write_all(&stdout)?;
        io::stderr().write_all(&stderr)?;
        return Err(Error::Other("Failed to run netsh".into()));
    }

    Ok(())
}

fn run_powershell(script: &str) -> Result<()> {
    let output = Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .output()
        .map_err(map_other)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Error::Other(
            format!("powershell failed: {}", stderr.trim()).into(),
        ));
    }
    Ok(())
}
