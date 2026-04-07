use net::RawNet;
use rd_interface::{Registry, Result};
use server::RawServer;
use tun_server::TunServer;

mod config;
mod device;
pub mod fake_ip;
mod forward;
mod gateway;
mod net;
mod route_setup;
mod server;
mod tun_server;
mod wrap;

pub fn init(registry: &mut Registry) -> Result<()> {
    registry.add_net::<RawNet>();
    registry.add_server::<RawServer>();
    registry.add_server::<TunServer>();

    Ok(())
}
