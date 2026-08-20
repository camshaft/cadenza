//! Content hashing — the platform's sole identity (`design/cadenza-platform.md` section 8).
//!
//! A [`Hash`] is the blake3 digest of some bytes. It names *and* authorizes: the content-addressed
//! store is unpermissioned because possessing a hash is what lets you read its bytes ("the hash is the
//! capability"). Two things reduce to hashing: a contract-id is the hash of a contract declaration
//! (section 1), and a blob is addressed by the hash of its bytes (section 8). So this is the bottom
//! primitive everything else routes and addresses by.
//!
//! Algorithm: **blake3** — the one content-address digest the fleet unified on (operator 2026-08-08),
//! fast and 32 bytes. A `Hash` is those 32 raw bytes: a fixed-size digest, so `[u8; 32]` (`Copy`, no
//! allocation) is its representation — the "`Bytes`, not `Vec<u8>`" convention is for *variable-length*
//! buffers, not a fixed digest.
//!
//! Text form: **base64url** — the URL-safe alphabet, unpadded — never hex, wherever a hash is rendered
//! as text (a name, a log line, an error, a wire field); section 8. [`Hash`]'s `Display` and `FromStr`
//! are that rendering and its inverse.

use std::fmt;
use std::str::FromStr;

/// The blake3 content hash of some bytes — the platform's sole identity (section 8).
///
/// `Copy` + cheaply comparable: a hash threads through routing, dispatch, and the store constantly, so
/// it must be trivially clonable. Ordering is by raw bytes (a total order for use as a map/set key).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Hash([u8; 32]);

impl Hash {
    /// The number of raw bytes in a hash (blake3's 256-bit digest).
    pub const LEN: usize = 32;

    /// The content hash of `bytes`. Deterministic: the same bytes always hash equal (that is what makes
    /// content addressing work), so this is a pure function of its input.
    #[must_use]
    pub fn of(bytes: &[u8]) -> Self {
        Self(*blake3::hash(bytes).as_bytes())
    }

    /// Wrap 32 raw digest bytes as a `Hash` (e.g. read back from the store or the wire).
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// The raw digest bytes. A hash IS raw bytes (section 8); this is the on-wire / in-store form.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// The base64url (URL-safe, unpadded) text rendering — the ONE textual form of a hash (section 8).
    /// Equivalent to `to_string()`; offered by name so call sites read as intent, not `Display` incidental.
    #[must_use]
    pub fn to_base64url(&self) -> String {
        base64url::encode(&self.0)
    }
}

/// base64url renders every textual hash (section 8): a name, a log line, an error all show this.
impl fmt::Display for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&base64url::encode(&self.0))
    }
}

/// A hash is opaque bytes; Debug shows the base64url text (not a 32-element byte array) so log/panic
/// output is the same identity a human reads everywhere else.
impl fmt::Debug for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Hash({})", base64url::encode(&self.0))
    }
}

/// Parse a hash from its base64url text form (the inverse of `Display`).
impl FromStr for Hash {
    type Err = HashParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes = base64url::decode(s).map_err(HashParseError::NotBase64url)?;
        let len = bytes.len();
        let arr: [u8; 32] = bytes
            .try_into()
            .map_err(|_| HashParseError::WrongLength(len))?;
        Ok(Self(arr))
    }
}

/// Why a string is not a valid hash text.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HashParseError {
    /// The text is not valid canonical base64url (bad character, impossible length, or non-canonical
    /// trailing bits).
    NotBase64url(base64url::DecodeError),
    /// Valid base64url, but not exactly 32 bytes — so not a blake3 digest. Carries the decoded length.
    WrongLength(usize),
}

impl fmt::Display for HashParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotBase64url(e) => write!(f, "hash text is not valid base64url: {e}"),
            Self::WrongLength(n) => {
                write!(f, "hash text decoded to {n} bytes, expected {}", Hash::LEN)
            }
        }
    }
}

impl fmt::Debug for HashParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl std::error::Error for HashParseError {}

/// base64url — the URL-safe base64 alphabet (`A-Z a-z 0-9 - _`), unpadded — the ONE textual byte
/// encoding for the platform (section 8: "base64url … never hex"). Hand-rolled to keep the crate's dep
/// floor at just blake3; decoding is strict/canonical so a hash has exactly one text form.
pub mod base64url {
    use std::fmt;

    /// index (0..64) -> character. `-` is 62, `_` is 63 (the URL-safe pair, vs standard base64's `+`/`/`).
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

    /// character -> 6-bit value, or 0xFF for "not an alphabet character". Built once at const time so
    /// decode is a table lookup, not a scan of `ALPHABET`.
    const REVERSE: [u8; 256] = {
        let mut t = [0xFFu8; 256];
        let mut i = 0;
        while i < 64 {
            t[ALPHABET[i] as usize] = i as u8;
            i += 1;
        }
        t
    };

    /// Encode bytes as unpadded base64url. Empty input encodes to the empty string.
    #[must_use]
    pub fn encode(bytes: &[u8]) -> String {
        // Every 3 input bytes -> 4 output chars; a trailing 1 or 2 bytes -> 2 or 3 chars (no padding).
        let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
        let mut chunks = bytes.chunks_exact(3);
        for c in &mut chunks {
            let n = (u32::from(c[0]) << 16) | (u32::from(c[1]) << 8) | u32::from(c[2]);
            out.push(ALPHABET[(n >> 18) as usize & 0x3F] as char);
            out.push(ALPHABET[(n >> 12) as usize & 0x3F] as char);
            out.push(ALPHABET[(n >> 6) as usize & 0x3F] as char);
            out.push(ALPHABET[n as usize & 0x3F] as char);
        }
        match chunks.remainder() {
            [a] => {
                let n = u32::from(*a) << 16;
                out.push(ALPHABET[(n >> 18) as usize & 0x3F] as char);
                out.push(ALPHABET[(n >> 12) as usize & 0x3F] as char);
            }
            [a, b] => {
                let n = (u32::from(*a) << 16) | (u32::from(*b) << 8);
                out.push(ALPHABET[(n >> 18) as usize & 0x3F] as char);
                out.push(ALPHABET[(n >> 12) as usize & 0x3F] as char);
                out.push(ALPHABET[(n >> 6) as usize & 0x3F] as char);
            }
            _ => {}
        }
        out
    }

    /// Decode canonical unpadded base64url. Strict: rejects a non-alphabet character, an impossible
    /// length (`len % 4 == 1`, which no byte string produces), and non-canonical trailing bits (the
    /// unused low bits of the last character must be zero) — so a given byte string has exactly ONE
    /// valid text form, which content addressing relies on.
    ///
    /// # Errors
    /// Returns [`DecodeError`] describing the first violation.
    pub fn decode(s: &str) -> Result<Vec<u8>, DecodeError> {
        let s = s.as_bytes();
        if s.len() % 4 == 1 {
            return Err(DecodeError::InvalidLength(s.len()));
        }
        let mut out = Vec::with_capacity(s.len() / 4 * 3);
        let mut chunks = s.chunks_exact(4);
        for c in &mut chunks {
            let n = (sextet(c[0])? << 18)
                | (sextet(c[1])? << 12)
                | (sextet(c[2])? << 6)
                | sextet(c[3])?;
            out.push((n >> 16) as u8);
            out.push((n >> 8) as u8);
            out.push(n as u8);
        }
        match chunks.remainder() {
            [] => {}
            [a, b] => {
                // 2 chars -> 1 byte: the 2nd char contributes 6 bits but only its top 2 are used; its
                // low 4 bits must be zero for a canonical encoding.
                let (a, b) = (sextet(*a)?, sextet(*b)?);
                if b & 0x0F != 0 {
                    return Err(DecodeError::NonCanonicalTrailingBits);
                }
                out.push(((a << 2) | (b >> 4)) as u8);
            }
            [a, b, c] => {
                // 3 chars -> 2 bytes: the 3rd char's low 2 bits are unused and must be zero.
                let (a, b, c) = (sextet(*a)?, sextet(*b)?, sextet(*c)?);
                if c & 0x03 != 0 {
                    return Err(DecodeError::NonCanonicalTrailingBits);
                }
                out.push(((a << 2) | (b >> 4)) as u8);
                out.push(((b << 4) | (c >> 2)) as u8);
            }
            // len % 4 == 1 was rejected above; no other remainder is possible.
            _ => unreachable!("chunks_exact(4) remainder is 0, 2, or 3 after the len%4==1 guard"),
        }
        Ok(out)
    }

    /// Map one character to its 6-bit value, or error if it is not in the alphabet.
    fn sextet(c: u8) -> Result<u32, DecodeError> {
        let v = REVERSE[c as usize];
        if v == 0xFF {
            Err(DecodeError::InvalidChar(c as char))
        } else {
            Ok(u32::from(v))
        }
    }

    /// Why a string is not canonical base64url.
    #[derive(Clone, Copy, PartialEq, Eq)]
    pub enum DecodeError {
        /// A character outside the URL-safe alphabet (`A-Z a-z 0-9 - _`) — e.g. base64's `+`/`/`, a `=`
        /// pad, or whitespace.
        InvalidChar(char),
        /// A length no byte string encodes to (`len % 4 == 1`).
        InvalidLength(usize),
        /// The unused low bits of the final character are nonzero, so the text is a non-canonical
        /// encoding of its bytes.
        NonCanonicalTrailingBits,
    }

    impl fmt::Display for DecodeError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::InvalidChar(c) => write!(f, "invalid base64url character {c:?}"),
                Self::InvalidLength(n) => write!(f, "invalid base64url length {n} (len % 4 == 1)"),
                Self::NonCanonicalTrailingBits => {
                    write!(f, "non-canonical base64url (nonzero unused trailing bits)")
                }
            }
        }
    }

    impl fmt::Debug for DecodeError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            fmt::Display::fmt(self, f)
        }
    }

    impl std::error::Error for DecodeError {}
}

#[cfg(test)]
mod tests {
    use super::base64url::{self, DecodeError};
    use super::{Hash, HashParseError};

    // ── base64url: pin the encoder against hand-verifiable RFC 4648 vectors ─────────────────────
    #[test]
    fn base64url_encodes_the_rfc4648_ascii_vectors_unpadded() {
        assert_eq!(base64url::encode(b""), "");
        assert_eq!(base64url::encode(b"f"), "Zg");
        assert_eq!(base64url::encode(b"fo"), "Zm8");
        assert_eq!(base64url::encode(b"foo"), "Zm9v");
        assert_eq!(base64url::encode(b"foob"), "Zm9vYg");
        assert_eq!(base64url::encode(b"fooba"), "Zm9vYmE");
        assert_eq!(base64url::encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn base64url_uses_the_url_safe_pair_dash_and_underscore() {
        // 0xFF,0xFF,0xFF -> six 1-bits groups -> the two top-index chars. index 62 = '-', 63 = '_'.
        // bytes chosen so both 62 and 63 appear (and never '+' or '/').
        let e = base64url::encode(&[0xFB, 0xFF, 0xBF]);
        assert!(e.contains('-') || e.contains('_'), "got {e}");
        assert!(
            !e.contains('+') && !e.contains('/'),
            "url-safe alphabet only, got {e}"
        );
        assert_eq!(base64url::encode(&[0xFF, 0xFF, 0xFF]), "____");
    }

    #[test]
    fn base64url_round_trips_every_small_length() {
        for len in 0..=64usize {
            // a deterministic, byte-varying buffer (no rng in tests).
            let buf: Vec<u8> = (0..len)
                .map(|i| (i as u8).wrapping_mul(31).wrapping_add(7))
                .collect();
            let text = base64url::encode(&buf);
            assert_eq!(
                base64url::decode(&text).unwrap(),
                buf,
                "round-trip failed at len {len}"
            );
        }
    }

    #[test]
    fn base64url_decode_rejects_bad_input() {
        assert_eq!(
            base64url::decode("Zg=="),
            Err(DecodeError::InvalidChar('='))
        ); // padding banned
        assert_eq!(
            base64url::decode("Zm9+"),
            Err(DecodeError::InvalidChar('+'))
        ); // std-base64 char
        assert_eq!(
            base64url::decode("Zm9/"),
            Err(DecodeError::InvalidChar('/'))
        );
        assert_eq!(base64url::decode("Z m"), Err(DecodeError::InvalidChar(' ')));
        assert_eq!(base64url::decode("A"), Err(DecodeError::InvalidLength(1))); // len%4==1
        assert_eq!(
            base64url::decode("ABCDE"),
            Err(DecodeError::InvalidLength(5))
        );
        // non-canonical: "Zh" decodes 'f' but 'h'(=33) has nonzero low-4 bits (33 & 0x0F = 1).
        assert_eq!(
            base64url::decode("Zh"),
            Err(DecodeError::NonCanonicalTrailingBits)
        );
    }

    // ── Hash ────────────────────────────────────────────────────────────────────────────────────
    #[test]
    fn hash_of_matches_blake3_and_is_deterministic() {
        // Golden: the documented blake3 digest of the empty input — pins the ALGORITHM (a swap to
        // sha256 or a different digest would flip this).
        let empty = Hash::of(b"");
        let blake3_empty_hex = "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262";
        assert_eq!(hex(empty.as_bytes()), blake3_empty_hex);
        // Deterministic + distinguishing.
        assert_eq!(Hash::of(b"cadenza"), Hash::of(b"cadenza"));
        assert_ne!(Hash::of(b"cadenza"), Hash::of(b"cadenzb"));
        assert_eq!(Hash::LEN, 32);
    }

    #[test]
    fn hash_text_is_base64url_and_round_trips() {
        let h = Hash::of(b"the hash is the capability");
        let text = h.to_string();
        assert_eq!(text, h.to_base64url());
        assert_eq!(text, base64url::encode(h.as_bytes()));
        // 32 bytes -> 43 unpadded base64url chars.
        assert_eq!(text.len(), 43);
        assert!(!text.contains('='), "unpadded");
        // Display -> FromStr is the identity.
        assert_eq!(text.parse::<Hash>().unwrap(), h);
    }

    #[test]
    fn hash_from_str_rejects_wrong_length_and_bad_text() {
        // Valid base64url but not 32 bytes -> WrongLength (3 bytes here).
        assert_eq!("Zm9v".parse::<Hash>(), Err(HashParseError::WrongLength(3)));
        // Not base64url at all.
        assert!(matches!(
            "not valid!".parse::<Hash>(),
            Err(HashParseError::NotBase64url(_))
        ));
        // A truncated hash text is rejected (either as the wrong decoded length, or — since dropping
        // the final char usually leaves a non-canonical trailing group — as invalid base64url). Either
        // way it must NOT parse to a Hash.
        let h = Hash::of(b"x").to_string();
        let short = &h[..h.len() - 1]; // 42 chars
        assert!(short.parse::<Hash>().is_err());
        // A canonical base64url that decodes to a valid-but-wrong length is specifically WrongLength:
        // 44 chars ("A" * 44) is canonical and decodes to 33 bytes.
        assert_eq!(
            "A".repeat(44).parse::<Hash>(),
            Err(HashParseError::WrongLength(33))
        );
    }

    #[test]
    fn hash_from_bytes_and_as_bytes_are_inverse() {
        let bytes = *Hash::of(b"roundtrip the raw digest").as_bytes();
        assert_eq!(*Hash::from_bytes(bytes).as_bytes(), bytes);
    }

    #[test]
    fn hash_debug_shows_base64url() {
        let h = Hash::of(b"debug");
        assert_eq!(format!("{h:?}"), format!("Hash({})", h.to_base64url()));
    }

    /// lowercase hex of raw bytes — TEST-ONLY, to pin the digest against the well-known hex vector.
    /// (Production text is base64url per section 8; hex lives only here.)
    fn hex(bytes: &[u8]) -> String {
        use std::fmt::Write;
        bytes.iter().fold(String::new(), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
    }
}
