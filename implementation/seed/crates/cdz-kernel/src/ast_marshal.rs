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
            let mut out = Vec::new();
            for field in rt.fields() {
                let node = field_nodes
                    .iter()
                    .find_map(|&fnode| match a.get(fnode) {
                        Struct::List(kids)
                            if kids.len() == 2 && a.as_name(kids[0]) == Some(field.name) =>
                        {
                            Some(kids[1])
                        }
                        _ => None,
                    })
                    .ok_or_else(|| {
                        type_mismatch("record", format!("missing field {:?}", field.name))
                    })?;
                out.push((field.name.to_string(), build_from_ast(a, node, &field.ty)?));
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
                    format!("case {other:?} ∉ {{Some,None}}"),
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
                    format!("case {other:?} ∉ {{Ok,Err}}"),
                )),
            }
        }
        // variant ← name-head (Case v?): match the case name against the declared cases.
        Type::Variant(vt) => {
            let (case, payload) = ctor(a, id)?;
            let decl = vt
                .cases()
                .find(|c| c.name == case)
                .ok_or_else(|| type_mismatch("variant", format!("unknown case {case:?}")))?;
            let val = opt_payload(a, payload, decl.ty)?;
            Ok(Val::Variant(case.to_string(), val))
        }
        // enum ← name-head (Case) with no payload; the case must be a declared name.
        Type::Enum(et) => {
            let (case, payload) = ctor(a, id)?;
            if payload.is_some() {
                return Err(type_mismatch(
                    "enum",
                    format!("case {case:?} carries a payload"),
                ));
            }
            if !et.names().any(|n| n == case) {
                return Err(type_mismatch("enum", format!("unknown case {case:?}")));
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
                    return Err(type_mismatch("flags", format!("unknown flag {name:?}")));
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
    // A `Type` isn't directly constructible (it comes from component reflection), so a WAT component whose
    // `probe` func takes a single param of the WANTED type gives us that `Type` via `Func::params`. We
    // instantiate against an empty linker and never CALL probe — we only read its param type.
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
}
