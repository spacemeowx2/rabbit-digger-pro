#[cfg(target_os = "linux")]
mod origin_addr;
#[cfg(target_os = "linux")]
mod redir;
#[cfg(target_os = "linux")]
mod socket;
#[cfg(target_os = "linux")]
mod tproxy;

use rd_interface::{Registry, Result};
#[cfg(target_os = "linux")]
use redir::RedirServer;
#[cfg(target_os = "linux")]
use tproxy::TProxyServer;

pub fn init(_registry: &mut Registry) -> Result<()> {
    #[cfg(target_os = "linux")]
    _registry.add_server::<RedirServer>();
    #[cfg(target_os = "linux")]
    _registry.add_server::<TProxyServer>();
    Ok(())
}
