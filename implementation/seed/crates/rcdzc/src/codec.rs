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
//!       FloatNan | FloatPosInf | FloatNegInf         (no payload — non-finite float values)
//!       List|Tuple|Record|Map|Set-Ctor | FieldPair | Member   (no payload — native-compound-data heads)
//! [ struct_count:var ]
//!   for each structure entry, in canonical (post-order) order:
//!     [ tag:1 ]
//!       Atom  [ leaf_id:var ]
//!       List  [ child_count:var ][ child_id:var ]*
//! [ root:var ]                        a StructId
//! ```
//!
//! This is the compiler's trusted-path decoder: `rcdzc` derives a component from the CANONICAL BINARY
//! AST that `decode` produces, with no textual parser between the stored bytes and the pipeline —
//! parsing/printing live in `cadenza-syntax`, outside the derive path:
//!
//= spec/contracts/ast-encoding.md#parsing-and-printing-are-not-in-the-compiler-s-trusted-path
//# The compiler MUST accept the canonical binary AST directly, without requiring a textual parser in the path that derives a component.
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
//! `encode` is a straight walk of the two vectors and `decode` reconstructs exactly the tree encoded, so
//! the encoding is a bijection; a CANONICAL arena has exactly one such encoding, so equal trees produce
//! identical bytes:
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
//! This binary serialization of the AST IS the program's canonical STORED form — the byte string that
//! is stored, hashed, and handed to the compiler, never a textual rendering. The compiler's derive path
//! decodes THESE bytes (`compile` decodes the `ast`-kinded artifact); a textual `.cdz` is only a
//! projection a tool reads/prints, not the stored program:
//!
//= spec/contracts/ast-encoding.md#the-canonical-stored-form-is-the-binary-ast
//# A Cadenza program's canonical stored form MUST be the binary serialization of its abstract syntax tree.
//= spec/contracts/ast-encoding.md#the-canonical-stored-form-is-the-binary-ast
//# A program MUST be stored as its binary AST rather than as a textual rendering.
//= spec/contracts/ast-encoding.md#the-canonical-stored-form-is-the-binary-ast
//# A program MUST be hashed as its binary AST rather than as a textual rendering.
//= spec/contracts/ast-encoding.md#the-canonical-stored-form-is-the-binary-ast
//# A program MUST be supplied to the compiler as its binary AST rather than as a textual rendering.
//!
//! This is also the agent-authoring canonical form: a program's canonical form IS this binary AST (its
//! identity is independent of any textual rendering), an agent READS it directly (`decode`, the same path
//! the compiler's derive uses — no textual syntax required), and an agent CONSTRUCTS it directly
//! (`encode`, or the arena builder — again no textual syntax).
//= spec/capabilities/agent-authoring.md#the-canonical-form-is-the-binary-ast
//# A program's canonical form MUST be the binary AST fixed by the ast-encoding contract, so that its identity is independent of any textual rendering.
//= spec/capabilities/agent-authoring.md#the-canonical-form-is-the-binary-ast
//# An agent MUST be able to read a program's canonical binary AST directly, without going through a textual syntax.
//= spec/capabilities/agent-authoring.md#the-canonical-form-is-the-binary-ast
//# An agent MUST be able to construct a program's canonical binary AST directly, without going through a textual syntax.
//!
//! This binary serialization of the AST IS the program's canonical form — one canonical byte form
//! independent of any textual rendering:
//!
//= constitution.md#x-programs-are-readable-by-agents-and-humans
//# The canonical form of a program MUST be a stable binary serialization of its abstract syntax tree, such that a program has one canonical byte form independent of any textual rendering.
//!
//! `decode` is TOTAL: it verifies the header and refuses (returns `None`) on a wrong header, malformed
//! length/tag, out-of-range id, or trailing bytes — it never panics and never returns a wrong tree.
//! Determinism ("equal programs -> identical bytes") is a property of CANONICAL arenas (see `canon.rs`),
//! not of the codec: the codec faithfully serializes whatever it is handed.
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
//! is an optional strengthening of the same check — not a gap: the refuse-on-mismatch guarantee holds
//! today, and swapping the tag's content is a drop-in change.

use crate::ast::{Arenas, CompoundCtor, Decimal, IntValue, Leaf, LeafId, Radix, Struct, StructId};
use crate::leb128::{self, Reader};
use alloc::string::String;
use alloc::vec::Vec;

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
// A TYPE-SUFFIXED numeric literal (`100N`/`0.5R`) from the syntax surface. The COMPILER has no
// `Suffixed` leaf variant: the reader already DESUGARED a suffixed atom to `(: <literal> BigInt|
// Rational)`, so the compiler needs only the bare literal. This tag therefore decodes to a plain
// `Int`/`Float` leaf (the suffix is dropped — its type role is carried by the surrounding annotation).
const KIND_SUFFIXED: u8 = 16;
// The non-finite float VALUES — payloadless kind tags (like `KIND_BOOL_*`), a single byte with no body.
// A frozen-contract assignment shared BYTE-IDENTICALLY with the cadenza-ast codec twin (this file is the
// vendored copy; the runtime's op93/decode `include!`s it, so it carries the tags for free).
const KIND_FLOAT_NAN: u8 = 17;
const KIND_FLOAT_POS_INF: u8 = 18;
const KIND_FLOAT_NEG_INF: u8 = 19;
// The native-compound-data CTOR-HEAD kinds — payloadless kind tags (like `KIND_BOOL_*` / the non-finite
// floats), a single byte with no body. A compound literal's HEAD child is one of these leaves, so the
// compound KIND is recognized by leaf-kind identity (a byte) rather than by comparing head text
// (`DESIGN-native-ast-compound-data.md` D1). Appended after the existing kinds (additive-evolution-safe,
// no renumber); IDENTICAL byte-for-byte with the cadenza-ast codec twin. `KIND_LIST_CTOR..=KIND_SET_CTOR`
// are the five collection constructors (a `Leaf::Ctor(CompoundCtor)` head); `KIND_FIELD_PAIR` is the
// record/map entry head (`=`) and `KIND_MEMBER` the member-access head (`.`).
const KIND_LIST_CTOR: u8 = 20;
const KIND_TUPLE_CTOR: u8 = 21;
const KIND_RECORD_CTOR: u8 = 22;
const KIND_MAP_CTOR: u8 = 23;
const KIND_SET_CTOR: u8 = 24;
const KIND_FIELD_PAIR: u8 = 25;
const KIND_MEMBER: u8 = 26;
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
        // Non-finite float VALUES — a single kind byte, no body (like the bool tags).
        Leaf::FloatNan => out.push(KIND_FLOAT_NAN),
        Leaf::FloatInf { negative } => {
            out.push(if *negative {
                KIND_FLOAT_NEG_INF
            } else {
                KIND_FLOAT_POS_INF
            });
        }
        // The native-compound-data CTOR-HEAD leaves — payloadless, one kind byte per constructor (the
        // leaf-kind identity IS the recognized compound kind). No body.
        Leaf::Ctor(ctor) => out.push(match ctor {
            CompoundCtor::List => KIND_LIST_CTOR,
            CompoundCtor::Tuple => KIND_TUPLE_CTOR,
            CompoundCtor::Record => KIND_RECORD_CTOR,
            CompoundCtor::Map => KIND_MAP_CTOR,
            CompoundCtor::Set => KIND_SET_CTOR,
        }),
        Leaf::FieldPair => out.push(KIND_FIELD_PAIR),
        Leaf::Member => out.push(KIND_MEMBER),
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
        // A symbol leaf — the interned name text (mirrors cadenza-syntax's codec `KIND_SYM`).
        Leaf::Sym(s) => {
            out.push(KIND_SYM);
            write_bytes(out, s.as_bytes());
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
    // Header. Bytes that are not the canonical encoding of any value — a wrong header here, or a
    // malformed structure / out-of-range id below — are REFUSED (`None`) rather than misread as a value
    // they do not encode.
    //= spec/contracts/deterministic-value-form.md#decoding-refuses-bytes-that-are-not-a-value-of-the-expected-type
    //# Decoding a byte sequence that is not the canonical byte encoding of any value of the expected type MUST be refused rather than yield a value, so that a decode never misinterprets bytes as a value they do not encode.
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

    // No trailing bytes: valid canonical bytes followed by extra bytes are a DETECTED error, not the
    // value those valid bytes encode — `decode` refuses (returns `None`) rather than silently ignore them.
    //= spec/contracts/deterministic-value-form.md#decoding-refuses-bytes-that-are-not-a-value-of-the-expected-type
    //# A byte sequence that has valid canonical bytes followed by additional bytes MUST NOT decode as the value those valid bytes encode, so that trailing bytes are a detected error rather than silently ignored.
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
        // Non-finite float VALUES — payloadless, so the tag alone reconstructs the leaf.
        KIND_FLOAT_NAN => Leaf::FloatNan,
        KIND_FLOAT_POS_INF => Leaf::FloatInf { negative: false },
        KIND_FLOAT_NEG_INF => Leaf::FloatInf { negative: true },
        // The native-compound-data CTOR-HEAD leaves — payloadless, so the kind byte alone reconstructs
        // the leaf.
        KIND_LIST_CTOR => Leaf::Ctor(CompoundCtor::List),
        KIND_TUPLE_CTOR => Leaf::Ctor(CompoundCtor::Tuple),
        KIND_RECORD_CTOR => Leaf::Ctor(CompoundCtor::Record),
        KIND_MAP_CTOR => Leaf::Ctor(CompoundCtor::Map),
        KIND_SET_CTOR => Leaf::Ctor(CompoundCtor::Set),
        KIND_FIELD_PAIR => Leaf::FieldPair,
        KIND_MEMBER => Leaf::Member,
        KIND_STR => Leaf::Str(read_string(r)?.into()),
        KIND_BYTES => Leaf::Bytes(read_raw_bytes(r)?),
        KIND_BOOL_FALSE => Leaf::Bool(false),
        KIND_BOOL_TRUE => Leaf::Bool(true),
        KIND_NAME => Leaf::Name(read_string(r)?.into()),
        KIND_SYM => Leaf::Sym(read_string(r)?.into()),
        KIND_BAD_ESCAPE => Leaf::BadEscape(read_string(r)?.chars().next()?),
        KIND_CHAR => Leaf::Char(read_string(r)?.chars().next()?),
        KIND_BAD_CHAR => Leaf::BadChar(read_string(r)?.into()),
        // A TYPE-SUFFIXED literal decodes to its BARE `Int`/`Float` leaf: the suffix's type role was
        // already applied by the reader's desugar to `(: <literal> BigInt|Rational)`, so the compiler
        // only needs the value. Skip the suffix byte, then read the body exactly as the int/float arms.
        KIND_SUFFIXED => {
            // Skip (but validate) the suffix byte — its type role is carried by the reader's `(: …)`
            // wrap, so the compiler only needs the body.
            match r.byte()? {
                SUFFIX_BIGINT | SUFFIX_RATIONAL => {}
                _ => return None,
            }
            match r.byte()? {
                BODY_INT => {
                    let (neg, radix) = match r.byte()? {
                        KIND_INT_POS_DEC => (false, Radix::Dec),
                        KIND_INT_POS_HEX => (false, Radix::Hex),
                        KIND_INT_POS_BIN => (false, Radix::Bin),
                        KIND_INT_NEG_DEC => (true, Radix::Dec),
                        KIND_INT_NEG_HEX => (true, Radix::Hex),
                        KIND_INT_NEG_BIN => (true, Radix::Bin),
                        _ => return None,
                    };
                    let len = r.read_var_len()?;
                    let mag = r.take(len)?;
                    Leaf::Int {
                        value: IntValue {
                            negative: neg,
                            magnitude: mag.to_vec(),
                        },
                        radix,
                    }
                }
                BODY_FLOAT => {
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
                _ => return None,
            }
        }
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
        let s = b.atom_leaf(Leaf::Str("héllo".into()));
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
    fn non_finite_float_leaves_encode_to_the_frozen_payloadless_tags_17_18_19() {
        // The non-finite float VALUES are a FROZEN wire contract that MUST stay byte-identical with the
        // cadenza-ast codec twin (and the runtime's op93/decode, which `include!`s this file): NaN=17,
        // +∞=18, −∞=19, each a single payloadless kind byte. Pin the exact tag bytes here too so a drift
        // between the twins is caught in whichever crate is edited.
        for (leaf, tag) in [
            (Leaf::FloatNan, 17u8),
            (Leaf::FloatInf { negative: false }, 18u8),
            (Leaf::FloatInf { negative: true }, 19u8),
        ] {
            let mut raw = Vec::new();
            write_leaf(&mut raw, &leaf);
            assert_eq!(
                raw,
                vec![tag],
                "{leaf:?} must encode to the frozen tag byte {tag}"
            );
            let mut b = Builder::new();
            let root = b.atom_leaf(leaf.clone());
            let a = b.finish(root);
            assert_eq!(
                decode(&encode(&a)).expect("decode of a lone non-finite-float leaf"),
                a,
                "{leaf:?} round-trip"
            );
        }
    }

    #[test]
    fn native_compound_ctor_head_leaves_encode_to_the_frozen_payloadless_tags_20_through_26() {
        // The native-compound-data CTOR-HEAD leaves are a FROZEN wire contract that MUST stay
        // byte-identical with the cadenza-ast codec twin (and the runtime, which `include!`s this file):
        // LIST_CTOR=20, TUPLE=21, RECORD=22, MAP=23, SET=24, FIELD_PAIR(`=`)=25, MEMBER(`.`)=26 — each a
        // single payloadless kind byte, appended after the non-finite floats (19). Pin the exact tag bytes
        // so a drift between the twins is caught in whichever crate is edited.
        let cases = [
            (Leaf::Ctor(CompoundCtor::List), 20u8),
            (Leaf::Ctor(CompoundCtor::Tuple), 21u8),
            (Leaf::Ctor(CompoundCtor::Record), 22u8),
            (Leaf::Ctor(CompoundCtor::Map), 23u8),
            (Leaf::Ctor(CompoundCtor::Set), 24u8),
            (Leaf::FieldPair, 25u8),
            (Leaf::Member, 26u8),
        ];
        for (leaf, tag) in &cases {
            let mut raw = Vec::new();
            write_leaf(&mut raw, leaf);
            assert_eq!(
                raw,
                vec![*tag],
                "{leaf:?} must encode to the frozen tag byte {tag}"
            );
            let mut b = Builder::new();
            let root = b.atom_leaf(leaf.clone());
            let a = b.finish(root);
            assert_eq!(
                decode(&encode(&a)).expect("decode of a lone ctor-head leaf"),
                a,
                "{leaf:?} round-trip"
            );
        }
        // All seven tags are distinct — no two ctor-head leaves collide on the wire.
        for i in 0..cases.len() {
            for j in (i + 1)..cases.len() {
                let (mut a, mut b) = (Vec::new(), Vec::new());
                write_leaf(&mut a, &cases[i].0);
                write_leaf(&mut b, &cases[j].0);
                assert_ne!(a, b, "ctor-head tags {i} and {j} must be distinct");
            }
        }
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
