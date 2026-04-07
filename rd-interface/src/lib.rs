pub use address::{Address, AddressDomain, IntoAddress};
pub use context::Context;
pub use error::{Error, ErrorContext, Result, NOT_IMPLEMENTED};
pub use interface::*;
pub use rd_derive::{rd_config, Config};
pub use registry::Registry;
pub use schemars;
pub use serde_json::Value;
pub use side_effect::SideEffectManager;

mod address;
pub mod config;
pub mod constant;
pub mod context;
pub mod error;
mod interface;
mod macros;
pub mod registry;
pub mod side_effect;

/// Prelude for easy defining `Config` struct.
pub mod prelude {
    pub use rd_derive::rd_config;
    pub use schemars;
}
