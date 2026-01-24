mod any;
pub mod config;
mod domain;
mod geoip;
mod ipcidr;
mod matcher;
mod rule_net;

use rd_interface::{registry::Builder, Net, Registry, Result};

impl Builder<Net> for rule_net::RuleNet {
    const NAME: &'static str = "rule";
    type Config = config::RuleNetConfig;
    type Item = Self;

    fn build(config: Self::Config) -> Result<Self> {
        rule_net::RuleNet::new(config)
    }
}

pub fn init(registry: &mut Registry) -> Result<()> {
    registry.add_net::<rule_net::RuleNet>();
    Ok(())
}
