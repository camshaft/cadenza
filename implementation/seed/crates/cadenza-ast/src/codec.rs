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
//!       Str | Name | Sym | Bytes      [ len:var ][ bytes ]   (Str/Name/Sym are UTF-8; Bytes is raw)
//!       Char | BadChar | BadEscape    [ len:var ][ utf8:bytes ]  (one scalar; BadChar/BadEscape are markers)
//!       BoolFalse | BoolTrue          (no payload)
//! [ struct_count:var ]
//!   for each structure entry, in canonical (post-order) order:
//!     [ tag:1 ]
//!       Atom  [ leaf_id:var ]
//!       List  [ child_count:var ][ child_id:var ]*
//! [ root:var ]                        a StructId
//! ```
//!
//! The structure is a tree of NODES — each an `Atom` (a leaf) or a `List` (an ordered sequence of
//! child node ids) — so the container form does not enumerate the language's node kinds; a new kind is
//! a new leaf/head, not a new wire shape:
//!
//= spec/contracts/ast-encoding.md#the-encoding-is-general-and-stable
//# The binary encoding MUST represent an abstract syntax tree as a tree of nodes, each a symbol applied to an ordered sequence of child nodes, so that the container form is independent of which node kinds the language currently defines.
//!
//= spec/contracts/ast-encoding.md#the-encoding-is-general-and-stable
//# The addition of a new node kind MUST be expressible as a new symbol without changing the binary encoding of a tree that does not reference it.
//!
//! Sign is expressed by TWO int kind tags (positive/negative) rather than a sign byte — a `-0` never
//! arises for `Int` so there is no signed-zero ambiguity, and small ints stay one byte tighter.
//! Radix (dec/hex/bin) is folded into the tag too, so the printed text re-reads to the same leaf.
//!
//! `encode` is a straight walk of the two vectors of a CANONICAL arena, so equal trees produce identical
//! bytes and `decode` reconstructs exactly the tree encoded — the encoding is a bijection with one
//! canonical byte form:
//!
//= spec/contracts/ast-encoding.md#the-encoding-is-a-bijection-with-one-canonical-byte-form
//# Each abstract syntax tree MUST have exactly one canonical binary encoding.
//!
//= spec/contracts/ast-encoding.md#the-encoding-is-a-bijection-with-one-canonical-byte-form
//# Two abstract syntax trees that are equal MUST have identical binary encodings.
//!
//= spec/contracts/ast-encoding.md#the-encoding-is-a-bijection-with-one-canonical-byte-form
//# Decoding a canonical binary encoding MUST yield the abstract syntax tree it was encoded from.
//!
//! This binary serialization of the AST IS the program's canonical form — one canonical byte form
//! independent of any textual rendering:
//!
//= constitution.md#x-programs-are-readable-by-agents-and-humans
//# The canonical form of a program MUST be a stable binary serialization of its abstract syntax tree, such that a program has one canonical byte form independent of any textual rendering.
//!
//! `decode` is TOTAL: it verifies the header and refuses (returns `None`) on a wrong header, malformed
//! length/tag, out-of-range id, a non-tree structure (a cycle or shared subtree among the reachable
//! nodes), or trailing bytes — it never panics and never returns a wrong tree. The tree check matters
//! because downstream consumers (e.g. `canon::canonicalize`) walk the structure recursively: a cyclic
//! arena would diverge and a shared subtree would expand exponentially, so a hostile byte string could
//! otherwise turn into a stack overflow or a decode-bomb. A canonical encoding is always a tree, so the
//! check refuses nothing a valid encoder produced.
//! Determinism ("equal programs -> identical bytes") is a property of CANONICAL arenas (see `canon.rs`),
//! which `encode` imposes before serializing.
//!
//! VERSIONING: the 8-byte `header` carries the container encoding version, and `decode` refuses any
//! bytes whose header it does not recognize (wrong header -> `None`) rather than misreading them:
//!
//= spec/contracts/ast-encoding.md#the-encoding-is-versioned
//# The binary encoding MUST carry the version of the container encoding it conforms to.
//!
//= spec/contracts/ast-encoding.md#the-encoding-is-versioned
//# A reader MUST refuse a binary AST whose container encoding version it does not implement rather than misinterpret it.
//!
//! The current tag is a fixed `cdzast\x00\x01` (a name + a version number). A future refinement could
//! make the version a truncated hash of the AST type schema so a schema change also bumps it, but that
//! is an optional strengthening of the same check — the refuse-on-mismatch guarantee holds today, and
//! swapping the tag's content is a drop-in change.

use crate::ast::{Arenas, Decimal, Leaf, LeafId, Radix, Struct, StructId, SuffixBody, SuffixKind};
use crate::leb128::{self, Reader};
use num_bigint::{BigInt, Sign};

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
const KIND_SYM: u8 = 15;
// A TYPE-SUFFIXED numeric literal (`100N`/`0.5R`). Payload: one suffix byte (`SUFFIX_*`), one
// body-shape byte (`BODY_*`), then the body encoded as a bare int/float would be.
const KIND_SUFFIXED: u8 = 16;
const SUFFIX_BIGINT: u8 = 0;
const SUFFIX_RATIONAL: u8 = 1;
const BODY_INT: u8 = 0;
const BODY_FLOAT: u8 = 1;

const TAG_ATOM: u8 = 0;
const TAG_LIST: u8 = 1;

/// The 8-byte container version tag (a name + a version number). `decode` verifies it and refuses any
/// bytes with an unrecognized header, per ast-encoding.md §The Encoding Is Versioned (see the module
/// header). The content could be strengthened to a schema hash later; swapping it is a drop-in change.
const SCHEMA_HEADER: [u8; 8] = *b"cdzast\x00\x01";

fn int_kind(sign: Sign, radix: Radix) -> u8 {
    let neg = matches!(sign, Sign::Minus);
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
///
/// The arena is CANONICALIZED first (`canon::canonicalize`), so equal programs encode to identical
/// bytes regardless of the order their occurrences were built — the two surfaces build the same tree
/// in different orders (see `canon.rs`). Encoding is thus the point at which the canonical normal
/// form is imposed; `decode` returns that canonical (structurally-equal, re-indexed) arena.
pub fn encode(arenas: &Arenas) -> Vec<u8> {
    // Canonicalize to normal form so equal programs encode to identical bytes. `canonicalize` returns
    // a `Cow` — borrowed (no clone/rebuild) when `arenas` is already canonical, which a fresh parse is.
    let canon = crate::canon::canonicalize(arenas);
    let arenas = &*canon;
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
            let (sign, mag) = value.to_bytes_be();
            out.push(int_kind(sign, *radix));
            leb128::write_u64(out, mag.len() as u64);
            out.extend_from_slice(&mag);
        }
        Leaf::Float(d) => {
            out.push(KIND_FLOAT);
            out.push(d.negative as u8);
            leb128::write_i64_be(out, d.exponent);
            // The significand is a non-negative magnitude; its sign lives in `d.negative`.
            let (_sign, mag) = d.significand.to_bytes_be();
            leb128::write_u64(out, mag.len() as u64);
            out.extend_from_slice(&mag);
        }
        Leaf::Str(s) => {
            out.push(KIND_STR);
            write_bytes(out, s.as_bytes());
        }
        // A char leaf — the scalar, UTF-8 encoded (a length then that many bytes, like a string body).
        Leaf::Char(c) => {
            out.push(KIND_CHAR);
            let mut buf = [0u8; 4];
            write_bytes(out, c.encode_utf8(&mut buf).as_bytes());
        }
        // A bad-char MARKER — the offending literal text (UTF-8, like a name/string body).
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
        // A symbol leaf — the interned name text (mirrors rcdzc's codec `KIND_SYM`).
        Leaf::Sym(s) => {
            out.push(KIND_SYM);
            write_bytes(out, s.as_bytes());
        }
        // A bad-escape MARKER — the offending escape char, UTF-8 encoded (like a name/string body).
        Leaf::BadEscape(c) => {
            out.push(KIND_BAD_ESCAPE);
            let mut buf = [0u8; 4];
            write_bytes(out, c.encode_utf8(&mut buf).as_bytes());
        }
        // A TYPE-SUFFIXED numeric literal: a suffix byte, a body-shape byte, then the body encoded
        // exactly as a bare `Int`/`Float` leaf would be (so `read_leaf` reuses the same body decode).
        Leaf::Suffixed { value, kind } => {
            out.push(KIND_SUFFIXED);
            out.push(match kind {
                SuffixKind::BigInt => SUFFIX_BIGINT,
                SuffixKind::Rational => SUFFIX_RATIONAL,
            });
            match value {
                SuffixBody::Int { value, radix } => {
                    out.push(BODY_INT);
                    let (sign, mag) = value.to_bytes_be();
                    out.push(int_kind(sign, *radix));
                    leb128::write_u64(out, mag.len() as u64);
                    out.extend_from_slice(&mag);
                }
                SuffixBody::Float(d) => {
                    out.push(BODY_FLOAT);
                    out.push(d.negative as u8);
                    leb128::write_i64_be(out, d.exponent);
                    let (_sign, mag) = d.significand.to_bytes_be();
                    leb128::write_u64(out, mag.len() as u64);
                    out.extend_from_slice(&mag);
                }
            }
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

    // The reachable structure from `root` must be a genuine TREE — every reachable node reached
    // exactly once. A canonical encoding is always a tree (`encode` re-emits every occurrence as a
    // fresh node via `canon`, so it never shares a subtree), hence this rejects nothing a valid
    // encoder produced. It DOES refuse a corrupted or hostile arena whose child ids form a CYCLE
    // (which would make a recursive consumer such as `canon::canonicalize` diverge and overflow the
    // stack) or SHARE a subtree (which such a consumer expands, up to exponentially — a decode-bomb).
    // Iterative walk, so the check itself cannot overflow on deep input. Unreachable ("dead") nodes
    // remain permitted — `canon` drops them — so this checks only reachability, not full coverage.
    {
        let mut visited = vec![false; structure.len()];
        let mut stack = vec![root.0 as usize];
        while let Some(id) = stack.pop() {
            if visited[id] {
                return None; // a node reached twice: a cycle or a shared subtree — not a tree
            }
            visited[id] = true;
            if let Struct::List(children) = &structure[id] {
                for StructId(child) in children {
                    stack.push(*child as usize);
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
            let sign = if neg { Sign::Minus } else { Sign::Plus };
            let value = BigInt::from_bytes_be(sign, mag);
            Leaf::Int { value, radix }
        }
        KIND_FLOAT => {
            let negative = read_bool(r)?;
            let exponent = r.read_i64_be()?;
            let sig_len = r.read_var_len()?;
            let mag = r.take(sig_len)?;
            let significand = BigInt::from_bytes_be(Sign::Plus, mag);
            Leaf::Float(Decimal {
                negative,
                significand,
                exponent,
            })
        }
        KIND_STR => Leaf::Str(read_string(r)?),
        KIND_BYTES => Leaf::Bytes(read_raw_bytes(r)?),
        KIND_BOOL_FALSE => Leaf::Bool(false),
        KIND_BOOL_TRUE => Leaf::Bool(true),
        KIND_NAME => Leaf::Name(read_string(r)?),
        KIND_SYM => Leaf::Sym(read_string(r)?),
        KIND_BAD_ESCAPE => Leaf::BadEscape(read_string(r)?.chars().next()?),
        KIND_CHAR => Leaf::Char(read_string(r)?.chars().next()?),
        KIND_BAD_CHAR => Leaf::BadChar(read_string(r)?),
        // A TYPE-SUFFIXED numeric literal: the suffix byte, a body-shape byte, then the body encoded
        // as a bare int/float (the same layout `write_leaf` emits and the int/float arms above read).
        KIND_SUFFIXED => {
            let kind = match r.byte()? {
                SUFFIX_BIGINT => SuffixKind::BigInt,
                SUFFIX_RATIONAL => SuffixKind::Rational,
                _ => return None,
            };
            let value = match r.byte()? {
                BODY_INT => {
                    let (neg, radix) = int_kind_parts(r.byte()?)?;
                    let len = r.read_var_len()?;
                    let mag = r.take(len)?;
                    let sign = if neg { Sign::Minus } else { Sign::Plus };
                    SuffixBody::Int {
                        value: BigInt::from_bytes_be(sign, mag),
                        radix,
                    }
                }
                BODY_FLOAT => {
                    let negative = read_bool(r)?;
                    let exponent = r.read_i64_be()?;
                    let sig_len = r.read_var_len()?;
                    let mag = r.take(sig_len)?;
                    SuffixBody::Float(Decimal {
                        negative,
                        significand: BigInt::from_bytes_be(Sign::Plus, mag),
                        exponent,
                    })
                }
                _ => return None,
            };
            Leaf::Suffixed { value, kind }
        }
        _ => return None,
    })
}

/// The (sign, radix) an int kind tag encodes — the inverse of [`int_kind`], for the suffixed-literal
/// body decode (which reuses the bare-int kind byte). `None` for a non-int tag.
fn int_kind_parts(kind: u8) -> Option<(bool, Radix)> {
    Some(match kind {
        KIND_INT_POS_DEC => (false, Radix::Dec),
        KIND_INT_POS_HEX => (false, Radix::Hex),
        KIND_INT_POS_BIN => (false, Radix::Bin),
        KIND_INT_NEG_DEC => (true, Radix::Dec),
        KIND_INT_NEG_HEX => (true, Radix::Hex),
        KIND_INT_NEG_BIN => (true, Radix::Bin),
        _ => return None,
    })
}

/// Read a raw byte sequence (a `Bytes` leaf's payload) — a length then that many bytes verbatim (no
/// UTF-8 check, unlike [`read_string`]).
fn read_raw_bytes(r: &mut Reader) -> Option<Vec<u8>> {
    let len = r.read_var_len()?;
    Some(r.take(len)?.to_vec())
}

fn read_string(r: &mut Reader) -> Option<String> {
    let len = r.read_var_len()?;
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
        // (+ x x) plus a big int, a hex int, a negative int, an exact decimal, a string, and a bool.
        let mut b = Builder::new();
        let plus = b.name("+");
        let x1 = b.name("x");
        let x2 = b.name("x");
        let big = b.atom_leaf(Leaf::Int {
            value: BigInt::from_str("123456789012345678901234567890").unwrap(),
            radix: Radix::Dec,
        });
        let hex = b.atom_leaf(Leaf::Int {
            value: BigInt::from(0x2A),
            radix: Radix::Hex,
        });
        let neg = b.atom_leaf(Leaf::Int {
            value: BigInt::from(-42),
            radix: Radix::Dec,
        });
        let flt = b.atom_leaf(Leaf::Float(Decimal {
            negative: false,
            significand: BigInt::from_str("15").unwrap(),
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
    fn an_empty_list_node_round_trips_through_the_codec() {
        // The `sample()` fixture only exercises NON-empty lists, yet an empty `Struct::List([])` is a real
        // arena node (the inner `()` of a quote pattern `(quote ())`, now reachable after the empty-list
        // pattern surface landed) — it encodes as a `TAG_LIST` + a var-length count of ZERO with no child
        // ids. Pin that the codec round-trips it (encode → decode → equal + re-encode determinism), both as
        // the root AND nested inside a larger list, so a future decode change that assumed a list has ≥1
        // child can't silently break the `decode` totality / round-trip invariant on the empty case.
        // `encode` canonicalizes (DFS re-index) before serializing, so `decode(encode(a))` returns the
        // CANONICAL arena — structurally equal to `a`, but raw-`==` only if `a` was already canonical.
        // Assert with `structurally_eq` (the round-trip contract), and pin encode DETERMINISM by
        // re-encoding the decoded arena (canonical → canonical is a fixed point → identical bytes).
        let mut b = Builder::new();
        let name = b.name("quote");
        let empty = b.list(vec![]); // `()` — a zero-child list
        let root = b.list(vec![name, empty]); // `(quote ())` — empty list nested under a head
        let a = b.finish(root);
        let bytes = encode(&a);
        let back = decode(&bytes).expect("decode of an arena carrying an empty list");
        assert!(
            a.structurally_eq(&back),
            "empty-list arena not preserved through the codec: {a:?} vs {back:?}"
        );
        assert_eq!(
            bytes,
            encode(&back),
            "re-encode of the decoded (canonical) arena is not byte-identical"
        );

        // Also the degenerate case: an empty list as the ROOT node.
        let mut b2 = Builder::new();
        let only = b2.list(vec![]);
        let a2 = b2.finish(only);
        let back2 = decode(&encode(&a2)).expect("decode of a lone empty-list root");
        assert!(
            a2.structurally_eq(&back2),
            "lone empty-list root not preserved: {a2:?} vs {back2:?}"
        );
    }

    #[test]
    fn a_unicode_name_leaf_round_trips_through_the_codec() {
        // `sample()` uses only ASCII names. A NAME leaf carrying MULTI-BYTE UTF-8 (a unicode identifier)
        // must survive the codec too — its bytes go through the same length-prefixed KIND_NAME encode as
        // a string, but names are the most common leaf and, since names now NFC-normalize at intern
        // (`Builder::leaf_name`), the interned name is a multi-byte NFC sequence the codec must preserve
        // byte-for-byte (a var-len miscount or a byte-vs-char length confusion would corrupt it). Pin a
        // precomposed `café` + a CJK `世界` name through encode → decode.
        let mut b = Builder::new();
        let f = b.name("caf\u{00e9}"); // café (NFC precomposed)
        let g = b.name("\u{4e16}\u{754c}"); // 世界
        let root = b.list(vec![f, g]);
        let a = b.finish(root);
        let back = decode(&encode(&a)).expect("decode of an arena with unicode name leaves");
        assert!(
            a.structurally_eq(&back),
            "unicode name leaves not preserved through the codec: {a:?} vs {back:?}"
        );
        assert_eq!(
            encode(&a),
            encode(&back),
            "re-encode of the decoded arena is not byte-identical"
        );
    }

    #[test]
    fn radix_round_trips() {
        // Same value, different bases -> distinct leaves that survive the round-trip.
        let mut b = Builder::new();
        let dec = b.atom_leaf(Leaf::Int {
            value: BigInt::from(42),
            radix: Radix::Dec,
        });
        let hex = b.atom_leaf(Leaf::Int {
            value: BigInt::from(42),
            radix: Radix::Hex,
        });
        let bin = b.atom_leaf(Leaf::Int {
            value: BigInt::from(42),
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
            significand: BigInt::from(0u32),
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

    #[test]
    fn cyclic_structure_refused() {
        // A hand-built arena whose sole node is a `List` referencing ITSELF. In-bounds (id 0 exists),
        // so the old id-range check accepted it — but it is not a tree, and `canon`'s recursive walk
        // would diverge. `decode` must refuse it rather than hand a consumer a cyclic "tree".
        let mut bytes = SCHEMA_HEADER.to_vec();
        leb128::write_u64(&mut bytes, 0); // leaf_count = 0
        leb128::write_u64(&mut bytes, 1); // struct_count = 1
        bytes.push(TAG_LIST);
        leb128::write_u64(&mut bytes, 1); // one child...
        leb128::write_u64(&mut bytes, 0); // ...which is node 0 itself — a self-cycle
        leb128::write_u64(&mut bytes, 0); // root = 0
        assert_eq!(
            decode(&bytes),
            None,
            "a self-referential list is not a tree"
        );
    }

    #[test]
    fn shared_subtree_refused() {
        // Node 2 is a list `[0, 0]` — leaf-atom node 0 appears twice. In-bounds, but a DAG, not a
        // tree; a naive recursive expander would duplicate the shared subtree (exponential on a chain
        // of such nodes — a decode-bomb). `decode` must refuse the reachable-twice node.
        let mut bytes = SCHEMA_HEADER.to_vec();
        leb128::write_u64(&mut bytes, 1); // leaf_count = 1
        bytes.push(KIND_BOOL_TRUE); // leaf 0
        leb128::write_u64(&mut bytes, 2); // struct_count = 2
        bytes.push(TAG_ATOM);
        leb128::write_u64(&mut bytes, 0); // node 0 = Atom(leaf 0)
        bytes.push(TAG_LIST);
        leb128::write_u64(&mut bytes, 2); // node 1 = List[0, 0] — node 0 shared
        leb128::write_u64(&mut bytes, 0);
        leb128::write_u64(&mut bytes, 0);
        leb128::write_u64(&mut bytes, 1); // root = 1
        assert_eq!(decode(&bytes), None, "a shared subtree is not a tree");
    }

    /// A tiny deterministic PRNG (SplitMix64) so the fuzz sweeps below are reproducible without a
    /// dependency — the crate stays "plain" (see `Cargo.toml`), matching the hand-rolled token-soup
    /// and never-panic tests in `lexer.rs`/`parser.rs`.
    struct SplitMix64(u64);
    impl SplitMix64 {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        }
        fn byte(&mut self) -> u8 {
            (self.next() & 0xff) as u8
        }
    }

    #[test]
    fn decode_is_total_on_arbitrary_bytes() {
        // The module header promises `decode` is TOTAL: it never panics on untrusted input — it
        // either reconstructs a tree (`Some`) or refuses (`None`). Pin that with a broad byte-level
        // fuzz: random junk of every short length, plus random payloads that carry the real header
        // (so the reader gets past the header check and exercises the leaf/struct decode paths). Any
        // panic (OOB slice, unwrap, capacity overflow, unchecked arithmetic) fails this test.
        let mut rng = SplitMix64(0x0bad_c0de_dead_beef);
        // Bare random bytes, lengths 0..=64.
        for len in 0..=64usize {
            for _ in 0..64 {
                let buf: Vec<u8> = (0..len).map(|_| rng.byte()).collect();
                let _ = decode(&buf); // must not panic
            }
        }
        // Random bytes PREFIXED with the real header, so the body decode runs on garbage.
        for len in 0..=96usize {
            for _ in 0..64 {
                let mut buf = SCHEMA_HEADER.to_vec();
                buf.extend((0..len).map(|_| rng.byte()));
                let _ = decode(&buf); // must not panic
            }
        }
    }

    /// The canonical-form fixed point: for any arena `decode` accepts, its CANONICAL encoding
    /// (`encode`, which canonicalizes) must round-trip identically — re-decoding the canonical bytes
    /// and re-encoding reproduces them. This is the bijection guarantee (ast-encoding.md §The Encoding
    /// Is A Bijection) checked on the canonical form. We do NOT compare against the accepted arena
    /// itself: `decode` is LENIENT (it accepts non-canonical layouts — forward references, unreferenced
    /// "dead" leaves), while `encode` canonicalizes, so the raw arena need not be reproduced.
    ///
    /// Return / panic contract: returns `true` iff `bytes` was accepted (decoded) and `false` if
    /// `decode` refused it, so a caller can count acceptances and guard against a vacuous
    /// (never-accepts) sweep. If an accepted input VIOLATES the canonical fixed point, the helper
    /// PANICS (the `assert_eq!` below) — a bug in the codec, which is what the fuzz callers are probing
    /// for; a `false` never signals a fixed-point failure, only a (legitimate) refusal.
    fn assert_canonical_fixed_point(bytes: &[u8]) -> bool {
        let Some(back) = decode(bytes) else {
            return false;
        };
        let canon = encode(&back);
        let redecoded = decode(&canon).expect("canonical bytes always decode");
        assert_eq!(
            canon,
            encode(&redecoded),
            "canonical encoding must be a fixed point"
        );
        true
    }

    #[test]
    fn decode_survives_every_single_byte_mutation_of_a_valid_encoding() {
        // Take real, valid encodings and corrupt them one byte at a time across a range of byte
        // values (plus a byte dropped and a byte inserted at each offset). Each corruption must decode
        // to a well-formed tree or be refused — never panic — and any accepted tree's canonical form
        // must be a fixed point. This walks the header, the length/tag/id fields, and every leaf
        // payload with a corruption at every offset.
        let mut rng = SplitMix64(0x5eed_1234_5678_9abc);
        for a in [sample(), radix_sample()] {
            let good = encode(&a);
            for pos in 0..good.len() {
                for delta in [1u8, 0x7f, 0x80, 0xff] {
                    let mut bytes = good.clone();
                    bytes[pos] = bytes[pos].wrapping_add(delta);
                    assert_canonical_fixed_point(&bytes); // must not panic; accepted → fixed point
                }
                let mut dropped = good.clone();
                dropped.remove(pos);
                assert_canonical_fixed_point(&dropped);
                let mut inserted = good.clone();
                inserted.insert(pos, rng.byte());
                assert_canonical_fixed_point(&inserted);
            }
        }
    }

    #[test]
    fn decode_round_trip_is_idempotent_on_accepted_inputs() {
        // For ANY accepted byte string, the canonical form is a fixed point (bijection guarantee).
        // Random bytes after the header almost never decode (a random `leaf_count` truncates), so we
        // seed the sweep with SMALL mutations of real encodings — those frequently still decode — and
        // assert we found a non-trivial number of accepted inputs so the test isn't vacuous.
        let mut rng = SplitMix64(0xfeed_face_cafe_babe);
        let seeds = [encode(&sample()), encode(&radix_sample())];
        let mut accepted = 0usize;
        for _ in 0..20_000 {
            let seed = &seeds[(rng.next() as usize) % seeds.len()];
            let mut buf = seed.clone();
            // Flip 1..=3 random bytes (keeps many inputs decodable, unlike wholesale randomness).
            let flips = 1 + (rng.next() % 3) as usize;
            for _ in 0..flips {
                if !buf.is_empty() {
                    let i = (rng.next() as usize) % buf.len();
                    buf[i] = rng.byte();
                }
            }
            if assert_canonical_fixed_point(&buf) {
                accepted += 1;
            }
        }
        assert!(
            accepted > 100,
            "sweep near-vacuous: only {accepted} accepted"
        );
    }

    /// A second, structurally different sample used by the mutation sweep: nested lists and every
    /// leaf kind that carries a payload, so the mutation walk touches more decode arms.
    fn radix_sample() -> Arenas {
        let mut b = Builder::new();
        let sym = b.atom_leaf(Leaf::Sym("sym".to_string()));
        let ch = b.atom_leaf(Leaf::Char('λ'));
        let by = b.atom_leaf(Leaf::Bytes(vec![0, 1, 2, 255]));
        let bad = b.atom_leaf(Leaf::BadChar("\\q".to_string()));
        let esc = b.atom_leaf(Leaf::BadEscape('z'));
        let suf = b.atom_leaf(Leaf::Suffixed {
            value: SuffixBody::Int {
                value: BigInt::from(255),
                radix: Radix::Hex,
            },
            kind: SuffixKind::BigInt,
        });
        let inner = b.list(vec![sym, ch, by]);
        let root = b.list(vec![inner, bad, esc, suf]);
        b.finish(root)
    }
}
