//! General component-model DEFINED-TYPE emission for arbitrary WIT value types ([`WitType`]).
//!
//! Step W3 of general WIT-bindings emission: a target world's typed boundary (its `record`s, `variant`s,
//! `enum`s, `result`s, …) becomes a table of component-model defined types that the component's function
//! types and canonical `lift`/`lower` reference by index. The component model requires every COMPOUND value
//! type to be its own indexed defined type (a record field or list element that is itself a compound is a
//! type *reference*, not inlined), so this module (a) flattens a [`WitType`] into an index-ordered
//! [`CDef`] table, children-first, and (b) hand-lays each defined type's bytes.
//!
//! As everywhere in this backend, the bytes are laid by hand — `wasm-encoder` is the tests-only byte
//! ORACLE, never in the compile path (`envelope.rs` header). The component-model structural tags
//! (`0x72` record, `0x71` variant, …) are not exposed as encoder constants, so every shape here is pinned
//! byte-for-byte against `wasm-encoder`'s `ComponentDefinedType` in the tests below.

use super::encode::{sleb128, uleb_bytes, uleb128};
use super::wasm_abi;
use crate::wit_world::WitType;

/// Component `char` — `wasm_abi` has no `COMP_CHAR` (its table skips `0x74`, the gap between `COMP_F64`
/// `0x75` and `COMP_STRING` `0x73`), so the one primitive byte named only here.
const COMP_CHAR: u8 = 0x74;

// Component-model defined-type tags (the leading byte of a `defvaltype` entry). Not exposed by
// `wasm-encoder`; pinned against its output by the oracle tests.
const TAG_RECORD: u8 = 0x72;
const TAG_VARIANT: u8 = 0x71;
const TAG_LIST: u8 = 0x70;
const TAG_TUPLE: u8 = 0x6f;
const TAG_FLAGS: u8 = 0x6e;
const TAG_ENUM: u8 = 0x6d;
const TAG_OPTION: u8 = 0x6b;
const TAG_RESULT: u8 = 0x6a;

/// How a WIT value type is referenced from a field / element / case payload: an inline PRIMITIVE valtype
/// byte, or an INDEX into the component type section (a compound is always its own indexed defined type).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CRef {
    /// A primitive component valtype, encoded inline as its single byte (`COMP_U8`, `COMP_STRING`, …).
    Prim(u8),
    /// A reference to the defined type at this component-type-section index.
    Idx(u32),
}

/// A resolved component-model defined type — one compound kind with its children already resolved to
/// [`CRef`]s (a primitive inline, a nested compound as an earlier index). A table's entry order IS its
/// type-section index order, so every [`CRef::Idx`] a `CDef` holds points at an earlier table entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CDef {
    /// `record { name: T, … }` — fields in declaration order.
    Record(Vec<(String, CRef)>),
    /// `variant { case, case(T), … }` — a case's `None` payload is a bare case.
    Variant(Vec<(String, Option<CRef>)>),
    /// `enum { name, … }` — all cases bare.
    Enum(Vec<String>),
    /// `flags { name, … }`.
    Flags(Vec<String>),
    /// `tuple<T, …>`.
    Tuple(Vec<CRef>),
    /// `option<T>`.
    Option(CRef),
    /// `result<ok, err>` — either arm may be absent.
    Result { ok: Option<CRef>, err: Option<CRef> },
    /// `list<T>`.
    List(CRef),
}

/// The primitive component valtype byte for a scalar [`WitType`], else `None` (a compound needs its own
/// indexed defined type; `Unit` is not a value type).
fn prim_byte(t: &WitType) -> Option<u8> {
    use wasm_abi::*;
    Some(match t {
        WitType::Bool => COMP_BOOL,
        WitType::S8 => COMP_S8,
        WitType::U8 => COMP_U8,
        WitType::S16 => COMP_S16,
        WitType::U16 => COMP_U16,
        WitType::S32 => COMP_S32,
        WitType::U32 => COMP_U32,
        WitType::S64 => COMP_S64,
        WitType::U64 => COMP_U64,
        WitType::F32 => COMP_F32,
        WitType::F64 => COMP_F64,
        WitType::Char => COMP_CHAR,
        WitType::String => COMP_STRING,
        _ => return None,
    })
}

/// Flatten `t` into `table` (appending, children-first, any compound defined types it needs) and return how
/// to reference it. `None` when `t` is not a value type — `Unit` (a function's "no result", handled at the
/// function-type level, not as a valtype) — or a not-yet-supported type (a resource handle).
pub fn add_wit_type(t: &WitType, table: &mut Vec<CDef>) -> Option<CRef> {
    // A primitive is an inline valtype byte, never its own defined type.
    if let Some(b) = prim_byte(t) {
        return Some(CRef::Prim(b));
    }
    let def = match t {
        WitType::List(e) => CDef::List(add_wit_type(e, table)?),
        WitType::Record(fields) => {
            let mut out = Vec::with_capacity(fields.len());
            for (name, fty) in fields {
                out.push((name.clone(), add_wit_type(fty, table)?));
            }
            CDef::Record(out)
        }
        WitType::Tuple(elems) => {
            let mut out = Vec::with_capacity(elems.len());
            for e in elems {
                out.push(add_wit_type(e, table)?);
            }
            CDef::Tuple(out)
        }
        WitType::Option(e) => CDef::Option(add_wit_type(e, table)?),
        WitType::Variant(cases) => {
            let mut out = Vec::with_capacity(cases.len());
            for (name, payload) in cases {
                let r = match payload {
                    Some(p) => Some(add_wit_type(p, table)?),
                    None => None,
                };
                out.push((name.clone(), r));
            }
            CDef::Variant(out)
        }
        WitType::Enum(cases) => CDef::Enum(cases.clone()),
        WitType::Flags(names) => CDef::Flags(names.clone()),
        WitType::Result { ok, err } => {
            let ok = match ok {
                Some(o) => Some(add_wit_type(o, table)?),
                None => None,
            };
            let err = match err {
                Some(e) => Some(add_wit_type(e, table)?),
                None => None,
            };
            CDef::Result { ok, err }
        }
        // `Unit` is not a value type; the primitive variants are already returned above (unreachable here,
        // but the match must be total).
        WitType::Unit
        | WitType::Bool
        | WitType::U8
        | WitType::U16
        | WitType::U32
        | WitType::U64
        | WitType::S8
        | WitType::S16
        | WitType::S32
        | WitType::S64
        | WitType::Char
        | WitType::String
        | WitType::F32
        | WitType::F64 => return None,
    };
    table.push(def);
    Some(CRef::Idx((table.len() - 1) as u32))
}

/// Like [`add_wit_type`] but DEDUPING at every level via a `memo` of already-built types: a compound is built
/// by recursively deduping its children first, so two structurally equal (sub)types resolve to ONE table
/// index. Used by the interface-instance assembler, where an exported named type and a nested field of the
/// SAME structure must reference one shared index (or the instance export references a type it never exports —
/// "instance not valid to be used as export"). [`add_wit_type`] stays non-deduping so its per-call
/// byte-identity with the `wasm-encoder` oracle is preserved.
pub fn add_wit_type_deduped(
    t: &WitType,
    table: &mut Vec<CDef>,
    memo: &mut Vec<(WitType, CRef)>,
) -> Option<CRef> {
    if let Some(b) = prim_byte(t) {
        return Some(CRef::Prim(b));
    }
    if let Some((_, r)) = memo.iter().find(|(u, _)| u == t) {
        return Some(r.clone());
    }
    let def = match t {
        WitType::List(e) => CDef::List(add_wit_type_deduped(e, table, memo)?),
        WitType::Record(fields) => {
            let mut out = Vec::with_capacity(fields.len());
            for (name, fty) in fields {
                out.push((name.clone(), add_wit_type_deduped(fty, table, memo)?));
            }
            CDef::Record(out)
        }
        WitType::Tuple(elems) => {
            let mut out = Vec::with_capacity(elems.len());
            for e in elems {
                out.push(add_wit_type_deduped(e, table, memo)?);
            }
            CDef::Tuple(out)
        }
        WitType::Option(e) => CDef::Option(add_wit_type_deduped(e, table, memo)?),
        WitType::Variant(cases) => {
            let mut out = Vec::with_capacity(cases.len());
            for (name, payload) in cases {
                let r = match payload {
                    Some(p) => Some(add_wit_type_deduped(p, table, memo)?),
                    None => None,
                };
                out.push((name.clone(), r));
            }
            CDef::Variant(out)
        }
        WitType::Enum(cases) => CDef::Enum(cases.clone()),
        WitType::Flags(names) => CDef::Flags(names.clone()),
        WitType::Result { ok, err } => {
            let ok = match ok {
                Some(o) => Some(add_wit_type_deduped(o, table, memo)?),
                None => None,
            };
            let err = match err {
                Some(e) => Some(add_wit_type_deduped(e, table, memo)?),
                None => None,
            };
            CDef::Result { ok, err }
        }
        _ => return None,
    };
    // Reuse a structurally-equal entry (children already deduped, so equal subtrees ⇒ equal `CDef`s).
    let r = if let Some(i) = table.iter().position(|d| *d == def) {
        CRef::Idx(i as u32)
    } else {
        table.push(def);
        CRef::Idx((table.len() - 1) as u32)
    };
    memo.push((t.clone(), r.clone()));
    Some(r)
}

/// Encode one [`CRef`] as a component-model `valtype`: a primitive is its single byte; a type reference is
/// the (non-negative) index as a signed LEB128 — the encoding `wasm-encoder` uses for
/// `ComponentValType::Type` (`idx as i64`), disjoint from the primitives' `0x73..=0x7f` bytes.
fn encode_cref(r: &CRef, out: &mut Vec<u8>) {
    match r {
        CRef::Prim(b) => out.push(*b),
        CRef::Idx(i) => sleb128(*i as i64, out),
    }
}

/// Encode a `name` the way a component-model label is encoded: its length as ULEB128, then its bytes.
fn encode_name(name: &str, out: &mut Vec<u8>) {
    out.extend_from_slice(&uleb_bytes(name.len() as u64));
    out.extend_from_slice(name.as_bytes());
}

/// Encode an `option<valtype>` slot (a variant case payload / a result arm): `0x00` for absent, else `0x01`
/// then the referenced valtype.
fn encode_opt_cref(r: Option<&CRef>, out: &mut Vec<u8>) {
    match r {
        None => out.push(0x00),
        Some(r) => {
            out.push(0x01);
            encode_cref(r, out);
        }
    }
}

/// The component-model defined-type bytes for one [`CDef`] — a single component `type`-section entry
/// (the `defvaltype` production: the structural tag then the kind's payload).
pub fn emit_cdef(def: &CDef) -> Vec<u8> {
    let mut out = Vec::new();
    match def {
        CDef::Record(fields) => {
            out.push(TAG_RECORD);
            uleb128(fields.len() as u64, &mut out);
            for (name, r) in fields {
                encode_name(name, &mut out);
                encode_cref(r, &mut out);
            }
        }
        CDef::Variant(cases) => {
            out.push(TAG_VARIANT);
            uleb128(cases.len() as u64, &mut out);
            for (name, payload) in cases {
                encode_name(name, &mut out);
                encode_opt_cref(payload.as_ref(), &mut out);
                out.push(0x00); // refines: none (a case never refines another here)
            }
        }
        CDef::Enum(names) => {
            out.push(TAG_ENUM);
            uleb128(names.len() as u64, &mut out);
            for name in names {
                encode_name(name, &mut out);
            }
        }
        CDef::Flags(names) => {
            out.push(TAG_FLAGS);
            uleb128(names.len() as u64, &mut out);
            for name in names {
                encode_name(name, &mut out);
            }
        }
        CDef::Tuple(elems) => {
            out.push(TAG_TUPLE);
            uleb128(elems.len() as u64, &mut out);
            for r in elems {
                encode_cref(r, &mut out);
            }
        }
        CDef::Option(r) => {
            out.push(TAG_OPTION);
            encode_cref(r, &mut out);
        }
        CDef::Result { ok, err } => {
            out.push(TAG_RESULT);
            encode_opt_cref(ok.as_ref(), &mut out);
            encode_opt_cref(err.as_ref(), &mut out);
        }
        CDef::List(r) => {
            out.push(TAG_LIST);
            encode_cref(r, &mut out);
        }
    }
    out
}

/// The component TYPE-section body for a whole `table`: the entry count as ULEB128, then each defined
/// type's bytes in index order. (The caller frames this with the section id + length.)
pub fn emit_type_section_body(table: &[CDef]) -> Vec<u8> {
    let mut out = Vec::new();
    uleb128(table.len() as u64, &mut out);
    for def in table {
        out.extend_from_slice(&emit_cdef(def));
    }
    out
}

/// The component-model FUNCTYPE bytes for a boundary function — one component `type`-section entry:
/// `0x40 <params-vec> <result>`. Each param is `<namelen> <name> <valtype>`; the result is `0x00 <valtype>`
/// (one unnamed result) or `0x01 0x00` (the zero-named-results form = no result / `unit`). A param or result
/// valtype references a defined type by index (a compound) or inlines a primitive byte, via its [`CRef`] —
/// so a func carrying records/variants references the defined types laid earlier in the same section. This
/// is the general form of `envelope::comp_functype`, whose result was only ever a primitive or `list<u8>`.
pub fn emit_functype(params: &[(String, CRef)], result: Option<&CRef>) -> Vec<u8> {
    let mut out = vec![wasm_abi::COMP_FUNCTYPE_FORM];
    uleb128(params.len() as u64, &mut out);
    for (name, r) in params {
        encode_name(name, &mut out);
        encode_cref(r, &mut out);
    }
    match result {
        Some(r) => {
            out.push(0x00); // one unnamed result, inline
            encode_cref(r, &mut out);
        }
        None => out.extend_from_slice(&[0x01, 0x00]), // zero named results = no result
    }
    out
}

// ── Canonical-ABI FLATTENING (step W4a) ──────────────────────────────────────────────────────────────
// A component value type flattens to a sequence of CORE value types — the shape a lifted core function's
// signature takes when the value crosses the boundary in registers (before the spill-to-memory limits).
// This is the pure type-level half of lift/lower: it fixes the core func signature the lift/lower body
// (a later slice) reads/writes. The rules are the component-model canonical ABI's `flatten_type`.

/// A core wasm value type — the flattened form a component value crosses in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoreTy {
    I32,
    I64,
    F32,
    F64,
}

impl CoreTy {
    /// The core valtype byte for this type.
    pub fn core_byte(self) -> u8 {
        match self {
            CoreTy::I32 => wasm_abi::CORE_I32,
            CoreTy::I64 => wasm_abi::CORE_I64,
            CoreTy::F32 => wasm_abi::CORE_F32,
            CoreTy::F64 => wasm_abi::CORE_F64,
        }
    }
}

/// Canonical-ABI flat-count limits: a function's params flatten in-place up to 16 core values, else spill to
/// a single `i32` pointer to a tuple in linear memory; its result flattens in-place up to 1, else spills to
/// an `i32` pointer to the result in linear memory. A record/variant result therefore always spills.
pub const MAX_FLAT_PARAMS: usize = 16;
const MAX_FLAT_RESULTS: usize = 1;

/// The canonical-ABI join of two core types at one flattened position of a variant — the type both cases'
/// values can be reinterpreted through: equal types keep; an int/float of the same width joins to the int;
/// anything else widens to `i64` (the component-model `join`).
fn join(a: CoreTy, b: CoreTy) -> CoreTy {
    use CoreTy::*;
    if a == b {
        return a;
    }
    match (a, b) {
        (I32, F32) | (F32, I32) => I32,
        (I64, F64) | (F64, I64) => I64,
        _ => I64,
    }
}

/// Flatten a tagged type (`variant`/`enum`/`option`/`result`) whose cases carry the given optional payloads:
/// an `i32` discriminant, then the position-wise `join` of every case's flattened payload (a `None` case
/// contributes nothing). `enum` (all `None`) flattens to just the discriminant.
fn flatten_variant(case_payloads: &[Option<&WitType>]) -> Vec<CoreTy> {
    let mut flat: Vec<CoreTy> = Vec::new();
    for payload in case_payloads.iter().copied().flatten() {
        for (i, ct) in flatten(payload).into_iter().enumerate() {
            if i < flat.len() {
                flat[i] = join(flat[i], ct);
            } else {
                flat.push(ct);
            }
        }
    }
    let mut out = vec![CoreTy::I32];
    out.extend(flat);
    out
}

/// The core value types a component value of type `t` flattens to (canonical ABI `flatten_type`): scalars to
/// their core type (`u64`/`s64` → `i64`, floats keep, everything else `i32`), `string`/`list` to `(ptr, len)`
/// = `[i32, i32]`, `record`/`tuple` to the concatenation of their elements, and the tagged types via
/// [`flatten_variant`]. `Unit` (a function's "no result") flattens to nothing.
pub fn flatten(t: &WitType) -> Vec<CoreTy> {
    use CoreTy::*;
    match t {
        WitType::Bool
        | WitType::U8
        | WitType::U16
        | WitType::U32
        | WitType::S8
        | WitType::S16
        | WitType::S32
        | WitType::Char => vec![I32],
        WitType::U64 | WitType::S64 => vec![I64],
        WitType::F32 => vec![F32],
        WitType::F64 => vec![F64],
        // A string / list crosses as a `(ptr, len)` pair into linear memory.
        WitType::String | WitType::List(_) => vec![I32, I32],
        WitType::Record(fields) => fields.iter().flat_map(|(_, ft)| flatten(ft)).collect(),
        WitType::Tuple(elems) => elems.iter().flat_map(flatten).collect(),
        WitType::Enum(_) => vec![I32], // discriminant only, no payloads
        // ceil(n/32) i32s hold n flag bits (a real flags has >= 1 label).
        WitType::Flags(names) => vec![I32; names.len().div_ceil(32)],
        WitType::Option(inner) => flatten_variant(&[None, Some(inner.as_ref())]),
        WitType::Variant(cases) => {
            let payloads: Vec<Option<&WitType>> = cases.iter().map(|(_, p)| p.as_ref()).collect();
            flatten_variant(&payloads)
        }
        WitType::Result { ok, err } => flatten_variant(&[ok.as_deref(), err.as_deref()]),
        WitType::Unit => vec![],
    }
}

/// The CORE function signature a component function `(params) -> result` lifts to under `canon lift` (a guest
/// export): its `(core_params, core_results)`. Params flatten in order and spill to a single `i32` pointer
/// when over [`MAX_FLAT_PARAMS`]; the result flattens and spills to a single `i32` return pointer when over
/// [`MAX_FLAT_RESULTS`] (so a record/variant result is returned by pointer). This is the signature the
/// lift/lower body (a later slice) is generated against.
pub fn flatten_func_core_sig(
    params: &[WitType],
    result: Option<&WitType>,
) -> (Vec<CoreTy>, Vec<CoreTy>) {
    let pflat: Vec<CoreTy> = params.iter().flat_map(flatten).collect();
    let core_params = if pflat.len() > MAX_FLAT_PARAMS {
        vec![CoreTy::I32]
    } else {
        pflat
    };
    let rflat: Vec<CoreTy> = result.map(flatten).unwrap_or_default();
    let core_results = if rflat.len() > MAX_FLAT_RESULTS {
        vec![CoreTy::I32]
    } else {
        rflat
    };
    (core_params, core_results)
}

/// Whether a value type contains a `list`/`string` leaf anywhere — a leaf that crosses through linear
/// memory as `(ptr, len)`, so a func carrying it must lift/lower with the Memory+Realloc canon options.
fn ty_touches_memory(t: &WitType) -> bool {
    match t {
        WitType::List(_) | WitType::String => true,
        WitType::Record(fs) => fs.iter().any(|(_, ft)| ty_touches_memory(ft)),
        WitType::Tuple(es) => es.iter().any(ty_touches_memory),
        WitType::Option(inner) => ty_touches_memory(inner),
        WitType::Variant(cs) => cs
            .iter()
            .any(|(_, p)| p.as_ref().is_some_and(ty_touches_memory)),
        WitType::Result { ok, err } => {
            ok.as_deref().is_some_and(ty_touches_memory)
                || err.as_deref().is_some_and(ty_touches_memory)
        }
        _ => false,
    }
}

/// Whether a boundary function's `canon lift` needs the Memory + Realloc options — true iff its signature
/// touches linear memory: a `list`/`string` leaf anywhere, OR params that spill (over [`MAX_FLAT_PARAMS`]),
/// OR a result that spills (over [`MAX_FLAT_RESULTS`], returned by pointer). A pure fixed-scalar signature
/// needs neither (its args cross in registers) — the [`crate::backend::wasm::envelope`] assembler uses this
/// to pick the plain lift vs the Memory+Realloc lift and whether to alias the core's memory + realloc.
pub fn sig_needs_memory(params: &[WitType], result: Option<&WitType>) -> bool {
    if params.iter().any(ty_touches_memory) || result.is_some_and(ty_touches_memory) {
        return true;
    }
    let params_flat: usize = params.iter().map(|t| flatten(t).len()).sum();
    let result_flat: usize = result.map(|t| flatten(t).len()).unwrap_or(0);
    params_flat > MAX_FLAT_PARAMS || result_flat > MAX_FLAT_RESULTS
}

// ── Canonical-ABI MEMORY LAYOUT (step W4c-spill) ──────────────────────────────────────────────────────
// When a boundary value spills to linear memory — a param tuple over MAX_FLAT_PARAMS, or a record/variant
// RESULT (which always exceeds MAX_FLAT_RESULTS, returned by pointer) — it is laid out at the component-
// model canonical offsets. These compute the size / alignment / field offsets per the canonical ABI; they
// are the input to the result-layout writer and the spilled-value reader (a later slice generates the core
// stores/loads against them). Nothing else in the backend computes canonical memory layout (compounds
// otherwise cross as the value-form `list<u8>`), so this is new.

/// Round `x` up to a multiple of the power-of-two alignment `a`.
fn align_to(x: u32, a: u32) -> u32 {
    (x + a - 1) & !(a - 1)
}

/// The byte size of a variant discriminant holding `n_cases` cases — the smallest unsigned int that fits.
fn disc_size(n_cases: usize) -> u32 {
    if n_cases <= (1 << 8) {
        1
    } else if n_cases <= (1 << 16) {
        2
    } else {
        4
    }
}

/// Canonical size (bytes) of a `flags` of `n` labels: packed to the smallest uint, then `ceil(n/32)` i32s.
fn flags_size(n: usize) -> u32 {
    if n == 0 {
        0
    } else if n <= 8 {
        1
    } else if n <= 16 {
        2
    } else {
        4 * (n as u32).div_ceil(32)
    }
}

/// Canonical alignment of a `flags` of `n` labels.
fn flags_align(n: usize) -> u32 {
    if n <= 8 {
        1
    } else if n <= 16 {
        2
    } else {
        4
    }
}

/// The disc byte-size + the canonical byte-offset of the payload area for a tagged type with these case
/// payloads — the inputs the result-lower's variant writer needs (`disc_store` width + where each arm's
/// payload lands after the discriminant). The payload area sits at the disc size rounded up to the max
/// present-payload alignment (`(disc, payload_offset)`); a pure enum has `payload_offset == disc` (unused).
pub fn variant_disc_layout(case_payloads: &[Option<&WitType>]) -> (u32, u32) {
    let disc = disc_size(case_payloads.len());
    let max_payload_align = case_payloads
        .iter()
        .flatten()
        .map(|t| canonical_align(t))
        .max()
        .unwrap_or(1);
    (disc, align_to(disc, max_payload_align))
}

/// Canonical alignment of a tagged type given its cases' payloads: the max of the discriminant's alignment
/// and every present case payload's alignment.
fn variant_align(case_payloads: &[Option<&WitType>]) -> u32 {
    let disc = disc_size(case_payloads.len());
    let max_case = case_payloads
        .iter()
        .flatten()
        .map(|t| canonical_align(t))
        .max()
        .unwrap_or(1);
    disc.max(max_case)
}

/// Canonical size of a tagged type: the discriminant, padded to the cases' max alignment, plus the max case
/// payload size, padded to the whole variant's alignment.
fn variant_size(case_payloads: &[Option<&WitType>]) -> u32 {
    let max_case_align = case_payloads
        .iter()
        .flatten()
        .map(|t| canonical_align(t))
        .max()
        .unwrap_or(1);
    let max_case_size = case_payloads
        .iter()
        .flatten()
        .map(|t| canonical_size(t))
        .max()
        .unwrap_or(0);
    let mut s = align_to(disc_size(case_payloads.len()), max_case_align);
    s += max_case_size;
    align_to(s, variant_align(case_payloads))
}

/// The canonical-ABI alignment (bytes) of a WIT value type. `Unit` (not a value type) is 1.
pub fn canonical_align(t: &WitType) -> u32 {
    match t {
        WitType::Bool | WitType::U8 | WitType::S8 => 1,
        WitType::U16 | WitType::S16 => 2,
        WitType::U32 | WitType::S32 | WitType::F32 | WitType::Char => 4,
        WitType::U64 | WitType::S64 | WitType::F64 => 8,
        // A list/string crosses as (ptr: i32, len: i32) → align 4.
        WitType::String | WitType::List(_) => 4,
        WitType::Record(fields) => fields
            .iter()
            .map(|(_, t)| canonical_align(t))
            .max()
            .unwrap_or(1),
        WitType::Tuple(elems) => elems.iter().map(canonical_align).max().unwrap_or(1),
        WitType::Enum(cases) => disc_size(cases.len()),
        WitType::Flags(names) => flags_align(names.len()),
        WitType::Variant(cases) => {
            variant_align(&cases.iter().map(|(_, p)| p.as_ref()).collect::<Vec<_>>())
        }
        WitType::Option(inner) => variant_align(&[None, Some(inner.as_ref())]),
        WitType::Result { ok, err } => variant_align(&[ok.as_deref(), err.as_deref()]),
        WitType::Unit => 1,
    }
}

/// The canonical-ABI size (bytes) of a WIT value type. `Unit` is 0.
pub fn canonical_size(t: &WitType) -> u32 {
    match t {
        WitType::Bool | WitType::U8 | WitType::S8 => 1,
        WitType::U16 | WitType::S16 => 2,
        WitType::U32 | WitType::S32 | WitType::F32 | WitType::Char => 4,
        WitType::U64 | WitType::S64 | WitType::F64 => 8,
        WitType::String | WitType::List(_) => 8, // (ptr, len)
        WitType::Record(fields) => {
            let mut s = 0;
            for (_, ft) in fields {
                s = align_to(s, canonical_align(ft));
                s += canonical_size(ft);
            }
            align_to(s, canonical_align(t))
        }
        WitType::Tuple(elems) => {
            let mut s = 0;
            for et in elems {
                s = align_to(s, canonical_align(et));
                s += canonical_size(et);
            }
            align_to(s, canonical_align(t))
        }
        WitType::Enum(cases) => disc_size(cases.len()),
        WitType::Flags(names) => flags_size(names.len()),
        WitType::Variant(cases) => {
            variant_size(&cases.iter().map(|(_, p)| p.as_ref()).collect::<Vec<_>>())
        }
        WitType::Option(inner) => variant_size(&[None, Some(inner.as_ref())]),
        WitType::Result { ok, err } => variant_size(&[ok.as_deref(), err.as_deref()]),
        WitType::Unit => 0,
    }
}

/// The canonical byte offset of each field of a `record`/`tuple`, in declaration order — each field placed
/// at the running size rounded up to its alignment. The paired input to [`canonical_size`] for laying a
/// record result into (or reading a spilled record out of) linear memory.
pub fn record_field_offsets(field_types: &[WitType]) -> Vec<u32> {
    let mut offsets = Vec::with_capacity(field_types.len());
    let mut running = 0;
    for ft in field_types {
        running = align_to(running, canonical_align(ft));
        offsets.push(running);
        running += canonical_size(ft);
    }
    offsets
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── the independent wasm-encoder oracle ──────────────────────────────────────────────────────────
    // Build the SAME component type section with `wasm-encoder` and diff bytes. The oracle flattener
    // mirrors `add_wit_type` (children-first), so its type indices line up with ours.

    fn oracle_prim(t: &WitType) -> Option<wasm_encoder::PrimitiveValType> {
        use wasm_encoder::PrimitiveValType as P;
        Some(match t {
            WitType::Bool => P::Bool,
            WitType::S8 => P::S8,
            WitType::U8 => P::U8,
            WitType::S16 => P::S16,
            WitType::U16 => P::U16,
            WitType::S32 => P::S32,
            WitType::U32 => P::U32,
            WitType::S64 => P::S64,
            WitType::U64 => P::U64,
            WitType::F32 => P::F32,
            WitType::F64 => P::F64,
            WitType::Char => P::Char,
            WitType::String => P::String,
            _ => return None,
        })
    }

    /// Flatten `t` into `ts` children-first (mirroring [`add_wit_type`]), returning its `ComponentValType`.
    /// `next` tracks the index of the next-appended defined type.
    fn oracle_add(
        ts: &mut wasm_encoder::ComponentTypeSection,
        t: &WitType,
        next: &mut u32,
    ) -> wasm_encoder::ComponentValType {
        use wasm_encoder::ComponentValType as V;
        if let Some(p) = oracle_prim(t) {
            return V::Primitive(p);
        }
        match t {
            WitType::List(e) => {
                let er = oracle_add(ts, e, next);
                ts.defined_type().list(er);
            }
            WitType::Record(fields) => {
                let fs: Vec<(String, V)> = fields
                    .iter()
                    .map(|(n, f)| (n.clone(), oracle_add(ts, f, next)))
                    .collect();
                ts.defined_type()
                    .record(fs.iter().map(|(n, v)| (n.as_str(), *v)));
            }
            WitType::Tuple(elems) => {
                let es: Vec<V> = elems.iter().map(|e| oracle_add(ts, e, next)).collect();
                ts.defined_type().tuple(es);
            }
            WitType::Option(e) => {
                let er = oracle_add(ts, e, next);
                ts.defined_type().option(er);
            }
            WitType::Variant(cases) => {
                let cs: Vec<(String, Option<V>)> = cases
                    .iter()
                    .map(|(n, p)| (n.clone(), p.as_ref().map(|p| oracle_add(ts, p, next))))
                    .collect();
                ts.defined_type()
                    .variant(cs.iter().map(|(n, v)| (n.as_str(), *v, None)));
            }
            WitType::Enum(names) => {
                ts.defined_type()
                    .enum_type(names.iter().map(|s| s.as_str()));
            }
            WitType::Flags(names) => {
                ts.defined_type().flags(names.iter().map(|s| s.as_str()));
            }
            WitType::Result { ok, err } => {
                let okr = ok.as_ref().map(|o| oracle_add(ts, o, next));
                let errr = err.as_ref().map(|e| oracle_add(ts, e, next));
                ts.defined_type().result(okr, errr);
            }
            _ => unreachable!("non-value type reached the oracle flattener"),
        }
        let idx = *next;
        *next += 1;
        V::Type(idx)
    }

    /// Assert our hand-laid type section for `tops` is byte-identical to `wasm-encoder`'s. Framing both as a
    /// bare component (magic + one type section) isolates the type-section bytes exactly.
    fn assert_byte_identical(tops: &[WitType]) {
        // ours
        let mut table = Vec::new();
        for t in tops {
            add_wit_type(t, &mut table).expect("value type");
        }
        let body = emit_type_section_body(&table);
        let mut mine = wasm_abi::COMPONENT_MAGIC.to_vec();
        mine.push(wasm_abi::COMP_SEC_TYPE);
        mine.extend_from_slice(&uleb_bytes(body.len() as u64));
        mine.extend_from_slice(&body);

        // oracle
        let mut ts = wasm_encoder::ComponentTypeSection::new();
        let mut next = 0u32;
        for t in tops {
            oracle_add(&mut ts, t, &mut next);
        }
        let mut c = wasm_encoder::Component::new();
        c.section(&ts);
        let oracle = c.finish();

        assert_eq!(mine, oracle, "type section bytes differ for {tops:?}");
    }

    fn rec(fields: &[(&str, WitType)]) -> WitType {
        WitType::Record(
            fields
                .iter()
                .map(|(n, t)| (n.to_string(), t.clone()))
                .collect(),
        )
    }

    #[test]
    fn record_of_scalars_matches_oracle() {
        assert_byte_identical(&[rec(&[
            ("family", WitType::String),
            ("version", WitType::U32),
        ])]);
    }

    #[test]
    fn list_and_tuple_and_option_match_oracle() {
        assert_byte_identical(&[WitType::List(Box::new(WitType::U8))]);
        assert_byte_identical(&[WitType::Tuple(vec![WitType::U32, WitType::String])]);
        assert_byte_identical(&[WitType::Option(Box::new(WitType::S64))]);
    }

    #[test]
    fn enum_and_flags_match_oracle() {
        assert_byte_identical(&[WitType::Enum(vec![
            "timeout".into(),
            "missing-handler".into(),
            "schema-violation".into(),
        ])]);
        assert_byte_identical(&[WitType::Flags(vec!["read".into(), "write".into()])]);
    }

    #[test]
    fn variant_with_payload_and_bare_cases_matches_oracle() {
        // outcome-shaped: a bare case + a payload case whose payload is a nested record → the record is an
        // earlier type index the variant references.
        assert_byte_identical(&[WitType::Variant(vec![
            ("continue".into(), None),
            ("close".into(), Some(rec(&[("code", WitType::U32)]))),
        ])]);
    }

    #[test]
    fn result_both_arms_and_absent_ok_match_oracle() {
        // result<list<u8>, enum> — both arms present, ok is a nested list.
        assert_byte_identical(&[WitType::Result {
            ok: Some(Box::new(WitType::List(Box::new(WitType::U8)))),
            err: Some(Box::new(WitType::Enum(vec!["timeout".into()]))),
        }]);
        // result<_, u32> — absent ok arm.
        assert_byte_identical(&[WitType::Result {
            ok: None,
            err: Some(Box::new(WitType::U32)),
        }]);
    }

    #[test]
    fn a_record_of_records_shares_indices_left_to_right() {
        // Nested compounds each become their own indexed type, children before parents; the reducer
        // `message`-ish shape (a record whose fields are a record and a byte-list) exercises the ordering.
        assert_byte_identical(&[rec(&[
            (
                "sender",
                rec(&[("reducer", WitType::List(Box::new(WitType::U8)))]),
            ),
            ("payload", WitType::List(Box::new(WitType::U8))),
        ])]);
    }

    #[test]
    fn add_wit_type_declines_unit_and_orders_children_first() {
        // Unit is not a value type.
        let mut table = Vec::new();
        assert_eq!(add_wit_type(&WitType::Unit, &mut table), None);
        assert!(table.is_empty());

        // A list<record> flattens to [record, list] — the record (index 0) before the list that refs it.
        let mut table = Vec::new();
        let r = add_wit_type(
            &WitType::List(Box::new(rec(&[("n", WitType::U8)]))),
            &mut table,
        );
        assert_eq!(r, Some(CRef::Idx(1)));
        assert_eq!(
            table,
            vec![
                CDef::Record(vec![("n".to_string(), CRef::Prim(wasm_abi::COMP_U8))]),
                CDef::List(CRef::Idx(0)),
            ]
        );
    }

    /// Assert our type section for a func signature — the param/result defined types, then the functype
    /// referencing them — is byte-identical to wasm-encoder's. Flattens params-then-result (both sides in
    /// the same order, so the defined-type indices line up), the functype being the section's last entry.
    fn assert_functype_identical(params: &[(&str, WitType)], result: Option<WitType>) {
        // ours
        let mut table = Vec::new();
        let param_refs: Vec<(String, CRef)> = params
            .iter()
            .map(|(n, t)| {
                (
                    n.to_string(),
                    add_wit_type(t, &mut table).expect("value type"),
                )
            })
            .collect();
        let result_ref = result
            .as_ref()
            .map(|t| add_wit_type(t, &mut table).expect("value type"));
        let mut body = Vec::new();
        uleb128(table.len() as u64 + 1, &mut body); // the defined types + the one functype entry
        for def in &table {
            body.extend_from_slice(&emit_cdef(def));
        }
        body.extend_from_slice(&emit_functype(&param_refs, result_ref.as_ref()));
        let mut mine = wasm_abi::COMPONENT_MAGIC.to_vec();
        mine.push(wasm_abi::COMP_SEC_TYPE);
        mine.extend_from_slice(&uleb_bytes(body.len() as u64));
        mine.extend_from_slice(&body);

        // oracle
        let mut ts = wasm_encoder::ComponentTypeSection::new();
        let mut next = 0u32;
        let oracle_params: Vec<(String, wasm_encoder::ComponentValType)> = params
            .iter()
            .map(|(n, t)| (n.to_string(), oracle_add(&mut ts, t, &mut next)))
            .collect();
        let oracle_result = result.as_ref().map(|t| oracle_add(&mut ts, t, &mut next));
        ts.function()
            .params(oracle_params.iter().map(|(n, v)| (n.as_str(), *v)))
            .result(oracle_result);
        let mut c = wasm_encoder::Component::new();
        c.section(&ts);
        let oracle = c.finish();

        assert_eq!(
            mine, oracle,
            "functype section differs for params={params:?} result={result:?}"
        );
    }

    #[test]
    fn functype_with_a_record_param_and_scalar_result_matches_oracle() {
        // on-message-ish: (msg: record{contract, token}) -> u32 — a defined-type param, an inline result.
        assert_functype_identical(
            &[(
                "msg",
                rec(&[
                    ("contract", WitType::List(Box::new(WitType::U8))),
                    ("token", WitType::List(Box::new(WitType::U8))),
                ]),
            )],
            Some(WitType::U32),
        );
    }

    #[test]
    fn functype_with_no_result_is_the_void_form() {
        // state.put-ish: (key: list<u8>, value: list<u8>) -> () — the zero-named-results (void) form.
        assert_functype_identical(
            &[
                ("key", WitType::List(Box::new(WitType::U8))),
                ("value", WitType::List(Box::new(WitType::U8))),
            ],
            None,
        );
    }

    #[test]
    fn functype_with_a_compound_result_references_it_by_index() {
        // () -> step-ish record result — the result is a defined type referenced by index, not inline.
        assert_functype_identical(&[], Some(rec(&[("keep-going", WitType::Bool)])));
    }

    // ── canonical-ABI flattening (W4a) ────────────────────────────────────────────────────────────────
    use CoreTy::{F64, I32, I64};

    #[test]
    fn flatten_scalars_and_list_and_string() {
        assert_eq!(flatten(&WitType::Bool), vec![I32]);
        assert_eq!(flatten(&WitType::U32), vec![I32]);
        assert_eq!(flatten(&WitType::U64), vec![I64]);
        assert_eq!(flatten(&WitType::S64), vec![I64]);
        assert_eq!(flatten(&WitType::F64), vec![F64]);
        assert_eq!(flatten(&WitType::Char), vec![I32]);
        // string and list<T> both cross as (ptr, len).
        assert_eq!(flatten(&WitType::String), vec![I32, I32]);
        assert_eq!(
            flatten(&WitType::List(Box::new(WitType::U8))),
            vec![I32, I32]
        );
    }

    #[test]
    fn flatten_record_is_field_concatenation_including_nested() {
        // record{ a: u32, b: list<u8> } → [i32] ++ [i32,i32]
        assert_eq!(
            flatten(&rec(&[
                ("a", WitType::U32),
                ("b", WitType::List(Box::new(WitType::U8)))
            ])),
            vec![I32, I32, I32]
        );
        // message-ish: record{ sender: record{ reducer: list<u8> }, payload: list<u8> } → [i32,i32]++[i32,i32]
        assert_eq!(
            flatten(&rec(&[
                (
                    "sender",
                    rec(&[("reducer", WitType::List(Box::new(WitType::U8)))]),
                ),
                ("payload", WitType::List(Box::new(WitType::U8))),
            ])),
            vec![I32, I32, I32, I32]
        );
    }

    #[test]
    fn flatten_tagged_types_prepend_a_discriminant_and_join_payloads() {
        // enum → discriminant only.
        assert_eq!(
            flatten(&WitType::Enum(vec!["a".into(), "b".into()])),
            vec![I32]
        );
        // flags (<=32) → one i32.
        assert_eq!(
            flatten(&WitType::Flags(vec!["r".into(), "w".into()])),
            vec![I32]
        );
        // option<u64> → [disc i32] ++ [i64]
        assert_eq!(
            flatten(&WitType::Option(Box::new(WitType::U64))),
            vec![I32, I64]
        );
        // result<list<u8>, enum> → [disc] ++ join([i32,i32],[i32]) = [i32, i32, i32]
        assert_eq!(
            flatten(&WitType::Result {
                ok: Some(Box::new(WitType::List(Box::new(WitType::U8)))),
                err: Some(Box::new(WitType::Enum(vec!["timeout".into()]))),
            }),
            vec![I32, I32, I32]
        );
        // variant{ continue, close(record{code:u32}) } → [disc] ++ join([], [i32]) = [i32, i32]
        assert_eq!(
            flatten(&WitType::Variant(vec![
                ("continue".into(), None),
                ("close".into(), Some(rec(&[("code", WitType::U32)]))),
            ])),
            vec![I32, I32]
        );
    }

    #[test]
    fn flatten_variant_joins_mismatched_widths_to_i64() {
        // variant{ a(u32), b(u64) } → [disc] ++ [join(i32,i64)=i64]
        assert_eq!(
            flatten(&WitType::Variant(vec![
                ("a".into(), Some(WitType::U32)),
                ("b".into(), Some(WitType::U64)),
            ])),
            vec![I32, I64]
        );
    }

    #[test]
    fn func_core_sig_inlines_within_limits_and_spills_over_them() {
        // (msg: record{c: list<u8>}) -> u32 : params flatten in place, scalar result inline.
        let (p, r) = flatten_func_core_sig(
            &[rec(&[("c", WitType::List(Box::new(WitType::U8)))])],
            Some(&WitType::U32),
        );
        assert_eq!((p, r), (vec![I32, I32], vec![I32]));

        // void result → no core results.
        let (_p, r) = flatten_func_core_sig(&[WitType::U32], None);
        assert_eq!(r, Vec::<CoreTy>::new());

        // a record result (>1 flat) spills to a single i32 return pointer.
        let (_p, r) =
            flatten_func_core_sig(&[], Some(&rec(&[("a", WitType::U32), ("b", WitType::U32)])));
        assert_eq!(r, vec![I32]);

        // params over 16 flats spill to a single i32 pointer: 9 list<u8> params = 18 flats.
        let many: Vec<WitType> = (0..9)
            .map(|_| WitType::List(Box::new(WitType::U8)))
            .collect();
        let (p, _r) = flatten_func_core_sig(&many, Some(&WitType::U32));
        assert_eq!(p, vec![I32]);
    }

    // ── canonical-ABI memory layout (W4c-spill) ───────────────────────────────────────────────────────

    #[test]
    fn canonical_size_and_align_of_scalars_and_list() {
        assert_eq!(
            (
                canonical_size(&WitType::Bool),
                canonical_align(&WitType::Bool)
            ),
            (1, 1)
        );
        assert_eq!(
            (
                canonical_size(&WitType::U16),
                canonical_align(&WitType::U16)
            ),
            (2, 2)
        );
        assert_eq!(
            (
                canonical_size(&WitType::U32),
                canonical_align(&WitType::U32)
            ),
            (4, 4)
        );
        assert_eq!(
            (
                canonical_size(&WitType::U64),
                canonical_align(&WitType::U64)
            ),
            (8, 8)
        );
        // list/string cross as (ptr, len): size 8, align 4.
        let lst = WitType::List(Box::new(WitType::U8));
        assert_eq!((canonical_size(&lst), canonical_align(&lst)), (8, 4));
        assert_eq!(
            (
                canonical_size(&WitType::String),
                canonical_align(&WitType::String)
            ),
            (8, 4)
        );
    }

    #[test]
    fn canonical_record_layout_pads_to_field_alignment() {
        // record{ a: u8, b: u64 }: a@0 (size 1), b padded to align 8 → @8; size align_to(16,8)=16, align 8.
        let r = rec(&[("a", WitType::U8), ("b", WitType::U64)]);
        assert_eq!((canonical_size(&r), canonical_align(&r)), (16, 8));
        assert_eq!(
            record_field_offsets(&[WitType::U8, WitType::U64]),
            vec![0, 8]
        );

        // message-ish: record{ a: u32, b: list<u8> }: a@0, b@4 (list align 4 size 8); size 12, align 4.
        let m = rec(&[
            ("a", WitType::U32),
            ("b", WitType::List(Box::new(WitType::U8))),
        ]);
        assert_eq!((canonical_size(&m), canonical_align(&m)), (12, 4));
        assert_eq!(
            record_field_offsets(&[WitType::U32, WitType::List(Box::new(WitType::U8))]),
            vec![0, 4]
        );
    }

    #[test]
    fn canonical_variant_option_enum_layout() {
        // option<u64> = variant{none, some(u64)}: disc 1 byte, padded to case align 8 → 8, + size 8 → 16.
        let o = WitType::Option(Box::new(WitType::U64));
        assert_eq!((canonical_size(&o), canonical_align(&o)), (16, 8));
        // outcome-ish variant{continue, close(record{code:u32})}: disc 1 → pad to 4 → 4, + size 4 → 8, align 4.
        let v = WitType::Variant(vec![
            ("continue".into(), None),
            ("close".into(), Some(rec(&[("code", WitType::U32)]))),
        ]);
        assert_eq!((canonical_size(&v), canonical_align(&v)), (8, 4));
        // enum of 3 cases = a 1-byte discriminant.
        let e = WitType::Enum(vec!["a".into(), "b".into(), "c".into()]);
        assert_eq!((canonical_size(&e), canonical_align(&e)), (1, 1));
    }
}
