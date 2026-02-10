use client::{HysteriaNet, HysteriaNetConfig};
use rd_interface::{registry::Builder, Net, Registry, Result};

mod client;
mod codec;
mod stream;

impl Builder<Net> for HysteriaNet {
    const NAME: &'static str = "hysteria";
    type Config = HysteriaNetConfig;
    type Item = Self;

    fn build(config: Self::Config) -> Result<Self> {
        HysteriaNet::new(config)
    }
}

pub fn init(registry: &mut Registry) -> Result<()> {
    registry.add_net::<HysteriaNet>();
    Ok(())
}
