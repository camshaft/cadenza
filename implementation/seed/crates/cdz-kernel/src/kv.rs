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
use bytes::Bytes;
use std::collections::BTreeMap;

/// The reducer's key-value state. Keys and values are opaque bytes (the reducer defines their schema —
/// §4). `BTreeMap` gives us a canonical key order for free, which is what makes both the root hash and
/// `prefix_scan` deterministic.
///
/// VALUES are stored as [`Bytes`] (Arc-backed), not owned `Vec<u8>` (operator cheaply-clonable directive):
/// [`Kv::clone`] is on the hot path — `fork_for_query` clones the whole KV per debug query, and the §4
/// free-snapshot model conceptually clones per event — so a `Bytes` value makes a clone an O(entries)
/// refcount-bump instead of an O(total-value-bytes) deep copy. The public API is UNCHANGED: `put` still
/// takes `Vec<u8>` (converted to `Bytes` on insert, a move not a copy), `get`/`prefix_scan` still hand
/// back `&[u8]`, and `encode`/`decode`/`root_hash` produce the identical frozen bytes — so no reducer,
/// no caller, and no on-disk/CAS form changes. Keys stay `Vec<u8>` (small, and the BTreeMap ordering key).
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct Kv {
    map: BTreeMap<Vec<u8>, Bytes>,
}

impl Kv {
    pub fn new() -> Self {
        Kv {
            map: BTreeMap::new(),
        }
    }

    pub fn get(&self, key: &[u8]) -> Option<&[u8]> {
        self.map.get(key).map(|v| v.as_ref())
    }

    pub fn put(&mut self, key: Vec<u8>, value: Vec<u8>) {
        // `Vec<u8> -> Bytes` is a MOVE (Bytes::from takes ownership of the Vec's allocation), not a copy —
        // so the public `put(Vec)` signature is unchanged and no extra allocation happens on insert.
        self.map.insert(key, Bytes::from(value));
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
            .map(|(k, v)| (k.as_slice(), v.as_ref()))
            .collect()
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// The KV's canonical byte serialization (§4 / §16c-S3 frozen encoding): entry count, then each
    /// `(key, value)` in sorted key order, every field `u64`-length-prefixed so no two distinct maps
    /// collide. This is the ONE canonical form — [`Kv::root_hash`] hashes exactly these bytes, and
    /// [`Kv::decode`] reconstructs the map from them. Producing the bytes (not just their hash) is what
    /// makes a snapshot RESTORABLE: store `encode()` in the blob store keyed by `root_hash()`, and a
    /// recovering/fast-forwarding session `decode`s it instead of replaying the whole log (§4 — the
    /// snapshot `(seq, root_hash, reducer)` is only a real checkpoint if the bytes it addresses exist).
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&(self.map.len() as u64).to_le_bytes());
        for (k, v) in &self.map {
            buf.extend_from_slice(&(k.len() as u64).to_le_bytes());
            buf.extend_from_slice(k);
            buf.extend_from_slice(&(v.len() as u64).to_le_bytes());
            buf.extend_from_slice(v);
        }
        buf
    }

    /// Reconstruct a KV from its canonical [`Kv::encode`] bytes. Total (§17): any malformed/truncated
    /// input yields `Err`, never a panic — the bytes come from CAS/an untrusted store, so a corrupt
    /// snapshot must fail cleanly (the caller falls back to log replay), not crash. A successful decode
    /// followed by `encode` is byte-identical (round-trips), so `decode(x).root_hash() == the key x was
    /// stored under` — the integrity check a CAS-backed snapshot restore performs.
    pub fn decode(bytes: &[u8]) -> Result<Kv, KvDecodeError> {
        let mut pos = 0usize;
        let take = |pos: &mut usize, n: usize| -> Result<&[u8], KvDecodeError> {
            let end = pos.checked_add(n).ok_or(KvDecodeError::BadLength)?;
            let slice = bytes.get(*pos..end).ok_or(KvDecodeError::Truncated)?;
            *pos = end;
            Ok(slice)
        };
        let read_u64 = |pos: &mut usize| -> Result<u64, KvDecodeError> {
            let b = take(pos, 8)?;
            Ok(u64::from_le_bytes(b.try_into().expect("took exactly 8")))
        };
        let read_len = |pos: &mut usize| -> Result<usize, KvDecodeError> {
            usize::try_from(read_u64(pos)?).map_err(|_| KvDecodeError::BadLength)
        };
        let count = read_len(&mut pos)?;
        let mut map = BTreeMap::new();
        let mut prev_key: Option<Vec<u8>> = None;
        for _ in 0..count {
            let klen = read_len(&mut pos)?;
            let key = take(&mut pos, klen)?.to_vec();
            let vlen = read_len(&mut pos)?;
            let val = Bytes::copy_from_slice(take(&mut pos, vlen)?);
            // The canonical form is sorted, ascending, with no duplicate keys. Reject bytes that
            // violate that (a non-canonical or tampered encoding) rather than silently accepting a form
            // whose re-`encode` wouldn't reproduce it — that would break the root-hash integrity check.
            if let Some(prev) = &prev_key {
                if key <= *prev {
                    return Err(KvDecodeError::NotCanonical);
                }
            }
            prev_key = Some(key.clone());
            map.insert(key, val);
        }
        // Exactly the framed bytes must be consumed — trailing bytes are corruption, not a valid KV.
        if pos != bytes.len() {
            return Err(KvDecodeError::TrailingBytes);
        }
        Ok(Kv { map })
    }

    /// Content-address the entire KV (§4 free snapshot). Hashes the canonical [`Kv::encode`] bytes, so
    /// the hash and the stored/restorable form are ONE frozen encoding (§16c-S3) — no drift between what
    /// `root_hash` addresses and what `decode` reconstructs.
    pub fn root_hash(&self) -> Hash {
        Hash::of(&self.encode())
    }
}

/// A KV snapshot decode failure. Total decode (§17): a corrupt/tampered snapshot from CAS fails cleanly
/// so the caller can fall back to log replay, never panics.
#[derive(Debug, PartialEq, Eq)]
pub enum KvDecodeError {
    /// Ran out of bytes mid-field (truncated snapshot).
    Truncated,
    /// A length field doesn't fit `usize` (32-bit) — reject rather than wrap-truncate into a mis-parse.
    BadLength,
    /// Keys weren't strictly ascending / a duplicate key — not the canonical sorted form, so its
    /// re-`encode` wouldn't reproduce it (would break the root-hash integrity check).
    NotCanonical,
    /// Bytes remained after the framed entries — corruption, not a valid canonical KV.
    TrailingBytes,
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
    fn encode_decode_round_trips_and_matches_root_hash() {
        let mut kv = Kv::new();
        kv.put(b"b".to_vec(), b"2".to_vec());
        kv.put(b"a".to_vec(), b"1".to_vec());
        kv.put(b"ab".to_vec(), b"".to_vec()); // empty value + length-prefix collision guard
        let bytes = kv.encode();
        // Restorable: decode reconstructs the exact map (a snapshot fast-forward, not a log replay).
        let restored = Kv::decode(&bytes).expect("round-trip");
        assert_eq!(restored, kv);
        // The stored bytes are addressed by root_hash — decode∘encode preserves it (the CAS integrity
        // check a snapshot restore performs).
        assert_eq!(restored.root_hash(), kv.root_hash());
        assert_eq!(Hash::of(&bytes), kv.root_hash());
    }

    #[test]
    fn empty_kv_round_trips() {
        let kv = Kv::new();
        assert_eq!(Kv::decode(&kv.encode()).unwrap(), kv);
    }

    #[test]
    fn decode_is_total_on_bad_input() {
        // Truncated: every proper prefix of a valid encoding must Err, never panic (§17).
        let mut kv = Kv::new();
        kv.put(b"k".to_vec(), b"vvvv".to_vec());
        let bytes = kv.encode();
        for cut in 0..bytes.len() {
            assert!(Kv::decode(&bytes[..cut]).is_err(), "prefix {cut} must Err");
        }
        // Trailing bytes after a valid frame → corruption, not a valid KV.
        let mut extra = bytes.clone();
        extra.push(0xFF);
        assert_eq!(Kv::decode(&extra), Err(KvDecodeError::TrailingBytes));
        // Non-canonical (keys not strictly ascending): hand-build count=2 with "b" then "a".
        let mut bad = Vec::new();
        bad.extend_from_slice(&2u64.to_le_bytes()); // count
        for (k, v) in [(&b"b"[..], &b"1"[..]), (&b"a"[..], &b"2"[..])] {
            bad.extend_from_slice(&(k.len() as u64).to_le_bytes());
            bad.extend_from_slice(k);
            bad.extend_from_slice(&(v.len() as u64).to_le_bytes());
            bad.extend_from_slice(v);
        }
        assert_eq!(Kv::decode(&bad), Err(KvDecodeError::NotCanonical));
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
