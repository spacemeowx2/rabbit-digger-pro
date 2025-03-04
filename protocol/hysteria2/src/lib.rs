use rd_interface::{Registry, Result};

pub(crate) mod client;
pub(crate) mod config;
pub(crate) mod crypto;
pub(crate) mod protocol;

pub fn init(registry: &mut Registry) -> Result<()> {
    registry.add_net::<client::Hysteria2Net>();
    Ok(())
}
