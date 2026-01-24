use rd_interface::Result;

use crate::registry::Registry;

pub fn load_builtin(_registry: &mut Registry) -> Result<()> {
    #[cfg(feature = "rd-std")]
    _registry.init_with_registry("std", rd_std::init)?;

    Ok(())
}
