//! Unsigned LEB128 varint (`VarU64`): 7 data bits per byte, high bit = continuation, up to 10 bytes.
//! Used for the small, slowly-growing fields of the wire form (counts, node-id references). Plus fixed
//! big-endian scalar helpers for the exact-width fields. No dependency.
//!
//! Reads are total: a truncated, over-64-bit, or NON-MINIMAL (overlong) varint returns `None` rather
//! than panicking, because decode operates on untrusted external bytes. Rejecting non-minimal
//! encodings (`0x80 0x00`, an overlong `0`) is what makes the varint a BIJECTION — one accepted byte
//! form per value — so the codec built on it inherits its "one canonical byte form" contract
//! (`ast-encoding.md` §The Encoding Is A Bijection With One Canonical Byte Form).

// `alloc` (not std's prelude) so this file compiles under the `#![no_std]` minimal core.
use alloc::vec::Vec;

/// Why a classified varint read failed — the distinction a streaming/log consumer needs to tell a
/// benign TORN write (the input simply ended mid-varint) from genuine CORRUPTION (a fully-present but
/// non-canonical / over-64-bit varint). [`Reader::read_varu64`] collapses both to `None`;
/// [`Reader::read_varu64_checked`] preserves the split.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarErr {
    /// The input ended while more varint bytes were expected (a continuation byte with nothing after,
    /// or no byte at all) — an interrupted/torn write, not corruption.
    Truncated,
    /// All the bytes the varint needs are present, but they do not form a valid canonical `VarU64` —
    /// an over-64-bit value or a non-minimal (overlong) encoding. Genuine corruption.
    Malformed,
}

/// Append the unsigned LEB128 encoding of `value` to `out`.
pub fn write_u64(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            break;
        }
    }
}

/// A cursor reading out of a byte slice, tracking position. Reads never panic.
pub struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Reader<'a> {
        Reader { bytes, pos: 0 }
    }

    pub fn position(&self) -> usize {
        self.pos
    }

    pub fn at_end(&self) -> bool {
        self.pos == self.bytes.len()
    }

    /// Read one raw byte, or `None` at end of input.
    pub fn byte(&mut self) -> Option<u8> {
        let b = *self.bytes.get(self.pos)?;
        self.pos += 1;
        Some(b)
    }

    /// Read `n` raw bytes as a slice, or `None` if fewer than `n` remain.
    pub fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        let slice = self.bytes.get(self.pos..end)?;
        self.pos = end;
        Some(slice)
    }

    /// Read a fixed 8-byte big-endian `u64`.
    pub fn read_u64_be(&mut self) -> Option<u64> {
        let b = self.take(8)?;
        Some(u64::from_be_bytes(b.try_into().ok()?))
    }

    /// Read a fixed 8-byte big-endian `i64`.
    pub fn read_i64_be(&mut self) -> Option<i64> {
        let b = self.take(8)?;
        Some(i64::from_be_bytes(b.try_into().ok()?))
    }

    /// Read a `VarU64` (unsigned LEB128). `None` on truncation, a value wider than 64 bits, or a
    /// NON-MINIMAL (overlong) encoding — a varint whose terminating byte is a zero group after a
    /// continuation byte (e.g. `0x80 0x00`, an overlong `0`; `0xFF 0x00`, an overlong `127`). The
    /// encoder ([`write_u64`]) never emits such a form, so every value has exactly ONE accepted
    /// encoding — the codec's "one canonical byte form" bijection (`ast-encoding.md`) would otherwise
    /// admit many byte strings decoding to the same tree. `0x00` alone is the canonical `0`.
    ///
    /// Delegates to [`Self::read_varu64_checked`] and drops the failure classification, so the two never
    /// diverge on which byte strings they accept.
    pub fn read_varu64(&mut self) -> Option<u64> {
        self.read_varu64_checked().ok()
    }

    /// Like [`Self::read_varu64`], but distinguishes WHY it failed: [`VarErr::Truncated`] (the input
    /// ended mid-varint — a torn/interrupted write) from [`VarErr::Malformed`] (a fully-present but
    /// non-canonical / over-64-bit varint — genuine corruption). A streaming/log consumer needs this
    /// split to tell a benign torn tail from real corruption; the plain [`Self::read_varu64`] collapses
    /// both to `None`. The accept set is IDENTICAL to `read_varu64` (which is `…_checked().ok()`).
    pub fn read_varu64_checked(&mut self) -> Result<u64, VarErr> {
        let mut result: u64 = 0;
        let mut shift = 0u32;
        loop {
            // No byte where the varint needs one = the input ended mid-varint = a torn write.
            let byte = self.byte().ok_or(VarErr::Truncated)?;
            if shift >= 64 {
                // We are about to read an 11th byte (shift is 70 here: bytes 1..=10 advanced it
                // 0→63→70). A u64 LEB128 is at most 10 bytes, so an 11th byte means the varint claims
                // more than 64 bits — fully present (we HAD the byte), so malformed, not truncated.
                // (The 10th byte, at shift 63, is bounds-checked by the `shift == 63 && payload > 1`
                // guard below.)
                return Err(VarErr::Malformed);
            }
            let payload = (byte & 0x7f) as u64;
            if shift == 63 && payload > 1 {
                return Err(VarErr::Malformed);
            }
            result |= payload << shift;
            if byte & 0x80 == 0 {
                // Minimality: a terminating zero group after at least one continuation byte means the
                // value fit in fewer bytes — a non-canonical encoding (all bytes present ⇒ malformed,
                // not truncated). (`shift == 0` is the single-byte case, where `0x00` is canonical `0`.)
                if payload == 0 && shift != 0 {
                    return Err(VarErr::Malformed);
                }
                return Ok(result);
            }
            shift += 7;
        }
    }

    /// Read a `VarU64` and narrow to `usize`. `None` if it exceeds `usize`.
    pub fn read_var_len(&mut self) -> Option<usize> {
        self.read_var_len_checked().ok()
    }

    /// Like [`Self::read_var_len`], but classifies the failure ([`VarErr`]). A value that exceeds
    /// `usize` (only possible on a &lt;64-bit target) is [`VarErr::Malformed`] — a fully-present but
    /// impossibly-large length, not a torn write.
    pub fn read_var_len_checked(&mut self) -> Result<usize, VarErr> {
        usize::try_from(self.read_varu64_checked()?).map_err(|_| VarErr::Malformed)
    }

    /// Read a big-endian `u64` length and narrow to `usize`.
    pub fn read_be_len(&mut self) -> Option<usize> {
        usize::try_from(self.read_u64_be()?).ok()
    }
}

/// Append a fixed 8-byte big-endian `u64`.
pub fn write_u64_be(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_be_bytes());
}

/// Append a fixed 8-byte big-endian `i64`.
pub fn write_i64_be(out: &mut Vec<u8>, value: i64) {
    out.extend_from_slice(&value.to_be_bytes());
}

// `all(test, feature = "std")`: libtest needs std, so this module only ever built under std — gating it
// explicitly stops cdz-runtime's no_std `#[path]` include (mechanism B) from pulling it into that build.
#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn varu64_round_trips() {
        for v in [0u64, 1, 127, 128, 300, 16_384, u64::MAX, u64::MAX - 1] {
            let mut buf = Vec::new();
            write_u64(&mut buf, v);
            let mut r = Reader::new(&buf);
            assert_eq!(r.read_varu64(), Some(v), "value {v}");
            assert!(r.at_end());
        }
    }

    #[test]
    fn varu64_is_compact() {
        // 0..=127 -> 1 byte, 128 -> 2 bytes, u64::MAX -> 10 bytes.
        let one = |v| {
            let mut b = Vec::new();
            write_u64(&mut b, v);
            b.len()
        };
        assert_eq!(one(0), 1);
        assert_eq!(one(127), 1);
        assert_eq!(one(128), 2);
        assert_eq!(one(u64::MAX), 10);
    }

    #[test]
    fn fixed_be_round_trips() {
        for v in [0u64, 1, 42, u64::MAX] {
            let mut buf = Vec::new();
            write_u64_be(&mut buf, v);
            assert_eq!(buf.len(), 8);
            let mut r = Reader::new(&buf);
            assert_eq!(r.read_u64_be(), Some(v));
        }
        for v in [0i64, -1, i64::MIN, i64::MAX] {
            let mut buf = Vec::new();
            write_i64_be(&mut buf, v);
            let mut r = Reader::new(&buf);
            assert_eq!(r.read_i64_be(), Some(v));
        }
    }

    #[test]
    fn truncated_is_none_not_panic() {
        assert_eq!(Reader::new(&[0x80]).read_varu64(), None);
    }

    #[test]
    fn checked_read_classifies_truncated_vs_malformed() {
        // TRUNCATED: the input ends while a continuation byte promised more (or there is no byte at
        // all) — a torn/interrupted write.
        assert_eq!(
            Reader::new(&[]).read_varu64_checked(),
            Err(VarErr::Truncated)
        );
        assert_eq!(
            Reader::new(&[0x80]).read_varu64_checked(),
            Err(VarErr::Truncated),
            "one continuation byte, then EOF"
        );
        assert_eq!(
            Reader::new(&[0x80, 0x80]).read_varu64_checked(),
            Err(VarErr::Truncated),
            "two continuation bytes, then EOF"
        );

        // MALFORMED: all the varint's bytes ARE present, but they don't form a canonical VarU64 —
        // corruption, NOT a torn tail. A non-minimal (overlong) encoding:
        assert_eq!(
            Reader::new(&[0x80, 0x00]).read_varu64_checked(),
            Err(VarErr::Malformed),
            "overlong 0 is fully present but non-canonical"
        );
        assert_eq!(
            Reader::new(&[0xFF, 0x00]).read_varu64_checked(),
            Err(VarErr::Malformed),
            "overlong 127"
        );
        // An over-64-bit value: 10 continuation groups + a terminator (all present) overflows u64.
        let over64 = [
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01,
        ];
        assert_eq!(
            Reader::new(&over64).read_varu64_checked(),
            Err(VarErr::Malformed),
            "an over-64-bit varint is malformed, not truncated"
        );

        // A valid value still reads as `Ok`.
        assert_eq!(Reader::new(&[0x80, 0x01]).read_varu64_checked(), Ok(128));
        assert_eq!(Reader::new(&[0x00]).read_varu64_checked(), Ok(0));
    }

    #[test]
    fn checked_and_plain_read_accept_the_same_byte_strings() {
        // `read_varu64` is defined as `read_varu64_checked().ok()`, so for EVERY input the two must
        // agree on accept/reject (and the value when accepted). Sweep the small byte space + valid
        // encodings to pin that they never diverge — a divergence would mean the Option API and the
        // classified API disagree on what a valid AST byte stream is.
        let mut rng = Rng(0x0dec_0de5_c0de_1eb1);
        for _ in 0..20_000 {
            let len = 1 + (rng.next() % 11) as usize;
            let buf: Vec<u8> = (0..len).map(|_| (rng.next() & 0xff) as u8).collect();
            let plain = Reader::new(&buf).read_varu64();
            let checked = Reader::new(&buf).read_varu64_checked().ok();
            assert_eq!(plain, checked, "accept sets diverge for {buf:?}");
        }
        // And every canonical encoding reads Ok with the same value + consumes the same bytes.
        for v in [0u64, 1, 127, 128, 255, 300, 16_384, u64::MAX] {
            let mut buf = Vec::new();
            write_u64(&mut buf, v);
            let mut rc = Reader::new(&buf);
            assert_eq!(rc.read_varu64_checked(), Ok(v));
            assert!(rc.at_end());
        }
    }

    #[test]
    fn overlong_is_none() {
        let buf = [0x80u8; 11];
        assert_eq!(Reader::new(&buf).read_varu64(), None);
    }

    #[test]
    fn non_minimal_encoding_is_rejected() {
        // A varint whose terminating byte is a zero group AFTER a continuation byte encodes a value
        // that fit in fewer bytes — a non-canonical form the encoder never emits. Reject it, so each
        // value has exactly one accepted encoding (the codec's canonical-byte-form bijection).
        assert_eq!(Reader::new(&[0x80, 0x00]).read_varu64(), None, "overlong 0");
        assert_eq!(
            Reader::new(&[0xFF, 0x00]).read_varu64(),
            None,
            "overlong 127"
        );
        assert_eq!(
            Reader::new(&[0x80, 0x80, 0x00]).read_varu64(),
            None,
            "overlong 0 (3 bytes)"
        );
        // But the single-byte `0x00` IS the canonical zero, and a genuine two-byte value whose high
        // group is nonzero is minimal and accepted.
        assert_eq!(Reader::new(&[0x00]).read_varu64(), Some(0));
        assert_eq!(Reader::new(&[0x80, 0x01]).read_varu64(), Some(128));
        assert_eq!(Reader::new(&[0xFF, 0x01]).read_varu64(), Some(255));
    }

    #[test]
    fn canonical_encoding_is_the_only_accepted_one() {
        // The bijection property: for a sweep of values, the encoder's output is the UNIQUE byte string
        // `read_varu64` accepts — appending a spurious `0x00` (making it non-minimal) is rejected, and
        // the canonical form decodes back to the value at exactly its length.
        for v in [0u64, 1, 127, 128, 255, 300, 16_383, 16_384, u64::MAX] {
            let mut buf = Vec::new();
            write_u64(&mut buf, v);
            let mut r = Reader::new(&buf);
            assert_eq!(r.read_varu64(), Some(v));
            assert!(
                r.at_end(),
                "canonical form consumes exactly its bytes for {v}"
            );

            // A non-minimal variant: clear the terminator's continuation-free status by turning the
            // last byte into a continuation and appending a `0x00`. This must be rejected.
            if v != 0 {
                let mut overlong = buf.clone();
                let last = overlong.len() - 1;
                overlong[last] |= 0x80; // make the former terminator a continuation byte
                overlong.push(0x00); // ... then terminate with a zero group -> non-minimal
                assert_eq!(
                    Reader::new(&overlong).read_varu64(),
                    None,
                    "non-minimal re-encoding of {v} is rejected"
                );
            }
        }
    }

    #[test]
    fn truncated_multibyte_is_none() {
        // A continuation byte with no following byte is truncation, not a value.
        assert_eq!(Reader::new(&[0x80, 0x80]).read_varu64(), None);
        assert_eq!(Reader::new(&[]).read_varu64(), None);
        // A reader that fails a varint mid-stream leaves the rest inspectable (position advanced past
        // the consumed bytes, no panic).
        let mut r = Reader::new(&[0x80]);
        assert_eq!(r.read_varu64(), None);
    }

    /// A tiny deterministic PRNG (SplitMix64) — reproducible byte-soup without a dependency (mirrors the
    /// unit-test PRNGs in `codec.rs`/`lexer.rs`).
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        }
    }

    /// One field of a heterogeneous wire record — the reader offers four write/read pairs, and the real
    /// codec relies on them COMPOSING: a mix written back-to-back must read back in order, every value
    /// recovered, landing exactly `at_end`. Isolated round-trip tests miss a cross-field position bug (a
    /// `take`/`byte` off-by-one corrupts every SUBSEQUENT field, not the one that mis-stepped).
    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Field {
        Var(u64),
        U64Be(u64),
        I64Be(i64),
        Raw(u8, u8), // (byte, count) — a run of `count` copies of `byte`, read via `take`
    }

    #[test]
    fn heterogeneous_field_sequence_round_trips_and_lands_at_end() {
        // Writing a mixed sequence of field kinds and reading them back in the same order recovers every
        // value AND consumes exactly the written bytes (`at_end`). This pins the readers' COMPOSITION —
        // each read must advance `position` by precisely the width the matching writer emitted — so a
        // future off-by-one in `take`/`byte`/`read_*_be` (which would silently desync every following
        // field) is caught, not just an in-isolation round-trip.
        let mut rng = Rng(0xf1e1_d5ec_0dea_5f17);
        for _ in 0..10_000 {
            let count = 1 + (rng.next() % 8) as usize;
            let mut fields = Vec::with_capacity(count);
            let mut buf = Vec::new();
            for _ in 0..count {
                let field = match rng.next() % 4 {
                    0 => Field::Var(rng.next()),
                    1 => Field::U64Be(rng.next()),
                    2 => Field::I64Be(rng.next() as i64),
                    _ => Field::Raw((rng.next() & 0xff) as u8, 1 + (rng.next() % 5) as u8),
                };
                match field {
                    Field::Var(v) => write_u64(&mut buf, v),
                    Field::U64Be(v) => write_u64_be(&mut buf, v),
                    Field::I64Be(v) => write_i64_be(&mut buf, v),
                    Field::Raw(b, n) => buf.extend(vec![b; n as usize]),
                }
                fields.push(field);
            }
            let mut r = Reader::new(&buf);
            for (i, field) in fields.iter().enumerate() {
                match *field {
                    Field::Var(v) => assert_eq!(r.read_varu64(), Some(v), "field {i} varu64"),
                    Field::U64Be(v) => assert_eq!(r.read_u64_be(), Some(v), "field {i} u64_be"),
                    Field::I64Be(v) => assert_eq!(r.read_i64_be(), Some(v), "field {i} i64_be"),
                    Field::Raw(b, n) => {
                        let want = vec![b; n as usize];
                        assert_eq!(r.take(n as usize), Some(want.as_slice()), "field {i} raw");
                    }
                }
            }
            assert!(
                r.at_end(),
                "reader must consume exactly the written bytes; pos {} of {} for {fields:?}",
                r.position(),
                buf.len()
            );
            // One more read past the end is `None`, never a panic or an over-read.
            assert_eq!(r.byte(), None, "byte past end");
            assert_eq!(r.read_varu64(), None, "varu64 past end");
            assert_eq!(r.read_u64_be(), None, "u64_be past end");
            assert!(r.position() <= buf.len(), "no over-read past end");
        }
    }

    #[test]
    fn read_varu64_over_arbitrary_bytes_never_panics_and_accept_implies_canonical() {
        // The bijection is TOTAL over arbitrary bytes: `read_varu64` on ANY byte prefix (a) never PANICS
        // and never over-reads (position stays within the slice), and (b) whenever it ACCEPTS, the value
        // it returns RE-ENCODES to exactly the bytes it consumed — i.e. an accepted input is always the
        // canonical form of its value. That "accept ⇒ canonical" property is precisely what makes the
        // codec's byte form a bijection (`ast-encoding.md`): no two byte strings can decode to the same
        // value, because any non-minimal string is rejected outright. The hand-written tests pin specific
        // overlong/truncated cases; this sweeps the whole 1..=11-byte space with random content — the
        // regression guard for the minimality check (a `shift`/`payload==0` off-by-one would let some
        // non-canonical string through, and this catches it as an accepted-but-non-canonical input).
        let mut rng = Rng(0x1eb1_28fa_ce5e_ed01);
        // 20k iterations over the small 1..=11-byte space amply covers it (matches the crate's other
        // mutation-sweep norm — a fresh Vec + canon re-encode per iter makes a larger count a gate-time
        // cost with no added coverage; PR#474 review nit).
        for _ in 0..20_000 {
            // 1..=11 bytes (11 > the 10-byte max, so over-length inputs are exercised too).
            let len = 1 + (rng.next() % 11) as usize;
            let buf: Vec<u8> = (0..len).map(|_| (rng.next() & 0xff) as u8).collect();
            let mut r = Reader::new(&buf);
            let got = r.read_varu64(); // must not panic
            // Position never runs past the slice, whether it accepted or rejected.
            assert!(
                r.position() <= buf.len(),
                "reader over-read: pos {} > len {} for {buf:?}",
                r.position(),
                buf.len()
            );
            if let Some(v) = got {
                // Accept ⇒ the consumed prefix is EXACTLY the canonical encoding of `v`.
                let consumed = &buf[..r.position()];
                let mut canon = Vec::new();
                write_u64(&mut canon, v);
                assert_eq!(
                    consumed,
                    canon.as_slice(),
                    "accepted a non-canonical encoding of {v}: consumed {consumed:?}, canonical {canon:?}"
                );
                // And that canonical form decodes right back to the same value at exactly its length.
                let mut r2 = Reader::new(&canon);
                assert_eq!(r2.read_varu64(), Some(v));
                assert!(r2.at_end());
            }
        }
    }
}
