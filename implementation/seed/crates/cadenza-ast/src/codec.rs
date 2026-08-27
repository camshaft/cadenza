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

use crate::ast::{
    Arenas, Decimal, IntValue, Leaf, LeafId, Radix, Struct, StructId, SuffixBody, SuffixKind,
};
use crate::leb128::{self, Reader};
// `alloc` (not std's prelude) so the minimal core compiles under `#![no_std]`.
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

// The dict transport plane (`DictSet`/content-`Hash`) is std-only — the no_std minimal core is just
// `encode`/`decode`. Gated with the dict-bearing functions below.
#[cfg(feature = "std")]
use crate::dict::{DictSet, Hash};

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
const SUFFIX_BIGINT: u8 = 0;
const SUFFIX_RATIONAL: u8 = 1;
const BODY_INT: u8 = 0;
const BODY_FLOAT: u8 = 1;

const TAG_ATOM: u8 = 0;
const TAG_LIST: u8 = 1;
// A dictionary reference — TRANSPORT-plane only (`cdzast\x00\x02`), NEVER in canonical `\x00\x01` bytes.
// Payload `[dict_idx:var][node_id:var]`: `dict_idx` indexes the import section's hash list, `node_id`
// indexes the referenced dictionary's `structure` arena. `decode_with_dicts` grafts the named subtree
// in place of the ref; the canonical `decode` never accepts a header carrying this tag.
#[cfg(feature = "std")]
const TAG_DICT_REF: u8 = 2;

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
    /// A `cdzast\x00\x02` TRANSPORT artifact imports a dictionary [`Hash`] that is NOT present in the
    /// supplied [`DictSet`] ([`decode_with_dicts`]). This is the HERMETIC-resolution failure (design/
    /// seq-120: the decoder never fetches a missing dict — it errors out). Distinct from corruption:
    /// the bytes are well-formed, the needed input artifact was simply not supplied. Only
    /// `decode_with_dicts` produces this; the canonical `decode`/`decode_detailed` never see `\x00\x02`.
    /// Only the std dict-transport plane produces it, so the variant is std-only (it carries a
    /// dict `Hash`); the no_std minimal core's `decode` never reaches the dict path.
    #[cfg(feature = "std")]
    MissingDict(Hash),
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

/// The 8-byte container tag for the DICTIONARY TRANSPORT plane (`cdzast\x00\x02`). A byte string with
/// this header is a NON-CANONICAL transport artifact (design option A): it may carry dict-imports +
/// `TAG_DICT_REF` nodes, and is decoded ONLY by [`decode_with_dicts`]. The canonical [`decode`]/
/// [`decode_detailed`] REFUSE it (`BadHeader`) — the structural guarantee that a transport artifact can
/// never be mistaken for an identity artifact. The `\x00\x01` canonical plane is untouched.
#[cfg(feature = "std")]
const TRANSPORT_HEADER: [u8; 8] = *b"cdzast\x00\x02";

/// The content-hash width in a transport artifact's import section (a [`Hash`] = 32 bytes).
#[cfg(feature = "std")]
const HASH_LEN: usize = 32;

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
/// crate's: `cadenza-ast` is the dependency-light bottom crate and its [`crate::dict::Hash`] is an
/// algo-free 32-byte container (caller-hashes is the established contract — concierge ruling
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

/// Extract the subtree rooted at `id` of `arenas` into its own standalone `Arenas` (a fresh, dense
/// arena rooted at the copied subtree). Used to compute a subtree's CANONICAL content bytes (via
/// `encode`) for dict-match keying. Iterative (explicit stack), so a deep subtree can't overflow.
#[cfg(feature = "std")]
fn subtree_arena(arenas: &Arenas, id: StructId) -> Arenas {
    let mut leaves: Vec<Leaf> = Vec::new();
    let mut structure: Vec<Struct> = Vec::new();
    enum Job {
        Visit(u32),
        EmitList(u32, usize),
    }
    let mut jobs = vec![Job::Visit(id.0)];
    let mut results: Vec<u32> = Vec::new();
    while let Some(job) = jobs.pop() {
        match job {
            Job::Visit(sid) => match arenas.get(StructId(sid)) {
                Struct::Atom(LeafId(lid)) => {
                    let new_leaf = leaves.len() as u32;
                    leaves.push(arenas.leaf(LeafId(*lid)).clone());
                    let new = structure.len() as u32;
                    structure.push(Struct::Atom(LeafId(new_leaf)));
                    results.push(new);
                }
                Struct::List(children) => {
                    jobs.push(Job::EmitList(sid, children.len()));
                    for &StructId(ch) in children.iter().rev() {
                        jobs.push(Job::Visit(ch));
                    }
                }
            },
            Job::EmitList(_sid, n) => {
                let kids = results.split_off(results.len() - n);
                let new = structure.len() as u32;
                structure.push(Struct::List(kids.into_iter().map(StructId).collect()));
                results.push(new);
            }
        }
    }
    let root = StructId(results.pop().expect("subtree_arena leaves a root"));
    Arenas {
        leaves,
        structure,
        root,
    }
}

/// Encode `arenas` as a TRANSPORT artifact (`cdzast\x00\x02`) that REFERENCES the supplied dictionaries:
/// any subtree of `arenas` that is structurally equal to an importable node of some dict in `dicts` MAY
/// be emitted as a `TAG_DICT_REF` instead of inline (compaction). v1 emits a ref for an EXACT subtree
/// match against a caller-SUPPLIED dict-set; it does NOT choose which subtrees to factor into a
/// dictionary (that is dict CONSTRUCTION, deferred — design §8).
///
/// Matching is GREEDY largest-first: a node is checked BEFORE its children, so a ref replaces the most
/// inline bytes; a smaller nested match inside a larger matched subtree is subsumed. This is purely a
/// compaction heuristic — any ref set round-trips, since `decode_with_dicts` grafts each ref back.
///
/// Imports are emitted in CANONICAL (hash-sorted) order, and only the dicts actually referenced are
/// listed. The identity guarantee: `decode_with_dicts(encode_with_dict(a, d), d) == canonicalize(a)`,
/// and `encode(decode_with_dicts(encode_with_dict(a, d), d)) == encode(a)` — transport is
/// identity-preserving; the canonical inline `encode` remains the sole identity form.
#[cfg(feature = "std")]
pub fn encode_with_dict(arenas: &Arenas, dicts: &DictSet) -> Vec<u8> {
    let canon = crate::canon::canonicalize(arenas);
    let arenas = &*canon;

    // Match table: canonical subtree bytes -> (hash, node_id). Built over every importable node of every
    // supplied dict. Keyed by the subtree's canonical `\x00\x01` bytes so a match is exact structural
    // equality (the same key `encode` would produce for that subtree inline).
    //
    // An UNSAFE-TO-WALK imported dict is SKIPPED entirely (not indexed): `subtree_arena` walks the dict
    // structure and copies its leaves, so a cyclic dict would diverge here during encoding and an
    // out-of-range structure/leaf id would PANIC — the encode-side sibling of the decode graft DoS.
    // `encode_with_dict` is infallible (returns `Vec<u8>`), so it cannot error on a bad dict; instead a
    // malformed dict simply contributes no match candidates → it is never referenced, and the output is
    // still a valid transport artifact (just uncompacted against that dict). `DictSet::insert` does not
    // validate (that is I3's job), so this guard makes the bottom-crate path robust on its own.
    let mut by_bytes: std::collections::HashMap<Vec<u8>, (Hash, u32)> =
        std::collections::HashMap::new();
    for (hash, dict) in dicts.iter() {
        if !dict_is_safe_to_walk(dict) {
            continue; // skip an unsafe dict — do not walk it (would diverge/panic) / index it
        }
        for node in 0..dict.structure.len() as u32 {
            let key = encode(&subtree_arena(dict, StructId(node)));
            // DETERMINISTIC tie-break: if two dict nodes (across the DictSet's HashMap-ordered iteration)
            // encode to the SAME subtree bytes, keep the SMALLEST (hash, node) so the emitted DictRef —
            // and thus the transport bytes — are identical run-to-run for identical DictSet contents (the
            // design's deterministic-transport goal). `or_insert` alone was HashMap-order-dependent.
            by_bytes
                .entry(key)
                .and_modify(|cur| {
                    if (*hash, node) < *cur {
                        *cur = (*hash, node);
                    }
                })
                .or_insert((*hash, node));
        }
    }

    // Walk the input's canonical structure GREEDILY (node before children). For each node, if its
    // canonical subtree bytes match an importable dict node, record a dict-ref; else recurse. Collect the
    // set of referenced dict hashes so the import section lists only what is used, in sorted order.
    //
    // We build the transport structure directly. `emit[sid]` maps an input StructId to its emitted
    // transport-structure id (post-order, parent-after-children), EXCEPT a matched node emits a single
    // TAG_DICT_REF entry and its subtree is not walked.
    // Assign each referenced hash a dict_idx AFTER collecting + sorting; so first record (hash, node) refs.
    let mut refs_used: std::collections::BTreeSet<Hash> = std::collections::BTreeSet::new();

    // First pass: decide, for each node in a pre-order walk, whether it is a dict-match (and skip its
    // subtree) — recording which hashes are referenced. Post-order emit happens in the second pass.
    // To keep this a single structural walk we memo the match decision per node.
    let n = arenas.structure.len();
    let mut matched: Vec<Option<(Hash, u32)>> = vec![None; n];
    {
        // Pre-order from root; when a node matches, record it and DON'T descend (greedy largest-first).
        let mut stack = vec![arenas.root.0 as usize];
        while let Some(sid) = stack.pop() {
            let key = encode(&subtree_arena(arenas, StructId(sid as u32)));
            if let Some(&(hash, node)) = by_bytes.get(&key) {
                matched[sid] = Some((hash, node));
                refs_used.insert(hash);
                continue; // subsumed — do not descend into a matched subtree
            }
            if let Struct::List(children) = &arenas.structure[sid] {
                for StructId(ch) in children {
                    stack.push(*ch as usize);
                }
            }
        }
    }

    // If nothing matched, there is no compaction to do — emit the plain canonical `\x00\x01` form so a
    // dict-free result stays byte-identical to `encode` (a transport artifact with zero refs would only
    // add an empty import section + a header bump for no benefit).
    if refs_used.is_empty() {
        return encode(arenas);
    }

    // Assign dict_idx by sorted hash (BTreeSet iterates in order).
    let imports: Vec<Hash> = refs_used.iter().copied().collect();
    let dict_idx_of: std::collections::HashMap<Hash, u32> = imports
        .iter()
        .enumerate()
        .map(|(i, h)| (*h, i as u32))
        .collect();

    // Build the transport leaves + structure. Only leaves referenced by inline (non-matched) atoms are
    // needed; but to keep this simple + correct we re-walk the (canonical) arena post-order, emitting a
    // fresh dense arena where a matched node becomes a single DICT_REF entry.
    let mut t_leaves: Vec<Leaf> = Vec::new();
    let mut leaf_map: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
    let mut t_structure: Vec<(u8, Vec<u64>)> = Vec::new(); // (tag, payload ids)
    enum Job {
        Visit(u32),
        EmitList(u32, usize),
    }
    let mut jobs = vec![Job::Visit(arenas.root.0)];
    let mut results: Vec<u32> = Vec::new();
    while let Some(job) = jobs.pop() {
        match job {
            Job::Visit(sid) => {
                if let Some((hash, node)) = matched[sid as usize] {
                    let idx = dict_idx_of[&hash];
                    let id = t_structure.len() as u32;
                    t_structure.push((TAG_DICT_REF, vec![idx as u64, node as u64]));
                    results.push(id);
                    continue;
                }
                match &arenas.structure[sid as usize] {
                    Struct::Atom(LeafId(lid)) => {
                        let new_leaf = *leaf_map.entry(*lid).or_insert_with(|| {
                            let nl = t_leaves.len() as u32;
                            t_leaves.push(arenas.leaf(LeafId(*lid)).clone());
                            nl
                        });
                        let id = t_structure.len() as u32;
                        t_structure.push((TAG_ATOM, vec![new_leaf as u64]));
                        results.push(id);
                    }
                    Struct::List(children) => {
                        jobs.push(Job::EmitList(sid, children.len()));
                        for &StructId(ch) in children.iter().rev() {
                            jobs.push(Job::Visit(ch));
                        }
                    }
                }
            }
            Job::EmitList(_sid, cnt) => {
                let kids = results.split_off(results.len() - cnt);
                let id = t_structure.len() as u32;
                t_structure.push((TAG_LIST, kids.iter().map(|k| *k as u64).collect()));
                results.push(id);
            }
        }
    }
    let t_root = results.pop().expect("encode_with_dict leaves a root") as u64;

    // Serialize the transport artifact.
    let mut out = Vec::new();
    out.extend_from_slice(&TRANSPORT_HEADER);
    leb128::write_u64(&mut out, imports.len() as u64);
    for h in &imports {
        out.extend_from_slice(h.as_bytes());
    }
    leb128::write_u64(&mut out, t_leaves.len() as u64);
    for leaf in &t_leaves {
        write_leaf(&mut out, leaf);
    }
    leb128::write_u64(&mut out, t_structure.len() as u64);
    for (tag, ids) in &t_structure {
        out.push(*tag);
        match *tag {
            TAG_ATOM => leb128::write_u64(&mut out, ids[0]),
            TAG_LIST => {
                leb128::write_u64(&mut out, ids.len() as u64);
                for id in ids {
                    leb128::write_u64(&mut out, *id);
                }
            }
            TAG_DICT_REF => {
                leb128::write_u64(&mut out, ids[0]);
                leb128::write_u64(&mut out, ids[1]);
            }
            _ => unreachable!(),
        }
    }
    leb128::write_u64(&mut out, t_root);
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

/// A structure entry on the TRANSPORT (`cdzast\x00\x02`) plane — the canonical `Atom`/`List`, PLUS a
/// dict reference. Kept internal to the transport decoder: it is resolved AWAY (grafted) into a normal
/// dict-free [`Struct`] before any `Arenas` is returned, so the transport variant never escapes into the
/// identity value model.
#[cfg(feature = "std")]
enum TStruct {
    Atom(LeafId),
    List(Vec<StructId>),
    /// `{dict_idx, node_id}` — resolves to the subtree rooted at `structure[node_id]` of the
    /// `dict_idx`-th imported dictionary.
    DictRef {
        dict_idx: u32,
        node_id: u32,
    },
}

/// Decode a possibly-dict-bearing TRANSPORT artifact against a supplied [`DictSet`], EXPANDING every
/// dict-ref into the subtree it names and returning a normal (dict-free, inline-canonical-equivalent)
/// [`Arenas`]. Total, like [`decode`]: never panics, never over-reads, never returns a wrong tree.
///
/// - A `cdzast\x00\x01` input behaves EXACTLY like [`decode_detailed`] (the `dicts` are unused); the two
///   never disagree on a dict-free artifact.
/// - A `cdzast\x00\x02` input: resolve each import hash against `dicts` (a missing hash →
///   [`DecodeError::MissingDict`], hermetic — no fetch), decode the structure (allowing [`TAG_DICT_REF`]),
///   bounds-check every ref (`dict_idx` < import count, `node_id` < that dict's `structure` len — else
///   [`DecodeError::IdOutOfRange`]), and GRAFT the named subtree in place of each ref, producing a
///   normal `Arenas`. The grafted arena is then subject to the SAME tree/decode-bomb guard as `decode`.
///
/// The returned `Arenas` re-encodes via [`encode`] to canonical `cdzast\x00\x01` — that is its identity.
/// The canonical [`decode`]/[`decode_detailed`] REFUSE `\x00\x02` (`BadHeader`): only THIS entry point
/// accepts the transport plane, so a dict artifact can never be mistaken for an identity artifact.
#[cfg(feature = "std")]
pub fn decode_with_dicts(bytes: &[u8], dicts: &DictSet) -> Result<Arenas, DecodeError> {
    // Dispatch on the header. Fewer than 8 bytes = truncated. The canonical `\x00\x01` plane is decoded
    // exactly as `decode_detailed` (dicts unused). Only `\x00\x02` engages the transport path.
    let header = bytes.get(..8).ok_or(DecodeError::Truncated)?;
    if header == SCHEMA_HEADER {
        return decode_detailed(bytes);
    }
    if header != TRANSPORT_HEADER {
        return Err(DecodeError::BadHeader);
    }
    let mut r = Reader::new(&bytes[8..]);

    // Import section: a count, then that many 32-byte content hashes. Resolve each against the supplied
    // DictSet immediately (hermetic — a missing hash is a hard error, never a fetch). `imports[i]` is the
    // decoded dictionary a `TAG_DICT_REF` with `dict_idx == i` grafts from.
    let import_count = r.read_var_len_checked()?;
    let mut imports: Vec<&Arenas> = Vec::with_capacity(import_count.min(1 << 16));
    for _ in 0..import_count {
        let raw = r.take(HASH_LEN).ok_or(DecodeError::Truncated)?;
        let mut h = [0u8; HASH_LEN];
        h.copy_from_slice(raw);
        let hash = Hash(h);
        let dict = dicts.get(&hash).ok_or(DecodeError::MissingDict(hash))?;
        imports.push(dict);
    }

    // Leaves — identical encoding to the canonical plane.
    let leaf_count = r.read_var_len_checked()?;
    let mut leaves = Vec::with_capacity(leaf_count.min(1 << 16));
    for _ in 0..leaf_count {
        leaves.push(read_leaf(&mut r)?);
    }

    // Structure — `Atom`/`List` as canonical, plus `TAG_DICT_REF`. Ids are bounds-checked after the full
    // structure is read (a forward reference is permitted, like the canonical decoder).
    let struct_count = r.read_var_len_checked()?;
    let mut tstructure: Vec<TStruct> = Vec::with_capacity(struct_count.min(1 << 16));
    for _ in 0..struct_count {
        let tag = r.byte().ok_or(DecodeError::Truncated)?;
        let entry = match tag {
            TAG_ATOM => {
                let leaf_id = r.read_varu64_checked()?;
                if leaf_id as usize >= leaves.len() {
                    return Err(DecodeError::IdOutOfRange);
                }
                TStruct::Atom(LeafId(
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
                TStruct::List(children)
            }
            TAG_DICT_REF => {
                let dict_idx = r.read_varu64_checked()?;
                let node_id = r.read_varu64_checked()?;
                // Bounds: which-dict must name a real import; the node id is bounds-checked against that
                // dict's arena during the graft (below), where the dict is in hand.
                if dict_idx as usize >= imports.len() {
                    return Err(DecodeError::IdOutOfRange);
                }
                TStruct::DictRef {
                    dict_idx: u32::try_from(dict_idx).map_err(|_| DecodeError::IdOutOfRange)?,
                    node_id: u32::try_from(node_id).map_err(|_| DecodeError::IdOutOfRange)?,
                }
            }
            _ => return Err(DecodeError::BadTag),
        };
        tstructure.push(entry);
    }

    // Root.
    let root_raw = r.read_varu64_checked()?;
    if root_raw as usize >= tstructure.len() {
        return Err(DecodeError::IdOutOfRange);
    }
    let root = u32::try_from(root_raw).map_err(|_| DecodeError::IdOutOfRange)? as usize;

    // Referential integrity for transport `List` child ids (into the transport structure).
    for entry in &tstructure {
        if let TStruct::List(children) = entry {
            for StructId(id) in children {
                if *id as usize >= tstructure.len() {
                    return Err(DecodeError::IdOutOfRange);
                }
            }
        }
    }

    // No trailing bytes — the whole transport artifact must be consumed.
    if !r.at_end() {
        return Err(DecodeError::TrailingBytes);
    }

    // Tree guard on the TRANSPORT structure, BEFORE grafting. The graft walks these child links, so a
    // cycle/shared subtree among the transport's OWN `List` ids would make the graft diverge/decode-bomb
    // — it must be refused here first (the post-graft guard below can't help: the graft never returns on
    // a cyclic input). A `DictRef` is a leaf for this walk (its expansion is a fresh copy of a dict's
    // tree, which cannot cycle back into the transport structure). Iterative, so it can't overflow.
    {
        let mut visited = vec![false; tstructure.len()];
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            if visited[id] {
                return Err(DecodeError::NotATree); // reached twice: a cycle or a shared subtree
            }
            visited[id] = true;
            if let TStruct::List(children) = &tstructure[id] {
                for StructId(child) in children {
                    stack.push(*child as usize);
                }
            }
        }
    }

    // GRAFT: build a normal dict-free `Arenas` by walking the transport structure from `root`, copying
    // each `Atom`/`List` and, at each `DictRef`, splicing a fresh copy of the named dictionary's subtree.
    // Iterative post-order (an explicit stack — the transport structure can be arbitrarily deep, like a
    // decoded canonical arena), so the graft cannot overflow the native stack. The result's structure is
    // rebuilt fresh (post-order, parent-after-children), so it is a genuine tree by construction; the
    // final tree guard below then also refuses a transport arena whose OWN child ids form a cycle/share.
    let mut g = Grafter {
        src_leaves: leaves,
        out_leaves: Vec::new(),
        leaf_dedup: std::collections::HashMap::new(),
        out: Vec::new(),
    };
    let out_root = g.graft_transport(&tstructure, root, &imports)?;
    let arenas = Arenas {
        leaves: g.out_leaves,
        structure: g.out,
        root: StructId(out_root),
    };

    // Defensive re-check: the transport structure was verified a tree above and the graft rebuilds a
    // fresh post-order tree, so the output IS a tree by construction — this only reasserts the invariant
    // the canonical decoder also enforces (cheap, iterative), guarding any future graft change.
    verify_tree(&arenas)?;
    Ok(arenas)
}

/// Accumulates the grafted (dict-free) output arena while expanding a transport structure. Leaves are
/// interned BY VALUE (dedup) into a single pool as they are encountered — the transport artifact's own
/// leaves AND every grafted dict subtree's leaves — so the result's leaf pool is the minimal deduped
/// pool an equivalent inline arena would have. This is REQUIRED for the identity guarantee: without
/// dedup, a dict subtree would append duplicate `pair`/`a`/… leaves and the re-encoded arena would
/// differ byte-wise from `encode(a)` (though structurally equal), breaking
/// `decode_with_dicts(encode_with_dict(a,d),d) == canonicalize(a)`. (`canon` numbers leaves by
/// FIRST-ENCOUNTER, not by value, so dedup must happen HERE, mirroring how `ast::Builder` interns.)
#[cfg(feature = "std")]
struct Grafter {
    /// The transport artifact's decoded leaf pool, indexed by the artifact's own leaf ids.
    src_leaves: Vec<Leaf>,
    /// The output pool being built (deduped by value via `leaf_dedup`).
    out_leaves: Vec<Leaf>,
    /// Value → output-leaf-id, so an identical leaf (from the artifact or any dict) interns once.
    leaf_dedup: std::collections::HashMap<Leaf, u32>,
    out: Vec<Struct>,
}

#[cfg(feature = "std")]
impl Grafter {
    /// Emit a node into the output arena, returning its new `StructId`. Bounds-checked: an artifact that
    /// expands past `u32::MAX` nodes is refused (`IdOutOfRange`) rather than SILENTLY WRAPPING the id into
    /// arena corruption — the ids are `u32`, and this runs on UNTRUSTED transport input (a dict-ref can
    /// fan out an arena far larger than its own byte length).
    fn push(&mut self, s: Struct) -> Result<u32, DecodeError> {
        let id = u32::try_from(self.out.len()).map_err(|_| DecodeError::IdOutOfRange)?;
        self.out.push(s);
        Ok(id)
    }

    /// Intern a leaf VALUE into the output pool (dedup), returning its output leaf id. Bounds-checked for
    /// the same reason as [`Self::push`] — the leaf id is `u32`.
    fn intern_leaf(&mut self, leaf: Leaf) -> Result<u32, DecodeError> {
        if let Some(&id) = self.leaf_dedup.get(&leaf) {
            return Ok(id);
        }
        let id = u32::try_from(self.out_leaves.len()).map_err(|_| DecodeError::IdOutOfRange)?;
        self.out_leaves.push(leaf.clone());
        self.leaf_dedup.insert(leaf, id);
        Ok(id)
    }

    /// Graft the transport node `t_id` (of `tstructure`) into the output arena, expanding dict-refs, and
    /// return its new id. Iterative post-order over an explicit work stack so deep input can't overflow.
    fn graft_transport(
        &mut self,
        tstructure: &[TStruct],
        t_root: usize,
        imports: &[&Arenas],
    ) -> Result<u32, DecodeError> {
        enum Job {
            Visit(usize),
            EmitList(usize), // finish a transport List: pop `n` child results and emit
        }
        let mut jobs: Vec<Job> = vec![Job::Visit(t_root)];
        let mut results: Vec<u32> = Vec::new();
        while let Some(job) = jobs.pop() {
            match job {
                Job::Visit(t_id) => match &tstructure[t_id] {
                    TStruct::Atom(LeafId(lid)) => {
                        // Re-intern by value: the transport leaf id indexes `src_leaves`; dedup into the
                        // output pool so the result matches a Builder-deduped equivalent.
                        let leaf = self.src_leaves[*lid as usize].clone();
                        let out_leaf = self.intern_leaf(leaf)?;
                        let new = self.push(Struct::Atom(LeafId(out_leaf)))?;
                        results.push(new);
                    }
                    TStruct::List(children) => {
                        jobs.push(Job::EmitList(children.len()));
                        for &StructId(ch) in children.iter().rev() {
                            jobs.push(Job::Visit(ch as usize));
                        }
                    }
                    TStruct::DictRef { dict_idx, node_id } => {
                        let dict = imports[*dict_idx as usize];
                        if (*node_id as usize) >= dict.structure.len() {
                            return Err(DecodeError::IdOutOfRange);
                        }
                        let new = self.graft_dict_subtree(dict, *node_id as usize)?;
                        results.push(new);
                    }
                },
                Job::EmitList(n) => {
                    let kids = results.split_off(results.len() - n);
                    let new = self.push(Struct::List(kids.into_iter().map(StructId).collect()))?;
                    results.push(new);
                }
            }
        }
        Ok(results.pop().expect("graft leaves the root's new id"))
    }

    /// Copy the subtree rooted at `d_root` of dictionary `dict` into the output arena, interning the
    /// dict's leaves (a dict brings its own leaf pool). Iterative post-order.
    ///
    /// DoS GUARD: this must NOT assume the dict subtree is a tree. A `DictSet` is caller-supplied and a
    /// `TAG_DICT_REF` can target ANY `node_id` — including an UNREACHABLE node of the dict, which
    /// `decode`'s tree guard does NOT cover (it checks reachability from the dict's own root only). A dict
    /// with a cycle among its unreachable nodes, referenced at such a node, would make an unguarded graft
    /// LOOP FOREVER (a decode DoS on untrusted input). So the walk carries its own `visited` guard over
    /// the dict's structure under `d_root`: a node reached twice → [`DecodeError::NotATree`]. (A dict
    /// produced by a correct encoder is a tree, so this refuses nothing legitimate.)
    fn graft_dict_subtree(&mut self, dict: &Arenas, d_root: usize) -> Result<u32, DecodeError> {
        enum Job {
            Visit(usize),
            EmitList(usize),
        }
        let mut visited = vec![false; dict.structure.len()];
        let mut jobs: Vec<Job> = vec![Job::Visit(d_root)];
        let mut results: Vec<u32> = Vec::new();
        while let Some(job) = jobs.pop() {
            match job {
                Job::Visit(d_id) => {
                    // Cycle/shared-subtree guard: a dict node reached twice under `d_root` is a cyclic or
                    // shared (non-tree) dict — refuse rather than loop/explode.
                    if *visited.get(d_id).ok_or(DecodeError::IdOutOfRange)? {
                        return Err(DecodeError::NotATree);
                    }
                    visited[d_id] = true;
                    let node = &dict.structure[d_id];
                    match node {
                        Struct::Atom(LeafId(lid)) => {
                            let leaf = dict
                                .leaves
                                .get(*lid as usize)
                                .ok_or(DecodeError::IdOutOfRange)?
                                .clone();
                            let out_leaf = self.intern_leaf(leaf)?;
                            let new = self.push(Struct::Atom(LeafId(out_leaf)))?;
                            results.push(new);
                        }
                        Struct::List(children) => {
                            jobs.push(Job::EmitList(children.len()));
                            for &StructId(ch) in children.iter().rev() {
                                jobs.push(Job::Visit(ch as usize));
                            }
                        }
                    }
                }
                Job::EmitList(n) => {
                    let kids = results.split_off(results.len() - n);
                    let new = self.push(Struct::List(kids.into_iter().map(StructId).collect()))?;
                    results.push(new);
                }
            }
        }
        Ok(results
            .pop()
            .expect("dict subtree graft leaves the root's new id"))
    }
}

/// Whether an imported dictionary's WHOLE structure is SAFE to walk from ANY node — i.e. NO node
/// (reachable or not) lies on a cycle, every structure child id is in range, AND every `Atom`'s leaf id
/// is in range. This is stronger than `verify_tree`'s root-reachability check ON PURPOSE: a
/// `TAG_DICT_REF` can target ANY `node_id`, including one unreachable from the dict's own root.
///
/// This is the guard for the ENCODE path ONLY. `encode_with_dict` walks each supplied dict via
/// `subtree_arena` to build its match table, and that walk is INFALLIBLE (it indexes `dict.structure`
/// via `Arenas::get` and `dict.leaves` via `Arenas::leaf`, both of which PANIC on an out-of-range id,
/// and would diverge/OOM on a cycle) — so a bad dict must be detected and SKIPPED before it is walked.
/// `decode_with_dicts` does NOT call this: its graft has its own FALLIBLE seam that enforces the same
/// invariant inline (out-of-range → `IdOutOfRange`, cycle/shared-subtree → `NotATree`). Named for the
/// property it guarantees (safe-to-walk), not just acyclicity. Iterative DFS with a 3-color
/// (unvisited/on-stack/done) marking over every node as a potential root; O(nodes+edges).
#[cfg(feature = "std")]
fn dict_is_safe_to_walk(dict: &Arenas) -> bool {
    // 0 = unvisited, 1 = on the current DFS stack (a back-edge to it = cycle), 2 = fully explored.
    let mut color = vec![0u8; dict.structure.len()];
    for start in 0..dict.structure.len() {
        if color[start] != 0 {
            continue;
        }
        // Iterative DFS; a frame is (node, whether we've entered it yet).
        let mut stack: Vec<(usize, bool)> = vec![(start, false)];
        while let Some((id, entered)) = stack.pop() {
            if entered {
                color[id] = 2; // all children explored
                continue;
            }
            if color[id] == 1 {
                // Already entered on this DFS stack (a shared DAG node re-reached via a second parent
                // before we colored it done) — SKIP the duplicate push; NOT cycle detection (a back-edge
                // to an on-stack node is caught below at the child-push site, before it's popped again).
                continue;
            }
            if color[id] == 2 {
                continue;
            }
            color[id] = 1;
            stack.push((id, true)); // post-visit marker to color it done
            match dict.structure.get(id) {
                Some(Struct::List(children)) => {
                    for StructId(child) in children {
                        let c = *child as usize;
                        if c >= dict.structure.len() {
                            // An out-of-range child id is INVALID, not just non-cyclic: on the ENCODE
                            // path, `subtree_arena` indexes `dict.structure` via `Arenas::get` and would
                            // PANIC on it. Reject the whole dict (skip it, like a cycle) so
                            // `encode_with_dict` never indexes a bad id. (The DECODE graft rejects it as
                            // IdOutOfRange separately; encode has no such fallible seam, so the guard must
                            // live HERE — the 3rd untrusted-input layer.)
                            return false;
                        }
                        match color[c] {
                            1 => return false, // back-edge to a node on the current stack = a cycle
                            0 => stack.push((c, false)),
                            _ => {}
                        }
                    }
                }
                // Same PANIC class as the out-of-range child, LEAF side: `subtree_arena` copies an Atom's
                // leaf via `Arenas::leaf` (`&self.leaves[i]`), which PANICS on an out-of-range id. The
                // structure DFS above never inspects a leaf id, so check it here — an Atom whose leaf id is
                // past the pool makes the dict unsafe to walk. Reject (skip) the whole dict.
                Some(Struct::Atom(LeafId(lid))) if *lid as usize >= dict.leaves.len() => {
                    return false;
                }
                Some(Struct::Atom(_)) => {} // in-range leaf: nothing more to check
                None => {} // unreachable: `id` came from `0..structure.len()` or an in-range child
            }
        }
    }
    true
}

/// The reachable-structure tree/decode-bomb guard used by `decode_with_dicts` on its GRAFTED output.
/// Every node reachable from the root must be reached EXACTLY once (no cycle, no shared subtree).
/// Iterative, so it cannot overflow on deep input. (`decode_detailed` performs the equivalent check
/// inline over its own decoded structure; this standalone form guards the transport graft's result.)
#[cfg(feature = "std")]
fn verify_tree(arenas: &Arenas) -> Result<(), DecodeError> {
    let mut visited = vec![false; arenas.structure.len()];
    let mut stack = vec![arenas.root.0 as usize];
    while let Some(id) = stack.pop() {
        if id >= arenas.structure.len() {
            return Err(DecodeError::IdOutOfRange);
        }
        if visited[id] {
            return Err(DecodeError::NotATree);
        }
        visited[id] = true;
        if let Struct::List(children) = &arenas.structure[id] {
            for StructId(child) in children {
                stack.push(*child as usize);
            }
        }
    }
    Ok(())
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

    // ---- I1: the dictionary TRANSPORT plane (`cdzast\x00\x02`) + decode_with_dicts ----

    /// Hand-build a `cdzast\x00\x02` transport artifact from parts (no encoder yet — I2). `imports` are
    /// the 32-byte dict hashes (in the order dict_idx references them); `leaves`/`structure` are the
    /// transport body. Structure entries are `(tag, [ids])` where an Atom is `(TAG_ATOM,[leaf_id])`, a
    /// List `(TAG_LIST, child_ids)`, a DictRef `(TAG_DICT_REF,[dict_idx,node_id])`.
    fn build_transport(
        imports: &[[u8; HASH_LEN]],
        leaves: &[Leaf],
        structure: &[(u8, Vec<u64>)],
        root: u64,
    ) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&TRANSPORT_HEADER);
        leb128::write_u64(&mut out, imports.len() as u64);
        for h in imports {
            out.extend_from_slice(h);
        }
        leb128::write_u64(&mut out, leaves.len() as u64);
        for leaf in leaves {
            write_leaf(&mut out, leaf);
        }
        leb128::write_u64(&mut out, structure.len() as u64);
        for (tag, ids) in structure {
            out.push(*tag);
            match *tag {
                TAG_ATOM => leb128::write_u64(&mut out, ids[0]),
                TAG_LIST => {
                    leb128::write_u64(&mut out, ids.len() as u64);
                    for id in ids {
                        leb128::write_u64(&mut out, *id);
                    }
                }
                TAG_DICT_REF => {
                    leb128::write_u64(&mut out, ids[0]); // dict_idx
                    leb128::write_u64(&mut out, ids[1]); // node_id
                }
                _ => unreachable!(),
            }
        }
        leb128::write_u64(&mut out, root);
        out
    }

    /// Parse ONLY the import section (the `HASH_LEN`-byte hashes right after the header's import count) of
    /// a `\x00\x02` transport blob. Lets a test assert EXACTLY which dicts were imported without scanning
    /// the whole blob for `HASH_LEN`-byte windows (which could spuriously match leaf-payload bytes, not the
    /// imports).
    fn parse_transport_imports(bytes: &[u8]) -> Vec<[u8; HASH_LEN]> {
        assert_eq!(&bytes[..8], &TRANSPORT_HEADER, "not a transport artifact");
        let mut r = leb128::Reader::new(&bytes[8..]);
        let count = r.read_var_len_checked().expect("import count");
        (0..count)
            .map(|_| {
                let raw = r.take(HASH_LEN).expect("import hash");
                let mut h = [0u8; HASH_LEN];
                h.copy_from_slice(raw);
                h
            })
            .collect()
    }

    #[test]
    fn decode_with_dicts_on_a_v1_artifact_is_exactly_decode() {
        // A canonical `\x00\x01` input decodes IDENTICALLY through decode_with_dicts (dicts unused) — the
        // two never disagree on a dict-free artifact. Assert over the whole `sample()` shape + an empty
        // AND a populated dict-set (the dicts must be irrelevant to a v1 artifact).
        let a = sample();
        let v1 = encode(&a);
        let empty = DictSet::new();
        let mut full = DictSet::new();
        full.insert(Hash([7u8; 32]), sample());
        for dicts in [&empty, &full] {
            let via_dicts =
                decode_with_dicts(&v1, dicts).expect("v1 decodes via decode_with_dicts");
            let via_plain = decode(&v1).expect("v1 decodes via decode");
            assert_eq!(
                via_dicts, via_plain,
                "decode_with_dicts != decode on a v1 artifact"
            );
            assert!(a.structurally_eq(&via_dicts));
        }
    }

    #[test]
    fn canonical_decode_refuses_a_transport_header() {
        // THE structural identity/transport separation: the canonical decode/decode_detailed accept ONLY
        // `\x00\x01`; a `\x00\x02` transport artifact is refused with BadHeader (refuse-on-mismatch). This
        // guarantees a dict-bearing artifact can NEVER be mistaken for an identity artifact.
        let bytes = build_transport(&[], &[Leaf::Bool(true)], &[(TAG_ATOM, vec![0])], 0);
        assert_eq!(
            decode(&bytes),
            None,
            "canonical decode must refuse \\x00\\x02"
        );
        assert_eq!(
            decode_detailed(&bytes),
            Err(DecodeError::BadHeader),
            "decode_detailed must classify a transport header as BadHeader"
        );
    }

    #[test]
    fn a_dict_free_transport_artifact_decodes_like_v1() {
        // A `\x00\x02` artifact that happens to carry NO imports + no dict-refs is just a re-headered v1
        // tree: decode_with_dicts yields the same arena as decoding the equivalent `\x00\x01` bytes.
        // `(true)` — a one-element list whose child is a Bool atom.
        let bytes = build_transport(
            &[],
            &[Leaf::Bool(true)],
            &[(TAG_ATOM, vec![0]), (TAG_LIST, vec![0])],
            1,
        );
        let a = decode_with_dicts(&bytes, &DictSet::new()).expect("dict-free transport decodes");
        // Compare against the same tree built canonically.
        let mut b = Builder::new();
        let t = b.atom_leaf(Leaf::Bool(true));
        let root = b.list(vec![t]);
        let expected = b.finish(root);
        assert!(
            a.structurally_eq(&expected),
            "dict-free transport != equivalent v1 tree"
        );
    }

    #[test]
    fn a_dict_ref_resolves_and_grafts_the_named_subtree() {
        // The core resolution: a transport artifact references node `j` of an imported dict; decode grafts
        // that subtree in place. Dict = `(pair a b)` (a 4-node arena); the transport is `(f <ref to the
        // dict's root>)`, so the result must be `(f (pair a b))`.
        let mut db = Builder::new();
        let p = db.name("pair");
        let da = db.name("a");
        let dbb = db.name("b");
        let dict_root = db.list(vec![p, da, dbb]); // structure[3] = the (pair a b) list
        let dict = db.finish(dict_root);
        let dict_root_id = dict.root.0 as u64;
        let hash = Hash([0xABu8; 32]);
        let mut dicts = DictSet::new();
        dicts.insert(hash, dict.clone());

        // transport: leaf[0] = Name "f"; structure: [Atom f, DictRef{0, dict_root}, List[0,1]]; root = 2.
        let bytes = build_transport(
            &[[0xABu8; 32]],
            &[Leaf::Name("f".into())],
            &[
                (TAG_ATOM, vec![0]),
                (TAG_DICT_REF, vec![0, dict_root_id]),
                (TAG_LIST, vec![0, 1]),
            ],
            2,
        );
        let got = decode_with_dicts(&bytes, &dicts).expect("dict-ref resolves");

        // Expected: `(f (pair a b))` built inline.
        let mut eb = Builder::new();
        let f = eb.name("f");
        let ep = eb.name("pair");
        let ea = eb.name("a");
        let eb_ = eb.name("b");
        let inner = eb.list(vec![ep, ea, eb_]);
        let eroot = eb.list(vec![f, inner]);
        let expected = eb.finish(eroot);
        assert!(
            got.structurally_eq(&expected),
            "grafted arena != (f (pair a b)); got {got:?}"
        );
        // And the grafted arena re-encodes to canonical `\x00\x01` (its identity) and round-trips.
        assert!(decode(&encode(&got)).unwrap().structurally_eq(&expected));
    }

    #[test]
    fn a_missing_import_hash_is_missing_dict() {
        // Hermetic resolution: a `\x00\x02` importing a hash NOT in the supplied DictSet is MissingDict —
        // NOT a fetch, NOT corruption. The error carries the offending hash.
        let missing = [0x99u8; 32];
        let bytes = build_transport(
            &[missing],
            &[Leaf::Name("f".into())],
            &[
                (TAG_ATOM, vec![0]),
                (TAG_DICT_REF, vec![0, 0]),
                (TAG_LIST, vec![0, 1]),
            ],
            2,
        );
        assert_eq!(
            decode_with_dicts(&bytes, &DictSet::new()),
            Err(DecodeError::MissingDict(Hash(missing))),
            "a missing import hash must be MissingDict(that hash)"
        );
    }

    #[test]
    fn an_out_of_range_dict_ref_is_id_out_of_range() {
        // Bounds: dict_idx past the import count, and node_id past the dict's arena, both → IdOutOfRange
        // (never a panic). Provide one valid dict so the node_id check is reachable for the second case.
        let mut db = Builder::new();
        let only = db.name("x");
        let dict = db.finish(only); // 1-node arena (structure[0])
        let mut dicts = DictSet::new();
        dicts.insert(Hash([1u8; 32]), dict);

        // (1) dict_idx = 5 but only import 0 exists.
        let bad_dict = build_transport(
            &[[1u8; 32]],
            &[Leaf::Name("f".into())],
            &[
                (TAG_ATOM, vec![0]),
                (TAG_DICT_REF, vec![5, 0]),
                (TAG_LIST, vec![0, 1]),
            ],
            2,
        );
        assert_eq!(
            decode_with_dicts(&bad_dict, &dicts),
            Err(DecodeError::IdOutOfRange)
        );

        // (2) dict_idx = 0 (valid), node_id = 9 past the 1-node dict arena.
        let bad_node = build_transport(
            &[[1u8; 32]],
            &[Leaf::Name("f".into())],
            &[
                (TAG_ATOM, vec![0]),
                (TAG_DICT_REF, vec![0, 9]),
                (TAG_LIST, vec![0, 1]),
            ],
            2,
        );
        assert_eq!(
            decode_with_dicts(&bad_node, &dicts),
            Err(DecodeError::IdOutOfRange)
        );
    }

    #[test]
    fn v1_canonical_bytes_are_unchanged_the_frozen_bijection_guard() {
        // §7.2 — THE guard that proves option A held: adding the transport plane must NOT move a single
        // byte of the canonical `\x00\x01` encoding. Re-encode `sample()` and assert the header is v1 and
        // the exact bytes match a decode→re-encode fixed point (encode is deterministic on canonical
        // arenas). If any `\x00\x01` byte shifts, the dict change perturbed the identity plane = wrong.
        let a = sample();
        let bytes = encode(&a);
        assert_eq!(
            &bytes[..8],
            &SCHEMA_HEADER,
            "canonical output must stay on the v1 header"
        );
        assert_ne!(&bytes[..8], &TRANSPORT_HEADER);
        // Determinism / round-trip fixed point (canonical → canonical is byte-identical).
        let back = decode(&bytes).expect("v1 decodes");
        assert_eq!(
            bytes,
            encode(&back),
            "canonical \\x00\\x01 bytes must be a fixed point"
        );
    }

    #[test]
    fn equal_schemas_built_in_different_orders_share_canonical_encode_bytes() {
        // The effect-schema content-hash STABILITY invariant (DESIGN-userspace-effects I11b): the
        // content-hash input is exactly the canonical `encode` bytes, over which the caller does
        // `Hash::of` (algo-free — `cadenza-ast` never hashes). The property callers rely on: two schema
        // arenas that are STRUCTURALLY EQUAL but built in a DIFFERENT occurrence order produce IDENTICAL
        // canonical bytes (so `Hash::of` of them is equal) — `encode` canonicalizes, so occurrence order
        // does not perturb the content address. A schema `(effect E (op get (-> Unit A)) (op put (-> A
        // Unit)))` built two ways.
        fn schema(order_swapped: bool) -> Arenas {
            let mut b = Builder::new();
            let effect = b.name("effect");
            let ename = b.name("E");
            // Build the two op sub-lists; swap which is constructed first to vary occurrence order.
            let mk_get = |b: &mut Builder| {
                let op = b.name("op");
                let n = b.name("get");
                let arrow = b.name("->");
                let unit = b.name("Unit");
                let a_ty = b.name("A");
                let sig = b.list(vec![arrow, unit, a_ty]);
                b.list(vec![op, n, sig])
            };
            let mk_put = |b: &mut Builder| {
                let op = b.name("op");
                let n = b.name("put");
                let arrow = b.name("->");
                let a_ty = b.name("A");
                let unit = b.name("Unit");
                let sig = b.list(vec![arrow, a_ty, unit]);
                b.list(vec![op, n, sig])
            };
            let (get, put) = if order_swapped {
                let put = mk_put(&mut b);
                let get = mk_get(&mut b);
                (get, put)
            } else {
                let get = mk_get(&mut b);
                let put = mk_put(&mut b);
                (get, put)
            };
            let root = b.list(vec![effect, ename, get, put]);
            b.finish(root)
        }
        assert_eq!(
            encode(&schema(false)),
            encode(&schema(true)),
            "structurally-equal schemas built in different orders must share canonical bytes \
             (so their effect-schema content hash is equal)"
        );
    }

    #[test]
    fn distinct_effect_schemas_encode_to_distinct_bytes() {
        // The OTHER half of the schema-identity contract (the negative direction of
        // `equal_schemas_…share…bytes`): the content address must DISCRIMINATE — schemas that differ in a
        // meaningful way (effect name, op set, or op signature) must encode to DIFFERENT canonical bytes,
        // so `Hash::of(encode(effect_schema_tree(..)))` gives distinct effects distinct identities and
        // never collides them. Without this, a future encode change that dropped op names/signatures from
        // the bytes would still pass every "same → same" test while silently collapsing unrelated effects
        // to one hash — a dangerous identity regression. Build via the canonical `effect_schema_tree` (the
        // one identity constructor) and assert each variation perturbs the bytes.
        let sig = |b: &mut Builder, from: &str, to: &str| {
            let (arrow, f, t) = (b.name("->"), b.name(from), b.name(to));
            b.list(vec![arrow, f, t])
        };
        let base = {
            let mut b = Builder::new();
            let s = sig(&mut b, "Unit", "A");
            let root = b.effect_schema_tree("E", &[("get", s)]);
            encode(&b.finish(root))
        };
        // (1) Different EFFECT NAME.
        let diff_name = {
            let mut b = Builder::new();
            let s = sig(&mut b, "Unit", "A");
            let root = b.effect_schema_tree("F", &[("get", s)]);
            encode(&b.finish(root))
        };
        // (2) Different OP NAME (same signature).
        let diff_op = {
            let mut b = Builder::new();
            let s = sig(&mut b, "Unit", "A");
            let root = b.effect_schema_tree("E", &[("put", s)]);
            encode(&b.finish(root))
        };
        // (3) Different op SIGNATURE (same op name).
        let diff_sig = {
            let mut b = Builder::new();
            let s = sig(&mut b, "A", "Unit");
            let root = b.effect_schema_tree("E", &[("get", s)]);
            encode(&b.finish(root))
        };
        // (4) An ADDED op (larger op set).
        let extra_op = {
            let mut b = Builder::new();
            let s1 = sig(&mut b, "Unit", "A");
            let s2 = sig(&mut b, "A", "Unit");
            let root = b.effect_schema_tree("E", &[("get", s1), ("put", s2)]);
            encode(&b.finish(root))
        };
        assert_ne!(
            base, diff_name,
            "distinct effect name must perturb the schema bytes"
        );
        assert_ne!(
            base, diff_op,
            "distinct op name must perturb the schema bytes"
        );
        assert_ne!(
            base, diff_sig,
            "distinct op signature must perturb the schema bytes"
        );
        assert_ne!(base, extra_op, "an added op must perturb the schema bytes");
    }

    // ---- I2: encode_with_dict (honor-supplied-dict transport encoder) ----

    /// The round-trip IDENTITY that is I2's whole correctness story (design §7): encoding against a dict
    /// then decoding against the SAME dict yields the canonical arena, and the transport is
    /// identity-preserving (re-encoding the decoded result gives `encode(a)`).
    fn assert_transport_identity(a: &Arenas, dicts: &DictSet) {
        let bytes = encode_with_dict(a, dicts);
        let decoded = decode_with_dicts(&bytes, dicts).expect("transport round-trips");
        assert!(
            crate::canon::canonicalize(a).structurally_eq(&decoded),
            "decode_with_dicts(encode_with_dict(a,d),d) != canonicalize(a)"
        );
        assert_eq!(
            encode(&decoded),
            encode(a),
            "transport is not identity-preserving: encode(decoded) != encode(a)"
        );
    }

    #[test]
    fn encode_with_dict_round_trips_over_empty_matching_and_superset_dicts() {
        // A tree `(f (pair a b) (pair a b))` — the `(pair a b)` subtree repeats, so a dict containing it
        // exercises a real ref (twice). Matrix: empty dict (no refs → plain v1), matching dict (the exact
        // subtree), superset dict (extra unrelated nodes). All must satisfy the round-trip identity.
        let mut b = Builder::new();
        let mk_pair = |b: &mut Builder| {
            let p = b.name("pair");
            let x = b.name("a");
            let y = b.name("b");
            b.list(vec![p, x, y])
        };
        let f = b.name("f");
        let p1 = mk_pair(&mut b);
        let p2 = mk_pair(&mut b);
        let root = b.list(vec![f, p1, p2]);
        let a = b.finish(root);

        // The dict = a standalone `(pair a b)` arena; any 32-byte hash works for the test as long as
        // encode_with_dict + decode_with_dicts use the SAME DictSet.
        let mut pb = Builder::new();
        let pair_only = mk_pair(&mut pb);
        let pair_dict = pb.finish(pair_only);

        let empty = DictSet::new();
        let mut matching = DictSet::new();
        matching.insert(Hash([0x11u8; 32]), pair_dict.clone());
        let mut superset = DictSet::new();
        superset.insert(Hash([0x11u8; 32]), pair_dict.clone());
        let mut extrab = Builder::new();
        let extra = extrab.name("unrelated");
        superset.insert(Hash([0x22u8; 32]), extrab.finish(extra));

        assert_transport_identity(&a, &empty);
        assert_transport_identity(&a, &matching);
        assert_transport_identity(&a, &superset);

        // With the matching dict the encode is a `\x00\x02` TRANSPORT artifact carrying refs; with the
        // empty dict it FALLS BACK to plain canonical `\x00\x01` (byte-identical to `encode`). (Whether
        // the transport form is SMALLER depends on subtree size vs the 32-byte import-hash overhead — a
        // tiny `(pair a b)` doesn't beat a 32-byte hash, so size is NOT asserted here; the compaction WIN
        // is a large-subtree property, and correctness is the round-trip identity above, not the size.)
        let with = encode_with_dict(&a, &matching);
        let without = encode_with_dict(&a, &empty);
        assert_eq!(
            &with[..8],
            &TRANSPORT_HEADER,
            "a matched encode is a transport artifact"
        );
        assert_eq!(
            &without[..8],
            &SCHEMA_HEADER,
            "an unmatched encode falls back to canonical v1"
        );
        assert_eq!(
            without,
            encode(&a),
            "the no-match fallback is byte-identical to encode"
        );
        // The matched transport form carries exactly one import (the pair dict) — the ref set is minimal.
        assert!(
            with.windows(32).any(|w| w == [0x11u8; 32]),
            "the matched transport artifact imports the pair dict's hash"
        );
    }

    #[test]
    fn encode_with_dict_compacts_a_large_repeated_subtree() {
        // The compaction WIN is real when the referenced subtree exceeds the 32-byte hash overhead. Build
        // a tree with a LARGE repeated subtree (a deep list) referenced twice; the transport form (two
        // 32-byte-hash-amortized refs replacing two large inline subtrees) is strictly SMALLER than inline.
        let mut sub_b = Builder::new();
        let mut cur = sub_b.name("leaf");
        for i in 0..40 {
            let tag = sub_b.name(if i % 2 == 0 { "wrap-a" } else { "wrap-b" });
            cur = sub_b.list(vec![tag, cur]);
        }
        let big_sub = sub_b.finish(cur); // a ~40-deep chain — far more than 32 bytes inline

        let mut b = Builder::new();
        let f = b.name("f");
        // two copies of the big subtree under `f` — inline that's ~2× the chain; as refs it's 2 small refs
        let build_copy = |b: &mut Builder| {
            let mut c = b.name("leaf");
            for i in 0..40 {
                let tag = b.name(if i % 2 == 0 { "wrap-a" } else { "wrap-b" });
                c = b.list(vec![tag, c]);
            }
            c
        };
        let c1 = build_copy(&mut b);
        let c2 = build_copy(&mut b);
        let root = b.list(vec![f, c1, c2]);
        let a = b.finish(root);

        let mut dicts = DictSet::new();
        dicts.insert(Hash([0x33u8; 32]), big_sub);
        assert_transport_identity(&a, &dicts);
        let with = encode_with_dict(&a, &dicts);
        let without = encode(&a);
        assert_eq!(&with[..8], &TRANSPORT_HEADER);
        assert!(
            with.len() < without.len(),
            "a large repeated subtree must compact: transport {} vs inline {}",
            with.len(),
            without.len()
        );
    }

    #[test]
    fn encode_with_dict_identity_over_generated_arenas_and_dicts() {
        // Property sweep (design §7.5): random arenas × random dicts (built from subtrees of the arena,
        // so matches actually occur) all satisfy the transport round-trip identity + never panic. Uses
        // the crate's SplitMix64 house style.
        struct Rng(u64);
        impl Rng {
            fn next(&mut self) -> u64 {
                self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
                let mut z = self.0;
                z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
                z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
                z ^ (z >> 31)
            }
            fn below(&mut self, n: usize) -> usize {
                (self.next() % n as u64) as usize
            }
        }
        fn gen_tree(rng: &mut Rng, b: &mut Builder, depth: usize) -> StructId {
            let names = ["a", "b", "f", "g", "pair", "+"];
            if depth == 0 || rng.next().is_multiple_of(3) {
                return b.name(names[rng.below(names.len())]);
            }
            let k = 1 + rng.below(3);
            let kids: Vec<StructId> = (0..k).map(|_| gen_tree(rng, b, depth - 1)).collect();
            b.list(kids)
        }
        let mut rng = Rng(0x0d1c_7c0d_e5da_7abc);
        for _ in 0..500 {
            let mut b = Builder::new();
            let depth = 1 + rng.below(4);
            let root = gen_tree(&mut rng, &mut b, depth);
            let a = b.finish(root);
            let canon = crate::canon::canonicalize(&a).into_owned();

            // Build a dict from a random subtree of `a` (so refs occur), keyed by a hash derived from the
            // iteration (distinct per dict). Empty half the time to exercise the fallback path too.
            let mut dicts = DictSet::new();
            if rng.next() & 1 == 0 && !canon.structure.is_empty() {
                let node = rng.below(canon.structure.len());
                let sub = subtree_arena(&canon, StructId(node as u32));
                let mut h = [0u8; 32];
                let seed = rng.next();
                h[..8].copy_from_slice(&seed.to_le_bytes());
                dicts.insert(Hash(h), sub);
            }
            assert_transport_identity(&a, &dicts);
        }
    }

    #[test]
    fn encode_with_dict_skips_a_cyclic_dict_and_terminates() {
        // #2093 review finding 2 (encode side): encode_with_dict walks each dict via subtree_arena to
        // build its match table, so a CYCLIC imported dict would diverge there. It must SKIP a cyclic dict
        // (encode is infallible) and still produce a valid uncompacted artifact — not hang.
        let cyclic = Arenas {
            leaves: vec![Leaf::Name("x".into())],
            structure: vec![Struct::List(vec![StructId(0)]), Struct::Atom(LeafId(0))], // node 0 → itself
            root: StructId(1),
        };
        let mut dicts = DictSet::new();
        dicts.insert(Hash([0x77u8; 32]), cyclic);
        let mut b = Builder::new();
        let f = b.name("f");
        let g = b.name("g");
        let root = b.list(vec![f, g]);
        let a = b.finish(root);
        // Must terminate; the cyclic dict contributes no matches → plain canonical fallback (no ref).
        let bytes = encode_with_dict(&a, &dicts);
        assert_eq!(
            &bytes[..8],
            &SCHEMA_HEADER,
            "a cyclic dict yields no match → canonical v1 fallback"
        );
        assert_eq!(bytes, encode(&a), "fallback is byte-identical to encode");
        // dict_is_safe_to_walk correctly classifies the cyclic dict + an acyclic one.
        let cyclic2 = Arenas {
            leaves: vec![],
            structure: vec![Struct::List(vec![StructId(0)])],
            root: StructId(0),
        };
        assert!(
            !dict_is_safe_to_walk(&cyclic2),
            "self-cyclic node is not safe to walk"
        );
        assert!(dict_is_safe_to_walk(&a), "a genuine tree is safe to walk");
    }

    #[test]
    fn encode_with_dict_skips_a_dict_with_an_out_of_range_child_and_does_not_panic() {
        // #2109 review finding (3rd untrusted-input layer): a dict whose structure has an OUT-OF-RANGE
        // child id is acyclic but UNWALKABLE — encode_with_dict's subtree_arena indexes dict.structure via
        // Arenas::get, which panics on a bad id. dict_is_safe_to_walk must reject it (return false → skip),
        // so encode never indexes it. DictSet::insert doesn't validate, so such a dict genuinely reaches here.
        let bad = Arenas {
            leaves: vec![Leaf::Name("x".into())],
            // node 0 = List[1, 9]: child 9 is past the 2-node structure.
            structure: vec![
                Struct::List(vec![StructId(1), StructId(9)]),
                Struct::Atom(LeafId(0)),
            ],
            root: StructId(0),
        };
        assert!(
            !dict_is_safe_to_walk(&bad),
            "an out-of-range child id must make the dict unsafe to walk"
        );
        let mut dicts = DictSet::new();
        dicts.insert(Hash([0x55u8; 32]), bad);
        let mut b = Builder::new();
        let f = b.name("f");
        let g = b.name("g");
        let root = b.list(vec![f, g]);
        let a = b.finish(root);
        // Must NOT panic; the bad dict is skipped → plain canonical fallback.
        let bytes = encode_with_dict(&a, &dicts);
        assert_eq!(
            bytes,
            encode(&a),
            "a bad dict yields no match → canonical v1 fallback"
        );
    }

    #[test]
    fn encode_with_dict_skips_a_dict_with_an_out_of_range_leaf_id_and_does_not_panic() {
        // #2121 review residual (leaf side of the #2109 out-of-range fix): the DFS only inspected List
        // children, so a Struct::Atom(LeafId) with an OUT-OF-RANGE leaf id slipped past — and
        // subtree_arena copies an Atom's leaf via Arenas::leaf (&self.leaves[i]), which PANICS on a bad
        // leaf id, same class as the structure-child gap. dict_is_safe_to_walk must reject it too.
        let bad_leaf = Arenas {
            leaves: vec![Leaf::Name("x".into())], // one leaf: only id 0 is valid
            // node 0 = Atom(leaf 7): leaf id 7 is past the 1-leaf pool.
            structure: vec![Struct::Atom(LeafId(7))],
            root: StructId(0),
        };
        assert!(
            !dict_is_safe_to_walk(&bad_leaf),
            "an out-of-range leaf id must make the dict unsafe to walk"
        );
        let mut dicts = DictSet::new();
        dicts.insert(Hash([0x56u8; 32]), bad_leaf);
        let mut b = Builder::new();
        let f = b.name("f");
        let g = b.name("g");
        let root = b.list(vec![f, g]);
        let a = b.finish(root);
        // Must NOT panic; the bad-leaf dict is skipped → plain canonical fallback.
        let bytes = encode_with_dict(&a, &dicts);
        assert_eq!(
            bytes,
            encode(&a),
            "a bad-leaf dict yields no match → canonical v1 fallback"
        );
    }

    #[test]
    fn encode_with_dict_skips_only_the_bad_dict_and_still_compacts_against_a_good_one() {
        // The bad/cyclic-dict skip is PER-DICT (`continue`), NOT a global bail: one unsafe dict in the set
        // must not disable compaction against the OTHER, sound dicts. Pin that resilience so a future
        // refactor (e.g. hoisting the acyclic check to a whole-set validate) can't silently regress it.
        // Set = { a self-cyclic dict (skipped) , a sound `(pair a b)` dict (used) }; the input contains
        // `(pair a b)` → it MUST still emit a dict-ref to the good dict (transport header, not the v1 one).
        let cyclic = Arenas {
            leaves: vec![Leaf::Name("x".into())],
            structure: vec![Struct::List(vec![StructId(0)])], // node 0 → itself
            root: StructId(0),
        };
        let mut gb = Builder::new();
        let p = gb.name("pair");
        let good_a = gb.name("a");
        let good_b = gb.name("b");
        let groot = gb.list(vec![p, good_a, good_b]);
        let good = gb.finish(groot);

        let mut dicts = DictSet::new();
        dicts.insert(Hash([0x01u8; 32]), cyclic);
        dicts.insert(Hash([0x02u8; 32]), good);

        // input = `(f (pair a b))` — the (pair a b) subtree matches the good dict's root.
        let mut b = Builder::new();
        let f = b.name("f");
        let ip = b.name("pair");
        let ia = b.name("a");
        let ib = b.name("b");
        let inner = b.list(vec![ip, ia, ib]);
        let root = b.list(vec![f, inner]);
        let a = b.finish(root);

        let bytes = encode_with_dict(&a, &dicts);
        assert_eq!(
            &bytes[..8],
            &TRANSPORT_HEADER,
            "the sound dict must still be used for compaction despite the cyclic one"
        );
        // Parse the import SECTION precisely (not a whole-blob 32-byte scan, which could match leaf-payload
        // bytes): the imports must be EXACTLY the good dict's hash — the skipped cyclic one is never listed.
        assert_eq!(
            parse_transport_imports(&bytes),
            vec![[0x02u8; 32]],
            "imports must be exactly the good dict (0x02); the cyclic dict (0x01) must be absent"
        );
        // And it round-trips back to the inline arena through decode_with_dicts.
        let got = decode_with_dicts(&bytes, &dicts).expect("compacted transport round-trips");
        assert!(got.structurally_eq(&a), "round-trip changed the arena");
    }

    #[test]
    fn encode_with_dict_is_deterministic_and_min_tie_breaks() {
        // #2093 review finding 1: identical (arena, DictSet) contents must yield identical transport bytes
        // run-to-run. Build TWO dicts that each contain the SAME importable subtree `(pair a b)` under
        // DIFFERENT hashes; the emitted ref must deterministically pick the SMALLEST (hash, node), so the
        // bytes don't vary with HashMap iteration order.
        let mk_pair_dict = || {
            let mut pb = Builder::new();
            let p = pb.name("pair");
            let x = pb.name("a");
            let y = pb.name("b");
            let r = pb.list(vec![p, x, y]);
            pb.finish(r)
        };
        let lo = Hash([0x01u8; 32]);
        let hi = Hash([0x02u8; 32]);
        let mut dicts = DictSet::new();
        dicts.insert(hi, mk_pair_dict());
        dicts.insert(lo, mk_pair_dict());

        let mut b = Builder::new();
        let f = b.name("f");
        let pp = b.name("pair");
        let pa = b.name("a");
        let pb2 = b.name("b");
        let pair = b.list(vec![pp, pa, pb2]);
        let root = b.list(vec![f, pair]);
        let a = b.finish(root);

        // Encode several times — identical bytes each time (determinism), and the import is the SMALLER
        // hash (0x01…), not whichever HashMap happened to yield first.
        let first = encode_with_dict(&a, &dicts);
        for _ in 0..5 {
            assert_eq!(
                encode_with_dict(&a, &dicts),
                first,
                "transport bytes must be deterministic"
            );
        }
        assert!(
            first.windows(32).any(|w| w == [0x01u8; 32]),
            "the min hash (0x01) must be the imported one (deterministic tie-break)"
        );
        assert!(
            !first.windows(32).any(|w| w == [0x02u8; 32]),
            "the larger hash (0x02) must NOT be imported when a smaller ties"
        );
        // And it still round-trips.
        assert_transport_identity(&a, &dicts);
    }

    #[test]
    fn a_dict_ref_into_a_cyclic_dict_subtree_is_not_a_tree_not_a_hang() {
        // DoS regression (#2086 review finding 1): a DictSet is caller-supplied and a TAG_DICT_REF can
        // target ANY node_id, including an UNREACHABLE dict node that `decode`'s reachability guard never
        // saw. If that node's subtree CYCLES, an unguarded graft loops forever. Build a dict arena whose
        // structure has a self-cyclic node (structure[0] = List[0] → itself) plus a real root, reference
        // node 0, and assert decode_with_dicts returns NotATree (fast) rather than hanging.
        let cyclic_dict = Arenas {
            leaves: vec![Leaf::Name("x".into())],
            // node 0: List[0] (points at itself — a cycle, unreachable from the real root node 1)
            // node 1: Atom(x) — the dict's declared root (a valid tree on its own)
            structure: vec![Struct::List(vec![StructId(0)]), Struct::Atom(LeafId(0))],
            root: StructId(1),
        };
        let mut dicts = DictSet::new();
        dicts.insert(Hash([0x55u8; 32]), cyclic_dict);
        // transport: (f <ref to dict node 0, the cyclic one>) — leaf[0]=f, [Atom f, DictRef{0,0}, List[0,1]], root 2
        let bytes = build_transport(
            &[[0x55u8; 32]],
            &[Leaf::Name("f".into())],
            &[
                (TAG_ATOM, vec![0]),
                (TAG_DICT_REF, vec![0, 0]),
                (TAG_LIST, vec![0, 1]),
            ],
            2,
        );
        assert_eq!(
            decode_with_dicts(&bytes, &dicts),
            Err(DecodeError::NotATree),
            "a dict-ref into a cyclic dict subtree must be NotATree, never a hang"
        );
    }

    #[test]
    fn a_transport_artifact_whose_own_structure_cycles_is_not_a_tree() {
        // The tree guard still applies on the transport plane: a `\x00\x02` whose own List ids form a
        // cycle (node 0 → node 0) must be refused, not diverge. (No dict-ref needed — this is the
        // transport structure's own referential hazard.)
        let bytes = build_transport(&[], &[], &[(TAG_LIST, vec![0])], 0); // node 0 = List[0] → itself
        assert_eq!(
            decode_with_dicts(&bytes, &DictSet::new()),
            Err(DecodeError::NotATree),
            "a self-cyclic transport structure must be NotATree"
        );
    }

    #[test]
    fn dict_idx_routes_to_the_correct_import_among_several() {
        // Every OTHER transport test imports ≤ 1 dict, so `dict_idx` is always 0 — a bug that grafted the
        // WRONG import (a transposed import list, an off-by-one in dict_idx → &imports[]) would pass them
        // all. Pin the routing: TWO distinct dicts, a ref into EACH plus a SECOND ref reusing dict 0, so
        // the result witnesses that dict_idx `i` grafts import `i` (not some other) AND that one dict can
        // be referenced twice. dict 0 = `(A1 A2)`, dict 1 = `(B1 B2 B3)`; transport = a 3-elem list
        // `<ref d0> <ref d1> <ref d0>` → `((A1 A2) (B1 B2 B3) (A1 A2))`.
        let mut ab = Builder::new();
        let a1 = ab.name("A1");
        let a2 = ab.name("A2");
        let a_root = ab.list(vec![a1, a2]);
        let dict_a = ab.finish(a_root);
        let a_root_id = dict_a.root.0 as u64;

        let mut bb = Builder::new();
        let b1 = bb.name("B1");
        let b2 = bb.name("B2");
        let b3 = bb.name("B3");
        let b_root = bb.list(vec![b1, b2, b3]);
        let dict_b = bb.finish(b_root);
        let b_root_id = dict_b.root.0 as u64;

        // Two DISTINCT hashes; the import list order fixes their dict_idx (A → 0, B → 1).
        let ha = [0x11u8; 32];
        let hb = [0x22u8; 32];
        let mut dicts = DictSet::new();
        dicts.insert(Hash(ha), dict_a);
        dicts.insert(Hash(hb), dict_b);

        // structure: [DictRef{0,a_root}, DictRef{1,b_root}, DictRef{0,a_root}, List[0,1,2]]; root = 3.
        let bytes = build_transport(
            &[ha, hb],
            &[],
            &[
                (TAG_DICT_REF, vec![0, a_root_id]),
                (TAG_DICT_REF, vec![1, b_root_id]),
                (TAG_DICT_REF, vec![0, a_root_id]),
                (TAG_LIST, vec![0, 1, 2]),
            ],
            3,
        );
        let got = decode_with_dicts(&bytes, &dicts).expect("multi-import refs resolve");

        // Expected inline: `((A1 A2) (B1 B2 B3) (A1 A2))`.
        let mut eb = Builder::new();
        let ea1 = eb.name("A1");
        let ea2 = eb.name("A2");
        let ea = eb.list(vec![ea1, ea2]);
        let eb1 = eb.name("B1");
        let eb2 = eb.name("B2");
        let eb3 = eb.name("B3");
        let ebl = eb.list(vec![eb1, eb2, eb3]);
        let ea1b = eb.name("A1");
        let ea2b = eb.name("A2");
        let ea_again = eb.list(vec![ea1b, ea2b]);
        let eroot = eb.list(vec![ea, ebl, ea_again]);
        let expected = eb.finish(eroot);
        assert!(
            got.structurally_eq(&expected),
            "multi-import graft misrouted: got {got:?}"
        );
        // And it re-encodes to its canonical identity + round-trips.
        assert!(decode(&encode(&got)).unwrap().structurally_eq(&expected));
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
