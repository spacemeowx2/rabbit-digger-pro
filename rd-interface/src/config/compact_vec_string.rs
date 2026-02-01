use crate as rd_interface;
use std::fmt;

use schemars::JsonSchema;
use serde::{
    de::{SeqAccess, Visitor},
    Deserialize, Serialize,
};

use crate::impl_empty_config;

#[derive(Clone, Hash, PartialEq, Eq)]
pub struct CompactVecString {
    underlying: Vec<u8>,
    index: Vec<usize>,
}

impl fmt::Debug for CompactVecString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.iter().take(32)).finish()
    }
}

impl CompactVecString {
    pub fn new() -> Self {
        Self {
            underlying: Vec::new(),
            index: Vec::new(),
        }
    }
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            underlying: Vec::with_capacity(capacity),
            index: Vec::with_capacity(capacity),
        }
    }
    pub fn push(&mut self, s: impl AsRef<str>) {
        let len = self.underlying.len();
        self.underlying.extend_from_slice(s.as_ref().as_bytes());
        self.index.push(len);
    }
    pub fn pop(&mut self) {
        if let Some(i) = self.index.pop() {
            self.underlying.truncate(i);
        }
    }
    pub fn shrink_to_fit(&mut self) {
        self.underlying.shrink_to_fit();
        self.index.shrink_to_fit();
    }
    pub fn iter(&self) -> Iter<'_> {
        Iter {
            inner: self,
            index: 0,
        }
    }
    pub fn len(&self) -> usize {
        self.index.len()
    }
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }
    pub fn join(&self, sep: &str) -> String {
        self.into_iter().collect::<Vec<_>>().join(sep)
    }
    pub fn get(&self, index: usize) -> Option<&str> {
        if index >= self.index.len() {
            return None;
        }
        let start = self.index[index];
        let end = if index + 1 < self.index.len() {
            self.index[index + 1]
        } else {
            self.underlying.len()
        };
        Some(unsafe { std::str::from_utf8_unchecked(&self.underlying[start..end]) })
    }
}

impl Extend<String> for CompactVecString {
    fn extend<T: IntoIterator<Item = String>>(&mut self, iter: T) {
        for s in iter {
            self.push(&s);
        }
    }
}

impl<'a> Extend<&'a str> for CompactVecString {
    fn extend<T: IntoIterator<Item = &'a str>>(&mut self, iter: T) {
        for s in iter {
            self.push(s);
        }
    }
}

impl FromIterator<String> for CompactVecString {
    fn from_iter<I: IntoIterator<Item = String>>(iter: I) -> Self {
        let mut c = Self::new();
        for s in iter {
            c.push(&s);
        }
        c
    }
}

impl<'a> FromIterator<&'a str> for CompactVecString {
    fn from_iter<I: IntoIterator<Item = &'a str>>(iter: I) -> Self {
        let mut c = Self::new();
        for s in iter {
            c.push(s);
        }
        c
    }
}

impl<'a> IntoIterator for &'a CompactVecString {
    type Item = &'a str;
    type IntoIter = Iter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

pub struct Iter<'a> {
    inner: &'a CompactVecString,
    index: usize,
}

impl<'a> Iterator for Iter<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        let result = self.inner.get(self.index);
        self.index += 1;
        result
    }
}

impl Serialize for CompactVecString {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        if self.len() == 1 {
            if let Some(s) = self.get(0) {
                return serializer.serialize_str(s);
            }
        }
        serializer.collect_seq(self.iter())
    }
}

impl<'de> Deserialize<'de> for CompactVecString {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct StringsVisitor;

        impl<'de> Visitor<'de> for StringsVisitor {
            type Value = CompactVecString;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                write!(formatter, "string or [string]")
            }

            fn visit_seq<V>(self, mut seq: V) -> Result<Self::Value, V::Error>
            where
                V: SeqAccess<'de>,
            {
                let len = seq.size_hint().unwrap_or(10);

                let mut values = CompactVecString::with_capacity(len);

                while let Some(value) = seq.next_element::<String>()? {
                    values.push(&value);
                }

                values.shrink_to_fit();

                Ok(values)
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                let mut values = CompactVecString::with_capacity(1);
                values.push(value);
                Ok(values)
            }
        }

        deserializer.deserialize_any(StringsVisitor)
    }
}

impl JsonSchema for CompactVecString {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("StringList")
    }

    fn json_schema(gen: &mut schemars::SchemaGenerator) -> schemars::Schema {
        gen.subschema_for::<super::SingleOrVec<String>>()
    }
}

impl<S: AsRef<str>> From<Vec<S>> for CompactVecString {
    fn from(v: Vec<S>) -> Self {
        let mut r = Self::new();
        for s in v.iter() {
            r.push(s.as_ref());
        }
        r.shrink_to_fit();
        r
    }
}

impl From<String> for CompactVecString {
    fn from(v: String) -> Self {
        let mut r = Self::with_capacity(1);
        r.push(&v);
        r
    }
}

impl From<&str> for CompactVecString {
    fn from(v: &str) -> Self {
        let mut r = Self::with_capacity(1);
        r.push(v);
        r
    }
}

impl_empty_config!(CompactVecString);

impl<I> PartialEq<Vec<I>> for CompactVecString
where
    I: AsRef<str>,
{
    fn eq(&self, other: &Vec<I>) -> bool {
        if self.len() != other.len() {
            return false;
        }
        let i1 = self.iter();
        let i2 = other.iter().map(|s| s.as_ref());
        i1.eq(i2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compact_vec_string_from() {
        let v = CompactVecString::from_iter(vec!["a", "b", "c"]);
        assert_eq!(v.len(), 3);
        assert_eq!(v.into_iter().collect::<Vec<_>>(), vec!["a", "b", "c"]);

        let v = CompactVecString::from("a");
        assert_eq!(v.len(), 1);
        assert_eq!(v.get(0).unwrap(), "a");

        let v = CompactVecString::from("a".to_string());
        assert_eq!(v.len(), 1);
        assert_eq!(v.get(0).unwrap(), "a");
    }

    #[test]
    fn test_debug() {
        let v = CompactVecString::from_iter(vec!["a", "b", "c"]);
        assert_eq!(format!("{v:?}"), "[\"a\", \"b\", \"c\"]");
    }

    #[test]
    fn test_extend() {
        let mut v = CompactVecString::new();
        v.extend(vec!["a", "b", "c"]);
        assert_eq!(v.len(), 3);
        assert_eq!(v.into_iter().collect::<Vec<_>>(), vec!["a", "b", "c"]);

        let mut v = CompactVecString::new();
        v.extend(vec!["a".to_string(), "b".to_string(), "c".to_string()]);
        assert_eq!(v.len(), 3);
        assert_eq!(v.into_iter().collect::<Vec<_>>(), vec!["a", "b", "c"]);
    }

    #[test]
    fn test_serde() {
        let v = CompactVecString::from_iter(vec!["a", "b", "c"]);
        let v2 =
            serde_json::from_str::<CompactVecString>(&serde_json::to_string(&v).unwrap()).unwrap();
        assert_eq!(v, v2);
        let v3 = serde_json::from_str::<CompactVecString>("[\"a\",\"b\",\"c\"]").unwrap();
        assert_eq!(v, v3);

        let v = CompactVecString::from("a");
        let v2 =
            serde_json::from_str::<CompactVecString>(&serde_json::to_string(&v).unwrap()).unwrap();
        assert_eq!(v, v2);
        let v3 = serde_json::from_str::<CompactVecString>("\"a\"").unwrap();
        assert_eq!(v, v3);

        let v = CompactVecString::from("a".to_string());
        assert_eq!(serde_json::to_string(&v).unwrap(), "\"a\"");

        let v = CompactVecString::from_iter(vec!["a", "b", "c"]);
        assert_eq!(serde_json::to_string(&v).unwrap(), "[\"a\",\"b\",\"c\"]");

        let v = CompactVecString::new();
        assert_eq!(serde_json::to_string(&v).unwrap(), "[]");
    }
}
