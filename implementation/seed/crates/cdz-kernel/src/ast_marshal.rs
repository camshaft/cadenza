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
//! - `list<T>` (T≠u8) → a NAME-HEAD `(list elem…)` form;
//! - `record{f: v…}` → `(record (= f v)…)` (NAME head; fields SORTED by name; each field a `(= name value)`
//!   3-list — the canonical deterministic-value-form, record-type Phase B, operator-ruled 2026-08-09);
//! - `tuple<v…>` → `(tuple v…)` (name head);
//! - `option<T>` → NAME-HEAD ctor `(Some v)` / `(None)`; `result<T,E>` → `(Ok v)` / `(Err e)`;
//! - `variant{Case(v)}` → NAME-HEAD ctor `(Case v)`; `enum{Case}` → `(Case)` (name head, no children);
//! - `flags{A,B}` → `(flags A B…)` (name head, set-flag names).
//!
//! Every VALUE form is NAME-head — this is cadenza's canonical deterministic-value-form, the exact wire
//! `value-encode` emits and `value-decode` (op 90) consumes (a Str-head aggregate is REJECTED by
//! value-decode as a NULL: `doc_atom_name` matches only a `Name` leaf). `rcdzc`'s `const_value_ast` builds
//! this same Name-head form, so the marshal is byte-aligned with the compiler. (The TYPE-DESCRIPTOR
//! vocabulary of [`build_type`] below is a DISTINCT, str-head form — a type NAMES a shape, it is not a
//! value, and it is decoded by a different consumer.) The READ side is tolerant: aggregates read via
//! `form` (accepts BOTH name- and str-head spellings) and records accept the legacy `(name value)` 2-list
//! alongside the canonical `(= name value)` — the same migration tolerance value-decode keeps.

use cadenza_ast::ast::{Arenas, Builder, Leaf, Radix, Struct, StructId, WitDir};
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
    Unmarshallable { val_kind: std::sync::Arc<str> },
    /// [`ast_to_val`]: the supplied bytes are not a decodable tagged-AST (`codec::decode` returned None) —
    /// a torn/corrupt arg payload, distinct from a well-formed AST of the wrong shape.
    Undecodable,
    /// [`ast_to_val`]: the decoded AST does not match the target WIT [`Type`] — the shape the caller's
    /// component expects (e.g. a record form where the type is a variant, or a missing field/case).
    /// `expected` names the WIT type kind; `found` is a BOUNDED shape hint (never the full untrusted AST).
    TypeMismatch {
        expected: std::sync::Arc<str>,
        found: std::sync::Arc<str>,
    },
    /// [`ast_to_val`]: an integer leaf is outside the target WIT integer type's range (e.g. a `300` for a
    /// `u8`, or a negative for a `u32`). The AST carries an arbitrary-precision `BigInt`, so a value that
    /// doesn't fit the declared width is a loud error, not a silent truncation.
    IntOutOfRange { wit_type: std::sync::Arc<str> },
    /// [`ast_to_val`]: the target WIT [`Type`] is one the arg-marshalling doesn't build (a resource
    /// handle, future/stream, error-context, or float pending the decimal slice) — the dual of
    /// [`MarshalError::Unmarshallable`] on the type side.
    UnsupportedType { wit_type: std::sync::Arc<str> },
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
/// documented at the module head (NAME-head for record/tuple/list/flags AND for the option/result/
/// variant/enum ctors, a lone `Leaf` for primitives, `Leaf::Bytes` for `list<u8>`; a record's fields are
/// sorted by name and each spelled `(= name value)` — the canonical deterministic-value-form).
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
        Val::String(s) => b.atom_leaf(Leaf::Str(s.clone().into())),
        // list<u8> is the ONE list special-cased to a single Bytes leaf (blob-optimized wire, #2063);
        // any other list<T> is a NAME-head (list elem…) form of per-element nodes.
        Val::List(items) => {
            if let Some(bytes) = as_u8_list(items) {
                b.atom_leaf(Leaf::Bytes(bytes.into()))
            } else {
                let mut children = vec![b.name("list")];
                for it in items.iter() {
                    let c = build_val(b, it)?;
                    children.push(c);
                }
                b.list(children)
            }
        }
        // record → (record (= name value)…): NAME head, fields SORTED by name (lexicographic), each field a
        // 3-list (= name value). This is the canonical deterministic-value-form value-encode produces and
        // value-decode consumes (record-type Phase B, operator-ruled 2026-08-09). The wasmtime `Val` carries
        // fields in WIT declaration order, so sort them here to match the canonical (BTreeMap) order.
        Val::Record(fields) => {
            let mut children = vec![b.name("record")];
            let mut sorted: Vec<&(String, Val)> = fields.iter().collect();
            sorted.sort_by(|x, y| x.0.cmp(&y.0));
            for (name, v) in sorted {
                let eq = b.name("=");
                let name_node = b.name(name);
                let val_node = build_val(b, v)?;
                let field = b.list(vec![eq, name_node, val_node]);
                children.push(field);
            }
            b.list(children)
        }
        // tuple → (tuple v…): name head, positional.
        Val::Tuple(items) => {
            let mut children = vec![b.name("tuple")];
            for it in items.iter() {
                let c = build_val(b, it)?;
                children.push(c);
            }
            b.list(children)
        }
        // option → NAME-head ctor (Some v) / (None unit). A nullary variant's payload is the unit atom
        // (`Core::Unit` renders as the lowercase `unit` name leaf) — value-decode's Sum arm requires each
        // variant to have EXACTLY two children (head + one payload node), so a bare (None) decodes to NULL.
        Val::Option(opt) => match opt {
            Some(v) => {
                let head = b.name("Some");
                let inner = build_val(b, v)?;
                b.list(vec![head, inner])
            }
            None => {
                let head = b.name("None");
                let unit = b.name("unit");
                b.list(vec![head, unit])
            }
        },
        // result → NAME-head ctor (Ok v) / (Err e). A payload-less Ok/Err (result<_, _> with a unit arm)
        // is the nullary two-element (Ok unit) / (Err unit) via build_ctor.
        Val::Result(res) => match res {
            Ok(v) => build_ctor(b, "Ok", v.as_deref())?,
            Err(e) => build_ctor(b, "Err", e.as_deref())?,
        },
        // variant → NAME-head ctor (Case v) (or (Case unit) for a payload-less case).
        Val::Variant(case, payload) => build_ctor(b, case, payload.as_deref())?,
        // enum → NAME-head ctor (Case unit): a nullary variant carries the unit atom as its payload.
        Val::Enum(case) => {
            let head = b.name(case);
            let unit = b.name("unit");
            b.list(vec![head, unit])
        }
        // flags → (flags A B…): name head, one Name per SET flag.
        Val::Flags(names) => {
            let mut children = vec![b.name("flags")];
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

/// Build a NAME-head ctor form `(Name payload)` — the shared shape for option/result/variant cases.
/// A nullary (payload-less) case → the two-element `(Name unit)` (its payload is the unit atom, the
/// lowercase `unit` name leaf); `Some` → `(Name inner)`. value-decode requires exactly two children.
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
        None => {
            let unit = b.name("unit");
            b.list(vec![head, unit])
        }
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

/// The stable STRUCTURAL identity of a WIT type — the content-hash of its type-descriptor AST
/// ([`type_to_ast`]). Two types with the SAME structure hash equal regardless of any name; two
/// DIFFERENT structures hash differently. This is the foundation of SCHEMA-BASED effect identity
/// (operator seq367: "identify effects by their SCHEMA, not their string name"): an effect's identity
/// is the schema-hash of its request/payload type, so a built-in and a userspace effect are identified
/// UNIFORMLY by shape — no closed enum, no arbitrary family string as the key. Pure over the descriptor
/// bytes ([`Hash::of`] = blake3, the one content-address algorithm), so the same shape yields the same
/// id everywhere (kernel, host, on the wire). A type with no value-crossing form (resource/future/
/// stream/error-context) has no schema-hash — [`MarshalError::UnsupportedType`], same as `type_to_ast`.
pub fn schema_hash(ty: &Type) -> Result<crate::hash::Hash, MarshalError> {
    Ok(crate::hash::Hash::of(&type_to_ast(ty)?))
}

/// The stable STRUCTURAL identity of an EFFECT — the content-hash of its schema tree
/// `(effect <name> (op <op-name> <sig>)…)`, where each op's `<sig>` is the type-descriptor
/// AST [`build_type`] emits into the SAME arena (one structural-encode path, no parallel encoder). This is
/// the effect-identity foundation (operator seq367/368, seq374 top feature): an effect is identified by its
/// SCHEMA — the shapes of its operations — not by a closed enum discriminant or an arbitrary family string.
/// The family/display name becomes a human ALIAS; THIS hash is the authoritative identity that authz gates
/// on (collision-proof) and the wire carries (resolved hash→name via the host's [`crate::schema_resolver`]).
/// The schema is the effect's DATA SHAPE only — there is NO authz node in it (operator directive: grants are
/// dynamic and live OUTSIDE the schema; the schema-hash is the identity a grant keys on, the grant is external).
///
/// ORDER-INDEPENDENT: `ops` are sorted by op-name before the tree is built, so the same operation set hashes
/// IDENTICALLY regardless of declaration order — a schema is its SET of named operations, not a sequence. A
/// shape change to ANY op's signature (or adding/removing an op) flips the hash. Pure
/// over the encoded tree ([`Hash::of`] = blake3, the one content-address algorithm), so the same effect
/// schema yields the same id everywhere (kernel, host, on the wire). An op whose signature has no
/// value-crossing descriptor propagates [`build_type`]'s [`MarshalError`].
pub fn effect_schema_hash(
    name: &str,
    ops: &[(&str, &Type)],
) -> Result<crate::hash::Hash, MarshalError> {
    let mut b = Builder::new();
    // Build each op's signature descriptor node into the shared arena (the SINGLE structural encoder).
    let op_nodes: Vec<(&str, StructId)> = ops
        .iter()
        .map(|(op_name, sig)| Ok((*op_name, build_type(&mut b, sig)?)))
        .collect::<Result<_, MarshalError>>()?;
    Ok(effect_schema_hash_from_nodes(b, name, &op_nodes))
}

/// The `&Type`-free core of [`effect_schema_hash`]: given op-signature descriptor nodes ALREADY built into
/// `b` (as `StructId`s in `b`'s arena), assemble the schema tree + content-hash it. The op-order-independence
/// (sort by op-name) and the encode→[`Hash::of`] happen HERE, so this is the single hashing path both callers
/// share. Use this when the op signatures come from a source OTHER than wasmtime component reflection — a
/// wasmtime [`Type`] is NOT directly constructible (it only comes from reflecting a real component), so a
/// KERNEL-authored effect schema (e.g. the built-in effects, which have no reflected component) must build its
/// op-sig descriptor nodes directly with the [`Builder`] (`build_type`'s primitive/compound forms — `bytes`,
/// `unit`, records, etc.) and pass them here. [`effect_schema_hash`] is the thin wrapper that reflects a
/// `&Type` per op into `b` then delegates. The schema carries the effect's DATA SHAPE only — no authz node
/// (operator directive: grants live OUTSIDE the schema, dynamic; the schema-hash is the identity a grant keys on).
pub fn effect_schema_hash_from_nodes(
    mut b: Builder,
    name: &str,
    ops: &[(&str, StructId)],
) -> crate::hash::Hash {
    // Sort by op-name so the identity is the SET of ops, order-independent.
    let mut op_nodes: Vec<(&str, StructId)> = ops.to_vec();
    op_nodes.sort_by(|a, c| a.0.cmp(c.0));
    let tree = b.effect_schema_tree(name, &op_nodes);
    crate::hash::Hash::of(&codec::encode(&b.finish(tree)))
}

/// Build the type-descriptor for `ty` INTO an existing arena `b`, returning the root node id — the
/// node-emitting core [`type_to_ast`] wraps. Exposed `pub` so a caller assembling a LARGER AST can emit
/// type nodes DIRECTLY into its own [`Builder`] (one shared arena, encoded ONCE) rather than nesting
/// self-encoded per-type byte blobs: e.g. v-agent-harness's component-signature descriptor is ONE
/// uniform AST `(component-signature (export (name)(params <type-node>…)(results <type-node>…))…)` where
/// each param/result type is a node built by THIS fn into the descriptor's arena (operator directive:
/// "why isn't the entire thing just an AST?"). Keeping this the SINGLE node-emitter means signature-query,
/// value/type marshalling, and v-ah-host's P2 all speak ONE AST surface — no parallel encoding. Use
/// [`type_to_ast`] instead when you want a lone type encoded to standalone `Vec<u8>` bytes.
///
/// Recursive: a compound type's sub-types (via wasmtime's `Type` reflection accessors — `lt.ty()`,
/// `rt.fields()`, `tt.types()`, `ot.ty()`, `rt.ok()`/`err()`, `vt.cases()`, `et.names()`, `ft.names()`)
/// are built first, then wrapped. Mirrors [`build_from_ast`]'s `Type` traversal exactly (same variants,
/// same accessors), but EMITS a descriptor node instead of READING a value — so the two can never
/// disagree on which types exist. The descriptor form vocabulary is documented on [`type_to_ast`].
pub fn build_type(b: &mut Builder, ty: &Type) -> Result<StructId, MarshalError> {
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
/// rules mirror [`build_val`] (name-head record/tuple/list/flags AND option/result/variant/enum ctors,
/// `list<u8>` from a `Leaf::Bytes`, primitives from their leaf), but here the TYPE drives which reader to
/// apply. Reads are HEAD-tolerant (`form` accepts both name- and str-head spellings) and a record field
/// may be the canonical `(= name value)` 3-list or the legacy `(name value)` 2-list.
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
        // list<u8> ← a single Leaf::Bytes; list<T≠u8> ← a name-head (list elem…) form, each elem
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
        // record ← (record (= name value)…): match each declared field by NAME (order-independent). The
        // canonical field is a 3-list (= name value); a legacy 2-list (name value) is ALSO accepted (the
        // same migration tolerance value-decode keeps at cdz-runtime lib.rs 3322-3324).
        Type::Record(rt) => {
            let field_nodes = form(a, id, "record")?;
            // Collect the AST fields as (name → value-node), rejecting a malformed field entry (neither a
            // (= name value) 3-list nor a (name value) 2-list) AND a DUPLICATE name up front. ast_to_val
            // decodes UNTRUSTED arg bytes, so the record must EXACTLY match the WIT shape (github-liaison
            // #2078): silently accepting extra or duplicate fields hides malformed input + yields a
            // surprising Val (same untrusted-input hardening as the #2050 {val:?} DoS; mirrors the tuple
            // arm's strict arity below).
            let mut ast_fields: std::collections::BTreeMap<&str, StructId> = Default::default();
            for &fnode in field_nodes {
                let (name, val_node) = match a.get(fnode) {
                    // canonical (= name value) 3-list — value-encode emits this form.
                    Struct::List(kids) if kids.len() == 3 && a.as_name(kids[0]) == Some("=") => {
                        match a.as_name(kids[1]) {
                            Some(n) => (n, kids[2]),
                            None => {
                                return Err(type_mismatch("record", "field name is not a name"))
                            }
                        }
                    }
                    // legacy (name value) 2-list — decode tolerance.
                    Struct::List(kids) if kids.len() == 2 => match a.as_name(kids[0]) {
                        Some(n) => (n, kids[1]),
                        None => return Err(type_mismatch("record", "field name is not a name")),
                    },
                    _ => {
                        return Err(type_mismatch(
                            "record",
                            "field is not a (= name value) or (name value) form",
                        ))
                    }
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
        // enum ← name-head (Case unit): a nullary variant's payload is the unit atom (a bare (Case) with
        // no payload is tolerated too). The case must be a declared name; a NON-unit payload is a mismatch.
        Type::Enum(et) => {
            let (case, payload) = ctor(a, id)?;
            if let Some(node) = payload {
                if a.as_name(node) != Some("unit") {
                    return Err(type_mismatch(
                        "enum",
                        format!("case {} carries a non-unit payload", bounded_name(case)),
                    ));
                }
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
        Some(Leaf::Bytes(b)) => Ok(b.to_vec()),
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
        // A nullary case is the two-element (Name unit): the unit-atom payload against a unit arm means
        // "no payload" (a bare (Name) with no payload node also reaches (None, None) above — both forms
        // accepted). A NON-unit payload against a unit arm is a genuine shape mismatch.
        (Some(node), None) if a.as_name(node) == Some("unit") => Ok(None),
        (Some(_), None) => Err(type_mismatch("ctor", "non-unit payload but arm is unit")),
        (None, Some(_)) => Err(type_mismatch(
            "ctor",
            "arm expects a payload but none present",
        )),
    }
}

fn type_mismatch(expected: &str, found: impl Into<std::sync::Arc<str>>) -> MarshalError {
    MarshalError::TypeMismatch {
        expected: expected.into(),
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
        val_kind: kind.into(),
    }
}

// --- binary-AST fold boundary (DESIGN-binary-ast-abi B1) -----------------------------------------------
//
// The fold boundary becomes `apply(list<u8>) -> list<u8>` where both sides are cadenza-ast value-form
// documents (B2 flips reducer.wit + deletes HeapHandle; B1 is the kernel-side builder/parser here). The
// kernel folds its three `apply` inputs (content-type, optional payload, optional resume token) into ONE
// event document, and parses the returned effect-list document back into `Vec<Effect>` — the marshalling
// that used to live in `HeapHandle`, now pure `cadenza-ast` byte work (no wasmtime `Func` binding, no heap).
//
// Schemas (value-form s-exprs, the small agreed shape the guest + kernel share):
//   event         = (event (content-type (family <str>) (version <int>)) (payload <opt>) (resumes <opt>))
//   effect-list   = (effects <effect-request>…)
//   effect-request= (effect-request (kind <name>) (target <bytes>) (payload <opt>) (correlation <opt>))
//   <opt>         = (some <bytes>) | (none)      — absent vs empty distinguished (matches Option<Payload>)
// `kind` is a NAME-head leaf of the effect family string (shell/http/model/now/timer/emit or an
// extension family); `target` is opaque bytes (Target=Bytes ruling). This mirrors reducer.wit's
// content-type/effect-kind/effect-request records as value-form AST rather than WIT records.

use crate::effect::{EffectRequest, Payload, Timeliness};
use crate::reducer::Effect;

/// A borrowed content-type view for [`build_event_document`] — `(family, version)` — so the caller passes
/// the event's content-type without this module depending on the `event::ContentType` layout beyond it.
pub struct ContentTypeRef<'a> {
    pub family: &'a str,
    pub version: u32,
}

/// Encode an optional byte payload as the `<opt>` shape: `(some <bytes>)` for present (incl. empty), or
/// `(none)` for absent — so a zero-length payload is distinct from no payload (matches `Option<Payload>`).
fn opt_bytes_form(b: &mut Builder, v: Option<&[u8]>) -> StructId {
    match v {
        Some(bytes) => {
            let head = b.name("some");
            let val = b.atom_leaf(Leaf::Bytes(bytes.to_vec().into()));
            b.list(vec![head, val])
        }
        None => {
            let head = b.name("none");
            b.list(vec![head])
        }
    }
}

/// Read an `<opt>` node — `(some <bytes>)` → `Some(bytes)`, `(none)` → `None`; anything else is malformed.
fn read_opt_bytes(a: &Arenas, id: StructId) -> Result<Option<Vec<u8>>, MarshalError> {
    match head_name(a, id) {
        Some("some") => {
            let kids = form(a, id, "some")?;
            match kids {
                [v] => Ok(Some(read_bytes(a, *v)?)),
                _ => Err(type_mismatch("some", "expected (some <bytes>)")),
            }
        }
        Some("none") => Ok(None),
        _ => Err(type_mismatch("opt", "expected (some <bytes>) or (none)")),
    }
}

/// The head NAME of a name-headed form, or `None` if `id` isn't a headed list. A thin helper over the
/// arena so the boundary readers can dispatch on `(head …)`.
fn head_name(a: &Arenas, id: StructId) -> Option<&str> {
    a.head_name(id)
}

/// Build the ONE event document the fold boundary passes IN (B1): fold the content-type, the optional
/// payload, and the optional resume token into a single value-form AST, returning its canonical bytes. The
/// guest `value-decode`s these bytes against the event descriptor. Reuses the shared `cadenza-ast` codec.
pub fn build_event_document(
    content_type: ContentTypeRef,
    payload: Option<&[u8]>,
    resumes: Option<&[u8]>,
) -> Vec<u8> {
    let mut b = Builder::new();
    // (content-type (family <str>) (version <int>))
    let ct = {
        let head = b.name("content-type");
        let fam = {
            let h = b.name("family");
            let v = b.atom_leaf(Leaf::Str(content_type.family.to_string().into()));
            b.list(vec![h, v])
        };
        let ver = {
            let h = b.name("version");
            let v = b.atom_leaf(Leaf::Int {
                value: BigInt::from(content_type.version),
                radix: Radix::Dec,
            });
            b.list(vec![h, v])
        };
        b.list(vec![head, fam, ver])
    };
    let payload_node = {
        let h = b.name("payload");
        let v = opt_bytes_form(&mut b, payload);
        b.list(vec![h, v])
    };
    let resumes_node = {
        let h = b.name("resumes");
        let v = opt_bytes_form(&mut b, resumes);
        b.list(vec![h, v])
    };
    let head = b.name("event");
    let root = b.list(vec![head, ct, payload_node, resumes_node]);
    codec::encode(&b.finish(root))
}

/// Produce the REDUCER world artifact — the `KIND_WIT_WORLD` binary-AST bytes (`db.wit_world`) a reducer
/// program targets so `rcdzc`'s full-A emit bytes-wraps its `fold.apply` (DESIGN-compiler-platform-separation
/// §3b). This is the "external artifact" source, built via the SHARED `cadenza-ast` world builders
/// (`world_schema_tree`/`wit_interface`/`wit_func_sig`) so it is byte-identical to v-syntax's inline
/// declaration and to v-cml's emit-side read BY CONSTRUCTION (one canonical world tree, hashed like an
/// effect schema). Each param/result type is a `build_type`-form descriptor.
///
/// SCOPE A (the genesis MVP world, agreed with v-cml): the SMALLEST honest slice of `reducer.wit` that makes
/// the genesis fold emit bytes — the `fold.apply` export (`list<u8> -> list<u8>`) plus the `kv` import members
/// the genesis fold uses, `get(list<u8>) -> option<list<u8>>` and `put(list<u8>, list<u8>)` (unit). The rest
/// of `kv` (`delete`/`prefix-scan`, needing `bool`/`tuple`/`list<tuple>`) is deferred until the emit reader
/// widens past `list<u8>`+`option` — a backward slice, not a fake shape.
pub fn reducer_world_artifact() -> Vec<u8> {
    let mut b = Builder::new();
    // `build_type`-form descriptors (must match `ast_marshal::build_type` / `rcdzc::wit_world::parse_wit_type`
    // EXACTLY): a primitive is a NAME-head 1-marker `(u8)`; `list<T>` is `("list" <T>)` (str head); `option<T>`
    // is `("option" <T>)` (str head); `unit` is `("unit")` (str head, no children). Built FRESH per occurrence
    // (as `build_type` does); the codec canonicalizes occurrence order, so identical worlds encode identically.
    let bytes_desc = |b: &mut Builder| {
        let u8d = b.name("u8");
        let u8_prim = b.list(vec![u8d]);
        let list_head = b.atom_leaf(Leaf::Str("list".into()));
        b.list(vec![list_head, u8_prim])
    };
    let opt_bytes_desc = |b: &mut Builder| {
        let inner = bytes_desc(b);
        let opt_head = b.atom_leaf(Leaf::Str("option".into()));
        b.list(vec![opt_head, inner])
    };
    let unit_desc = |b: &mut Builder| {
        let head = b.atom_leaf(Leaf::Str("unit".into()));
        b.list(vec![head])
    };

    // export `fold`: `apply(event: list<u8>) -> list<u8>`
    let apply_sig = {
        let ev = bytes_desc(&mut b);
        let res = bytes_desc(&mut b);
        b.wit_func_sig(&[("event", ev)], res)
    };
    let fold = b.wit_interface(WitDir::Export, "fold", &[("apply", apply_sig)]);

    // import `kv`: `get(key: list<u8>) -> option<list<u8>>`, `put(key: list<u8>, value: list<u8>)` (unit)
    let get_sig = {
        let key = bytes_desc(&mut b);
        let res = opt_bytes_desc(&mut b);
        b.wit_func_sig(&[("key", key)], res)
    };
    let put_sig = {
        let key = bytes_desc(&mut b);
        let value = bytes_desc(&mut b);
        let res = unit_desc(&mut b);
        b.wit_func_sig(&[("key", key), ("value", value)], res)
    };
    let kv = b.wit_interface(WitDir::Import, "kv", &[("get", get_sig), ("put", put_sig)]);

    let world = b.world_schema_tree("reducer", &[fold, kv]);
    codec::encode(&b.finish(world))
}

/// The PURE-FOLD world artifact — the smallest world for the pure-genesis INTERMEDIATE co-land: a reducer
/// that only EXPORTS `fold.apply(list<u8>) -> list<u8>` and imports NOTHING (no `kv`). Targeting this world,
/// `rcdzc`'s landed world-driven emit (GAP A) bytes-wraps a pure `apply(Event) -> effect-list` guest so it
/// crosses as `list<u8>`, and the kernel host drives it through `build_event_document` / `parse_effect_list`
/// — proving the CADENZA-guest bytes path end-to-end WITHOUT the host-fused `kv` emit (GAP B). Uses the same
/// builders and descriptors as [`reducer_world_artifact`], minus the `kv` import (so it stays within the
/// emit's `list<u8>` vocab and needs no host-import fusion).
pub fn pure_fold_world_artifact() -> Vec<u8> {
    let mut b = Builder::new();
    let bytes_desc = |b: &mut Builder| {
        let u8d = b.name("u8");
        let u8_prim = b.list(vec![u8d]);
        let list_head = b.atom_leaf(Leaf::Str("list".into()));
        b.list(vec![list_head, u8_prim])
    };
    // export `fold`: `apply(event: list<u8>) -> list<u8>` — the only member; no `kv` import.
    let apply_sig = {
        let ev = bytes_desc(&mut b);
        let res = bytes_desc(&mut b);
        b.wit_func_sig(&[("event", ev)], res)
    };
    let fold = b.wit_interface(WitDir::Export, "fold", &[("apply", apply_sig)]);
    let world = b.world_schema_tree("reducer", &[fold]);
    codec::encode(&b.finish(world))
}

/// Parse the effect-list document the fold boundary returns (B1) into `Vec<Effect>` — the dual of
/// [`build_event_document`], replacing the old `HeapHandle` walk of the returned effect-list handle. The
/// guest `value-encode`d its requested effects as `(effects <effect-request>…)`; this decodes them back
/// into the kernel's `Effect` handoff type (an `EffectRequest` + the reducer's optional continuation token).
/// TOTAL over arbitrary bytes: undecodable bytes → `Undecodable`; a well-formed-but-wrong shape →
/// `TypeMismatch` — never a panic (untrusted guest output, same discipline as `ast_to_val`).
pub fn parse_effect_list(bytes: &[u8]) -> Result<Vec<Effect>, MarshalError> {
    let a = codec::decode(bytes).ok_or(MarshalError::Undecodable)?;
    let reqs = form(&a, a.root, "effects")?;
    let mut out = Vec::with_capacity(reqs.len());
    for &r in reqs {
        out.push(parse_effect_request(&a, r)?);
    }
    Ok(out)
}

/// Parse one `(effect-request (kind <name>) (target <bytes>) (payload <opt>) (correlation <opt>))` into an
/// [`Effect`]. `kind` is the effect FAMILY name (seq-39 identity): a well-known family maps to its
/// `EffectKind`, an extension family takes the `Emit` placeholder (kernel dispatch/idempotency key on the
/// family, so the placeholder is inert) — via [`EffectRequest::new_with_family`], the same
/// register-by-string constructor the rest of the kernel uses. Timeliness defaults to `Interactive` (the
/// value-form event schema doesn't carry it yet — a follow-up envelope field if a batchable guest effect
/// needs it).
fn parse_effect_request(a: &Arenas, id: StructId) -> Result<Effect, MarshalError> {
    let fields = form(a, id, "effect-request")?;
    let mut kind_family: Option<String> = None;
    let mut target: Option<Vec<u8>> = None;
    let mut payload: Option<Option<Vec<u8>>> = None;
    let mut correlation: Option<Option<Vec<u8>>> = None;
    for &f in fields {
        match head_name(a, f) {
            Some("kind") => {
                let kids = form(a, f, "kind")?;
                let [n] = kids else {
                    return Err(type_mismatch("kind", "expected (kind <name>)"));
                };
                let name = a
                    .as_name(*n)
                    .ok_or_else(|| type_mismatch("kind", "kind is not a name"))?;
                kind_family = Some(name.to_string());
            }
            Some("target") => {
                let kids = form(a, f, "target")?;
                let [v] = kids else {
                    return Err(type_mismatch("target", "expected (target <bytes>)"));
                };
                target = Some(read_bytes(a, *v)?);
            }
            Some("payload") => {
                let kids = form(a, f, "payload")?;
                let [v] = kids else {
                    return Err(type_mismatch("payload", "expected (payload <opt>)"));
                };
                payload = Some(read_opt_bytes(a, *v)?);
            }
            Some("correlation") => {
                let kids = form(a, f, "correlation")?;
                let [v] = kids else {
                    return Err(type_mismatch("correlation", "expected (correlation <opt>)"));
                };
                correlation = Some(read_opt_bytes(a, *v)?);
            }
            _ => return Err(type_mismatch("effect-request", "unknown field")),
        }
    }
    let family = kind_family.ok_or_else(|| type_mismatch("effect-request", "missing kind"))?;
    let target = target.ok_or_else(|| type_mismatch("effect-request", "missing target"))?;
    // payload/correlation default to absent if the field is omitted (a fire-and-forget, payload-free effect).
    let payload = payload.flatten().map(|b| Payload::Inline(b.into()));
    let token = correlation.flatten();
    let request = EffectRequest::new_with_family(family, target, payload, Timeliness::Interactive);
    Ok(Effect { request, token })
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

    // Pin the CANONICAL value-form `val_to_ast` PRODUCES for a record: a NAME-head `record`, fields SORTED
    // by name, each a 3-list `(= name value)`. This IS the deterministic-value-form a guest's value-decode
    // (op 90) consumes and value-encode produces — a Str head decodes to NULL, an unsorted/2-list form
    // drifts from value-encode. Built with fields in NON-sorted order (`size` before `kind`) to prove the
    // sort. This is the invariant the bytes fold boundary (A1) routes the Event doc through.
    #[test]
    fn val_to_ast_emits_canonical_name_head_sorted_equals_record() {
        let v = Val::Record(vec![
            ("size".into(), Val::U32(7)),
            ("kind".into(), Val::String("wasm".into())),
        ]);
        let bytes = val_to_ast(&v).expect("val_to_ast");
        let a = codec::decode(&bytes).expect("decode");
        let Struct::List(kids) = a.get(a.root) else {
            panic!("record root is a list");
        };
        // head is a NAME leaf `record` (a Str head is rejected by value-decode as a NULL).
        assert_eq!(
            a.as_name(kids[0]),
            Some("record"),
            "head is the name `record`"
        );
        assert!(
            matches!(leaf_at(&a, kids[0]), Leaf::Name(_)),
            "head must be a Name leaf, not a Str leaf"
        );
        assert_eq!(kids.len(), 3, "head + 2 fields");
        // each field is a `(= name value)` 3-list; fields are SORTED by name (`kind` before `size`).
        let field_name = |i: usize| -> &str {
            let Struct::List(f) = a.get(kids[i]) else {
                panic!("field is a list");
            };
            assert_eq!(f.len(), 3, "field is a (= name value) 3-list");
            assert_eq!(a.as_name(f[0]), Some("="), "field head is the `=` name");
            a.as_name(f[1]).expect("field name is a name")
        };
        assert_eq!(field_name(1), "kind", "fields sorted: kind first");
        assert_eq!(field_name(2), "size", "fields sorted: size second");
    }

    // ast_to_val decodes UNTRUSTED arg bytes, so a record must EXACTLY match the WIT shape (github-liaison
    // #2078): an EXTRA field beyond the declared set, or a DUPLICATE field name, is rejected — not silently
    // accepted. Build the malformed record AST by hand (the LEGACY str-head ("record" (name val)…) 2-list
    // form, still accepted by the reader's migration tolerance) and decode against a 1-field record type.
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
        // NULLARY variant case (stay, no payload) → (stay unit) two-element; the reader accepts the unit
        // atom against a unit arm via opt_payload. Pins the nullary two-element read path.
        assert_eq!(
            round_trip(
                Val::Variant("stay".into(), None),
                r#"(variant (case "move-to" u32) (case "stay"))"#
            ),
            Val::Variant("stay".into(), None)
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
        assert_eq!(
            root_leaf(&a),
            &Leaf::Bytes(vec![0xDE, 0xAD, 0x00, 0xFF].into())
        );
        // empty list<u8> → empty Bytes
        let empty = decode(&val_to_ast(&Val::List(vec![])).unwrap());
        assert_eq!(root_leaf(&empty), &Leaf::Bytes(vec![].into()));
    }

    #[test]
    fn a_non_u8_list_marshals_to_a_name_head_list_form() {
        let v = Val::List(vec![Val::U32(1), Val::U32(2)]);
        let a = decode(&val_to_ast(&v).unwrap());
        // (list 1 2): NAME head, read via as_form (the canonical value-form head)
        let elems = a.as_form(a.root, "list").expect("name-head list form");
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
    fn a_record_marshals_to_a_name_head_record_of_equals_name_value_fields() {
        let v = Val::Record(vec![
            ("kind".into(), Val::String("wasm".into())),
            ("size".into(), Val::U32(7)),
        ]);
        let a = decode(&val_to_ast(&v).unwrap());
        // (record (= kind "wasm") (= size 7)): NAME head, read via as_form
        let fields = a.as_form(a.root, "record").expect("name-head record form");
        assert_eq!(fields.len(), 2);
        // each field is a (= name value) 3-list (record-type Phase B canonical value-form)
        let f0 = match a.get(fields[0]) {
            Struct::List(kids) => kids.clone(),
            _ => panic!("field is a list"),
        };
        assert_eq!(f0.len(), 3);
        assert_eq!(a.as_name(f0[0]), Some("="));
        assert_eq!(a.as_name(f0[1]), Some("kind"));
        assert_eq!(leaf_at(&a, f0[2]), &Leaf::Str("wasm".into()));
    }

    #[test]
    fn a_tuple_marshals_to_a_name_head_tuple_form() {
        let v = Val::Tuple(vec![Val::Bool(true), Val::U8(9)]);
        let a = decode(&val_to_ast(&v).unwrap());
        let elems = a.as_form(a.root, "tuple").expect("name-head tuple form");
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
        // None — nullary two-element (None unit): the payload is the lowercase `unit` name atom
        // (value-decode's Sum arm requires exactly two children; a bare (None) decodes to NULL).
        let none = decode(&val_to_ast(&Val::Option(None)).unwrap());
        let none_kids = none
            .as_form(none.root, "None")
            .expect("name-head (None unit)");
        assert_eq!(none_kids.len(), 1);
        assert_eq!(none.as_name(none_kids[0]), Some("unit"));
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
        // enum Case — nullary two-element (Case unit)
        let en = decode(&val_to_ast(&Val::Enum("Red".into())).unwrap());
        let en_kids = en.as_form(en.root, "Red").expect("name-head (Red unit)");
        assert_eq!(en_kids.len(), 1);
        assert_eq!(en.as_name(en_kids[0]), Some("unit"));
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

    // The reducer world artifact (KIND_WIT_WORLD bytes) is the SCOPE-A world: export fold.apply
    // (list<u8> -> list<u8>) + import kv get/put, built via the shared cadenza-ast world builders so it is
    // byte-identical to v-syntax's inline decl + v-cml's emit read. Pin the vocabulary (world/interface/
    // member/param NAME atoms + the build_type-form descriptor STR heads list/option/unit) + determinism,
    // without coupling to v-syntax's exact node layout (their S1 tests pin that + its byte-stable identity).
    #[test]
    fn reducer_world_artifact_is_the_scope_a_world() {
        let bytes = reducer_world_artifact();
        let a = codec::decode(&bytes).expect("world artifact decodes");
        assert_eq!(
            a.head_name(a.root),
            Some("world"),
            "root head is the `world` name"
        );
        fn collect(a: &Arenas, id: StructId, names: &mut Vec<String>, strs: &mut Vec<String>) {
            if let Some(n) = a.as_name(id) {
                names.push(n.to_string());
                return;
            }
            match a.get(id) {
                Struct::Atom(lid) => {
                    if let Leaf::Str(s) = a.leaf(*lid) {
                        strs.push(s.to_string());
                    }
                }
                Struct::List(kids) => {
                    for &k in kids.iter() {
                        collect(a, k, names, strs);
                    }
                }
            }
        }
        let (mut names, mut strs) = (Vec::new(), Vec::new());
        collect(&a, a.root, &mut names, &mut strs);
        for expect in [
            "world", "reducer", "fold", "apply", "kv", "get", "put", "event", "key", "value", "u8",
        ] {
            assert!(
                names.iter().any(|n| n == expect),
                "missing NAME atom {expect}"
            );
        }
        // build_type-form descriptor heads are STR atoms: list<u8> -> ("list" (u8)); option -> ("option" ..);
        // unit -> ("unit"). Their presence proves the param/result descriptors are the shared build_type form.
        for expect in ["list", "option", "unit"] {
            assert!(
                strs.iter().any(|s| s == expect),
                "missing STR head {expect}"
            );
        }
        assert_eq!(reducer_world_artifact(), bytes, "artifact is deterministic");
    }

    // The PURE-FOLD world (pure-genesis intermediate): export fold.apply only, NO kv import — the smallest
    // world for proving the Cadenza-guest bytes path through the host without the GAP-B kv emit.
    #[test]
    fn pure_fold_world_artifact_has_fold_apply_and_no_kv() {
        let bytes = pure_fold_world_artifact();
        let a = codec::decode(&bytes).expect("pure world decodes");
        assert_eq!(
            a.head_name(a.root),
            Some("world"),
            "root head is the `world` name"
        );
        fn collect_names(a: &Arenas, id: StructId, out: &mut Vec<String>) {
            if let Some(n) = a.as_name(id) {
                out.push(n.to_string());
                return;
            }
            if let Struct::List(kids) = a.get(id) {
                for &k in kids.iter() {
                    collect_names(a, k, out);
                }
            }
        }
        let mut names = Vec::new();
        collect_names(&a, a.root, &mut names);
        for expect in ["world", "reducer", "fold", "apply", "event"] {
            assert!(
                names.iter().any(|n| n == expect),
                "missing NAME atom {expect}"
            );
        }
        // pure = NO kv import (that is the whole point — no host-fused import for the intermediate).
        assert!(
            !names.iter().any(|n| n == "kv"),
            "pure-fold world must NOT declare a kv import"
        );
        assert_eq!(
            pure_fold_world_artifact(),
            bytes,
            "artifact is deterministic"
        );
    }

    // Val::Flags round-trip (v-syntax review F2, LOW): built (name-head `(flags a c …)`) + read
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
                Leaf::Str(s) => s.to_string(),
                Leaf::Name(n) => n.to_string(),
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
    fn schema_hash_is_structural_same_shape_same_hash_different_shape_different() {
        // seq367 schema-identity foundation: schema_hash is the content-hash of the type-descriptor AST,
        // so it's a STABLE STRUCTURAL id — same shape → same hash (deterministic, name-independent),
        // different shape → different hash (distinguishes effects by structure, not a family string).
        let u32_a = schema_hash(&param_type(&probe_component("u32"))).expect("u32 schema");
        let u32_b = schema_hash(&param_type(&probe_component("u32"))).expect("u32 schema again");
        assert_eq!(
            u32_a, u32_b,
            "same shape (u32) → same schema-hash (deterministic)"
        );

        let string_h = schema_hash(&param_type(&probe_component("string"))).expect("string schema");
        assert_ne!(
            u32_a, string_h,
            "different shapes (u32 vs string) → different schema-hash"
        );

        // A compound shape hashes distinctly from its element shape (record != u32 != list<u32>).
        let rec_h = schema_hash(&param_type(&probe_component(r#"(record (field "a" u32))"#)))
            .expect("record schema");
        let list_h = schema_hash(&param_type(&probe_component("(list u32)"))).expect("list schema");
        assert_ne!(rec_h, u32_a, "record (field a u32) is not u32");
        assert_ne!(rec_h, list_h, "record (field a u32) is not (list u32)");
        // And it's exactly Hash::of(type_to_ast(ty)) — the schema-hash IS the descriptor's content address.
        let u32_ty = param_type(&probe_component("u32"));
        assert_eq!(
            u32_a,
            crate::hash::Hash::of(&type_to_ast(&u32_ty).unwrap()),
            "schema_hash == content-hash of the type-descriptor AST"
        );
    }

    #[test]
    fn effect_schema_hash_is_op_order_independent_and_shape_sensitive() {
        // seq367/374 effect-identity: an effect's identity is the hash of its schema tree
        // `(effect <name> (op <op-name> <sig>)…)`, and a schema is the SET of its named ops — so the
        // SAME ops in a DIFFERENT source order must hash IDENTICALLY (order-independence), while any
        // shape change (a different op signature, or a renamed op) must flip the hash.
        let u32_ty = param_type(&probe_component("u32"));
        let str_ty = param_type(&probe_component("string"));

        // Same two ops, opposite declaration order → identical schema-hash.
        let ab =
            effect_schema_hash("kv", &[("get", &str_ty), ("put", &u32_ty)]).expect("kv schema ab");
        let ba =
            effect_schema_hash("kv", &[("put", &u32_ty), ("get", &str_ty)]).expect("kv schema ba");
        assert_eq!(
            ab, ba,
            "op order must not affect the effect-schema identity"
        );

        // A different op SIGNATURE (get: u32 not string) flips the hash.
        let sig_changed = effect_schema_hash("kv", &[("get", &u32_ty), ("put", &u32_ty)])
            .expect("kv schema sig-changed");
        assert_ne!(ab, sig_changed, "an op signature change must flip the hash");

        // A different op NAME (getx not get) flips the hash.
        let name_changed = effect_schema_hash("kv", &[("getx", &str_ty), ("put", &u32_ty)])
            .expect("kv schema name-changed");
        assert_ne!(ab, name_changed, "an op rename must flip the hash");

        // A different effect NAME flips the hash (the name is part of the tree head).
        let effect_renamed = effect_schema_hash("store", &[("get", &str_ty), ("put", &u32_ty)])
            .expect("store schema");
        assert_ne!(
            ab, effect_renamed,
            "the effect name is part of its identity"
        );

        // Deterministic: recomputing the same schema yields the same hash.
        let ab_again = effect_schema_hash("kv", &[("get", &str_ty), ("put", &u32_ty)])
            .expect("kv schema again");
        assert_eq!(ab, ab_again, "effect-schema hashing is deterministic");
    }

    #[test]
    fn effect_schema_hash_from_nodes_equals_the_type_path_and_hashes_directly_built_ops() {
        // effect_schema_hash_from_nodes is the &Type-free core: given op-sig descriptor nodes already built
        // into a Builder, it assembles the schema tree + hashes it — the SAME hashing path effect_schema_hash
        // uses after it reflects each &Type into a node. Two properties:
        //
        // (1) EQUIVALENCE — for the same schema, the from_nodes path yields the IDENTICAL hash to the &Type
        //     path. Build the op-sig nodes with build_type (exactly what effect_schema_hash does internally),
        //     pass them to from_nodes, and confirm it matches.
        let u32_ty = param_type(&probe_component("u32"));
        let str_ty = param_type(&probe_component("string"));
        let via_type = effect_schema_hash("kv", &[("get", &str_ty), ("put", &u32_ty)])
            .expect("kv schema via &Type");
        let via_nodes = {
            let mut b = Builder::new();
            let get_sig = build_type(&mut b, &str_ty).unwrap();
            let put_sig = build_type(&mut b, &u32_ty).unwrap();
            effect_schema_hash_from_nodes(b, "kv", &[("get", get_sig), ("put", put_sig)])
        };
        assert_eq!(
            via_type, via_nodes,
            "from_nodes must match the &Type path for the same schema (shared hashing core)"
        );

        // (2) DIRECTLY-BUILT ops — the KERNEL built-in path: a schema whose op sigs are built WITHOUT a
        //     wasmtime Type (which is not constructible). Build a `(bytes)` prim node directly and hash a
        //     single-perform-op schema `(effect http (op perform (-> bytes bytes)))` shape. This is how the
        //     built-in effects author their schemas (they have no reflected component). Order-independence +
        //     name-sensitivity still hold on this path.
        let built_in = |name: &str| {
            let mut b = Builder::new();
            // A `(-> bytes bytes)` op signature built directly: result form is ("result" bytes bytes)-ish
            // isn't needed here — the op sig is any descriptor node; use a lone `(bytes)` marker as the
            // stand-in data-shape (the real built-in derivation composes -> in out, but a prim node exercises
            // the from_nodes path identically).
            let bytes_head = b.name("bytes");
            let bytes_sig = b.list(vec![bytes_head]);
            effect_schema_hash_from_nodes(b, name, &[("perform", bytes_sig)])
        };
        let http = built_in("http");
        let http_again = built_in("http");
        let shell = built_in("shell");
        assert_eq!(
            http, http_again,
            "directly-built built-in schema is deterministic"
        );
        assert_ne!(
            http, shell,
            "built-ins with the same op-shape but different NAME hash differently (name in the (effect <name>) head)"
        );
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
    fn build_type_composes_multiple_types_into_one_caller_arena() {
        // The pub `build_type` use case (v-agent-harness's one-uniform-AST descriptor, operator directive):
        // emit SEVERAL type nodes DIRECTLY into a caller's own Builder, wrap them in a larger form, encode
        // ONCE — no per-type nested-encoded byte blobs. Here: a mock ("params" <type>…) node carrying two
        // param types (u32, (list string)) built inline into the same arena.
        let u32_ty = param_type(&probe_component("u32"));
        let list_ty = param_type(&probe_component("(list string)"));
        let mut b = Builder::new();
        let head = b.atom_leaf(Leaf::Str("params".into()));
        let p0 = build_type(&mut b, &u32_ty).expect("build u32 into arena");
        let p1 = build_type(&mut b, &list_ty).expect("build list into arena");
        let root = b.list(vec![head, p0, p1]);
        let arenas = b.finish(root);
        // ONE arena, ONE encode — and it's ordinary cdzast (round-trips byte-identical).
        let bytes = codec::encode(&arenas);
        let decoded = decode(&bytes);
        assert_eq!(head_str(&decoded, decoded.root), "params");
        let ks = kids(&decoded, decoded.root);
        assert_eq!(ks.len(), 3, "head + 2 param types in ONE arena");
        assert_eq!(head_str(&decoded, ks[1]), "u32");
        assert_eq!(head_str(&decoded, ks[2]), "list");
        assert_eq!(
            codec::encode(&decoded),
            bytes,
            "the composed descriptor re-encodes byte-identical"
        );
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

    // --- binary-AST fold boundary (B1) tests ---

    #[test]
    fn event_document_builds_the_content_type_payload_resumes_shape() {
        // B1: build_event_document folds (content-type, optional payload, optional resumes) into ONE
        // value-form doc. Decode it back + assert the (event (content-type (family)(version)) (payload)
        // (resumes)) shape, with (some <bytes>) vs (none) distinguishing an empty payload from absent.
        let bytes = build_event_document(
            ContentTypeRef {
                family: "message",
                version: 1,
            },
            Some(b"hi"),
            None,
        );
        let a = codec::decode(&bytes).expect("event doc decodes");
        // Head is `event` with 3 children.
        let ev = form(&a, a.root, "event").expect("event form");
        assert_eq!(ev.len(), 3);
        // content-type carries family + version.
        let ct = form(&a, ev[0], "content-type").expect("content-type");
        assert_eq!(
            read_str(&a, form(&a, ct[0], "family").unwrap()[0]).unwrap(),
            "message"
        );
        // payload = (some b"hi"); resumes = (none).
        assert_eq!(
            read_opt_bytes(&a, form(&a, ev[1], "payload").unwrap()[0]).unwrap(),
            Some(b"hi".to_vec())
        );
        assert_eq!(
            read_opt_bytes(&a, form(&a, ev[2], "resumes").unwrap()[0]).unwrap(),
            None
        );
        // An EMPTY payload is (some []), distinct from absent (none).
        let empty = build_event_document(
            ContentTypeRef {
                family: "m",
                version: 1,
            },
            Some(b""),
            None,
        );
        let ae = codec::decode(&empty).unwrap();
        let eve = form(&ae, ae.root, "event").unwrap();
        assert_eq!(
            read_opt_bytes(&ae, form(&ae, eve[1], "payload").unwrap()[0]).unwrap(),
            Some(vec![]),
            "an intentionally-empty payload is (some []), not (none)"
        );
    }

    #[test]
    fn effect_list_parses_back_into_effects_with_kind_target_payload_token() {
        // B1: parse_effect_list is the dual — a value-form (effects <effect-request>…) doc → Vec<Effect>.
        // Build a two-effect list by hand (as a guest's value-encode would emit it) + parse it back,
        // asserting kind/target/payload/token per effect (a payload-and-token http effect + a bare now).
        let mut b = Builder::new();
        let mk_opt = |b: &mut Builder, v: Option<&[u8]>| match v {
            Some(x) => {
                let h = b.name("some");
                let val = b.atom_leaf(Leaf::Bytes(x.to_vec().into()));
                b.list(vec![h, val])
            }
            None => {
                let h = b.name("none");
                b.list(vec![h])
            }
        };
        let mk_req =
            |b: &mut Builder, kind: &str, target: &[u8], pl: Option<&[u8]>, corr: Option<&[u8]>| {
                let head = b.name("effect-request");
                let kind_n = {
                    let h = b.name("kind");
                    let n = b.name(kind);
                    b.list(vec![h, n])
                };
                let tgt = {
                    let h = b.name("target");
                    let v = b.atom_leaf(Leaf::Bytes(target.to_vec().into()));
                    b.list(vec![h, v])
                };
                let payload = {
                    let h = b.name("payload");
                    let v = mk_opt(b, pl);
                    b.list(vec![h, v])
                };
                let correlation = {
                    let h = b.name("correlation");
                    let v = mk_opt(b, corr);
                    b.list(vec![h, v])
                };
                b.list(vec![head, kind_n, tgt, payload, correlation])
            };
        let r1 = mk_req(
            &mut b,
            "http",
            b"https://ok/x",
            Some(b"body"),
            Some(b"cont-1"),
        );
        let r2 = mk_req(&mut b, "now", b"", None, None);
        let head = b.name("effects");
        let root = b.list(vec![head, r1, r2]);
        let bytes = codec::encode(&b.finish(root));

        let effects = parse_effect_list(&bytes).expect("parses");
        assert_eq!(effects.len(), 2);
        // Effect 0: http with a body payload + a continuation token.
        assert_eq!(effects[0].request.content_type.family, "http");
        assert_eq!(effects[0].request.target_str().unwrap(), "https://ok/x");
        assert!(
            matches!(&effects[0].request.payload, Some(Payload::Inline(p)) if p.as_ref() == b"body")
        );
        assert_eq!(effects[0].token.as_deref(), Some(&b"cont-1"[..]));
        // Effect 1: bare now — no payload, no token (fire-and-forget).
        assert_eq!(effects[1].request.content_type.family, "now");
        assert!(effects[1].request.payload.is_none());
        assert!(effects[1].token.is_none());

        // TOTAL over garbage: undecodable bytes → Undecodable, not a panic.
        assert_eq!(
            parse_effect_list(b"\xff\xff not ast"),
            Err(MarshalError::Undecodable)
        );
        // Well-formed but wrong head → TypeMismatch.
        let mut bw = Builder::new();
        let wrong = {
            let h = bw.name("nope");
            bw.list(vec![h])
        };
        let wb = codec::encode(&bw.finish(wrong));
        assert!(matches!(
            parse_effect_list(&wb),
            Err(MarshalError::TypeMismatch { .. })
        ));
    }

    #[test]
    fn effect_list_round_trips_a_register_by_string_family_with_no_builtin_kind() {
        // GUARDRAIL (DESIGN-binary-ast-abi §2 / the B2-revert lesson): the effect-request `kind` crosses
        // the bytes boundary as a family STRING, NEVER a closed enum discriminant — so a REGISTER-BY-STRING
        // userspace family with NO matching `EffectKind` variant (seq-39) round-trips intact. This is the
        // exact invariant whose violation (a kind-discriminant ABI) caused the B2+B3 revert; pin it in the
        // gate so a future change to the effect codec can't quietly re-introduce a closed-enum kind.
        let mut b = Builder::new();
        // A novel family that is NOT one of shell/http/model/now/timer/emit — pure register-by-string.
        let novel = "effect/reply";
        let req = {
            let head = b.name("effect-request");
            let kind_n = {
                let h = b.name("kind");
                let n = b.name(novel);
                b.list(vec![h, n])
            };
            let tgt = {
                let h = b.name("target");
                let v = b.atom_leaf(Leaf::Bytes(b"sess-42".to_vec().into()));
                b.list(vec![h, v])
            };
            let payload = {
                let h = b.name("payload");
                let inner = {
                    let sh = b.name("some");
                    let sv = b.atom_leaf(Leaf::Bytes(b"answer".to_vec().into()));
                    b.list(vec![sh, sv])
                };
                b.list(vec![h, inner])
            };
            let correlation = {
                let h = b.name("correlation");
                let inner = {
                    let sh = b.name("some");
                    let sv = b.atom_leaf(Leaf::Bytes(b"tok-9".to_vec().into()));
                    b.list(vec![sh, sv])
                };
                b.list(vec![h, inner])
            };
            b.list(vec![head, kind_n, tgt, payload, correlation])
        };
        let head = b.name("effects");
        let root = b.list(vec![head, req]);
        let bytes = codec::encode(&b.finish(root));

        let effects = parse_effect_list(&bytes).expect("register-by-string family parses");
        assert_eq!(effects.len(), 1);
        // The family is preserved VERBATIM as the string — not coerced to a built-in kind, not dropped.
        assert_eq!(effects[0].request.content_type.family, novel);
        // Sanity: it is genuinely NOT one of the well-known families (this is the register-by-string path).
        assert!(!["shell", "http", "model", "now", "timer", "emit"].contains(&novel));
        assert_eq!(effects[0].request.target_str().unwrap(), "sess-42");
        assert!(
            matches!(&effects[0].request.payload, Some(Payload::Inline(p)) if p.as_ref() == b"answer")
        );
        assert_eq!(effects[0].token.as_deref(), Some(&b"tok-9"[..]));
    }
}
