use crate::{config::CompactVecString, Value};
use serde::{de::DeserializeOwned, ser::SerializeMap, Serialize, Serializer};
use serde_json::Map;
use std::{
    collections::HashMap, fmt, io::Cursor, iter::FromIterator, mem::replace, net::SocketAddr,
};
use thiserror::Error;

/// Context error
#[derive(Debug, Error)]
pub enum Error {
    #[error("serde error {0}")]
    Serde(#[from] serde_json::Error),
    #[error("item not exists")]
    NonExist,
    #[error("Bad format")]
    BadFormat,
}
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Defines common used field with its key and type
pub trait CommonField: DeserializeOwned + Serialize + 'static {
    const KEY: &'static str;
}

/// A context stores a source endpoint, a process info and other any values
/// during connecting.
#[derive(Clone)]
pub struct Context {
    data: HashMap<String, Value>,
    net_list: CompactVecString,
}

impl fmt::Debug for Context {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let buf = Cursor::new(Vec::new());
        let mut serializer = serde_json::Serializer::new(buf);
        let mut visit = || {
            let mut serializer = serializer.serialize_map(None)?;

            for (k, v) in self.data.iter() {
                serializer.serialize_entry(&k, &v)?;
            }
            serializer.serialize_entry("net_list", self.net_list())?;

            serializer.end()
        };

        visit().map_err(|_| std::fmt::Error)?;

        f.write_str(
            String::from_utf8(serializer.into_inner().into_inner())
                .unwrap_or_default()
                .as_str(),
        )
    }
}

impl Default for Context {
    fn default() -> Self {
        Self::new()
    }
}

impl Context {
    /// new a empty context
    pub fn new() -> Context {
        Context {
            data: HashMap::with_capacity(16),
            net_list: CompactVecString::with_capacity(16),
        }
    }
    /// new a context from socket addr
    pub fn from_socketaddr(addr: SocketAddr) -> Context {
        let mut ctx = Context::new();
        ctx.insert_common(common_field::SrcSocketAddr(addr))
            .expect("Failed to insert src_socket_addr");
        ctx
    }
    /// Inserts a key-value pair into the context.
    pub fn insert<I: Serialize>(&mut self, key: String, value: I) -> Result<()> {
        self.data.insert(key, serde_json::to_value(value)?);
        Ok(())
    }
    /// Removes a key from the context
    pub fn remove(&mut self, key: &str) -> Result<()> {
        self.data.remove(key).ok_or(Error::NonExist)?;
        Ok(())
    }
    /// Returns a value corresponding to the key.
    pub fn get<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        let value = self.data.get(key);
        value
            .map(|v| serde_json::from_value(v.clone()))
            .transpose()
            .map_err(Into::into)
    }
    /// Inserts a key-value pair into the context.
    pub fn insert_value(&mut self, key: String, value: Value) {
        self.data.insert(key, value);
    }
    /// Removes a key from the context, returning the value at the key if the key was previously in the context.
    pub fn remove_value(&mut self, key: &str) -> Result<()> {
        self.data.remove(key);
        Ok(())
    }
    /// Returns a value corresponding to the key.
    pub fn get_value(&self, key: &str) -> Result<Value> {
        match self.data.get(key) {
            Some(v) => Ok(v.to_owned()),
            None => Err(Error::NonExist),
        }
    }
    /// Inserts a key-value pair into the context.
    pub fn insert_common<T: CommonField>(&mut self, value: T) -> Result<()> {
        self.insert(T::KEY.to_string(), value)
    }
    /// Returns a value corresponding to the key.
    pub fn get_common<T: CommonField>(&self) -> Result<Option<T>> {
        self.get(T::KEY)
    }
    /// Add net to net_list
    pub fn append_net(&mut self, net_name: impl AsRef<str>) {
        self.net_list.push(net_name.as_ref())
    }
    /// Get net_list
    pub fn net_list(&self) -> &CompactVecString {
        &self.net_list
    }
    /// Take net_list
    pub fn take_net_list(&mut self) -> CompactVecString {
        replace(&mut self.net_list, CompactVecString::new())
    }
    /// to Value
    pub fn to_value(&self) -> Value {
        let mut map = Map::from_iter(self.data.clone().into_iter());
        map.insert(
            "net_list".to_string(),
            serde_json::to_value(&self.net_list).unwrap(),
        );
        Value::Object(map)
    }
    /// from Value
    pub fn from_value(value: Value) -> Result<Self> {
        let mut ctx = Context::new();
        if let Value::Object(mut map) = value {
            if let Some(net_list) = map.remove("net_list") {
                ctx.net_list = serde_json::from_value(net_list)?;
            }
            ctx.data = HashMap::from_iter(map.into_iter());
            Ok(ctx)
        } else {
            Err(Error::BadFormat)
        }
    }
}

/// Common context keys and types
pub mod common_field {
    use crate::address::AddressDomain;

    use super::CommonField;
    use serde::{Deserialize, Serialize};
    use std::net::SocketAddr;

    #[derive(Debug, Deserialize, Serialize)]
    pub struct ProcessInfo {
        pub process_name: String,
    }

    impl CommonField for ProcessInfo {
        const KEY: &'static str = "process_info";
    }

    /// format: `domain:port`
    #[derive(Debug, Deserialize, Serialize)]
    pub struct DestDomain(pub AddressDomain);

    impl CommonField for DestDomain {
        const KEY: &'static str = "dest_domain";
    }

    #[derive(Debug, Deserialize, Serialize)]
    pub struct DestSocketAddr(pub SocketAddr);

    impl CommonField for DestSocketAddr {
        const KEY: &'static str = "dest_socket_addr";
    }

    #[derive(Debug, Deserialize, Serialize)]
    pub struct SrcSocketAddr(pub SocketAddr);

    impl CommonField for SrcSocketAddr {
        const KEY: &'static str = "src_socket_addr";
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_new() {
        let ctx = Context::new();
        assert_eq!(ctx.data.len(), 0);
        assert_eq!(ctx.net_list.len(), 0);
    }

    #[test]
    fn test_context_from_socketaddr() {
        let addr = SocketAddr::from(([127, 0, 0, 1], 80));
        let ctx = Context::from_socketaddr(addr);
        assert_eq!(ctx.data.len(), 1);
        assert_eq!(ctx.net_list.len(), 0);
    }

    #[test]
    fn test_context_insert() {
        let mut ctx = Context::new();
        ctx.insert("key".to_string(), "value").unwrap();
        assert_eq!(ctx.data.len(), 1);
        assert_eq!(ctx.net_list.len(), 0);

        ctx.insert("key2".to_string(), "value2").unwrap();
        assert_eq!(ctx.data.len(), 2);
        assert_eq!(ctx.net_list.len(), 0);
    }

    #[test]
    fn test_context_remove() {
        let mut ctx = Context::new();
        ctx.insert("key".to_string(), "value").unwrap();
        ctx.insert("key2".to_string(), "value2").unwrap();
        assert_eq!(ctx.data.len(), 2);
        assert_eq!(ctx.net_list.len(), 0);

        ctx.remove("key").unwrap();
        assert_eq!(ctx.data.len(), 1);
        assert_eq!(ctx.net_list.len(), 0);

        ctx.remove("key2").unwrap();
        assert_eq!(ctx.data.len(), 0);
        assert_eq!(ctx.net_list.len(), 0);
    }

    #[test]
    fn test_context_get() {
        let mut ctx = Context::new();
        ctx.insert("key".to_string(), "value").unwrap();
        ctx.insert("key2".to_string(), "value2").unwrap();
        assert_eq!(ctx.data.len(), 2);
        assert_eq!(ctx.net_list.len(), 0);

        assert_eq!(ctx.get::<String>("key").unwrap().unwrap(), "value");
        assert_eq!(ctx.get::<String>("key2").unwrap().unwrap(), "value2");
        assert_eq!(ctx.data.len(), 2);
        assert_eq!(ctx.net_list.len(), 0);
    }

    #[test]
    fn test_context_get_non_exist() {
        let mut ctx = Context::new();
        ctx.insert("key".to_string(), "value").unwrap();

        assert_eq!(ctx.get::<String>("key2").unwrap(), None);
        assert_eq!(ctx.data.len(), 1);
        assert_eq!(ctx.net_list.len(), 0);
    }

    #[test]
    fn test_context_common() {
        let mut ctx = Context::new();
        ctx.insert_common::<common_field::SrcSocketAddr>(common_field::SrcSocketAddr(
            SocketAddr::from(([127, 0, 0, 1], 80)),
        ))
        .unwrap();

        assert_eq!(
            ctx.get_common::<common_field::SrcSocketAddr>()
                .unwrap()
                .unwrap()
                .0,
            SocketAddr::from(([127, 0, 0, 1], 80),)
        );
    }

    #[test]
    fn test_context_append_net() {
        let mut ctx = Context::new();
        ctx.append_net("net1");
        ctx.append_net("net2");
        ctx.append_net("net3");
        assert_eq!(ctx.net_list.len(), 3);
    }
}
