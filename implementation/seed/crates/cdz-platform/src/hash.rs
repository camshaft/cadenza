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
//! are that rendering and its inverse, and neither allocates: `Display` streams chars to the formatter,
//! and `FromStr` decodes into the fixed 32-byte array in place.

use std::fmt;
use std::fmt::Write as _; // brings `Formatter::write_char` into scope for the allocation-free Display
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

    /// The number of base64url characters in a hash's text form (`ceil(32 * 4 / 3)`, unpadded).
    pub const TEXT_LEN: usize = 43;

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

    /// The base64url text of this hash as a lazy `char` iterator — the ONE textual form (section 8),
    /// allocation-free. `Display` uses this; a caller that genuinely needs an owned `String` collects it
    /// (an explicit, opt-in allocation) rather than every render paying for one.
    pub fn text(&self) -> base64url::Encode<'_> {
        base64url::encode(&self.0)
    }
}

/// base64url renders every textual hash (section 8) — streamed to the formatter, no intermediate `String`.
impl fmt::Display for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for c in self.text() {
            f.write_char(c)?;
        }
        Ok(())
    }
}

/// A hash is opaque bytes; Debug shows the base64url text (not a 32-element byte array) so log/panic
/// output is the same identity a human reads everywhere else.
impl fmt::Debug for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Hash(")?;
        fmt::Display::fmt(self, f)?;
        f.write_str(")")
    }
}

/// Parse a hash from its base64url text form (the inverse of `Display`). Decodes straight into the fixed
/// 32-byte array — no allocation. The error is a [`base64url::DecodeError`]: a text that decodes to some
/// other length surfaces as [`base64url::DecodeError::OutputLenMismatch`] against the 32-byte target, so
/// "not a valid hash" needs no separate error type on top of the decoder's.
impl FromStr for Hash {
    type Err = base64url::DecodeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut buf = [0u8; 32];
        base64url::decode_into(s, &mut buf)?;
        Ok(Self(buf))
    }
}

/// base64url — the URL-safe base64 alphabet (`A-Z a-z 0-9 - _`), unpadded — the ONE textual byte
/// encoding for the platform (section 8: "base64url … never hex"). Hand-rolled to keep the crate's dep
/// floor at just blake3, and **allocation-free**: [`encode`] is a lazy `char` iterator and
/// [`decode_into`] writes into a caller-sized buffer (a hash's length is always known, so the caller
/// owns the buffer). Decoding is strict/canonical so a byte string has exactly one text form.
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

    /// The number of unpadded base64url characters that `n` bytes encode to.
    #[must_use]
    pub const fn encoded_len(n: usize) -> usize {
        // each full 3-byte group -> 4 chars; a 1- or 2-byte tail -> 2 or 3 chars.
        (n / 3) * 4
            + match n % 3 {
                0 => 0,
                1 => 2,
                _ => 3,
            }
    }

    /// The number of bytes that a `chars`-long canonical base64url string decodes to. `chars % 4 == 1`
    /// is impossible (no byte string encodes to it) and is reported as an error by [`decode_into`], not
    /// here; this returns the length for the reachable cases.
    #[must_use]
    pub const fn decoded_len(chars: usize) -> usize {
        (chars / 4) * 3
            + match chars % 4 {
                2 => 1,
                3 => 2,
                _ => 0,
            }
    }

    /// A lazy iterator over the base64url characters of `bytes` — allocation-free. Collect it into a
    /// `String` only where an owned string is genuinely needed; otherwise write it straight out.
    #[must_use]
    pub fn encode(bytes: &[u8]) -> Encode<'_> {
        Encode {
            bytes,
            byte_pos: 0,
            // pending 6-bit indices produced from the current group, and how many are still to yield.
            pending: [0u8; 4],
            pending_len: 0,
            pending_pos: 0,
        }
    }

    /// The iterator returned by [`encode`]. Yields one `char` at a time; refills from the next up-to-3
    /// input bytes when its 4-char group is drained. [`ExactSizeIterator`], so `.size_hint()` is exact.
    pub struct Encode<'a> {
        bytes: &'a [u8],
        byte_pos: usize,
        pending: [u8; 4],
        pending_len: u8,
        pending_pos: u8,
    }

    impl Iterator for Encode<'_> {
        type Item = char;

        fn next(&mut self) -> Option<char> {
            if self.pending_pos == self.pending_len {
                let rem = &self.bytes[self.byte_pos..];
                let take = rem.len().min(3);
                if take == 0 {
                    return None;
                }
                let a = u32::from(rem[0]);
                let (n, chars) = match take {
                    1 => (a << 16, 2),
                    2 => ((a << 16) | (u32::from(rem[1]) << 8), 3),
                    _ => ((a << 16) | (u32::from(rem[1]) << 8) | u32::from(rem[2]), 4),
                };
                self.pending[0] = ((n >> 18) & 0x3F) as u8;
                self.pending[1] = ((n >> 12) & 0x3F) as u8;
                self.pending[2] = ((n >> 6) & 0x3F) as u8;
                self.pending[3] = (n & 0x3F) as u8;
                self.pending_len = chars;
                self.pending_pos = 0;
                self.byte_pos += take;
            }
            let idx = self.pending[self.pending_pos as usize];
            self.pending_pos += 1;
            Some(ALPHABET[idx as usize] as char)
        }

        fn size_hint(&self) -> (usize, Option<usize>) {
            let buffered = usize::from(self.pending_len - self.pending_pos);
            let rest = encoded_len(self.bytes.len() - self.byte_pos);
            let n = buffered + rest;
            (n, Some(n))
        }
    }

    impl ExactSizeIterator for Encode<'_> {}

    /// Decode canonical unpadded base64url into `out`, which the caller sizes to the known decoded
    /// length ([`decoded_len`]). No allocation. Strict: rejects a non-alphabet character, an impossible
    /// length (`chars % 4 == 1`), non-canonical trailing bits (the unused low bits of the last character
    /// must be zero), and an `out` whose length is not exactly the decoded length — so a given byte
    /// string has exactly ONE valid text form, which content addressing relies on.
    ///
    /// # Errors
    /// Returns the first [`DecodeError`] encountered (length/output-size checks before per-char decode).
    pub fn decode_into(s: &str, out: &mut [u8]) -> Result<(), DecodeError> {
        let s = s.as_bytes();
        if s.len() % 4 == 1 {
            return Err(DecodeError::InvalidLength(s.len()));
        }
        let expected = decoded_len(s.len());
        if out.len() != expected {
            return Err(DecodeError::OutputLenMismatch {
                expected,
                got: out.len(),
            });
        }
        let mut oi = 0;
        let mut chunks = s.chunks_exact(4);
        for c in &mut chunks {
            let n = (sextet(c[0])? << 18)
                | (sextet(c[1])? << 12)
                | (sextet(c[2])? << 6)
                | sextet(c[3])?;
            out[oi] = (n >> 16) as u8;
            out[oi + 1] = (n >> 8) as u8;
            out[oi + 2] = n as u8;
            oi += 3;
        }
        match chunks.remainder() {
            [] => {}
            [a, b] => {
                // 2 chars -> 1 byte: the 2nd char's low 4 bits are unused and must be zero (canonical).
                let (a, b) = (sextet(*a)?, sextet(*b)?);
                if b & 0x0F != 0 {
                    return Err(DecodeError::NonCanonicalTrailingBits);
                }
                out[oi] = ((a << 2) | (b >> 4)) as u8;
            }
            [a, b, c] => {
                // 3 chars -> 2 bytes: the 3rd char's low 2 bits are unused and must be zero.
                let (a, b, c) = (sextet(*a)?, sextet(*b)?, sextet(*c)?);
                if c & 0x03 != 0 {
                    return Err(DecodeError::NonCanonicalTrailingBits);
                }
                out[oi] = ((a << 2) | (b >> 4)) as u8;
                out[oi + 1] = ((b << 4) | (c >> 2)) as u8;
            }
            // chars % 4 == 1 was rejected above; no other remainder is possible.
            _ => unreachable!("chunks_exact(4) remainder is 0, 2, or 3 after the len%4==1 guard"),
        }
        Ok(())
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

    /// Why a string is not canonical base64url decodable into the given buffer.
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
        /// The output buffer's length is not the number of bytes this text decodes to.
        OutputLenMismatch { expected: usize, got: usize },
    }

    impl fmt::Display for DecodeError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::InvalidChar(c) => write!(f, "invalid base64url character {c:?}"),
                Self::InvalidLength(n) => write!(f, "invalid base64url length {n} (len % 4 == 1)"),
                Self::NonCanonicalTrailingBits => {
                    write!(f, "non-canonical base64url (nonzero unused trailing bits)")
                }
                Self::OutputLenMismatch { expected, got } => {
                    write!(
                        f,
                        "output buffer is {got} bytes, text decodes to {expected}"
                    )
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
    use super::Hash;
    use super::base64url::{self, DecodeError};

    /// Test helper: collect the encode iterator into a `String` (the tests want to compare text).
    fn enc(bytes: &[u8]) -> String {
        base64url::encode(bytes).collect()
    }

    /// Test helper: decode into a right-sized buffer (the caller always knows the length).
    fn dec(s: &str) -> Result<Vec<u8>, DecodeError> {
        let mut out = vec![0u8; base64url::decoded_len(s.len())];
        base64url::decode_into(s, &mut out).map(|()| out)
    }

    // ── base64url: pin the encoder against hand-verifiable RFC 4648 vectors ─────────────────────
    #[test]
    fn base64url_encodes_the_rfc4648_ascii_vectors_unpadded() {
        assert_eq!(enc(b""), "");
        assert_eq!(enc(b"f"), "Zg");
        assert_eq!(enc(b"fo"), "Zm8");
        assert_eq!(enc(b"foo"), "Zm9v");
        assert_eq!(enc(b"foob"), "Zm9vYg");
        assert_eq!(enc(b"fooba"), "Zm9vYmE");
        assert_eq!(enc(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn base64url_uses_the_url_safe_pair_dash_and_underscore() {
        let e = enc(&[0xFB, 0xFF, 0xBF]);
        assert!(e.contains('-') || e.contains('_'), "got {e}");
        assert!(
            !e.contains('+') && !e.contains('/'),
            "url-safe alphabet only, got {e}"
        );
        assert_eq!(enc(&[0xFF, 0xFF, 0xFF]), "____");
    }

    #[test]
    fn encode_iterator_size_hint_is_exact() {
        for len in 0..=9usize {
            let buf: Vec<u8> = (0..len).map(|i| i as u8).collect();
            let it = base64url::encode(&buf);
            let expected = base64url::encoded_len(len);
            assert_eq!(it.size_hint(), (expected, Some(expected)));
            assert_eq!(it.count(), expected);
        }
    }

    #[test]
    fn base64url_round_trips_every_small_length() {
        for len in 0..=64usize {
            // a deterministic, byte-varying buffer (no rng in tests).
            let buf: Vec<u8> = (0..len)
                .map(|i| (i as u8).wrapping_mul(31).wrapping_add(7))
                .collect();
            let text = enc(&buf);
            assert_eq!(dec(&text).unwrap(), buf, "round-trip failed at len {len}");
        }
    }

    #[test]
    fn base64url_decode_rejects_bad_input() {
        assert_eq!(dec("Zg=="), Err(DecodeError::InvalidChar('='))); // padding banned
        assert_eq!(dec("Zm9+"), Err(DecodeError::InvalidChar('+'))); // std-base64 chars
        assert_eq!(dec("Zm9/"), Err(DecodeError::InvalidChar('/')));
        assert_eq!(dec("Z m"), Err(DecodeError::InvalidChar(' ')));
        assert_eq!(dec("A"), Err(DecodeError::InvalidLength(1))); // len % 4 == 1
        assert_eq!(dec("ABCDE"), Err(DecodeError::InvalidLength(5)));
        // non-canonical: "Zh" decodes 'f' but 'h'(=33) has nonzero low-4 bits (33 & 0x0F = 1).
        assert_eq!(dec("Zh"), Err(DecodeError::NonCanonicalTrailingBits));
    }

    #[test]
    fn decode_into_rejects_a_wrong_sized_buffer() {
        // "Zm9v" decodes to 3 bytes; a 2- or 4-byte buffer is a mismatch.
        let mut two = [0u8; 2];
        assert_eq!(
            base64url::decode_into("Zm9v", &mut two),
            Err(DecodeError::OutputLenMismatch {
                expected: 3,
                got: 2
            })
        );
        let mut four = [0u8; 4];
        assert_eq!(
            base64url::decode_into("Zm9v", &mut four),
            Err(DecodeError::OutputLenMismatch {
                expected: 3,
                got: 4
            })
        );
        let mut three = [0u8; 3];
        assert!(base64url::decode_into("Zm9v", &mut three).is_ok());
        assert_eq!(&three, b"foo");
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
        assert_eq!(text, h.text().collect::<String>());
        assert_eq!(text, base64url::encode(h.as_bytes()).collect::<String>());
        // 32 bytes -> 43 unpadded base64url chars.
        assert_eq!(text.len(), Hash::TEXT_LEN);
        assert_eq!(text.len(), 43);
        assert!(!text.contains('='), "unpadded");
        // Display -> FromStr is the identity.
        assert_eq!(text.parse::<Hash>().unwrap(), h);
    }

    #[test]
    fn hash_from_str_rejects_wrong_length_and_bad_text() {
        // Valid base64url but not 32 bytes -> OutputLenMismatch against the 32-byte target (3 bytes here).
        // from_str decodes into a fixed [u8;32], so a non-43-char text mismatches regardless of its chars.
        assert_eq!(
            "Zm9v".parse::<Hash>(),
            Err(DecodeError::OutputLenMismatch {
                expected: 3,
                got: 32
            })
        );
        // A 43-char text (decodes to exactly 32 bytes) with a bad char reaches char-validation ->
        // InvalidChar. Take a real hash's text and corrupt one char to a non-alphabet '!'.
        let good = Hash::of(b"corrupt me").to_string();
        assert_eq!(good.len(), 43);
        let bad = format!("{}!", &good[..42]); // still 43 chars, but char 43 is invalid
        assert_eq!(bad.parse::<Hash>(), Err(DecodeError::InvalidChar('!')));
        // A truncated hash text is rejected (either wrong decoded length or a non-canonical group).
        let h = Hash::of(b"x").to_string();
        assert!(h[..h.len() - 1].parse::<Hash>().is_err());
        // A canonical base64url that decodes to a valid-but-wrong length mismatches the 32-byte target:
        // 44 chars ("A" * 44) is canonical and decodes to 33 bytes.
        assert_eq!(
            "A".repeat(44).parse::<Hash>(),
            Err(DecodeError::OutputLenMismatch {
                expected: 33,
                got: 32
            })
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
        assert_eq!(format!("{h:?}"), format!("Hash({h})"));
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
