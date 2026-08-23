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
}
