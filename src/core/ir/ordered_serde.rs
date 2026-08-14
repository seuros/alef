use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::hash::BuildHasher;

use serde::{Serialize, Serializer};

pub(super) fn serialize_map<S, K, V, H>(map: &HashMap<K, V, H>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
    K: Ord + Serialize,
    V: Serialize,
    H: BuildHasher,
{
    map.iter().collect::<BTreeMap<_, _>>().serialize(serializer)
}

pub(super) fn serialize_set<S, T, H>(set: &HashSet<T, H>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
    T: Ord + Serialize,
    H: BuildHasher,
{
    set.iter().collect::<BTreeSet<_>>().serialize(serializer)
}
