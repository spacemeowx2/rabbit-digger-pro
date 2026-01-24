use anyhow::Result;

pub fn build_registry() -> Result<rabbit_digger::Registry> {
    let mut registry = rabbit_digger::Registry::new_core();

    #[cfg(feature = "std")]
    registry.init_with_registry("std", rd_std::init)?;

    #[cfg(feature = "ss")]
    registry.init_with_registry("ss", ss::init)?;
    #[cfg(feature = "trojan")]
    registry.init_with_registry("trojan", trojan::init)?;
    #[cfg(feature = "rpc")]
    registry.init_with_registry("rpc", rpc::init)?;
    #[cfg(feature = "raw")]
    registry.init_with_registry("raw", raw::init)?;
    #[cfg(feature = "obfs")]
    registry.init_with_registry("obfs", obfs::init)?;

    Ok(registry)
}
