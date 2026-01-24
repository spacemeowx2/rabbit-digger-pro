use std::{
    collections::{HashMap, VecDeque},
    hash::Hash,
};

use topological_sort::TopologicalSort;

#[derive(Debug)]
pub enum RootType<K> {
    All,
    Key(Vec<K>),
}

#[derive(Clone, Copy, Debug, Hash, Eq, PartialEq, PartialOrd, Ord)]
enum Node<T> {
    Item(T),
    Root,
}

pub fn topological_sort<K, V, D, E>(
    root: RootType<K>,
    map: impl Iterator<Item = (K, V)>,
    get_deps: D,
) -> Result<Option<Vec<(K, V)>>, E>
where
    K: Hash + Ord + Eq + Clone + std::fmt::Debug,
    D: Fn(&K, &V) -> Result<Vec<K>, E>,
{
    let mut ts = TopologicalSort::<Node<K>>::new();
    let mut map = map.collect::<HashMap<_, _>>();

    for (k, v) in map.iter() {
        for d in get_deps(k, v)?.into_iter() {
            ts.add_dependency(Node::Item(k.clone()), Node::Item(d));
        }
        if let RootType::All = root {
            ts.add_dependency(Node::Root, Node::Item(k.clone()));
        }
    }
    if let RootType::Key(root) = root {
        for k in root {
            ts.add_dependency(Node::Root, Node::Item(k));
        }
    }

    let mut list = VecDeque::<K>::new();
    while let Some(k) = ts.pop() {
        if let Node::Item(k) = k {
            list.push_front(k.clone());
        }
    }

    if !ts.is_empty() {
        return Ok(None);
    }

    Ok(Some(
        list.into_iter()
            .flat_map(|k| {
                let v = map.remove(&k);
                v.map(|v| (k, v))
            })
            .collect(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_topological_sort() {
        let mut map = HashMap::new();
        map.insert("a", 1);
        map.insert("b", 2);
        map.insert("c", 3);
        map.insert("d", 4);

        let res = topological_sort(
            RootType::All,
            map.into_iter(),
            |k: &&str, _: &u8| -> Result<Vec<_>, ()> {
                match k.as_ref() {
                    "a" => Ok(vec!["b", "c"]),
                    "b" => Ok(vec!["c"]),
                    "c" => Ok(vec!["d"]),
                    "d" => Ok(vec![]),
                    _ => unreachable!(),
                }
            },
        );

        assert_eq!(
            res.unwrap().unwrap(),
            vec![("d", 4), ("c", 3), ("b", 2), ("a", 1)]
        );
    }

    #[test]
    fn test_topological_sort_conflict() {
        let mut map = HashMap::new();
        map.insert("a", 1);
        map.insert("b", 2);

        let res = topological_sort(
            RootType::All,
            map.into_iter(),
            |k: &&str, _: &u8| -> Result<Vec<_>, ()> {
                match k.as_ref() {
                    "a" => Ok(vec!["b"]),
                    "b" => Ok(vec!["a"]),
                    _ => unreachable!(),
                }
            },
        );

        assert!(res.unwrap().is_none());
    }

    #[test]
    fn test_topological_sort_root() {
        let mut map = HashMap::new();
        map.insert("a", 1);
        map.insert("b", 2);
        map.insert("c", 3);
        map.insert("d", 4);

        let res = topological_sort(
            RootType::Key(vec!["b"]),
            map.into_iter(),
            |k: &&str, _: &u8| -> Result<Vec<_>, ()> {
                match k.as_ref() {
                    "a" => Ok(vec![]),
                    "b" => Ok(vec!["a"]),
                    "c" => Ok(vec![]),
                    "d" => Ok(vec![]),
                    _ => unreachable!(),
                }
            },
        );

        assert_eq!(res.unwrap().unwrap(), vec![("a", 1), ("b", 2)]);
    }
}
