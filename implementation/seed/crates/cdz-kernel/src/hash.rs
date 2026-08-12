//! Content addressing — the one hashing primitive the whole kernel is built on.
//!
//! Everything KERNEL-INTERNAL and durable (events, KV nodes, blobs, reducer wasm, snapshots) is addressed
//! by the blake3 hash of its canonical bytes. Design invariant (§4/§16c-S3/S8): the *encoding* that feeds
//! this hash must be **canonical and frozen** — the same logical value must always produce the same bytes,
//! or replay/snapshot verification silently rots. For v0 we don't claim cross-version replay, but we DO
//! fix the encoding now so we never have to migrate it later.
//!
//! ONE exception, documented so it isn't mistaken for uniformity: the EXTERNAL on-disk component store
//! (the seed/nix `CDZ_STORE`) is **SHA-256**-addressed by its producers (`xtask`, `cdz-run`, v-nix's
//! `componentStore`), and `REQUIRED_RUNTIME_HASH` IS that SHA-256. So [`crate::component_store`] — the ONE
//! reader of that external store — content-verifies with SHA-256 to match it, NOT with [`Hash::of`]. That
//! is the single dual-hash boundary; `Hash::of` (blake3) is for kernel-internal durable state only, and
//! the two address spaces never cross except at that one reader.

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

    /// URL-safe base64 (RFC 4648 §5 alphabet, NO padding) — the ENCODE-ONLY display form for the two
    /// permitted hash-to-string sites (operator directive 2026-08-12): (1) tracing/log output, (2)
    /// rendering FS/S3 paths. base64url is ~1.33x the 32 raw bytes (43 chars) vs hex's 2x (64 chars),
    /// and its alphabet (`A-Z a-z 0-9 - _`) is filesystem/URL safe with no separators. Deliberately
    /// ENCODE-ONLY: there is NO base64 DECODE anywhere — a runtime hash is raw bytes produced only by
    /// hashing, never parsed back from a string (that is the whole point of the no-`from_hex` sweep).
    pub fn to_base64url(&self) -> String {
        const ALPHABET: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let bytes = &self.0;
        // 32 bytes → 10 full 3-byte groups (40 chars) + a 2-byte tail (3 chars) = 43 chars, no padding.
        let mut out = String::with_capacity(43);
        let mut i = 0;
        while i + 3 <= bytes.len() {
            let n = (u32::from(bytes[i]) << 16)
                | (u32::from(bytes[i + 1]) << 8)
                | u32::from(bytes[i + 2]);
            out.push(ALPHABET[((n >> 18) & 63) as usize] as char);
            out.push(ALPHABET[((n >> 12) & 63) as usize] as char);
            out.push(ALPHABET[((n >> 6) & 63) as usize] as char);
            out.push(ALPHABET[(n & 63) as usize] as char);
            i += 3;
        }
        match bytes.len() - i {
            1 => {
                let n = u32::from(bytes[i]) << 16;
                out.push(ALPHABET[((n >> 18) & 63) as usize] as char);
                out.push(ALPHABET[((n >> 12) & 63) as usize] as char);
            }
            2 => {
                let n = (u32::from(bytes[i]) << 16) | (u32::from(bytes[i + 1]) << 8);
                out.push(ALPHABET[((n >> 18) & 63) as usize] as char);
                out.push(ALPHABET[((n >> 12) & 63) as usize] as char);
                out.push(ALPHABET[((n >> 6) & 63) as usize] as char);
            }
            _ => {}
        }
        out
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
        // Short prefix is enough to eyeball in test output. Debug is diagnostic/trace output, one of the
        // two permitted encode sites, so it uses the base64url display form (`to_base64url`), not hex.
        write!(f, "Hash({}…)", &self.to_base64url()[..12])
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
    fn base64url_is_43_chars_no_padding_and_url_safe_alphabet() {
        let s = Hash::of(b"x").to_base64url();
        // 32 bytes → 43 base64 chars, no `=` padding.
        assert_eq!(s.len(), 43);
        assert!(!s.contains('='));
        // Only the url-safe alphabet: A-Z a-z 0-9 - _ (no `+` or `/`).
        assert!(
            s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
            "unexpected char in {s}"
        );
    }

    #[test]
    fn base64url_known_vectors() {
        // All-zero 32 bytes → 43 'A' (index 0 of the alphabet).
        assert_eq!(Hash::from_bytes([0u8; 32]).to_base64url(), "A".repeat(43));
        // All-0xFF 32 bytes: ten 0xFFFFFF groups → 40 '_' (index 63), then the 2-byte tail
        // 0xFFFF → sextets 63,63,60 → '_','_','8' ⇒ 42 '_' followed by '8'.
        assert_eq!(
            Hash::from_bytes([0xFFu8; 32]).to_base64url(),
            format!("{}8", "_".repeat(42))
        );
    }

    #[test]
    fn base64url_distinguishes_and_is_stable() {
        let a = Hash::of(b"alpha").to_base64url();
        let b = Hash::of(b"beta").to_base64url();
        assert_ne!(a, b);
        // Deterministic: same content, same encoding.
        assert_eq!(a, Hash::of(b"alpha").to_base64url());
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
