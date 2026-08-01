//! Content addressing — the one hashing primitive the whole kernel is built on.
//!
//! Everything durable (events, KV nodes, blobs, reducer wasm, snapshots) is addressed by the blake3
//! hash of its canonical bytes. Design invariant (§4/§16c-S3/S8): the *encoding* that feeds this hash
//! must be **canonical and frozen** — the same logical value must always produce the same bytes, or
//! replay/snapshot verification silently rots. For v0 we don't claim cross-version replay, but we DO
//! fix the encoding now so we never have to migrate it later.

use core::fmt;

/// A content hash: the blake3 digest of some canonical bytes. Opaque, cheap to copy, `Eq`/`Ord` so it
/// can key maps and be totally ordered (needed for deterministic iteration — §16c-S8).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Hash([u8; 32]);

impl Hash {
    /// Content-address a byte slice. This is the ONLY way to make a `Hash` from content, so the
    /// digest algorithm lives in exactly one place.
    pub fn of(bytes: &[u8]) -> Self {
        Hash(*blake3::hash(bytes).as_bytes())
    }

    /// The raw 32 bytes, for storage keys / wire encoding.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Reconstruct from raw bytes (e.g. read back from disk / an envelope).
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Hash(bytes)
    }

    /// Lowercase hex, for logs and debug output.
    pub fn to_hex(&self) -> String {
        let mut s = String::with_capacity(64);
        for b in &self.0 {
            s.push_str(&format!("{b:02x}"));
        }
        s
    }

    /// Parse the canonical hex form (the inverse of [`Hash::to_hex`]): exactly 64 LOWERCASE hex chars.
    /// `None` on any wrong-length / non-hex / UPPERCASE input. Uppercase is rejected on purpose — a
    /// content address is canonical LOWERCASE hex, so admitting `AB..` alongside `ab..` would give one
    /// hash two spellings (i.e. one blob two keys). This is the single home for hex→`Hash` (the inverse
    /// of `to_hex`), so callers that read a hash out of a name/URL/build-metadata (e.g. a component
    /// dependency's `+<hash>`) parse it here rather than reimplementing the rule.
    pub fn from_hex(hex: &str) -> Option<Self> {
        if hex.len() != 64 {
            return None;
        }
        // Reject uppercase explicitly (u8::from_str_radix would accept it): canonical addresses are
        // lowercase, and two spellings of one hash would break content-addressing.
        if hex.bytes().any(|b| b.is_ascii_uppercase()) {
            return None;
        }
        let mut bytes = [0u8; 32];
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = u8::from_str_radix(hex.get(i * 2..i * 2 + 2)?, 16).ok()?;
        }
        Some(Hash(bytes))
    }
}

impl fmt::Debug for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Short prefix is enough to eyeball in test output; full hash via `to_hex`.
        write!(f, "Hash({}…)", &self.to_hex()[..12])
    }
}

impl fmt::Display for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_bytes_same_hash() {
        assert_eq!(Hash::of(b"hello"), Hash::of(b"hello"));
    }

    #[test]
    fn different_bytes_different_hash() {
        assert_ne!(Hash::of(b"hello"), Hash::of(b"world"));
    }

    #[test]
    fn roundtrips_through_raw_bytes() {
        let h = Hash::of(b"content");
        assert_eq!(h, Hash::from_bytes(*h.as_bytes()));
    }

    #[test]
    fn hex_is_64_chars() {
        assert_eq!(Hash::of(b"x").to_hex().len(), 64);
    }

    #[test]
    fn from_hex_round_trips_and_rejects_noncanonical() {
        let h = Hash::of(b"the content");
        // Inverse of to_hex.
        assert_eq!(Hash::from_hex(&h.to_hex()), Some(h));
        // Wrong length / non-hex → None.
        assert_eq!(Hash::from_hex("tooshort"), None);
        assert_eq!(Hash::from_hex(&"z".repeat(64)), None);
        // UPPERCASE is non-canonical → None (else one hash would have two spellings).
        assert_eq!(Hash::from_hex(&h.to_hex().to_uppercase()), None);
    }
}
