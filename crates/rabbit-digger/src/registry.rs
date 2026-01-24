//! A registry with plugin name

use rd_interface::{
    error::ErrorContext,
    registry::{NetGetter, Resolver},
    schemars::Schema,
    Net, Result, Server, Value,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fmt};

use crate::builtin::load_builtin;

pub struct Item<T> {
    id: String,
    plugin_name: String,
    resolver: Resolver<T>,
}

impl<T> Item<T> {
    pub fn schema(&self) -> &Schema {
        self.resolver.schema()
    }
}

impl Item<Net> {
    pub fn build(&self, getter: NetGetter, config: &mut Value) -> rd_interface::Result<Net> {
        self.resolver
            .build(getter, config)
            .with_context(|| format!("Failed to build net: {}", self.id))
    }
}

impl Item<Server> {
    pub fn build(&self, getter: NetGetter, config: &mut Value) -> rd_interface::Result<Server> {
        self.resolver
            .build(getter, config)
            .with_context(|| format!("Failed to build server: {}", self.id))
    }
}

#[derive(Debug)]
pub struct Registry {
    net: BTreeMap<String, Item<Net>>,
    server: BTreeMap<String, Item<Server>>,
}

impl Default for Registry {
    fn default() -> Self {
        let mut registry = Self::new();

        load_builtin(&mut registry).expect("Failed to load builtin");

        registry
    }
}

impl fmt::Debug for Item<Net> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NetItem")
            .field("plugin_name", &self.plugin_name)
            .finish()
    }
}

impl fmt::Debug for Item<Server> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServerItem")
            .field("plugin_name", &self.plugin_name)
            .finish()
    }
}

impl fmt::Display for Registry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Net")?;
        for (k, v) in self.net.iter() {
            writeln!(f, "\t{}: {}", k, v.plugin_name)?;
        }
        writeln!(f, "Server")?;
        for (k, v) in self.server.iter() {
            writeln!(f, "\t{}: {}", k, v.plugin_name)?;
        }
        Ok(())
    }
}

impl Registry {
    pub fn new() -> Registry {
        Registry {
            net: BTreeMap::new(),
            server: BTreeMap::new(),
        }
    }
    /// Creates an empty registry without loading any builtin implementations.
    ///
    /// This is the preferred constructor for “core-only” builds; composition
    /// layers (e.g. `rdp-bundle`) should register adapters/plugins explicitly.
    pub fn new_core() -> Registry {
        Self::new()
    }
    pub fn new_with_builtin() -> Result<Self> {
        let mut registry = Self::new();

        load_builtin(&mut registry)?;

        Ok(registry)
    }
    pub fn load_builtin(&mut self) -> Result<()> {
        load_builtin(self)
    }
    pub fn init_with_registry(
        &mut self,
        name: impl AsRef<str>,
        init: impl Fn(&mut rd_interface::Registry) -> rd_interface::Result<()>,
    ) -> rd_interface::Result<()> {
        let mut r = rd_interface::Registry::new();
        init(&mut r)?;
        self.add_registry(name.as_ref(), r);
        Ok(())
    }
    fn add_registry(&mut self, plugin_name: &str, registry: rd_interface::Registry) {
        self.net
            .extend(registry.net.into_iter().map(|(id, resolver)| {
                (
                    id.clone(),
                    Item {
                        id,
                        plugin_name: plugin_name.to_string(),
                        resolver,
                    },
                )
            }));
        self.server
            .extend(registry.server.into_iter().map(|(id, resolver)| {
                (
                    id.clone(),
                    Item {
                        id,
                        plugin_name: plugin_name.to_string(),
                        resolver,
                    },
                )
            }));
    }
    pub fn net(&self) -> &BTreeMap<String, Item<Net>> {
        &self.net
    }
    pub fn server(&self) -> &BTreeMap<String, Item<Server>> {
        &self.server
    }
    pub fn get_net(&self, net_type: &str) -> Result<&Item<Net>> {
        self.net.get(net_type).ok_or_else(|| {
            rd_interface::Error::other(format!("Net type is not loaded: {}", net_type))
        })
    }
    pub fn get_server(&self, server_type: &str) -> Result<&Item<Server>> {
        self.server.get(server_type).ok_or_else(|| {
            rd_interface::Error::other(format!("Server type is not loaded: {}", server_type))
        })
    }
    pub fn get_registry_schema(&self) -> RegistrySchema {
        let mut r = RegistrySchema {
            net: BTreeMap::new(),
            server: BTreeMap::new(),
        };

        for (key, value) in self.net() {
            r.net.insert(key.clone(), value.resolver.schema().clone());
        }
        for (key, value) in self.server() {
            r.server
                .insert(key.clone(), value.resolver.schema().clone());
        }

        r
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RegistrySchema {
    net: BTreeMap<String, Schema>,
    server: BTreeMap<String, Schema>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "rd-std")]
    use rd_interface::{Error, IntoDyn};
    #[cfg(feature = "rd-std")]
    use serde_json::Map;

    #[test]
    fn test_registry() {
        let mut registry = Registry::new();
        assert_eq!(registry.net().len(), 0);
        assert_eq!(registry.server().len(), 0);
        registry.load_builtin().unwrap();
        #[cfg(feature = "rd-std")]
        {
            assert!(!registry.net().is_empty());
            assert!(!registry.server().is_empty());
        }

        let registry = Registry::new_with_builtin().unwrap();
        #[cfg(not(feature = "rd-std"))]
        let _ = &registry;
        #[cfg(feature = "rd-std")]
        {
            assert!(!registry.net().is_empty());
            assert!(!registry.server().is_empty());
        }

        let registry = Registry::default();
        #[cfg(not(feature = "rd-std"))]
        let _ = &registry;
        #[cfg(feature = "rd-std")]
        {
            assert!(!registry.net().is_empty());
            assert!(!registry.server().is_empty());
        }

        #[cfg(feature = "rd-std")]
        {
            assert!(registry.get_net("local").is_ok());
            assert!(registry.get_server("socks5").is_ok());
        }

        assert!(registry.get_net("_NOT_EXISTED").is_err());
        assert!(registry.get_server("_NOT_EXISTED").is_err());
    }

    #[test]
    fn test_registry_debug() {
        let registry = Registry::new_with_builtin().unwrap();

        let _ = format!("{:?}", registry);
        let _ = format!("{}", registry);
    }

    #[test]
    fn test_registry_build() {
        let registry = Registry::new_with_builtin().unwrap();
        #[cfg(not(feature = "rd-std"))]
        let _ = registry;
        #[cfg(feature = "rd-std")]
        let test_net = rd_std::tests::TestNet::new().into_dyn();

        #[cfg(feature = "rd-std")]
        assert!(registry
            .get_net("local")
            .unwrap()
            .build(
                &|_, _| Err(Error::NotFound("not found".to_string())),
                &mut Value::Object(Map::new())
            )
            .is_ok());

        #[cfg(feature = "rd-std")]
        assert!(registry
            .get_server("socks5")
            .unwrap()
            .build(&|_, _| Ok(test_net.clone()), &mut Value::Object(Map::new()))
            .is_err());

        #[cfg(feature = "rd-std")]
        assert!(registry
            .get_server("socks5")
            .unwrap()
            .build(
                &|_, _| Ok(test_net.clone()),
                &mut serde_json::json!({
                    "bind": "127.0.0.1:1080"
                })
            )
            .is_ok());
    }
}
