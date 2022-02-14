//! A registry with plugin name

use rd_interface::{
    error::ErrorContext,
    registry::{NetGetter, NetResolver, ServerResolver},
    Net, Result, Server, Value,
};
use std::{collections::BTreeMap, fmt};

use crate::builtin::load_builtin;

pub struct NetItem {
    id: String,
    pub plugin_name: String,
    pub resolver: NetResolver,
}

pub struct ServerItem {
    id: String,
    pub plugin_name: String,
    pub resolver: ServerResolver,
}

impl NetItem {
    pub fn build(&self, getter: NetGetter, config: Value) -> Result<Net> {
        self.resolver
            .build(getter, config)
            .with_context(|| format!("Failed to build net: {}", self.id))
    }
}

impl ServerItem {
    pub fn build(&self, getter: NetGetter, config: Value) -> Result<Server> {
        self.resolver
            .build(getter, config)
            .with_context(|| format!("Failed to build server: {}", self.id))
    }
}

#[derive(Debug)]
pub struct Registry {
    net: BTreeMap<String, NetItem>,
    server: BTreeMap<String, ServerItem>,
}

impl Default for Registry {
    fn default() -> Self {
        let mut registry = Self::new();

        load_builtin(&mut registry).expect("Failed to load builtin");

        registry
    }
}

impl fmt::Debug for NetItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NetItem")
            .field("plugin_name", &self.plugin_name)
            .finish()
    }
}

impl fmt::Debug for ServerItem {
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
        name: impl Into<String>,
        init: impl Fn(&mut rd_interface::Registry) -> rd_interface::Result<()>,
    ) -> rd_interface::Result<()> {
        let mut r = rd_interface::Registry::new();
        init(&mut r)?;
        self.add_registry(name.into(), r);
        Ok(())
    }
    fn add_registry(&mut self, plugin_name: String, registry: rd_interface::Registry) {
        for (k, v) in registry.net {
            self.net.insert(
                k.clone(),
                NetItem {
                    id: k,
                    plugin_name: plugin_name.clone(),
                    resolver: v,
                },
            );
        }
        for (k, v) in registry.server {
            self.server.insert(
                k.clone(),
                ServerItem {
                    id: k,
                    plugin_name: plugin_name.clone(),
                    resolver: v,
                },
            );
        }
    }
    pub fn net(&self) -> &BTreeMap<String, NetItem> {
        &self.net
    }
    pub fn server(&self) -> &BTreeMap<String, ServerItem> {
        &self.server
    }
    pub fn get_net(&self, net_type: &str) -> Result<&NetItem> {
        self.net.get(net_type).ok_or_else(|| {
            rd_interface::Error::other(format!("Net type is not loaded: {}", net_type))
        })
    }
    pub fn get_server(&self, server_type: &str) -> Result<&ServerItem> {
        self.server.get(server_type).ok_or_else(|| {
            rd_interface::Error::other(format!("Server type is not loaded: {}", server_type))
        })
    }
}
