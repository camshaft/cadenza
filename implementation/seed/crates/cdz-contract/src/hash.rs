//! Content hashing — the platform's sole identity (`design/cadenza-platform.md` section 8).
//!
//! A [`Hash`] is **self-describing**: a one-byte [`HashTag`] saying what the hash names, followed by the
//! blake3 digest of the content. It names *and* authorizes: the content-addressed store is unpermissioned
//! because possessing a hash is what lets you read its bytes ("the hash is the capability"). Two things
//! reduce to hashing: a contract-id is the hash of a contract declaration (section 1), and a blob is
//! addressed by the hash of its bytes (section 8). So this is the bottom primitive everything else routes
//! and addresses by.
//!
//! The leading tag makes a hash tell you what it is at runtime, the way the typed id newtypes (a
//! contract-id, a reducer-id — `cdz-platform`'s `ids`) do at compile time: given a bare hash off the wire or
//! stored as a graph edge kind, you can still tell a contract-id from a reducer-id from a raw blob. The tag
//! is a self-description, not a
//! cryptographic commitment — the digest is what commits to the content, so re-tagging a hash does not let
//! anyone forge content for it; a use site that cares checks the tag against what it expects.
//!
//! Algorithm: **blake3** — the one content-address digest the fleet unified on (operator 2026-08-08),
//! fast and 32 bytes. A `Hash` is the tag byte plus those 32 digest bytes: a fixed-size `[u8; 33]` (`Copy`,
//! no allocation) — the "`Bytes`, not `Vec<u8>`" convention is for *variable-length* buffers, not a fixed
//! digest.
//!
//! Text form: **base62** — the digits and letters `0-9 A-Z a-z`, no separators — the single textual form of
//! a hash wherever one is rendered (a name, a log line, an error, a wire field); section 8. base62 is the
//! compact encoding whose alphabet is legal on *every* surface a hash text reaches: the runtime
//! content-address rides in a WebAssembly component-import semver build-metadata suffix, whose grammar
//! admits only `[0-9A-Za-z-]`, so base64url's `_` was invalid there — base62 drops `-`/`_` entirely and so
//! is the one encoding used uniformly, with no per-surface exception and never hex. [`Hash`]'s `Display`
//! and `FromStr` are that rendering and its inverse over all 33 bytes, both computed in a fixed 45-byte
//! stack buffer, so neither allocates.

use std::fmt;
use std::str::FromStr;

/// What a [`Hash`] names — the leading byte of every hash, so a hash is self-describing at runtime. Each
/// value has a typed-id counterpart in [`ids`](crate::ids) (the compile-time version of the same
/// distinction); `Blob` is a raw content address (the store), and `SystemProperty` tags the platform's own
/// well-known hashes, such as the structural edge kinds of the reducer graph (spawn, watch-exit).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum HashTag {
    /// A contract-id — the hash of a contract declaration (section 1).
    Contract = 1,
    /// A reducer/session id — the hash of a genesis (section 3).
    Reducer = 2,
    /// A program hash — the content of a program a reducer is spawned from (section 3/8).
    Program = 3,
    /// A host id — the identity of a host a reducer runs on (section 3/11).
    Host = 4,
    /// A raw content address — a blob in the store (section 8).
    Blob = 5,
    /// A platform-internal well-known hash — e.g. a structural edge kind of the reducer graph.
    SystemProperty = 6,
    /// A state root — the content hash of a reducer's durable key-value state at a point (section 7). A
    /// distinct kind from [`Blob`](Self::Blob) (an arbitrary stored payload) and
    /// [`SystemProperty`](Self::SystemProperty) (a well-known structural constant): a state root is a
    /// *dynamic* digest of live state, so it carries its own tag and stays distinguishable at runtime.
    StateRoot = 7,
}

impl HashTag {
    /// The tag from its byte, or `None` if the byte is not a known tag (e.g. a hash minted by a newer
    /// version). Reading is total, so a foreign tag is `None`, never a panic.
    #[must_use]
    pub const fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            1 => Some(Self::Contract),
            2 => Some(Self::Reducer),
            3 => Some(Self::Program),
            4 => Some(Self::Host),
            5 => Some(Self::Blob),
            6 => Some(Self::SystemProperty),
            7 => Some(Self::StateRoot),
            _ => None,
        }
    }
}

/// The self-describing blake3 content hash of some bytes — the platform's sole identity (section 8): a
/// leading [`HashTag`] byte, then the 32-byte digest.
///
/// `Copy` + cheaply comparable: a hash threads through routing, dispatch, and the store constantly, so
/// it must be trivially clonable. Ordering is by raw bytes (tag first, then digest — a total order for use
/// as a map/set key); the base62 alphabet is digit-ordered, so the text form sorts the same way.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Hash([u8; 33]);

impl Hash {
    /// The number of raw bytes in a hash — the tag byte plus blake3's 256-bit digest.
    pub const LEN: usize = 33;

    /// The number of digest bytes (blake3's 256-bit output), after the tag byte.
    pub const DIGEST_LEN: usize = 32;

    /// The number of base62 characters in a hash's text form. 45 base62 digits carry `62^45 > 2^264`, and
    /// 44 carry `62^44 < 2^264`, so 45 is the fixed, minimal width for the 33-byte (264-bit) tagged hash.
    pub const TEXT_LEN: usize = base62::CHARS;

    /// The content hash of `bytes`, tagged as `tag`. Deterministic: the same tag and bytes always hash
    /// equal (that is what makes content addressing work), so this is a pure function of its inputs.
    #[must_use]
    pub fn of(tag: HashTag, bytes: &[u8]) -> Self {
        Self::from_digest(tag, blake3::hash(bytes).as_bytes())
    }

    /// Begin an **incremental** hash tagged as `tag`: feed pieces with [`Hasher::update`], then
    /// [`Hasher::finalize`]. This hashes several fields without allocating and copying them into one
    /// combined buffer first — e.g. deriving an id from a few fixed-size fields plus a variable-length one.
    /// Feeding all the bytes as one `update` gives exactly the same digest as [`Hash::of`] of those bytes.
    #[must_use]
    pub fn hasher(tag: HashTag) -> Hasher {
        Hasher {
            tag,
            inner: blake3::Hasher::new(),
        }
    }

    /// Assemble a hash from a tag and a raw 32-byte digest.
    fn from_digest(tag: HashTag, digest: &[u8; 32]) -> Self {
        let mut bytes = [0u8; Self::LEN];
        bytes[0] = tag as u8;
        bytes[1..].copy_from_slice(digest);
        Self(bytes)
    }

    /// Wrap 33 raw bytes (the tag byte followed by the digest) as a `Hash` (e.g. read back from the store or
    /// the wire).
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 33]) -> Self {
        Self(bytes)
    }

    /// The raw bytes — the tag byte followed by the digest. A hash IS raw bytes (section 8); this is the
    /// on-wire / in-store form.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 33] {
        &self.0
    }

    /// What this hash names, or `None` if its leading byte is not a known [`HashTag`].
    #[must_use]
    pub const fn tag(&self) -> Option<HashTag> {
        HashTag::from_byte(self.0[0])
    }

    /// The 32-byte blake3 digest (the content commitment), without the tag byte. Recompute
    /// `blake3(content)` and compare against this to verify a hash names given content.
    #[must_use]
    pub fn digest(&self) -> &[u8; 32] {
        <&[u8; 32]>::try_from(&self.0[1..]).expect("a 33-byte hash has a 32-byte digest")
    }

    /// The base62 text of this hash as a fixed 45-byte ASCII array — the ONE textual form (section 8),
    /// computed in place with no heap allocation. `Display` writes this straight out; a caller that
    /// genuinely needs an owned `String` builds one from it (an explicit, opt-in allocation) rather than
    /// every render paying for one.
    #[must_use]
    pub fn text(&self) -> [u8; Self::TEXT_LEN] {
        base62::encode(&self.0)
    }
}

/// Read a hash back from raw bytes (section 8) — the on-wire / in-store form a hash arrives as when it
/// crosses a boundary that carries it as a byte slice (a WIT payload, a stored key). Fails if the slice is
/// not exactly [`Hash::LEN`] bytes, so a wrong-length slice names no hash rather than being silently
/// truncated or padded. The tag byte is not validated here (an unknown tag is still a well-formed hash, just
/// one [`tag`](Hash::tag) reports as `None`) — this is the length-checked counterpart of the infallible
/// [`from_bytes`](Hash::from_bytes).
impl TryFrom<&[u8]> for Hash {
    type Error = std::array::TryFromSliceError;

    fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
        Ok(Self::from_bytes(<[u8; Self::LEN]>::try_from(bytes)?))
    }
}

/// An incremental hash builder tagged with a [`HashTag`] — feed bytes with [`update`](Hasher::update), then
/// [`finalize`](Hasher::finalize) to a [`Hash`]. blake3 under the hood; the same digest as [`Hash::of`]
/// over the concatenation of the fed bytes. Feeding fixed-size fields before a variable-length one keeps
/// the concatenation unambiguous (the fixed fields can always be split back off), which is why an id
/// derived from several fields does not need separators.
pub struct Hasher {
    tag: HashTag,
    inner: blake3::Hasher,
}

impl Hasher {
    /// Feed `bytes` into the hash, returning `self` so calls chain.
    pub fn update(&mut self, bytes: &[u8]) -> &mut Self {
        self.inner.update(bytes);
        self
    }

    /// Finish and return the tagged [`Hash`] of everything fed so far.
    #[must_use]
    pub fn finalize(&self) -> Hash {
        Hash::from_digest(self.tag, self.inner.finalize().as_bytes())
    }
}

/// base62 renders every textual hash (section 8) — written from a fixed stack buffer, no allocation.
impl fmt::Display for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = self.text();
        // The base62 alphabet is ASCII, so the buffer is always valid UTF-8.
        f.write_str(std::str::from_utf8(&text).expect("base62 alphabet is ASCII"))
    }
}

/// A hash is opaque bytes; Debug shows the base62 text (not a 33-element byte array) so log/panic
/// output is the same identity a human reads everywhere else.
impl fmt::Debug for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Hash(")?;
        fmt::Display::fmt(self, f)?;
        f.write_str(")")
    }
}

/// Parse a hash from its base62 text form (the inverse of `Display`). Decodes straight into the fixed
/// 33-byte array (tag + digest) — no allocation. The error is a [`base62::DecodeError`]: text of the wrong
/// length, a non-alphabet character, or a value that does not fit 33 bytes all surface there, so "not a
/// valid hash" needs no separate error type on top of the decoder's. (The leading byte need not be a known
/// tag; [`Hash::tag`] reports `None` for an unrecognized one.)
impl FromStr for Hash {
    type Err = base62::DecodeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(base62::decode(s)?))
    }
}

/// base62 — the alphabet `0-9 A-Z a-z`, no padding or separators — the ONE textual byte encoding for the
/// platform (section 8). Specialized to a hash's fixed 33-byte value (tag + digest): [`encode`] takes 33
/// bytes and yields exactly [`CHARS`] characters, [`decode`] does the inverse, both in place with no
/// allocation.
///
/// Unlike a power-of-two encoding, base62 is a change of base of the whole 264-bit number, not an
/// independent regrouping of bits, so it is computed by long division / multiply-accumulate over the hash
/// treated as one big-endian integer. The output is fixed-width (leading zero digits are the `0`
/// character), and decoding is strict — exactly [`CHARS`] characters, alphabet only, and a value that fits
/// 33 bytes — so a byte string has exactly one text form, which content addressing relies on.
///
/// The alphabet is **digit-ordered** (`0..9`, then `A..Z`, then `a..z`), which is also ASCII order, so a
/// fixed-width base62 text sorts lexicographically the same way the raw bytes sort (tag first, then digest).
pub mod base62 {
    use std::fmt;

    /// The number of raw bytes a hash occupies — the tag byte plus blake3's 256-bit digest.
    const BYTES: usize = 33;

    /// The fixed number of base62 characters a 33-byte hash encodes to. `62^45 > 2^264 > 62^44`, so 45
    /// digits are exactly enough and no fewer suffice.
    pub const CHARS: usize = 45;

    /// index (0..62) -> character, digit-ordered so the text sorts like the raw bytes.
    const ALPHABET: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

    /// character -> value (0..62), or 0xFF for "not an alphabet character". Built once at const time so
    /// decode is a table lookup, not a scan of `ALPHABET`.
    const REVERSE: [u8; 256] = {
        let mut t = [0xFFu8; 256];
        let mut i = 0;
        while i < 62 {
            t[ALPHABET[i] as usize] = i as u8;
            i += 1;
        }
        t
    };

    /// Encode a 33-byte hash to its fixed 45-character base62 text. Treats `bytes` as one big-endian
    /// 264-bit integer and repeatedly divides by 62, taking the remainder as the least-significant digit;
    /// after 45 divisions the quotient is zero (because `62^45 > 2^264`), so every digit is captured and
    /// the result is left-padded with the `0` character to the fixed width. No allocation.
    #[must_use]
    pub fn encode(bytes: &[u8; BYTES]) -> [u8; CHARS] {
        let mut n = *bytes; // big-endian working copy, mutated down to zero by the divisions
        let mut out = [0u8; CHARS];
        for slot in out.iter_mut().rev() {
            // one long-division pass: n, rem = divmod(n, 62), big-endian.
            let mut rem = 0u32;
            for b in &mut n {
                let acc = (rem << 8) | u32::from(*b);
                *b = (acc / 62) as u8;
                rem = acc % 62;
            }
            *slot = ALPHABET[rem as usize];
        }
        debug_assert!(
            n.iter().all(|&b| b == 0),
            "45 base62 digits fully consume a 264-bit value"
        );
        out
    }

    /// Decode a base62 hash text back to its 33 raw bytes (the inverse of [`encode`]). Strict: the text must
    /// be exactly [`CHARS`] characters, every one in the alphabet, and the decoded value must fit 33 bytes —
    /// so a given hash has exactly one valid text form.
    ///
    /// # Errors
    /// Returns the first [`DecodeError`]: [`DecodeError::InvalidLength`] if not [`CHARS`] characters,
    /// [`DecodeError::InvalidChar`] for a non-alphabet character, or [`DecodeError::Overflow`] if the 45
    /// characters name a value `>= 2^264` (a canonical base62 string that does not fit a 33-byte hash).
    pub fn decode(s: &str) -> Result<[u8; BYTES], DecodeError> {
        let s = s.as_bytes();
        // The alphabet is ASCII, so a valid text is exactly CHARS bytes; any multibyte character makes the
        // byte length differ from CHARS (caught here) or is rejected as a non-alphabet byte below.
        if s.len() != CHARS {
            return Err(DecodeError::InvalidLength(s.len()));
        }
        let mut n = [0u8; BYTES]; // big-endian accumulator: n = n*62 + digit, per character
        for &c in s {
            let digit = REVERSE[c as usize];
            if digit == 0xFF {
                return Err(DecodeError::InvalidChar(c as char));
            }
            let mut carry = u32::from(digit);
            for b in n.iter_mut().rev() {
                let acc = u32::from(*b) * 62 + carry;
                *b = acc as u8;
                carry = acc >> 8;
            }
            // A carry out of the top byte means the running value exceeded what 33 bytes hold — the text
            // names a number too large for a tagged hash, so it is not a canonical hash text.
            if carry != 0 {
                return Err(DecodeError::Overflow);
            }
        }
        Ok(n)
    }

    /// Why a string is not a canonical base62 hash text.
    #[derive(Clone, Copy, PartialEq, Eq)]
    pub enum DecodeError {
        /// A character outside the base62 alphabet (`0-9 A-Z a-z`) — e.g. `-`, `_`, `+`, `/`, a `=` pad, or
        /// whitespace.
        InvalidChar(char),
        /// The text is not exactly [`CHARS`] characters (a hash text is fixed-width).
        InvalidLength(usize),
        /// The 45 characters are all valid but name a value `>= 2^264`, which does not fit a 33-byte hash —
        /// so it is not a canonical encoding of any hash.
        Overflow,
    }

    impl fmt::Display for DecodeError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::InvalidChar(c) => write!(f, "invalid base62 character {c:?}"),
                Self::InvalidLength(n) => {
                    write!(f, "invalid base62 hash text length {n} (expected {CHARS})")
                }
                Self::Overflow => {
                    write!(
                        f,
                        "base62 hash text names a value that does not fit 33 bytes"
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
    use super::base62::{self, DecodeError};
    use super::{Hash, HashTag};

    /// Test helper: encode a 33-byte hash value to an owned `String` (the tests want to compare text).
    fn enc(bytes: &[u8; 33]) -> String {
        String::from_utf8(base62::encode(bytes).to_vec()).unwrap()
    }

    /// A 33-byte value: `tag` in the leading byte, then `fill` repeated across the digest.
    fn raw(tag: u8, fill: u8) -> [u8; 33] {
        let mut b = [fill; 33];
        b[0] = tag;
        b
    }

    // ── base62: pin the codec against hand-verifiable vectors ───────────────────────────────────
    #[test]
    fn base62_encodes_the_boundary_values() {
        // The all-zero value -> all `0` characters (the zero-digit), fixed width.
        assert_eq!(enc(&[0u8; 33]), "0".repeat(45));
        // The value 1 (big-endian) -> ...0001.
        let mut one = [0u8; 33];
        one[32] = 1;
        assert_eq!(enc(&one), format!("{}1", "0".repeat(44)));
        // 61 -> the last alphabet character 'z' with a full zero pad.
        let mut sixtyone = [0u8; 33];
        sixtyone[32] = 61;
        assert_eq!(enc(&sixtyone), format!("{}z", "0".repeat(44)));
        // 62 -> "10" (one carry) with a zero pad.
        let mut sixtytwo = [0u8; 33];
        sixtytwo[32] = 62;
        assert_eq!(enc(&sixtytwo), format!("{}10", "0".repeat(43)));
    }

    #[test]
    fn base62_uses_only_digits_and_letters_no_separators() {
        // The all-ones value is the largest 264-bit value; its text uses the alphabet only.
        let text = enc(&[0xFFu8; 33]);
        assert_eq!(text.len(), 45);
        assert!(
            text.bytes().all(|b| b.is_ascii_alphanumeric()),
            "base62 is 0-9 A-Z a-z only, got {text}"
        );
        assert!(
            !text.contains('-')
                && !text.contains('_')
                && !text.contains('+')
                && !text.contains('/'),
            "no base64/base64url separators, got {text}"
        );
    }

    #[test]
    fn base62_round_trips_every_value_pattern() {
        // deterministic, byte-varying values (no rng in tests).
        for seed in 0u8..=255 {
            let mut bytes = [0u8; 33];
            for (i, b) in bytes.iter_mut().enumerate() {
                *b = seed.wrapping_mul(31).wrapping_add(i as u8).wrapping_mul(7);
            }
            let text = enc(&bytes);
            assert_eq!(text.len(), 45);
            assert_eq!(
                base62::decode(&text).unwrap(),
                bytes,
                "round-trip failed for seed {seed}"
            );
        }
        // The two extremes explicitly.
        for bytes in [[0u8; 33], [0xFFu8; 33]] {
            assert_eq!(base62::decode(&enc(&bytes)).unwrap(), bytes);
        }
    }

    #[test]
    fn base62_decode_rejects_bad_input() {
        let good = enc(&raw(5, 0x11));
        assert_eq!(good.len(), 45);
        // Wrong length (short, long, empty).
        assert_eq!(base62::decode(""), Err(DecodeError::InvalidLength(0)));
        assert_eq!(
            base62::decode(&good[..44]),
            Err(DecodeError::InvalidLength(44))
        );
        assert_eq!(
            base62::decode(&format!("{good}0")),
            Err(DecodeError::InvalidLength(46))
        );
        // Non-alphabet characters at the fixed width: url-safe/base64 separators, pad, space.
        let with = |ch: char| format!("{}{ch}", &good[..44]);
        assert_eq!(
            base62::decode(&with('-')),
            Err(DecodeError::InvalidChar('-'))
        );
        assert_eq!(
            base62::decode(&with('_')),
            Err(DecodeError::InvalidChar('_'))
        );
        assert_eq!(
            base62::decode(&with('+')),
            Err(DecodeError::InvalidChar('+'))
        );
        assert_eq!(
            base62::decode(&with('/')),
            Err(DecodeError::InvalidChar('/'))
        );
        assert_eq!(
            base62::decode(&with('=')),
            Err(DecodeError::InvalidChar('='))
        );
        assert_eq!(
            base62::decode(&with(' ')),
            Err(DecodeError::InvalidChar(' '))
        );
    }

    #[test]
    fn base62_decode_rejects_values_over_264_bits() {
        // "zz..z" (45 'z's) is the largest 45-char base62 string and is far above 2^264, so it is rejected
        // as a non-canonical hash text rather than truncating.
        assert_eq!(base62::decode(&"z".repeat(45)), Err(DecodeError::Overflow));
        // The all-ones 33-byte value (the largest that fits) encodes and decodes fine.
        let max = enc(&[0xFFu8; 33]);
        assert!(base62::decode(&max).is_ok());
    }

    // ── Hash ────────────────────────────────────────────────────────────────────────────────────
    #[test]
    fn hash_of_matches_blake3_and_is_deterministic() {
        // Golden: the documented blake3 digest of the empty input — pins the ALGORITHM (a swap to
        // sha256 or a different digest would flip this). The digest is the content commitment, after the
        // tag byte.
        let empty = Hash::of(HashTag::Blob, b"");
        let blake3_empty_hex = "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262";
        assert_eq!(hex(empty.digest()), blake3_empty_hex);
        // Deterministic + distinguishing.
        assert_eq!(
            Hash::of(HashTag::Blob, b"cadenza"),
            Hash::of(HashTag::Blob, b"cadenza")
        );
        assert_ne!(
            Hash::of(HashTag::Blob, b"cadenza"),
            Hash::of(HashTag::Blob, b"cadenzb")
        );
        assert_eq!(Hash::LEN, 33);
        assert_eq!(Hash::DIGEST_LEN, 32);
    }

    #[test]
    fn a_hash_is_self_describing_and_the_tag_is_readable() {
        // The tag rides in the leading byte and reads back; the digest is unaffected by it.
        let contract = Hash::of(HashTag::Contract, b"same content");
        let reducer = Hash::of(HashTag::Reducer, b"same content");
        assert_eq!(contract.tag(), Some(HashTag::Contract));
        assert_eq!(reducer.tag(), Some(HashTag::Reducer));
        // Same content, different tag: distinct hashes, but the same digest (the tag is not hashed in).
        assert_ne!(contract, reducer);
        assert_eq!(contract.digest(), reducer.digest());
        // A byte that is not a known tag reads back as None (a hash from a newer version stays parseable).
        let mut raw = *contract.as_bytes();
        raw[0] = 0xFF;
        assert_eq!(Hash::from_bytes(raw).tag(), None);
        assert_eq!(HashTag::from_byte(2), Some(HashTag::Reducer));
        assert_eq!(HashTag::from_byte(7), Some(HashTag::StateRoot));
        assert_eq!(HashTag::from_byte(0), None);
        // A byte past the last known tag is still an unknown (forward-compatible), not a panic.
        assert_eq!(HashTag::from_byte(8), None);
    }

    #[test]
    fn hash_text_is_base62_and_round_trips() {
        let h = Hash::of(HashTag::Contract, b"the hash is the capability");
        let text = h.to_string();
        assert_eq!(text.as_bytes(), &h.text());
        // 33 bytes -> 45 fixed base62 chars, alphabet only.
        assert_eq!(text.len(), Hash::TEXT_LEN);
        assert_eq!(text.len(), 45);
        assert!(text.bytes().all(|b| b.is_ascii_alphanumeric()));
        // Display -> FromStr is the identity, tag included.
        let parsed = text.parse::<Hash>().unwrap();
        assert_eq!(parsed, h);
        assert_eq!(parsed.tag(), Some(HashTag::Contract));
    }

    #[test]
    fn hash_text_order_matches_byte_order() {
        // The digit-ordered alphabet means the fixed-width text sorts the same as the raw bytes (tag first).
        let lo = Hash::from_bytes(raw(1, 0)); // tag byte 1
        let hi = Hash::from_bytes(raw(2, 0)); // tag byte 2
        assert!(lo < hi);
        assert!(lo.to_string() < hi.to_string());
    }

    #[test]
    fn hash_from_str_rejects_wrong_length_and_bad_text() {
        // Too short.
        assert_eq!("abc".parse::<Hash>(), Err(DecodeError::InvalidLength(3)));
        // A 45-char text with a bad char reaches char-validation -> InvalidChar. Corrupt one char.
        let good = Hash::of(HashTag::Blob, b"corrupt me").to_string();
        assert_eq!(good.len(), 45);
        let bad = format!("{}!", &good[..44]); // still 45 chars, but char 45 is invalid
        assert_eq!(bad.parse::<Hash>(), Err(DecodeError::InvalidChar('!')));
        // A truncated hash text is the wrong length.
        let h = Hash::of(HashTag::Blob, b"x").to_string();
        assert_eq!(
            h[..h.len() - 1].parse::<Hash>(),
            Err(DecodeError::InvalidLength(44))
        );
        // The largest 45-char string names a value too large for 33 bytes.
        assert_eq!("z".repeat(45).parse::<Hash>(), Err(DecodeError::Overflow));
    }

    #[test]
    fn hash_from_bytes_and_as_bytes_are_inverse() {
        let bytes = *Hash::of(HashTag::Blob, b"roundtrip the raw digest").as_bytes();
        assert_eq!(*Hash::from_bytes(bytes).as_bytes(), bytes);
    }

    #[test]
    fn try_from_a_slice_round_trips_and_rejects_a_wrong_length() {
        let h = Hash::of(HashTag::Blob, b"round-trip through a slice");
        // A slice of exactly `Hash::LEN` bytes reconstructs the hash it came from.
        assert_eq!(Hash::try_from(h.as_bytes().as_slice()).unwrap(), h);
        // Anything shorter or longer names no hash.
        assert!(Hash::try_from(b"too short".as_slice()).is_err());
        assert!(Hash::try_from([0u8; Hash::LEN + 1].as_slice()).is_err());
    }

    #[test]
    fn hash_debug_shows_base62() {
        let h = Hash::of(HashTag::Blob, b"debug");
        assert_eq!(format!("{h:?}"), format!("Hash({h})"));
    }

    #[test]
    fn incremental_hasher_matches_one_shot_and_chains() {
        // Feeding pieces incrementally equals hashing their concatenation in one shot — so an id built
        // field-by-field needs no combined buffer. Same tag on both sides.
        let mut h = Hash::hasher(HashTag::Reducer);
        let built = h
            .update(b"the hash ")
            .update(b"is the ")
            .update(b"capability")
            .finalize();
        assert_eq!(
            built,
            Hash::of(HashTag::Reducer, b"the hash is the capability")
        );
        assert_eq!(built.tag(), Some(HashTag::Reducer));
        // A single update equals `Hash::of`.
        assert_eq!(
            Hash::hasher(HashTag::Reducer).update(b"x").finalize(),
            Hash::of(HashTag::Reducer, b"x")
        );
        // The tag is part of the identity: the same bytes under a different tag differ.
        assert_ne!(
            Hash::hasher(HashTag::Reducer).update(b"x").finalize(),
            Hash::hasher(HashTag::Contract).update(b"x").finalize()
        );
        // Order matters (it is a concatenation), so different field boundaries with the same total bytes
        // are only equal when the bytes are identical — distinct content stays distinct.
        assert_ne!(
            Hash::hasher(HashTag::Reducer).update(b"ab").finalize(),
            Hash::hasher(HashTag::Reducer).update(b"ba").finalize()
        );
    }

    /// lowercase hex of raw bytes — TEST-ONLY, to pin the digest against the well-known hex vector.
    /// (Production text is base62 per section 8; hex lives only here.)
    fn hex(bytes: &[u8]) -> String {
        use std::fmt::Write;
        bytes.iter().fold(String::new(), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
    }
}
