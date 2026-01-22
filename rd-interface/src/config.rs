use std::borrow::Cow;
use std::cell::RefCell;

use crate::{self as rd_interface, Address, Net};
pub use resolvable::{Resolvable, ResolvableSchema};
use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Result;
pub use compact_vec_string::CompactVecString;
pub use single_or_vec::SingleOrVec;

mod compact_vec_string;
mod resolvable;
mod single_or_vec;

#[derive(Clone)]
pub struct NetSchema;
impl JsonSchema for NetSchema {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("NetRef")
    }

    fn json_schema(gen: &mut SchemaGenerator) -> Schema {
        let string_schema = serde_json::to_value(gen.subschema_for::<String>())
            .expect("schemars schema should be serializable");

        let mut obj = serde_json::Map::new();
        obj.insert(
            "anyOf".to_string(),
            Value::Array(vec![
                string_schema,
                Value::Object(serde_json::Map::from_iter([(
                    "$ref".to_string(),
                    Value::String("#/definitions/Net".to_string()),
                )])),
            ]),
        );
        Schema::from(obj)
    }
}

impl ResolvableSchema for NetSchema {
    type Represent = Value;
    type Value = Net;
}

pub type NetRef = Resolvable<NetSchema>;

impl Default for NetRef {
    fn default() -> Self {
        NetRef::new("local".into())
    }
}

pub trait Visitor {
    #[allow(unused_variables)]
    fn visit_net_ref(&mut self, ctx: &mut VisitorContext, net_ref: &mut NetRef) -> Result<()> {
        Ok(())
    }
}

pub struct VisitorContext {
    path: CompactVecString,
}

impl VisitorContext {
    pub(crate) fn new() -> VisitorContext {
        VisitorContext {
            path: CompactVecString::new(),
        }
    }
    pub fn push(&mut self, field: impl AsRef<str>) -> &mut Self {
        self.path.push(field.as_ref());
        self
    }
    pub fn pop(&mut self) {
        self.path.pop();
    }
    pub fn path(&self) -> &CompactVecString {
        &self.path
    }
}

pub trait Config {
    fn visit(&mut self, ctx: &mut VisitorContext, visitor: &mut dyn Visitor) -> Result<()>;
}

impl Config for NetRef {
    fn visit(&mut self, ctx: &mut VisitorContext, visitor: &mut dyn Visitor) -> Result<()> {
        visitor.visit_net_ref(ctx, self)
    }
}

#[macro_export]
macro_rules! impl_empty_config {
    ($($x:ident),+ $(,)?) => ($(
        impl rd_interface::config::Config for $x {
            fn visit(&mut self, _ctx: &mut rd_interface::config::VisitorContext, _visitor: &mut dyn rd_interface::config::Visitor) -> rd_interface::Result<()>
            {
                Ok(())
            }
        }
    )*)
}

mod impl_std {
    use super::Config;
    use crate as rd_interface;
    use crate::{Address, Result};
    use std::collections::{BTreeMap, HashMap, LinkedList, VecDeque};
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
    use std::path::PathBuf;

    macro_rules! impl_container_config {
        ($($x:ident),+ $(,)?) => ($(
            impl<T: Config> Config for $x<T> {
                fn visit(&mut self, ctx: &mut rd_interface::config::VisitorContext, visitor: &mut dyn rd_interface::config::Visitor) -> rd_interface::Result<()>
                {
                    for (key, i) in self.iter_mut().enumerate() {
                        ctx.push(key.to_string());
                        i.visit(ctx, visitor)?;
                        ctx.pop();
                    }
                    Ok(())
                }
            }
        )*)
    }
    macro_rules! impl_key_container_config {
        ($($x:ident),+ $(,)?) => ($(
            impl<K: std::string::ToString, T: Config> Config for $x<K, T> {
                fn visit(&mut self, ctx: &mut rd_interface::config::VisitorContext, visitor: &mut dyn rd_interface::config::Visitor) -> rd_interface::Result<()>
                {
                    for (key, i) in self.iter_mut() {
                        ctx.push(key.to_string());
                        i.visit(ctx, visitor)?;
                        ctx.pop();
                    }
                    Ok(())
                }
            }
        )*)
    }

    impl_empty_config! { Address }
    impl_empty_config! { String, u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize, bool, f32, f64 }
    impl_empty_config! { IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6 }
    impl_empty_config! { PathBuf }
    impl_container_config! { Vec, Option, VecDeque, Result, LinkedList }
    impl_key_container_config! { HashMap, BTreeMap }

    impl<T1, T2> rd_interface::config::Config for (T1, T2) {
        fn visit(
            &mut self,
            _ctx: &mut rd_interface::config::VisitorContext,
            _visitor: &mut dyn rd_interface::config::Visitor,
        ) -> rd_interface::Result<()> {
            Ok(())
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct EmptyConfig(Value);

impl JsonSchema for EmptyConfig {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("EmptyConfig")
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        let mut obj = serde_json::Map::new();
        obj.insert("type".to_string(), Value::String("object".to_string()));
        Schema::from(obj)
    }
}

crate::impl_empty_config! { EmptyConfig, Value }

impl JsonSchema for Address {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("Address")
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        let mut obj = serde_json::Map::new();
        obj.insert("type".to_string(), Value::String("string".to_string()));
        obj.insert(
            "description".to_string(),
            Value::String(
                "An address contains host and port.\nFor example: example.com:80, 1.1.1.1:53, [::1]:443".to_string(),
            ),
        );
        Schema::from(obj)
    }
}

#[derive(PartialEq, Clone)]
pub enum ConfigField {
    /// Default field type
    Common,
    /// Field type for detailed mode, which may be very large
    Detail,
    /// Field type for sensitive mode, which may contain sensitive information
    Sensitive,
}

pub const ALL_SERIALIZE_FIELDS: [ConfigField; 3] = [
    ConfigField::Common,
    ConfigField::Detail,
    ConfigField::Sensitive,
];
thread_local!(static SERIALIZE_FIELDS: RefCell<Vec<ConfigField>> = RefCell::new(vec![ConfigField::Common]));

pub fn detailed_field<T>(_: T) -> bool {
    !SERIALIZE_FIELDS.with(|x| x.borrow().contains(&ConfigField::Detail))
}

pub fn sensitive_field<T>(_: T) -> bool {
    !SERIALIZE_FIELDS.with(|x| x.borrow().contains(&ConfigField::Sensitive))
}

pub fn serialize_with_fields<T, F: FnOnce() -> T>(fields: Vec<ConfigField>, f: F) -> T {
    SERIALIZE_FIELDS.with(|x| {
        let old_fields = std::mem::replace(&mut *x.borrow_mut(), fields);
        let ret = f();
        *x.borrow_mut() = old_fields;
        ret
    })
}
