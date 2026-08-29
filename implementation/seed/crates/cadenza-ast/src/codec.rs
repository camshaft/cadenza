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

use crate::ast::{
    Arenas, CompoundCtor, Decimal, IntValue, Leaf, LeafId, Radix, Struct, StructId, SuffixBody,
    SuffixKind,
};
use crate::leb128::{self, Reader};
// `alloc` (not std's prelude) so the minimal core compiles under `#![no_std]`.
use alloc::string::String;
use alloc::vec;
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
// A TYPE-SUFFIXED numeric literal (`100N`/`0.5R`). Payload: one suffix byte (`SUFFIX_*`), one
// body-shape byte (`BODY_*`), then the body encoded as a bare int/float would be.
const KIND_SUFFIXED: u8 = 16;
// The non-finite float VALUES — payloadless kind tags (like `KIND_BOOL_*`), a single byte with no body,
// so they are canonical and byte-identical by construction and total over the non-finite space. A
// frozen-contract assignment shared byte-identically with the rcdzc codec twin (and the runtime's op93/
// decode, which `include!`s that twin): `Ast.encode` of a computed NaN/±∞ emits one of these.
const KIND_FLOAT_NAN: u8 = 17;
const KIND_FLOAT_POS_INF: u8 = 18;
const KIND_FLOAT_NEG_INF: u8 = 19;
// The native-compound-data CTOR-HEAD kinds — payloadless kind tags (like `KIND_BOOL_*` / the non-finite
// floats), a single byte with no body. A compound literal's HEAD child is one of these leaves, so the
// compound KIND is recognized by leaf-kind identity (a byte) rather than by comparing the head's text
// (`native-ast-compound-data`; `DESIGN-native-ast-compound-data.md` D1). Appended after the existing
// kinds so the assignment is additive-evolution-safe (no renumber); IDENTICAL byte-for-byte in the rcdzc
// codec twin. `KIND_LIST_CTOR..=KIND_SET_CTOR` are the five collection constructors (a
// [`Leaf::Ctor(CompoundCtor)`] head); `KIND_FIELD_PAIR` is the record/map entry head (`=`) and
// `KIND_MEMBER` the member-access head (`.`).
const KIND_LIST_CTOR: u8 = 20;
const KIND_TUPLE_CTOR: u8 = 21;
const KIND_RECORD_CTOR: u8 = 22;
const KIND_MAP_CTOR: u8 = 23;
const KIND_SET_CTOR: u8 = 24;
const KIND_FIELD_PAIR: u8 = 25;
const KIND_MEMBER: u8 = 26;
// The native RATIONAL head (`Leaf::Rational`) — the payloadless tag of a `(RationalTag <num> <den>)`
// two-child node (children = ordinary `Leaf::Int` value leaves); a distinct data type recognized by kind
// (operator seq-204/207). Payloadless like FIELD_PAIR/MEMBER.
const KIND_RATIONAL: u8 = 27;
const SUFFIX_BIGINT: u8 = 0;
const SUFFIX_RATIONAL: u8 = 1;
const BODY_INT: u8 = 0;
const BODY_FLOAT: u8 = 1;

const TAG_ATOM: u8 = 0;
const TAG_LIST: u8 = 1;

/// Why [`decode_detailed`] rejected a byte string. The load-bearing distinction for a streaming/log
/// consumer (e.g. the agent-harness kernel's crash recovery) is [`DecodeError::Truncated`] — the input
/// ended mid-read, a benign torn/interrupted write — versus EVERY OTHER variant, which means the bytes
/// were all present but did not form a valid canonical AST: genuine corruption. A consumer that only
/// needs that split matches `Truncated` and treats the rest as one "corrupt" case; the finer variants
/// are for diagnostics. `decode` (the `Option`-returning API) is exactly `decode_detailed(_).ok()`, so
/// the two never disagree on which byte strings decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// A read needed more bytes than remained — the input ended mid-header, mid-varint, or mid-field.
    /// An interrupted/torn write, NOT corruption (map to a torn-tail / clean-end in a log).
    Truncated,
    /// The 8-byte container version header is present but is not the recognized tag — a different/older
    /// format or corruption. (Fewer than 8 bytes is [`Self::Truncated`], not this.)
    BadHeader,
    /// A tag/discriminant byte that is present but unrecognized: a structure entry tag (not
    /// atom/list), a leaf kind, a suffix/body shape, or a bool byte (not 0/1).
    BadTag,
    /// A varint (a count, id, or length) is present but not a valid canonical `VarU64` — non-minimal
    /// (overlong) or wider than 64 bits. (`leb128::VarErr::Malformed`.)
    MalformedVarint,
    /// A text field (string/name/sym/char/bad-char/bad-escape body) is present but not valid UTF-8, or
    /// a single-scalar field (`char`/bad-escape) whose bytes are valid UTF-8 but empty.
    BadText,
    /// A referential id is present but out of range: a leaf id ≥ the leaf count, a structure child id
    /// or the root ≥ the structure count, or an id that overflows `u32`.
    IdOutOfRange,
    /// The reachable structure from the root is present and in-range but is not a genuine TREE — a
    /// cycle or a shared subtree (a decode-bomb / stack-overflow hazard for a recursive consumer).
    NotATree,
    /// The AST decoded but bytes remain after it — a framing error or corruption.
    TrailingBytes,
}

impl From<crate::leb128::VarErr> for DecodeError {
    fn from(e: crate::leb128::VarErr) -> Self {
        match e {
            crate::leb128::VarErr::Truncated => DecodeError::Truncated,
            crate::leb128::VarErr::Malformed => DecodeError::MalformedVarint,
        }
    }
}

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

/// Serialize `arenas` to the canonical `cdzast\x00\x01` bytes (with the schema header).
///
/// The arena is CANONICALIZED first (`canon::canonicalize`), so equal programs encode to identical
/// bytes regardless of the order their occurrences were built — the two surfaces build the same tree
/// in different orders (see `canon.rs`). Encoding is thus the point at which the canonical normal
/// form is imposed; `decode` returns that canonical (structurally-equal, re-indexed) arena.
///
/// These bytes ARE the canonical content-address input — the single-source over which a caller takes
/// a content hash. In particular an effect SCHEMA (its op signatures + type contract, represented AS a
/// name-headed cdzast AST — DESIGN-userspace-effects I11b) gets its EFFECT-SCHEMA CONTENT HASH as
/// `Hash::of(encode(&schema_ast))`, where `Hash::of` is the codebase's one unified content-address
/// (blake3, per the operator's one-algo ruling). The hash step is DELIBERATELY the caller's, not this
/// crate's: `cadenza-ast` is the dependency-light bottom crate and stays algo-free — it produces the
/// canonical bytes and the caller hashes them (the established contract — concierge ruling
/// 2026-08-08, floor call (B)). Single-sourcing the ENCODING here removes the one thing that could
/// drift; the hash step is a uniform `Hash::of` everywhere, so there is no per-caller re-derivation.
///
/// Identity taken this way is STABLE across cdzast container-format evolution the same way the kernel's
/// `Event::hash` is: it hashes the canonical `\x00\x01` bytes, which are format-pinned, so equal schemas
/// always hash equal regardless of later additive vocabulary growth (new head names need no format bump;
/// only a genuinely new leaf kind bumps to `\x00\x02`).
pub fn encode(arenas: &Arenas) -> Vec<u8> {
    // Under std, canonicalize to normal form so equal programs encode to identical bytes. `canonicalize`
    // returns a `Cow` — borrowed (no clone/rebuild) when `arenas` is already canonical, which a fresh
    // parse is. The no_std minimal core has no `canon` module and serializes the arena AS GIVEN: a
    // Builder-built or `decode`d arena is already canonical (leaves interned/deduped on insert, structure
    // in occurrence order), so the bytes match — this mirrors rcdzc's minimal encode, which has no canon.
    #[cfg(feature = "std")]
    let canon = crate::canon::canonicalize(arenas);
    #[cfg(feature = "std")]
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

/// Serialize an integer body: the `int_kind` tag byte (sign + radix), then the LEB-framed big-endian
/// magnitude. Shared by the bare [`Leaf::Int`] leaf (whose kind tag IS this leading byte) and the
/// [`SuffixBody::Int`] body (which prefixes a `BODY_INT` marker, then this identical sequence), so both
/// emit byte-identical bytes. Its inverse is [`read_int_body`].
fn write_int_body(out: &mut Vec<u8>, value: &IntValue, radix: Radix) {
    // Zero is never the negative kind (empty magnitude, positive) — the canonical wire form.
    let neg = value.negative && !value.magnitude.is_empty();
    out.push(int_kind(neg, radix));
    leb128::write_u64(out, value.magnitude.len() as u64);
    out.extend_from_slice(&value.magnitude);
}

/// Serialize a float/decimal body: the `negative` flag, the LEB i64 exponent, then the LEB-framed
/// big-endian significand magnitude (the significand is a non-negative magnitude; its sign lives in
/// `negative`). Shared by the bare [`Leaf::Float`] leaf and the [`SuffixBody::Float`] body, each after its
/// own leading kind/`BODY_FLOAT` byte, so both emit byte-identical bytes. Its inverse is [`read_float_body`].
fn write_float_body(out: &mut Vec<u8>, d: &Decimal) {
    out.push(d.negative as u8);
    leb128::write_i64_be(out, d.exponent);
    // The significand is already a non-negative big-endian magnitude (empty = zero).
    leb128::write_u64(out, d.significand.len() as u64);
    out.extend_from_slice(&d.significand);
}

fn write_leaf(out: &mut Vec<u8>, leaf: &Leaf) {
    match leaf {
        Leaf::Int { value, radix } => {
            write_int_body(out, value, *radix);
        }
        Leaf::Float(d) => {
            out.push(KIND_FLOAT);
            write_float_body(out, d);
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
        Leaf::Rational => out.push(KIND_RATIONAL),
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
                    write_int_body(out, value, *radix);
                }
                SuffixBody::Float(d) => {
                    out.push(BODY_FLOAT);
                    write_float_body(out, d);
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
///
/// This is exactly [`decode_detailed`] with the failure reason dropped — use `decode_detailed` when
/// you need to tell a TORN write ([`DecodeError::Truncated`]) from CORRUPTION (every other variant),
/// e.g. a log/stream consumer's crash recovery. Keeping this the sole `Option` surface guarantees the
/// two never disagree on which byte strings decode.
pub fn decode(bytes: &[u8]) -> Option<Arenas> {
    decode_detailed(bytes).ok()
}

/// Decode bytes to `Arenas`, classifying WHY it failed (see [`DecodeError`]). Total: never panics,
/// never over-reads, never returns a wrong tree. `Truncated` means a read ran past the end of the input
/// (a torn/interrupted write); every other variant means the bytes were all present but did not form a
/// valid canonical AST (corruption). Verifies the version header, referential integrity, tree-ness (no
/// cycle or shared subtree — a decode-bomb guard), and that the whole input is consumed.
pub fn decode_detailed(bytes: &[u8]) -> Result<Arenas, DecodeError> {
    // Header. Fewer than 8 bytes = the input ended mid-header = truncated; 8 present but wrong = a
    // different/older format or corruption = BadHeader.
    let header = bytes.get(..8).ok_or(DecodeError::Truncated)?;
    if header != SCHEMA_HEADER {
        return Err(DecodeError::BadHeader);
    }
    let mut r = Reader::new(&bytes[8..]);

    // Leaves.
    let leaf_count = r.read_var_len_checked()?;
    let mut leaves = Vec::with_capacity(leaf_count.min(1 << 16));
    for _ in 0..leaf_count {
        leaves.push(read_leaf(&mut r)?);
    }

    // Structure.
    let struct_count = r.read_var_len_checked()?;
    let mut structure = Vec::with_capacity(struct_count.min(1 << 16));
    for _ in 0..struct_count {
        let tag = r.byte().ok_or(DecodeError::Truncated)?;
        let entry = match tag {
            TAG_ATOM => {
                let leaf_id = r.read_varu64_checked()?;
                if leaf_id as usize >= leaves.len() {
                    return Err(DecodeError::IdOutOfRange); // referential integrity: leaf id in range
                }
                Struct::Atom(LeafId(
                    u32::try_from(leaf_id).map_err(|_| DecodeError::IdOutOfRange)?,
                ))
            }
            TAG_LIST => {
                let n = r.read_var_len_checked()?;
                let mut children = Vec::with_capacity(n.min(1 << 16));
                for _ in 0..n {
                    let child = r.read_varu64_checked()?;
                    children.push(StructId(
                        u32::try_from(child).map_err(|_| DecodeError::IdOutOfRange)?,
                    ));
                }
                Struct::List(children)
            }
            _ => return Err(DecodeError::BadTag),
        };
        structure.push(entry);
    }

    // Root.
    let root = r.read_varu64_checked()?;
    if root as usize >= structure.len() {
        return Err(DecodeError::IdOutOfRange);
    }
    let root = StructId(u32::try_from(root).map_err(|_| DecodeError::IdOutOfRange)?);

    // Referential integrity for structure child ids: every id must be in range. (Atom leaf ids
    // were checked above.) A forward reference is permitted — the codec requires only in-boundsness.
    for entry in &structure {
        if let Struct::List(children) = entry {
            for StructId(id) in children {
                if *id as usize >= structure.len() {
                    return Err(DecodeError::IdOutOfRange);
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
                return Err(DecodeError::NotATree); // reached twice: a cycle or a shared subtree
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
        return Err(DecodeError::TrailingBytes);
    }
    Ok(Arenas {
        leaves,
        structure,
        root,
    })
}

/// Decode an integer body whose already-read `kind` tag encodes its sign + radix: read the LEB-framed
/// big-endian magnitude and rebuild the signed `BigInt`. The inverse of [`write_int_body`], shared by the
/// bare [`Leaf::Int`] arm (which reads the kind tag as the leaf discriminator) and the [`SuffixBody::Int`]
/// arm (which reads the kind tag after its `BODY_INT` marker).
fn read_int_body(r: &mut Reader, kind: u8) -> Result<(IntValue, Radix), DecodeError> {
    let (neg, radix) = int_kind_parts(kind)?;
    let len = r.read_var_len_checked()?;
    let magnitude = r.take(len).ok_or(DecodeError::Truncated)?.to_vec();
    // Store the magnitude verbatim; zero (empty magnitude) is never negative (canonical).
    let negative = neg && !magnitude.is_empty();
    Ok((
        IntValue {
            negative,
            magnitude,
        },
        radix,
    ))
}

/// Decode a float/decimal body: the `negative` flag, the i64 exponent, then the LEB-framed big-endian
/// significand magnitude. The inverse of [`write_float_body`], shared by the bare [`Leaf::Float`] arm and
/// the [`SuffixBody::Float`] arm.
fn read_float_body(r: &mut Reader) -> Result<Decimal, DecodeError> {
    let negative = read_bool(r)?;
    let exponent = r.read_i64_be().ok_or(DecodeError::Truncated)?;
    let sig_len = r.read_var_len_checked()?;
    let magnitude = r.take(sig_len).ok_or(DecodeError::Truncated)?.to_vec();
    Ok(Decimal {
        negative,
        significand: magnitude,
        exponent,
    })
}

fn read_leaf(r: &mut Reader) -> Result<Leaf, DecodeError> {
    let kind = r.byte().ok_or(DecodeError::Truncated)?;
    Ok(match kind {
        KIND_INT_POS_DEC | KIND_INT_POS_HEX | KIND_INT_POS_BIN | KIND_INT_NEG_DEC
        | KIND_INT_NEG_HEX | KIND_INT_NEG_BIN => {
            let (value, radix) = read_int_body(r, kind)?;
            Leaf::Int { value, radix }
        }
        KIND_FLOAT => Leaf::Float(read_float_body(r)?),
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
        KIND_RATIONAL => Leaf::Rational,
        KIND_STR => Leaf::Str(read_string(r)?.into()),
        KIND_BYTES => Leaf::Bytes(read_raw_bytes(r)?.into()),
        KIND_BOOL_FALSE => Leaf::Bool(false),
        KIND_BOOL_TRUE => Leaf::Bool(true),
        KIND_NAME => Leaf::Name(read_string(r)?.into()),
        KIND_SYM => Leaf::Sym(read_string(r)?.into()),
        KIND_BAD_ESCAPE => Leaf::BadEscape(read_scalar(r)?),
        KIND_CHAR => Leaf::Char(read_scalar(r)?),
        KIND_BAD_CHAR => Leaf::BadChar(read_string(r)?.into()),
        // A TYPE-SUFFIXED numeric literal: the suffix byte, a body-shape byte, then the body encoded
        // as a bare int/float (the same layout `write_leaf` emits and the int/float arms above read).
        KIND_SUFFIXED => {
            let kind = match r.byte().ok_or(DecodeError::Truncated)? {
                SUFFIX_BIGINT => SuffixKind::BigInt,
                SUFFIX_RATIONAL => SuffixKind::Rational,
                _ => return Err(DecodeError::BadTag),
            };
            let value = match r.byte().ok_or(DecodeError::Truncated)? {
                BODY_INT => {
                    let kind = r.byte().ok_or(DecodeError::Truncated)?;
                    let (value, radix) = read_int_body(r, kind)?;
                    SuffixBody::Int { value, radix }
                }
                BODY_FLOAT => SuffixBody::Float(read_float_body(r)?),
                _ => return Err(DecodeError::BadTag),
            };
            Leaf::Suffixed { value, kind }
        }
        _ => return Err(DecodeError::BadTag),
    })
}

/// The (sign, radix) an int kind tag encodes — the inverse of [`int_kind`], used for both the bare-int
/// leaf and the suffixed-literal body (which reuses the bare-int kind byte). A non-int tag is a
/// present-but-invalid discriminant → [`DecodeError::BadTag`].
fn int_kind_parts(kind: u8) -> Result<(bool, Radix), DecodeError> {
    Ok(match kind {
        KIND_INT_POS_DEC => (false, Radix::Dec),
        KIND_INT_POS_HEX => (false, Radix::Hex),
        KIND_INT_POS_BIN => (false, Radix::Bin),
        KIND_INT_NEG_DEC => (true, Radix::Dec),
        KIND_INT_NEG_HEX => (true, Radix::Hex),
        KIND_INT_NEG_BIN => (true, Radix::Bin),
        _ => return Err(DecodeError::BadTag),
    })
}

/// Read a raw byte sequence (a `Bytes` leaf's payload) — a length then that many bytes verbatim (no
/// UTF-8 check, unlike [`read_string`]).
fn read_raw_bytes(r: &mut Reader) -> Result<Vec<u8>, DecodeError> {
    let len = r.read_var_len_checked()?;
    Ok(r.take(len).ok_or(DecodeError::Truncated)?.to_vec())
}

/// Read a length-prefixed UTF-8 string. A short read is [`DecodeError::Truncated`]; bytes that are
/// present but not valid UTF-8 are [`DecodeError::BadText`].
fn read_string(r: &mut Reader) -> Result<String, DecodeError> {
    let len = r.read_var_len_checked()?;
    let bytes = r.take(len).ok_or(DecodeError::Truncated)?;
    String::from_utf8(bytes.to_vec()).map_err(|_| DecodeError::BadText)
}

/// Read a single-scalar field (a `Char` / `BadEscape` body) — a UTF-8 string that must hold EXACTLY
/// one scalar. The encoder writes exactly one (`c.encode_utf8`), so anything else is corruption:
/// [`DecodeError::BadText`] for zero scalars (empty) OR more than one. Rejecting a multi-scalar body
/// (rather than taking the first and dropping the tail) keeps decode INJECTIVE — otherwise `"a"` and
/// `"ab"` would both decode to `Char('a')`, and two byte strings decoding to the same value breaks the
/// codec's one-canonical-byte-form bijection (the same discipline as refusing overlong varints /
/// non-tree structures: reject anything a valid encoder never emits).
fn read_scalar(r: &mut Reader) -> Result<char, DecodeError> {
    let s = read_string(r)?;
    let mut chars = s.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => Ok(c),       // exactly one scalar
        _ => Err(DecodeError::BadText), // zero, or more than one
    }
}

fn read_bool(r: &mut Reader) -> Result<bool, DecodeError> {
    match r.byte().ok_or(DecodeError::Truncated)? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(DecodeError::BadTag),
    }
}

// `all(test, feature = "std")`: libtest needs std, so this module only ever built under std — gating it
// explicitly stops cdz-runtime's no_std `#[path]` include (mechanism B) from pulling it into that build.
#[cfg(all(test, feature = "std"))]
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
            value: IntValue::from_bigint(
                &BigInt::from_str("123456789012345678901234567890").unwrap(),
            ),
            radix: Radix::Dec,
        });
        let hex = b.atom_leaf(Leaf::Int {
            value: IntValue::from_i64(0x2A),
            radix: Radix::Hex,
        });
        let neg = b.atom_leaf(Leaf::Int {
            value: IntValue::from_i64(-42),
            radix: Radix::Dec,
        });
        let flt = b.atom_leaf(Leaf::Float(Decimal {
            negative: false,
            significand: IntValue::from_bigint(&BigInt::from_str("15").unwrap()).magnitude,
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
    fn value_encode_of_a_framed_int_tuple_is_the_colon_framed_golden() {
        // CROSS-BACKEND byte-identity pin (mirror of cdz-runtime's
        // `value_encode_of_a_framed_int_tuple_is_the_colon_framed_golden`). `Value.encode (tuple 5 105)`
        // at type `(Tuple Int64 Int64)` must produce the SAME 70-byte colon-framed document on BOTH
        // backends: the wasm face is the cdz-runtime `value-encode` op; the native-rust face builds this
        // exact framed `Arenas` and calls `cadenza_ast::codec::encode` (the codec the emitted rust links).
        // cdz-runtime is a cdylib with no cadenza-ast dep, so the invariant is pinned PER-SIDE against the
        // same golden bytes. This is the standing AUTO guard for the "bare-vs-framed" divergence class
        // (a self-consistent per-backend round-trip once masked a 35-vs-70-byte bug): a future codec change
        // that keeps round-trips green but shifts these bytes fails loud here.
        let mut b = Builder::new();
        // value form: (tuple 5 105)
        let th = b.name("tuple");
        let i5 = b.atom_leaf(Leaf::Int {
            value: IntValue::from_i64(5),
            radix: Radix::Dec,
        });
        let i105 = b.atom_leaf(Leaf::Int {
            value: IntValue::from_i64(105),
            radix: Radix::Dec,
        });
        let tuple_v = b.list(vec![th, i5, i105]);
        // type node: (Tuple Int64 Int64)
        let tn_head = b.name("Tuple");
        let tn_a = b.name("Int64");
        let tn_b = b.name("Int64");
        let type_node = b.list(vec![tn_head, tn_a, tn_b]);
        // frame: (: <value> <type-node>)
        let colon = b.name(":");
        let root = b.list(vec![colon, tuple_v, type_node]);
        let a = b.finish(root);

        let golden: &[u8] = b"cdzast\x00\x01\x06\n\x01:\n\x05tuple\x00\x01\x05\x00\x01i\n\x05Tuple\n\x05Int64\n\x00\x00\x00\x01\x00\x02\x00\x03\x01\x03\x01\x02\x03\x00\x04\x00\x05\x00\x05\x01\x03\x05\x06\x07\x01\x03\x00\x04\x08\t";
        let got = encode(&a);
        assert_eq!(
            got.len(),
            70,
            "framed (tuple 5 105) encoded length changed (was 70): {} bytes",
            got.len()
        );
        assert_eq!(
            got, golden,
            "cadenza_ast::codec::encode of the framed (tuple 5 105) diverged from the cross-backend golden \
             (mirror of cdz-runtime's runtime value-encode pin)"
        );
    }

    // Shared assertion for the cross-backend byte-identity golden pins below: encode the framed value-form
    // arena and require the EXACT bytes. `encode` canonicalizes (interns identical leaves + DFS re-index),
    // which is why e.g. the Record golden carries only 8 leaves though the value + type mention
    // `record`/`=`/`a`/`b`/`Int64` more than once. Each golden was recorded from the native-rust
    // `Value.encode` face and
    // byte-verified equal to the wasm `value-encode` op; v-runtime pins the runtime side to the same bytes.
    fn assert_encodes_to(a: &Arenas, golden: &[u8], what: &str) {
        let got = encode(a);
        assert_eq!(
            got, golden,
            "cadenza_ast::codec::encode of {what} diverged from the cross-backend golden"
        );
    }

    fn int(b: &mut Builder, n: i64) -> crate::ast::StructId {
        b.atom_leaf(Leaf::Int {
            value: IntValue::from_i64(n),
            radix: Radix::Dec,
        })
    }

    #[test]
    fn value_encode_of_a_framed_record_is_the_colon_framed_golden() {
        // (: (record (= a 5) (= b 105)) (record (a Int64) (b Int64))) — the structural Record frame. BOTH
        // the value head AND the type-node head are LOWERCASE `record` (the descriptor `type_node_of`, NOT
        // `type_ast`'s capital `Record`/`(: k T)`), so they intern to ONE atom and each type field is a bare
        // `(name Type)` node — 8 deduped leaves (: record = a INT5 b INT105 Int64). Matches the wasm face;
        // an earlier draft used capital `Record` + colon fields (9 leaves) which DIVERGED — v-runtime's
        // per-side pin caught it, fixed alongside the rcdzc emit_type_node Record arm.
        let mut b = Builder::new();
        let a5 = {
            let eq = b.name("=");
            let ka = b.name("a");
            let v = int(&mut b, 5);
            b.list(vec![eq, ka, v])
        };
        let b105 = {
            let eq = b.name("=");
            let kb = b.name("b");
            let v = int(&mut b, 105);
            b.list(vec![eq, kb, v])
        };
        let rec_head = b.name("record");
        let value = b.list(vec![rec_head, a5, b105]);
        let ta = {
            let ka = b.name("a");
            let ty = b.name("Int64");
            b.list(vec![ka, ty])
        };
        let tb = {
            let kb = b.name("b");
            let ty = b.name("Int64");
            b.list(vec![kb, ty])
        };
        let trec_head = b.name("record");
        let type_node = b.list(vec![trec_head, ta, tb]);
        let colon = b.name(":");
        let root = b.list(vec![colon, value, type_node]);
        let a = b.finish(root);
        let golden: &[u8] = b"cdzast\x00\x01\x08\n\x01:\n\x06record\n\x01=\n\x01a\x00\x01\x05\n\x01b\x00\x01i\n\x05Int64\x14\x00\x00\x00\x01\x00\x02\x00\x03\x00\x04\x01\x03\x02\x03\x04\x00\x02\x00\x05\x00\x06\x01\x03\x06\x07\x08\x01\x03\x01\x05\t\x00\x01\x00\x03\x00\x07\x01\x02\x0c\r\x00\x05\x00\x07\x01\x02\x0f\x10\x01\x03\x0b\x0e\x11\x01\x03\x00\n\x12\x13";
        assert_encodes_to(&a, golden, "the framed (record (= a 5) (= b 105))");
    }

    #[test]
    fn value_encode_of_a_framed_generic_sum_some_is_the_colon_framed_golden() {
        // (: (Some 5) (Option Int64)) — a GENERIC sum, root Framed with the parametric (Option Int64) type node.
        let mut b = Builder::new();
        let some_head = b.name("Some");
        let five = int(&mut b, 5);
        let value = b.list(vec![some_head, five]);
        let opt = b.name("Option");
        let i64n = b.name("Int64");
        let type_node = b.list(vec![opt, i64n]);
        let colon = b.name(":");
        let root = b.list(vec![colon, value, type_node]);
        let a = b.finish(root);
        let golden: &[u8] = b"cdzast\x00\x01\x05\n\x01:\n\x04Some\x00\x01\x05\n\x06Option\n\x05Int64\x08\x00\x00\x00\x01\x00\x02\x01\x02\x01\x02\x00\x03\x00\x04\x01\x02\x04\x05\x01\x03\x00\x03\x06\x07";
        assert_encodes_to(&a, golden, "the framed (Some 5) : (Option Int64)");
    }

    #[test]
    fn value_encode_of_a_framed_generic_sum_none_is_the_colon_framed_golden() {
        // (: (None unit) (Option Int64)) — the nullary variant renders (None unit).
        let mut b = Builder::new();
        let none_head = b.name("None");
        let unit = b.name("unit");
        let value = b.list(vec![none_head, unit]);
        let opt = b.name("Option");
        let i64n = b.name("Int64");
        let type_node = b.list(vec![opt, i64n]);
        let colon = b.name(":");
        let root = b.list(vec![colon, value, type_node]);
        let a = b.finish(root);
        let golden: &[u8] = b"cdzast\x00\x01\x05\n\x01:\n\x04None\n\x04unit\n\x06Option\n\x05Int64\x08\x00\x00\x00\x01\x00\x02\x01\x02\x01\x02\x00\x03\x00\x04\x01\x02\x04\x05\x01\x03\x00\x03\x06\x07";
        assert_encodes_to(&a, golden, "the framed None : (Option Int64)");
    }

    #[test]
    fn value_encode_of_a_framed_monomorphic_sum_multi_payload_is_the_named_framed_golden() {
        // (: (Rect 5 6) Shape) — a MONOMORPHIC sum roots at Named (BARE-name type node `Shape`, not a
        // parametric list), and Rect is a MULTI-payload variant so its two ints spread FLAT: (Rect 5 6).
        let mut b = Builder::new();
        let rect = b.name("Rect");
        let p0 = int(&mut b, 5);
        let p1 = int(&mut b, 6);
        let value = b.list(vec![rect, p0, p1]);
        let type_node = b.name("Shape");
        let colon = b.name(":");
        let root = b.list(vec![colon, value, type_node]);
        let a = b.finish(root);
        let golden: &[u8] = b"cdzast\x00\x01\x05\n\x01:\n\x04Rect\x00\x01\x05\x00\x01\x06\n\x05Shape\x07\x00\x00\x00\x01\x00\x02\x00\x03\x01\x03\x01\x02\x03\x00\x04\x01\x03\x00\x04\x05\x06";
        assert_encodes_to(&a, golden, "the framed (Rect 5 6) : Shape");
    }

    #[test]
    fn value_encode_of_a_framed_float_tuple_is_the_colon_framed_golden() {
        // (: (tuple 5 2.5) (Tuple Int64 Float64)) — pins the exact-shortest-decimal FLOAT leaf
        // (`Leaf::Float(Decimal::from_f64(2.5))` = {false, 25, -1}, KIND_FLOAT), the newest + trickiest
        // codec shape. A lossy-bits encoding would diverge from the wasm `float_leaf` here. Guards the
        // Decimal round-trip encoding cross-backend, mirroring the runtime float pin.
        let mut b = Builder::new();
        let th = b.name("tuple");
        let i5 = int(&mut b, 5);
        let f25 = b.atom_leaf(Leaf::Float(Decimal::from_f64(2.5).unwrap()));
        let value = b.list(vec![th, i5, f25]);
        let tn_head = b.name("Tuple");
        let tn_int = b.name("Int64");
        let tn_float = b.name("Float64");
        let type_node = b.list(vec![tn_head, tn_int, tn_float]);
        let colon = b.name(":");
        let root = b.list(vec![colon, value, type_node]);
        let a = b.finish(root);
        let golden: &[u8] = b"cdzast\x00\x01\x07\n\x01:\n\x05tuple\x00\x01\x05\x06\x00\xff\xff\xff\xff\xff\xff\xff\xff\x01\x19\n\x05Tuple\n\x05Int64\n\x07Float64\n\x00\x00\x00\x01\x00\x02\x00\x03\x01\x03\x01\x02\x03\x00\x04\x00\x05\x00\x06\x01\x03\x05\x06\x07\x01\x03\x00\x04\x08\t";
        assert_encodes_to(
            &a,
            golden,
            "the framed (tuple 5 2.5) : (Tuple Int64 Float64)",
        );
    }

    #[test]
    fn value_encode_of_a_framed_map_is_the_colon_framed_golden() {
        // (: (map (7 70) (8 99)) (Map Int64 Int64)) — the `(map (k v) …)` shape, entries in canonical KEY
        // order; each entry a `(key value)` 2-list. 8 leaves.
        let mut b = Builder::new();
        let map_head = b.name("map");
        let e0 = {
            let k = int(&mut b, 7);
            let v = int(&mut b, 70);
            b.list(vec![k, v])
        };
        let e1 = {
            let k = int(&mut b, 8);
            let v = int(&mut b, 99);
            b.list(vec![k, v])
        };
        let value = b.list(vec![map_head, e0, e1]);
        let tmap = b.name("Map");
        let tk = b.name("Int64");
        let tv = b.name("Int64");
        let type_node = b.list(vec![tmap, tk, tv]);
        let colon = b.name(":");
        let root = b.list(vec![colon, value, type_node]);
        let a = b.finish(root);
        let golden: &[u8] = b"cdzast\x00\x01\x08\n\x01:\n\x03map\x00\x01\x07\x00\x01F\x00\x01\x08\x00\x01c\n\x03Map\n\x05Int64\x0e\x00\x00\x00\x01\x00\x02\x00\x03\x01\x02\x02\x03\x00\x04\x00\x05\x01\x02\x05\x06\x01\x03\x01\x04\x07\x00\x06\x00\x07\x00\x07\x01\x03\t\n\x0b\x01\x03\x00\x08\x0c\r";
        assert_encodes_to(
            &a,
            golden,
            "the framed (map (7 70) (8 99)) : (Map Int64 Int64)",
        );
    }

    #[test]
    fn value_encode_of_a_framed_set_is_the_colon_framed_golden() {
        // (: ((. Set of) (list 7 12 17)) (Set Int64)) — the Set shape: a 2-child value list of the
        // member-access head `(. Set of)` and a `(list …)` of elements in canonical order. 9 leaves.
        let mut b = Builder::new();
        let set_of = {
            let dot = b.name(".");
            let set = b.name("Set");
            let of = b.name("of");
            b.list(vec![dot, set, of])
        };
        let list_v = {
            let lh = b.name("list");
            let e0 = int(&mut b, 7);
            let e1 = int(&mut b, 12);
            let e2 = int(&mut b, 17);
            b.list(vec![lh, e0, e1, e2])
        };
        let value = b.list(vec![set_of, list_v]);
        let tset = b.name("Set");
        let te = b.name("Int64");
        let type_node = b.list(vec![tset, te]);
        let colon = b.name(":");
        let root = b.list(vec![colon, value, type_node]);
        let a = b.finish(root);
        let golden: &[u8] = b"cdzast\x00\x01\t\n\x01:\n\x01.\n\x03Set\n\x02of\n\x04list\x00\x01\x07\x00\x01\x0c\x00\x01\x11\n\x05Int64\x0f\x00\x00\x00\x01\x00\x02\x00\x03\x01\x03\x01\x02\x03\x00\x04\x00\x05\x00\x06\x00\x07\x01\x04\x05\x06\x07\x08\x01\x02\x04\t\x00\x02\x00\x08\x01\x02\x0b\x0c\x01\x03\x00\n\r\x0e";
        assert_encodes_to(
            &a,
            golden,
            "the framed ((. Set of) (list 7 12 17)) : (Set Int64)",
        );
    }

    #[test]
    fn value_encode_of_a_framed_list_is_the_colon_framed_golden() {
        // (: (list 7 12 17) (List Int64)) — the `(list e …)` runtime-length shape. 7 leaves.
        let mut b = Builder::new();
        let lh = b.name("list");
        let e0 = int(&mut b, 7);
        let e1 = int(&mut b, 12);
        let e2 = int(&mut b, 17);
        let value = b.list(vec![lh, e0, e1, e2]);
        let tlist = b.name("List");
        let te = b.name("Int64");
        let type_node = b.list(vec![tlist, te]);
        let colon = b.name(":");
        let root = b.list(vec![colon, value, type_node]);
        let a = b.finish(root);
        let golden: &[u8] = b"cdzast\x00\x01\x07\n\x01:\n\x04list\x00\x01\x07\x00\x01\x0c\x00\x01\x11\n\x04List\n\x05Int64\n\x00\x00\x00\x01\x00\x02\x00\x03\x00\x04\x01\x04\x01\x02\x03\x04\x00\x05\x00\x06\x01\x02\x06\x07\x01\x03\x00\x05\x08\t";
        assert_encodes_to(&a, golden, "the framed (list 7 12 17) : (List Int64)");
    }

    #[test]
    fn every_payload_leaf_kind_including_markers_round_trips_equal_through_the_codec() {
        // `round_trips()` above uses `sample()`, which only carries Int/Float/Str/Bool/Name — it does NOT
        // exercise Sym, Char, Bytes, or the two MARKER leaves (BadChar/BadEscape). `radix_sample()` carries
        // exactly those (+ Suffixed), but it's only fed to the TOTALITY/mutation/idempotence sweeps, which
        // assert decode doesn't PANIC — not that the arena round-trips EQUAL. That leaves a gap: a decode
        // change could corrupt a marker's/Sym's/Char's payload (wrong scalar, truncated text) while still
        // not panicking, so totality holds but faithful round-trip silently breaks. This matters most for
        // the markers: BadChar/BadEscape exist specifically to SURVIVE the binary codec so the compiler can
        // reject them (CDZ0001/0002) — if the codec mangled a marker, the compiler would reject the wrong
        // thing or miss the defect. Pin encode->decode EQUALITY over every payload-carrying leaf kind, plus
        // re-encode determinism. `encode` canonicalizes (DFS re-index), so assert with `structurally_eq`
        // (the round-trip contract, robust to a non-canonical build) rather than raw `==`.
        let a = radix_sample();
        let bytes = encode(&a);
        let back = decode(&bytes).expect("decode of the every-leaf-kind fixture");
        assert!(
            a.structurally_eq(&back),
            "every-leaf-kind arena (Sym/Char/Bytes/BadChar/BadEscape/Suffixed/FloatNan/FloatInf) not \
             preserved through the codec: {a:?} vs {back:?}"
        );
        assert_eq!(
            bytes,
            encode(&back),
            "re-encode of the decoded (canonical) every-leaf-kind arena is not byte-identical"
        );
    }

    #[test]
    fn non_finite_float_leaves_encode_to_the_frozen_payloadless_tags_17_18_19() {
        // The operator-directed non-finite float VALUES (so `Ast.encode` of NaN/±∞ SUCCEEDS) are a
        // FROZEN contract shared byte-identically across cadenza-ast, the rcdzc codec twin, and the
        // runtime's op93/decode: KIND_FLOAT_NAN=17, KIND_FLOAT_POS_INF=18, KIND_FLOAT_NEG_INF=19 —
        // each a single kind byte with NO body (canonical + total). Pin the EXACT tag bytes (a future
        // edit cannot silently renumber them), payloadlessness (a lone-atom leaf section is exactly the
        // one kind byte), and that each round-trips encode->decode equal.
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
                "{leaf:?} must encode to the single frozen tag byte {tag}"
            );
            let mut r = Reader::new(&raw);
            assert_eq!(
                read_leaf(&mut r).unwrap(),
                leaf,
                "read_leaf inverts tag {tag}"
            );
            let mut b = Builder::new();
            let root = b.atom_leaf(leaf.clone());
            let a = b.finish(root);
            let back = decode(&encode(&a)).expect("decode of a lone non-finite-float leaf");
            assert!(a.structurally_eq(&back), "{leaf:?} arena round-trip");
        }
        // The three tags are distinct — no two non-finite leaves collide on the wire.
        let enc = |l: &Leaf| {
            let mut v = Vec::new();
            write_leaf(&mut v, l);
            v
        };
        let nan = enc(&Leaf::FloatNan);
        let pinf = enc(&Leaf::FloatInf { negative: false });
        let ninf = enc(&Leaf::FloatInf { negative: true });
        assert!(
            nan != pinf && pinf != ninf && nan != ninf,
            "the three non-finite float tags are distinct"
        );
    }

    #[test]
    fn native_compound_ctor_head_leaves_encode_to_the_frozen_payloadless_tags_20_through_26() {
        // The native-compound-data CTOR-HEAD leaves (a compound literal's head is recognized by leaf-KIND
        // identity, not head text — `DESIGN-native-ast-compound-data.md` D1) are a FROZEN wire contract
        // shared byte-identically with the rcdzc codec twin: KIND_LIST_CTOR=20, TUPLE=21, RECORD=22,
        // MAP=23, SET=24, FIELD_PAIR(`=`)=25, MEMBER(`.`)=26 — each a single kind byte with NO body,
        // appended after the non-finite floats (19) so the assignment is additive-evolution-safe. Pin the
        // EXACT tag bytes (a future edit cannot silently renumber them), payloadlessness, and equal
        // round-trip encode->decode.
        for (leaf, tag) in [
            (Leaf::Ctor(CompoundCtor::List), 20u8),
            (Leaf::Ctor(CompoundCtor::Tuple), 21u8),
            (Leaf::Ctor(CompoundCtor::Record), 22u8),
            (Leaf::Ctor(CompoundCtor::Map), 23u8),
            (Leaf::Ctor(CompoundCtor::Set), 24u8),
            (Leaf::FieldPair, 25u8),
            (Leaf::Member, 26u8),
            (Leaf::Rational, 27u8),
        ] {
            let mut raw = Vec::new();
            write_leaf(&mut raw, &leaf);
            assert_eq!(
                raw,
                vec![tag],
                "{leaf:?} must encode to the single frozen tag byte {tag}"
            );
            let mut r = Reader::new(&raw);
            assert_eq!(
                read_leaf(&mut r).unwrap(),
                leaf,
                "read_leaf inverts tag {tag}"
            );
            // A lone ctor-head leaf as an arena root round-trips through the full codec.
            let mut b = Builder::new();
            let root = b.atom_leaf(leaf.clone());
            let a = b.finish(root);
            let back = decode(&encode(&a)).expect("decode of a lone ctor-head leaf");
            assert!(a.structurally_eq(&back), "{leaf:?} arena round-trip");
        }
        // All seven tags are distinct — no two ctor-head leaves collide on the wire.
        let enc = |l: &Leaf| {
            let mut v = Vec::new();
            write_leaf(&mut v, l);
            v
        };
        let all = [
            enc(&Leaf::Ctor(CompoundCtor::List)),
            enc(&Leaf::Ctor(CompoundCtor::Tuple)),
            enc(&Leaf::Ctor(CompoundCtor::Record)),
            enc(&Leaf::Ctor(CompoundCtor::Map)),
            enc(&Leaf::Ctor(CompoundCtor::Set)),
            enc(&Leaf::FieldPair),
            enc(&Leaf::Member),
            enc(&Leaf::Rational),
        ];
        for i in 0..all.len() {
            for j in (i + 1)..all.len() {
                assert_ne!(
                    all[i], all[j],
                    "ctor-head tags {i} and {j} must be distinct"
                );
            }
        }
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
    fn a_bytes_leaf_round_trips_through_the_codec_including_empty_and_high_bytes() {
        // `Leaf::Bytes` is the length-prefixed raw-bytes wire node (`KIND_BYTES` + `write_bytes` = a
        // var-len byte count then the raw bytes; decode via `read_raw_bytes`). The generated `gen_leaf`
        // sweep only ever produces FIXED-LENGTH-2 byte vectors, so two contract edges go unexercised by
        // it: the EMPTY byte sequence (length prefix 0, zero payload — the case most prone to a
        // count/`read_raw_bytes` off-by-one) and HIGH bytes ≥ 0x80 / an embedded 0x00 (which must ride
        // verbatim, NOT as UTF-8 like a `Str`). Pin both explicitly. This is also the exact wire contract
        // the new `Ast.Bytes` metaprogramming node rests on: it reuses THIS `Leaf::Bytes`/`KIND_BYTES`
        // path (no new frozen tag — v-metaprogramming's Ast.Bytes maps a bytes value onto a Bytes leaf
        // atom), so a regression here would silently break `Ast.encode`/`decode` of a bytes literal.
        let mut b = Builder::new();
        let empty = b.atom_leaf(Leaf::Bytes(vec![].into())); // zero-length: length prefix 0, no payload
        let high = b.atom_leaf(Leaf::Bytes(vec![0x89, b'P', b'N', b'G', 0x00, 0xff].into())); // PNG-ish, incl 0x00/0xff
        let ascii = b.atom_leaf(Leaf::Bytes(b"hi".to_vec().into()));
        let root = b.list(vec![empty, high, ascii]);
        let a = b.finish(root);
        let bytes = encode(&a);
        let back = decode(&bytes).expect("decode of an arena carrying Bytes leaves");
        assert!(
            a.structurally_eq(&back),
            "Bytes leaves (empty + high-byte) not preserved through the codec: {a:?} vs {back:?}"
        );
        assert_eq!(
            bytes,
            encode(&back),
            "re-encode of the decoded arena is not byte-identical (Bytes wire not deterministic)"
        );
        // Three DISTINCT Bytes leaves SURVIVE the codec (a Bytes value's identity is its exact byte
        // sequence — the empty, the high-byte, and the ASCII vec must not collapse or reorder). Assert on
        // the DECODED `back` arena, not the pre-encode `a` we just built with 3 (that would be
        // tautological — a codec that dropped/merged a Bytes leaf changes `back.leaves`, not `a.leaves`).
        assert_eq!(
            a.leaves.len(),
            3,
            "input built with three distinct Bytes leaves"
        );
        assert_eq!(
            back.leaves.len(),
            3,
            "three distinct Bytes leaves must SURVIVE the codec (decoded pool preserved, none dropped/merged)"
        );
        // And a Bytes leaf is NOT confused with a same-text Str: `b"hi"` (Bytes) ≠ `"hi"` (Str) on the wire.
        let mut b2 = Builder::new();
        let as_str = b2.atom_leaf(Leaf::Str("hi".into()));
        let str_root = b2.list(vec![as_str]);
        let str_a = b2.finish(str_root);
        let mut b3 = Builder::new();
        let as_bytes = b3.atom_leaf(Leaf::Bytes(b"hi".to_vec().into()));
        let bytes_root = b3.list(vec![as_bytes]);
        let bytes_a = b3.finish(bytes_root);
        assert_ne!(
            encode(&str_a),
            encode(&bytes_a),
            "a Str and a Bytes carrying the same text must encode DISTINCTLY (different KIND tag)"
        );
    }

    #[test]
    fn radix_round_trips() {
        // Same value, different bases -> distinct leaves that survive the round-trip.
        let mut b = Builder::new();
        let dec = b.atom_leaf(Leaf::Int {
            value: IntValue::from_i64(42),
            radix: Radix::Dec,
        });
        let hex = b.atom_leaf(Leaf::Int {
            value: IntValue::from_i64(42),
            radix: Radix::Hex,
        });
        let bin = b.atom_leaf(Leaf::Int {
            value: IntValue::from_i64(42),
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
            significand: IntValue::from_i64((0u32) as i64).magnitude,
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
    fn decode_detailed_classifies_torn_vs_corrupt() {
        // The whole point of `decode_detailed`: a consumer (the agent-harness kernel's crash recovery)
        // must tell a TORN write — the input ended mid-read, a benign interrupted append — from
        // CORRUPTION — the bytes are all present but do not form a valid canonical AST. `Truncated` is
        // the torn case; EVERY other variant is corruption.
        let a = sample();
        let good = encode(&a);
        assert!(decode_detailed(&good).is_ok(), "the sample decodes");

        // TRUNCATED: every proper prefix (past the 8-byte header — a shorter one is also Truncated)
        // ends mid-read. A torn tail, never mislabeled as corruption.
        for cut in 0..good.len() {
            assert_eq!(
                decode_detailed(&good[..cut]),
                Err(DecodeError::Truncated),
                "a {cut}-byte prefix is a torn write, not corruption"
            );
        }

        // BAD_HEADER: 8 bytes present but not the tag (a different/older format).
        {
            let mut b = good.clone();
            b[0] ^= 0xff;
            assert_eq!(decode_detailed(&b), Err(DecodeError::BadHeader));
        }

        // TRAILING_BYTES: a complete AST followed by extra bytes.
        {
            let mut b = good.clone();
            b.push(0x00);
            assert_eq!(decode_detailed(&b), Err(DecodeError::TrailingBytes));
        }

        // BAD_TAG: a structure-entry tag that is neither atom nor list — hand-build a 1-node arena
        // (0 leaves) whose sole entry tag is bogus.
        {
            let mut b = SCHEMA_HEADER.to_vec();
            leb128::write_u64(&mut b, 0); // leaf_count
            leb128::write_u64(&mut b, 1); // struct_count
            b.push(0x7f); // neither TAG_ATOM nor TAG_LIST
            assert_eq!(decode_detailed(&b), Err(DecodeError::BadTag));
        }

        // BAD_TAG: an unknown LEAF kind byte.
        {
            let mut b = SCHEMA_HEADER.to_vec();
            leb128::write_u64(&mut b, 1); // leaf_count = 1
            b.push(0xfe); // an unknown leaf kind
            assert_eq!(decode_detailed(&b), Err(DecodeError::BadTag));
        }

        // ID_OUT_OF_RANGE: a leaf id ≥ the leaf count.
        {
            let mut b = SCHEMA_HEADER.to_vec();
            leb128::write_u64(&mut b, 0); // leaf_count = 0
            leb128::write_u64(&mut b, 1); // struct_count = 1
            b.push(TAG_ATOM);
            leb128::write_u64(&mut b, 0); // leaf id 0 — out of range (no leaves)
            leb128::write_u64(&mut b, 0); // root
            assert_eq!(decode_detailed(&b), Err(DecodeError::IdOutOfRange));
        }

        // NOT_A_TREE: a single list node that references itself (in-bounds but cyclic).
        {
            let mut b = SCHEMA_HEADER.to_vec();
            leb128::write_u64(&mut b, 0); // leaf_count = 0
            leb128::write_u64(&mut b, 1); // struct_count = 1
            b.push(TAG_LIST);
            leb128::write_u64(&mut b, 1); // one child…
            leb128::write_u64(&mut b, 0); // …which is node 0 itself → a cycle
            leb128::write_u64(&mut b, 0); // root
            assert_eq!(decode_detailed(&b), Err(DecodeError::NotATree));
        }

        // MALFORMED_VARINT: a non-canonical (overlong) leaf-count varint — all bytes present but not a
        // valid VarU64. Corruption, NOT truncation, even though it sits right after the header.
        {
            let mut b = SCHEMA_HEADER.to_vec();
            b.extend_from_slice(&[0x80, 0x00]); // overlong 0
            assert_eq!(decode_detailed(&b), Err(DecodeError::MalformedVarint));
        }

        // BAD_TEXT: a Str leaf whose body is present but not valid UTF-8.
        {
            let mut b = SCHEMA_HEADER.to_vec();
            leb128::write_u64(&mut b, 1); // leaf_count = 1
            b.push(KIND_STR);
            leb128::write_u64(&mut b, 1); // body len = 1
            b.push(0xff); // 0xff is never valid UTF-8
            assert_eq!(decode_detailed(&b), Err(DecodeError::BadText));
        }

        // BAD_TEXT: a single-scalar field (Char) whose body is VALID UTF-8 but holds MORE THAN one
        // scalar ("ab"). The encoder writes exactly one scalar, so a multi-scalar body is corruption —
        // and accepting it (taking the first, dropping "b") would make "a" and "ab" both decode to
        // Char('a'), breaking the one-canonical-byte-form bijection. Also the empty (zero-scalar) case.
        {
            let mut b = SCHEMA_HEADER.to_vec();
            leb128::write_u64(&mut b, 1); // leaf_count = 1
            b.push(KIND_CHAR);
            leb128::write_u64(&mut b, 2); // body len = 2
            b.extend_from_slice(b"ab"); // two scalars — must reject, not truncate to 'a'
            assert_eq!(
                decode_detailed(&b),
                Err(DecodeError::BadText),
                "a multi-scalar char body is corruption, not a silently-truncated 'a'"
            );
        }
        {
            let mut b = SCHEMA_HEADER.to_vec();
            leb128::write_u64(&mut b, 1); // leaf_count = 1
            b.push(KIND_CHAR);
            leb128::write_u64(&mut b, 0); // body len = 0 — zero scalars
            assert_eq!(decode_detailed(&b), Err(DecodeError::BadText));
        }
    }

    #[test]
    fn suffixed_leaf_round_trips_every_kind_and_body_shape() {
        // The `Suffixed` leaf is a 2×2 space — {BigInt, Rational} suffix × {Int, Float} body — yet the
        // fixtures only exercise (BigInt, Int) (`radix_sample`). The other three corners
        // (Rational-suffixed, and any Float body) go through decode/encode arms no test reaches, so a
        // future change to the suffix-byte or body-shape-byte layout could silently break them and still
        // pass the whole suite. Pin all four corners through encode → decode → structurally-equal + a
        // byte-identical re-encode (encode canonicalizes, so the round-trip contract is `structurally_eq`;
        // determinism is the re-encode of the decoded canonical arena).
        for kind in [SuffixKind::BigInt, SuffixKind::Rational] {
            for body in [
                SuffixBody::Int {
                    value: IntValue::from_i64(-255),
                    radix: Radix::Hex,
                },
                SuffixBody::Float(Decimal {
                    negative: true,
                    significand: IntValue::from_i64(15).magnitude,
                    exponent: -1,
                }),
            ] {
                let mut b = Builder::new();
                let leaf = b.atom_leaf(Leaf::Suffixed {
                    value: body.clone(),
                    kind,
                });
                let root = b.list(vec![leaf]);
                let a = b.finish(root);
                let bytes = encode(&a);
                let back = decode(&bytes)
                    .unwrap_or_else(|| panic!("decode of a suffixed leaf ({kind:?}, {body:?})"));
                assert!(
                    a.structurally_eq(&back),
                    "suffixed leaf not preserved through the codec ({kind:?}, {body:?}): {a:?} vs {back:?}"
                );
                assert_eq!(
                    bytes,
                    encode(&back),
                    "re-encode of the decoded suffixed leaf ({kind:?}, {body:?}) is not byte-identical"
                );
            }
        }
    }

    #[test]
    fn suffixed_leaf_rejects_a_present_but_invalid_sub_discriminant() {
        // The `KIND_SUFFIXED` decode arm reads THREE inner discriminant bytes after the kind byte — the
        // suffix byte ({BigInt, Rational}), the body-shape byte ({Int, Float}), and (for an Int body) the
        // nested int-kind byte. Each is a present-but-invalid tag → `BadTag`, exactly like the top-level
        // leaf-kind and structure-tag bytes the sibling test pins. But those inner bytes have no reject
        // test, so a decode that accidentally accepted a bogus inner tag (widening the byte form beyond
        // the encoder's output — a bijection break) would go uncaught. Pin all three, each a `Suffixed`
        // leaf truncated right after the offending byte (`Truncated` past that would be a DIFFERENT
        // variant, so we assert the exact `BadTag` at the discriminant, not a later short read).

        // (1) A bogus SUFFIX byte (neither SUFFIX_BIGINT=0 nor SUFFIX_RATIONAL=1).
        {
            let mut b = SCHEMA_HEADER.to_vec();
            leb128::write_u64(&mut b, 1); // leaf_count = 1
            b.push(KIND_SUFFIXED);
            b.push(0x7f); // not a valid suffix kind
            assert_eq!(
                decode_detailed(&b),
                Err(DecodeError::BadTag),
                "an unknown suffix-kind byte is corruption, not truncation"
            );
        }

        // (2) A valid suffix byte, then a bogus BODY-SHAPE byte (neither BODY_INT=0 nor BODY_FLOAT=1).
        {
            let mut b = SCHEMA_HEADER.to_vec();
            leb128::write_u64(&mut b, 1); // leaf_count = 1
            b.push(KIND_SUFFIXED);
            b.push(SUFFIX_BIGINT);
            b.push(0x7f); // not a valid body shape
            assert_eq!(
                decode_detailed(&b),
                Err(DecodeError::BadTag),
                "an unknown suffixed body-shape byte is corruption, not truncation"
            );
        }

        // (3) A valid suffix + Int body, then a bogus NESTED INT-KIND byte (> KIND_INT_NEG_BIN=5).
        {
            let mut b = SCHEMA_HEADER.to_vec();
            leb128::write_u64(&mut b, 1); // leaf_count = 1
            b.push(KIND_SUFFIXED);
            b.push(SUFFIX_RATIONAL);
            b.push(BODY_INT);
            b.push(0x7f); // not a valid int-kind tag (int_kind_parts rejects it)
            assert_eq!(
                decode_detailed(&b),
                Err(DecodeError::BadTag),
                "an unknown nested int-kind byte in a suffixed Int body is corruption, not truncation"
            );
        }
    }

    #[test]
    fn decode_and_decode_detailed_agree_on_every_input() {
        // `decode` IS `decode_detailed(_).ok()`, so for ANY bytes they must agree on accept/reject and
        // on the decoded arena. Sweep random byte soup (with and without a valid header prefix) to pin
        // that they never diverge — a divergence would mean the Option surface and the classified
        // surface disagree on what a valid AST byte stream is.
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
        let mut rng = Rng(0xc0de_c0de_1eb1_2803);
        for _ in 0..20_000 {
            let len = (rng.next() % 40) as usize;
            let mut buf: Vec<u8> = (0..len).map(|_| (rng.next() & 0xff) as u8).collect();
            // Half the time, prepend a valid header so the interesting post-header paths are reached.
            if rng.next() & 1 == 0 {
                let mut h = SCHEMA_HEADER.to_vec();
                h.extend_from_slice(&buf);
                buf = h;
            }
            assert_eq!(
                decode(&buf),
                decode_detailed(&buf).ok(),
                "decode and decode_detailed diverge on {buf:?}"
            );
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
        let sym = b.atom_leaf(Leaf::Sym("sym".into()));
        let ch = b.atom_leaf(Leaf::Char('λ'));
        let by = b.atom_leaf(Leaf::Bytes(vec![0, 1, 2, 255].into()));
        let bad = b.atom_leaf(Leaf::BadChar("\\q".into()));
        let esc = b.atom_leaf(Leaf::BadEscape('z'));
        let suf = b.atom_leaf(Leaf::Suffixed {
            value: SuffixBody::Int {
                value: IntValue::from_i64(255),
                radix: Radix::Hex,
            },
            kind: SuffixKind::BigInt,
        });
        let nan = b.atom_leaf(Leaf::FloatNan);
        let pinf = b.atom_leaf(Leaf::FloatInf { negative: false });
        let ninf = b.atom_leaf(Leaf::FloatInf { negative: true });
        let inner = b.list(vec![sym, ch, by]);
        let root = b.list(vec![inner, bad, esc, suf, nan, pinf, ninf]);
        b.finish(root)
    }

    // ---- decode totality (never-panic) ----
    //
    // `decode` is a TOTAL function on arbitrary bytes: it must return `None`/`Err` on any malformed,
    // truncated, or hostile input — never panic, overflow the stack, or loop. `decode` parses UNTRUSTED
    // transport bytes (a component's embedded AST, a peer's schema payload), so "no input crashes the
    // decoder" is a real robustness invariant, distinct from the hand-targeted per-`DecodeError`-variant
    // tests above (those pin a SPECIFIC corruption → a SPECIFIC error; these pin the WHOLE input space is
    // panic-free). Deterministic (no RNG — unavailable/non-reproducible in this harness). Kept IN-CRATE
    // (not a `tests/*.rs` integration binary) so it compiles with the crate, links nothing extra, and
    // runs fast + per-crate-cacheable (operator directive, prefer-unit-tests).

    /// A small valid arena `(f a 1)` — the seed for the truncation + bit-flip families.
    fn np_sample_encoding() -> Vec<u8> {
        let mut b = Builder::new();
        let f = b.name("f");
        let a = b.name("a");
        let one = b.name("1");
        let root = b.list(vec![f, a, one]);
        encode(&b.finish(root))
    }

    /// Decoding must not panic; also cross-check the two entry points agree (`decode().is_some()` iff
    /// `decode_detailed().is_ok()`), so a future refactor adding a panic to either surface is caught.
    fn np_decode_is_total(bytes: &[u8]) -> bool {
        decode(bytes).is_some() == decode_detailed(bytes).is_ok()
    }

    #[test]
    fn decode_is_total_on_structured_adversarial_families() {
        let mut inputs: Vec<Vec<u8>> = Vec::new();
        // Degenerate lengths.
        inputs.push(vec![]);
        inputs.push(vec![0x00]);
        inputs.push(SCHEMA_HEADER.to_vec()); // header only, no body
        // Valid header + a hostile/garbage body of varied shapes.
        for tail in [
            vec![0xff; 4],
            vec![0x80; 8], // continuation-byte run → overlong/never-terminating varint
            vec![0x7f, 0x7f, 0x7f], // huge-count-then-nothing
            vec![0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01], // a giant-count varint
            vec![0x00, 0x00], // 0 leaves, 0 structures → root read fails
        ] {
            let mut b = SCHEMA_HEADER.to_vec();
            b.extend_from_slice(&tail);
            inputs.push(b);
        }
        // Wrong / near-miss headers.
        inputs.push(b"cdzast\x00\x02".to_vec()); // a future format version
        inputs.push(b"CDZAST\x00\x01".to_vec()); // wrong case
        inputs.push(vec![0xde, 0xad, 0xbe, 0xef, 0xde, 0xad, 0xbe, 0xef]);
        // A long incompressible-looking run.
        inputs.push((0u8..=255).cycle().take(1024).collect());

        for (i, inp) in inputs.iter().enumerate() {
            assert!(
                np_decode_is_total(inp),
                "decode disagreed with decode_detailed on adversarial input #{i} ({} bytes)",
                inp.len()
            );
        }
    }

    #[test]
    fn decode_is_total_on_every_truncation_prefix_of_a_valid_encoding() {
        // A truncated-mid-stream artifact (a partial download, a clipped payload) must decode to a clean
        // error at every cut point, never a panic.
        let good = np_sample_encoding();
        assert!(decode(&good).is_some(), "the seed encoding decodes");
        for cut in 0..=good.len() {
            assert!(
                np_decode_is_total(&good[..cut]),
                "decode panicked/inconsistent on the {cut}-byte prefix"
            );
        }
        // Only the full length is a valid decode; a prefix of a canonical encoding is never canonical.
        for cut in 0..good.len() {
            assert!(
                decode(&good[..cut]).is_none(),
                "a {cut}-byte prefix of a valid encoding must not decode"
            );
        }
    }

    #[test]
    fn decode_is_total_on_every_single_byte_flip_of_a_valid_encoding() {
        // A single corrupted byte anywhere must yield a clean error or a still-valid arena — never a
        // panic/overflow/hang. Flip the high bit then the low bit of each byte (deterministic mutations
        // hitting headers, tags, counts, ids).
        let good = np_sample_encoding();
        for mask in [0x80u8, 0x01] {
            for i in 0..good.len() {
                let mut m = good.clone();
                m[i] ^= mask;
                assert!(
                    np_decode_is_total(&m),
                    "decode panicked/inconsistent on a {mask:#x} flip at byte {i}"
                );
            }
        }
    }

    #[test]
    fn decode_is_total_on_deeply_nested_and_wide_valid_encodings() {
        // A deeply-nested or very-wide valid arena must decode without overflowing the stack (the codec's
        // reachability/tree check is iterative for exactly this reason).
        let mut b = Builder::new();
        let mut node: StructId = b.name("x");
        for _ in 0..2000 {
            node = b.list(vec![node]);
        }
        let bytes = encode(&b.finish(node));
        assert!(
            decode(&bytes).is_some(),
            "a 2000-deep arena round-trips without overflow"
        );

        let mut b2 = Builder::new();
        let kids: Vec<StructId> = (0..5000).map(|k| b2.name(format!("n{k}"))).collect();
        let wide_root = b2.list(kids);
        let wbytes = encode(&b2.finish(wide_root));
        assert!(decode(&wbytes).is_some(), "a 5000-wide arena round-trips");
    }
}
