use crate::util::NotImplementedNet;
use rd_interface::{config::EmptyConfig, registry::Builder, Net, Result};

pub struct NoopNet;

impl Builder<Net> for NoopNet {
    const NAME: &'static str = "noop";
    type Config = EmptyConfig;
    type Item = NotImplementedNet;

    fn build(_config: Self::Config) -> Result<Self::Item> {
        Ok(NotImplementedNet)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_noop_builder() {
        let result = NoopNet::build(EmptyConfig::default());
        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), NotImplementedNet));
    }
}
