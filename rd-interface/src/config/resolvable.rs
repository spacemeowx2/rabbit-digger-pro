use std::{borrow::Cow, fmt, ops::Deref};

use schemars::JsonSchema;
use serde::{de, ser};

pub trait ResolvableSchema {
    type Represent;
    type Value;
}

#[derive(Clone)]
pub struct Resolvable<S>
where
    S: ResolvableSchema,
{
    represent: S::Represent,
    value: Option<S::Value>,
}

impl<S> fmt::Debug for Resolvable<S>
where
    S: ResolvableSchema,
    S::Represent: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Resolvable")
            .field("represent", &self.represent)
            .finish()
    }
}

impl<S> Resolvable<S>
where
    S: ResolvableSchema,
{
    pub fn new(represent: S::Represent) -> Self {
        Self {
            represent,
            value: None,
        }
    }
    pub fn new_with_value(represent: S::Represent, value: S::Value) -> Self {
        Self {
            represent,
            value: Some(value),
        }
    }
    pub fn represent(&self) -> &S::Represent {
        &self.represent
    }
    pub fn represent_mut(&mut self) -> &mut S::Represent {
        &mut self.represent
    }
    pub fn value(&self) -> Option<&S::Value> {
        self.value.as_ref()
    }
    pub(crate) fn set_value(&mut self, value: S::Value) {
        self.value = Some(value);
    }
}

impl<S> Resolvable<S>
where
    S: ResolvableSchema,
    S::Value: Clone,
{
    /// Returns a clone of the value.
    /// Panic if the value is not set
    pub fn value_cloned(&self) -> S::Value {
        self.value.clone().expect("value not set")
    }
}

impl<S> Deref for Resolvable<S>
where
    S: ResolvableSchema,
{
    type Target = S::Value;
    fn deref(&self) -> &Self::Target {
        self.value
            .as_ref()
            .expect("Resolvable must be resolved before Deref")
    }
}

impl<S> ser::Serialize for Resolvable<S>
where
    S: ResolvableSchema,
    S::Represent: ser::Serialize,
{
    fn serialize<Ser>(&self, serializer: Ser) -> Result<Ser::Ok, Ser::Error>
    where
        Ser: serde::Serializer,
    {
        ser::Serialize::serialize(&self.represent, serializer)
    }
}

impl<'de, S> de::Deserialize<'de> for Resolvable<S>
where
    S: ResolvableSchema,
    S::Represent: de::Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let r = de::Deserialize::deserialize(deserializer)?;

        Ok(Self::new(r))
    }
}

impl<S> JsonSchema for Resolvable<S>
where
    S: ResolvableSchema + JsonSchema,
{
    fn schema_name() -> Cow<'static, str> {
        S::schema_name()
    }

    fn json_schema(gen: &mut schemars::SchemaGenerator) -> schemars::Schema {
        S::json_schema(gen)
    }
}
