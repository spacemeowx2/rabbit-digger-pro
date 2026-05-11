use rd_interface::{registry::Builder, Net, Registry, Result};

mod client;
mod proto;

pub use client::{AnyTlsNet, AnyTlsNetConfig};

impl Builder<Net> for AnyTlsNet {
    const NAME: &'static str = "anytls";
    type Config = AnyTlsNetConfig;
    type Item = Self;

    fn build(config: Self::Config) -> Result<Self> {
        AnyTlsNet::new(config)
    }
}

pub fn init(registry: &mut Registry) -> Result<()> {
    registry.add_net::<AnyTlsNet>();
    Ok(())
}
