//! The binary codec — a plain hand-rolled byte format for [`Arenas`]. No CBOR, no serde.
//!
//! Wire layout (all multi-byte counts/ids/lengths are LEB128 via [`crate::leb128`]):
//!
//! ```text
//! [ header:8 ]                       first 8 bytes of SHA-256 over the arena TYPE TERM (MSB-first)
//! [ leaf_count:u ]
//!   for each leaf, in canonical order:
//!     [ kind:1 ]
//!       0 Int    [ sign:1 ][ mag_len:u ][ mag_be:bytes ]        big-endian magnitude, arbitrary precision
//!       1 Float  [ sign:1 ][ exp:i ][ sig_len:u ][ sig_be:bytes ]   exact decimal; sign carries -0.0
//!       2 Str    [ len:u ][ utf8:bytes ]                        NFC
//!       3 Bool   [ 0 | 1 ]
//!       4 Name   [ len:u ][ utf8:bytes ]
//! [ struct_count:u ]
//!   for each structure entry, in canonical (post-order) order:
//!     [ tag:1 ]
//!       0 Atom   [ leaf_id:u ]
//!       1 List   [ child_count:u ][ child_id:u ]*
//! [ root:u ]                         a StructId
//! ```
//!
//! `encode` is a straight walk of the two vectors. `decode` is TOTAL: it verifies the header and
//! refuses (returns `None`) on a wrong header, a malformed length/tag, an out-of-range id, or
//! trailing bytes — it never panics and never returns a wrong tree. Determinism ("equal programs
//! -> identical bytes") is a property of CANONICAL arenas (see `canon.rs`), not of the codec: the
//! codec faithfully serializes whatever it is handed.

use crate::ast::{Arenas, Decimal, Leaf, LeafId, Struct, StructId};
use crate::leb128::{self, Reader};
use num_bigint::{BigInt, Sign};
use sha2::{Digest, Sha256};

const KIND_INT: u8 = 0;
const KIND_FLOAT: u8 = 1;
const KIND_STR: u8 = 2;
const KIND_BOOL: u8 = 3;
const KIND_NAME: u8 = 4;

const TAG_ATOM: u8 = 0;
const TAG_LIST: u8 = 1;

/// The 8-byte schema-hash header: the first 8 bytes of SHA-256 over a canonical byte description of
/// the arena's TYPE (its variant tags and payload shapes), MSB-first. Evolving the AST type yields
/// a different header, so an old reader refuses newer bytes rather than misreading them. Fixed for
/// a given codec version — computed from a constant string, not from any program's data.
fn schema_header() -> [u8; 8] {
    // A stable textual description of the frozen type. Any change to the wire shape must change
    // this string so the header changes with it.
    const TYPE_TERM: &[u8] = b"cadenza-syntax/arenas/v1\n\
        leaf = Int(bigint) | Float(sign,exp:i,sig:bigint) | Str(utf8) | Bool(u8) | Name(utf8)\n\
        struct = Atom(leafid) | List(structid*)\n\
        file = [header:8, leaves*, structure*, root:structid]\n";
    let digest = Sha256::digest(TYPE_TERM);
    let mut header = [0u8; 8];
    header.copy_from_slice(&digest[..8]);
    header
}

/// Serialize `arenas` to bytes (with the schema-hash header).
pub fn encode(arenas: &Arenas) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&schema_header());

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
        Leaf::Int(n) => {
            out.push(KIND_INT);
            write_bigint(out, n);
        }
        Leaf::Float(d) => {
            out.push(KIND_FLOAT);
            out.push(d.negative as u8);
            leb128::write_i64(out, d.exponent);
            // The significand is a non-negative magnitude; its sign lives in `d.negative`.
            let (_sign, mag) = d.significand.to_bytes_be();
            leb128::write_u64(out, mag.len() as u64);
            out.extend_from_slice(&mag);
        }
        Leaf::Str(s) => {
            out.push(KIND_STR);
            write_bytes(out, s.as_bytes());
        }
        Leaf::Bool(b) => {
            out.push(KIND_BOOL);
            out.push(*b as u8);
        }
        Leaf::Name(n) => {
            out.push(KIND_NAME);
            write_bytes(out, n.as_bytes());
        }
    }
}

/// Write a signed big integer as `[sign:1][mag_len:u][mag_be:bytes]`, sign 0=+/zero, 1=-.
fn write_bigint(out: &mut Vec<u8>, n: &BigInt) {
    let (sign, mag) = n.to_bytes_be();
    out.push(matches!(sign, Sign::Minus) as u8);
    leb128::write_u64(out, mag.len() as u64);
    out.extend_from_slice(&mag);
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
    if header != schema_header() {
        return None;
    }
    let mut r = Reader::new(&bytes[8..]);

    // Leaves.
    let leaf_count = r.read_len()?;
    let mut leaves = Vec::with_capacity(leaf_count.min(1 << 16));
    for _ in 0..leaf_count {
        leaves.push(read_leaf(&mut r)?);
    }

    // Structure.
    let struct_count = r.read_len()?;
    let mut structure = Vec::with_capacity(struct_count.min(1 << 16));
    for _ in 0..struct_count {
        let tag = r.byte()?;
        let entry = match tag {
            TAG_ATOM => {
                let leaf_id = r.read_u64()?;
                // Referential integrity: the leaf id must be in range.
                if leaf_id as usize >= leaves.len() {
                    return None;
                }
                Struct::Atom(LeafId(u32::try_from(leaf_id).ok()?))
            }
            TAG_LIST => {
                let n = r.read_len()?;
                let mut children = Vec::with_capacity(n.min(1 << 16));
                for _ in 0..n {
                    let child = r.read_u64()?;
                    children.push(StructId(u32::try_from(child).ok()?));
                }
                Struct::List(children)
            }
            _ => return None,
        };
        structure.push(entry);
    }

    // Root.
    let root = r.read_u64()?;
    if root as usize >= structure.len() {
        return None;
    }
    let root = StructId(u32::try_from(root).ok()?);

    // Referential integrity for structure child ids: every id must be in range. (Atom leaf ids
    // were checked above.) A forward reference is permitted — canonical order is post-order, but
    // the codec does not require it, only in-boundsness.
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
    Some(Arenas { leaves, structure, root })
}

fn read_leaf(r: &mut Reader) -> Option<Leaf> {
    let kind = r.byte()?;
    Some(match kind {
        KIND_INT => Leaf::Int(read_bigint(r)?),
        KIND_FLOAT => {
            let negative = read_bool(r)?;
            let exponent = r.read_i64()?;
            let sig_len = r.read_len()?;
            let mag = r.take(sig_len)?;
            let significand = BigInt::from_bytes_be(Sign::Plus, mag);
            Leaf::Float(Decimal { negative, significand, exponent })
        }
        KIND_STR => Leaf::Str(read_string(r)?),
        KIND_BOOL => Leaf::Bool(read_bool(r)?),
        KIND_NAME => Leaf::Name(read_string(r)?),
        _ => return None,
    })
}

fn read_bigint(r: &mut Reader) -> Option<BigInt> {
    let neg = read_bool(r)?;
    let len = r.read_len()?;
    let mag = r.take(len)?;
    let sign = if neg { Sign::Minus } else { Sign::Plus };
    // BigInt::from_bytes_be with Sign::Minus and empty/zero magnitude yields zero (sign ignored),
    // which is correct: a zero has no sign here.
    Some(BigInt::from_bytes_be(sign, mag))
}

fn read_string(r: &mut Reader) -> Option<String> {
    let len = r.read_len()?;
    let bytes = r.take(len)?;
    String::from_utf8(bytes.to_vec()).ok()
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
    use num_bigint::BigInt;
    use std::str::FromStr;

    fn sample() -> Arenas {
        // (+ x x) with a big int, an exact decimal, a string, and a bool thrown in as a list.
        let mut b = Builder::new();
        let plus = b.name("+");
        let x1 = b.name("x");
        let x2 = b.name("x");
        let big = b.atom_leaf(Leaf::Int(BigInt::from_str("123456789012345678901234567890").unwrap()));
        let neg = b.atom_leaf(Leaf::Int(BigInt::from_str("-42").unwrap()));
        let flt = b.atom_leaf(Leaf::Float(Decimal {
            negative: false,
            significand: BigInt::from_str("15").unwrap(),
            exponent: -1,
        }));
        let s = b.atom_leaf(Leaf::Str("héllo".to_string()));
        let t = b.atom_leaf(Leaf::Bool(true));
        let root = b.list(vec![plus, x1, x2, big, neg, flt, s, t]);
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
    fn signed_zero_preserved() {
        let mut b = Builder::new();
        let neg_zero = b.atom_leaf(Leaf::Float(Decimal {
            negative: true,
            significand: BigInt::from(0u32),
            exponent: 0,
        }));
        let a = b.finish(neg_zero);
        let back = decode(&encode(&a)).expect("decode");
        assert_eq!(a, back);
        let Leaf::Float(d) = &back.leaves[0] else { panic!() };
        assert!(d.negative, "-0.0 must stay negative");
    }

    #[test]
    fn wrong_header_refused() {
        let a = sample();
        let mut bytes = encode(&a);
        bytes[0] ^= 0xff; // corrupt the header
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
            // Every prefix past the header must be refused, never panic.
            assert_eq!(decode(&bytes[..cut]), None, "prefix len {cut}");
        }
    }

    #[test]
    fn out_of_range_leaf_id_refused() {
        // Hand-craft: header + 0 leaves + one Atom referencing leaf 0 (which does not exist).
        let mut bytes = schema_header().to_vec();
        leb128::write_u64(&mut bytes, 0); // leaf_count
        leb128::write_u64(&mut bytes, 1); // struct_count
        bytes.push(TAG_ATOM);
        leb128::write_u64(&mut bytes, 0); // leaf id 0 — out of range
        leb128::write_u64(&mut bytes, 0); // root
        assert_eq!(decode(&bytes), None);
    }

    #[test]
    fn header_is_stable() {
        // The header must not depend on program data.
        assert_eq!(schema_header(), schema_header());
        assert_eq!(&encode(&sample())[..8], &schema_header());
    }
}
