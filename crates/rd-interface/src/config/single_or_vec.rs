use std::{mem::replace, slice};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{Config, Visitor, VisitorContext};
use crate::Result;

#[derive(JsonSchema, Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SingleOrVec<T> {
    Single(T),
    Vec(Vec<T>),
}

impl<T: Config> Config for SingleOrVec<T> {
    fn visit(&mut self, ctx: &mut VisitorContext, visitor: &mut dyn Visitor) -> Result<()> {
        match self {
            SingleOrVec::Single(x) => x.visit(ctx, visitor)?,
            SingleOrVec::Vec(x) => {
                for x in x.iter_mut() {
                    x.visit(ctx, visitor)?;
                }
            }
        }
        Ok(())
    }
}

impl<A> Extend<A> for SingleOrVec<A> {
    fn extend<T: IntoIterator<Item = A>>(&mut self, iter: T) {
        let iter = iter.into_iter();
        let mut this = replace(self, SingleOrVec::Vec(Vec::new()));
        match this {
            SingleOrVec::Single(i) => {
                let mut v = Vec::with_capacity(1 + iter.size_hint().0);
                v.push(i);
                v.extend(iter);
                this = SingleOrVec::Vec(v);
            }
            SingleOrVec::Vec(ref mut v) => v.extend(iter),
        }
        *self = this;
    }
}

impl<'a, T> IntoIterator for &'a mut SingleOrVec<T> {
    type Item = &'a mut T;

    type IntoIter = IterMut<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

impl<T> From<T> for SingleOrVec<T> {
    fn from(x: T) -> Self {
        SingleOrVec::Single(x)
    }
}

impl<T> From<Vec<T>> for SingleOrVec<T> {
    fn from(x: Vec<T>) -> Self {
        SingleOrVec::Vec(x)
    }
}

impl<T> SingleOrVec<T> {
    pub fn into_vec(self) -> Vec<T> {
        match self {
            SingleOrVec::Single(t) => vec![t],
            SingleOrVec::Vec(v) => v,
        }
    }
    pub fn iter<'a>(&'a self) -> Iter<'a, T> {
        match self {
            SingleOrVec::Single(t) => Iter::Single(Some(t)),
            SingleOrVec::Vec(v) => Iter::Vec(v.iter()),
        }
    }
    pub fn iter_mut<'a>(&'a mut self) -> IterMut<'a, T> {
        match self {
            SingleOrVec::Single(t) => IterMut::Single(Some(t)),
            SingleOrVec::Vec(v) => IterMut::Vec(v.iter_mut()),
        }
    }
    pub fn shrink_to_fit(&mut self) {
        match self {
            SingleOrVec::Single(_) => {}
            SingleOrVec::Vec(x) => {
                x.shrink_to_fit();
            }
        }
    }
}

pub enum IterMut<'a, T> {
    Single(Option<&'a mut T>),
    Vec(slice::IterMut<'a, T>),
}

impl<'a, T> Iterator for IterMut<'a, T> {
    type Item = &'a mut T;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            IterMut::Single(x) => x.take(),
            IterMut::Vec(x) => x.next(),
        }
    }
}

pub enum Iter<'a, T> {
    Single(Option<&'a T>),
    Vec(slice::Iter<'a, T>),
}

impl<'a, T> Iterator for Iter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Iter::Single(x) => x.take(),
            Iter::Vec(x) => x.next(),
        }
    }
}
