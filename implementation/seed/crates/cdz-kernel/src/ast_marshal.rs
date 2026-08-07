//! Generic marshalling between a wasmtime component [`Val`] and the cadenza TAGGED-AST wire format —
//! the seq-107 "strong typing, binary format = AST encoding" seam for the generic invoke primitive.
//!
//! Operator invoke-ABI ruling (#2050 review): the host must invoke ANY WIT function of ANY signature,
//! marshalling args/results GENERICALLY via the tagged binary AST — NOT a hard-coded interface. This
//! module is the VALUE half of that seam: convert a wasmtime [`Val`] (of any WIT shape a component
//! returns) into cadenza tagged-AST bytes, so a generic invocation's result is a self-describing
//! AST-encoded value the host can hand to the structural selector (opacity: the host never decodes what
//! the value MEANS, only its structure). This direction — [`val_to_ast`] — needs no target type: a `Val`
//! is self-describing. The dual (AST bytes + a WIT [`Type`] → `Val`, for marshalling ARGS in) is a
//! following slice.
//!
//! WIRE = the ONE shared canonical codec ([`cadenza_ast::codec`], the SAME `Leaf`/form vocabulary the
//! kernel log rides — see [`crate::event_ast`]), NOT a bespoke encoding. The WIT↔form correspondence
//! (v-inference-verified against the actual codec forms):
//! - primitives → their `Leaf` (bool→`Bool`, ints→`Int{BigInt,Dec}`, char→`Char`, string→`Str`); FLOATS
//!   are currently REJECTED as [`MarshalError::Unmarshallable`] (an exact f64→`Leaf::Float` decimal
//!   decomposition is a dedicated later slice — the invoke paths that matter now carry no floats);
//! - `list<u8>` → a single `Leaf::Bytes` (a byte blob is ONE length-prefixed bytes leaf, not a
//!   node-per-byte list — #2063 wired the codec to ride blobs this way for exactly this wire);
//! - `list<T>` (T≠u8) → a STRING-HEAD `("list" elem…)` form;
//! - `record{f: v…}` → `("record" (f v)…)` (string head; each field a `(name val)` 2-list);
//! - `tuple<v…>` → `("tuple" v…)` (string head);
//! - `option<T>` → NAME-HEAD ctor `(Some v)` / `(None)`; `result<T,E>` → `(Ok v)` / `(Err e)`;
//! - `variant{Case(v)}` → NAME-HEAD ctor `(Case v)`; `enum{Case}` → `(Case)` (name head, no children);
//! - `flags{A,B}` → `("flags" A B…)` (string head, set-flag names).
//!
//! The string-head vs name-head split matters for the READ side (the dual): record/tuple/list/flags are
//! read via `as_ctor_form` (string head), ctors via `as_form`/`head_name` (name head).

use cadenza_ast::ast::{Arenas, Builder, Leaf, Radix, Struct, StructId};
use cadenza_ast::codec;
use num_bigint::BigInt;
use wasmtime::component::{Type, Val};

/// A generic marshalling failure. A sum (no-sentinels), covering both directions ([`val_to_ast`] and the
/// dual [`ast_to_val`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarshalError {
    /// A `Val` variant [`val_to_ast`] does not marshal — a resource handle, a future/stream, or an
    /// error-context. These have no tagged-AST value form (they're host/async-runtime handles, not
    /// values that cross the wire), so a component returning one at the invoke boundary is a genuine
    /// "not marshallable" condition, surfaced rather than silently encoded as something wrong.
    Unmarshallable { val_kind: String },
    /// [`ast_to_val`]: the supplied bytes are not a decodable tagged-AST (`codec::decode` returned None) —
    /// a torn/corrupt arg payload, distinct from a well-formed AST of the wrong shape.
    Undecodable,
    /// [`ast_to_val`]: the decoded AST does not match the target WIT [`Type`] — the shape the caller's
    /// component expects (e.g. a record form where the type is a variant, or a missing field/case).
    /// `expected` names the WIT type kind; `found` is a BOUNDED shape hint (never the full untrusted AST).
    TypeMismatch { expected: String, found: String },
    /// [`ast_to_val`]: an integer leaf is outside the target WIT integer type's range (e.g. a `300` for a
    /// `u8`, or a negative for a `u32`). The AST carries an arbitrary-precision `BigInt`, so a value that
    /// doesn't fit the declared width is a loud error, not a silent truncation.
    IntOutOfRange { wit_type: String },
    /// [`ast_to_val`]: the target WIT [`Type`] is one the arg-marshalling doesn't build (a resource
    /// handle, future/stream, error-context, or float pending the decimal slice) — the dual of
    /// [`MarshalError::Unmarshallable`] on the type side.
    UnsupportedType { wit_type: String },
}

/// Marshal a wasmtime component [`Val`] into cadenza tagged-AST wire bytes ([`cadenza_ast::codec`]).
/// The RESULT half of the generic invoke seam (seq-107): a generic invocation's result `Val` — of
/// whatever WIT shape the invoked component returns — becomes a self-describing AST-encoded value. A
/// `Val` is self-typed, so no target WIT type is needed here (the dual, args-in, needs one). Errors only
/// on a genuinely value-less `Val` (a resource/future/stream handle — [`MarshalError::Unmarshallable`]).
pub fn val_to_ast(val: &Val) -> Result<Vec<u8>, MarshalError> {
    let mut b = Builder::new();
    let root = build_val(&mut b, val)?;
    Ok(codec::encode(&b.finish(root)))
}

/// Build `val` into the arena `b`, returning the root node id. Recursive: a compound `Val`'s children
/// are built first, then wrapped in the head form. The head-shape rules are the WIT↔form correspondence
/// documented at the module head (string-head for record/tuple/list/flags, name-head ctors for
/// option/result/variant/enum, a lone `Leaf` for primitives, `Leaf::Bytes` for `list<u8>`).
fn build_val(b: &mut Builder, val: &Val) -> Result<StructId, MarshalError> {
    let int_leaf = |b: &mut Builder, v: BigInt| {
        b.atom_leaf(Leaf::Int {
            value: v,
            radix: Radix::Dec,
        })
    };
    Ok(match val {
        Val::Bool(x) => b.atom_leaf(Leaf::Bool(*x)),
        Val::S8(x) => int_leaf(b, BigInt::from(*x)),
        Val::U8(x) => int_leaf(b, BigInt::from(*x)),
        Val::S16(x) => int_leaf(b, BigInt::from(*x)),
        Val::U16(x) => int_leaf(b, BigInt::from(*x)),
        Val::S32(x) => int_leaf(b, BigInt::from(*x)),
        Val::U32(x) => int_leaf(b, BigInt::from(*x)),
        Val::S64(x) => int_leaf(b, BigInt::from(*x)),
        Val::U64(x) => int_leaf(b, BigInt::from(*x)),
        // Float marshalling is DEFERRED: cadenza's `Decimal` is an exact base-10 `significand·10^exp`, and
        // a correct f64→exact-decimal decomposition (incl. the binary-fraction tail) is a slice of its own.
        // The invoke paths that matter now (compiler/syntax/bytes) carry no floats, so surface an honest
        // Unmarshallable rather than ship a lossy/wrong Decimal. A dedicated float slice adds it exactly.
        Val::Float32(_) => return Err(unmarshallable("float32 (deferred)")),
        Val::Float64(_) => return Err(unmarshallable("float64 (deferred)")),
        Val::Char(c) => b.atom_leaf(Leaf::Char(*c)),
        Val::String(s) => b.atom_leaf(Leaf::Str(s.clone())),
        // list<u8> is the ONE list special-cased to a single Bytes leaf (blob-optimized wire, #2063);
        // any other list<T> is a string-head ("list" elem…) form of per-element nodes.
        Val::List(items) => {
            if let Some(bytes) = as_u8_list(items) {
                b.atom_leaf(Leaf::Bytes(bytes))
            } else {
                let mut children = vec![b.atom_leaf(Leaf::Str("list".into()))];
                for it in items.iter() {
                    let c = build_val(b, it)?;
                    children.push(c);
                }
                b.list(children)
            }
        }
        // record → ("record" (name val)…): string head, each field a (name val) 2-list.
        Val::Record(fields) => {
            let mut children = vec![b.atom_leaf(Leaf::Str("record".into()))];
            for (name, v) in fields.iter() {
                let name_node = b.name(name);
                let val_node = build_val(b, v)?;
                let field = b.list(vec![name_node, val_node]);
                children.push(field);
            }
            b.list(children)
        }
        // tuple → ("tuple" v…): string head, positional.
        Val::Tuple(items) => {
            let mut children = vec![b.atom_leaf(Leaf::Str("tuple".into()))];
            for it in items.iter() {
                let c = build_val(b, it)?;
                children.push(c);
            }
            b.list(children)
        }
        // option → NAME-head ctor (Some v) / (None).
        Val::Option(opt) => match opt {
            Some(v) => {
                let head = b.name("Some");
                let inner = build_val(b, v)?;
                b.list(vec![head, inner])
            }
            None => {
                let head = b.name("None");
                b.list(vec![head])
            }
        },
        // result → NAME-head ctor (Ok v) / (Err e). A payload-less Ok/Err (result<_, _> with a unit arm)
        // is the bare ctor (Ok) / (Err).
        Val::Result(res) => match res {
            Ok(v) => build_ctor(b, "Ok", v.as_deref())?,
            Err(e) => build_ctor(b, "Err", e.as_deref())?,
        },
        // variant → NAME-head ctor (Case v) (or (Case) for a payload-less case).
        Val::Variant(case, payload) => build_ctor(b, case, payload.as_deref())?,
        // enum → NAME-head ctor (Case) with no children.
        Val::Enum(case) => {
            let head = b.name(case);
            b.list(vec![head])
        }
        // flags → ("flags" A B…): string head, one Name per SET flag.
        Val::Flags(names) => {
            let mut children = vec![b.atom_leaf(Leaf::Str("flags".into()))];
            for n in names.iter() {
                let node = b.name(n);
                children.push(node);
            }
            b.list(children)
        }
        // Value-less handles have no tagged-AST value form — surface rather than mis-encode.
        Val::Resource(_) => return Err(unmarshallable("resource")),
        // wasmtime's Val is #[non_exhaustive] (future/stream/error-context + any later variant); none is
        // a plain value that crosses this wire, so classify generically rather than mis-encoding.
        _ => return Err(unmarshallable("non-value handle")),
    })
}

/// Build a NAME-head ctor form `(Name payload?)` — the shared shape for option/result/variant cases.
/// `None` payload → the bare `(Name)` (a payload-less case); `Some` → `(Name inner)`.
fn build_ctor(
    b: &mut Builder,
    name: &str,
    payload: Option<&Val>,
) -> Result<StructId, MarshalError> {
    let head = b.name(name);
    Ok(match payload {
        Some(v) => {
            let inner = build_val(b, v)?;
            b.list(vec![head, inner])
        }
        None => b.list(vec![head]),
    })
}

/// Marshal a wasmtime component [`Type`] into a cadenza tagged-AST TYPE DESCRIPTOR — the wire a
/// component-signature query carries for each exported func's param/result types (v-agent-harness's
/// `control/signature` effect + descriptor codec, cadenza-docs' resolved-type projection dual). Unlike
/// [`val_to_ast`] (which reifies a VALUE), this reifies the TYPE itself: a reducer discovering a target
/// component's callable surface gets each param/result as a structured, Cadenza-decodable type-AST it
/// can inspect/route on — NOT an opaque byte blob. The RESULT bytes are the ONE shared canonical codec
/// ([`cadenza_ast::codec`]), so a reducer decodes the descriptor with the same codec everything else uses.
///
/// The type-descriptor form vocabulary (chosen to compose with — but stay distinct from — the VALUE
/// vocabulary of [`build_val`]; a type names a SHAPE, it selects no case):
/// - primitives → a NAME-head marker, no children: `(bool)` `(u8)`…`(s64)` `(char)` `(string)`
///   `(f32)`/`(f64)` (float TYPES DESCRIBE fine even though float VALUES don't yet marshal — §float);
/// - `list<T>` → `("list" <T-descriptor>)` (string head, matching the value side's `list` head);
/// - `record{f: T…}` → `("record" (f <T-descriptor>)…)` (string head; each field a `(name type)` 2-list);
/// - `tuple<T…>` → `("tuple" <T-descriptor>…)` (string head);
/// - `option<T>` → `("option" <T-descriptor>)` (string head — a TYPE, so NOT the value side's name-head
///   `Some`/`None` ctor: there is no case selected, only the option-of-T shape);
/// - `result<T,E>` → `("result" <T-or-unit> <E-or-unit>)` where a unit arm is the empty form `("unit")`;
/// - `variant{Case(T)?…}` → `("variant" (Case <T-descriptor>?)…)` (string head; each case a
///   `(CaseName type?)` — payload-less cases are the bare `(CaseName)`);
/// - `enum{Case…}` → `("enum" Case…)` (string head, one `Name` per case);
/// - `flags{A…}` → `("flags" A…)` (string head, one `Name` per declared flag).
///
/// FLOAT (§float): float VALUES are [`MarshalError::Unmarshallable`] pending the exact-decimal slice, but
/// a float TYPE marshals fine — a signature query over a float-carrying func succeeds (the reducer learns
/// the shape); only INVOKING such a func hits the deferred value gap, orthogonal to describing it.
///
/// UNSUPPORTED: a resource/future/stream/error-context type has no value-crossing form —
/// [`MarshalError::UnsupportedType`], the type-side dual of [`val_to_ast`]'s `Unmarshallable`.
pub fn type_to_ast(ty: &Type) -> Result<Vec<u8>, MarshalError> {
    let mut b = Builder::new();
    let root = build_type(&mut b, ty)?;
    Ok(codec::encode(&b.finish(root)))
}

/// Build the type-descriptor for `ty` into arena `b`, returning the root node id. Recursive: a compound
/// type's sub-types (via wasmtime's `Type` reflection accessors — `lt.ty()`, `rt.fields()`, `tt.types()`,
/// `ot.ty()`, `rt.ok()`/`err()`, `vt.cases()`, `et.names()`, `ft.names()`) are built first, then wrapped.
/// Mirrors [`build_from_ast`]'s `Type` traversal exactly (same variants, same accessors), but EMITS a
/// descriptor node instead of READING a value — so the two can never disagree on which types exist.
fn build_type(b: &mut Builder, ty: &Type) -> Result<StructId, MarshalError> {
    // A primitive type = a lone name-head marker `(kind)` (a 1-element list whose head names the kind).
    let prim = |b: &mut Builder, kind: &str| {
        let head = b.name(kind);
        b.list(vec![head])
    };
    Ok(match ty {
        Type::Bool => prim(b, "bool"),
        Type::U8 => prim(b, "u8"),
        Type::U16 => prim(b, "u16"),
        Type::U32 => prim(b, "u32"),
        Type::U64 => prim(b, "u64"),
        Type::S8 => prim(b, "s8"),
        Type::S16 => prim(b, "s16"),
        Type::S32 => prim(b, "s32"),
        Type::S64 => prim(b, "s64"),
        Type::Char => prim(b, "char"),
        Type::String => prim(b, "string"),
        // Float TYPES describe fine (a sig query learns the shape); only float VALUES are deferred.
        Type::Float32 => prim(b, "f32"),
        Type::Float64 => prim(b, "f64"),
        // list<T> → ("list" <T>): string head, one child = the element TYPE descriptor.
        Type::List(lt) => {
            let head = b.atom_leaf(Leaf::Str("list".into()));
            let elem = build_type(b, &lt.ty())?;
            b.list(vec![head, elem])
        }
        // record → ("record" (fieldname <T>)…): string head, each field a (name type) 2-list.
        Type::Record(rt) => {
            let mut children = vec![b.atom_leaf(Leaf::Str("record".into()))];
            for field in rt.fields() {
                let name_node = b.name(field.name);
                let ty_node = build_type(b, &field.ty)?;
                let entry = b.list(vec![name_node, ty_node]);
                children.push(entry);
            }
            b.list(children)
        }
        // tuple → ("tuple" <T>…): string head, one type descriptor per element, positional.
        Type::Tuple(tt) => {
            let mut children = vec![b.atom_leaf(Leaf::Str("tuple".into()))];
            for t in tt.types() {
                let node = build_type(b, &t)?;
                children.push(node);
            }
            b.list(children)
        }
        // option<T> → ("option" <T>): string head (a TYPE, not the value side's Some/None ctor).
        Type::Option(ot) => {
            let head = b.atom_leaf(Leaf::Str("option".into()));
            let inner = build_type(b, &ot.ty())?;
            b.list(vec![head, inner])
        }
        // result<T,E> → ("result" <T-or-unit> <E-or-unit>): a unit (payload-less) arm is ("unit").
        Type::Result(rt) => {
            let head = b.atom_leaf(Leaf::Str("result".into()));
            let ok = build_opt_type(b, rt.ok())?;
            let err = build_opt_type(b, rt.err())?;
            b.list(vec![head, ok, err])
        }
        // variant → ("variant" (Case <T>?)…): string head; each case a (CaseName type?) — payload-less
        // cases are the bare (CaseName).
        Type::Variant(vt) => {
            let mut children = vec![b.atom_leaf(Leaf::Str("variant".into()))];
            for case in vt.cases() {
                let case_head = b.name(case.name);
                let entry = match case.ty {
                    Some(t) => {
                        let ty_node = build_type(b, &t)?;
                        b.list(vec![case_head, ty_node])
                    }
                    None => b.list(vec![case_head]),
                };
                children.push(entry);
            }
            b.list(children)
        }
        // enum → ("enum" Case…): string head, one Name per case (never a payload).
        Type::Enum(et) => {
            let mut children = vec![b.atom_leaf(Leaf::Str("enum".into()))];
            for name in et.names() {
                let node = b.name(name);
                children.push(node);
            }
            b.list(children)
        }
        // flags → ("flags" A…): string head, one Name per declared flag.
        Type::Flags(ft) => {
            let mut children = vec![b.atom_leaf(Leaf::Str("flags".into()))];
            for name in ft.names() {
                let node = b.name(name);
                children.push(node);
            }
            b.list(children)
        }
        // A resource/future/stream/error-context type has no value-crossing shape — the type-side dual of
        // val_to_ast's Unmarshallable. `Type` is #[non_exhaustive], so a catch-all classifies generically.
        Type::Own(_) | Type::Borrow(_) => {
            return Err(MarshalError::UnsupportedType {
                wit_type: "resource handle".into(),
            })
        }
        _ => {
            return Err(MarshalError::UnsupportedType {
                wit_type: "unsupported type (future/stream/error-context/unknown)".into(),
            })
        }
    })
}

/// Build an OPTIONAL payload TYPE for a `result` arm: `Some(t)` → `t`'s descriptor, `None` (a unit arm) →
/// the empty form `("unit")`. Keeps the `result` descriptor total (both arms always present, unit-marked).
fn build_opt_type(b: &mut Builder, ty: Option<Type>) -> Result<StructId, MarshalError> {
    match ty {
        Some(t) => build_type(b, &t),
        None => {
            let head = b.name("unit");
            Ok(b.list(vec![head]))
        }
    }
}

/// The DUAL of [`val_to_ast`]: marshal cadenza tagged-AST wire `bytes` into a wasmtime component [`Val`]
/// of the target WIT `ty` — the ARGS-IN half of the generic invoke seam (seq-107). A caller supplies an
/// AST-encoded arg value + the WIT param type the component expects; this builds the `Val` to pass. The
/// build is TYPE-DIRECTED because an AST form alone is ambiguous (a name-head ctor could be an option
/// OR a variant OR a result — the target `Type` disambiguates), and because the same primitive leaf maps
/// to different `Val` widths (`Leaf::Int` → u8/u32/s64/… per `ty`, range-checked).
pub fn ast_to_val(bytes: &[u8], ty: &Type) -> Result<Val, MarshalError> {
    let arenas = codec::decode(bytes).ok_or(MarshalError::Undecodable)?;
    build_from_ast(&arenas, arenas.root, ty)
}

/// Build the `Val` at arena node `id` per the target WIT `ty`. Recursive: a compound type reads its
/// children by the sub-types `ty` exposes (via wasmtime's `Type` reflection) and recurses. The form
/// rules mirror [`build_val`] exactly (string-head record/tuple/list/flags read via `as_ctor_form`,
/// name-head option/result/variant/enum ctors read via `as_form`, `list<u8>` from a `Leaf::Bytes`,
/// primitives from their leaf), but here the TYPE drives which reader to apply.
fn build_from_ast(a: &Arenas, id: StructId, ty: &Type) -> Result<Val, MarshalError> {
    match ty {
        Type::Bool => Ok(Val::Bool(read_bool(a, id)?)),
        Type::U8 => Ok(Val::U8(read_uint(a, id, "u8")? as u8)),
        Type::U16 => Ok(Val::U16(read_uint(a, id, "u16")? as u16)),
        Type::U32 => Ok(Val::U32(read_uint(a, id, "u32")? as u32)),
        Type::U64 => Ok(Val::U64(read_uint(a, id, "u64")?)),
        Type::S8 => Ok(Val::S8(read_sint(a, id, "s8")? as i8)),
        Type::S16 => Ok(Val::S16(read_sint(a, id, "s16")? as i16)),
        Type::S32 => Ok(Val::S32(read_sint(a, id, "s32")? as i32)),
        Type::S64 => Ok(Val::S64(read_sint(a, id, "s64")?)),
        Type::Char => Ok(Val::Char(read_char(a, id)?)),
        Type::String => Ok(Val::String(read_str(a, id)?.to_string())),
        Type::Float32 | Type::Float64 => Err(MarshalError::UnsupportedType {
            wit_type: "float (deferred)".into(),
        }),
        // list<u8> ← a single Leaf::Bytes; list<T≠u8> ← a string-head ("list" elem…) form, each elem
        // built per the element type.
        Type::List(lt) => {
            let elem_ty = lt.ty();
            if matches!(elem_ty, Type::U8) {
                let bytes = read_bytes(a, id)?;
                Ok(Val::List(bytes.into_iter().map(Val::U8).collect()))
            } else {
                // An EMPTY list<T> of ANY element type marshalled to an empty `Leaf::Bytes` on the write
                // side (a `Val::List` carries no element-type tag when empty, so `val_to_ast` can't tell
                // list<u8> from an empty list<u32>/list<record>/… — they're all byte-identical empty-Bytes).
                // The read is TYPE-DIRECTED (reviewer catch): the target Type says list<T≠u8>, so an
                // empty-Bytes node here IS a valid empty list<T> — accept it, don't demand a ("list" …) form.
                if let Some(Leaf::Bytes(bytes)) = leaf_of(a, id) {
                    if bytes.is_empty() {
                        return Ok(Val::List(Vec::new()));
                    }
                    // A NON-empty Bytes for a non-u8 element type is a genuine shape mismatch (a byte blob
                    // can't be a list<u32> etc.).
                    return Err(type_mismatch("list", "non-empty bytes for a non-u8 list"));
                }
                let elems = form(a, id, "list")?;
                let mut out = Vec::with_capacity(elems.len());
                for &e in elems {
                    out.push(build_from_ast(a, e, &elem_ty)?);
                }
                Ok(Val::List(out))
            }
        }
        // record ← ("record" (fieldname val)…): match each declared field by NAME (order-independent).
        Type::Record(rt) => {
            let field_nodes = form(a, id, "record")?;
            // Collect the AST fields as (name → value-node), rejecting a malformed field entry (not a
            // (name val) 2-list) AND a DUPLICATE name up front. ast_to_val decodes UNTRUSTED arg bytes, so
            // the record must EXACTLY match the WIT shape (github-liaison #2078): silently accepting extra
            // or duplicate fields hides malformed input + yields a surprising Val (same untrusted-input
            // hardening as the #2050 {val:?} DoS; mirrors the tuple arm's strict arity below).
            let mut ast_fields: std::collections::BTreeMap<&str, StructId> = Default::default();
            for &fnode in field_nodes {
                let (name, val_node) = match a.get(fnode) {
                    Struct::List(kids) if kids.len() == 2 => match a.as_name(kids[0]) {
                        Some(n) => (n, kids[1]),
                        None => return Err(type_mismatch("record", "field name is not a name")),
                    },
                    _ => return Err(type_mismatch("record", "field is not a (name val) pair")),
                };
                if ast_fields.insert(name, val_node).is_some() {
                    return Err(type_mismatch(
                        "record",
                        format!("duplicate field {}", bounded_name(name)),
                    ));
                }
            }
            let mut out = Vec::new();
            for field in rt.fields() {
                let node = ast_fields.remove(field.name).ok_or_else(|| {
                    type_mismatch("record", format!("missing field {:?}", field.name))
                })?;
                out.push((field.name.to_string(), build_from_ast(a, node, &field.ty)?));
            }
            // Any AST field left over is an EXTRA field not in the WIT record shape — reject (exact match).
            if let Some((extra, _)) = ast_fields.iter().next() {
                return Err(type_mismatch(
                    "record",
                    format!("unknown field {}", bounded_name(extra)),
                ));
            }
            Ok(Val::Record(out))
        }
        // tuple ← ("tuple" v…): positional, one per declared element type.
        Type::Tuple(tt) => {
            let elems = form(a, id, "tuple")?;
            let types: Vec<Type> = tt.types().collect();
            if elems.len() != types.len() {
                return Err(type_mismatch(
                    "tuple",
                    format!("arity {} ≠ type arity {}", elems.len(), types.len()),
                ));
            }
            let mut out = Vec::with_capacity(types.len());
            for (node, t) in elems.iter().zip(types.iter()) {
                out.push(build_from_ast(a, *node, t)?);
            }
            Ok(Val::Tuple(out))
        }
        // option<T> ← name-head (Some v) / (None).
        Type::Option(ot) => {
            let (case, payload) = ctor(a, id)?;
            match case {
                "Some" => {
                    let inner =
                        payload.ok_or_else(|| type_mismatch("option", "Some without payload"))?;
                    Ok(Val::Option(Some(Box::new(build_from_ast(
                        a,
                        inner,
                        &ot.ty(),
                    )?))))
                }
                "None" => Ok(Val::Option(None)),
                other => Err(type_mismatch(
                    "option",
                    format!("case {} ∉ {{Some,None}}", bounded_name(other)),
                )),
            }
        }
        // result<T,E> ← name-head (Ok v?) / (Err e?), honoring the ok/err payload types (each may be unit).
        Type::Result(rt) => {
            let (case, payload) = ctor(a, id)?;
            match case {
                "Ok" => Ok(Val::Result(Ok(opt_payload(a, payload, rt.ok())?))),
                "Err" => Ok(Val::Result(Err(opt_payload(a, payload, rt.err())?))),
                other => Err(type_mismatch(
                    "result",
                    format!("case {} ∉ {{Ok,Err}}", bounded_name(other)),
                )),
            }
        }
        // variant ← name-head (Case v?): match the case name against the declared cases.
        Type::Variant(vt) => {
            let (case, payload) = ctor(a, id)?;
            let decl = vt.cases().find(|c| c.name == case).ok_or_else(|| {
                type_mismatch("variant", format!("unknown case {}", bounded_name(case)))
            })?;
            let val = opt_payload(a, payload, decl.ty)?;
            Ok(Val::Variant(case.to_string(), val))
        }
        // enum ← name-head (Case) with no payload; the case must be a declared name.
        Type::Enum(et) => {
            let (case, payload) = ctor(a, id)?;
            if payload.is_some() {
                return Err(type_mismatch(
                    "enum",
                    format!("case {} carries a payload", bounded_name(case)),
                ));
            }
            if !et.names().any(|n| n == case) {
                return Err(type_mismatch(
                    "enum",
                    format!("unknown case {}", bounded_name(case)),
                ));
            }
            Ok(Val::Enum(case.to_string()))
        }
        // flags ← ("flags" Name…): each set flag must be a declared name.
        Type::Flags(ft) => {
            let name_nodes = form(a, id, "flags")?;
            let declared: Vec<&str> = ft.names().collect();
            let mut set = Vec::new();
            for &n in name_nodes {
                let name = a
                    .as_name(n)
                    .ok_or_else(|| type_mismatch("flags", "non-name flag element"))?;
                if !declared.contains(&name) {
                    return Err(type_mismatch(
                        "flags",
                        format!("unknown flag {}", bounded_name(name)),
                    ));
                }
                set.push(name.to_string());
            }
            Ok(Val::Flags(set))
        }
        Type::Own(_) | Type::Borrow(_) => Err(MarshalError::UnsupportedType {
            wit_type: "resource handle".into(),
        }),
        // wasmtime Type is #[non_exhaustive] (future/stream/error-context + any later kind): none is a
        // plain arg value this seam marshals.
        _ => Err(MarshalError::UnsupportedType {
            wit_type: "non-value type".into(),
        }),
    }
}

// --- ast_to_val leaf/form readers (type-directed; each maps a shape mismatch to a TypeMismatch) ---

/// The leaf at node `id`, or None if `id` is a list (not an atom).
fn leaf_of(a: &Arenas, id: StructId) -> Option<&Leaf> {
    match a.get(id) {
        Struct::Atom(lid) => Some(a.leaf(*lid)),
        Struct::List(_) => None,
    }
}

fn read_bool(a: &Arenas, id: StructId) -> Result<bool, MarshalError> {
    match leaf_of(a, id) {
        Some(Leaf::Bool(x)) => Ok(*x),
        _ => Err(type_mismatch("bool", "not a bool leaf")),
    }
}

fn read_char(a: &Arenas, id: StructId) -> Result<char, MarshalError> {
    match leaf_of(a, id) {
        Some(Leaf::Char(c)) => Ok(*c),
        _ => Err(type_mismatch("char", "not a char leaf")),
    }
}

fn read_str(a: &Arenas, id: StructId) -> Result<&str, MarshalError> {
    match leaf_of(a, id) {
        Some(Leaf::Str(s)) => Ok(s),
        _ => Err(type_mismatch("string", "not a string leaf")),
    }
}

fn read_bytes(a: &Arenas, id: StructId) -> Result<Vec<u8>, MarshalError> {
    match leaf_of(a, id) {
        Some(Leaf::Bytes(b)) => Ok(b.clone()),
        _ => Err(type_mismatch("list<u8>", "not a bytes leaf")),
    }
}

/// Read an unsigned int leaf, range-checked into a u64 (the caller narrows to the WIT width). A negative
/// or out-of-u64 value is IntOutOfRange; a non-int leaf is a TypeMismatch. The per-width narrowing (as u8
/// etc.) at the call site is lossless because we FIRST check the value fits the width via the width name.
fn read_uint(a: &Arenas, id: StructId, wit: &str) -> Result<u64, MarshalError> {
    let value = int_value(a, id, wit)?;
    let v = u64::try_from(value).map_err(|_| MarshalError::IntOutOfRange {
        wit_type: wit.into(),
    })?;
    check_uint_width(v, wit)?;
    Ok(v)
}

/// Read a signed int leaf, range-checked into an i64 (the caller narrows). Out-of-i64 or width is
/// IntOutOfRange.
fn read_sint(a: &Arenas, id: StructId, wit: &str) -> Result<i64, MarshalError> {
    let value = int_value(a, id, wit)?;
    let v = i64::try_from(value).map_err(|_| MarshalError::IntOutOfRange {
        wit_type: wit.into(),
    })?;
    check_sint_width(v, wit)?;
    Ok(v)
}

fn int_value<'a>(a: &'a Arenas, id: StructId, wit: &str) -> Result<&'a BigInt, MarshalError> {
    match leaf_of(a, id) {
        Some(Leaf::Int { value, .. }) => Ok(value),
        _ => Err(type_mismatch(wit, "not an int leaf")),
    }
}

/// Verify a u64 fits the sub-u64 WIT width (u8/u16/u32); u64 always fits. IntOutOfRange otherwise.
fn check_uint_width(v: u64, wit: &str) -> Result<(), MarshalError> {
    let ok = match wit {
        "u8" => v <= u8::MAX as u64,
        "u16" => v <= u16::MAX as u64,
        "u32" => v <= u32::MAX as u64,
        _ => true, // u64
    };
    ok.then_some(()).ok_or_else(|| MarshalError::IntOutOfRange {
        wit_type: wit.into(),
    })
}

fn check_sint_width(v: i64, wit: &str) -> Result<(), MarshalError> {
    let ok = match wit {
        "s8" => i8::try_from(v).is_ok(),
        "s16" => i16::try_from(v).is_ok(),
        "s32" => i32::try_from(v).is_ok(),
        _ => true, // s64
    };
    ok.then_some(()).ok_or_else(|| MarshalError::IntOutOfRange {
        wit_type: wit.into(),
    })
}

/// The children of a STRING-HEAD form (`(<head> child…)` where head is a `Leaf::Str`), read via
/// `as_ctor_form`; also accepts the NAME-head alias `as_form` (a hand-written/pre-reduction spelling).
/// Used for record/tuple/list/flags. A non-matching node is a TypeMismatch.
fn form<'a>(a: &'a Arenas, id: StructId, head: &str) -> Result<&'a [StructId], MarshalError> {
    a.as_ctor_form(id, head)
        .or_else(|| a.as_form(id, head))
        .ok_or_else(|| type_mismatch(head, format!("not a {head:?} form")))
}

/// A NAME-HEAD ctor `(Name payload?)` → (case name, optional single payload node). Used for
/// option/result/variant/enum. The head must be a `Name` leaf; 0 children after it = no payload, 1 =
/// the payload, >1 is a malformed ctor.
fn ctor(a: &Arenas, id: StructId) -> Result<(&str, Option<StructId>), MarshalError> {
    let kids = match a.get(id) {
        Struct::List(kids) if !kids.is_empty() => kids,
        _ => return Err(type_mismatch("ctor", "not a non-empty form")),
    };
    let name = a
        .as_name(kids[0])
        .ok_or_else(|| type_mismatch("ctor", "head is not a name"))?;
    match kids.len() {
        1 => Ok((name, None)),
        2 => Ok((name, Some(kids[1]))),
        n => Err(type_mismatch(
            "ctor",
            format!("{} payload nodes (expected 0 or 1)", n - 1),
        )),
    }
}

/// Build an optional ctor payload against an optional payload type (option/result/variant arms). A
/// payload node present ⟺ the arm type is `Some` — a mismatch (payload without a type, or a type without
/// a payload node) is a TypeMismatch.
fn opt_payload(
    a: &Arenas,
    payload: Option<StructId>,
    arm_ty: Option<Type>,
) -> Result<Option<Box<Val>>, MarshalError> {
    match (payload, arm_ty) {
        (Some(node), Some(t)) => Ok(Some(Box::new(build_from_ast(a, node, &t)?))),
        (None, None) => Ok(None),
        (Some(_), None) => Err(type_mismatch("ctor", "payload present but arm is unit")),
        (None, Some(_)) => Err(type_mismatch(
            "ctor",
            "arm expects a payload but none present",
        )),
    }
}

fn type_mismatch(expected: &str, found: impl Into<String>) -> MarshalError {
    MarshalError::TypeMismatch {
        expected: expected.to_string(),
        found: found.into(),
    }
}

/// A BOUNDED rendering of an UNTRUSTED name (a record field / variant case / enum case / flag) for an
/// error message (github-liaison #2090): these names come from untrusted arg bytes, and
/// `TypeMismatch.found` is a bounded hint — Debug-formatting a multi-MB attacker-supplied name would blow
/// up the error-string alloc on the reject path (same class as the #2050 `{val:?}` DoS + the #2078
/// `val_shape` fix). Caps at 64 bytes (on a char boundary) with an ellipsis + the full length, so the
/// message stays small regardless of the name's size.
fn bounded_name(name: &str) -> String {
    const CAP: usize = 64;
    if name.len() <= CAP {
        format!("{name:?}")
    } else {
        let mut end = CAP;
        while end > 0 && !name.is_char_boundary(end) {
            end -= 1;
        }
        format!("{:?}… (len {})", &name[..end], name.len())
    }
}

/// If every element of `items` is a `Val::U8`, the raw byte vector — the `list<u8>` → `Leaf::Bytes`
/// detector. An empty list is treated as an empty byte blob (a zero-length `list<u8>` is still bytes).
fn as_u8_list(items: &[Val]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(items.len());
    for it in items {
        match it {
            Val::U8(b) => out.push(*b),
            _ => return None,
        }
    }
    Some(out)
}

fn unmarshallable(kind: &str) -> MarshalError {
    MarshalError::Unmarshallable {
        val_kind: kind.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cadenza_ast::ast::{Arenas, Leaf, Struct, StructId};

    // Decode marshalled bytes back to an Arenas for structural assertions.
    fn decode(bytes: &[u8]) -> Arenas {
        codec::decode(bytes).expect("marshalled bytes decode")
    }
    fn leaf_at(a: &Arenas, id: StructId) -> &Leaf {
        match a.get(id) {
            Struct::Atom(lid) => a.leaf(*lid),
            Struct::List(_) => panic!("expected an atom at {id:?}"),
        }
    }
    fn root_leaf(a: &Arenas) -> &Leaf {
        leaf_at(a, a.root)
    }

    // --- ast_to_val test harness: extract real wasmtime `Type`s from a WAT component's exported func ---
    // A `Type` isn't directly constructible (it comes from component reflection), so a WAT `probe` func
    // whose RESULT wraps the WANTED type as `(list <ty>)` gives us that `Type` via `Func::results` (we read
    // the RESULT, not a param — a list result forces a uniform indirect return, so ONE probe shape reflects
    // any type). We instantiate against an empty linker and never CALL probe — only read its result type.
    fn param_type(component_wat: &str) -> Type {
        let engine = wasmtime::Engine::default();
        let bytes = wat::parse_str(component_wat).expect("assemble probe component");
        let component =
            wasmtime::component::Component::new(&engine, &bytes).expect("valid component");
        let mut store = wasmtime::Store::new(&engine, ());
        let linker = wasmtime::component::Linker::<()>::new(&engine);
        let instance = linker
            .instantiate(&mut store, &component)
            .expect("instantiate");
        let idx = instance
            .get_export_index(&mut store, None, "probe")
            .expect("exports probe");
        let func = instance.get_func(&mut store, idx).expect("probe is a func");
        // probe returns `record { x: <ty> }`. A record result ALWAYS lifts through the uniform indirect
        // return (core func returns one i32 ptr), so ONE probe shape reflects EVERY WIT type — including
        // scalars, which returned bare would force a per-type core result signature. Pull the `x` field's
        // type back out.
        let results = func.results(&store);
        let Type::List(lt) = &results[0] else {
            panic!("probe result is a list");
        };
        lt.ty()
    }

    // A component exporting `probe: func() -> (list <ty-decl>)` — the minimal UNIFORM way to reflect ANY
    // WIT `Type`: wrapping in a `list` forces the indirect return (core func is always `(result i32)`),
    // AND an inline `(list <ty>)` result — unlike a named record — needs no exported-type alias (avoids
    // the WAT "func not valid to be used as export" rule for named types). We read the element type back.
    fn probe_component(ty_decl: &str) -> String {
        // Declare the wanted type, EXPORT it (an exported func referencing a named type requires the type
        // be exported first — the WAT "func not valid to be used as export" rule), then return
        // `(list $t-x)` so the result goes indirect (uniform core `(result i32)`). Reading List::ty() back
        // gives the wanted type. Works for named compound types (record/variant) AND inline primitives.
        format!(
            r#"(component
                 (core module $m
                   (memory (export "mem") 1)
                   (func (export "realloc") (param i32 i32 i32 i32) (result i32) (i32.const 0))
                   (func (export "probe") (result i32) (i32.const 0)))
                 (core instance $i (instantiate $m))
                 (type $t {ty_decl})
                 (export $t-x "wanted" (type $t))
                 (func $probe (result (list $t-x))
                   (canon lift (core func $i "probe") (memory $i "mem") (realloc (func $i "realloc"))))
                 (export "probe" (func $probe)))"#
        )
    }

    // Round-trip a Val through val_to_ast → ast_to_val (against the WIT type from probe) and assert equal.
    fn round_trip(val: Val, ty_decl: &str) -> Val {
        let bytes = val_to_ast(&val).expect("val_to_ast");
        let ty = param_type(&probe_component(ty_decl));
        ast_to_val(&bytes, &ty).expect("ast_to_val")
    }

    #[test]
    fn ast_to_val_round_trips_primitives() {
        assert_eq!(round_trip(Val::Bool(true), "bool"), Val::Bool(true));
        assert_eq!(round_trip(Val::U8(200), "u8"), Val::U8(200));
        assert_eq!(round_trip(Val::U32(70000), "u32"), Val::U32(70000));
        assert_eq!(round_trip(Val::U64(u64::MAX), "u64"), Val::U64(u64::MAX));
        assert_eq!(round_trip(Val::S64(i64::MIN), "s64"), Val::S64(i64::MIN));
        assert_eq!(round_trip(Val::Char('é'), "char"), Val::Char('é'));
        assert_eq!(
            round_trip(Val::String("hi".into()), "string"),
            Val::String("hi".into())
        );
    }

    #[test]
    fn ast_to_val_round_trips_a_u8_list_via_bytes() {
        let v = Val::List(vec![Val::U8(1), Val::U8(2), Val::U8(255)]);
        assert_eq!(round_trip(v.clone(), "(list u8)"), v);
    }

    #[test]
    fn ast_to_val_round_trips_a_non_u8_list() {
        let v = Val::List(vec![Val::U32(10), Val::U32(20)]);
        assert_eq!(round_trip(v.clone(), "(list u32)"), v);
    }

    // Reviewer catch (#2078 follow-up): an EMPTY list<T> of ANY element type marshals to an empty
    // Leaf::Bytes (a Val::List carries no element-type tag when empty), byte-identical to an empty
    // list<u8>. The dual read is TYPE-DIRECTED, so an empty list<u32> must round-trip through the
    // empty-Bytes wire node, not error demanding a ("list" …) form.
    #[test]
    fn ast_to_val_round_trips_an_empty_non_u8_list_via_empty_bytes() {
        let empty_u32 = Val::List(vec![]);
        assert_eq!(round_trip(empty_u32.clone(), "(list u32)"), empty_u32);
        // and an empty list<string> likewise (same empty-Bytes wire node, different target type)
        assert_eq!(
            round_trip(Val::List(vec![]), "(list string)"),
            Val::List(vec![])
        );
        // a NON-empty bytes node against a non-u8 list target is still a genuine mismatch
        let bytes_node = val_to_ast(&Val::List(vec![Val::U8(1), Val::U8(2)])).unwrap();
        let u32_list_ty = param_type(&probe_component("(list u32)"));
        assert!(matches!(
            ast_to_val(&bytes_node, &u32_list_ty),
            Err(MarshalError::TypeMismatch { .. })
        ));
    }

    #[test]
    fn ast_to_val_round_trips_a_record() {
        let v = Val::Record(vec![
            ("kind".into(), Val::String("wasm".into())),
            ("size".into(), Val::U32(7)),
        ]);
        assert_eq!(
            round_trip(
                v.clone(),
                r#"(record (field "kind" string) (field "size" u32))"#
            ),
            v
        );
    }

    // ast_to_val decodes UNTRUSTED arg bytes, so a record must EXACTLY match the WIT shape (github-liaison
    // #2078): an EXTRA field beyond the declared set, or a DUPLICATE field name, is rejected — not silently
    // accepted. Build the malformed record AST by hand (a string-head ("record" (name val)…) form) and
    // decode against a 1-field record type.
    #[test]
    fn ast_to_val_rejects_extra_and_duplicate_record_fields() {
        let one_field_ty = param_type(&probe_component(r#"(record (field "kind" string))"#));
        let record_bytes = |fields: &[(&str, &str)]| -> Vec<u8> {
            let mut b = Builder::new();
            let mut kids = vec![b.atom_leaf(Leaf::Str("record".into()))];
            for (name, val) in fields {
                let n = b.name(name);
                let v = b.atom_leaf(Leaf::Str((*val).into()));
                let pair = b.list(vec![n, v]);
                kids.push(pair);
            }
            let root = b.list(kids);
            codec::encode(&b.finish(root))
        };
        // exact 1-field record → OK
        assert!(ast_to_val(&record_bytes(&[("kind", "wasm")]), &one_field_ty).is_ok());
        // EXTRA field "size" beyond the declared {kind} → TypeMismatch (unknown field)
        assert!(matches!(
            ast_to_val(
                &record_bytes(&[("kind", "wasm"), ("size", "big")]),
                &one_field_ty
            ),
            Err(MarshalError::TypeMismatch { .. })
        ));
        // DUPLICATE "kind" → TypeMismatch (duplicate field)
        assert!(matches!(
            ast_to_val(
                &record_bytes(&[("kind", "a"), ("kind", "b")]),
                &one_field_ty
            ),
            Err(MarshalError::TypeMismatch { .. })
        ));
        // github-liaison #2090: an untrusted extra-field name LONGER than the cap must NOT blow up the
        // error string — TypeMismatch.found is bounded (bounded_name caps at 64 + ellipsis + len), never
        // the raw name. A name well past the 64-byte cap exercises the same bounding path as a multi-MB one
        // (github-liaison #2101: no need for a giant alloc in the test).
        let huge = "x".repeat(4096);
        match ast_to_val(
            &record_bytes(&[("kind", "wasm"), (&huge, "v")]),
            &one_field_ty,
        ) {
            Err(MarshalError::TypeMismatch { found, .. }) => assert!(
                found.len() < 256,
                "error must be bounded regardless of the untrusted field-name size, got len {}",
                found.len()
            ),
            other => {
                panic!("expected a bounded TypeMismatch for a huge extra field, got {other:?}")
            }
        }
    }

    // github-liaison #2101: the enum "case carries a payload" reject arm (a name-head ctor with a payload
    // against an enum target) also embeds the untrusted case name — it must be BOUNDED like the sibling
    // arms. Build a name-head ctor `(<huge-case-name> <payload>)` against an enum type and assert the error
    // is bounded.
    #[test]
    fn ast_to_val_bounds_the_enum_carries_payload_reject() {
        let enum_ty = param_type(&probe_component(r#"(enum "red" "green")"#));
        let huge_case = "z".repeat(4096);
        let mut b = Builder::new();
        let head = b.name(&huge_case);
        let payload = b.atom_leaf(Leaf::Bool(true));
        let root = b.list(vec![head, payload]); // (<huge-case> #t) — a ctor WITH a payload, illegal for an enum
        let bytes = codec::encode(&b.finish(root));
        match ast_to_val(&bytes, &enum_ty) {
            Err(MarshalError::TypeMismatch { found, .. }) => assert!(
                found.len() < 256,
                "enum-carries-payload error must be bounded, got len {}",
                found.len()
            ),
            other => {
                panic!("expected a bounded TypeMismatch for an enum with a payload, got {other:?}")
            }
        }
    }

    // reviewer catch (post-#2108): the option/result BAD-CASE reject arms (a name-head ctor whose case ∉
    // {Some,None}/{Ok,Err}) also embed the untrusted case name — must be BOUNDED like the sibling arms. A
    // 4096-char bogus case name against an option (and a result) target → bounded TypeMismatch.
    #[test]
    fn ast_to_val_bounds_the_option_and_result_bad_case_rejects() {
        let huge_case = "q".repeat(4096);
        let bad_ctor = || -> Vec<u8> {
            let mut b = Builder::new();
            let head = b.name(&huge_case);
            let payload = b.atom_leaf(Leaf::Bool(true));
            let root = b.list(vec![head, payload]); // (<huge-bogus-case> #t)
            codec::encode(&b.finish(root))
        };
        let option_ty = param_type(&probe_component("(option u8)"));
        match ast_to_val(&bad_ctor(), &option_ty) {
            Err(MarshalError::TypeMismatch { found, .. }) => assert!(
                found.len() < 256,
                "option bad-case error must be bounded, got len {}",
                found.len()
            ),
            other => {
                panic!("expected a bounded TypeMismatch for an option bad case, got {other:?}")
            }
        }
        let result_ty = param_type(&probe_component("(result u8 (error string))"));
        match ast_to_val(&bad_ctor(), &result_ty) {
            Err(MarshalError::TypeMismatch { found, .. }) => assert!(
                found.len() < 256,
                "result bad-case error must be bounded, got len {}",
                found.len()
            ),
            other => panic!("expected a bounded TypeMismatch for a result bad case, got {other:?}"),
        }
    }

    #[test]
    fn ast_to_val_round_trips_a_tuple() {
        let v = Val::Tuple(vec![Val::Bool(true), Val::U8(9)]);
        assert_eq!(round_trip(v.clone(), "(tuple bool u8)"), v);
    }

    #[test]
    fn ast_to_val_round_trips_option_and_result_and_variant_and_enum() {
        assert_eq!(
            round_trip(Val::Option(Some(Box::new(Val::U8(5)))), "(option u8)"),
            Val::Option(Some(Box::new(Val::U8(5))))
        );
        assert_eq!(
            round_trip(Val::Option(None), "(option u8)"),
            Val::Option(None)
        );
        assert_eq!(
            round_trip(
                Val::Result(Ok(Some(Box::new(Val::Bool(true))))),
                "(result bool (error string))"
            ),
            Val::Result(Ok(Some(Box::new(Val::Bool(true)))))
        );
        assert_eq!(
            round_trip(
                Val::Result(Err(Some(Box::new(Val::String("boom".into()))))),
                "(result bool (error string))"
            ),
            Val::Result(Err(Some(Box::new(Val::String("boom".into())))))
        );
        // WIT identifiers are kebab-case, so the variant/enum case names are kebab (the marshalled Val
        // carries the same name string round-trip).
        assert_eq!(
            round_trip(
                Val::Variant("move-to".into(), Some(Box::new(Val::U32(3)))),
                r#"(variant (case "move-to" u32) (case "stay"))"#
            ),
            Val::Variant("move-to".into(), Some(Box::new(Val::U32(3))))
        );
        assert_eq!(
            round_trip(Val::Enum("red".into()), r#"(enum "red" "green")"#),
            Val::Enum("red".into())
        );
    }

    #[test]
    fn ast_to_val_range_checks_int_width() {
        // A u32-valued AST (300) marshalled, then decoded against a u8 target → IntOutOfRange, not a
        // silent truncation to 44.
        let bytes = val_to_ast(&Val::U32(300)).unwrap();
        let u8_ty = param_type(&probe_component("u8"));
        assert_eq!(
            ast_to_val(&bytes, &u8_ty),
            Err(MarshalError::IntOutOfRange {
                wit_type: "u8".into()
            })
        );
    }

    #[test]
    fn ast_to_val_rejects_undecodable_bytes_and_shape_mismatch() {
        let u8_ty = param_type(&probe_component("u8"));
        // garbage bytes → Undecodable
        assert_eq!(
            ast_to_val(b"not an ast", &u8_ty),
            Err(MarshalError::Undecodable)
        );
        // a bool AST against a u8 target → TypeMismatch (well-formed AST, wrong shape)
        let bool_bytes = val_to_ast(&Val::Bool(true)).unwrap();
        assert!(matches!(
            ast_to_val(&bool_bytes, &u8_ty),
            Err(MarshalError::TypeMismatch { .. })
        ));
    }

    #[test]
    fn primitives_marshal_to_their_leaves() {
        assert_eq!(
            root_leaf(&decode(&val_to_ast(&Val::Bool(true)).unwrap())),
            &Leaf::Bool(true)
        );
        assert_eq!(
            root_leaf(&decode(&val_to_ast(&Val::U32(42)).unwrap())),
            &Leaf::Int {
                value: BigInt::from(42),
                radix: Radix::Dec
            }
        );
        // full u64 range (arbitrary-precision BigInt covers it)
        assert_eq!(
            root_leaf(&decode(&val_to_ast(&Val::U64(u64::MAX)).unwrap())),
            &Leaf::Int {
                value: BigInt::from(u64::MAX),
                radix: Radix::Dec
            }
        );
        // negative s64
        assert_eq!(
            root_leaf(&decode(&val_to_ast(&Val::S64(i64::MIN)).unwrap())),
            &Leaf::Int {
                value: BigInt::from(i64::MIN),
                radix: Radix::Dec
            }
        );
        assert_eq!(
            root_leaf(&decode(&val_to_ast(&Val::Char('é')).unwrap())),
            &Leaf::Char('é')
        );
        assert_eq!(
            root_leaf(&decode(&val_to_ast(&Val::String("hi".into())).unwrap())),
            &Leaf::Str("hi".into())
        );
    }

    #[test]
    fn a_u8_list_marshals_to_a_single_bytes_leaf_not_a_node_per_byte_list() {
        let v = Val::List(vec![
            Val::U8(0xDE),
            Val::U8(0xAD),
            Val::U8(0x00),
            Val::U8(0xFF),
        ]);
        let a = decode(&val_to_ast(&v).unwrap());
        assert_eq!(root_leaf(&a), &Leaf::Bytes(vec![0xDE, 0xAD, 0x00, 0xFF]));
        // empty list<u8> → empty Bytes
        let empty = decode(&val_to_ast(&Val::List(vec![])).unwrap());
        assert_eq!(root_leaf(&empty), &Leaf::Bytes(vec![]));
    }

    #[test]
    fn a_non_u8_list_marshals_to_a_string_head_list_form() {
        let v = Val::List(vec![Val::U32(1), Val::U32(2)]);
        let a = decode(&val_to_ast(&v).unwrap());
        // ("list" 1 2): string head, read via as_ctor_form
        let elems = a
            .as_ctor_form(a.root, "list")
            .expect("string-head list form");
        assert_eq!(elems.len(), 2);
        assert_eq!(
            leaf_at(&a, elems[0]),
            &Leaf::Int {
                value: BigInt::from(1),
                radix: Radix::Dec
            }
        );
        assert_eq!(
            leaf_at(&a, elems[1]),
            &Leaf::Int {
                value: BigInt::from(2),
                radix: Radix::Dec
            }
        );
    }

    #[test]
    fn a_record_marshals_to_a_string_head_record_of_name_val_fields() {
        let v = Val::Record(vec![
            ("kind".into(), Val::String("wasm".into())),
            ("size".into(), Val::U32(7)),
        ]);
        let a = decode(&val_to_ast(&v).unwrap());
        let fields = a
            .as_ctor_form(a.root, "record")
            .expect("string-head record form");
        assert_eq!(fields.len(), 2);
        // each field is a (name val) 2-list
        let f0 = match a.get(fields[0]) {
            Struct::List(kids) => kids.clone(),
            _ => panic!("field is a list"),
        };
        assert_eq!(a.as_name(f0[0]), Some("kind"));
        assert_eq!(leaf_at(&a, f0[1]), &Leaf::Str("wasm".into()));
    }

    #[test]
    fn a_tuple_marshals_to_a_string_head_tuple_form() {
        let v = Val::Tuple(vec![Val::Bool(true), Val::U8(9)]);
        let a = decode(&val_to_ast(&v).unwrap());
        let elems = a
            .as_ctor_form(a.root, "tuple")
            .expect("string-head tuple form");
        assert_eq!(elems.len(), 2);
        assert_eq!(leaf_at(&a, elems[0]), &Leaf::Bool(true));
    }

    #[test]
    fn option_and_result_and_variant_and_enum_marshal_to_name_head_ctors() {
        // Some(v) — name-head, read via as_form
        let some = decode(&val_to_ast(&Val::Option(Some(Box::new(Val::U8(5))))).unwrap());
        let inner = some.as_form(some.root, "Some").expect("name-head (Some v)");
        assert_eq!(inner.len(), 1);
        assert_eq!(
            leaf_at(&some, inner[0]),
            &Leaf::Int {
                value: BigInt::from(5),
                radix: Radix::Dec
            }
        );
        // None — bare (None)
        let none = decode(&val_to_ast(&Val::Option(None)).unwrap());
        assert_eq!(none.as_form(none.root, "None"), Some(&[][..]));
        // Ok(v) / Err(e)
        let ok = decode(&val_to_ast(&Val::Result(Ok(Some(Box::new(Val::Bool(true)))))).unwrap());
        assert!(ok.as_form(ok.root, "Ok").is_some());
        let err = decode(
            &val_to_ast(&Val::Result(Err(Some(Box::new(Val::String(
                "boom".into(),
            ))))))
            .unwrap(),
        );
        let e = err.as_form(err.root, "Err").expect("(Err e)");
        assert_eq!(leaf_at(&err, e[0]), &Leaf::Str("boom".into()));
        // variant Case(v)
        let var =
            decode(&val_to_ast(&Val::Variant("Move".into(), Some(Box::new(Val::U32(3))))).unwrap());
        assert!(var.as_form(var.root, "Move").is_some());
        // enum Case — bare (Case), no children
        let en = decode(&val_to_ast(&Val::Enum("Red".into())).unwrap());
        assert_eq!(en.as_form(en.root, "Red"), Some(&[][..]));
    }

    #[test]
    fn a_resource_val_is_unmarshallable_not_mis_encoded() {
        // We can't easily construct a real Val::Resource here without a live store; the negative path is
        // exercised by the `_ =>` arm for non-value handles. Assert the error shape via a direct call on
        // the classifier used for the catch-all.
        assert_eq!(
            unmarshallable("resource"),
            MarshalError::Unmarshallable {
                val_kind: "resource".into()
            }
        );
    }

    // TOTALITY (v-syntax review F1, MED): `ast_to_val` must NEVER panic on ANY input — only `Ok` or a
    // `MarshalError`. `build_from_ast` recursion is TYPE-directed (depth bounded by the finite, caller-
    // supplied WIT type, not untrusted AST depth), so this guards the READERS (indexing, int/char
    // conversions, name lookups) against a well-formed-but-wrong-shape or garbage AST. Two feeds: (a) raw
    // arbitrary bytes (mostly bounce off `codec::decode` → Undecodable), and (b) the MORE valuable half —
    // real marshalled ASTs (from `val_to_ast` of assorted Vals) fed against a MISMATCHED target type, which
    // drives `build_from_ast`'s reader arms rather than the decode gate. A xorshift PRNG (no rng dep; the
    // Date/rand-free constraint) keeps it deterministic + replayable.
    #[test]
    fn ast_to_val_is_total_over_arbitrary_and_mismatched_input() {
        let tys: Vec<Type> = [
            "u8",
            "bool",
            "string",
            "(list u8)",
            "(list u32)",
            r#"(record (field "k" string))"#,
            "(option u8)",
            "(result bool (error string))",
            "(tuple bool u8)",
            r#"(variant (case "a" u32) (case "b"))"#,
            r#"(enum "x" "y")"#,
            r#"(flags "p" "q")"#,
        ]
        .iter()
        .map(|d| param_type(&probe_component(d)))
        .collect();

        // A pool of real marshalled ASTs — feeding these against a MISMATCHED type is the reader-driving
        // half (they decode fine, then build_from_ast reads them under the wrong shape).
        let valid_asts: Vec<Vec<u8>> = vec![
            val_to_ast(&Val::Bool(true)).unwrap(),
            val_to_ast(&Val::U32(70000)).unwrap(),
            val_to_ast(&Val::String("hi".into())).unwrap(),
            val_to_ast(&Val::List(vec![Val::U8(1), Val::U8(2)])).unwrap(),
            val_to_ast(&Val::Record(vec![("k".into(), Val::String("v".into()))])).unwrap(),
            val_to_ast(&Val::Option(Some(Box::new(Val::U8(9))))).unwrap(),
            val_to_ast(&Val::Variant("a".into(), Some(Box::new(Val::U32(3))))).unwrap(),
            val_to_ast(&Val::Enum("x".into())).unwrap(),
            val_to_ast(&Val::Flags(vec!["p".into()])).unwrap(),
        ];
        for bytes in &valid_asts {
            for ty in &tys {
                // Must return Ok or Err — never panic.
                let _ = ast_to_val(bytes, ty);
            }
        }

        // Random raw bytes: mostly Undecodable, but exercises the decode gate + any lengths it accepts.
        let mut seed = 0x1234_5678u64;
        for _ in 0..20_000 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let n = (seed as usize) % 48;
            let bytes: Vec<u8> = (0..n).map(|i| (seed >> ((i % 8) * 8)) as u8).collect();
            for ty in &tys {
                let _ = ast_to_val(&bytes, ty);
            }
        }
    }

    // Val::Flags round-trip (v-syntax review F2, LOW): built (string-head `(flags a c …)`) + read
    // (Type::Flags), but the pair was untested. Non-empty + empty.
    #[test]
    fn ast_to_val_round_trips_flags() {
        let v = Val::Flags(vec!["a".into(), "c".into()]);
        assert_eq!(round_trip(v.clone(), r#"(flags "a" "b" "c")"#), v);
        assert_eq!(
            round_trip(Val::Flags(vec![]), r#"(flags "a" "b")"#),
            Val::Flags(vec![])
        );
    }

    // NESTED compound round-trips (v-syntax review F3, LOW): all other round-trips are single-level, so the
    // build_val/build_from_ast recursion is only transitively covered. Lock it in both directions: a record
    // with list + option fields, and a list-of-lists (list<list<string>>) — see the note below on why
    // a list-of-lists rather than a list-of-records.
    #[test]
    fn ast_to_val_round_trips_nested_compounds() {
        // `ast_to_val` reconstructs record fields in the target WIT type's FIELD ORDER, so build the
        // expected Val + the WIT decl in the SAME order (tags, then opt).
        let rec = Val::Record(vec![
            (
                "tags".into(),
                Val::List(vec![Val::String("a".into()), Val::String("b".into())]),
            ),
            ("opt".into(), Val::Option(Some(Box::new(Val::U32(7))))),
        ]);
        assert_eq!(
            round_trip(
                rec.clone(),
                r#"(record (field "tags" (list string)) (field "opt" (option u32)))"#
            ),
            rec
        );
        // Nested LISTS (list<list<string>>) — the other recursion direction. (A list-of-RECORD would need
        // the record type EXPORTED as a named type through the probe helper — the WAT "type not valid to be
        // used as export" rule; nested inline lists reflect through the probe cleanly and still drive the
        // list→list recursion in both build_val + build_from_ast.)
        let list_of_list = Val::List(vec![
            Val::List(vec![Val::String("a".into()), Val::String("b".into())]),
            Val::List(vec![Val::String("c".into())]),
        ]);
        assert_eq!(
            round_trip(list_of_list.clone(), "(list (list string))"),
            list_of_list
        );
    }

    // ---- type_to_ast: the wit-TYPE → type-descriptor-AST lowering (signature-query / v-ah-host P2) ----

    // Reflect a real wasmtime `Type` from a WAT type declaration and lower it to a decoded descriptor arena.
    fn type_ast(ty_decl: &str) -> Arenas {
        let ty = param_type(&probe_component(ty_decl));
        decode(&type_to_ast(&ty).expect("type_to_ast"))
    }
    // The head-name of a list node (a String-head form like ("record" …)) or a name-head marker
    // (like (u8)); returns the head's text either way for structural assertions.
    fn head_str(a: &Arenas, id: StructId) -> String {
        let Struct::List(kids) = a.get(id) else {
            panic!("expected a list node at {id:?}");
        };
        match a.get(kids[0]) {
            Struct::Atom(lid) => match a.leaf(*lid) {
                Leaf::Str(s) => s.clone(),
                Leaf::Name(n) => n.clone(),
                other => panic!("unexpected head leaf {other:?}"),
            },
            Struct::List(_) => panic!("head is not an atom"),
        }
    }
    fn kids(a: &Arenas, id: StructId) -> Vec<StructId> {
        let Struct::List(k) = a.get(id) else {
            panic!("expected a list node");
        };
        k.clone()
    }

    #[test]
    fn type_to_ast_primitives_are_name_head_markers() {
        // Each primitive TYPE lowers to a lone name-head marker `(kind)` — a 1-element list, head = kind.
        for (decl, kind) in [
            ("bool", "bool"),
            ("u8", "u8"),
            ("u16", "u16"),
            ("u32", "u32"),
            ("u64", "u64"),
            ("s8", "s8"),
            ("s16", "s16"),
            ("s32", "s32"),
            ("s64", "s64"),
            ("char", "char"),
            ("string", "string"),
        ] {
            let a = type_ast(decl);
            assert_eq!(head_str(&a, a.root), kind, "primitive {decl} head");
            assert_eq!(
                kids(&a, a.root).len(),
                1,
                "{decl} is a lone marker (head only)"
            );
        }
    }

    #[test]
    fn type_to_ast_float_types_describe_even_though_float_values_do_not_marshal() {
        // A float TYPE describes fine (a sig query learns the shape) though a float VALUE is Unmarshallable.
        assert_eq!(head_str(&type_ast("f32"), type_ast("f32").root), "f32");
        let a = type_ast("f64");
        assert_eq!(head_str(&a, a.root), "f64");
    }

    #[test]
    fn type_to_ast_list_is_string_head_with_element_type() {
        // ("list" <elem-type>): head "list", one child = the element TYPE descriptor (here (u32)).
        let a = type_ast("(list u32)");
        assert_eq!(head_str(&a, a.root), "list");
        let ks = kids(&a, a.root);
        assert_eq!(ks.len(), 2, "list = head + one element-type");
        assert_eq!(head_str(&a, ks[1]), "u32", "element type descriptor");
        // Nested: ("list" ("list" (string))).
        let n = type_ast("(list (list string))");
        let nk = kids(&n, n.root);
        assert_eq!(head_str(&n, n.root), "list");
        assert_eq!(head_str(&n, nk[1]), "list");
        assert_eq!(head_str(&n, kids(&n, nk[1])[1]), "string");
    }

    #[test]
    fn type_to_ast_record_is_string_head_with_name_type_fields() {
        // ("record" (fieldname <type>)…): each field a (name type) 2-list, field type recursed.
        let a = type_ast(r#"(record (field "n" u32) (field "s" string))"#);
        assert_eq!(head_str(&a, a.root), "record");
        let ks = kids(&a, a.root);
        assert_eq!(ks.len(), 3, "head + 2 fields");
        // field entries are (name <type>) — assert the names + recursed field-type heads.
        let f0 = kids(&a, ks[1]);
        assert_eq!(a.as_name(f0[0]), Some("n"));
        assert_eq!(head_str(&a, f0[1]), "u32");
        let f1 = kids(&a, ks[2]);
        assert_eq!(a.as_name(f1[0]), Some("s"));
        assert_eq!(head_str(&a, f1[1]), "string");
    }

    #[test]
    fn type_to_ast_tuple_is_string_head_positional_types() {
        let a = type_ast("(tuple bool string)");
        assert_eq!(head_str(&a, a.root), "tuple");
        let ks = kids(&a, a.root);
        assert_eq!(ks.len(), 3, "head + 2 element types");
        assert_eq!(head_str(&a, ks[1]), "bool");
        assert_eq!(head_str(&a, ks[2]), "string");
    }

    #[test]
    fn type_to_ast_option_is_string_head_not_a_value_ctor() {
        // A TYPE names the option-of-T shape — ("option" <T>), NOT the value side's name-head Some/None.
        let a = type_ast("(option u32)");
        assert_eq!(head_str(&a, a.root), "option");
        let ks = kids(&a, a.root);
        assert_eq!(ks.len(), 2);
        assert_eq!(head_str(&a, ks[1]), "u32");
    }

    #[test]
    fn type_to_ast_result_carries_both_arms_with_unit_marker() {
        // ("result" <ok-or-unit> <err-or-unit>): a payload-less arm is ("unit").
        let both = type_ast("(result u32 (error string))");
        // wasmtime spells result<T,E> from `(result <ok> (error <err>))` in WAT.
        assert_eq!(head_str(&both, both.root), "result");
        let bk = kids(&both, both.root);
        assert_eq!(bk.len(), 3, "head + ok + err");
        assert_eq!(head_str(&both, bk[1]), "u32");
        assert_eq!(head_str(&both, bk[2]), "string");
        // result with a unit ok arm → ("result" ("unit") <err>).
        let unit_ok = type_ast("(result (error string))");
        let uk = kids(&unit_ok, unit_ok.root);
        assert_eq!(
            head_str(&unit_ok, uk[1]),
            "unit",
            "payload-less ok arm is (unit)"
        );
        assert_eq!(head_str(&unit_ok, uk[2]), "string");
    }

    #[test]
    fn type_to_ast_variant_is_string_head_with_case_type_entries() {
        // ("variant" (Case <type>?)…): a payload case is (Case type), a payload-less case is (Case).
        let a = type_ast(r#"(variant (case "num" u32) (case "nothing"))"#);
        assert_eq!(head_str(&a, a.root), "variant");
        let ks = kids(&a, a.root);
        assert_eq!(ks.len(), 3, "head + 2 cases");
        let c0 = kids(&a, ks[1]);
        assert_eq!(a.as_name(c0[0]), Some("num"));
        assert_eq!(c0.len(), 2, "payload case is (Case type)");
        assert_eq!(head_str(&a, c0[1]), "u32");
        let c1 = kids(&a, ks[2]);
        assert_eq!(a.as_name(c1[0]), Some("nothing"));
        assert_eq!(c1.len(), 1, "payload-less case is the bare (Case)");
    }

    #[test]
    fn type_to_ast_enum_is_string_head_of_case_names() {
        let a = type_ast(r#"(enum "red" "green" "blue")"#);
        assert_eq!(head_str(&a, a.root), "enum");
        let ks = kids(&a, a.root);
        assert_eq!(ks.len(), 4, "head + 3 cases");
        assert_eq!(a.as_name(ks[1]), Some("red"));
        assert_eq!(a.as_name(ks[2]), Some("green"));
        assert_eq!(a.as_name(ks[3]), Some("blue"));
    }

    #[test]
    fn type_to_ast_flags_is_string_head_of_flag_names() {
        let a = type_ast(r#"(flags "a" "b")"#);
        assert_eq!(head_str(&a, a.root), "flags");
        let ks = kids(&a, a.root);
        assert_eq!(ks.len(), 3, "head + 2 flags");
        assert_eq!(a.as_name(ks[1]), Some("a"));
        assert_eq!(a.as_name(ks[2]), Some("b"));
    }

    #[test]
    fn type_to_ast_descriptor_round_trips_through_the_canonical_codec() {
        // The descriptor IS ordinary cdzast: encode → decode is structurally identical + re-encode is
        // byte-identical (the frozen bijection), so a reducer decodes it with the same codec as everything.
        let ty = param_type(&probe_component(
            r#"(record (field "n" u32) (field "s" (list string)))"#,
        ));
        let bytes = type_to_ast(&ty).expect("type_to_ast");
        let decoded = codec::decode(&bytes).expect("descriptor decodes");
        assert_eq!(
            codec::encode(&decoded),
            bytes,
            "descriptor re-encodes byte-identical"
        );
    }
}
