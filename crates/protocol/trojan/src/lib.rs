use client::{TrojanNet, TrojanNetConfig, TrojancNetConfig};
use rd_interface::{registry::Builder, Net, Registry, Result};

mod client;
mod stream;
mod websocket;

impl Builder<Net> for TrojanNet {
    const NAME: &'static str = "trojan";
    type Config = TrojanNetConfig;
    type Item = Self;

    fn build(config: Self::Config) -> Result<Self> {
        TrojanNet::new_trojan(config)
    }
}

pub struct TrojancNet;
impl Builder<Net> for TrojancNet {
    const NAME: &'static str = "trojanc";
    type Config = TrojancNetConfig;
    type Item = TrojanNet;

    fn build(config: Self::Config) -> Result<TrojanNet> {
        TrojanNet::new_trojanc(config)
    }
}

pub fn init(registry: &mut Registry) -> Result<()> {
    registry.add_net::<TrojanNet>();
    registry.add_net::<TrojancNet>();

    Ok(())
}
