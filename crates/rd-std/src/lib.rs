pub use context::ContextExt;
use rd_interface::{Registry, Result};

pub mod builtin;
mod context;
pub mod http;
pub mod mixed;
pub mod rule;
pub mod select;
pub mod sniffer;
pub mod socks5;
pub mod tests;
pub mod tls;
pub mod transparent;
pub mod util;

pub fn init(registry: &mut Registry) -> Result<()> {
    builtin::init(registry)?;
    sniffer::init(registry)?;
    http::init(registry)?;
    mixed::init(registry)?;
    tls::init(registry)?;
    transparent::init(registry)?;
    rule::init(registry)?;
    socks5::init(registry)?;
    select::init(registry)?;
    Ok(())
}

#[cfg(test)]
mod init_tests {
    use super::*;

    #[test]
    fn test_std_init() {
        let mut registry = Registry::new();
        init(&mut registry).unwrap();
    }
}
