pub use dns_sniffer::DNSSnifferNet;
pub use sni_sniffer::SNISnifferNet;

use rd_interface::{
    prelude::*,
    rd_config,
    registry::{Builder, NetRef},
    Net, Registry, Result,
};

mod dns_sniffer;
mod service;
mod sni_sniffer;

#[rd_config]
#[derive(Debug)]
pub struct DNSNetConfig {
    #[serde(default)]
    net: NetRef,
}

impl Builder<Net> for DNSSnifferNet {
    const NAME: &'static str = "dns_sniffer";
    type Config = DNSNetConfig;
    type Item = Self;

    fn build(config: Self::Config) -> Result<Self> {
        Ok(DNSSnifferNet::new(config.net.value_cloned()))
    }
}

#[rd_config]
#[derive(Debug)]
pub struct SNINetConfig {
    #[serde(default)]
    net: NetRef,
    /// Ports to sniff.
    /// If not set, only 443 port will be sniffed.
    #[serde(default)]
    ports: Option<Vec<u16>>,
    /// Force sniff domain.
    /// By default, only sniff connection to IP address.
    /// If set to true, will sniff all connection.
    #[serde(default)]
    force_sniff: bool,
}

impl Builder<Net> for SNISnifferNet {
    const NAME: &'static str = "sni_sniffer";
    type Config = SNINetConfig;
    type Item = Self;

    fn build(config: Self::Config) -> Result<Self> {
        Ok(SNISnifferNet::new(
            config.net.value_cloned(),
            config.ports,
            config.force_sniff,
        ))
    }
}

pub fn init(registry: &mut Registry) -> Result<()> {
    registry.add_net::<DNSSnifferNet>();
    registry.add_net::<SNISnifferNet>();
    Ok(())
}
