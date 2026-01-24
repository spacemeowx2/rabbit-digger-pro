use std::{collections::BTreeMap, fmt};

pub use crate::config::NetRef;
use crate::{
    config::{Config, Visitor, VisitorContext},
    IntoDyn, Net, Result, Server,
};
pub use schemars::JsonSchema;
use schemars::{schema_for, Schema};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;

pub type NetGetter<'a> = &'a dyn Fn(&mut NetRef, &VisitorContext) -> Result<Net>;

pub struct Registry {
    pub net: BTreeMap<String, NetResolver>,
    pub server: BTreeMap<String, ServerResolver>,
}

impl fmt::Debug for Registry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Registry")
            .field("net", &self.net.keys())
            .field("server", &self.server.keys())
            .finish()
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

impl Registry {
    pub fn new() -> Registry {
        Registry {
            net: BTreeMap::new(),
            server: BTreeMap::new(),
        }
    }
    pub fn add_net<N: Builder<Net>>(&mut self) {
        self.net.insert(N::NAME.into(), NetResolver::new::<N>());
    }
    pub fn add_server<S: Builder<Server>>(&mut self) {
        self.server
            .insert(S::NAME.into(), ServerResolver::new::<S>());
    }
}

pub trait Builder<ItemType> {
    const NAME: &'static str;
    type Config: Serialize + DeserializeOwned + JsonSchema + Config + 'static;
    type Item: IntoDyn<ItemType> + Sized + 'static;

    fn build(config: Self::Config) -> Result<Self::Item>;
}

impl<ItemType, T: Builder<ItemType>> BuilderExt<ItemType> for T {}
trait BuilderExt<ItemType>: Builder<ItemType> {
    fn build_dyn(getter: NetGetter, cfg: &mut Value) -> Result<ItemType> {
        let config = std::mem::replace(cfg, Value::Null);
        let mut config = serde_json::from_value(config)?;
        resolve_net(&mut config, getter)?;
        *cfg = serde_json::to_value(&config)?;

        Ok(Self::build(config)?.into_dyn())
    }
}

pub struct Resolver<ItemType> {
    build: fn(getter: NetGetter, cfg: &mut Value) -> Result<ItemType>,
    schema: Schema,
}
pub type NetResolver = Resolver<Net>;
pub type ServerResolver = Resolver<Server>;

impl<ItemType> Resolver<ItemType> {
    pub fn new<N: Builder<ItemType>>() -> Self {
        let schema = schema_for!(N::Config);
        Self {
            build: N::build_dyn,
            schema,
        }
    }
    pub fn build(&self, getter: NetGetter, cfg: &mut Value) -> Result<ItemType> {
        (self.build)(getter, cfg)
    }
    pub fn schema(&self) -> &Schema {
        &self.schema
    }
}

pub fn resolve_net(config: &mut dyn Config, getter: NetGetter) -> Result<()> {
    struct ResolveNetVisitor<'a>(NetGetter<'a>);

    impl<'a> Visitor for ResolveNetVisitor<'a> {
        fn visit_net_ref(&mut self, ctx: &mut VisitorContext, net_ref: &mut NetRef) -> Result<()> {
            let net = self.0(net_ref, ctx)?;
            net_ref.set_value(net);
            Ok(())
        }
    }

    config.visit(&mut VisitorContext::new(), &mut ResolveNetVisitor(getter))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{self as rd_interface, rd_config, Error, INet, IntoDyn};
    use std::collections::HashMap;

    #[test]
    fn test_registry_debug() {
        let reg = Registry::default();
        assert_eq!(format!("{reg:?}"), "Registry { net: [], server: [] }");
    }

    struct NotImplementedNet;
    impl INet for NotImplementedNet {}

    #[test]
    fn test_net_ref() {
        #[rd_config]
        struct TestConfig {
            net: Vec<NetRef>,
        }

        let mut test: TestConfig = serde_json::from_str(r#"{ "net": ["test"] }"#).unwrap();

        assert_eq!(test.net[0].represent(), "test");

        let mut net_map = HashMap::new();
        let noop = NotImplementedNet.into_dyn();

        net_map.insert("test".to_string(), noop.clone());
        resolve_net(&mut test, &|key, _ctx| {
            net_map
                .get(key.represent().as_str().unwrap())
                .cloned()
                .ok_or_else(|| Error::NotFound("not found".to_string()))
        })
        .unwrap();

        assert_eq!(test.net[0].as_ptr(), noop.as_ptr())
    }
}
