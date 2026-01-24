use net::RawNet;
use rd_interface::{Registry, Result};
use server::RawServer;

mod config;
mod device;
mod forward;
mod gateway;
mod net;
mod server;
mod wrap;

pub fn init(registry: &mut Registry) -> Result<()> {
    registry.add_net::<RawNet>();
    registry.add_server::<RawServer>();

    Ok(())
}
