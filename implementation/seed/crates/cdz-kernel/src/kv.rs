//! The session-attached key-value store — the reducer's projected state (§4).
//!
//! The reducer is stateless between events: it reads/writes this KV during a fold and holds nothing
//! else (§4). Two properties the design leans on:
//!
//! - **The root hash is a free per-event snapshot (§4).** After every fold we can content-address the
//!   whole KV; `(seq, root_hash, reducer_hash)` IS a snapshot. So checkpointing is a retention choice,
//!   not a compute cost.
//! - **Deterministic iteration (§16c-S8).** `prefix_scan` returns keys in a *fixed total order* (byte
//!   order of the key) so a reducer that folds over a scan emits effects in a replay-stable order.
//!
//! v0 uses a straightforward sorted map with a hash computed over its canonical serialization. That
//! gives correct root-hashing and deterministic iteration today; swapping in a structurally-shared
//! persistent map (CHAMP — the runtime already has one) for cheap incremental root hashes is a later,
//! behavior-preserving optimization. The *interface* here is what the reducer sees, so it won't change.

use crate::hash::Hash;
use std::collections::BTreeMap;

/// The reducer's key-value state. Keys and values are opaque bytes (the reducer defines their schema —
/// §4). `BTreeMap` gives us a canonical key order for free, which is what makes both the root hash and
/// `prefix_scan` deterministic.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct Kv {
    map: BTreeMap<Vec<u8>, Vec<u8>>,
}

impl Kv {
    pub fn new() -> Self {
        Kv {
            map: BTreeMap::new(),
        }
    }

    pub fn get(&self, key: &[u8]) -> Option<&[u8]> {
        self.map.get(key).map(|v| v.as_slice())
    }

    pub fn put(&mut self, key: Vec<u8>, value: Vec<u8>) {
        self.map.insert(key, value);
    }

    pub fn delete(&mut self, key: &[u8]) -> bool {
        self.map.remove(key).is_some()
    }

    /// All (key, value) pairs whose key starts with `prefix`, in canonical (byte-ascending) key order.
    /// Deterministic (§16c-S8): the order is a pure function of the keys, never insertion history.
    pub fn prefix_scan(&self, prefix: &[u8]) -> Vec<(&[u8], &[u8])> {
        self.map
            .range(prefix.to_vec()..)
            .take_while(|(k, _)| k.starts_with(prefix))
            .map(|(k, v)| (k.as_slice(), v.as_slice()))
            .collect()
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Content-address the entire KV (§4 free snapshot). Canonical: entries are hashed in sorted key
    /// order with length-prefixing so no two distinct maps collide. Frozen encoding (§16c-S3).
    pub fn root_hash(&self) -> Hash {
        let mut buf = Vec::new();
        buf.extend_from_slice(&(self.map.len() as u64).to_le_bytes());
        for (k, v) in &self.map {
            buf.extend_from_slice(&(k.len() as u64).to_le_bytes());
            buf.extend_from_slice(k);
            buf.extend_from_slice(&(v.len() as u64).to_le_bytes());
            buf.extend_from_slice(v);
        }
        Hash::of(&buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_put_delete() {
        let mut kv = Kv::new();
        assert_eq!(kv.get(b"k"), None);
        kv.put(b"k".to_vec(), b"v".to_vec());
        assert_eq!(kv.get(b"k"), Some(&b"v"[..]));
        assert!(kv.delete(b"k"));
        assert!(!kv.delete(b"k"));
        assert_eq!(kv.get(b"k"), None);
    }

    #[test]
    fn root_hash_is_order_independent_of_insertion() {
        let mut a = Kv::new();
        a.put(b"a".to_vec(), b"1".to_vec());
        a.put(b"b".to_vec(), b"2".to_vec());
        let mut b = Kv::new();
        b.put(b"b".to_vec(), b"2".to_vec());
        b.put(b"a".to_vec(), b"1".to_vec());
        // Same logical contents, different insertion order → same root hash (§4/§16c-S3).
        assert_eq!(a.root_hash(), b.root_hash());
    }

    #[test]
    fn root_hash_changes_with_contents() {
        let mut kv = Kv::new();
        let empty = kv.root_hash();
        kv.put(b"k".to_vec(), b"v".to_vec());
        assert_ne!(kv.root_hash(), empty);
    }

    #[test]
    fn root_hash_no_length_collision() {
        // ("ab","") vs ("a","b") must not collide — length-prefixing prevents it.
        let mut a = Kv::new();
        a.put(b"ab".to_vec(), b"".to_vec());
        let mut b = Kv::new();
        b.put(b"a".to_vec(), b"b".to_vec());
        assert_ne!(a.root_hash(), b.root_hash());
    }

    #[test]
    fn prefix_scan_is_sorted_and_bounded() {
        let mut kv = Kv::new();
        kv.put(b"pending/2".to_vec(), b"y".to_vec());
        kv.put(b"pending/1".to_vec(), b"x".to_vec());
        kv.put(b"other".to_vec(), b"z".to_vec());
        let got = kv.prefix_scan(b"pending/");
        assert_eq!(
            got,
            vec![
                (&b"pending/1"[..], &b"x"[..]),
                (&b"pending/2"[..], &b"y"[..]),
            ]
        );
    }
}
