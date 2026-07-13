//! The binary codec — a plain hand-rolled byte format for [`Arenas`]. No CBOR, no serde.
//!
//! Wire layout (counts / ids / lengths are `VarU64` unsigned LEB128 via [`crate::leb128`]):
//!
//! ```text
//! [ header:8 ]                       container version tag (see the versioning note below)
//! [ leaf_count:var ]
//!   for each leaf, in canonical order:
//!     [ kind:1 ]
//!       0  IntPosDec / … the sign AND radix are folded into the kind tag (see the kind constants):
//!          IntPos{Dec,Hex,Bin} / IntNeg{Dec,Hex,Bin}  [ mag_len:var ][ mag_be:bytes ]
//!       Float                         [ sign:1 ][ exp:i64-be ][ sig_len:var ][ sig_be:bytes ]
//!       Str                           [ len:var ][ utf8:bytes ]
//!       BoolFalse | BoolTrue          (no payload)
//!       Name                          [ len:var ][ utf8:bytes ]
//! [ struct_count:var ]
//!   for each structure entry, in canonical (post-order) order:
//!     [ tag:1 ]
//!       Atom  [ leaf_id:var ]
//!       List  [ child_count:var ][ child_id:var ]*
//! [ root:var ]                        a StructId
//! ```
//!
//! Sign is expressed by TWO int kind tags (positive/negative) rather than a sign byte — a `-0` never
//! arises for `Int` so there is no signed-zero ambiguity, and small ints stay one byte tighter.
//! Radix (dec/hex/bin) is folded into the tag too, so the printed text re-reads to the same leaf.
//!
//! `encode` is a straight walk of the two vectors. `decode` is TOTAL: it verifies the header and
//! refuses (returns `None`) on a wrong header, malformed length/tag, out-of-range id, or trailing
//! bytes — it never panics and never returns a wrong tree. Determinism ("equal programs -> identical
//! bytes") is a property of CANONICAL arenas (see `canon.rs`), not of the codec: the codec faithfully
//! serializes whatever it is handed.
//!
//! VERSIONING: the 8-byte `header` carries the container encoding version, and `decode` refuses any
//! bytes whose header it does not recognize (wrong header -> `None`) rather than misreading them:
//!
//= spec/contracts/ast-encoding.md#the-encoding-is-versioned
//# A reader MUST refuse a binary AST whose container encoding version it does not implement rather than misinterpret it.
//!
//! The current tag is a fixed `cdzast\x00\x01` (a name + a version number). A future refinement could
//! make the version a truncated hash of the AST type schema so a schema change also bumps it, but that
//! is an optional strengthening of the same check — not a gap: the refuse-on-mismatch guarantee holds
//! today, and swapping the tag's content is a drop-in change.

use crate::ast::{Arenas, Decimal, IntValue, Leaf, LeafId, Radix, Struct, StructId};
use crate::leb128::{self, Reader};

// Leaf kind tags. Int folds (sign, radix) into the tag.
const KIND_INT_POS_DEC: u8 = 0;
const KIND_INT_POS_HEX: u8 = 1;
const KIND_INT_POS_BIN: u8 = 2;
const KIND_INT_NEG_DEC: u8 = 3;
const KIND_INT_NEG_HEX: u8 = 4;
const KIND_INT_NEG_BIN: u8 = 5;
const KIND_FLOAT: u8 = 6;
const KIND_STR: u8 = 7;
const KIND_BOOL_FALSE: u8 = 8;
const KIND_BOOL_TRUE: u8 = 9;
const KIND_NAME: u8 = 10;
const KIND_BYTES: u8 = 11;
const KIND_BAD_ESCAPE: u8 = 12;
const KIND_CHAR: u8 = 13;
const KIND_BAD_CHAR: u8 = 14;

const TAG_ATOM: u8 = 0;
const TAG_LIST: u8 = 1;

/// The 8-byte container version tag (a name + a version number). `decode` verifies it and refuses any
/// bytes with an unrecognized header, per ast-encoding.md §The Encoding Is Versioned (see the module
/// header). The content could be strengthened to a schema hash later; swapping it is a drop-in change.
const SCHEMA_HEADER: [u8; 8] = *b"cdzast\x00\x01";

fn int_kind(neg: bool, radix: Radix) -> u8 {
    match (neg, radix) {
        (false, Radix::Dec) => KIND_INT_POS_DEC,
        (false, Radix::Hex) => KIND_INT_POS_HEX,
        (false, Radix::Bin) => KIND_INT_POS_BIN,
        (true, Radix::Dec) => KIND_INT_NEG_DEC,
        (true, Radix::Hex) => KIND_INT_NEG_HEX,
        (true, Radix::Bin) => KIND_INT_NEG_BIN,
    }
}

/// Serialize `arenas` to bytes (with the schema header).
pub fn encode(arenas: &Arenas) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&SCHEMA_HEADER);

    leb128::write_u64(&mut out, arenas.leaves.len() as u64);
    for leaf in &arenas.leaves {
        write_leaf(&mut out, leaf);
    }

    leb128::write_u64(&mut out, arenas.structure.len() as u64);
    for entry in &arenas.structure {
        match entry {
            Struct::Atom(LeafId(id)) => {
                out.push(TAG_ATOM);
                leb128::write_u64(&mut out, *id as u64);
            }
            Struct::List(children) => {
                out.push(TAG_LIST);
                leb128::write_u64(&mut out, children.len() as u64);
                for StructId(id) in children {
                    leb128::write_u64(&mut out, *id as u64);
                }
            }
        }
    }

    leb128::write_u64(&mut out, arenas.root.0 as u64);
    out
}

fn write_leaf(out: &mut Vec<u8>, leaf: &Leaf) {
    match leaf {
        Leaf::Int { value, radix } => {
            // `value` already IS a (sign, big-endian magnitude) pair — no conversion needed. Zero
            // carries an empty magnitude (never the negative kind tag), matching the wire contract.
            let neg = value.negative && !value.magnitude.is_empty();
            out.push(int_kind(neg, *radix));
            leb128::write_u64(out, value.magnitude.len() as u64);
            out.extend_from_slice(&value.magnitude);
        }
        Leaf::Float(d) => {
            out.push(KIND_FLOAT);
            out.push(d.negative as u8);
            leb128::write_i64_be(out, d.exponent);
            // The significand is a non-negative magnitude; its sign lives in `d.negative`.
            leb128::write_u64(out, d.significand.len() as u64);
            out.extend_from_slice(&d.significand);
        }
        Leaf::Str(s) => {
            out.push(KIND_STR);
            write_bytes(out, s.as_bytes());
        }
        // A char leaf — the scalar, UTF-8 encoded (mirrors cadenza-syntax's codec).
        Leaf::Char(c) => {
            out.push(KIND_CHAR);
            let mut buf = [0u8; 4];
            write_bytes(out, c.encode_utf8(&mut buf).as_bytes());
        }
        // A bad-char MARKER — the offending literal text (mirrors cadenza-syntax's codec).
        Leaf::BadChar(s) => {
            out.push(KIND_BAD_CHAR);
            write_bytes(out, s.as_bytes());
        }
        Leaf::Bytes(b) => {
            out.push(KIND_BYTES);
            write_bytes(out, b);
        }
        Leaf::Bool(b) => {
            out.push(if *b { KIND_BOOL_TRUE } else { KIND_BOOL_FALSE });
        }
        Leaf::Name(n) => {
            out.push(KIND_NAME);
            write_bytes(out, n.as_bytes());
        }
        // A bad-escape MARKER — the offending escape char, UTF-8 encoded (mirrors cadenza-syntax's codec).
        Leaf::BadEscape(c) => {
            out.push(KIND_BAD_ESCAPE);
            let mut buf = [0u8; 4];
            write_bytes(out, c.encode_utf8(&mut buf).as_bytes());
        }
    }
}

fn write_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    leb128::write_u64(out, bytes.len() as u64);
    out.extend_from_slice(bytes);
}

/// Decode bytes to `Arenas`, verifying the header and consuming the whole input. Total: returns
/// `None` on header mismatch, malformed structure, out-of-range id, or trailing bytes.
pub fn decode(bytes: &[u8]) -> Option<Arenas> {
    // Header.
    let header = bytes.get(..8)?;
    if header != SCHEMA_HEADER {
        return None;
    }
    let mut r = Reader::new(&bytes[8..]);

    // Leaves.
    let leaf_count = r.read_var_len()?;
    let mut leaves = Vec::with_capacity(leaf_count.min(1 << 16));
    for _ in 0..leaf_count {
        leaves.push(read_leaf(&mut r)?);
    }

    // Structure.
    let struct_count = r.read_var_len()?;
    let mut structure = Vec::with_capacity(struct_count.min(1 << 16));
    for _ in 0..struct_count {
        let tag = r.byte()?;
        let entry = match tag {
            TAG_ATOM => {
                let leaf_id = r.read_varu64()?;
                if leaf_id as usize >= leaves.len() {
                    return None; // referential integrity: leaf id in range
                }
                Struct::Atom(LeafId(u32::try_from(leaf_id).ok()?))
            }
            TAG_LIST => {
                let n = r.read_var_len()?;
                let mut children = Vec::with_capacity(n.min(1 << 16));
                for _ in 0..n {
                    let child = r.read_varu64()?;
                    children.push(StructId(u32::try_from(child).ok()?));
                }
                Struct::List(children)
            }
            _ => return None,
        };
        structure.push(entry);
    }

    // Root.
    let root = r.read_varu64()?;
    if root as usize >= structure.len() {
        return None;
    }
    let root = StructId(u32::try_from(root).ok()?);

    // Referential integrity for structure child ids: every id must be in range. (Atom leaf ids
    // were checked above.) A forward reference is permitted — the codec requires only in-boundsness.
    for entry in &structure {
        if let Struct::List(children) = entry {
            for StructId(id) in children {
                if *id as usize >= structure.len() {
                    return None;
                }
            }
        }
    }

    // No trailing bytes.
    if !r.at_end() {
        return None;
    }
    Some(Arenas {
        leaves,
        structure,
        root,
    })
}

fn read_leaf(r: &mut Reader) -> Option<Leaf> {
    let kind = r.byte()?;
    Some(match kind {
        KIND_INT_POS_DEC | KIND_INT_POS_HEX | KIND_INT_POS_BIN | KIND_INT_NEG_DEC
        | KIND_INT_NEG_HEX | KIND_INT_NEG_BIN => {
            let (neg, radix) = match kind {
                KIND_INT_POS_DEC => (false, Radix::Dec),
                KIND_INT_POS_HEX => (false, Radix::Hex),
                KIND_INT_POS_BIN => (false, Radix::Bin),
                KIND_INT_NEG_DEC => (true, Radix::Dec),
                KIND_INT_NEG_HEX => (true, Radix::Hex),
                _ => (true, Radix::Bin),
            };
            let len = r.read_var_len()?;
            let mag = r.take(len)?;
            // Store the magnitude verbatim so decode is a faithful inverse of encode. The sign is
            // carried by the kind tag; a zero value (empty magnitude) is never the negative tag.
            let value = IntValue {
                negative: neg,
                magnitude: mag.to_vec(),
            };
            Leaf::Int { value, radix }
        }
        KIND_FLOAT => {
            let negative = read_bool(r)?;
            let exponent = r.read_i64_be()?;
            let sig_len = r.read_var_len()?;
            let mag = r.take(sig_len)?;
            Leaf::Float(Decimal {
                negative,
                significand: mag.to_vec(),
                exponent,
            })
        }
        KIND_STR => Leaf::Str(read_string(r)?),
        KIND_BYTES => Leaf::Bytes(read_raw_bytes(r)?),
        KIND_BOOL_FALSE => Leaf::Bool(false),
        KIND_BOOL_TRUE => Leaf::Bool(true),
        KIND_NAME => Leaf::Name(read_string(r)?),
        KIND_BAD_ESCAPE => Leaf::BadEscape(read_string(r)?.chars().next()?),
        KIND_CHAR => Leaf::Char(read_string(r)?.chars().next()?),
        KIND_BAD_CHAR => Leaf::BadChar(read_string(r)?),
        _ => return None,
    })
}

fn read_string(r: &mut Reader) -> Option<String> {
    let len = r.read_var_len()?;
    let bytes = r.take(len)?;
    String::from_utf8(bytes.to_vec()).ok()
}

/// Read a raw byte sequence (a `Bytes` leaf's payload) — a length then that many bytes, verbatim (no
/// UTF-8 check, unlike [`read_string`]: a byte sequence is arbitrary bytes).
fn read_raw_bytes(r: &mut Reader) -> Option<Vec<u8>> {
    let len = r.read_var_len()?;
    Some(r.take(len)?.to_vec())
}

fn read_bool(r: &mut Reader) -> Option<bool> {
    match r.byte()? {
        0 => Some(false),
        1 => Some(true),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Builder;

    /// An `IntValue` from an `i64` (test convenience, mirrors `IntValue::from_i64`).
    fn int(v: i64) -> IntValue {
        IntValue::from_i64(v)
    }

    fn sample() -> Arenas {
        // (+ x x) plus a big int, a hex int, a negative int, an exact decimal, a string, and a bool.
        let mut b = Builder::new();
        let plus = b.name("+");
        let x1 = b.name("x");
        let x2 = b.name("x");
        // 123456789012345678901234567890 as raw big-endian magnitude bytes (wider than i64) — proves
        // the representation carries arbitrary precision with no bignum library behind it.
        let big = b.atom_leaf(Leaf::Int {
            value: IntValue {
                negative: false,
                magnitude: vec![
                    0x01, 0x8E, 0xE9, 0x0F, 0xF6, 0xC3, 0x73, 0xE0, 0xEE, 0x4E, 0x3F, 0x0A, 0xD2,
                ],
            },
            radix: Radix::Dec,
        });
        let hex = b.atom_leaf(Leaf::Int {
            value: int(0x2A),
            radix: Radix::Hex,
        });
        let neg = b.atom_leaf(Leaf::Int {
            value: int(-42),
            radix: Radix::Dec,
        });
        let flt = b.atom_leaf(Leaf::Float(Decimal {
            negative: false,
            significand: vec![15], // 15 * 10^-1 = 1.5
            exponent: -1,
        }));
        let s = b.atom_leaf(Leaf::Str("héllo".to_string()));
        let t = b.atom_leaf(Leaf::Bool(true));
        let root = b.list(vec![plus, x1, x2, big, hex, neg, flt, s, t]);
        b.finish(root)
    }

    #[test]
    fn round_trips() {
        let a = sample();
        let bytes = encode(&a);
        let back = decode(&bytes).expect("decode");
        assert_eq!(a, back);
        // Determinism: re-encoding the decoded arenas reproduces the bytes.
        assert_eq!(bytes, encode(&back));
    }

    #[test]
    fn radix_round_trips() {
        // Same value, different bases -> distinct leaves that survive the round-trip.
        let mut b = Builder::new();
        let dec = b.atom_leaf(Leaf::Int {
            value: int(42),
            radix: Radix::Dec,
        });
        let hex = b.atom_leaf(Leaf::Int {
            value: int(42),
            radix: Radix::Hex,
        });
        let bin = b.atom_leaf(Leaf::Int {
            value: int(42),
            radix: Radix::Bin,
        });
        let root = b.list(vec![dec, hex, bin]);
        let a = b.finish(root);
        assert_eq!(decode(&encode(&a)).unwrap(), a);
        // Three distinct leaves (radix is part of leaf identity).
        assert_eq!(a.leaves.len(), 3);
    }

    #[test]
    fn signed_zero_preserved() {
        let mut b = Builder::new();
        let neg_zero = b.atom_leaf(Leaf::Float(Decimal {
            negative: true,
            significand: Vec::new(), // zero magnitude
            exponent: 0,
        }));
        let a = b.finish(neg_zero);
        let back = decode(&encode(&a)).expect("decode");
        assert_eq!(a, back);
        let Leaf::Float(d) = &back.leaves[0] else {
            panic!()
        };
        assert!(d.negative, "-0.0 must stay negative");
    }

    #[test]
    fn wrong_header_refused() {
        let a = sample();
        let mut bytes = encode(&a);
        bytes[0] ^= 0xff;
        assert_eq!(decode(&bytes), None);
    }

    #[test]
    fn trailing_bytes_refused() {
        let a = sample();
        let mut bytes = encode(&a);
        bytes.push(0);
        assert_eq!(decode(&bytes), None);
    }

    #[test]
    fn truncated_refused() {
        let a = sample();
        let bytes = encode(&a);
        for cut in 8..bytes.len() {
            assert_eq!(decode(&bytes[..cut]), None, "prefix len {cut}");
        }
    }

    #[test]
    fn out_of_range_leaf_id_refused() {
        let mut bytes = SCHEMA_HEADER.to_vec();
        leb128::write_u64(&mut bytes, 0); // leaf_count
        leb128::write_u64(&mut bytes, 1); // struct_count
        bytes.push(TAG_ATOM);
        leb128::write_u64(&mut bytes, 0); // leaf id 0 — out of range
        leb128::write_u64(&mut bytes, 0); // root
        assert_eq!(decode(&bytes), None);
    }
}
