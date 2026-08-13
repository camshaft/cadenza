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

/// Build a `record{field: ty…}` WIT type descriptor node directly into `b` — the `("record" (fname <ty>)…)`
/// form [`build_type`]'s `Type::Record` arm emits (a `Str("record")` head, then one `(name-node ty-node)`
/// 2-list per field). Used by the built-in effect schema declarations, whose op arg/result types are records
/// (e.g. a model request) that have NO reflected wasmtime [`Type`] to feed [`build_type`] — a KERNEL-authored
/// schema builds its descriptor nodes directly, exactly as [`effect_schema_hash_from_nodes`]'s doc prescribes.
/// The head is a `Str` (matching `build_type`, NOT a `Name`), so a built-in record schema hashes identically
/// to the same record reflected off a component. Fields are emitted NAME-SORTED (see below), so the caller
/// may pass them in any order.
///
/// CANONICAL FIELD ORDER = NAME-SORTED (concierge ruling 2026-08-13, constraint-forced): a record schema's
/// fields are emitted in field-NAME order, NOT the caller's declaration order. This is the ONLY order under
/// which all three schema-descriptor producers agree — the wasmtime-reflected [`build_type`] (which sorts its
/// `rt.fields()` for the same reason), a kernel built-in decl (here), and the rcdzc userspace producer (whose
/// `Ty::Record` is a name-sorted `BTreeMap` and can emit NO other order) — so the same record shape is
/// content-addressed to the SAME schema-hash regardless of who produced it. It also matches the value-form's
/// already-canonical field order (`val_to_ast` sorts record fields by name), removing an intra-kernel skew.
fn wit_type_record(b: &mut Builder, fields: &[(&str, StructId)]) -> StructId {
    let mut sorted: Vec<(&str, StructId)> = fields.to_vec();
    sorted.sort_by(|x, y| x.0.cmp(y.0));
    let mut children = Vec::with_capacity(1 + sorted.len());
    children.push(b.atom_leaf(Leaf::Str("record".into())));
    for (name, ty) in sorted {
        let name_node = b.name(name);
        let entry = b.list(vec![name_node, ty]);
        children.push(entry);
    }
    b.list(children)
}

/// Build a `variant{Case(T)?…}` WIT type descriptor node directly into `b` — the `("variant" (Case <T>?)…)`
/// form [`build_type`]'s `Type::Variant` arm emits (a `Str("variant")` head, then one `(CaseName ty?)` entry
/// per case — a payload-bearing case is a 2-list `(CaseName ty)`, a payload-less case a 1-list `(CaseName)`).
/// Used by a built-in/family effect schema whose op result/arg is a SUM (e.g. control/close's CloseOutcome
/// `Success(payload)|Failure(string)`) that has no reflected wasmtime [`Type`]. The head is a `Str` (matching
/// `build_type`, NOT a `Name`) so a directly-built variant hashes identically to the same variant reflected
/// off a component. Cases are passed in the caller's order (order participates in the identity).
fn wit_type_variant(b: &mut Builder, cases: &[(&str, Option<StructId>)]) -> StructId {
    let mut children = Vec::with_capacity(1 + cases.len());
    children.push(b.atom_leaf(Leaf::Str("variant".into())));
    for &(case, ty) in cases {
        let case_head = b.name(case);
        let entry = match ty {
            Some(t) => b.list(vec![case_head, t]),
            None => b.list(vec![case_head]),
        };
        children.push(entry);
    }
    b.list(children)
}

/// The canonical schema-hash of a BUILT-IN effect ([`crate::effect::EffectKind`]) — its identity under the
/// schema-hash-only effect model (operator directive 2026-08-12: effect identity is the schema-hash, computed
/// over the AST-typed schema, for EVERY effect including built-ins, hashed by the SAME [`effect_schema_hash`]
/// path as a userspace effect — no special-casing). A built-in has no reflected wasmtime component, so its
/// op-signature descriptor nodes are built DIRECTLY with the [`Builder`] (the [`effect_schema_hash_from_nodes`]
/// path), reusing the effect's landed payload/result shapes as the op arg/result types so the schema mirrors
/// the real wire. Each built-in is a single-op effect named for its verb. Pure + deterministic (content-address
/// over the encoded schema tree), so the same built-in yields the same id everywhere (kernel/host/wire) and on
/// replay — the id a router/authz grant keys on. The schema carries the effect's DATA SHAPE only (no authz).
pub fn builtin_effect_schema_hash(kind: &crate::effect::EffectKind) -> crate::hash::Hash {
    use crate::effect::EffectKind;
    let mut b = Builder::new();
    // The op-signature is a `(func (param PName Desc)… (result Desc))` node (WIT func shape), built via the
    // shared `wit_func_sig`; the descriptor nodes reuse the effect's landed payload/result shapes.
    let (name, op_name, sig): (&str, &str, StructId) = match kind {
        // model.invoke(request: ModelRequest) -> ModelResponse. Records reuse the b1 ModelRequest/Response
        // shapes: request = {model: string, messages: list<chat-message>, tools: list<tool-def>,
        // max-tokens: option<u64>}; response = {stop-reason: string, content: list<content-block>}. The
        // nested chat-message/tool-def/content-block element types are opaque `unit`-shaped placeholders here
        // (the op-signature pins the TOP-LEVEL record shape — the identity — not the full nested grammar,
        // which the payload codec owns); a later slice can deepen them additively.
        EffectKind::Model => {
            let model = b.wit_type_prim("string");
            let messages = {
                let elem = b.wit_type_unit();
                b.wit_type_list(elem)
            };
            let tools = {
                let elem = b.wit_type_unit();
                b.wit_type_list(elem)
            };
            let max_tokens = {
                let u64_ty = b.wit_type_prim("u64");
                b.wit_type_option(u64_ty)
            };
            let request = wit_type_record(
                &mut b,
                &[
                    ("model", model),
                    ("messages", messages),
                    ("tools", tools),
                    ("max-tokens", max_tokens),
                ],
            );
            let stop_reason = b.wit_type_prim("string");
            let content = {
                let elem = b.wit_type_unit();
                b.wit_type_list(elem)
            };
            let response = wit_type_record(
                &mut b,
                &[("stop-reason", stop_reason), ("content", content)],
            );
            let sig = b.wit_func_sig(&[("request", request)], response);
            ("model", "invoke", sig)
        }
        // http.request(method: string, headers: list<tuple<string,string>>, body: option<list<u8>>)
        //   -> record{status: u16, headers: list<tuple<string,string>>, body: list<u8>}.
        EffectKind::Http => {
            let method = b.wit_type_prim("string");
            let headers_ty = |b: &mut Builder| {
                let k = b.wit_type_prim("string");
                let v = b.wit_type_prim("string");
                let pair = b.wit_type_tuple(&[k, v]);
                b.wit_type_list(pair)
            };
            let req_headers = headers_ty(&mut b);
            let body = {
                let u8_ty = b.wit_type_prim("u8");
                let bytes = b.wit_type_list(u8_ty);
                b.wit_type_option(bytes)
            };
            let status = b.wit_type_prim("u16");
            let resp_headers = headers_ty(&mut b);
            let resp_body = {
                let u8_ty = b.wit_type_prim("u8");
                b.wit_type_list(u8_ty)
            };
            let response = wit_type_record(
                &mut b,
                &[
                    ("status", status),
                    ("headers", resp_headers),
                    ("body", resp_body),
                ],
            );
            let sig = b.wit_func_sig(
                &[("method", method), ("headers", req_headers), ("body", body)],
                response,
            );
            ("http", "request", sig)
        }
        // shell.run(pipeline: list<record{program: string, args: list<string>}>) -> unit. The outcome folds
        // back as a separate result event; the op result is unit (the dispatched-request shape). Reuses the
        // ShellPipeline/ShellStage shape (a pipeline is a list of stages).
        EffectKind::Shell => {
            let stage = {
                let program = b.wit_type_prim("string");
                let args = {
                    let s = b.wit_type_prim("string");
                    b.wit_type_list(s)
                };
                wit_type_record(&mut b, &[("program", program), ("args", args)])
            };
            let pipeline = b.wit_type_list(stage);
            let unit = b.wit_type_unit();
            let sig = b.wit_func_sig(&[("pipeline", pipeline)], unit);
            ("shell", "run", sig)
        }
        // now.read() -> record{ms: u64}. No params (unit-arg-free); the wall-clock ms anchor folds back as a
        // time-result event, the op result records the ms shape.
        EffectKind::Now => {
            let ms = b.wit_type_prim("u64");
            let time = wit_type_record(&mut b, &[("ms", ms)]);
            let sig = b.wit_func_sig(&[], time);
            ("now", "read", sig)
        }
        // timer.arm(deadline-ms: u64) -> unit. The fire arrives as a separate injected timer-fired event, not
        // this op's result.
        EffectKind::Timer => {
            let deadline = b.wit_type_prim("u64");
            let unit = b.wit_type_unit();
            let sig = b.wit_func_sig(&[("deadline-ms", deadline)], unit);
            ("timer", "arm", sig)
        }
        // emit.send(payload: list<u8>) -> unit. target-OUT: the peer session id rides `req.target` (the
        // RESOURCE a grant gates, SEC-F1), NOT the effect's schema/identity — consistent with the
        // target-out-of-schema ruling (identity = what-effect; target = which-resource). payload is the
        // signal body (opaque bytes). Delivery folds back as a separate ack/failure event, so the op result
        // is unit. (This is the one slice-1 built-in that listed `target` as an op-param; corrected here.)
        EffectKind::Emit => {
            let payload = {
                let u8_ty = b.wit_type_prim("u8");
                b.wit_type_list(u8_ty)
            };
            let unit = b.wit_type_unit();
            let sig = b.wit_func_sig(&[("payload", payload)], unit);
            ("emit", "send", sig)
        }
    };
    effect_schema_hash_from_nodes(b, name, &[(op_name, sig)])
}

/// The MEMOIZED schema-hash of a built-in effect — the accessor `EffectRequest::new` uses on the
/// construction path. [`builtin_effect_schema_hash`] is a PURE recompute (builds the schema tree, encodes
/// it, blake3-hashes it) that is far from free; a built-in's identity is FIXED (content-addressed over a
/// schema that never changes at runtime), so the operator model is that the kernel computes each built-in's
/// schema-hash ONCE and HOLDS it. This computes all six the first time it's called and returns an O(1) copy
/// thereafter — so threading a `schema_hash` onto every constructed [`crate::effect::EffectRequest`] costs a
/// slice-index, not a re-hash. The value is identical to [`builtin_effect_schema_hash`] by construction (it
/// just caches it), so the identity contract that function's test pins covers this too.
pub fn builtin_effect_schema_hash_memo(kind: &crate::effect::EffectKind) -> crate::hash::Hash {
    use crate::effect::EffectKind;
    // Order matches the index match below; a built-in's identity is frozen, so a `LazyLock` array is the
    // whole cache (six [u8;32] hashes, computed on first touch).
    static MEMO: std::sync::LazyLock<[crate::hash::Hash; 6]> = std::sync::LazyLock::new(|| {
        [
            builtin_effect_schema_hash(&EffectKind::Shell),
            builtin_effect_schema_hash(&EffectKind::Http),
            builtin_effect_schema_hash(&EffectKind::Model),
            builtin_effect_schema_hash(&EffectKind::Now),
            builtin_effect_schema_hash(&EffectKind::Timer),
            builtin_effect_schema_hash(&EffectKind::Emit),
        ]
    });
    let idx = match kind {
        EffectKind::Shell => 0,
        EffectKind::Http => 1,
        EffectKind::Model => 2,
        EffectKind::Now => 3,
        EffectKind::Timer => 4,
        EffectKind::Emit => 5,
    };
    MEMO[idx]
}

/// The schema-hash of a WELL-KNOWN effect family that has NO [`crate::effect::EffectKind`] variant — the
/// fs/blob/metric/ws/lifecycle world effects plus the control-plane families — under the schema-hash-only
/// effect model. Same hashing path as [`builtin_effect_schema_hash`] (`effect_schema_hash_from_nodes` over a
/// kernel-authored op-signature), reusing each family's landed payload/result shapes.
///
/// **target-OUT** (operator ruling via concierge 2026-08-13): the effect TARGET — the fs path, the blob
/// content-hash, the ws conn-id, the lifecycle session-id — rides `req.target` as the SEC-F1 RESOURCE a grant
/// gates; it is NOT an op-param of the schema. The schema is the effect's DATA shape only (payload in, result
/// out): identity = WHAT-effect, target = WHICH-resource, matching the landed authz-raw-bytes ruling. So a
/// target-only effect (fs/read, fs/glob, blob/get, ws/dial, lifecycle/suspend|resume|terminate) has an EMPTY
/// param list — it stays distinct from its siblings via the effect NAME + OP NAME, both hashed into the
/// schema tree (so lifecycle/suspend != resume != terminate though all are unit->unit).
///
/// A family string `"x/y"` decomposes to effect name `x`, op name `y`. `None` ONLY for a register-by-string
/// EXTENSION family (unknown to the kernel) — every WELL-KNOWN non-`EffectKind` family now carries a declared
/// schema (control/signature was the last to land). An exact-match table, not a prefix match.
pub fn family_effect_schema_hash(family: &str) -> Option<crate::hash::Hash> {
    use crate::effect::effect_ct;
    let mut b = Builder::new();
    // `list<u8>` — the uniform opaque-bytes descriptor (file content, blob content, a hash, a ws frame, a
    // session id). Takes `&mut Builder` as a param so it does not capture `b` (no borrow conflict).
    fn bytes(b: &mut Builder) -> StructId {
        let u8_ty = b.wit_type_prim("u8");
        b.wit_type_list(u8_ty)
    }
    // The 16 STRUCTURALLY-SIMPLE families collapse to a data table (family → name/op/one of four op-sig
    // SHAPES over `bytes`/`unit`), driven by one build loop below. HASH-PRESERVING: each row reconstructs the
    // EXACT same `wit_func_sig` (same op-name, same param-NAME where present, same result descriptor) the old
    // per-arm code built, so `family_effect_schema_hash` is byte-identical for every family (the identity is
    // frozen). The 6 STRUCTURALLY-COMPLEX families (blob/get option, metric/publish record, store/add|remove
    // MemberOp, store/resolve-all list, control/close variant) stay bespoke in the `match` below the table.
    // The four shapes (`bytes` = `list<u8>`, target-out means no target param — the target rides req.target):
    //   UnitToBytes   `()          -> bytes`   ·  BytesToUnit  `(<p>: bytes) -> unit`
    //   UnitToUnit    `()          -> unit`    ·  BytesToBytes `(<p>: bytes) -> bytes`
    enum SimpleShape {
        UnitToBytes,
        BytesToUnit(&'static str),
        UnitToUnit,
        BytesToBytes(&'static str),
    }
    use SimpleShape::*;
    // (family const, effect-name, op-name, shape). Param names are load-bearing (they participate in the
    // hash) — kept EXACTLY as the prior per-arm code spelled them.
    const SIMPLE: &[(&str, &str, &str, SimpleShape)] = &[
        (effect_ct::FS_READ, "fs", "read", UnitToBytes),
        (effect_ct::FS_WRITE, "fs", "write", BytesToUnit("content")),
        (effect_ct::FS_GLOB, "fs", "glob", UnitToBytes),
        (effect_ct::BLOB_PUT, "blob", "put", BytesToBytes("content")),
        (effect_ct::WS_SEND, "ws", "send", BytesToUnit("frame")),
        (effect_ct::WS_DIAL, "ws", "dial", UnitToUnit),
        (
            effect_ct::LIFECYCLE_SPAWN,
            "lifecycle",
            "spawn",
            BytesToBytes("reducer-hash"),
        ),
        (
            effect_ct::LIFECYCLE_SUSPEND,
            "lifecycle",
            "suspend",
            UnitToUnit,
        ),
        (
            effect_ct::LIFECYCLE_RESUME,
            "lifecycle",
            "resume",
            UnitToUnit,
        ),
        (
            effect_ct::LIFECYCLE_TERMINATE,
            "lifecycle",
            "terminate",
            UnitToUnit,
        ),
        (
            effect_ct::CAPABILITIES,
            "control",
            "capabilities",
            BytesToUnit("manifest"),
        ),
        (
            effect_ct::SUMMARY,
            "control",
            "summary",
            BytesToUnit("summary"),
        ),
        (effect_ct::SIGNATURE, "control", "signature", UnitToBytes),
        (effect_ct::STORE_SET, "store", "set", BytesToUnit("pointer")),
        (effect_ct::STORE_RESOLVE, "store", "resolve", UnitToBytes),
        (
            effect_ct::EFFECT_REPLY,
            "effect",
            "reply",
            BytesToUnit("response"),
        ),
    ];
    if let Some((_, name, op_name, shape)) = SIMPLE.iter().find(|(fam, ..)| *fam == family) {
        let sig = match shape {
            UnitToBytes => {
                let result = bytes(&mut b);
                b.wit_func_sig(&[], result)
            }
            BytesToUnit(p) => {
                let param = bytes(&mut b);
                let unit = b.wit_type_unit();
                b.wit_func_sig(&[(p, param)], unit)
            }
            UnitToUnit => {
                let unit = b.wit_type_unit();
                b.wit_func_sig(&[], unit)
            }
            BytesToBytes(p) => {
                let param = bytes(&mut b);
                let result = bytes(&mut b);
                b.wit_func_sig(&[(p, param)], result)
            }
        };
        return Some(effect_schema_hash_from_nodes(b, name, &[(op_name, sig)]));
    }
    // The 6 STRUCTURALLY-COMPLEX families — bespoke op-sig shapes (option/record/tuple/variant/list) that do
    // not fit the four simple shapes above; kept as explicit arms.
    let (name, op_name, sig): (&str, &str, StructId) = match family {
        // blob/get(-> option<bytes>): target = the content hash (target-out); result = the content if present
        // (CAS hit = Some, miss = None — a normal answer, not an error).
        effect_ct::BLOB_GET => {
            let content = bytes(&mut b);
            let maybe = b.wit_type_option(content);
            let sig = b.wit_func_sig(&[], maybe);
            ("blob", "get", sig)
        }
        // metric/publish(sample: record -> unit): the MetricSample shape (name/kind/value/unit/labels).
        effect_ct::METRIC_PUBLISH => {
            let name_ty = b.wit_type_prim("string");
            let kind_ty = b.wit_type_prim("string");
            let value_ty = b.wit_type_prim("f64");
            let unit_ty = b.wit_type_prim("string");
            let labels = {
                let k = b.wit_type_prim("string");
                let v = b.wit_type_prim("string");
                let pair = b.wit_type_tuple(&[k, v]);
                b.wit_type_list(pair)
            };
            let sample = wit_type_record(
                &mut b,
                &[
                    ("name", name_ty),
                    ("kind", kind_ty),
                    ("value", value_ty),
                    ("unit", unit_ty),
                    ("labels", labels),
                ],
            );
            let unit = b.wit_type_unit();
            let sig = b.wit_func_sig(&[("sample", sample)], unit);
            ("metric", "publish", sig)
        }
        // store/add | store/remove(op: member-op record -> unit): target = the group name (target-out);
        // payload = the OR-set MemberOp {add: bool, member: hash-bytes, tag: tuple<hash-bytes, u64>}. Distinct
        // identities via the OP NAME (add vs remove), both carrying the same MemberOp shape.
        effect_ct::STORE_ADD | effect_ct::STORE_REMOVE => {
            let add = b.wit_type_prim("bool");
            let member = bytes(&mut b);
            let tag = {
                let tag_hash = bytes(&mut b);
                let tag_seq = b.wit_type_prim("u64");
                b.wit_type_tuple(&[tag_hash, tag_seq])
            };
            let op = wit_type_record(&mut b, &[("add", add), ("member", member), ("tag", tag)]);
            let unit = b.wit_type_unit();
            let sig = b.wit_func_sig(&[("op", op)], unit);
            if family == effect_ct::STORE_ADD {
                ("store", "add", sig)
            } else {
                ("store", "remove", sig)
            }
        }
        // store/resolve-all(-> list<hash-bytes>): target = the group name (target-out); result = the current
        // member set (a list of member hashes).
        effect_ct::STORE_RESOLVE_ALL => {
            let members = {
                let member = bytes(&mut b);
                b.wit_type_list(member)
            };
            let sig = b.wit_func_sig(&[], members);
            ("store", "resolve-all", sig)
        }
        // effect/reply(response: bytes -> unit): target = the opaque 32-byte reply TOKEN (target-out); payload
        // = the response bytes. The host ReplyExecutor validates+consumes the token and settles the request.
        effect_ct::EFFECT_REPLY => {
            let response = bytes(&mut b);
            let unit = b.wit_type_unit();
            let sig = b.wit_func_sig(&[("response", response)], unit);
            ("effect", "reply", sig)
        }
        // control/close(outcome: CloseOutcome -> unit): NO target; payload = the close outcome, a variant
        // Success(payload-bytes) | Failure(message-string). The §6 self-close signal.
        effect_ct::CLOSE => {
            let success_payload = bytes(&mut b);
            let failure_msg = b.wit_type_prim("string");
            let outcome = wit_type_variant(
                &mut b,
                &[
                    ("Success", Some(success_payload)),
                    ("Failure", Some(failure_msg)),
                ],
            );
            let unit = b.wit_type_unit();
            let sig = b.wit_func_sig(&[("outcome", outcome)], unit);
            ("control", "close", sig)
        }
        _ => return None,
    };
    Some(effect_schema_hash_from_nodes(b, name, &[(op_name, sig)]))
}

/// Memoized [`family_effect_schema_hash`] — the accessor `EffectRequest::new_with_family` uses on the
/// construction path (same compute-once-and-hold model as [`builtin_effect_schema_hash_memo`]). The declared
/// families are hashed on first touch; a family not in the table returns `None`.
pub fn family_effect_schema_hash_memo(family: &str) -> Option<crate::hash::Hash> {
    use crate::effect::effect_ct;
    static MEMO: std::sync::LazyLock<std::collections::HashMap<&'static str, crate::hash::Hash>> =
        std::sync::LazyLock::new(|| {
            let mut m = std::collections::HashMap::new();
            for fam in [
                effect_ct::FS_READ,
                effect_ct::FS_WRITE,
                effect_ct::FS_GLOB,
                effect_ct::BLOB_PUT,
                effect_ct::BLOB_GET,
                effect_ct::METRIC_PUBLISH,
                effect_ct::WS_SEND,
                effect_ct::WS_DIAL,
                effect_ct::LIFECYCLE_SPAWN,
                effect_ct::LIFECYCLE_SUSPEND,
                effect_ct::LIFECYCLE_RESUME,
                effect_ct::LIFECYCLE_TERMINATE,
                effect_ct::CAPABILITIES,
                effect_ct::SUMMARY,
                effect_ct::SIGNATURE,
                effect_ct::STORE_SET,
                effect_ct::STORE_RESOLVE,
                effect_ct::STORE_ADD,
                effect_ct::STORE_REMOVE,
                effect_ct::STORE_RESOLVE_ALL,
                effect_ct::EFFECT_REPLY,
                effect_ct::CLOSE,
            ] {
                if let Some(h) = family_effect_schema_hash(fam) {
                    m.insert(fam, h);
                }
            }
            m
        });
    MEMO.get(family).copied()
}

/// The schema-hash of ANY well-known effect, keyed UNIFORMLY by its FAMILY STRING — the single host-callable
/// reflection a host executor (cdz-agent-host) uses to DECLARE its served schema-hash without knowing whether
/// its family happens to be an [`crate::effect::EffectKind`] built-in or a non-kind family. This is the
/// slice-B seam: an executor serving `"model"` or `"fs/read"` or `"store/set"` calls this once per served
/// family and matches `req.schema_hash` against the result — it never branches on `EffectKind`, so the
/// EffectKind-as-identity distinction the schema-hash-only removal is ELIMINATING does not leak into the host.
///
/// Resolves a built-in family via [`EffectKind::from_family`] → [`builtin_effect_schema_hash_memo`], else a
/// non-kind family via [`family_effect_schema_hash_memo`]. `Some` for EVERY family a host executor serves (all
/// 6 built-in families + the 22 declared non-kind families); `None` ONLY for a family with no declared schema —
/// a register-by-string EXTENSION unknown to the kernel, or an inbound-only name (`ws/connect`) no executor
/// dispatches — so a `None` can never strand a dispatch (an executor never serves a schemaless family).
pub fn effect_family_schema_hash(family: &str) -> Option<crate::hash::Hash> {
    use crate::effect::EffectKind;
    match EffectKind::from_family(family) {
        Some(k) => Some(builtin_effect_schema_hash_memo(&k)),
        None => family_effect_schema_hash_memo(family),
    }
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
    // Emit through the shared `cadenza-ast` descriptor builders (the single source ALL three world sources
    // target — v-syntax's `b27906601`), so build_type / the inline surface / rcdzc emit stay byte-identical.
    let prim = |b: &mut Builder, kind: &str| b.wit_type_prim(kind);
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
            let elem = build_type(b, &lt.ty())?;
            b.wit_type_list(elem)
        }
        // record → ("record" (fieldname <T>)…): string head, each field a (name type) 2-list, NAME-SORTED.
        // Fields are emitted in field-NAME order (concierge ruling 2026-08-13, constraint-forced) so a
        // reflected component's record hashes identically to the same record from a kernel built-in decl
        // (`wit_type_record`, name-sorted) and from the rcdzc userspace producer (a name-sorted `BTreeMap` that
        // can emit no other order) — the one canonical order all three schema producers agree on, matching the
        // value-form's already-name-sorted fields (`val_to_ast`).
        Type::Record(rt) => {
            let mut children = vec![b.atom_leaf(Leaf::Str("record".into()))];
            let mut sorted: Vec<_> = rt.fields().collect();
            sorted.sort_by(|x, y| x.name.cmp(y.name));
            for field in sorted {
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
            let inner = build_type(b, &ot.ty())?;
            b.wit_type_option(inner)
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

/// Encode a HANDLER REPLY outcome to its value-form bytes — the exact INVERSE of [`decode_reply_outcome`]
/// (`encode` then `decode` is the identity on the Ok/Err subset). A caller that builds a reply payload
/// (the host's `ReplyExecutor` + its tests) hands in a kernel [`EffectOutcome`] and gets the canonical
/// value-form bytes, WITHOUT touching wasmtime `Val`s directly (cdz-agent-host deliberately doesn't dep
/// wasmtime) — mirroring the other kernel encode helpers (`encode_name_set`, `encode_model_request`).
///
/// Restricted to the Ok/Err subset a handler can reply: `Ok(Some(Inline b))` → `(Ok (Inline b))`,
/// `Ok(Some(Blob h))` → `(Ok (Blob <hash>))`, `Ok(None)` → `(Ok (Inline []))`, `Err{message,retryability}`
/// → `(Err (record (= message ..) (= retryable ..)))`. `TimedOut`/`Deferred` are NOT handler-repliable
/// (kernel-injected / transient) — encoding one is a programming error, surfaced as `Unmarshallable`. Reuses
/// the SINGLE outcome-view builder (`wasm_host::effect_outcome_view`) so encode + the guest-Event outcome
/// child can never drift.
pub fn encode_reply_outcome(o: &EffectOutcome) -> Result<Vec<u8>, MarshalError> {
    match o {
        EffectOutcome::Ok(_) | EffectOutcome::Err { .. } => {
            val_to_ast(&crate::wasm_host::effect_outcome_view(o))
        }
        EffectOutcome::TimedOut => Err(unmarshallable(
            "EffectOutcome::TimedOut (not handler-repliable)",
        )),
        EffectOutcome::Deferred => Err(unmarshallable(
            "EffectOutcome::Deferred (not handler-repliable)",
        )),
    }
}

/// Decode a HANDLER REPLY's outcome value-form back into a kernel [`EffectOutcome`] — the DUAL of the
/// encode side (`wasm_host::effect_outcome_view`), restricted to the Ok/Err SUBSET a handler can reply. A
/// handler NEVER replies `TimedOut` (the kernel injects that on a deadline) and `Deferred` is a transient
/// executor signal, never a wire value — so this decodes only `Ok`/`Err`; any other ctor head is a
/// TypeMismatch. The host's `ReplyExecutor` calls this on the reply payload bytes and settles the resulting
/// `EffectOutcome`; it maps an `Err(_)` from here fail-closed to a PERMANENT error (a reply that can't be
/// read as a well-formed outcome must not be mistaken for a spurious success — same untrusted-input posture
/// as [`ast_to_val`]). Keeping the decode HERE, beside the value-form pin and the encode, keeps the codec in
/// one place so the two halves cannot drift.
///
/// The reply form is the bare ctor pinned by `val_to_ast_pins_the_err_reply_outcome_value_form`:
/// - `(Ok <ReplyPayload>)` where the payload is DISCRIMINATED so a blob-ref reply survives (operator ruling:
///   no-capability-drop): `(Ok (Inline <bytes>))` → `EffectOutcome::Ok(Some(Payload::Inline(bytes)))`;
///   `(Ok (Blob <32-hash-bytes>))` → `EffectOutcome::Ok(Some(Payload::Blob(hash)))` (a handler replies a
///   blob-ref for a large response so it need not inline into the durable log). A wrong-length blob hash or
///   an unknown payload head is a TypeMismatch (fail-closed);
/// - `(Err (record (= message <bytes>) (= retryable <bool>)))` → `EffectOutcome::Err { message, retryability }`,
///   `retryable` the TYPED retryability (true = `Retryable`, false = `Permanent` — the reducer folds on the
///   bool, not a parsed token). The record is STRICT: exactly `message` (valid utf-8 bytes) + `retryable`;
///   an extra/missing/duplicate field is a TypeMismatch.
pub fn decode_reply_outcome(bytes: &[u8]) -> Result<EffectOutcome, MarshalError> {
    let a = codec::decode(bytes).ok_or(MarshalError::Undecodable)?;
    let (case, payload) = ctor(&a, a.root)?;
    match case {
        "Ok" => {
            let node =
                payload.ok_or_else(|| type_mismatch("reply-outcome", "Ok without a payload"))?;
            Ok(EffectOutcome::Ok(Some(read_reply_payload(&a, node)?)))
        }
        "Err" => {
            let node =
                payload.ok_or_else(|| type_mismatch("reply-outcome", "Err without a payload"))?;
            let (message, retryable) = read_err_reply_record(&a, node)?;
            Ok(EffectOutcome::Err {
                message,
                retryability: if retryable {
                    Retryability::Retryable
                } else {
                    Retryability::Permanent
                },
            })
        }
        // TimedOut is caller-side only (kernel-injected); a handler replying it — or any other head — is a
        // malformed reply the host maps fail-closed to a permanent Err.
        other => Err(type_mismatch(
            "reply-outcome",
            format!("unexpected ctor {}", bounded_name(other)),
        )),
    }
}

/// Decode the Ok arm's DISCRIMINATED reply payload `(Inline <bytes>)` | `(Blob <32-hash-bytes>)` →
/// [`Payload::Inline`] / [`Payload::Blob`]. Preserving the discriminant is what keeps a blob-ref reply from
/// being flattened to opaque bytes (operator ruling). A `Blob` payload MUST be exactly 32 bytes (a
/// [`crate::hash::Hash`]); a wrong length, or any other payload head, is a TypeMismatch (fail-closed).
/// `Hash::from_bytes` is a raw-bytes constructor, not a forbidden hex `from_hex` decode.
fn read_reply_payload(a: &Arenas, id: StructId) -> Result<Payload, MarshalError> {
    let (case, node) = ctor(a, id)?;
    match case {
        "Inline" => {
            let n = node
                .ok_or_else(|| type_mismatch("reply-payload", "Inline without a bytes payload"))?;
            Ok(Payload::Inline(read_bytes(a, n)?.into()))
        }
        "Blob" => {
            let n = node.ok_or_else(|| type_mismatch("reply-payload", "Blob without a hash"))?;
            let raw = read_bytes(a, n)?;
            let hash: [u8; 32] = raw
                .as_slice()
                .try_into()
                .map_err(|_| type_mismatch("reply-payload", "Blob hash is not 32 bytes"))?;
            Ok(Payload::Blob(crate::hash::Hash::from_bytes(hash)))
        }
        other => Err(type_mismatch(
            "reply-payload",
            format!("unexpected payload ctor {}", bounded_name(other)),
        )),
    }
}

/// Read the err-reply `(record (= message <bytes>) (= retryable <bool>))` → (message, retryable). STRICT
/// against UNTRUSTED reply bytes (mirrors [`ast_to_val`]'s exact-record hardening): exactly the two fields,
/// no extra / missing / duplicate; the message bytes must be valid utf-8. Accepts the canonical `(= name
/// value)` 3-list and the legacy `(name value)` 2-list field spelling, the same tolerance `build_from_ast`
/// keeps.
fn read_err_reply_record(a: &Arenas, id: StructId) -> Result<(String, bool), MarshalError> {
    let field_nodes = form(a, id, "record")?;
    let mut message: Option<Vec<u8>> = None;
    let mut retryable: Option<bool> = None;
    for &fnode in field_nodes {
        let (name, val_node) = match a.get(fnode) {
            Struct::List(kids) if kids.len() == 3 && a.as_name(kids[0]) == Some("=") => {
                match a.as_name(kids[1]) {
                    Some(n) => (n, kids[2]),
                    None => return Err(type_mismatch("reply-outcome", "field name is not a name")),
                }
            }
            Struct::List(kids) if kids.len() == 2 => match a.as_name(kids[0]) {
                Some(n) => (n, kids[1]),
                None => return Err(type_mismatch("reply-outcome", "field name is not a name")),
            },
            _ => {
                return Err(type_mismatch(
                    "reply-outcome",
                    "Err field is not a (= name value) or (name value) form",
                ))
            }
        };
        match name {
            "message" => {
                if message.replace(read_bytes(a, val_node)?).is_some() {
                    return Err(type_mismatch("reply-outcome", "duplicate message field"));
                }
            }
            "retryable" => {
                if retryable.replace(read_bool(a, val_node)?).is_some() {
                    return Err(type_mismatch("reply-outcome", "duplicate retryable field"));
                }
            }
            other => {
                return Err(type_mismatch(
                    "reply-outcome",
                    format!("unknown Err field {}", bounded_name(other)),
                ))
            }
        }
    }
    let message = message.ok_or_else(|| type_mismatch("reply-outcome", "Err missing message"))?;
    let retryable =
        retryable.ok_or_else(|| type_mismatch("reply-outcome", "Err missing retryable"))?;
    let message = String::from_utf8(message)
        .map_err(|_| type_mismatch("reply-outcome", "message is not valid utf-8"))?;
    Ok((message, retryable))
}

/// Encode the §6 CHILD-COMPLETED signal to its value-form bytes — the Inbound payload a session's PARENT
/// folds when a child reaches a terminal outcome. ONE value-form covers BOTH close-classes: a self-close
/// (a reducer's `FoldOutput::close`, CloseOutcome Success|Failure) AND a terminate (`lifecycle/terminate`,
/// CloseOutcome::Failure(reason)) — the [`CloseOutcome`] discriminates the cause, so a guest supervisor folds
/// both uniformly. The host's supervision routing calls this + delivers the bytes to the parent inbox; the
/// parent's GUEST supervisor reducer `value-decode`s it. It is a VALUE-FORM (guest-decodable), NOT the durable
/// `event_ast::encode_child_exited` bytes (which a guest can't value-decode — same durable-vs-value-form
/// distinction as the err-reply outcome; migrate the terminate-I7 path onto THIS builder for a real guest).
///
/// Value-form: `(record (= child <bytes>) (= outcome <ChildOutcome>))` where `ChildOutcome = (Success
/// <ReplyPayload>) | (Failure <bytes>)` and `ReplyPayload = (Inline <bytes>) | (Blob <32-hash>)` — Success
/// REUSES the reply-outcome payload discriminant so a blob-ref success payload survives (no-capability-drop);
/// Failure carries the reason bytes. `child` is the completed child's 32-byte SessionId (= genesis hash).
pub fn encode_child_completed(child: &crate::hash::Hash, outcome: &CloseOutcome) -> Vec<u8> {
    val_to_ast(&child_completed_val(child, outcome))
        .expect("the child-completed value is always marshallable")
}

/// The `child-completed` value-form NODE `(record (= child <bytes>) (= outcome <CloseOutcome>))` — the same
/// record [`encode_child_completed`] marshals, exposed as a `Val` so [`build_event_document`] can surface it
/// as a FIRST-CLASS TYPED Event field (the §6 V2 per-child seam) rather than an opaque payload a `.cdz` guest
/// can't value-decode. `child` = the completed child's 32-byte genesis hash; `outcome` reuses the shared
/// [`CloseOutcome`] value-form (Success(ReplyPayload)|Failure(bytes)).
pub fn child_completed_val(child: &crate::hash::Hash, outcome: &CloseOutcome) -> Val {
    let bytes = |b: &[u8]| Val::List(b.iter().copied().map(Val::U8).collect());
    Val::Record(vec![
        ("child".into(), bytes(child.as_bytes())),
        ("outcome".into(), close_outcome_val(outcome)),
    ])
}

/// The shared `CloseOutcome` value-form node `(Success <ReplyPayload>) | (Failure <bytes>)` — the `outcome`
/// field of [`encode_child_completed`] AND the whole payload of a `control/close` self-close signal
/// ([`encode_close_outcome`]). Success REUSES the reply-outcome payload discriminant (`(Inline <bytes>) |
/// (Blob <32-hash>)`) so a blob-ref success payload survives (no-capability-drop); Failure carries the
/// reason bytes. One builder so the child-completed and self-close forms cannot drift.
fn close_outcome_val(outcome: &CloseOutcome) -> Val {
    let bytes = |b: &[u8]| Val::List(b.iter().copied().map(Val::U8).collect());
    let payload_view = |p: &Payload| match p {
        Payload::Inline(b) => Val::Variant("Inline".into(), Some(Box::new(bytes(b)))),
        Payload::Blob(h) => Val::Variant("Blob".into(), Some(Box::new(bytes(h.as_bytes())))),
    };
    match outcome {
        CloseOutcome::Success(p) => Val::Variant("Success".into(), Some(Box::new(payload_view(p)))),
        CloseOutcome::Failure(reason) => {
            Val::Variant("Failure".into(), Some(Box::new(bytes(reason.as_bytes()))))
        }
    }
}

/// Encode a bare `CloseOutcome` value-form — the payload a `.cdz` guest reducer emits on a `control/close`
/// effect to SELF-COMPLETE (§6 supervision). The DUAL of [`decode_close_outcome`], and the same node
/// [`encode_child_completed`] nests under its `outcome` field. Provided for the Rust reducer / host path +
/// round-trip testing; a guest builds these bytes via rcdzc instead.
pub fn encode_close_outcome(outcome: &CloseOutcome) -> Vec<u8> {
    val_to_ast(&close_outcome_val(outcome)).expect("the close-outcome value is always marshallable")
}

/// Decode a bare `CloseOutcome` value-form `(Success <ReplyPayload>) | (Failure <bytes>)` from bytes — the
/// payload a `.cdz` GUEST reducer emits on a [`control/close`](crate::effect::effect_ct::CLOSE) effect to
/// SELF-COMPLETE. A guest reducer's `apply` can only return a value-form effect-list (it can't return
/// [`crate::reducer::FoldOutput::close`] like a Rust reducer), so it signals a clean self-close through the
/// effect-list; the wasm fold adapter value-decodes the outcome HERE and maps it to `FoldOutput::close`.
/// The top-level entry point onto the same `(Success|Failure)` decode [`decode_child_completed`] runs on
/// its nested `outcome` field. STRICT / fail-closed against untrusted guest bytes: undecodable bytes, an
/// unknown outcome/payload head, a wrong-length blob hash, or a non-utf-8 Failure reason is a `TypeMismatch`
/// (a guest asking to close with a malformed outcome is a fold failure, not a silent success).
pub fn decode_close_outcome(bytes: &[u8]) -> Result<CloseOutcome, MarshalError> {
    let a = codec::decode(bytes).ok_or(MarshalError::Undecodable)?;
    read_child_outcome(&a, a.root)
}

/// Decode a [`encode_child_completed`] value-form back into `(child SessionId, CloseOutcome)` — the DUAL of
/// the encode (for the host's Rust supervisor path + round-trip testing; a guest supervisor value-decodes it
/// via rcdzc instead). STRICT (untrusted-input posture): exactly `child` (32 bytes) + `outcome`; extra /
/// missing / duplicate field, a non-32-byte child, an unknown outcome/payload head, or a non-utf-8 Failure
/// reason is a TypeMismatch. `Hash::from_bytes` is a raw-bytes constructor, not a forbidden hex decode.
pub fn decode_child_completed(
    bytes: &[u8],
) -> Result<(crate::hash::Hash, CloseOutcome), MarshalError> {
    let a = codec::decode(bytes).ok_or(MarshalError::Undecodable)?;
    let field_nodes = form(&a, a.root, "record")?;
    let mut child: Option<Vec<u8>> = None;
    let mut outcome: Option<CloseOutcome> = None;
    for &fnode in field_nodes {
        let (name, val_node) = read_named_field(&a, fnode, "child-completed")?;
        match name {
            "child" => {
                if child.replace(read_bytes(&a, val_node)?).is_some() {
                    return Err(type_mismatch("child-completed", "duplicate child field"));
                }
            }
            "outcome" => {
                if outcome.replace(read_child_outcome(&a, val_node)?).is_some() {
                    return Err(type_mismatch("child-completed", "duplicate outcome field"));
                }
            }
            other => {
                return Err(type_mismatch(
                    "child-completed",
                    format!("unknown field {}", bounded_name(other)),
                ))
            }
        }
    }
    let child = child.ok_or_else(|| type_mismatch("child-completed", "missing child field"))?;
    let hash: [u8; 32] = child
        .as_slice()
        .try_into()
        .map_err(|_| type_mismatch("child-completed", "child is not a 32-byte hash"))?;
    let outcome =
        outcome.ok_or_else(|| type_mismatch("child-completed", "missing outcome field"))?;
    Ok((crate::hash::Hash::from_bytes(hash), outcome))
}

/// The child-completed outcome value-form `(Success <ReplyPayload>)` | `(Failure <bytes>)` → [`CloseOutcome`].
/// Success reuses [`read_reply_payload`] (Inline|Blob) so a blob-ref success payload survives; Failure's
/// reason bytes must be valid utf-8. Any other head is a TypeMismatch (fail-closed).
fn read_child_outcome(a: &Arenas, id: StructId) -> Result<CloseOutcome, MarshalError> {
    let (case, node) = ctor(a, id)?;
    match case {
        "Success" => {
            let n =
                node.ok_or_else(|| type_mismatch("child-outcome", "Success without a payload"))?;
            Ok(CloseOutcome::Success(read_reply_payload(a, n)?))
        }
        "Failure" => {
            let n =
                node.ok_or_else(|| type_mismatch("child-outcome", "Failure without a reason"))?;
            let reason = String::from_utf8(read_bytes(a, n)?)
                .map_err(|_| type_mismatch("child-outcome", "Failure reason is not valid utf-8"))?;
            Ok(CloseOutcome::Failure(reason))
        }
        other => Err(type_mismatch(
            "child-outcome",
            format!("unexpected outcome ctor {}", bounded_name(other)),
        )),
    }
}

/// Extract `(name, value-node)` from a record field entry — the canonical `(= name value)` 3-list or the
/// legacy `(name value)` 2-list (the same tolerance `build_from_ast` keeps). `ctx` labels the error.
fn read_named_field<'a>(
    a: &'a Arenas,
    fnode: StructId,
    ctx: &'static str,
) -> Result<(&'a str, StructId), MarshalError> {
    match a.get(fnode) {
        Struct::List(kids) if kids.len() == 3 && a.as_name(kids[0]) == Some("=") => a
            .as_name(kids[1])
            .map(|n| (n, kids[2]))
            .ok_or_else(|| type_mismatch(ctx, "field name is not a name")),
        Struct::List(kids) if kids.len() == 2 => a
            .as_name(kids[0])
            .map(|n| (n, kids[1]))
            .ok_or_else(|| type_mismatch(ctx, "field name is not a name")),
        _ => Err(type_mismatch(
            ctx,
            "field is not a (= name value) or (name value) form",
        )),
    }
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
use crate::event::{CloseOutcome, EffectOutcome, Retryability};
use crate::reducer::Effect;

/// A borrowed content-type view for [`build_event_document`] — `(family, version)` — so the caller passes
/// the event's content-type without this module depending on the `event::ContentType` layout beyond it.
pub struct ContentTypeRef<'a> {
    pub family: &'a str,
    pub version: u32,
}

/// Build the ONE event document the fold boundary passes IN (B1): fold the content-type, the optional
/// payload, the optional resume token, and the optional effect `outcome` into a single value-form AST,
/// returning its canonical bytes. The guest `value-decode`s these bytes against the event descriptor.
/// Reuses the shared `cadenza-ast` codec.
///
/// `outcome` is the discriminated effect-result view (`Some(Ok|Err|TimedOut)` for an `EffectResult` event,
/// `None` for every other event kind) — the caller (`wasm_host::event_to_guest_inputs`) builds it via
/// `effect_outcome_view` so the guest can tell a successful reply from a failure, which the raw `payload`
/// bytes alone cannot express (the discriminant is dropped by `effect_outcome_bytes`).
pub fn build_event_document(
    content_type: ContentTypeRef,
    payload: Option<&[u8]>,
    resumes: Option<&[u8]>,
    outcome: Option<Val>,
    child_completed: Option<Val>,
) -> Vec<u8> {
    // Build the Event as a wasmtime `Val` and marshal it via the SHARED canonical codec ([`val_to_ast`]),
    // so the bytes are the deterministic value-form the guest's `value-decode` (op 90) reconstructs the
    // Event from — byte-identical to what `value-encode` produces (record-type Phase B; the ad-hoc named
    // form is gone). The Event is `record { content-type: record { family: string, version: u32 },
    // payload: option<list<u8>>, resumes: option<list<u8>>, outcome: option<Ok(list<u8>) | Err{message:
    // list<u8>, retryable: bool} | TimedOut>, child-completed: option<record { child: list<u8>, outcome:
    // Success(<ReplyPayload>) | Failure(list<u8>) }> }`; the kebab field names match the guest reducer's Event type
    // (Cadenza allows kebab identifiers). `list<u8>` marshals to a `Bytes` leaf; an `option` present/absent
    // (incl. an empty `Some []`) distinguishes an empty payload from an absent one. Record fields are matched
    // by NAME on decode (order-independent), but the field SET must match the guest's Event type exactly.
    let opt_bytes = |v: Option<&[u8]>| {
        Val::Option(
            v.map(|bytes| Box::new(Val::List(bytes.iter().copied().map(Val::U8).collect()))),
        )
    };
    let event = Val::Record(vec![
        (
            "content-type".to_string(),
            Val::Record(vec![
                (
                    "family".to_string(),
                    Val::String(content_type.family.to_string()),
                ),
                ("version".to_string(), Val::U32(content_type.version)),
            ]),
        ),
        ("payload".to_string(), opt_bytes(payload)),
        ("resumes".to_string(), opt_bytes(resumes)),
        ("outcome".to_string(), Val::Option(outcome.map(Box::new))),
        // §6 V2 per-child: `Some(record{child,outcome})` ONLY for a ChildCompleted event, `None` otherwise —
        // so a `.cdz` supervisor reads `(. e child-completed)` and gets the typed child id + CloseOutcome
        // directly (child = the completed child's genesis hash), rather than value-decoding an opaque payload.
        (
            "child-completed".to_string(),
            Val::Option(child_completed.map(Box::new)),
        ),
    ]);
    // The Event's shape is fixed (record/option/list<u8>/string/u32 + the outcome sum — all marshallable),
    // so `val_to_ast` never errors here.
    val_to_ast(&event).expect("the Event value is always marshallable")
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
    // Type descriptors via the shared `cadenza-ast` builders (the single source ALL three world sources
    // target — v-syntax's `b27906601`), so this artifact stays byte-identical to the inline surface + rcdzc
    // emit: `list<u8>` is `("list" (u8))`, `option<list<u8>>` is `("option" ("list" (u8)))`. `unit` is
    // `("unit")` (str head, no children) — no shared builder yet (MVP is prim/list/option), so built inline.
    let bytes_desc = |b: &mut Builder| {
        let u8_prim = b.wit_type_prim("u8");
        b.wit_type_list(u8_prim)
    };
    let opt_bytes_desc = |b: &mut Builder| {
        let inner = bytes_desc(b);
        b.wit_type_option(inner)
    };
    // `unit` = STR-head `("unit")` (put's result), via the shared builder (v-syntax's `wit_type_unit`).
    let unit_desc = |b: &mut Builder| b.wit_type_unit();
    // `bool` is a NAME-head primitive descriptor `(bool)`, like `(u8)` — the faithful boundary form of
    // `kv.delete`'s scalar result (present-or-removed). No retptr lift needed (a flat scalar, unlike
    // `option<list<u8>>`), so it emits like `put`'s unit result.
    let bool_desc = |b: &mut Builder| b.wit_type_prim("bool");
    // `kv.prefix-scan`'s result `list<tuple<list<u8>, list<u8>>>` — the key-value pairs. STR-head compounds
    // (build_type form): `("list" ("tuple" ("list" (u8)) ("list" (u8))))`, via the shared builders
    // (`wit_type_tuple`/`wit_type_list`). The nested list-of-byte-pairs is a genuine compound host result
    // (rcdzc's GAP-C+ lift, landed) — not a flat scalar. All 5 MVP member types now route through the
    // single shared `wit_type_*` source (prim/list/option/unit/tuple), byte-identical across all 3 sources.
    let scan_pairs_desc = |b: &mut Builder| {
        let k = bytes_desc(b);
        let v = bytes_desc(b);
        let pair = b.wit_type_tuple(&[k, v]);
        b.wit_type_list(pair)
    };

    // export `fold`: `apply(event: list<u8>) -> list<u8>`
    let apply_sig = {
        let ev = bytes_desc(&mut b);
        let res = bytes_desc(&mut b);
        b.wit_func_sig(&[("event", ev)], res)
    };
    let fold = b.wit_interface(
        WitDir::Export,
        "cadenza:agent-kernel/fold",
        &[("apply", apply_sig)],
    );

    // import `kv`: the full reducer.wit interface — `get(key) -> option<list<u8>>`,
    // `put(key, value)` (unit), `delete(key) -> bool`, `prefix-scan(prefix) -> list<tuple<list<u8>,
    // list<u8>>>`. Member order matches reducer.wit (get, put, delete, prefix-scan). All four now emit:
    // get's option + prefix-scan's nested list-of-pairs are compound (retptr) lifts, put's unit +
    // delete's bool are flat scalars.
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
    let delete_sig = {
        let key = bytes_desc(&mut b);
        let res = bool_desc(&mut b);
        b.wit_func_sig(&[("key", key)], res)
    };
    let prefix_scan_sig = {
        let prefix = bytes_desc(&mut b);
        let res = scan_pairs_desc(&mut b);
        b.wit_func_sig(&[("prefix", prefix)], res)
    };
    let kv = b.wit_interface(
        WitDir::Import,
        "cadenza:agent-kernel/kv",
        &[
            ("get", get_sig),
            ("put", put_sig),
            ("delete", delete_sig),
            ("prefix-scan", prefix_scan_sig),
        ],
    );

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
    // `list<u8>` descriptor via the shared `cadenza-ast` builders (single-source, byte-identical across
    // sources — v-syntax's `b27906601`), same as [`reducer_world_artifact`].
    let bytes_desc = |b: &mut Builder| {
        let u8_prim = b.wit_type_prim("u8");
        b.wit_type_list(u8_prim)
    };
    // export `fold`: `apply(event: list<u8>) -> list<u8>` — the only member; no `kv` import.
    let apply_sig = {
        let ev = bytes_desc(&mut b);
        let res = bytes_desc(&mut b);
        b.wit_func_sig(&[("event", ev)], res)
    };
    let fold = b.wit_interface(
        WitDir::Export,
        "cadenza:agent-kernel/fold",
        &[("apply", apply_sig)],
    );
    let world = b.world_schema_tree("reducer", &[fold]);
    codec::encode(&b.finish(world))
}

/// Read a canonical value-form record `(record (= name value)…)` (legacy `(name value)` fields also
/// accepted, matching the migration tolerance) into its `(field-name, value-node)` pairs — the same field
/// shape [`build_from_ast`]'s record arm reads. A malformed field entry is a `TypeMismatch`.
fn read_canonical_record(a: &Arenas, id: StructId) -> Result<Vec<(&str, StructId)>, MarshalError> {
    let fields = form(a, id, "record")?;
    let mut out = Vec::with_capacity(fields.len());
    for &f in fields {
        let (name, val) = match a.get(f) {
            Struct::List(kids) if kids.len() == 3 && a.as_name(kids[0]) == Some("=") => {
                match a.as_name(kids[1]) {
                    Some(n) => (n, kids[2]),
                    None => return Err(type_mismatch("record", "field name is not a name")),
                }
            }
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
        out.push((name, val));
    }
    Ok(out)
}

/// Read a canonical `option<list<u8>>` value — `(Some <bytes>)` / `(None <unit>)` (the capital ctor form
/// `value-encode` produces; a nullary `None`'s payload is the unit atom) → `Some(bytes)` / `None`.
fn read_canonical_opt_bytes(a: &Arenas, id: StructId) -> Result<Option<Vec<u8>>, MarshalError> {
    let (case, payload) = ctor(a, id)?;
    match case {
        "Some" => Ok(Some(read_bytes(
            a,
            payload.ok_or_else(|| type_mismatch("option", "Some without payload"))?,
        )?)),
        "None" => Ok(None),
        other => Err(type_mismatch(
            "option",
            format!("case {} ∉ {{Some,None}}", bounded_name(other)),
        )),
    }
}

/// Deep-copy the subtree rooted at `id` of `a` into a fresh arena and encode it to its canonical codec
/// bytes — the standalone byte form of a nested value-form value. Uses only the public `Arenas`/`Builder`
/// surface (an atom copies its leaf; a list copies its children), so it needs no codec-internal access.
/// This is how a `Structured` payload's inner value (e.g. an M1 `ModelRequest` record) becomes the bytes a
/// schema-typed host decoder consumes — the SAME value-form the guest emitted, re-serialized standalone.
fn reencode_subtree(a: &Arenas, id: StructId) -> Vec<u8> {
    fn copy_node(a: &Arenas, id: StructId, b: &mut Builder) -> StructId {
        match a.get(id) {
            Struct::Atom(lid) => {
                let leaf = a.leaf(*lid).clone();
                b.atom_leaf(leaf)
            }
            Struct::List(kids) => {
                let kid_ids: Vec<StructId> = kids.clone();
                let copied: Vec<StructId> = kid_ids.iter().map(|&k| copy_node(a, k, b)).collect();
                b.list(copied)
            }
        }
    }
    let mut b = Builder::new();
    let root = copy_node(a, id, &mut b);
    codec::encode(&b.finish(root))
}

/// Read a canonical `option<payload>` under the §GAP-1 b1 SHAPE — a SELF-DESCRIBING value-form dispatch,
/// NOT a tagged sum the kernel must know the arms of. The inner value distinguishes ITSELF by shape: a bare
/// `list<u8>` marshals to a single bytes-LEAF (`val_to_ast`'s `list<u8>` special-case) and IS an opaque
/// payload; a name-headed COMPOUND `(Structured <value>)` is a structured value-form payload, re-encoded
/// standalone to the value-form bytes a schema-typed decoder (e.g. `decode_model_request`) reads.
/// `(None <unit>)` → `None`. Both arms yield the effect's inline bytes, so the kernel [`Payload::Inline`]
/// stays a UNIFORM byte payload. This is not an adapter: the value-form already distinguishes a leaf from a
/// compound, so "leaf = opaque, `Structured`-compound = value-form" is a dispatch over the ONE value-form —
/// and an effect's IDENTITY keys on its schema-hash, never on this payload shape. An opaque reducer emits a
/// bare bytes payload UNCHANGED; only a reducer with a structured payload wraps it in a `Structured` ctor.
fn read_canonical_opt_payload(a: &Arenas, id: StructId) -> Result<Option<Vec<u8>>, MarshalError> {
    let (case, payload) = ctor(a, id)?;
    match case {
        "Some" => {
            let inner = payload.ok_or_else(|| type_mismatch("option", "Some without payload"))?;
            match a.get(inner) {
                // A bare bytes-leaf IS an opaque payload (the natural value-form of `list<u8>`).
                Struct::Atom(_) => Ok(Some(read_bytes(a, inner)?)),
                // A name-headed compound is a TAGGED payload arm. A reducer whose payload field is a
                // TWO-arm sum `Raw(Bytes) | Structured(<value>)` (needed when it emits BOTH opaque and
                // structured payloads — the two arms defeat newtype erasure, so the tag survives on the
                // wire) tags opaque bytes as `(Raw <bytes>)` and a structured value as `(Structured <v>)`.
                // `Raw` unwraps to the opaque bytes; `Structured` re-encodes its value standalone. (A
                // single-payload reducer instead emits a bare bytes-leaf — the Atom arm above.)
                Struct::List(_) => {
                    let (tag, val) = ctor(a, inner)?;
                    match tag {
                        "Raw" => Ok(Some(read_bytes(
                            a,
                            val.ok_or_else(|| type_mismatch("payload", "Raw without bytes"))?,
                        )?)),
                        "Structured" => Ok(Some(reencode_subtree(
                            a,
                            val.ok_or_else(|| {
                                type_mismatch("payload", "Structured without value")
                            })?,
                        ))),
                        other => Err(type_mismatch(
                            "payload",
                            format!("payload arm {} ∉ {{Raw,Structured}}", bounded_name(other)),
                        )),
                    }
                }
            }
        }
        "None" => Ok(None),
        other => Err(type_mismatch(
            "option",
            format!("case {} ∉ {{Some,None}}", bounded_name(other)),
        )),
    }
}

/// Parse the effect-list document the fold boundary returns (B1) into `Vec<Effect>` — the dual of
/// [`build_event_document`]. The guest `value-encode`s its returned `list<effect-request>` as the canonical
/// value-form `(list <record>…)` — a NAME-head `list` of canonical `(record (= field value)…)` elements.
/// TOTAL over arbitrary bytes: undecodable → `Undecodable`; a well-formed-but-wrong shape → `TypeMismatch`.
pub fn parse_effect_list(bytes: &[u8]) -> Result<Vec<Effect>, MarshalError> {
    let a = codec::decode(bytes).ok_or(MarshalError::Undecodable)?;
    let reqs = form(&a, a.root, "list")?;
    let mut out = Vec::with_capacity(reqs.len());
    for &r in reqs {
        out.push(parse_effect_request(&a, r)?);
    }
    Ok(out)
}

/// Parse one effect-request from its canonical value-form record `(record (= correlation <opt>) (= kind
/// <family-string>) (= payload <opt>) (= target <bytes>))` (fields sorted by name) into an [`Effect`].
/// `kind` is the effect FAMILY STRING (seq-39 identity; register-by-string, NEVER a closed enum) → mapped
/// via [`EffectRequest::new_with_family`] (a well-known family resolves, an extension family takes the
/// inert `Emit` placeholder keyed on the family). Timeliness defaults to `Interactive` (not on the wire yet).
fn parse_effect_request(a: &Arenas, id: StructId) -> Result<Effect, MarshalError> {
    let fields = read_canonical_record(a, id)?;
    let field = |name: &str| fields.iter().find(|(n, _)| *n == name).map(|(_, v)| *v);
    let family = read_str(
        a,
        field("kind").ok_or_else(|| type_mismatch("effect-request", "missing kind"))?,
    )?
    .to_string();
    let target = read_bytes(
        a,
        field("target").ok_or_else(|| type_mismatch("effect-request", "missing target"))?,
    )?;
    // payload/correlation absent if the field is omitted (a fire-and-forget, payload-free effect).
    // The payload is the b1 self-describing shape (bare bytes = opaque | Structured compound = value-form);
    // correlation stays a bare opaque option<bytes> (a correlation token is never structured).
    let payload = match field("payload") {
        Some(n) => read_canonical_opt_payload(a, n)?,
        None => None,
    }
    .map(|b| Payload::Inline(b.into()));
    let token = match field("correlation") {
        Some(n) => read_canonical_opt_bytes(a, n)?,
        None => None,
    };
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

    // Pin the CANONICAL val_to_ast value-form of a §GAP-1 M1 `ModelRequest` — the SHARED CONTRACT the b1
    // model-effect co-land hangs on. Under b1 the reducer builds a `ModelRequest` as an ordinary structural
    // record (its `ContentBlock` sum declared nominally per v-compiler-ml's Q1 answer, the record fields
    // structural), the A1 boundary marshals it through THIS path, and `decode_model_request` (v-compiler-ml,
    // re-pointed from the bespoke `(model-request …)` head to the value-form) reads exactly these bytes. So
    // this test IS the b1 wire: the field/ctor NAMES are the contract vocabulary, and the SHAPE invariants
    // pinned here (sum-ctor head is a NAME leaf — value-decode reads a case by name-head, a Str head decodes
    // to NULL; record fields SORTED by name — v-compiler-ml's decode is by-name/order-independent, so the
    // reducer's field-declaration order never matters; nested record-in-sum-arm-in-list fully supported per
    // the Q2 answer) are what any drift is caught against. `max-tokens` is kebab (Cadenza-idiomatic; decode
    // maps it to the struct's `max_tokens`); every other field is single-word, identical either convention.
    #[test]
    fn val_to_ast_pins_the_b1_model_request_value_form() {
        // Nav helpers over the decoded arena (nested `fn`s — they capture nothing, take `&Arenas`).
        fn kids(a: &Arenas, id: StructId) -> Vec<StructId> {
            match a.get(id) {
                Struct::List(k) => k.to_vec(),
                Struct::Atom(_) => panic!("expected a list at {id:?}"),
            }
        }
        fn field_names(a: &Arenas, record_kids: &[StructId]) -> Vec<String> {
            // record_kids[0] is the `record` head; each remaining child is a `(= name value)` 3-list.
            record_kids[1..]
                .iter()
                .map(|id| {
                    let f = kids(a, *id);
                    assert_eq!(f.len(), 3, "field is a (= name value) 3-list");
                    assert_eq!(a.as_name(f[0]), Some("="), "field head is the `=` name");
                    a.as_name(f[1]).expect("field name is a name").to_string()
                })
                .collect()
        }
        // record_kids[i] is `(= <name> <value>)`; return the value node.
        fn field_val(a: &Arenas, record_kids: &[StructId], i: usize) -> StructId {
            kids(a, record_kids[i])[2]
        }

        // A representative M1: one user turn carrying a Text block AND a ToolCall block, one tool offered,
        // a max-tokens cap — exercises every sub-shape (record-in-list, sum-in-list, record-in-sum-arm, bytes).
        let text_block = Val::Variant("Text".into(), Some(Box::new(Val::String("hi".into()))));
        let tool_call_block = Val::Variant(
            "ToolCall".into(),
            Some(Box::new(Val::Record(vec![
                ("id".into(), Val::String("t1".into())),
                ("name".into(), Val::String("shell".into())),
                (
                    "input".into(),
                    Val::List(vec![Val::U8(b'{'), Val::U8(b'}')]),
                ),
            ]))),
        );
        let message = Val::Record(vec![
            ("role".into(), Val::String("user".into())),
            (
                "content".into(),
                Val::List(vec![text_block, tool_call_block]),
            ),
        ]);
        let tool = Val::Record(vec![
            ("name".into(), Val::String("shell".into())),
            (
                "schema".into(),
                Val::List(vec![Val::U8(b'{'), Val::U8(b'}')]),
            ),
        ]);
        let model_request = Val::Record(vec![
            ("model".into(), Val::String("claude".into())),
            ("messages".into(), Val::List(vec![message])),
            ("tools".into(), Val::List(vec![tool])),
            (
                "max-tokens".into(),
                Val::Option(Some(Box::new(Val::U64(1024)))),
            ),
        ]);

        let a = decode(&val_to_ast(&model_request).expect("val_to_ast"));

        // Root: the ModelRequest record — NAME head `record`, fields SORTED by name.
        let mr = kids(&a, a.root);
        assert_eq!(a.as_name(mr[0]), Some("record"), "ModelRequest is a record");
        assert!(
            matches!(leaf_at(&a, mr[0]), Leaf::Name(_)),
            "record head is a NAME leaf, not a Str"
        );
        assert_eq!(
            field_names(&a, &mr),
            vec!["max-tokens", "messages", "model", "tools"],
            "M1 fields sorted by name (contract vocabulary)"
        );

        // model → the Str leaf "claude".
        assert_eq!(
            leaf_at(&a, field_val(&a, &mr, 3)),
            &Leaf::Str("claude".into()),
            "model field is the Str \"claude\""
        );
        // max-tokens → (Some <int>): a 2-list NAME-head ctor.
        let max_tokens = kids(&a, field_val(&a, &mr, 1));
        assert_eq!(
            a.as_name(max_tokens[0]),
            Some("Some"),
            "max-tokens is Some(..)"
        );

        // messages → (list <msg>); the message is a record with fields sorted content, role.
        let messages = kids(&a, field_val(&a, &mr, 2));
        assert_eq!(
            a.as_name(messages[0]),
            Some("list"),
            "messages is a (list ..)"
        );
        let msg = kids(&a, messages[1]);
        assert_eq!(a.as_name(msg[0]), Some("record"), "a message is a record");
        assert_eq!(
            field_names(&a, &msg),
            vec!["content", "role"],
            "ChatMessage fields sorted"
        );

        // content → (list <block>…): block[0] = (Text "hi"), block[1] = (ToolCall (record id,input,name)).
        let content = kids(&a, field_val(&a, &msg, 1));
        assert_eq!(
            a.as_name(content[0]),
            Some("list"),
            "content is a (list ..)"
        );
        let text = kids(&a, content[1]);
        assert_eq!(a.as_name(text[0]), Some("Text"), "block 0 is the Text ctor");
        assert!(
            matches!(leaf_at(&a, text[0]), Leaf::Name(_)),
            "ContentBlock ctor head is a NAME leaf (value-decode reads a case by name-head)"
        );
        assert_eq!(
            leaf_at(&a, text[1]),
            &Leaf::Str("hi".into()),
            "Text payload is the Str \"hi\""
        );
        let tool_call = kids(&a, content[2]);
        assert_eq!(
            a.as_name(tool_call[0]),
            Some("ToolCall"),
            "block 1 is the ToolCall ctor"
        );
        let tc_rec = kids(&a, tool_call[1]);
        assert_eq!(
            field_names(&a, &tc_rec),
            vec!["id", "input", "name"],
            "ToolCall record fields sorted (input is a list<u8> → Bytes leaf)"
        );
    }

    // Pin the CANONICAL val_to_ast value-form of the §err-reply effect-result OUTCOME — the SHARED CONTRACT
    // the err-reply co-land hangs on. Today event_to_guest_inputs flattens EventBody::EffectResult via
    // effect_outcome_bytes, DROPPING the Ok/Err/TimedOut discriminant (the guest can't tell a successful
    // reply from an error). The seam surfaces a first-class `outcome` child on the reducer Event, a value-form
    // faithfully mirroring the kernel EffectOutcome: `outcome: option<Ok(bytes) | Err{message,retryable} |
    // TimedOut>` — None for non-EffectResult events, Some(view) for an effect result. The HANDLER's reply
    // (v-agent-harness-host's ReplyExecutor decodes it from the reply payload) uses the Ok/Err subset
    // (no TimedOut — kernel-injected only). Pinned so any drift in the sum-ctor heads (must be NAME leaves —
    // value-decode reads a case by name-head, a Str head decodes to NULL) or the Err record field order
    // (SORTED by name: message < retryable) is caught. Names are the err-reply contract vocabulary.
    #[test]
    fn val_to_ast_pins_the_err_reply_outcome_value_form() {
        fn kids(a: &Arenas, id: StructId) -> Vec<StructId> {
            match a.get(id) {
                Struct::List(k) => k.to_vec(),
                Struct::Atom(_) => panic!("expected a list at {id:?}"),
            }
        }
        let bytes = |s: &[u8]| Val::List(s.iter().copied().map(Val::U8).collect());

        // (Some (Ok (Inline <response-bytes>))) — a successful reply carrying the response INLINE. The Ok
        // payload is DISCRIMINATED (Inline | Blob) so a blob-ref reply survives (operator ruling:
        // no-capability-drop) — NOT flattened to bare bytes.
        let ok = Val::Option(Some(Box::new(Val::Variant(
            "Ok".into(),
            Some(Box::new(Val::Variant(
                "Inline".into(),
                Some(Box::new(bytes(b"reply-ok"))),
            ))),
        ))));
        let a = decode(&val_to_ast(&ok).expect("val_to_ast"));
        let some = kids(&a, a.root);
        assert_eq!(some.len(), 2, "outcome option is a 2-list (Some payload)");
        assert_eq!(a.as_name(some[0]), Some("Some"), "outcome option head");
        assert!(
            matches!(leaf_at(&a, some[0]), Leaf::Name(_)),
            "Some is a NAME leaf"
        );
        let ok_ctor = kids(&a, some[1]);
        assert_eq!(
            a.as_name(ok_ctor[0]),
            Some("Ok"),
            "the outcome view is the Ok ctor"
        );
        assert!(
            matches!(leaf_at(&a, ok_ctor[0]), Leaf::Name(_)),
            "outcome ctor head is a NAME leaf (value-decode reads a case by name-head)"
        );
        let inline_ctor = kids(&a, ok_ctor[1]);
        assert_eq!(
            a.as_name(inline_ctor[0]),
            Some("Inline"),
            "Ok's payload is the Inline ReplyPayload ctor"
        );
        assert!(
            matches!(leaf_at(&a, inline_ctor[0]), Leaf::Name(_)),
            "Inline head is a NAME leaf"
        );
        assert_eq!(
            leaf_at(&a, inline_ctor[1]),
            &Leaf::Bytes(b"reply-ok".to_vec().into()),
            "Inline carries the response bytes"
        );

        // (Some (Ok (Blob <32-hash-bytes>))) — a LARGE response replied as a blob-ref, NOT inlined. Pin that
        // the Blob arm carries the 32 hash bytes under its own NAME head so the discriminant survives.
        let hash = [7u8; 32];
        let ok_blob = Val::Option(Some(Box::new(Val::Variant(
            "Ok".into(),
            Some(Box::new(Val::Variant(
                "Blob".into(),
                Some(Box::new(bytes(&hash))),
            ))),
        ))));
        let a = decode(&val_to_ast(&ok_blob).expect("val_to_ast"));
        let blob_ctor = kids(&a, kids(&a, a.root)[1]);
        assert_eq!(a.as_name(blob_ctor[0]), Some("Ok"), "the Ok ctor");
        let blob_inner = kids(&a, blob_ctor[1]);
        assert_eq!(
            a.as_name(blob_inner[0]),
            Some("Blob"),
            "Ok's payload is the Blob ReplyPayload ctor (blob-ref survives)"
        );
        assert_eq!(
            leaf_at(&a, blob_inner[1]),
            &Leaf::Bytes(hash.to_vec().into()),
            "Blob carries the 32 hash bytes"
        );

        // (Some (Err (record (= message <bytes>) (= retryable <bool>)))) — a failed reply; the Err arm carries
        // a record with the message + the typed retryability (retryable Bool: true=Retryable, false=Permanent).
        let err = Val::Option(Some(Box::new(Val::Variant(
            "Err".into(),
            Some(Box::new(Val::Record(vec![
                ("message".into(), bytes(b"boom")),
                ("retryable".into(), Val::Bool(false)),
            ]))),
        ))));
        let a = decode(&val_to_ast(&err).expect("val_to_ast"));
        let err_ctor = kids(&a, kids(&a, a.root)[1]);
        assert_eq!(a.as_name(err_ctor[0]), Some("Err"), "the Err ctor");
        let err_rec = kids(&a, err_ctor[1]);
        assert_eq!(
            a.as_name(err_rec[0]),
            Some("record"),
            "Err carries a record"
        );
        let names: Vec<String> = err_rec[1..]
            .iter()
            .map(|id| {
                let f = kids(&a, *id);
                assert_eq!(a.as_name(f[0]), Some("="), "field head is `=`");
                a.as_name(f[1]).expect("field name").to_string()
            })
            .collect();
        assert_eq!(
            names,
            vec!["message", "retryable"],
            "Err record fields sorted by name"
        );

        // (Some (TimedOut unit)) — a timeout (caller-side only; the kernel injects it, a handler never replies it).
        let timed_out = Val::Option(Some(Box::new(Val::Variant("TimedOut".into(), None))));
        let a = decode(&val_to_ast(&timed_out).expect("val_to_ast"));
        let to_ctor = kids(&a, kids(&a, a.root)[1]);
        assert_eq!(
            a.as_name(to_ctor[0]),
            Some("TimedOut"),
            "the TimedOut ctor (nullary)"
        );

        // (None unit) — a non-effect-result event (Inbound/timer) carries no outcome.
        let none = Val::Option(None);
        let a = decode(&val_to_ast(&none).expect("val_to_ast"));
        assert_eq!(
            a.as_name(kids(&a, a.root)[0]),
            Some("None"),
            "absent outcome is (None unit)"
        );
    }

    // decode_reply_outcome is the DUAL of the handler-reply encode: it recovers a kernel EffectOutcome from
    // the Ok/Err-subset value-form pinned above, and is fail-closed on any malformed / non-reply shape (the
    // host maps its Err → a permanent EffectOutcome::Err). Round-trips the Ok + both Err retryabilities, and
    // pins the fail-closed rejections (TimedOut is caller-side only; unknown ctor; missing/extra/duplicate
    // Err field; non-utf-8 message). This + the encode-side wasm_host test lock the codec pair end to end.
    #[test]
    fn decode_reply_outcome_round_trips_the_ok_err_subset_and_is_fail_closed() {
        let bytes = |s: &[u8]| Val::List(s.iter().copied().map(Val::U8).collect());
        // The reply payload is the BARE outcome ctor (no Option wrapper — that's the caller-side Event child).
        let wire = |v: &Val| val_to_ast(v).expect("reply marshals");
        // The Ok payload is DISCRIMINATED (Inline | Blob) so a blob-ref reply survives (operator ruling).
        let ok_inline = |b: &[u8]| {
            Val::Variant(
                "Ok".into(),
                Some(Box::new(Val::Variant(
                    "Inline".into(),
                    Some(Box::new(bytes(b))),
                ))),
            )
        };
        let ok_blob = |h: &[u8]| {
            Val::Variant(
                "Ok".into(),
                Some(Box::new(Val::Variant(
                    "Blob".into(),
                    Some(Box::new(bytes(h))),
                ))),
            )
        };
        let err = |msg: &[u8], retryable: bool| {
            Val::Variant(
                "Err".into(),
                Some(Box::new(Val::Record(vec![
                    ("message".into(), bytes(msg)),
                    ("retryable".into(), Val::Bool(retryable)),
                ]))),
            )
        };

        // Ok(Inline bytes) → EffectOutcome::Ok(Some(Payload::Inline(bytes))).
        assert_eq!(
            decode_reply_outcome(&wire(&ok_inline(b"response-bytes"))).expect("Ok(Inline) decodes"),
            EffectOutcome::Ok(Some(Payload::Inline(b"response-bytes".to_vec().into()))),
        );
        // Ok(Blob <32 hash bytes>) → EffectOutcome::Ok(Some(Payload::Blob(hash))) — the blob-ref survives.
        let hash = [9u8; 32];
        assert_eq!(
            decode_reply_outcome(&wire(&ok_blob(&hash))).expect("Ok(Blob) decodes"),
            EffectOutcome::Ok(Some(Payload::Blob(crate::hash::Hash::from_bytes(hash)))),
        );
        // A Blob whose payload is NOT 32 bytes is fail-closed (a hash is exactly 32 bytes).
        assert!(decode_reply_outcome(&wire(&ok_blob(b"too-short"))).is_err());
        // An unknown Ok payload head (neither Inline nor Blob) is fail-closed.
        assert!(decode_reply_outcome(&wire(&Val::Variant(
            "Ok".into(),
            Some(Box::new(Val::Variant(
                "Raw".into(),
                Some(Box::new(bytes(b"x")))
            )))
        )))
        .is_err());
        // Err Permanent (retryable=false) and Err Retryable (retryable=true) recover the TYPED retryability.
        assert_eq!(
            decode_reply_outcome(&wire(&err(b"boom", false))).expect("Err decodes"),
            EffectOutcome::err("boom"),
        );
        assert_eq!(
            decode_reply_outcome(&wire(&err(b"throttled", true))).expect("Err decodes"),
            EffectOutcome::err_retryable("throttled"),
        );

        // Fail-closed: a handler never replies TimedOut (kernel-injected) — decoding it is an error.
        assert!(decode_reply_outcome(&wire(&Val::Variant("TimedOut".into(), None))).is_err());
        // An unknown ctor head is rejected.
        assert!(decode_reply_outcome(&wire(&Val::Variant(
            "Deferred".into(),
            Some(Box::new(bytes(b"x")))
        )))
        .is_err());
        // A missing Err field (only message) is rejected.
        let missing = Val::Variant(
            "Err".into(),
            Some(Box::new(Val::Record(vec![("message".into(), bytes(b"m"))]))),
        );
        assert!(decode_reply_outcome(&wire(&missing)).is_err());
        // An EXTRA Err field beyond {message, retryable} is rejected.
        let extra = Val::Variant(
            "Err".into(),
            Some(Box::new(Val::Record(vec![
                ("message".into(), bytes(b"m")),
                ("retryable".into(), Val::Bool(true)),
                ("code".into(), Val::U8(7)),
            ]))),
        );
        assert!(decode_reply_outcome(&wire(&extra)).is_err());
        // A non-utf-8 message is rejected (EffectOutcome.message is a String).
        assert!(decode_reply_outcome(&wire(&err(&[0xff, 0xfe], false))).is_err());
        // Undecodable bytes are rejected (not a panic).
        assert!(decode_reply_outcome(b"\xff\x00not-ast").is_err());
    }

    // encode_reply_outcome is the exact INVERSE of decode_reply_outcome: encode-then-decode is the identity
    // over the Ok/Err subset a handler can reply (Ok Inline / Ok Blob / Err both retryabilities), and the two
    // never drift because encode reuses the single outcome-view builder. TimedOut/Deferred are not
    // handler-repliable → encode errors rather than emitting a decodable-but-illegal reply.
    #[test]
    fn encode_reply_outcome_is_the_inverse_of_decode() {
        use crate::hash::Hash;
        let round_trips = |o: EffectOutcome| {
            let bytes = encode_reply_outcome(&o).expect("encodes");
            assert_eq!(
                decode_reply_outcome(&bytes).expect("decodes"),
                o,
                "encode∘decode = id"
            );
        };
        round_trips(EffectOutcome::Ok(Some(Payload::Inline(
            b"resp".to_vec().into(),
        ))));
        round_trips(EffectOutcome::Ok(Some(Payload::Blob(Hash::from_bytes(
            [5u8; 32],
        )))));
        round_trips(EffectOutcome::err("boom"));
        round_trips(EffectOutcome::err_retryable("throttled"));
        // Ok(None) encodes as Ok(Inline []) → decodes to Ok(Some(Inline empty)) (a payload-less success is a
        // zero-length inline reply); assert the concrete decoded shape.
        let empty = encode_reply_outcome(&EffectOutcome::Ok(None)).expect("encodes");
        assert_eq!(
            decode_reply_outcome(&empty).expect("decodes"),
            EffectOutcome::Ok(Some(Payload::Inline(Vec::new().into()))),
        );
        // Not handler-repliable → encode errors (never emits a decodable-but-illegal TimedOut/Deferred reply).
        assert!(encode_reply_outcome(&EffectOutcome::TimedOut).is_err());
        assert!(encode_reply_outcome(&EffectOutcome::Deferred).is_err());
    }

    // The §6 CHILD-COMPLETED value-form round-trips (encode∘decode = id) for BOTH close-classes: a Success
    // with an Inline OR Blob payload (blob-ref survives, reusing the reply ReplyPayload discriminant) and a
    // Failure(reason). Fail-closed on undecodable bytes + an unknown outcome ctor. One value-form covers
    // self-close (Success|Failure) AND terminate (Failure(reason)) — the CloseOutcome discriminates the cause.
    #[test]
    fn child_completed_value_form_round_trips_both_close_classes() {
        use crate::hash::Hash;
        let child = Hash::of(b"child-session");
        let round_trips = |o: CloseOutcome| {
            let bytes = encode_child_completed(&child, &o);
            assert_eq!(
                decode_child_completed(&bytes).expect("decodes"),
                (child, o),
                "encode∘decode = id (child + outcome)"
            );
        };
        round_trips(CloseOutcome::Success(Payload::Inline(
            b"result".to_vec().into(),
        )));
        round_trips(CloseOutcome::Success(Payload::Blob(Hash::of(
            b"a-big-result-blob",
        ))));
        round_trips(CloseOutcome::Failure("goal unreachable".to_string()));

        // Fail-closed: undecodable bytes.
        assert!(decode_child_completed(b"\xff\x00not-ast").is_err());
        // An unknown outcome ctor (neither Success nor Failure) is rejected.
        let bad = {
            let bytes = |b: &[u8]| Val::List(b.iter().copied().map(Val::U8).collect());
            let rec = Val::Record(vec![
                ("child".into(), bytes(child.as_bytes())),
                (
                    "outcome".into(),
                    Val::Variant("Weird".into(), Some(Box::new(bytes(b"x")))),
                ),
            ]);
            val_to_ast(&rec).unwrap()
        };
        assert!(decode_child_completed(&bad).is_err());
    }

    // The bare CloseOutcome value-form (the payload a `.cdz` guest emits on a `control/close` self-close)
    // round-trips (encode∘decode = id) for all three shapes — Success(Inline), Success(Blob) (blob-ref
    // success survives), Failure(reason) — and is the SAME node child-completed nests, so a guest's
    // self-close outcome and a supervisor's child-completed outcome are byte-identical. Fail-closed on
    // undecodable bytes + an unknown ctor head.
    #[test]
    fn close_outcome_value_form_round_trips_and_matches_child_completed_nested_form() {
        use crate::hash::Hash;
        let round_trips = |o: CloseOutcome| {
            let bytes = encode_close_outcome(&o);
            assert_eq!(
                decode_close_outcome(&bytes).expect("decodes"),
                o.clone(),
                "encode∘decode = id"
            );
            // The self-close encoding is byte-identical to child-completed's nested `outcome` field: the
            // child-completed record embeds exactly `encode_close_outcome(&o)` under `outcome`.
            let child = Hash::of(b"c");
            let (decoded_child, decoded_outcome) =
                decode_child_completed(&encode_child_completed(&child, &o)).expect("decodes");
            assert_eq!((decoded_child, decoded_outcome), (child, o));
        };
        round_trips(CloseOutcome::Success(Payload::Inline(
            b"done".to_vec().into(),
        )));
        round_trips(CloseOutcome::Success(Payload::Blob(Hash::of(b"blob"))));
        round_trips(CloseOutcome::Failure("gave up".to_string()));

        // Fail-closed: undecodable bytes, and an unknown outcome ctor head.
        assert!(decode_close_outcome(b"\xff\x00nope").is_err());
        let bad = val_to_ast(&Val::Variant("Weird".into(), Some(Box::new(Val::U8(1))))).unwrap();
        assert!(decode_close_outcome(&bad).is_err());
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
            "world",
            "reducer",
            "cadenza:agent-kernel/fold",
            "apply",
            "cadenza:agent-kernel/kv",
            "get",
            "put",
            "delete",
            "prefix-scan",
            "event",
            "key",
            "value",
            "prefix",
            "u8",
            "bool",
        ] {
            assert!(
                names.iter().any(|n| n == expect),
                "missing NAME atom {expect}"
            );
        }
        // build_type-form descriptor heads are STR atoms: list<u8> -> ("list" (u8)); option -> ("option" ..);
        // unit -> ("unit"); tuple -> ("tuple" ..). Their presence proves the param/result descriptors are
        // the shared build_type form.
        for expect in ["list", "option", "unit", "tuple"] {
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
        for expect in [
            "world",
            "reducer",
            "cadenza:agent-kernel/fold",
            "apply",
            "event",
        ] {
            assert!(
                names.iter().any(|n| n == expect),
                "missing NAME atom {expect}"
            );
        }
        // pure = NO kv import (that is the whole point — no host-fused import for the intermediate).
        assert!(
            !names.iter().any(|n| n == "cadenza:agent-kernel/kv"),
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
        // FIELD ORDER IS NOT IDENTITY — a record's schema-hash is NAME-SORTED (concierge ruling 2026-08-13),
        // so the SAME field-set in a DIFFERENT declaration order hashes IDENTICALLY. This is the invariant that
        // lets the three producers (reflected `build_type`, kernel built-in decls, rcdzc's name-sorted-BTreeMap
        // userspace) content-address the same record shape. Without the sort in `build_type`'s record arm these
        // two would hash differently (a silent cross-producer divergence).
        let ab = schema_hash(&param_type(&probe_component(
            r#"(record (field "a" u32) (field "b" string))"#,
        )))
        .expect("record a,b schema");
        let ba = schema_hash(&param_type(&probe_component(
            r#"(record (field "b" string) (field "a" u32))"#,
        )))
        .expect("record b,a schema");
        assert_eq!(
            ab, ba,
            "a record's schema-hash is field-name-sorted: {{a,b}} and {{b,a}} are the same shape → same hash"
        );
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
    fn builtin_effect_schema_hashes_are_stable_and_pairwise_distinct() {
        // Effect-identity removal (operator 2026-08-12): every built-in effect has a schema, hashed by the
        // SAME path as a userspace effect, and that hash IS its identity (what the router + Cedar authz key
        // on). Pin the two properties the identity contract needs:
        //   (1) STABILITY — the hash is a deterministic function of the schema, so recomputing yields the
        //       SAME id (a router built at startup and a replay both see the same identity). This is what
        //       lets a grant bind to a specific built-in's schema-hash durably.
        //   (2) PAIRWISE-DISTINCTNESS — the six built-ins have distinct schemas (different name AND op
        //       shape), so no two built-ins collide onto one identity (a collision would let an http grant
        //       authorize a shell request, etc.).
        use crate::effect::EffectKind;
        let kinds = [
            EffectKind::Shell,
            EffectKind::Http,
            EffectKind::Model,
            EffectKind::Now,
            EffectKind::Timer,
            EffectKind::Emit,
        ];
        // (1) stable: recompute each and confirm it equals the first computation.
        for k in &kinds {
            assert_eq!(
                builtin_effect_schema_hash(k),
                builtin_effect_schema_hash(k),
                "built-in {k:?} schema-hash must be deterministic"
            );
        }
        // (2) pairwise-distinct: all six hashes differ.
        let hashes: Vec<_> = kinds.iter().map(builtin_effect_schema_hash).collect();
        for i in 0..hashes.len() {
            for j in (i + 1)..hashes.len() {
                assert_ne!(
                    hashes[i], hashes[j],
                    "built-in {:?} and {:?} must have distinct schema-hashes",
                    kinds[i], kinds[j]
                );
            }
        }
    }

    #[test]
    fn a_userspace_effect_declared_same_shape_as_a_builtin_hashes_identically() {
        // CONTENT-ADDRESS GATE (schema-hash effect-identity phase-1a, the KERNEL side of the byte-identity
        // gate co-owned with v-rust-backend): a schema-hash must CONTENT-ADDRESS — an effect declared with
        // the SAME shape by a DIFFERENT producer resolves to the SAME identity. Phase-1a's router/authz key
        // on the schema-hash, so a userspace reducer performing an effect whose declared shape MATCHES a
        // built-in MUST hash to the built-in's identity — else the same shape from two producers would route/
        // authorize differently (the divergence the whole removal exists to prevent).
        //
        // Here the KERNEL side: rebuild `emit.send`'s descriptor exactly as a USERSPACE producer would — via
        // `effect_schema_hash_from_nodes` (the &Type-free core), building the op-sig nodes by hand with the
        // shared WIT builders — and assert it EQUALS `builtin_effect_schema_hash(EffectKind::Emit)`. This is
        // the assertion rcdzc's byte-identity gate mirrors on its side (its copied builders + `ty_to_wit_desc`
        // must produce the identical descriptor bytes for the same Cadenza `Ty` shape). emit.send is
        // `(effect "emit" (op "send" (func (param "payload" (list u8)) (result unit))))`.
        use crate::effect::EffectKind;
        let userspace_emit = {
            let mut b = Builder::new();
            // payload: list<u8> — a Cadenza `Bytes` maps to `list<u8>` at the boundary (NOT a "bytes" prim);
            // this exercises the exact Ty::Bytes -> wit_type_list(wit_type_prim("u8")) mapping rcdzc must use.
            let payload = {
                let u8_ty = b.wit_type_prim("u8");
                b.wit_type_list(u8_ty)
            };
            let unit = b.wit_type_unit();
            let sig = b.wit_func_sig(&[("payload", payload)], unit);
            effect_schema_hash_from_nodes(b, "emit", &[("send", sig)])
        };
        assert_eq!(
            userspace_emit,
            builtin_effect_schema_hash(&EffectKind::Emit),
            "a userspace effect declared with emit.send's shape must hash to the built-in emit identity \
             (content-address: same shape -> same identity across producers)"
        );

        // A MULTI-FIELD RECORD op — the field-order trap. Build `model.invoke`'s request record shape as a
        // userspace producer would, with the fields in a DIFFERENT (non-name-sorted) declaration order than
        // the built-in decl lists them, and assert it STILL hashes to the built-in model identity. This pins
        // the name-sorted canonical order (0cbebf470): `wit_type_record` sorts by field name, so the caller's
        // field order does not affect identity — the exact cross-producer property (rcdzc's `Ty::Record` is a
        // name-sorted BTreeMap, so it can only emit sorted; the built-in decl and this reconstruction must
        // match it regardless of the order the fields are written).
        let userspace_model = {
            let mut b = Builder::new();
            // model.invoke(request: {model:string, messages:list<unit>, tools:list<unit>, max-tokens:option<u64>})
            //   -> {stop-reason:string, content:list<unit>}. Fields written in a SCRAMBLED order on purpose.
            let request = {
                let messages = {
                    let e = b.wit_type_unit();
                    b.wit_type_list(e)
                };
                let max_tokens = {
                    let u64_ty = b.wit_type_prim("u64");
                    b.wit_type_option(u64_ty)
                };
                let model = b.wit_type_prim("string");
                let tools = {
                    let e = b.wit_type_unit();
                    b.wit_type_list(e)
                };
                // SCRAMBLED order (max-tokens, tools, model, messages) — name-sort must normalize it.
                wit_type_record(
                    &mut b,
                    &[
                        ("max-tokens", max_tokens),
                        ("tools", tools),
                        ("model", model),
                        ("messages", messages),
                    ],
                )
            };
            let response = {
                let content = {
                    let e = b.wit_type_unit();
                    b.wit_type_list(e)
                };
                let stop_reason = b.wit_type_prim("string");
                // SCRAMBLED (content, stop-reason) vs the decl's (stop-reason, content).
                wit_type_record(
                    &mut b,
                    &[("content", content), ("stop-reason", stop_reason)],
                )
            };
            let sig = b.wit_func_sig(&[("request", request)], response);
            effect_schema_hash_from_nodes(b, "model", &[("invoke", sig)])
        };
        assert_eq!(
            userspace_model,
            builtin_effect_schema_hash(&EffectKind::Model),
            "a model-shaped userspace effect hashes to the built-in model identity regardless of the field \
             WRITING order — the schema descriptor's record fields are name-sorted (content-address holds)"
        );
    }

    #[test]
    fn family_effect_schema_hashes_are_stable_declared_set_pairwise_distinct_and_distinct_from_builtins(
    ) {
        use crate::effect::{effect_ct, EffectKind};
        // The 22 well-known non-EffectKind families that have a DECLARED schema (target-OUT) — every
        // well-known non-kind family now carries one (control/signature was the last to land).
        let families = [
            effect_ct::FS_READ,
            effect_ct::FS_WRITE,
            effect_ct::FS_GLOB,
            effect_ct::BLOB_PUT,
            effect_ct::BLOB_GET,
            effect_ct::METRIC_PUBLISH,
            effect_ct::WS_SEND,
            effect_ct::WS_DIAL,
            effect_ct::LIFECYCLE_SPAWN,
            effect_ct::LIFECYCLE_SUSPEND,
            effect_ct::LIFECYCLE_RESUME,
            effect_ct::LIFECYCLE_TERMINATE,
            effect_ct::CAPABILITIES,
            effect_ct::SUMMARY,
            effect_ct::SIGNATURE,
            effect_ct::STORE_SET,
            effect_ct::STORE_RESOLVE,
            effect_ct::STORE_ADD,
            effect_ct::STORE_REMOVE,
            effect_ct::STORE_RESOLVE_ALL,
            effect_ct::EFFECT_REPLY,
            effect_ct::CLOSE,
        ];
        // (1) each is declared (Some), stable, and the memo agrees with the pure recompute.
        for f in &families {
            let h = family_effect_schema_hash(f).expect("declared family has a schema-hash");
            assert_eq!(
                h,
                family_effect_schema_hash(f).unwrap(),
                "family {f} deterministic"
            );
            assert_eq!(
                Some(h),
                family_effect_schema_hash_memo(f),
                "memo agrees for {f}"
            );
        }
        // (2) a register-by-string EXTENSION family is None (every well-known non-kind family is now
        // declared; only an unknown extension has no schema). An exact-match table, not a prefix match — a
        // "store/whatever" extension the kernel doesn't know is still None.
        assert_eq!(family_effect_schema_hash("custom/metrics"), None);
        assert_eq!(family_effect_schema_hash("store/whatever"), None);
        assert_eq!(family_effect_schema_hash_memo("custom/metrics"), None);
        // (3) THE IDENTITY GUARD: all 6 built-ins + 22 families = 28 PAIRWISE-DISTINCT hashes. This proves
        // target-OUT is safe — the unit->unit families (ws/dial, lifecycle/suspend|resume|terminate) do NOT
        // collide with each other despite identical signatures, because the effect NAME + OP NAME are hashed.
        let builtins = [
            EffectKind::Shell,
            EffectKind::Http,
            EffectKind::Model,
            EffectKind::Now,
            EffectKind::Timer,
            EffectKind::Emit,
        ];
        let mut all: Vec<crate::hash::Hash> =
            builtins.iter().map(builtin_effect_schema_hash).collect();
        all.extend(
            families
                .iter()
                .map(|f| family_effect_schema_hash(f).unwrap()),
        );
        assert_eq!(all.len(), 28);
        for i in 0..all.len() {
            for j in (i + 1)..all.len() {
                assert_ne!(
                    all[i], all[j],
                    "schema-hash collision at indices {i} and {j}"
                );
            }
        }
    }

    #[test]
    fn effect_family_schema_hash_is_uniform_by_family_string_over_builtins_and_non_kind_families() {
        use crate::effect::{effect_ct, EffectKind};
        // The uniform host-callable seam: keyed by FAMILY STRING, it returns the SAME hash whether the family
        // is an EffectKind built-in or a non-kind family — so a host executor never branches on EffectKind.
        // (1) each of the 6 built-in families resolves, and AGREES with builtin_effect_schema_hash(kind).
        for k in [
            EffectKind::Shell,
            EffectKind::Http,
            EffectKind::Model,
            EffectKind::Now,
            EffectKind::Timer,
            EffectKind::Emit,
        ] {
            assert_eq!(
                effect_family_schema_hash(k.family()),
                Some(builtin_effect_schema_hash(&k)),
                "built-in family {} resolves to its built-in hash",
                k.family()
            );
        }
        // (2) a non-kind family resolves, and AGREES with family_effect_schema_hash.
        for fam in [
            effect_ct::FS_READ,
            effect_ct::STORE_SET,
            effect_ct::EFFECT_REPLY,
            effect_ct::CLOSE,
            effect_ct::SIGNATURE,
        ] {
            assert_eq!(
                effect_family_schema_hash(fam),
                family_effect_schema_hash(fam),
                "non-kind family {fam} resolves via the family path"
            );
            assert!(effect_family_schema_hash(fam).is_some());
        }
        // (3) EVERY served family — all 6 built-ins + all declared non-kind — is Some (no host executor is
        // ever left without a served hash). An extension family unknown to the kernel is None.
        for fam in [
            effect_ct::SHELL,
            effect_ct::HTTP,
            effect_ct::MODEL,
            effect_ct::NOW,
            effect_ct::TIMER,
            effect_ct::EMIT,
            effect_ct::FS_READ,
            effect_ct::FS_WRITE,
            effect_ct::FS_GLOB,
            effect_ct::BLOB_PUT,
            effect_ct::BLOB_GET,
            effect_ct::METRIC_PUBLISH,
            effect_ct::WS_SEND,
            effect_ct::WS_DIAL,
            effect_ct::LIFECYCLE_SPAWN,
            effect_ct::LIFECYCLE_SUSPEND,
            effect_ct::LIFECYCLE_RESUME,
            effect_ct::LIFECYCLE_TERMINATE,
            effect_ct::CAPABILITIES,
            effect_ct::SUMMARY,
            effect_ct::SIGNATURE,
            effect_ct::STORE_SET,
            effect_ct::STORE_RESOLVE,
            effect_ct::STORE_ADD,
            effect_ct::STORE_REMOVE,
            effect_ct::STORE_RESOLVE_ALL,
            effect_ct::EFFECT_REPLY,
            effect_ct::CLOSE,
        ] {
            assert!(
                effect_family_schema_hash(fam).is_some(),
                "served family {fam} must have a schema-hash"
            );
        }
        assert_eq!(effect_family_schema_hash("custom/extension"), None);
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
    fn event_document_is_the_canonical_event_value_form() {
        // build_event_document now builds the Event as a Val::Record + val_to_ast, so its bytes ARE the
        // canonical value-form the guest's value-decode reconstructs the Event from (the ad-hoc named form
        // is gone). Pin it against val_to_ast of the manually-built Event Val — record { content-type:
        // { family, version }, payload: option<list<u8>>, resumes: option<list<u8>> }.
        let event_val =
            |family: &str, version: u32, payload: Option<&[u8]>, resumes: Option<&[u8]>| {
                let opt = |v: Option<&[u8]>| {
                    Val::Option(
                        v.map(|b| Box::new(Val::List(b.iter().copied().map(Val::U8).collect()))),
                    )
                };
                Val::Record(vec![
                    (
                        "content-type".into(),
                        Val::Record(vec![
                            ("family".into(), Val::String(family.to_string())),
                            ("version".into(), Val::U32(version)),
                        ]),
                    ),
                    ("payload".into(), opt(payload)),
                    ("resumes".into(), opt(resumes)),
                    // These non-EffectResult docs carry no outcome — the field is present as (None unit).
                    ("outcome".into(), Val::Option(None)),
                    // Non-ChildCompleted docs carry no child-completed record — present as (None unit).
                    ("child-completed".into(), Val::Option(None)),
                ])
            };
        assert_eq!(
            build_event_document(
                ContentTypeRef {
                    family: "message",
                    version: 1
                },
                Some(b"hi"),
                None,
                None,
                None
            ),
            val_to_ast(&event_val("message", 1, Some(b"hi"), None)).unwrap(),
            "build_event_document == val_to_ast of the Event Val (canonical value-form)"
        );
        // An intentionally-EMPTY payload (Some []) is distinct from an absent one (None) — the option
        // present/absent carries it (Some [] vs None), so the two docs differ.
        let empty = build_event_document(
            ContentTypeRef {
                family: "m",
                version: 1,
            },
            Some(b""),
            None,
            None,
            None,
        );
        let absent = build_event_document(
            ContentTypeRef {
                family: "m",
                version: 1,
            },
            None,
            None,
            None,
            None,
        );
        assert_ne!(
            empty, absent,
            "an empty payload (Some []) differs from absent (None)"
        );
        assert_eq!(
            empty,
            val_to_ast(&event_val("m", 1, Some(b""), None)).unwrap()
        );
    }

    // Build a canonical effect-list the way a guest's value-encode emits it: `(list <record>…)` of flat
    // effect-request records { kind: string, target: list<u8>, payload: option<list<u8>>, correlation:
    // option<list<u8>> } — via val_to_ast (the shared canonical encoder), so the test bytes match what the
    // guest produces by construction (sorted (= name value) fields, capital Some/None, Bytes leaves).
    fn effect_req_val(
        kind: &str,
        target: &[u8],
        payload: Option<&[u8]>,
        corr: Option<&[u8]>,
    ) -> Val {
        let bytes = |b: &[u8]| Val::List(b.iter().copied().map(Val::U8).collect());
        let opt = |v: Option<&[u8]>| Val::Option(v.map(|x| Box::new(bytes(x))));
        Val::Record(vec![
            ("kind".into(), Val::String(kind.to_string())),
            ("target".into(), bytes(target)),
            ("payload".into(), opt(payload)),
            ("correlation".into(), opt(corr)),
        ])
    }
    fn effect_list_bytes(reqs: Vec<Val>) -> Vec<u8> {
        val_to_ast(&Val::List(reqs)).expect("effect-list is marshallable")
    }

    #[test]
    fn effect_list_parses_back_into_effects_with_kind_target_payload_token() {
        // parse_effect_list is the dual of build_event_document: the guest value-encodes its returned
        // list<effect-request> as `(list <record>…)`; decode it back, asserting kind/target/payload/token
        // per effect (a payload-and-token http effect + a bare now).
        let bytes = effect_list_bytes(vec![
            effect_req_val("http", b"https://ok/x", Some(b"body"), Some(b"cont-1")),
            effect_req_val("now", b"", None, None),
        ]);
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
        // Well-formed but wrong head (not a `list`) → TypeMismatch.
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
    fn effect_list_structured_payload_arm_reencodes_the_value_form_bytes() {
        // b1 self-describing payload dispatch: an opaque payload is a bare bytes-leaf (covered by the
        // sibling test's `body` payload — unchanged), and a STRUCTURED payload is a name-headed
        // `(Structured <value>)` compound. parse_effect_list must produce the effect's Inline payload =
        // the CANONICAL value-form bytes of that inner value — exactly the bytes a schema-typed host
        // decoder (decode_model_request) reads. Re-encoded standalone, byte-identical to `val_to_ast` of
        // the same value (the form pinned by `val_to_ast_pins_the_b1_model_request_value_form`).
        let model_request = Val::Record(vec![
            ("model".into(), Val::String("claude".into())),
            (
                "messages".into(),
                Val::List(vec![Val::Record(vec![
                    ("role".into(), Val::String("user".into())),
                    (
                        "content".into(),
                        Val::List(vec![Val::Variant(
                            "Text".into(),
                            Some(Box::new(Val::String("hi".into()))),
                        )]),
                    ),
                ])]),
            ),
            ("tools".into(), Val::List(vec![])),
            (
                "max-tokens".into(),
                Val::Option(Some(Box::new(Val::U64(1024)))),
            ),
        ]);
        // The canonical standalone bytes of the ModelRequest value — what the Structured arm must yield.
        let want_inline = val_to_ast(&model_request).expect("model-request marshals");

        // A model effect whose payload is the Structured value-form arm carrying that ModelRequest.
        let structured_effect = Val::Record(vec![
            ("kind".into(), Val::String("model".into())),
            (
                "target".into(),
                Val::List(b"llm".iter().copied().map(Val::U8).collect()),
            ),
            (
                "payload".into(),
                Val::Option(Some(Box::new(Val::Variant(
                    "Structured".into(),
                    Some(Box::new(model_request.clone())),
                )))),
            ),
            ("correlation".into(), Val::Option(None)),
        ]);
        let bytes = effect_list_bytes(vec![structured_effect]);
        let effects = parse_effect_list(&bytes).expect("structured payload parses");
        assert_eq!(effects.len(), 1);
        assert_eq!(effects[0].request.content_type.family, "model");
        match &effects[0].request.payload {
            Some(Payload::Inline(p)) => assert_eq!(
                p.as_ref(),
                want_inline.as_slice(),
                "Structured arm re-encodes the inner value-form standalone"
            ),
            other => panic!("expected an Inline value-form payload, got {other:?}"),
        }
    }

    #[test]
    fn effect_list_raw_payload_arm_unwraps_to_the_opaque_bytes() {
        // A reducer whose payload field is a TWO-arm `Raw(Bytes) | Structured(<value>)` sum (it emits BOTH
        // opaque and structured payloads — the agent-loop) tags an opaque payload as `(Raw <bytes>)`, not a
        // bare bytes-leaf, because the two arms defeat newtype erasure so the tag survives. parse_effect_list
        // must UNWRAP `(Raw <bytes>)` to the same inline bytes a bare-leaf opaque payload yields — so a tool
        // effect from the two-arm agent-loop reads identically to an opaque payload from a bare reducer.
        let raw_effect = Val::Record(vec![
            ("kind".into(), Val::String("tool".into())),
            (
                "target".into(),
                Val::List(b"shell".iter().copied().map(Val::U8).collect()),
            ),
            (
                "payload".into(),
                Val::Option(Some(Box::new(Val::Variant(
                    "Raw".into(),
                    Some(Box::new(Val::List(
                        b"cargo test".iter().copied().map(Val::U8).collect(),
                    ))),
                )))),
            ),
            ("correlation".into(), Val::Option(None)),
        ]);
        let effects = parse_effect_list(&effect_list_bytes(vec![raw_effect])).expect("Raw parses");
        assert_eq!(effects.len(), 1);
        assert_eq!(effects[0].request.content_type.family, "tool");
        assert!(
            matches!(&effects[0].request.payload, Some(Payload::Inline(p)) if p.as_ref() == b"cargo test"),
            "the Raw arm unwraps to the opaque bytes verbatim"
        );
    }

    #[test]
    fn effect_list_round_trips_a_register_by_string_family_with_no_builtin_kind() {
        // GUARDRAIL (DESIGN-binary-ast-abi §2 / the B2-revert lesson): the effect-request `kind` crosses
        // the bytes boundary as a family STRING, NEVER a closed enum discriminant — so a REGISTER-BY-STRING
        // userspace family with NO matching `EffectKind` variant (seq-39) round-trips intact. This is the
        // exact invariant whose violation (a kind-discriminant ABI) caused the B2+B3 revert; pin it in the
        // gate so a future change to the effect codec can't quietly re-introduce a closed-enum kind.
        // A novel family that is NOT one of shell/http/model/now/timer/emit — pure register-by-string.
        let novel = "effect/reply";
        let bytes = effect_list_bytes(vec![effect_req_val(
            novel,
            b"sess-42",
            Some(b"answer"),
            Some(b"tok-9"),
        )]);
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
