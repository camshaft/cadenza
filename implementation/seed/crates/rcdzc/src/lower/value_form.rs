//! `lower::value_form` — runtime/constant VALUE-FORM encoding, split out of `lower.rs`. Builds the
//! byte templates + runtime leaf holes for a compound's `encode()` (`RuntimeLeaf`/`ValueFormTemplate`/
//! `SumFormTemplate`), the shape descriptors (`sum`/`map`/`set`/`bare`/`value_cmp`), the WIT/effect
//! schema descriptors and `reify_effect_to_tuple`, and the compile-time constant value-AST synthesis
//! (`const_value_ast*`). Behaviour-preserving move: items keep their original visibility (`pub` items
//! stay `pub`, re-exported by `pub use value_form::*` in `lower` so `crate::lower::*` paths are
//! unchanged); private items become `pub(super)` and reach the rest of the tree via `use super::*`.

use super::*;

/// A RUNTIME leaf hole in a value-form byte template: the byte OFFSET in the template where the leaf's
/// runtime value is written, the WALK PATH of `arr-get` indices from the root heap handle to the leaf,
/// and its KIND (how many bytes / which encoding). The template bakes everything static (structure,
/// names, type nodes, kind/len framing); at run time `encode()` walks each hole's path and writes the
/// value. (`DESIGN-value-heap-rcdzc.md` §3a R2 — the runtime compound escape.)
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RuntimeLeaf {
    /// Byte offset in the template where the runtime value is written.
    pub offset: usize,
    /// `arr-get` indices from the root handle down to this leaf (empty = the root is itself the leaf).
    pub path: Vec<u32>,
    /// How the leaf's runtime value fills its hole.
    pub kind: LeafFill,
    /// Whether the walk starts by recovering the SUM PAYLOAD: when this leaf lives inside a sum
    /// variant's payload, the walker first calls `sum-payload(rep)` to get the payload handle, THEN
    /// applies `path`. A single-payload variant leaves `path` empty (the payload handle IS the boxed
    /// leaf); a multi-payload variant's `path` indexes into the payload tuple. `false` for a plain
    /// tuple/record leaf (the walk starts at the root handle directly). Set on the per-variant templates
    /// a [`SumFormTemplate`] holds.
    pub via_sum_payload: bool,
}

/// How a runtime leaf's value fills its template hole.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LeafFill {
    /// A boxed integer: read `get-int` (s64), write 8 big-endian magnitude bytes at `offset` (the
    /// template reserves an 8-byte magnitude with `len=8`; a non-minimal magnitude decodes fine because
    /// `BigInt::from_bytes_be` normalizes leading zeros). A NEGATIVE value also flips the kind byte at
    /// `offset - 2` from `INT_POS_DEC` to `INT_NEG_DEC` and writes the ABSOLUTE magnitude.
    Int,
    /// A boxed boolean: read `get-bool`, write the kind byte at `offset` — `9` (true) or `8` (false).
    Bool,
}

/// The value-form byte TEMPLATE for a runtime compound of type `ty`: the codec bytes with every leaf's
/// value left as a placeholder, plus the [`RuntimeLeaf`] holes to fill at run time. Everything static —
/// the `(: value type)` structure, the `tuple`/`record` heads + field names, the whole TYPE node, and
/// each leaf's kind/len framing — is baked; only the leaf VALUES are holes. `encode()` copies this
/// template into linear memory, walks each hole's heap path, and writes the value (R2). `None` if the
/// type has no value-form surface (a function/type-value). Every leaf is treated as a runtime hole
/// (walked from the handle), so a mixed const/runtime compound needs no special-casing — a constant
/// element still sits boxed on the heap and reads back the same.
pub fn runtime_value_form_template(
    ty: &crate::ty::Ty,
    ncx: &crate::ty::NameCtx,
) -> Option<ValueFormTemplate> {
    let mut b = crate::ast::Builder::new();
    let colon = b.name(":");
    // Build the value AST with PLACEHOLDER leaves, recording each leaf's walk path + kind as we go.
    let mut leaves: Vec<PendingLeaf> = Vec::new();
    let value = template_value_ast(&mut b, ty, &mut Vec::new(), &mut leaves)?;
    let type_ast = type_ast(&mut b, ty, ncx)?;
    let root = b.list(vec![colon, value, type_ast]);
    let arenas = b.finish(root);
    let bytes = crate::codec::encode(&arenas);
    // Locate each placeholder leaf's byte offset in the encoded LEAF POOL (leaves are encoded in order
    // right after the 8-byte header + leaf-count LEB). Walk the pool, tracking offsets; a leaf that was
    // recorded as runtime (by its LeafId) gets its hole offset resolved here.
    let holes = resolve_leaf_offsets(&bytes, &arenas, &leaves)?;
    Some(ValueFormTemplate {
        bytes,
        leaves: holes,
    })
}

/// A compile-time-constant leaf value recovered from an escaping return's core, for [`bake_constant_leaves`].
pub(super) enum ConstLeafVal {
    /// A constant integer — its `IntValue` (sign + big-endian magnitude), ≤ 8 magnitude bytes (the fixed
    /// Int-hole width); a wider magnitude does not fit the hole and is left runtime.
    Int(crate::ast::IntValue),
    /// A constant boolean.
    Bool(bool),
}

/// Follow a runtime leaf's `path` (`arr-get` indices) down the escaping return's core `root` to the leaf's
/// core node, and — when that node is a compile-time constant matching the hole's `kind` — return its value.
/// The walk descends ONLY through literal `Tuple`/`Record` cores (the shapes a value-form template indexes);
/// any other node (a runtime operand, a projection, a call, a `Let`, a non-literal compound) makes the path
/// non-navigable → `None`, so the leaf stays a runtime hole. Record fields are visited in `BTreeMap` (sorted)
/// order, matching the positional `arr-get` index the template records.
pub(super) fn resolve_const_leaf(
    db: &mut Db,
    root: StructId,
    path: &[u32],
    kind: LeafFill,
) -> Option<ConstLeafVal> {
    let mut cur = root;
    for &idx in path {
        cur = match core_of(db, cur) {
            Core::Tuple { elems } => *elems.get(idx as usize)?,
            Core::Record { fields } => *fields.values().nth(idx as usize)?,
            _ => return None,
        };
    }
    match (core_of(db, cur), kind) {
        (Core::ConstInt(v), LeafFill::Int) if v.magnitude.len() <= 8 => Some(ConstLeafVal::Int(v)),
        (Core::ConstBool(x), LeafFill::Bool) => Some(ConstLeafVal::Bool(x)),
        _ => None,
    }
}

/// Write a constant Int leaf into a value-form template's bytes at `offset`, byte-IDENTICALLY to the runtime
/// hole-fill walker (`serialize.rs` `EscapeForm` encode body): 8 big-endian magnitude bytes right-aligned at
/// `offset` (the template reserves an 8-byte, `len=8` magnitude — a non-minimal magnitude decodes fine,
/// `BigInt::from_bytes_be` drops leading zeros), and for a NEGATIVE value the kind byte at `offset - 2` set to
/// `3` (`KIND_INT_NEG_DEC`). A positive value keeps the placeholder's positive kind (untouched).
pub(super) fn write_int_leaf(bytes: &mut [u8], offset: usize, iv: &crate::ast::IntValue) {
    for b in bytes.iter_mut().skip(offset).take(8) {
        *b = 0;
    }
    let start = 8 - iv.magnitude.len();
    for (i, &m) in iv.magnitude.iter().enumerate() {
        bytes[offset + start + i] = m;
    }
    if iv.negative {
        bytes[offset - 2] = 3; // KIND_INT_NEG_DEC
    }
}

/// Write a constant Bool leaf's kind byte at `offset`, byte-identically to the runtime walker (`8` false /
/// `9` true — the walker writes `8 + get-bool`).
pub(super) fn write_bool_leaf(bytes: &mut [u8], offset: usize, x: bool) {
    bytes[offset] = 8 + x as u8;
}

/// §2d STATIC-DATA / pre-encode (Axis 2): given a value-form `template` for an escaping compound return and
/// the return body's core `root`, BAKE every leaf whose value is a compile-time constant directly into the
/// template bytes — the SAME byte write the runtime hole-fill walker performs — and DROP that leaf's runtime
/// hole. A leaf whose core is runtime, or unreachable by a static `Tuple`/`Record` path (a non-literal
/// compound return, a sum-payload leaf), stays a hole and the walker fills it per event as before. When EVERY
/// leaf bakes, the returned template has ZERO holes = fully static: the escape walker copies it to the output
/// with no heap walk and no per-event leaf reads (the per-event value-encode is gone for the constant parts).
/// Byte-IDENTICAL to `template` when nothing bakes, so it is safe to run on every flat-compound escape. The
/// walker consumes `.leaves` unchanged — fewer holes simply means fewer per-event writes.
pub fn bake_constant_leaves(
    db: &mut Db,
    root: StructId,
    template: &ValueFormTemplate,
) -> ValueFormTemplate {
    let mut bytes = template.bytes.clone();
    let mut remaining: Vec<RuntimeLeaf> = Vec::new();
    for leaf in &template.leaves {
        // A sum-payload leaf is reached through the runtime sum rep (`sum-payload`), not a static
        // `Tuple`/`Record` index — leave it runtime this slice.
        if leaf.via_sum_payload {
            remaining.push(leaf.clone());
            continue;
        }
        match resolve_const_leaf(db, root, &leaf.path, leaf.kind) {
            Some(ConstLeafVal::Int(iv)) => write_int_leaf(&mut bytes, leaf.offset, &iv),
            Some(ConstLeafVal::Bool(x)) => write_bool_leaf(&mut bytes, leaf.offset, x),
            None => remaining.push(leaf.clone()),
        }
    }
    ValueFormTemplate {
        bytes,
        leaves: remaining,
    }
}

/// The two STATIC halves of a runtime `Bytes` value form, for the looping `encode()` walker (L2b).
/// The value form of `(: <bytes> Bytes)` is `PREFIX · <LEB len> · <n raw bytes> · SUFFIX`, where ONLY
/// the leaf's length-LEB and payload are runtime — the prefix (header … the `KIND_BYTES` tag) and the
/// suffix (the `Bytes` type-name leaf + the whole struct table + root) are byte-identical regardless of
/// `n` (verified across n = 0 / 3 / 130). So the walker writes `prefix`, then the runtime LEB of
/// `bytes-len`, then copies the bytes, then `suffix` — no fixed-size template. `DESIGN-runtime-bytes-
/// escape-walker.md`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RuntimeBytesForm {
    /// Bytes to write verbatim BEFORE the runtime length+payload — the header through the KIND_BYTES tag.
    pub prefix: Vec<u8>,
    /// Bytes to write verbatim AFTER the runtime payload — the type-name leaf + struct table + root.
    pub suffix: Vec<u8>,
}

/// Compute the [`RuntimeBytesForm`] for `Ty::Bytes` — build the ZERO-length Bytes value form (`…0b 00
/// <suffix>`) and split it at the leaf's length byte: `prefix` = everything up to and INCLUDING the
/// `KIND_BYTES` tag, `suffix` = everything AFTER the `00` length byte. A runtime walker fills the gap
/// with `<LEB n><n bytes>`. `None` if the encoded form does not have the expected `0b 00` shape (a
/// codec change) — the escape then declines rather than emit a wrong walker.
pub fn runtime_bytes_form(db: &mut Db) -> Option<RuntimeBytesForm> {
    runtime_leaf_form(db, false)
}

/// The runtime STRING escape form — `(: "" String)` with an empty `Leaf::Str`, split at the `KIND_STR`
/// tag. A runtime String is a UTF-8 byte-rope leaf (the same heap rep as Bytes — `String.concat` is
/// `bytes-concat`), so it escapes through the SAME looping walker (`emit_runtime_bytes_resource`); only
/// the value-form framing differs (`(: "…" String)` vs `(: b"…" Bytes)`). The walker's `bytes-len`/
/// `bytes-get` read the same leaf either way; the payload bytes ARE the UTF-8 the codec decodes back to a
/// `Leaf::Str`, so `cdz-run` renders `(: "…" String)`.
pub fn runtime_string_form(db: &mut Db) -> Option<RuntimeBytesForm> {
    runtime_leaf_form(db, true)
}

/// Shared builder for the runtime Bytes/String escape form: encode `(: <empty-leaf> <TypeName>)` and split
/// at the leaf's tag so the walker can splice the runtime `LEB(len) · payload` between the static prefix
/// (header … tag) and suffix (the `<TypeName>` + struct framing). `is_string` selects a `Leaf::Str`/
/// `"String"`/`KIND_STR` split vs a `Leaf::Bytes`/`"Bytes"`/`KIND_BYTES` one — the ONLY difference between
/// a runtime String and a runtime Bytes escape (both are UTF-8/byte leaves on the rope heap).
pub(super) fn runtime_leaf_form(db: &mut Db, is_string: bool) -> Option<RuntimeBytesForm> {
    let _ = db; // (kept for signature symmetry with the other form builders; not needed here)
    const KIND_BYTES: u8 = 11;
    const KIND_STR: u8 = 7;
    let mut b = crate::ast::Builder::new();
    let colon = b.name(":");
    let (empty, ty_name, kind) = if is_string {
        (
            b.atom_leaf(crate::ast::Leaf::Str(String::new().into())),
            b.name("String"),
            KIND_STR,
        )
    } else {
        (
            b.atom_leaf(crate::ast::Leaf::Bytes(Vec::new().into())),
            b.name("Bytes"),
            KIND_BYTES,
        )
    };
    let root = b.list(vec![colon, empty, ty_name]);
    let arenas = b.finish(root);
    let encoded = crate::codec::encode(&arenas);
    // Find the leaf's KIND tag IMMEDIATELY followed by its `0x00` length byte (the empty leaf). `":"` and
    // the type name are NAME leaves (`0x0a …`), so the only `<kind> 00` pair is the empty payload leaf.
    let pos = encoded.windows(2).position(|w| w == [kind, 0x00])?;
    let prefix = encoded[..=pos].to_vec();
    let suffix = encoded[pos + 2..].to_vec();
    Some(RuntimeBytesForm { prefix, suffix })
}

/// A value-form template: the byte buffer (placeholders in the leaf values) + the runtime holes.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ValueFormTemplate {
    pub bytes: Vec<u8>,
    pub leaves: Vec<RuntimeLeaf>,
}

/// The escape template for a SUM result — one complete value-form template per variant (its rendered
/// `(: (VariantName payload…) SumType)` bytes + holes), indexed by DISCRIMINANT. Unlike a tuple/record
/// (one static shape, one template), a sum renders DIFFERENTLY per variant (`(Some 5)` vs `(None unit)`
/// — different name, different payload), so the walker must switch on the runtime discriminant
/// (`sum-disc`) and emit the matching variant's template. Each variant's payload leaves carry
/// `via_sum_payload` (they are reached through `sum-payload` first). `type-system.md §A Match Is
/// Exhaustive Against The Sum Type's Variant Set` — the variant set is closed, so the switch is total.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SumFormTemplate {
    /// One template per variant, in DISCRIMINANT (declaration) order — `variants[disc]` renders the
    /// value with that discriminant.
    pub variants: Vec<ValueFormTemplate>,
}

/// Build the [`SumFormTemplate`] for a `Ty::Sum` result: one value-form template per variant. Each
/// variant's template renders `(: (VariantName payload…) SumType)` with the payload leaves left as
/// holes reached through `sum-payload`. A NULLARY variant renders `(VariantName unit)` (no holes) — the
/// corpus form (`(None unit)`). A SINGLE-payload variant renders `(VariantName <scalar-hole>)`, the
/// hole reached directly off the payload handle (`via_sum_payload`, empty `path`). A MULTI-payload
/// variant renders `(VariantName p0 p1 …)`, the holes reached by `arr-get` into the payload tuple. A
/// payload whose type has no value-form surface (a function/nested-sum for now) makes the whole thing
/// `None` — the escape declines. Needs `db` to read the variant names + payload types from
/// `db.type_decls` (found by the sum's `decl` occurrence).
pub fn sum_form_template(db: &mut Db, ty: &crate::ty::Ty) -> Option<SumFormTemplate> {
    let crate::ty::Ty::Sum { decl, args, .. } = ty else {
        return None;
    };
    // Recover the variant set + the declaration's type PARAMS from the declaration occurrence. A generic
    // sum's payload occurrences mention the params (a lowercase `a`); the instantiation's `args` are the
    // concrete types to substitute for them, positionally.
    let decl_ref = db.type_decl_by_occ(*decl)?;
    let params = decl_ref.params.clone();
    // Clone the shape out so we can reduce payload types with `&mut db` below.
    let variants: Vec<(String, Vec<StructId>)> = decl_ref
        .variants
        .iter()
        .map(|v| (v.name.clone(), v.payloads.clone()))
        .collect();
    let mut out = Vec::with_capacity(variants.len());
    for (disc, (_, payload_occs)) in variants.iter().enumerate() {
        // Reduce each payload TYPE occurrence to a `Ty` AT THE INSTANTIATION: a payload that IS a type
        // parameter (a bare name in `params`) becomes the corresponding concrete `arg`; any other
        // payload reduces normally (`typeval_of`). This is what makes a generic `Option Int64` escape
        // with its `Some` payload templated as `Int64` rather than the unresolvable param `a`.
        let mut payload_tys = Vec::with_capacity(payload_occs.len());
        for &p in payload_occs {
            let pty = match db.ast.as_name(p) {
                Some(n) if params.iter().any(|q| q == n) => {
                    let idx = params.iter().position(|q| q == n).unwrap();
                    args.get(idx).cloned()?
                }
                _ => crate::eval::typeval_of(db, p)?,
            };
            payload_tys.push(pty);
        }
        // The variant HEAD — BARE (`Some`, `Cons`) normally, QUALIFIED `(. Ast List)` when the sum has a
        // variant name a prelude entry shadows (see `variant_head_ast`) — so the runtime walker writes the
        // same head the constant bake does.
        out.push(variant_form_template(
            db,
            *decl,
            disc as u32,
            &payload_tys,
            ty,
        )?);
    }
    Some(SumFormTemplate { variants: out })
}

// ─── Shape descriptor (for the runtime `value-encode` op) ────────────────────────────────────────
//
// This descriptor IS the type-directed code the compiler emits so a compound result crosses the
// boundary as an ordinary string: the compiler (which holds the names + the static type) bakes a shape
// table, and the runtime `value-encode` walker reads it to walk the value through the runtime's
// accessors and assemble its canonical text — the runtime never names or renders the value itself.
//= spec/contracts/component-abi.md#a-compound-result-is-rendered-by-compiler-emitted-code
//# The observable result of a program that produces a compound value MUST be an ordinary string the program returns, produced by type-directed code the compiler emits that walks the value through the runtime's accessors and assembles its canonical text, rather than by the runtime rendering the value or by the program's own component owning a display of it, so that the names a rendering requires stay with the compiler that holds them and the host reads back a plain string (host-interface-binding.md §The Host Does Not Format A Component's Values).
// The text this descriptor drives is the value's CANONICAL text form (the runtime's byte-exact
// `value_encode_form_matches_the_codec` cross-check + the corpus keep the encoder in lock-step), so a
// compound result crossing the boundary is byte-identical to the same value's recorded corpus form:
//= spec/contracts/component-abi.md#a-compound-result-is-rendered-by-compiler-emitted-code
//# The text the compiler-emitted rendering produces MUST be the value's canonical text form under deterministic-value-form.md, so that a compound result crossing the boundary is byte-identical to the same value's recorded corpus form.
// Because THE COMPILER exposes this render (and the codec exposes read-text→canonical-binary), the host
// never formats a value itself — it reads back a plain string the compiler-emitted code produced:
//= spec/contracts/host-interface-binding.md#the-host-does-not-format-a-component-s-values
//# The rendering of a value to its canonical text form and the reading of text to a value's canonical binary form MUST be operations the compiler exposes, not operations the host performs, so that "what a Cadenza value looks like" lives in the compiler rather than the host.
//
// The compiler-baked descriptor the runtime `value-encode` walker reads to render a RUNTIME value —
// including a self-referential (recursive) sum, which has no fixed hole-template. A descriptor is a
// TABLE of shapes + a root index; a child position references another entry by index, so a recursive
// type closes as a finite cycle (a `Ref` back to the sum's entry). Wire format is documented on the
// runtime side (`cdz-runtime` value-encode note); this is the ENCODER, kept in lock-step (a drift is
// caught by the runtime's byte-exact `value_encode_form_matches_the_codec` cross-check + the corpus).
//
// Shape tags: 0 Int, 1 Bool, 2 Float, 3 Str, 4 Bytes, 5 Unit, 6 Tuple[n][idx…], 7 List[idx],
// 8 Record[n](name,idx)…, 9 Sum[n](head,idx)…, 10 Named(name,idx), 11 Ref(idx), 12 Set[idx],
// 13 Map(k,v), 14 Float32, 15 Framed(head,[arg…],idx), 16 Spread[n][idx…] (a multi-payload variant's
// tuple payload, rendered FLAT under the variant head).

/// Build the shape descriptor bytes for a `Ty::Sum` result, wrapped in the outer `(: <value> <Type>)`
/// frame — the input to the runtime `value-encode` op. Handles a RECURSIVE sum: a sum decl already in
/// the table is referenced by index (`Ref`), closing the cycle. `None` if any payload type has no
/// renderable shape yet (a Float/Str/Bytes payload — a later slice; the escape then declines cleanly).
/// Build a `Set`-ROOTED shape descriptor for `set-to-list`: the runtime `op_set_to_list` resolves the
/// descriptor's ROOT and requires `Shape::Set(elem)` DIRECTLY (to order by the element shape), NOT the
/// `Framed(<type-node>, …)` value-form wrapper `sum_shape_descriptor` produces. So encode the bare
/// `shape_of(Set elem)` root. Returns `None` if the element shape has no descriptor (unorderable).
pub fn set_shape_descriptor(db: &mut Db, elem_ty: &crate::ty::Ty) -> Option<Vec<u8>> {
    let mut builder = ShapeTableBuilder::default();
    let set_ty = crate::ty::Ty::Set(Box::new(elem_ty.clone()));
    let root = builder.shape_of(db, &set_ty)?;
    Some(builder.encode(root))
}

/// Build a `Map`-ROOTED shape descriptor for `map-to-list`: `op_map_to_list` resolves the root and
/// requires `Shape::Map(key, val)` DIRECTLY (to order by the KEY shape), NOT the `Framed` value-form
/// wrapper. Encode the bare `shape_of(Map key val)` root. `None` if the key/value shape has no
/// descriptor (unorderable). The map companion of [`set_shape_descriptor`].
pub fn map_shape_descriptor(
    db: &mut Db,
    key_ty: &crate::ty::Ty,
    val_ty: &crate::ty::Ty,
) -> Option<Vec<u8>> {
    let mut builder = ShapeTableBuilder::default();
    let map_ty = crate::ty::Ty::Map(Box::new(key_ty.clone()), Box::new(val_ty.clone()));
    let root = builder.shape_of(db, &map_ty)?;
    Some(builder.encode(root))
}

/// Build a bare-`ty`-ROOTED shape descriptor for the runtime `value-cmp` op: `value_cmp_shaped` resolves
/// the descriptor's ROOT to the operands' shape DIRECTLY and walks it (blessed per-leaf orders + compound
/// lexicographic) — NOT the `Framed(<type-node>, …)` value-form wrapper `sum_shape_descriptor` produces.
/// So encode the bare `shape_of(ty)` root (the same discipline as `set_shape_descriptor`/`map_shape_
/// descriptor`). `None` if the type has no descriptor (a component the shape table can't encode — the emit
/// then declines cleanly, matching the compiler's decision not to order it).
pub fn value_cmp_shape_descriptor(db: &mut Db, ty: &crate::ty::Ty) -> Option<Vec<u8>> {
    let mut builder = ShapeTableBuilder::default();
    let root = builder.shape_of(db, ty)?;
    Some(builder.encode(root))
}

/// The BARE value-form shape descriptor for `ty` — the inner value shape with NO `Named`/`Framed`
/// type-frame wrapper. The reducer FOLD BOUNDARY (apply param decode + result encode) carries BARE value
/// documents: the kernel's `val_to_ast`/`build_event_document`/`parse_effect_list` are bare (root head
/// `record`/`list`, `= name value` fields), and the type is statically known on BOTH sides, so no inline
/// type frame is wanted. `value-encode` frames the output as `(: value Type)` ONLY when the descriptor
/// ROOT is `Shape::Named`/`Framed` (v-ah + v-runtime ruling 2026-08-12); rooting at the bare `shape_of`
/// makes it emit the bare form, byte-matching the kernel. `sum_shape_descriptor` (the ESCAPE path) keeps
/// the frame — that path crosses to an untyped host that renders `(: value Type)`; this boundary does not.
pub fn bare_shape_descriptor(db: &mut Db, ty: &crate::ty::Ty) -> Option<Vec<u8>> {
    // Same DOMAIN as `sum_shape_descriptor` — only a value-form COMPOUND (sum/collection/record/tuple/
    // bignum) has a value-form descriptor; a bare scalar/function/etc. has NONE and must decline (the bytes
    // boundary requires a value-decodable compound param + a value-encodable compound result). `shape_of`
    // alone would accept a scalar, so gate on `sum_shape_descriptor` first, then emit the UNFRAMED shape.
    sum_shape_descriptor(db, ty)?;
    let mut builder = ShapeTableBuilder::default();
    let root = builder.shape_of(db, ty)?;
    Some(builder.encode(root))
}

pub fn sum_shape_descriptor(db: &mut Db, ty: &crate::ty::Ty) -> Option<Vec<u8>> {
    let mut builder = ShapeTableBuilder::default();
    match ty {
        // A boxed sum. A MONOMORPHIC sum (`args: []`) wraps in `Named(<type name>, …)` — the bare-name
        // `(: <value> <Type>)` frame (`(: (Neg unit) Sign)`). A GENERIC sum (`args` non-empty) must render
        // its type ARGUMENTS too (`(: (Some "é") (Option String))`, NOT the bare `(: … Option)`), so it
        // wraps in a PARAMETRIC `Framed(<type-node>, …)` frame built from the full type (`type_node_of`
        // renders `(Option String)`), exactly as a `List`/`Map`/`Set` does. Without this a generic sum
        // result dropped its type args at the boundary.
        crate::ty::Ty::Sum { decl, args, .. } => {
            let name = db.type_decl_by_occ(*decl)?.name.clone();
            let inner = builder.shape_of(db, ty)?;
            if args.is_empty() {
                let named = builder.push(ShapeNode::Named(name, inner));
                Some(builder.encode(named))
            } else {
                let type_node = type_node_of(ty, &db.name_ctx())?;
                let framed = builder.push(ShapeNode::Framed(type_node, inner));
                Some(builder.encode(framed))
            }
        }
        // A NOMINAL newtype (a recursive one that escapes): its `shape_of` ALREADY produces a
        // `Named(<type name>, …)` root (the erased-tag frame), so encode it directly — wrapping again
        // would double-name it. This is what carries the recursive newtype's OWN name to the host
        // (`(: … Lst)`), where routing on the stripped inner sum would have named it `Option`.
        crate::ty::Ty::Nominal { .. } => {
            let root = builder.shape_of(db, ty)?;
            Some(builder.encode(root))
        }
        // A LIST/SET/MAP result: build the value shape, then wrap in a PARAMETRIC `Framed(<type-node>, …)`
        // frame so the value form renders `(: (list …) (List <elem>))` etc. — the element/key/value types
        // OBSERVABLE, matching the constant-collection value form. The type node is built RECURSIVELY from
        // the full type (`type_node_of`), so a nested element type crosses too: `(List (List Int64))`,
        // `(Map Int64 (Set Int64))`, `(Set (Tuple Int64 Int64))`. The inner VALUE shape (`shape_of`) already
        // recurses over nested collections, so the walker renders them; only the type node needed lifting.
        crate::ty::Ty::List(_) | crate::ty::Ty::Set(_) | crate::ty::Ty::Map(_, _) => {
            let type_node = type_node_of(ty, &db.name_ctx())?;
            let inner = builder.shape_of(db, ty)?;
            let framed = builder.push(ShapeNode::Framed(type_node, inner));
            Some(builder.encode(framed))
        }
        // A RUNTIME-computed `BigInt`/`Rational` result: its value form is VARIABLE-length (a BigInt
        // magnitude is however many bytes the value needs — the fixed-hole `runtime_value_form_template`
        // cannot serve it), so it escapes via the SAME runtime `value-encode` walker as a collection. The
        // runtime already renders `Shape::BigInt` (tag 17) / `Shape::Rational` as a `KIND_INT` leaf / a
        // `{num,den}` record. Wrap in a `Framed(<type-node>, …)` frame so the value form is `(: N BigInt)`
        // (the `Named` bare-name frame the constant escape uses), the type node observable.
        crate::ty::Ty::BigInt | crate::ty::Ty::Rational => {
            let type_node = type_node_of(ty, &db.name_ctx())?;
            let inner = builder.shape_of(db, ty)?;
            let framed = builder.push(ShapeNode::Framed(type_node, inner));
            Some(builder.encode(framed))
        }
        // A TUPLE/RECORD result whose value shape is renderable but which contains a VARIABLE-length element
        // (a list/map/set, or a sum) — `runtime_value_form_template` returns `None` for it (no fixed-size
        // static template), so it escapes via the same runtime `value-encode` walker as a collection. Wrap in
        // a PARAMETRIC `Framed(<type-node>, …)` frame so the value form renders `(: (tuple …) (Tuple …))` /
        // `(: (record …) (Record …))` with the element/field types observable. `shape_of` already recurses
        // over the nested elements (a list element loops, a nested sum switches on its disc). A fixed-shape
        // tuple/record (all scalar/byte/fixed-compound elements) still takes the cheaper static-template path
        // (`runtime_value_form_template`), which the caller tries FIRST — this descriptor path is the fallback
        // for the variable-shape case only.
        crate::ty::Ty::Tuple(_) | crate::ty::Ty::Record(_) => {
            let type_node = type_node_of(ty, &db.name_ctx())?;
            let inner = builder.shape_of(db, ty)?;
            let framed = builder.push(ShapeNode::Framed(type_node, inner));
            Some(builder.encode(framed))
        }
        _ => None,
    }
}

/// The RECURSIVE type node for a `Framed` frame's type position — mirrors `Ty::render_name` structurally
/// so the runtime's `render_type_node` reproduces the same written type. A leaf (a scalar/nominal/nullary
/// sum) is a bare-name node with no children; a parametric type (`List`/`Set`/`Map`/`Tuple`/`Record`/a
/// generic sum) is a head plus child type nodes, nested to any depth. `None` for a type that never appears
/// as an escaping collection element (Fn/Qty/Var/Any/Type) — the escape declines rather than misrender it.
pub(super) fn type_node_of(ty: &crate::ty::Ty, ncx: &crate::ty::NameCtx) -> Option<TypeNode> {
    use crate::ty::Ty;
    let leaf = |s: String| TypeNode {
        head: s,
        children: vec![],
    };
    Some(match ty {
        Ty::Int(_)
        | Ty::Bool
        | Ty::Unit
        | Ty::String
        | Ty::Char
        | Ty::Symbol
        | Ty::BigInt
        | Ty::Rational
        | Ty::Float(_)
        | Ty::Bytes => leaf(ty.render_name(ncx)),
        Ty::List(e) => TypeNode {
            head: "List".to_string(),
            children: vec![type_node_of(e, ncx)?],
        },
        Ty::Set(e) => TypeNode {
            head: "Set".to_string(),
            children: vec![type_node_of(e, ncx)?],
        },
        Ty::Map(k, v) => TypeNode {
            head: "Map".to_string(),
            children: vec![type_node_of(k, ncx)?, type_node_of(v, ncx)?],
        },
        Ty::Tuple(elems) => {
            let mut children = Vec::with_capacity(elems.len());
            for e in elems.iter() {
                children.push(type_node_of(e, ncx)?);
            }
            TypeNode {
                head: "Tuple".to_string(),
                children,
            }
        }
        // A record renders as `(record (name Type) …)`: each field is itself a node `(name <type>)` — head
        // = the field name, one child = the field's type node. `render_type_node` reproduces `(name Type)`.
        Ty::Record(fields) => {
            let mut children = Vec::with_capacity(fields.len());
            for (k, t) in fields.iter() {
                children.push(TypeNode {
                    head: k.name.to_string(),
                    children: vec![type_node_of(t, ncx)?],
                });
            }
            TypeNode {
                head: "record".to_string(),
                children,
            }
        }
        // A monomorphic sum renders as its bare name; a generic sum as `(Name arg…)`.
        Ty::Sum { decl, args, .. } => {
            let name = ncx.name_of(*decl)?.to_string();
            if args.is_empty() {
                leaf(name)
            } else {
                let mut children = Vec::with_capacity(args.len());
                for a in args.iter() {
                    children.push(type_node_of(a, ncx)?);
                }
                TypeNode {
                    head: name,
                    children,
                }
            }
        }
        Ty::Nominal { decl, .. } => leaf(ncx.name_of(*decl)?.to_string()),
        // Qty/Fn/Var/Any/Type: not an escaping collection element/arg — decline rather than misrender.
        _ => return None,
    })
}

/// Append `v` to `out` as an unsigned LEB128 varint — the count/length/index encoding the shape-table
/// descriptor wire format uses throughout (see `ShapeTableBuilder::encode`). Kept local to the shape
/// descriptor here rather than shared with the wasm backend's `encode::uleb128`, which the backend
/// deliberately scopes as a wasm-target concern.
pub(super) fn leb(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let mut b = (v & 0x7f) as u8;
        v >>= 7;
        if v != 0 {
            b |= 0x80;
        }
        out.push(b);
        if v == 0 {
            break;
        }
    }
}

/// A shape-table entry (indices reference other entries — recursion closes via `Ref`).
pub(super) enum ShapeNode {
    Int,
    /// An arbitrary-precision integer (a runtime `BigInt`). The runtime (descriptor tag 17) reads it via
    /// `unbox_bigint` and renders the SAME `KIND_INT` leaf as `Int` — the codec leaf is already
    /// arbitrary-width (sign + big-endian magnitude), so no new wire KIND is needed, only the shape tag.
    BigInt,
    /// A runtime `Rational` — a normalized 2-BigInt-handle node. The runtime (descriptor tag 18) reads both
    /// components via `unbox_rational` and renders the `num/den` NAME leaf (each component formatted decimal
    /// in the runtime — the codec's Int leaf renders decimal on the HOST, but a rational is ONE name leaf,
    /// so the runtime formats it), matching the constant-Rational value form (`(: 1/2 Rational)`).
    Rational,
    Bool,
    /// A `Char` — an i32 Unicode code-point at run time (NO distinct runtime rep: char is an int, exactly
    /// like `Bool` is an i32 0/1). This is a pure RENDER tag, the bool-analog: the runtime (descriptor tag
    /// 19) reads the i32 code-point and renders it as a `#\c` char literal (via the codec's `KIND_CHAR`
    /// leaf), the SAME mechanism by which `Bool` (tag 1) renders `true`/`false`. value-EQ/-CMP/hash treat a
    /// Char IDENTICALLY to `Int` (by code-point) — only the render differs — so a compound carrying a Char
    /// leaf orders/compares as before; the tag change is render-only. (Was `ShapeNode::Int` until the
    /// char-as-bool ruling; the render tag makes a runtime `String.scalar-at` char display as `#\c`.)
    Char,
    /// A `Symbol` — shares the constant-STRING runtime rep (a Symbol's identity IS its UTF-8 text, see
    /// `Symbol.of`), so value-EQ/-CMP/hash/ORDER treat a Symbol IDENTICALLY to `Str` (a Set/Map Symbol key
    /// orders by its text). This is a pure RENDER tag (descriptor tag 20), the `Char`-over-`Int` analog:
    /// the runtime reads the string like `Str` but renders the CONSTRUCTION form `((. Symbol of) "text")`
    /// (the #7694 member-compound, byte-matching `const_value_ast` + the rust backend), NOT the bare `Str`
    /// leaf `"text"` (which is ambiguous with a real String). (Was `ShapeNode::Str` — a runtime Symbol in a
    /// compound then mis-rendered as a bare string, divergent from rust; the render tag fixes the parity.)
    Symbol,
    Float,
    Float32,
    Str,
    Bytes,
    Unit,
    Tuple(Vec<u32>),
    List(u32),
    Record(Vec<(String, u32)>),
    Sum(Vec<(String, u32)>),
    Named(String, u32),
    Ref(u32),
    Set(u32),
    Map(u32, u32),
    /// A `(: <value> <type-node>)` frame — an arbitrary (possibly NESTED) type node. The runtime
    /// `value-encode` decodes this as descriptor tag 15 and renders the type node RECURSIVELY, so a runtime
    /// collection crosses as `(: (list …) (List Int64))` — or, with nesting, `(: … (List (List Int64)))`.
    Framed(TypeNode, u32),
    /// A MULTI-payload variant's payload — a tuple handle at run time whose elements render FLATTENED
    /// under the variant head (`(Cons h t)`, NOT `(Cons (tuple h t))`). The runtime (descriptor tag 16)
    /// reads it exactly like a `Tuple` (each element via `arr-get`) but renders the elements DIRECTLY as
    /// the variant's children rather than wrapping them in a `tuple` form. Only a `Sum` variant references
    /// a `Spread` (it is the multi-payload variant payload); a genuine tuple VALUE stays a `Tuple`.
    Spread(Vec<u32>),
}

/// A compile-time-built TYPE node for a `Framed` frame's type position, written to the descriptor wire as
/// `[ head ][ n_children ]( TypeNode )*n` and rebuilt+rendered by the runtime's `render_type_node`. A leaf
/// (a scalar/nominal) has no children; `(List Int64)` = head `List`, one child `Int64`; nests arbitrarily.
pub(super) struct TypeNode {
    head: String,
    children: Vec<TypeNode>,
}

/// Builds the shape table, memoizing each `Ty::Sum`/`Ty::Nominal` by its full INSTANTIATION (decl + a
/// structural fingerprint of its type args) so a recursive reference reuses the same entry (a `Ref`)
/// rather than expanding forever. Keying on the decl ALONE is unsound for a GENERIC decl: `Option Bytes`
/// and `Option P` share the `Option` decl but are DIFFERENT types (`type-system.md`: two sums agree iff
/// decl AND args agree), so a decl-only memo aliases the second instantiation's shape onto the first —
/// silently mis-encoding sibling `Option`-of-different-arg fields (the `Option<P>`-beside-`Option<Bytes>`
/// value-encode miscompile). The args are a FINITE tree (recursion lives in variant PAYLOADS, closed by
/// this same memo — never in the args), so the fingerprint terminates.
#[derive(Default)]
pub(super) struct ShapeTableBuilder {
    table: Vec<ShapeNode>,
    /// (sum/nominal decl, args-fingerprint) → its table index (filled BEFORE the variants are built, so a
    /// self-reference at the SAME instantiation resolves to a `Ref`).
    sums: std::collections::HashMap<(StructId, String), u32>,
}

/// A stable structural fingerprint of a type, injective enough to distinguish two instantiations of the
/// same generic decl (`Option Bytes` vs `Option P`). Uses decl NUMBERS (not names) for nominal/sum
/// identity and recurses over children/args — it descends into a sum/nominal's ARGS (finite) but NOT its
/// variant payloads (that recursion is closed by the `sums` memo, and the args already disambiguate the
/// instantiation). Only used as a `HashMap` key; not written to any wire format.
pub(super) fn ty_instantiation_key(ty: &crate::ty::Ty) -> String {
    use crate::ty::Ty;
    use std::fmt::Write;
    let mut s = String::new();
    fn go(s: &mut String, ty: &Ty) {
        use crate::ty::Ty;
        match ty {
            Ty::Int(w) => {
                let _ = write!(s, "I{w:?}");
            }
            Ty::BigInt => s.push_str("BI"),
            Ty::Rational => s.push_str("Ra"),
            Ty::Bool => s.push('B'),
            Ty::Float(w) => {
                let _ = write!(s, "F{w:?}");
            }
            Ty::String => s.push_str("St"),
            Ty::Symbol => s.push_str("Sy"),
            Ty::Bytes => s.push_str("By"),
            Ty::Unit => s.push('U'),
            Ty::Tuple(es) => {
                s.push_str("T(");
                for e in es.iter() {
                    go(s, e);
                    s.push(',');
                }
                s.push(')');
            }
            Ty::List(e) => {
                s.push_str("L(");
                go(s, e);
                s.push(')');
            }
            Ty::Set(e) => {
                s.push_str("Se(");
                go(s, e);
                s.push(')');
            }
            Ty::Map(k, v) => {
                s.push_str("M(");
                go(s, k);
                s.push(',');
                go(s, v);
                s.push(')');
            }
            Ty::Record(fs) => {
                s.push_str("R(");
                for (n, t) in fs.iter() {
                    let _ = write!(s, "{}:", n.name);
                    go(s, t);
                    s.push(',');
                }
                s.push(')');
            }
            Ty::Sum { decl, args } => {
                let _ = write!(s, "S{}<", decl.0);
                for a in args.iter() {
                    go(s, a);
                    s.push(',');
                }
                s.push('>');
            }
            Ty::Nominal { decl, args, .. } => {
                let _ = write!(s, "N{}<", decl.0);
                for a in args.iter() {
                    go(s, a);
                    s.push(',');
                }
                s.push('>');
            }
            Ty::Qty { inner, .. } => {
                s.push_str("Q(");
                go(s, inner);
                s.push(')');
            }
            // Any/Var and other non-instantiation-bearing leaves: a coarse tag is fine (they never key a
            // distinct sum instantiation — a fully-solved reducer boundary type has no free vars).
            other => {
                let _ = write!(s, "?{other:?}");
            }
        }
    }
    go(&mut s, ty);
    s
}

impl ShapeTableBuilder {
    fn push(&mut self, node: ShapeNode) -> u32 {
        self.table.push(node);
        (self.table.len() - 1) as u32
    }

    /// The table index of a shape for `ty`, building it (and its sub-shapes) if new. A `Ty::Sum` already
    /// in progress returns a `Ref` to its (reserved) entry, closing recursion. `None` for an
    /// unrenderable leaf type (Float/Str/Bytes — a later slice).
    fn shape_of(&mut self, db: &mut Db, ty: &crate::ty::Ty) -> Option<u32> {
        use crate::ty::Ty;
        Some(match ty {
            Ty::Int(_) => self.push(ShapeNode::Int),
            // A runtime BigInt (arbitrary precision) escapes as `ShapeNode::BigInt` — the runtime reads it
            // via `unbox_bigint` and renders the same arbitrary-width `KIND_INT` leaf as a fixed-width int.
            Ty::BigInt => self.push(ShapeNode::BigInt),
            // A runtime Rational escapes as `ShapeNode::Rational` — the runtime reads its two BigInt
            // children and renders the `num/den` name leaf (R3c).
            Ty::Rational => self.push(ShapeNode::Rational),
            Ty::Bool => self.push(ShapeNode::Bool),
            // A FLOAT payload is renderable at BOTH widths: `box_op_ty`/`get_op_ty` box it via its width's
            // leaf (`box-float`/`box-float32`), and the runtime `value-encode` renders a KIND_FLOAT decimal
            // — Float64 from the f64, Float32 from its OWN 4-byte leaf (the f32's shortest decimal).
            Ty::Float(ft) if ft.ground_width() == 64 => self.push(ShapeNode::Float),
            Ty::Float(ft) if ft.ground_width() == 32 => self.push(ShapeNode::Float32),
            Ty::String => self.push(ShapeNode::Str),
            // A SYMBOL shares the String byte-leaf runtime rep (tagless heap — no intern table; a Symbol IS
            // its canonical UTF-8 leaf, see `Symbol.of`), so it stays orderable-like-Str: `orderable_leaf_or_
            // compound` admits `Ty::Symbol` (a `(Set Symbol)`/`(Map Symbol _)` key orders by content) and the
            // runtime `value_cmp_shaped` orders `Shape::Symbol` IDENTICALLY to `Shape::Str`. But its descriptor
            // is now the RENDER-ONLY `ShapeNode::Symbol` (tag 20, the `Char`-over-`Int` analog) so the runtime
            // renders the CONSTRUCTION form `((. Symbol of) "text")` (byte-matching `const_value_ast` + rust),
            // NOT the bare `Str` leaf. (Was `ShapeNode::Str`: a runtime Symbol in a compound then mis-rendered
            // as a bare string `"text"`, ambiguous with a real String + divergent from rust's `(Symbol.of …)`
            // — the #7694/breaker parity gap. The const path already bakes the member form via `const_value_
            // ast`; this closes the RUNTIME value-encode path.)
            Ty::Symbol => self.push(ShapeNode::Symbol),
            // A CHAR is a scalar — an i32 Unicode code-point slot at run time (Char-rep). Its value-op
            // descriptor is `ShapeNode::Char` (the bool-analog RENDER tag, descriptor tag 19): the runtime
            // reads the i32 code-point and RENDERS it as a `#\c` char literal (via the codec's KIND_CHAR),
            // exactly as `ShapeNode::Bool` renders an i32 0/1 as `true`/`false`. It is a RENDER tag only, NOT
            // a distinct runtime rep — value-EQ/-CMP/hash treat a Char IDENTICALLY to Int (chars equal iff
            // code-points equal, order by code-point), so a compound carrying a `Char` leaf (e.g. the `Ast`
            // sum's `Ast.Char` variant) compares/orders exactly as before; only the RENDER changed from a
            // bare code-point number to a char literal (char-as-bool ruling, seq-247 — supersedes the prior
            // `ShapeNode::Int` mapping + the runtime-Char WONTFIX). Requires the runtime's tag-19 arm in
            // decode_shape/value-encode/value_cmp_shaped/value_eq/hash (v-runtime flag-day). A bare-`Char`
            // scalar compare never reaches here (scalar path). (Ast.encode/decode still ride KIND_CHAR too.)
            Ty::Char => self.push(ShapeNode::Char),
            Ty::Bytes => self.push(ShapeNode::Bytes),
            Ty::Unit => self.push(ShapeNode::Unit),
            Ty::Tuple(elems) => {
                // TYPE-DIRECTED render (concierge ruling B, 2026-07-28): Unit and `(Tuple)` are DISTINCT
                // types (05-compound:9232-9239 — a variant's explicit empty-tuple payload keeps its `(tuple)`
                // form, distinct from a nullary `unit`). So an EMPTY `Ty::Tuple` must emit a `Tuple[0]`
                // shape (rendered `(tuple)` by the runtime `value-encode` walker), NOT collapse to
                // `ShapeNode::Unit` (which would render `unit` — the wrong type surface for a `(Tuple)`-typed
                // value, and inconsistent with the const path + the rust `cdz_render_expr` path, both of
                // which already render `(tuple)`). Pairs LOCKSTEP with the runtime walker's `Shape::Tuple`
                // empty arm (cdz-runtime `value-encode`): descriptor carrying `Tuple[0]` + walker rendering
                // `(tuple)` together fix the heap-path `unit` mis-render; NEITHER alone changes it.
                let mut idxs = Vec::with_capacity(elems.len());
                for e in elems.iter() {
                    idxs.push(self.shape_of(db, e)?);
                }
                self.push(ShapeNode::Tuple(idxs))
            }
            Ty::List(elem) => {
                let e = self.shape_of(db, elem)?;
                self.push(ShapeNode::List(e))
            }
            // A SET renders `(Set.of (list …))` with elements in CANONICAL element-VALUE order. The runtime
            // sorts by the element's canonical value order via `value_cmp_shaped` — the SAME descriptor-guided
            // total order the runtime `<`/`Core::ValueCmp` walk uses — which orders any ORDERABLE element:
            // a blessed scalar leaf (Int/Bool/Unit/String/Symbol/BigInt/Rational) OR an orderable COMPOUND
            // (a tuple/list/record/sum all of whose leaves are orderable), lexicographically. So admit a set
            // over any such element (`orderable_leaf_or_compound`, stripping a nominal wrapper — a newtype is
            // erased to its inner value). A float/bytes/set/map leaf has no blessed total order → decline
            // (matching `<`'s carve-outs and the const escape).
            Ty::Set(elem)
                if orderable_leaf_or_compound(db, elem.strip_nominal(), true, &mut Vec::new()) =>
            {
                let e = self.shape_of(db, elem)?;
                self.push(ShapeNode::Set(e))
            }
            // A MAP renders `(map (k1 v1) …)` with entries in CANONICAL KEY order. The runtime sorts by the
            // KEY's canonical value order via `value_cmp_shaped` (the same total order `<` uses), so admit a
            // map over any ORDERABLE key — a blessed scalar leaf OR an orderable compound (tuple/list/record/
            // sum of orderable leaves); the VALUE may be any encodable shape (the walk recurses on it). A
            // float/bytes/set/map key leaf has no blessed total order → decline.
            Ty::Map(key, val)
                if orderable_leaf_or_compound(db, key.strip_nominal(), true, &mut Vec::new()) =>
            {
                let k = self.shape_of(db, key)?;
                let v = self.shape_of(db, val)?;
                self.push(ShapeNode::Map(k, v))
            }
            Ty::Record(fields) => {
                let mut out = Vec::with_capacity(fields.len());
                for (name, t) in fields.iter() {
                    let idx = self.shape_of(db, t)?;
                    out.push((name.name.to_string(), idx));
                }
                self.push(ShapeNode::Record(out))
            }
            Ty::Sum { decl, .. } => {
                // Key by the full INSTANTIATION (decl + args fingerprint), NOT decl alone — a generic
                // `Option Bytes` and `Option P` share the `Option` decl but have DIFFERENT shapes; a
                // decl-only memo would alias the second onto the first (the sibling-Option value-encode
                // miscompile). A recursive self-reference has the SAME instantiation key, so it still
                // closes to a `Ref`.
                let key = (*decl, ty_instantiation_key(ty));
                // Already building/built this instantiation → a Ref to its reserved entry (closes recursion).
                if let Some(&existing) = self.sums.get(&key) {
                    return Some(self.push(ShapeNode::Ref(existing)));
                }
                // Reserve THIS sum's entry index BEFORE building the variants (a variant payload that
                // references the sum resolves to this index). Fill it in place once the variants are built.
                let self_ix = self.push(ShapeNode::Unit); // placeholder, overwritten below
                self.sums.insert(key, self_ix);
                let variants = sum_variant_payload_types(db, ty)?;
                let mut out = Vec::with_capacity(variants.len());
                for (head, payload_tys) in variants {
                    // A variant's payload shape: no payload → Unit; one → that type; MANY → a SPREAD.
                    // A MULTI-payload variant `(Cons Int64 L)` boxes its payloads as a tuple handle at run
                    // time, but its CANONICAL value form is FLAT — `(Cons h t)`, matching both the surface
                    // construction `(L.Cons h t)` and the non-recursive `sum_form_template` render
                    // (`(Variant p0 p1 …)`). So it uses `Spread` (the runtime renders the variant head
                    // followed by each tuple ELEMENT, no `tuple` wrapper) — NOT `Tuple` (which would render
                    // `(Cons (tuple h t))`, exposing the internal boxing). A SINGLE tuple-typed payload
                    // `(Cons (Tuple Int64 L))` is a genuine one-payload variant whose payload IS a tuple, so
                    // it takes the `1 =>` arm and renders `(Cons (tuple h t))` correctly.
                    let payload_ix = match payload_tys.len() {
                        0 => self.push(ShapeNode::Unit),
                        1 => self.shape_of(db, &payload_tys[0])?,
                        _ => {
                            let mut idxs = Vec::with_capacity(payload_tys.len());
                            for pt in &payload_tys {
                                idxs.push(self.shape_of(db, pt)?);
                            }
                            self.push(ShapeNode::Spread(idxs))
                        }
                    };
                    out.push((head, payload_ix));
                }
                self.table[self_ix as usize] = ShapeNode::Sum(out);
                self_ix
            }
            // A NOMINAL newtype is ERASED at run time — the value IS its underlying value — so its shape
            // is its inner's shape, wrapped in `Named(<type name>, …)` so the host renders `(: <underlying>
            // <TypeName>)`. Recursion closes on the nominal's OWN `decl` (a RECURSIVE newtype's inner
            // re-references it): reserve the entry keyed by `decl` BEFORE building the inner (a
            // self-reference resolves to a `Ref`), then fill it. The inner's `Ty::Sum{decl}` back-edge (the
            // erased-newtype μ-binder) resolves to this same reserved entry via `sums`, so the shape table
            // is finite. Reuses `Named`, which the runtime `value-encode` walker already renders.
            Ty::Nominal { decl, inner, .. } => {
                // Same instantiation-keying as `Ty::Sum` — a generic nominal `Box Int64` vs `Box Bool`
                // share the decl but differ, so key on (decl, args fingerprint). A recursive nominal's
                // inner back-edge re-references the SAME instantiation → resolves to the reserved `Ref`.
                let key = (*decl, ty_instantiation_key(ty));
                if let Some(&existing) = self.sums.get(&key) {
                    return Some(self.push(ShapeNode::Ref(existing)));
                }
                let name = db.type_decl_by_occ(*decl)?.name.clone();
                let self_ix = self.push(ShapeNode::Unit); // placeholder, filled below
                self.sums.insert(key, self_ix);
                let inner_ix = self.shape_of(db, inner)?;
                self.table[self_ix as usize] = ShapeNode::Named(name, inner_ix);
                self_ix
            }
            // A QUANTITY erases to its inner scalar at runtime (the unit is a compile-time concern carried
            // only in the solved `Ty::Qty`), so its RUNTIME SHAPE is exactly its inner's shape — peel `Ty::Qty`
            // to the inner and recurse. This is what lets a quantity be an element of a compound Map/Set KEY (a
            // `(List (Qty Int64 meter))` / `(Tuple (Qty …) …)` key): the key comparator/hasher canonicalizes
            // on the erased magnitude, matching how a bare flat `(Qty …)` key already hashes by its inner. The
            // VALUE-render frame (`type_node_of`) still declines a bare Qty element (the unit-labelled type node
            // is a later slice), so this newly enables only the descriptor/key path, not a Qty value-render.
            Ty::Qty { inner, .. } => self.shape_of(db, inner)?,
            // Float payload rendering is a later slice — decline (the escape falls through). Str/Bytes are
            // supported (→ `ShapeNode::Str`/`Bytes`, above); the runtime `value-encode` renders their leaves.
            _ => return None,
        })
    }

    /// Serialize the table + root to the descriptor wire format (all counts/lengths unsigned LEB128).
    fn encode(&self, root: u32) -> Vec<u8> {
        fn name(out: &mut Vec<u8>, s: &str) {
            leb(out, s.len() as u64);
            out.extend_from_slice(s.as_bytes());
        }
        let mut d = Vec::new();
        leb(&mut d, self.table.len() as u64);
        for node in &self.table {
            match node {
                ShapeNode::Int => d.push(0),
                ShapeNode::BigInt => d.push(17), // matches the runtime `decode_shape` tag 17 = BigInt
                ShapeNode::Rational => d.push(18), // matches the runtime `decode_shape` tag 18 = Rational
                ShapeNode::Bool => d.push(1),
                ShapeNode::Char => d.push(19), // matches the runtime `decode_shape` tag 19 = Char (render-only; value = i32 code-point, cmp/eq/hash as Int)
                ShapeNode::Symbol => d.push(20), // matches the runtime `decode_shape` tag 20 = Symbol (render-only; value = str, cmp/eq/hash/order as Str; renders `((. Symbol of) "…")`)
                ShapeNode::Float => d.push(2),   // matches the runtime `decode_shape` tag 2 = Float
                ShapeNode::Float32 => d.push(14), // matches the runtime `decode_shape` tag 14 = Float32
                ShapeNode::Str => d.push(3),      // matches the runtime `decode_shape` tag 3 = Str
                ShapeNode::Bytes => d.push(4), // matches the runtime `decode_shape` tag 4 = Bytes
                ShapeNode::Unit => d.push(5),
                ShapeNode::Tuple(idxs) => {
                    d.push(6);
                    leb(&mut d, idxs.len() as u64);
                    for &i in idxs {
                        leb(&mut d, i as u64);
                    }
                }
                ShapeNode::List(i) => {
                    d.push(7);
                    leb(&mut d, *i as u64);
                }
                ShapeNode::Record(fields) => {
                    d.push(8);
                    leb(&mut d, fields.len() as u64);
                    for (k, i) in fields {
                        name(&mut d, k);
                        leb(&mut d, *i as u64);
                    }
                }
                ShapeNode::Sum(variants) => {
                    d.push(9);
                    leb(&mut d, variants.len() as u64);
                    for (h, i) in variants {
                        name(&mut d, h);
                        leb(&mut d, *i as u64);
                    }
                }
                ShapeNode::Named(n, i) => {
                    d.push(10);
                    name(&mut d, n);
                    leb(&mut d, *i as u64);
                }
                ShapeNode::Ref(i) => {
                    d.push(11);
                    leb(&mut d, *i as u64);
                }
                ShapeNode::Set(i) => {
                    d.push(12); // matches the runtime `decode_shape` tag 12 = Set
                    leb(&mut d, *i as u64);
                }
                ShapeNode::Map(k, v) => {
                    d.push(13); // matches the runtime `decode_shape` tag 13 = Map
                    leb(&mut d, *k as u64);
                    leb(&mut d, *v as u64);
                }
                ShapeNode::Framed(type_node, i) => {
                    // recursive TypeNode wire: [ head ][ n_children ]( TypeNode )*n — the runtime's
                    // `decode_type_node` mirrors this depth-first walk.
                    fn write_type_node(out: &mut Vec<u8>, tn: &TypeNode) {
                        leb(out, tn.head.len() as u64);
                        out.extend_from_slice(tn.head.as_bytes());
                        leb(out, tn.children.len() as u64);
                        for c in &tn.children {
                            write_type_node(out, c);
                        }
                    }
                    d.push(15); // matches the runtime `decode_shape` tag 15 = Framed (14 = Float32)
                    write_type_node(&mut d, type_node);
                    leb(&mut d, *i as u64);
                }
                ShapeNode::Spread(idxs) => {
                    d.push(16); // matches the runtime `decode_shape` tag 16 = Spread
                    leb(&mut d, idxs.len() as u64);
                    for &i in idxs {
                        leb(&mut d, i as u64);
                    }
                }
            }
        }
        leb(&mut d, root as u64);
        d
    }
}

/// Map a solved [`Ty`] to its WIT SCHEMA-DESCRIPTOR node in `b`'s arena — the byte-exact per-`Ty` rule
/// (v-effects spec, 2026-08-13) that makes a userspace effect's op-signature descriptor hash IDENTICALLY
/// to a built-in of the same shape (the schema-hash-only effect identity). Returns `None` (DECLINE) for a
/// type with no WIT form or a still-unground numeric width — never guesses, since a wrong descriptor would
/// bake a wrong effect identity.
///
/// The descriptor MUST be a genuine TREE: this builds a FRESH node per occurrence (a recursive call per
/// constituent), never a memoized/shared node — because the kernel `codec::decode`s the descriptor on
/// ingest (ruling A) and rejects a shared subtree as `NotATree` (a decode-bomb guard). Do NOT add
/// memoization here (unlike the value-encode `ShapeTableBuilder`, whose sharing is internal and never
/// crosses the codec).
///
/// The per-`Ty` mapping (matches the kernel's built-in decls + `build_type`):
///  - `Bool`/`Char`/`String` → `wit_type_prim`; `Unit` → `wit_type_unit`.
///  - `Int{sign,width}` → `wit_type_prim` of `u8/u16/u32/u64` (unsigned) or `s8/s16/s32/s64` (signed),
///    GROUNDING the sign+width exactly as the value boundary does (`ty_natural_wit`/`valtype_of`): a
///    still-deferred axis defaults (signed, 64) so an effect op's integer always lowers to a definite wit
///    prim, matching how the built-ins' integer types ground. A non-{8,16,32,64} width has no prim → DECLINE.
///  - `Float{width}` → `wit_type_prim` `f32`/`f64` by ground width; a non-{32,64} width → DECLINE.
///  - `List(e)` → `wit_type_list(rec e)`; `Bytes` → `wit_type_list(prim u8)` (NOT a `bytes` prim — the
///    built-ins spell bytes as `list<u8>`, and a `bytes` prim would hash-diverge).
///  - `Tuple(elems)` → `wit_type_tuple([rec e…])` (POSITIONAL — order is identity).
///  - `Record(BTreeMap)` → `wit_type_record([(name, rec t)…])` iterating the BTreeMap in NATURAL order,
///    which is name-lexicographic (the `Symbol` key orders by its `str`) = the canonical NAME-SORTED order
///    the kernel now emits — so the sort is FREE, never applied here.
///  - `Sum{decl=Option, args=[i]}` → `wit_type_option(rec i)`. rcdzc has NO `Ty::Option` — Option is an
///    ordinary sum over the prelude `Option` decl — but the built-ins reflect an optional value as WIT
///    `option<T>`, so this ONE sum is special-cased to the option form (detected by decl name); a bare
///    `Some|None` variant would hash-diverge from a built-in's `option<T>`.
///  - `Sum{decl}` (any other) → `wit_type_variant([(CaseName, payload?)…])` in DECL order
///    (`sum_variant_payload_types` returns cases in `decl.variants` order); a case's payload is `None`
///    (nullary), the single payload's desc (1), or a `tuple` desc (multiple — the value side reps a
///    multi-payload variant as a tuple).
///  - `Nominal{inner}` → recurse on `inner` (nominal is ERASED at the wire — its machine rep IS `inner`).
///  - `Map`/`Set`/`Symbol`/`Qty`/`BigInt`/`Rational`/`Fn`/`Type`/`Any`/… → DECLINE (no built-in effect
///    op-sig uses them yet; inventing a wit form would bake a wrong identity).
// STAGED: the production caller is the schema-hash phase-1a perform-fork at lower.rs:1026 (non-import-member
// effect → reify_effect_to_tuple → this descriptor build), landing in the NEXT slice. Until that fork wires
// it, the only callers are the unit tests below, so the non-test lib build sees it as unused — the allow is
// removed when the fork lands (it is genuinely live then). Not hiding dead code: this is a tested, complete
// function staged one slice ahead of its production call site.
pub(super) fn ty_to_wit_desc(
    db: &mut Db,
    b: &mut crate::ast::Builder,
    ty: &crate::ty::Ty,
) -> Option<StructId> {
    use crate::ty::Ty;
    match ty {
        Ty::Bool => Some(b.wit_type_prim("bool")),
        Ty::Char => Some(b.wit_type_prim("char")),
        Ty::String => Some(b.wit_type_prim("string")),
        Ty::Unit => Some(b.wit_type_unit()),
        Ty::Int(int_ty) => {
            // GROUND the sign+width the same way the value boundary does (`ty_natural_wit`,
            // `valtype_of`): a still-deferred axis defaults (signed, 64), never declines — an effect op's
            // integer must lower to a definite wit prim, matching how the built-ins' integer types ground.
            let kind = match (int_ty.ground_signed(), int_ty.ground_width()) {
                (true, 8) => "s8",
                (true, 16) => "s16",
                (true, 32) => "s32",
                (true, 64) => "s64",
                (false, 8) => "u8",
                (false, 16) => "u16",
                (false, 32) => "u32",
                (false, 64) => "u64",
                _ => return None, // a non-{8,16,32,64} width has no wit prim
            };
            Some(b.wit_type_prim(kind))
        }
        Ty::Float(float_ty) => {
            let kind = match float_ty.ground_width() {
                32 => "f32",
                64 => "f64",
                _ => return None,
            };
            Some(b.wit_type_prim(kind))
        }
        Ty::List(elem) => {
            let e = ty_to_wit_desc(db, b, elem)?;
            Some(b.wit_type_list(e))
        }
        Ty::Bytes => {
            let u8 = b.wit_type_prim("u8");
            Some(b.wit_type_list(u8))
        }
        Ty::Tuple(elems) => {
            let mut descs = Vec::with_capacity(elems.len());
            for e in elems.iter() {
                descs.push(ty_to_wit_desc(db, b, e)?);
            }
            Some(b.wit_type_tuple(&descs))
        }
        Ty::Record(fields) => {
            // Iterate the BTreeMap in natural (name-lexicographic) order = the canonical NAME-SORTED order.
            // Collect names first so the &str borrows outlive the builder calls.
            let entries: Vec<(std::sync::Arc<str>, crate::ty::Ty)> = fields
                .iter()
                .map(|(sym, t)| (sym.name.clone(), t.clone()))
                .collect();
            let mut built = Vec::with_capacity(entries.len());
            for (name, t) in &entries {
                let desc = ty_to_wit_desc(db, b, t)?;
                built.push((&**name, desc));
            }
            Some(b.wit_type_record(&built))
        }
        Ty::Sum { decl, args } => {
            // OPTION is spelled `("option" <inner>)`, NOT an inlined `Some|None` variant — rcdzc has no
            // `Ty::Option` (Option is an ordinary sum over the prelude `Option` decl), but the built-ins
            // reflect an optional value as WIT `option<T>` (`build_type`'s `Type::Option` → wit_type_option),
            // so a userspace effect taking an Option must hash to THAT, not to a variant. Detect the prelude
            // Option decl by name (the established precedent, infer.rs `option_payload_mismatch_hint`) and
            // emit the option form; every OTHER sum (a user CloseOutcome-style variant) → wit_type_variant.
            let is_option = db.name_ctx().name_of(*decl) == Some("Option");
            if is_option {
                let [inner] = &args[..] else {
                    return None; // a malformed Option instantiation — decline rather than guess
                };
                let inner = inner.clone();
                let i = ty_to_wit_desc(db, b, &inner)?;
                return Some(b.wit_type_option(i));
            }
            // Cases in DECL order, each with its payload types at this instantiation.
            let variants = sum_variant_payload_types(db, ty)?;
            let mut cases: Vec<(String, Option<StructId>)> = Vec::with_capacity(variants.len());
            for (name, payloads) in variants {
                let payload_desc = match payloads.len() {
                    0 => None,
                    1 => Some(ty_to_wit_desc(db, b, &payloads[0])?),
                    // A multi-payload variant reps as a tuple (matching the value side, lower.rs:15120).
                    // No built-in effect exercises this yet, so it is an emit-only extrapolation of the
                    // spec's single-`payload?` rule; the kernel does not currently reflect it.
                    _ => {
                        let mut descs = Vec::with_capacity(payloads.len());
                        for p in &payloads {
                            descs.push(ty_to_wit_desc(db, b, p)?);
                        }
                        Some(b.wit_type_tuple(&descs))
                    }
                };
                cases.push((name, payload_desc));
            }
            let case_refs: Vec<(&str, Option<StructId>)> =
                cases.iter().map(|(n, d)| (n.as_str(), *d)).collect();
            Some(b.wit_type_variant(&case_refs))
        }
        // Nominal is erased at the wire — its machine rep IS `inner`, so hash through `inner`.
        Ty::Nominal { inner, .. } => ty_to_wit_desc(db, b, inner),
        // A MAP reflects as `list<tuple<K, V>>` — the canonical value-form a Map crosses/encodes as (a
        // sorted-key list of key/value pairs, matching `map_shape_descriptor` + the runtime value-encode
        // walker). A userspace effect op CAN be Map-typed (e.g. `(op vars (-> Unit (Map Int64 Int64)))`,
        // `(op stamp (-> (Map String Int64) (Map String Int64)))`) — 15 such op decls in the corpus — so the
        // schema-hash descriptor MUST cover it or v-effects' mandatory-schema_hash flip regresses those cases
        // (S3 hard gate). Self-consistent + rcdzc-only: the kernel HASHES the descriptor nodes emitted here
        // (`effect_schema_hash_from_nodes` canonicalizes + hashes the tree; it does not re-derive via
        // `build_type`), so this node shape IS the identity — no kernel Map/Set reflection needed.
        Ty::Map(k, v) => {
            let kd = ty_to_wit_desc(db, b, k)?;
            let vd = ty_to_wit_desc(db, b, v)?;
            let pair = b.wit_type_tuple(&[kd, vd]);
            Some(b.wit_type_list(pair))
        }
        // A SET reflects as `list<E>` — the canonical value-form a Set crosses/encodes as (a sorted list of
        // its elements, matching `set_shape_descriptor` + the value-encode walker). Reachable as an effect-op
        // type (`(op check (-> (Set (Tuple Int64 Int64)) Int64))`), so covered alongside Map (S3 hard gate).
        Ty::Set(elem) => {
            let e = ty_to_wit_desc(db, b, elem)?;
            Some(b.wit_type_list(e))
        }
        // No built-in effect op-sig uses these yet — DECLINE rather than invent a (wrong) identity.
        _ => None,
    }
}

/// Reduce an effect operation's declared arrow-type occurrence (`OpDecl.ty`, the `(-> P… R)` written after
/// the op name) to `(param-types, result-type)` — the shape an op's `wit_func_sig` schema node needs. The
/// arrow is curried (`(-> A B C)` = `(-> A (-> B (-> C)))`), so peel every `Ty::Fn` to collect the params
/// in declaration (positional) order and take the final non-arrow as the result.
///
/// The ELIDED-UNIT convention (mirrors `effects.rs` arm-arity handling): a `(-> Unit R)` operation takes NO
/// value parameter — a bare unit is "no argument", not a unit-typed one — so a single `Unit` param is
/// dropped to an empty param list (matching the kernel's `UnitToBytes`/`UnitToUnit` built-in shapes, which
/// emit `wit_func_sig(&[], …)`). A `Unit` in any OTHER position is a real positional param and kept.
///
/// `None` if the op has no declared type (a malformed `(op NAME)`) or the type does not reduce to an arrow
/// (a nullary op with a bare result type — no params, but then there is no arrow to peel; handled by the
/// caller as a 0-param op). Takes `&mut Db` because `typeval_of` reduces the type occurrence.
pub(super) fn op_arrow_param_result_tys(
    db: &mut Db,
    ty_occ: StructId,
) -> Option<(Vec<crate::ty::Ty>, crate::ty::Ty)> {
    use crate::ty::Ty;
    let mut ty = crate::eval::typeval_of(db, ty_occ)?;
    let mut params = Vec::new();
    while let Ty::Fn(param, result) = ty {
        params.push(*param);
        ty = *result;
    }
    // ELIDED UNIT: `(-> Unit R)` is a no-parameter op (a unit arg is "nothing"), so a lone `Unit` param
    // drops to zero params — matching the kernel's Unit-arg built-in ops (`wit_func_sig(&[], …)`).
    if params.len() == 1 && params[0] == Ty::Unit {
        params.clear();
    }
    Some((params, ty))
}

/// Build a NAME-FREE effect-op function-signature node `(func (param Desc)… (result Desc))` — the op-sig
/// shape the EFFECT-schema path uses (schema-hash ruling B, concierge-confirmed 2026-08-13): op-param NAMES
/// are NOT part of an effect's identity (a userspace op arrow is POSITIONAL and anonymous — there is no name
/// to recover — so a name-bearing identity would make a userspace effect un-matchable to a built-in and
/// defeat content-address routing). So each param node is a 2-list `(param Desc)`, NOT the name-bearing
/// 3-list `(param Name Desc)` the SHARED `Builder::wit_func_sig` emits for the WIT-WORLD path (where WIT
/// member param names ARE the contract). rcdzc only READS world descriptors, never builds them, so its
/// effect path uses this name-free form exclusively. `params` are the positional type descriptors in arrow
/// order; `result` is the always-present result descriptor (a no-return op passes `unit`). Heads are NAME
/// atoms (head-kind-fixed, matching `effect_schema_tree`), so identical op sigs encode byte-identically.
pub(super) fn effect_op_sig_name_free(
    b: &mut crate::ast::Builder,
    params: &[StructId],
    result: StructId,
) -> StructId {
    let mut children = Vec::with_capacity(1 + params.len() + 1);
    let func_head = b.name("func");
    children.push(func_head);
    for &desc in params {
        let param_head = b.name("param");
        let param_node = b.list(vec![param_head, desc]);
        children.push(param_node);
    }
    let result_head = b.name("result");
    let result_node = b.list(vec![result_head, result]);
    children.push(result_node);
    b.list(children)
}

/// Build the full effect SCHEMA-descriptor tree `(effect Name (op OpName Sig)…)` for `decl` into `b` — the
/// tree whose canonical-encode content-hash is the effect's schema-hash identity (the schema-hash-only
/// effect model). Each op's `Sig` is a NAME-FREE `(func (param Desc)… (result Desc))` (ruling B — op-param
/// names are not identity) over its arrow's positional param types + result, each mapped by `ty_to_wit_desc`.
///
/// Ops are sorted by name (matching the kernel's `effect_schema_hash_from_nodes`, which re-sorts on ingest)
/// so the identity is the SET of ops, order-independent — and so this descriptor's OWN hash matches the
/// built-in without relying on the kernel's re-sort. `None` (DECLINE) if any op's type is missing or a
/// param/result type has no WIT form (a still-generic op, an unsupported type) — never emits a partial
/// descriptor that would bake a wrong identity. Every node is FRESH (no sharing) so the descriptor is a
/// genuine tree the kernel `codec::decode`s without a `NotATree` rejection (ruling A).
pub(super) fn effect_schema_descriptor(
    db: &mut Db,
    b: &mut crate::ast::Builder,
    decl: StructId,
) -> Option<StructId> {
    // Snapshot (name, ty-occ) per op so the borrow of `db`'s EffectDecl ends before the ty_to_wit_desc calls
    // (which take `&mut db`). Op order is the declaration order; we sort by name below.
    let ops: Vec<(String, Option<StructId>)> = db
        .effect_decl_by_occ(decl)?
        .ops
        .iter()
        .map(|op| (op.name.clone(), op.ty))
        .collect();
    let effect_name = db.effect_decl_by_occ(decl)?.name.clone();

    let mut built: Vec<(String, StructId)> = Vec::with_capacity(ops.len());
    for (op_name, ty_occ) in ops {
        let ty_occ = ty_occ?; // a malformed `(op NAME)` with no type → DECLINE the whole effect
        let (params, result) = op_arrow_param_result_tys(db, ty_occ)?;
        let mut param_descs = Vec::with_capacity(params.len());
        for p in &params {
            param_descs.push(ty_to_wit_desc(db, b, p)?);
        }
        let result_desc = ty_to_wit_desc(db, b, &result)?;
        let sig = effect_op_sig_name_free(b, &param_descs, result_desc);
        built.push((op_name, sig));
    }
    // Sort by op-name (order-independent identity — matches the kernel's effect_schema_hash_from_nodes).
    built.sort_by(|a, c| a.0.cmp(&c.0));
    let op_refs: Vec<(&str, StructId)> = built.iter().map(|(n, s)| (n.as_str(), *s)).collect();
    Some(b.effect_schema_tree(&effect_name, &op_refs))
}

/// Whether the reify EMITS a `schema_descriptor` field for the effect `decl` — i.e. whether
/// [`effect_schema_descriptor`] BUILDS (it declines for a still-generic op / a type with no WIT
/// descriptor). The SINGLE SOURCE OF TRUTH for the reify's schema_descriptor condition: the reify emit
/// (`reify_effect_to_tuple`) gates the field on this, AND the reify TYPING (`infer::world_effect_request_ty`
/// via its callsite) gates the `schema_descriptor: Bytes` record field on this — so the emitted record shape
/// and its typed shape CANNOT DRIFT (the phase-3 producer-bake bug: the emit added the field but the typing
/// didn't, so the typed 3-field record DROPPED the emit's 4th field → schema_hash None at the kernel). Both
/// sides call THIS, so a change to when the descriptor builds moves both in lockstep. `pub(crate)` so `infer`
/// can call it.
pub(crate) fn effect_has_schema_descriptor(db: &mut Db, decl: StructId) -> bool {
    let mut b = crate::ast::Builder::new();
    effect_schema_descriptor(db, &mut b, decl).is_some()
}

/// Lower a NON-import (world-effect) perform `(E.op args…)` to its reified EFFECT-REQUEST record — the
/// entry a reducer's `apply` returns in its effect-list (schema-hash phase-1a; v-ah ruling: apply returns
/// the effect-list, a world-effect perform reifies into it, NO result-threading — the result arrives as a
/// later event). The record is the canonical value-form the kernel's `parse_effect_request` decodes — a
/// 3-field record (v-ah ruling S2, 2026-08-14), NO target column:
///   `("record" (= correlation (None)) (= kind "effect/<name>") (= payload <Option>))`
/// fields NAME-SORTED (correlation < kind < payload — the sorted-slot order value-encode and the kernel
/// both read; a `Record` BTreeMap sorts them, so building via the record AST is inherently sorted).
///
/// CAPABILITY-BLIND (v-ah G3-reify ruling, GENERIC-COMPILER-clean): the reify does NOT split a "target"
/// arg out, and there is NO target column at all (that would need to know which effects are target-chosen
/// = a baked capability vocabulary, forbidden). Every arg rides the payload value-form; the DOWNSTREAM
/// kernel/executor extracts the SEC-F1 resource — an arg INSIDE the payload — via the effect DECL's
/// `(resource N)` marker, not a wire field. Target-free effects (model/now/timer) carry no resource, so a
/// no-target-column record is uniformly correct.
///
/// `kind` = `"effect/<effect-name>"` (the register-by-string family; String for phase-1a — v-effects flips
/// it to the schema-hash at their phase-2 core-flip, a one-line change here). `correlation` = `None` (a
/// concurrency token is a later slice).
///
/// The RESOURCE ARG IS SKIPPED. `resource` is the op's `@resource`-marked param index (`OpDecl.resource`,
/// from the decl's `(resource N)` sibling) — the SEC-F1 destination the kernel/executor extracts + Cedar
/// authorizes. It is dropped from the args here (NOT put in the payload) — the reducer's `Emit.send(dest,
/// payload)` carries `dest` as arg `N`, and the kernel derives the request's target from it via the marker;
/// the wire record has no target column (ruling S2). This is capability-blind: it reads the DECL's marker
/// index, not a hard-coded which-effects-are-target-chosen vocabulary. Crucially it also reduces arg count
/// — a 2-arg `Emit.send(@resource dest, payload)` becomes a ONE-remaining-arg payload → reifies with NO
/// in-fold value-encode (the resource-skip, not R2, is what unblocks the common one-resource-one-payload op).
///
/// The PAYLOAD is the args with the resource arg removed: `Some(<the single Bytes remaining arg>)`, or
/// `None` for zero remaining args. A structured (non-Bytes) remaining arg (`Model.request(mr:
/// ModelRequest)`) OR more than one remaining arg needs an in-fold value-encode primitive that does not
/// exist yet (R2, escalated to the operator) — DECLINE cleanly until it lands. Built by synthesizing the
/// record AST + `resolve_subtree` + `core_of`, so the typed `Some`/`None`/record machinery is reused.
pub(super) fn reify_effect_to_tuple(
    db: &mut Db,
    effect: &str,
    args: &[StructId],
    resource: Option<usize>,
    decl: Option<StructId>,
) -> Core {
    // Split the args into the `@resource`-designated TARGET (rides its own `target` wire field) and the
    // PAYLOAD args (everything else). The reducer-chosen destination of a target-having effect
    // (`Emit.send(@resource dest, payload)`) MUST reach the kernel as a RUNTIME VALUE — SEC-F1 authorizes
    // the dest VALUE against a resource predicate (DescendantOf etc.), which needs the value — so it rides
    // the wire in a dedicated `target` field (v-rb ruling A, 2026-08-14, resolving v-effects' freeze-blocking
    // reify/2b-extraction crux). The `@resource` arg is skipped from the PAYLOAD-encode ONLY (payload stays
    // single-Bytes → no R2), NOT dropped. Capability-blind: the kernel reads `target` as bare bytes via the
    // existing optional `field("target")` read (zero kernel change) WITHOUT decoding the payload. A resource
    // index out of range degrades to "no target" (a malformed decl; the arity check below then declines).
    let (target_arg, payload_args): (Option<StructId>, Vec<StructId>) = match resource {
        Some(idx) if idx < args.len() => {
            let rest = args
                .iter()
                .enumerate()
                .filter_map(|(i, &a)| (i != idx).then_some(a))
                .collect();
            (Some(args[idx]), rest)
        }
        _ => (None, args.to_vec()),
    };
    // The `@resource` target rides the wire as a BARE Bytes field (the kernel reads `target` via `read_bytes`).
    // A non-Bytes resource designation needs the deferred in-fold value-encode → decline (phase-1a: a Bytes
    // dest — e.g. a peer/session id — is the only target shape that reifies today).
    if let Some(t) = target_arg
        && !matches!(crate::infer::type_of(db, t), crate::ty::Ty::Bytes)
    {
        // ROADMAP (v-effects world-effect reification): a non-Bytes @resource target is schema-hash
        // phase-1a — a structured resource target will ride an in-fold value-encode in a later increment.
        return Core::Poison(Reject::unsupported(
            "reifying a world-effect perform whose @resource target is not Bytes is unsupported: only a \
             Bytes resource designation (a peer or session id) rides the target wire field; a structured \
             resource target requires an in-fold value-encode the reifier does not apply to the @resource \
             field",
        ));
    }
    // PAYLOAD (capability-blind): one Bytes remaining arg → Some(bytes verbatim); zero → None; a SINGLE
    // STRUCTURED (non-Bytes) arg → Some(Value.encode arg) (R2 in-fold value-form); MULTI remaining arg still
    // declines (phase-1b: tuple-then-encode).
    let payload_field = match payload_args.as_slice() {
        [] => {
            let none_h = db.push_name("None");
            db.push_list(vec![none_h]) // (None) — the payload-free effect
        }
        [only] if matches!(crate::infer::type_of(db, *only), crate::ty::Ty::Bytes) => {
            let some_h = db.push_name("Some");
            db.push_list(vec![some_h, *only]) // (Some <bytes-arg>) — the arg IS the payload, no encode
        }
        [only] => {
            // A SINGLE STRUCTURED (non-Bytes) payload (R2 carve-out, v-effects-unblocked 2026-08-15: S3
            // schema-hash identity flip + the R2 Value.encode prim both landed). Value-encode the arg IN-FOLD
            // by synthesizing `(Some ((. Value encode) <arg>))`: the member-access application lowers to
            // `Core::ValueEncode { value, desc: sum_shape_descriptor(arg-ty) }` (the existing `Value.encode`
            // lowering computes the descriptor), producing the canonical binary-AST value-form bytes the
            // DOWNSTREAM CONSUMER's `Value.decode` reads (the Model handler decoding `ModelRequest`; the
            // reply consumer). A RUNTIME `Core::ValueEncode` is valid here because reify runs IN-FOLD (unlike
            // `Ast.encode`'s constant-only limit). The payload is OPAQUE to the kernel `parse_effect_request`
            // (it reads it as `Payload::Inline` bytes, never decoding/re-hashing it — the effect-IDENTITY
            // descriptor is the SEPARATE `schema_descriptor` field above), so no framing beyond the value form
            // is needed. Unblocks `Model.request(ModelRequest)` + `close`/`reply(structured)`.
            let dot = db.push_name(".");
            let value_mod = db.push_name("Value");
            let encode = db.push_name("encode");
            let member = db.push_list(vec![dot, value_mod, encode]); // (. Value encode)
            let app = db.push_list(vec![member, *only]); // ((. Value encode) <arg>)
            let some_h = db.push_name("Some");
            db.push_list(vec![some_h, app]) // (Some ((. Value encode) <arg>))
        }
        _ => {
            // ROADMAP (v-effects world-effect reification): multi-arg payload is schema-hash phase-1b —
            // bundling multiple args into a tuple before the in-fold value-encode is a later increment.
            return Core::Poison(Reject::unsupported(
                "reifying a world-effect perform with a multi-argument payload is unsupported: a single \
                 Bytes or single structured payload reifies; multiple args require bundling into a tuple \
                 before the in-fold value-encode",
            ));
        }
    };
    // kind = "effect/<name>" (String for phase-1a; isolated for the phase-2 String→schema-hash flip).
    let kind_field = db.push_str(&format!("effect/{effect}"));
    // correlation = None (a concurrency token is a later slice).
    let correlation_field = {
        let none_h = db.push_name("None");
        db.push_list(vec![none_h])
    };
    // target field (v-rb ruling A, 2026-08-14): PRESENT iff the effect designates an `@resource` target — a
    // BARE Bytes wire field carrying the reducer-chosen destination VALUE (e.g. the peer/session id of an
    // `Emit.send(@resource dest, …)`). A target-FREE effect (model/now/timer/tool — no `@resource`) emits NO
    // target field, so its record stays 3-field {correlation, kind, payload}; the kernel reads `target` as
    // OPTIONAL (absent → empty), so both shapes parse uniformly (no vestigial always-empty field — no-adapter
    // directive). This resolves v-effects' freeze-blocking reify/2b crux: the dest is NOT lost (ruling B was
    // wrong — SEC-F1 authorizes the dest value) and NOT value-encoded with the payload (ruling C = R2 +
    // breaks capability-blindness); it rides its own field, so 2b @resource extraction is the existing
    // `field("target")` read, and v-pc's `target_str()=="out"` stays green.
    let target_field = target_arg;

    // schema_descriptor field (phase-3 producer-bake, v-effects (B) 2026-08-14): the effect's RAW schema
    // descriptor as encoded bytes, so the kernel `codec::decode`s it + hashes it via the one-hasher
    // (`effect_schema_hash_from_nodes`) to obtain the authoritative schema-hash identity — RATHER than rcdzc
    // baking the hash (which would MISMATCH: rcdzc emits raw bottom-up bytes, the kernel re-canonicalizes on
    // ingest — ruling A, no `canon.rs` port; the bridge test pins the through-canon identity). BEST-EFFORT:
    // `effect_schema_descriptor` DECLINES (None) for a still-generic op or an unsupported type — in that case
    // OMIT the field, and the kernel falls back to the family-derived hash (byte-identical to pre-phase-3), so
    // a reify whose descriptor doesn't build is UNCHANGED (no regression). The descriptor is built in its own
    // `Builder` arena, so it can't splice into this db-arena record directly; encode it to a `Bytes` leaf (the
    // kernel decodes the field bytes back into descriptor nodes). INERT until the kernel's schema_descriptor
    // read lands (an unrecognized field is ignored by `parse_effect_request`) — co-gated with that read.
    let schema_descriptor_field: Option<StructId> = decl.and_then(|d| {
        let mut b = crate::ast::Builder::new();
        let root = effect_schema_descriptor(db, &mut b, d)?;
        let bytes = crate::codec::encode(&b.finish(root));
        Some(db.push_atom(crate::ast::Leaf::Bytes(bytes.into())))
    });

    // Synthesize the record AST `(#record (= correlation …) (= kind …) (= payload …) [(= target …)]
    // [(= schema_descriptor …)])` in the M2 NATIVE form: a native RecordCtor-leaf head + native FieldPair-leaf
    // fields (`push_compound`/`push_field_pair`), not the legacy `("record" …)` string head. `resolve_record`
    // reads the fields into a name-SORTED BTreeMap, so the slot order is canonical regardless of order here.
    let field = |db: &mut Db, name: &str, val: StructId| -> StructId {
        let n = db.push_name(name);
        db.push_field_pair(n, val)
    };
    let corr = field(db, "correlation", correlation_field);
    let kind = field(db, "kind", kind_field);
    let payload = field(db, "payload", payload_field);
    let mut record_items = vec![corr, kind, payload];
    if let Some(t) = target_field {
        let target = field(db, "target", t);
        record_items.push(target);
    }
    if let Some(sd) = schema_descriptor_field {
        let sd_field = field(db, "schema_descriptor", sd);
        record_items.push(sd_field);
    }
    let record = db.push_compound(crate::ast::CompoundCtor::Record, record_items);
    // Re-resolve the synthesized subtree (binds `Some`/`None`/the field spellings) then lower it — reusing
    // the typed record/sum-value machinery so the `Core::Record` + its `Some`/`None` payloads are built +
    // typed exactly as a source-written record would be.
    crate::resolve::resolve_subtree(db, record);
    core_of(db, record)
}

/// The FAMILY STRING an async (non-import) perform reifies its effect-request `kind` field to, for the
/// effect whose declaration occurrence is `decl` — `"effect/" + <declared effect name>` (schema-hash
/// phase-1a reify, v-effects ruling A 2026-08-13). The kernel's `parse_effect_request` reads `kind` as a
/// register-by-string family (`new_with_family`); a userspace effect routes via this `effect/<name>` family
/// (a handler session claims `effect/weather` etc. at runtime, dispatched by the `UserspaceEffectExecutor`
/// fallback). This is the family string TODAY — phase-2 flips `kind` to the schema-hash (`effect_schema_
/// descriptor` becomes the identity carrier); this helper's caller (`reify_effect_to_tuple`) swaps to the
/// hash at that flip. The op name is NOT in the family (the op is implicit in the payload shape today;
/// phase-2's schema-hash carries op identity). `None` if `decl` names no effect declaration.
#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn userspace_effect_family_kind(db: &Db, decl: StructId) -> Option<String> {
    let name = &db.effect_decl_by_occ(decl)?.name;
    Some(format!("effect/{name}"))
}

/// The variants of a `Ty::Sum` as `(head-name, payload-types)` pairs at this instantiation — the head
/// spelled as the runtime template writes it (a BARE variant name; the value form renders variants bare,
/// e.g. `(Cons …)`, `(None unit)`). Mirrors `sum_form_template`'s variant/payload recovery.
pub(super) fn sum_variant_payload_types(
    db: &mut Db,
    ty: &crate::ty::Ty,
) -> Option<Vec<(String, Vec<crate::ty::Ty>)>> {
    let crate::ty::Ty::Sum { decl, args, .. } = ty else {
        return None;
    };
    let decl_ref = db.type_decl_by_occ(*decl)?;
    let params = decl_ref.params.clone();
    let variants: Vec<(String, Vec<StructId>)> = decl_ref
        .variants
        .iter()
        .map(|v| (v.name.clone(), v.payloads.clone()))
        .collect();
    let mut out = Vec::with_capacity(variants.len());
    for (head, payload_occs) in variants {
        let mut payload_tys = Vec::with_capacity(payload_occs.len());
        for &p in &payload_occs {
            let pty = match db.ast.as_name(p) {
                Some(n) if params.iter().any(|q| q == n) => {
                    let idx = params.iter().position(|q| q == n).unwrap();
                    args.get(idx).cloned()?
                }
                _ => crate::eval::typeval_of(db, p)?,
            };
            payload_tys.push(pty);
        }
        out.push((head, payload_tys));
    }
    Some(out)
}

/// One variant's value-form template: `(: <variant-head> payload…) SumType)`, payload leaves as holes
/// reached via `sum-payload`. Arity shapes the value + the hole paths (see [`sum_form_template`]). The
/// variant HEAD is built by [`variant_head_ast`] (bare normally, qualified `(. Type Variant)` when the
/// sum has a prelude-shadowed variant), so the runtime template writes the identical head the constant
/// bake does.
pub(super) fn variant_form_template(
    db: &mut Db,
    decl: StructId,
    disc: u32,
    payloads: &[crate::ty::Ty],
    sum_ty: &crate::ty::Ty,
) -> Option<ValueFormTemplate> {
    let mut b = crate::ast::Builder::new();
    let colon = b.name(":");
    let mut leaves: Vec<PendingLeaf> = Vec::new();
    // The VALUE: `(<variant-head> payload…)`.
    let value = {
        let head = variant_head_ast(db, &mut b, decl, disc)?;
        let mut children = vec![head];
        match payloads.len() {
            // Nullary: `(VariantName unit)` — the corpus form (`(None unit)`), no holes.
            0 => {
                children.push(b.name("unit"));
            }
            // Single payload: reached DIRECTLY off the payload handle — `via_sum_payload`, empty path.
            1 => {
                let mut path = Vec::new();
                children.push(template_value_ast_flagged(
                    &mut b,
                    &payloads[0],
                    &mut path,
                    &mut leaves,
                    true,
                )?);
            }
            // Multiple payloads: the payload is a tuple handle — `arr-get(i)` into it, `via_sum_payload`.
            _ => {
                for (i, pty) in payloads.iter().enumerate() {
                    let mut path = vec![i as u32];
                    children.push(template_value_ast_flagged(
                        &mut b,
                        pty,
                        &mut path,
                        &mut leaves,
                        true,
                    )?);
                }
            }
        }
        b.list(children)
    };
    // The TYPE node — the sum's full type surface: a bare `Sign` for a monomorphic sum, `(Option
    // Int64)` for a generic instantiation (`type_ast`'s `Ty::Sum` arm renders both from the solved
    // type). So `(: (Some 5) (Option Int64))` — the corpus parameterized form.
    let type_node = type_ast(&mut b, sum_ty, &db.name_ctx())?;
    let root = b.list(vec![colon, value, type_node]);
    let arenas = b.finish(root);
    let bytes = crate::codec::encode(&arenas);
    let holes = resolve_leaf_offsets(&bytes, &arenas, &leaves)?;
    Some(ValueFormTemplate {
        bytes,
        leaves: holes,
    })
}

/// A leaf recorded during template construction, before its byte offset is resolved: its arena `LeafId`
/// (to locate it in the encoded pool) plus the runtime info the hole carries.
pub(super) struct PendingLeaf {
    leaf_id: crate::ast::LeafId,
    path: Vec<u32>,
    kind: LeafFill,
    /// Whether this leaf is reached through `sum-payload` first (a sum variant payload leaf) — carried
    /// onto the resolved [`RuntimeLeaf`]. `false` for a plain tuple/record leaf.
    via_sum_payload: bool,
}

/// Build the VALUE s-expression for a type with PLACEHOLDER leaves, recording each scalar leaf's walk
/// `path` (the `arr-get` indices to reach it) and kind. A tuple/record recurses, pushing the positional
/// index onto the path; a scalar emits a placeholder atom and records a `PendingLeaf`. `None` for a type
/// with no value surface.
pub(super) fn template_value_ast(
    b: &mut crate::ast::Builder,
    ty: &crate::ty::Ty,
    path: &mut Vec<u32>,
    out: &mut Vec<PendingLeaf>,
) -> Option<StructId> {
    template_value_ast_flagged(b, ty, path, out, false)
}

/// The core of [`template_value_ast`] with the `via_sum_payload` flag threaded onto each recorded leaf
/// — set when building a sum VARIANT PAYLOAD's sub-template (the leaves are reached through
/// `sum-payload` first). The flat tuple/record path passes `false`.
pub(super) fn template_value_ast_flagged(
    b: &mut crate::ast::Builder,
    ty: &crate::ty::Ty,
    path: &mut Vec<u32>,
    out: &mut Vec<PendingLeaf>,
    via_sum_payload: bool,
) -> Option<StructId> {
    use crate::ast::{Leaf, Radix};
    use crate::ty::Ty;
    match ty {
        Ty::Int(_) => {
            // Placeholder: a positive zero with a FIXED 8-byte magnitude, so the template reserves an
            // 8-byte hole (len=8) the runtime overwrites with the leaf's big-endian magnitude (a
            // non-minimal magnitude decodes fine — `BigInt::from_bytes_be` drops leading zeros). Pushed
            // NON-deduped (`leaf_unique`) so this occurrence has its OWN pool entry and hence its own
            // byte offset — two equal placeholders must not collapse to one hole.
            let leaf_id = b.leaf_unique(Leaf::Int {
                value: crate::ast::IntValue {
                    negative: false,
                    magnitude: vec![0u8; 8],
                },
                radix: Radix::Dec,
            });
            let atom = b.atom(leaf_id);
            out.push(PendingLeaf {
                leaf_id,
                path: path.clone(),
                kind: LeafFill::Int,
                via_sum_payload,
            });
            Some(atom)
        }
        Ty::Bool => {
            // Placeholder `false`; the runtime overwrites the kind byte (8=false / 9=true). Pushed
            // NON-deduped so each bool occurrence has its own pool entry + offset.
            let leaf_id = b.leaf_unique(Leaf::Bool(false));
            let atom = b.atom(leaf_id);
            out.push(PendingLeaf {
                leaf_id,
                path: path.clone(),
                kind: LeafFill::Bool,
                via_sum_payload,
            });
            Some(atom)
        }
        Ty::Tuple(elems) => {
            let head = b.name("tuple");
            let mut children = vec![head];
            for (i, e) in elems.iter().enumerate() {
                path.push(i as u32);
                children.push(template_value_ast_flagged(
                    b,
                    e,
                    path,
                    out,
                    via_sum_payload,
                )?);
                path.pop();
            }
            Some(b.list(children))
        }
        Ty::Record(fields) => {
            let head = b.name("record");
            let mut children = vec![head];
            // A record is a positional heap array in canonical (sorted) field order — the same order the
            // BTreeMap iterates, so the `arr-get` index is the field's position in that order.
            for (i, (name, t)) in fields.iter().enumerate() {
                // `(= name value)` ascription form (record-type Phase B full-symmetry migration —
                // literals, patterns, AND value-output all spell `(= name value)`; operator-ruled
                // 2026-08-09). Mirrors the runtime `value_encode` + rust `cdz_render` record renders.
                //
                // Intern the FieldPair leaf FIRST (before the key/value), NOT via `b.field_pair` (which
                // interns it last). The value-form template is `codec::encode`d, which CANONICALIZES under
                // std — and canonical leaf-pool order is TRAVERSAL order (head-first: `= `, then key, then
                // value). Building the entry head-first keeps the arena already-canonical, so `encode`'s
                // canonicalization is a no-op and the hole offsets (`resolve_leaf_offsets`, computed on this
                // arena's pool) match the encoded bytes. Building it with `b.field_pair` interned the `=`
                // leaf AFTER the key/value → non-canonical order → canon reordered the pool → an Int hole's
                // offset landed one byte early (on the leaf's LEN byte), corrupting the filled bytes so they
                // failed to decode (`template_fills_a_runtime_record`; the #5158 canon-in-tests residue).
                let fp_head = b.atom_leaf(crate::ast::Leaf::FieldPair);
                let fname = b.name(&*name.name);
                path.push(i as u32);
                let fval = template_value_ast_flagged(b, t, path, out, via_sum_payload)?;
                path.pop();
                children.push(b.list(vec![fp_head, fname, fval]));
            }
            Some(b.list(children))
        }
        // A RUNTIME QUANTITY renders its construction form `((. Qty of) <inner-hole> <unit>)` — the SAME
        // surface the CONSTANT path bakes (`const_value_ast`'s Qty arm), but with the inner magnitude left
        // as a RUNTIME HOLE instead of a baked constant. The unit is a COMPILE-TIME constant baked into the
        // template (units are compile-time-only — the operator ruling: a Qty erases to its bare inner scalar
        // at runtime, zero runtime cost, no runtime unit tracking; the label is injected here at compile
        // time from the SOLVED `Ty::Qty`). The erased inner scalar is boxed on the heap by the `make` body
        // (`EscapeForm::FlatScalar`'s `box-int` before `resource-new`), so the walker reads its `get-int`
        // hole off the resulting root handle exactly like a plain Int leaf reached at an empty path.
        // SCOPED (slices 1+2): a REFERENCE unit (scale 1/1) over ANY-width Int inner. A non-reference unit
        // needs a compile-time SCALE MULTIPLY before the scalar crosses (the flat leaf-hole template can't
        // express it) and a non-Int inner (Float) needs `LeafFill::Float` — both decline here (return
        // `None`), falling back to today's bare-scalar cross, until their slices land. A NARROW int (8/16/32)
        // is fine here: its magnitude hole is width-agnostic (8-byte, like any Int leaf), the width lives in
        // the baked type annotation, and the i32→i64 extend a narrow scalar needs before `box-int` is emitted
        // in the `make` body (`EscapeForm::FlatScalar { extend }`, computed by `emit_runtime_resource`).
        Ty::Qty { inner, unit } => {
            let (num, den) = unit.scale();
            if (num, den) != (1, 1) {
                return None;
            }
            if !matches!(inner.as_ref(), Ty::Int(_)) {
                return None;
            }
            // Build the list HEAD (`(. Qty of)`) BEFORE the inner-magnitude hole + unit, same head-first
            // discipline as the RECORD arm above (and for the same reason): the value-form template is
            // `codec::encode`d, which CANONICALIZES the leaf pool into traversal (head-first pre-order); the
            // runtime hole's byte offset (`resolve_leaf_offsets`) is measured on THIS arena's pre-canon pool,
            // so the pool must already be canonical or the offset drifts. Building `qty_of` LAST left the
            // magnitude Int hole at pool index ~1 while canon moved it AFTER the head's leaves → the runtime
            // magnitude write landed at the wrong offset, corrupting a struct child-id → the reader's
            // `cdzast-decode-error IdOutOfRange` on `(: (Qty.of N unit) …)` under --guarded-all. Interning
            // the head's leaves first makes the magnitude hole share the SAME preceding-leaf set in build and
            // canonical order (the byte sum is order-invariant over that set), so the offset stays valid.
            let qty_of = b.name("Qty.of");
            let inner_hole = template_value_ast_flagged(b, inner, path, out, via_sum_payload)?;
            let unit_ast = unit_value_ast(b, &unit.at_reference());
            Some(b.list(vec![qty_of, inner_hole, unit_ast]))
        }
        // A NOMINAL newtype's runtime value IS its erased inner's value (the box adds nothing — `type-
        // system.md §156`). Its VALUE form is therefore the INNER's value hole VERBATIM (a bare `5`, no
        // ctor wrapper), while its TYPE surface (`type_ast`, applied at the value-form root) renders the
        // NOMINAL NAME. So `(type W (Mk Int64))`'s runtime `(Mk k)` escape renders `(: 5 W)` — matching the
        // NULLARY constant path (`const_value_ast`), which already bakes the nominal name. Without this arm
        // `template_value_ast` returned `None` for a `Ty::Nominal`, so `mod.rs`'s escape router passed the
        // STRIPPED inner (`Ty::Int`) to build the template → the value form rendered `(: 5 Int64)`, dropping
        // the declared nominal (adv-64, a value-form regression from the adv-63b crash fix: the crash is
        // gone but the rendering lost the tag). Recurse on the inner for the value; the wrapper is erased.
        Ty::Nominal { inner, .. } => {
            template_value_ast_flagged(b, inner, path, out, via_sum_payload)
        }
        _ => None,
    }
}

/// Resolve each pending leaf's BYTE OFFSET in the encoded template. Re-encodes the leaf pool the same
/// way `codec::encode` does (header + count, then each leaf), tracking the running offset; when a leaf's
/// `LeafId` matches a pending runtime leaf, its hole offset is the magnitude position (Int: after the
/// kind + len bytes) or the kind-byte position (Bool). Returns the resolved holes in the pending order.
pub(super) fn resolve_leaf_offsets(
    bytes: &[u8],
    arenas: &crate::ast::Arenas,
    pending: &[PendingLeaf],
) -> Option<Vec<RuntimeLeaf>> {
    // Offset walk mirrors `codec::encode`: 8-byte header, then a LEB128 leaf-count, then each leaf.
    let mut off = 8usize;
    off += leb_len(arenas.leaves.len() as u64);
    // Map each LeafId → (magnitude offset for Int, kind-byte offset for Bool).
    let mut leaf_off: std::collections::HashMap<u32, (usize, LeafFill)> =
        std::collections::HashMap::new();
    for (i, leaf) in arenas.leaves.iter().enumerate() {
        let kind_off = off;
        match leaf {
            crate::ast::Leaf::Int { value, .. } => {
                // kind byte (1) + len LEB + magnitude.
                let len = value.magnitude.len();
                let mag_off = off + 1 + leb_len(len as u64);
                leaf_off.insert(i as u32, (mag_off, LeafFill::Int));
                off = mag_off + len;
            }
            crate::ast::Leaf::Bool(_) => {
                leaf_off.insert(i as u32, (kind_off, LeafFill::Bool));
                off += 1;
            }
            crate::ast::Leaf::Name(n) => {
                off += 1 + leb_len(n.len() as u64) + n.len();
            }
            crate::ast::Leaf::Str(s) => {
                off += 1 + leb_len(s.len() as u64) + s.len();
            }
            // A symbol leaf encodes like a Str/Name (kind byte + len LEB + utf8 bytes) and is compile-
            // time-only (a unit erases before the boundary — a symbol never reaches a runtime value
            // form), so advance past it with no runtime hole.
            crate::ast::Leaf::Sym(s) => {
                off += 1 + leb_len(s.len() as u64) + s.len();
            }
            // A bytes leaf is a fully-baked constant (no runtime hole) — advance past it like a Str
            // (kind byte + len LEB + the raw bytes).
            crate::ast::Leaf::Bytes(bs) => {
                off += 1 + leb_len(bs.len() as u64) + bs.len();
            }
            // No float (finite or non-finite NaN/±∞) is in the runtime escape yet — bail the template.
            crate::ast::Leaf::Float(_)
            | crate::ast::Leaf::FloatNan
            | crate::ast::Leaf::FloatInf { .. } => return None,
            // A char leaf encodes like a Str (kind byte + len LEB + utf8 bytes); a char does not yet
            // cross the boundary in the runtime escape, so advance past it (no runtime hole).
            crate::ast::Leaf::Char(c) => {
                off += 1 + leb_len(c.len_utf8() as u64) + c.len_utf8();
            }
            // A bad-escape / bad-char marker is a POISON — it never reaches a constant value form
            // (resolving it rejects CDZ0001/CDZ0002 before any escape emission), so a runtime template
            // over it is meaningless.
            crate::ast::Leaf::BadEscape(_) | crate::ast::Leaf::BadChar(_) => return None,
            // A type-suffixed numeric literal (`100N`/`0.5R`) is a SYNTAX-side leaf the codec decodes to a
            // plain Int/Float before the compiler sees it, so it never reaches a decoded runtime template;
            // bail conservatively (like the Float arm) for enum exhaustiveness.
            crate::ast::Leaf::Suffixed { .. } => return None,
            // A native-compound-data CTOR-HEAD leaf (`Leaf::Ctor`/`FieldPair`/`Member`) is payloadless —
            // one kind byte, no body and no runtime hole (it is a fixed structural head, never a patched
            // value leaf; the runtime holes are the Int/Bool value leaves elsewhere in the template). Skip
            // its single byte, mirroring how the old Str/Name head was skipped (just one byte now).
            crate::ast::Leaf::Ctor(_)
            | crate::ast::Leaf::FieldPair
            | crate::ast::Leaf::Member
            | crate::ast::Leaf::Rational => {
                off += 1;
            }
        }
    }
    let _ = bytes;
    let mut holes = Vec::with_capacity(pending.len());
    for p in pending {
        let (offset, _) = leaf_off.get(&p.leaf_id.0)?;
        holes.push(RuntimeLeaf {
            offset: *offset,
            path: p.path.clone(),
            kind: p.kind,
            via_sum_payload: p.via_sum_payload,
        });
    }
    Some(holes)
}

/// The number of bytes the unsigned LEB128 encoding of `n` occupies (matches `encode::uleb128`).
pub(super) fn leb_len(mut n: u64) -> usize {
    let mut c = 1;
    while n >= 0x80 {
        n >>= 7;
        c += 1;
    }
    c
}

/// Build the variant HEAD s-expression for variant `disc` of the sum declared at `decl`, as it appears
/// in an observed value's canonical form: the variant's BARE NAME atom — `Some`, `Sm`, `Cons`, `Pos`. A
/// variant renders the SAME whether its sum is BUILT-IN (Option/Result) or USER-declared: the value form
/// of a variant does not depend on where its sum was declared (the built-in-vs-user split that rendered a
/// user variant as the member-access `(. Type Variant)` while a built-in rendered bare was an
/// inconsistency — a rendered VALUE should be a variant name, not a projection expression). The rendered
/// value is always annotated with its sum type (`(: (Sm 42) Opt)`), which disambiguates a bare variant
/// name shared across sums (sum identity is by declaration occurrence, carried by the annotation). `None`
/// if the disc is out of range (a compiler bug). Shared by the constant-escape bake and the
/// runtime-escape template so both write the identical head.
// `pub(crate)` so the Cadenza backend (`backend::cadenza`) re-emits a runtime `Core::SumNew`'s variant
// head with the SAME bare-vs-qualified spelling lower's value surface uses — sharing this keeps the
// re-emitted variant head re-readable (a bare name that collides with a non-ctor prelude binding must be
// qualified, exactly as here).
pub(crate) fn variant_head_ast(
    db: &mut Db,
    b: &mut crate::ast::Builder,
    decl: StructId,
    disc: u32,
) -> Option<StructId> {
    let t = db.type_decl_by_occ(decl)?;
    let tname = t.name.clone();
    let vname = t.variants.get(disc as usize)?.name.clone();
    // A variant head normally renders BARE (`Some`, `Cons`, `Neg`) — the value reads back because the
    // bare name resolves to that variant. But when a variant name is SHADOWED by a prelude entry that is
    // NOT a variant ctor (`Ast.Int`/`Ast.List` — `Int` is the integer type ctor, `List` the list
    // module), a bare head would read back as that other binding, not the variant, so the value form
    // would not round-trip. Such a sum renders EVERY head QUALIFIED `(. Type Variant)` (a consistent
    // per-sum spelling, so mixed variants don't split): the member access resolves unambiguously to the
    // variant. This is the render-side twin of the load-time `variant_ctor_index` prelude-collision skip
    // (`db.rs`) — the same rule (don't let a colliding variant name masquerade as its prelude binding),
    // applied to the escaping VALUE FORM. `Some`/`None` are in the prelude too, but bound to their OWN
    // variant ctors, so they round-trip bare and are NOT qualified.
    if sum_needs_qualified_heads(db, decl) {
        let dot = b.name(".");
        let ty_name = b.name(tname);
        let var_name = b.name(vname);
        return Some(b.list(vec![dot, ty_name, var_name]));
    }
    Some(b.name(vname))
}

/// Whether the sum declared at `decl` must render its variant heads QUALIFIED (see [`variant_head_ast`]):
/// true iff ANY variant name is bound in the prelude to something that is NOT a variant ctor (a type
/// ctor, a module, a value). A per-sum property (not per-variant) so every head of the sum spells the
/// same way. A variant whose prelude binding IS a variant ctor (`Some`/`None`/`Ok`/`Err`) round-trips
/// bare, so it does not force qualification; a variant name absent from the prelude (`Cons`, `Neg`)
/// likewise resolves bare to its own ctor.
pub(crate) fn sum_needs_qualified_heads(db: &mut Db, decl: StructId) -> bool {
    let Some(t) = db.type_decl_by_occ(decl) else {
        return false;
    };
    let names: Vec<String> = t.variants.iter().map(|v| v.name.clone()).collect();
    names
        .iter()
        .any(|name| match db.prelude.get(name).copied() {
            // Bound in the prelude to a non-variant-ctor (no `(meta variant)`) → bare would resolve to
            // that OTHER binding, so the whole sum must qualify.
            Some(occ) => crate::eval::variant_disc_of(db, occ).is_none(),
            // Not a prelude name → bare resolves to this sum's own variant ctor; no qualification needed.
            None => false,
        })
}

/// Reconstruct the VALUE s-expression of a constant node into `b`: a scalar → its literal atom; a
/// `Core::Tuple` → `(tuple <elem>…)`; a `Core::Record` → `(record (<name> <value>)…)` in canonical field
/// order. `None` if the node is not a constant the escape path can bake.
pub(super) fn const_value_ast(
    db: &mut Db,
    b: &mut crate::ast::Builder,
    id: StructId,
) -> Option<StructId> {
    use crate::ast::{Leaf, Radix};
    // A QUANTITY value renders its CONSTRUCTION form `(Qty.of <inner-value> <unit>)` — the unit is a
    // compile-time value that erased from the core (a `Qty.of` node lowers to its inner value's core),
    // so it is recovered from the SOLVED TYPE `Ty::Qty` and re-materialized as source structure. This is
    // checked FIRST (before the core match) because the erased core is a bare scalar (`ConstFloat`),
    // which would otherwise render as the bare number, losing the quantity the corpus records.
    if let crate::ty::Ty::Qty { inner, unit } = crate::infer::type_of(db, id) {
        // A quantity DISPLAYS at its dimension's REFERENCE unit, with its magnitude SCALED to that
        // reference — the same normalize-to-reference the mixed-unit combine path runs, so a single
        // quantity and a homogeneous combine render identically (`5 kilometer` and `5 km + 0 m` both
        // → `(Qty.of 5000.0 (Unit.base #"meter"))`). This is the fix for the calc relabel bug
        // (mlrepro-calc-bare-quantity-relabels-to-base-without-scaling): the OLD render took the unit's
        // exponent-map NAME (`meter`) but dropped its SCALE (`1000`), so the number (`5`) and unit
        // (`meter`) disagreed. Scaling here is a DISPLAY concern only — construction is untouched, so no
        // stored value is truncated (DESIGN-quantity-reference-normalized-unwrap.md §1a REVISED: lazy,
        // not eager). `Unit.in` still converts by the exact direct source→target ratio (unaffected — it
        // renders a bare number, not a Qty). The scale is applied in the inner numeric type: Float
        // rounds, Rational is exact, Int truncates on a non-whole ratio — the same rule the combine path
        // and the numeric core already use (a scale-1 reference unit is a no-op, byte-neutral).
        //= spec/capabilities/units-of-measure.md#a-stored-quantity-displays-at-its-dimension-s-reference-unit
        //# A quantity stored at a scaled or named unit MUST, when it crosses the machine boundary as a value, display with its magnitude scaled to its dimension's reference unit and its unit shown as that reference, so that the number and the unit agree rather than disagreeing — a `5 kilometer` quantity displays as `5000 meter`, not the misleading `5 meter` that names the reference unit while keeping the source magnitude.
        //= spec/capabilities/units-of-measure.md#a-stored-quantity-displays-at-its-dimension-s-reference-unit
        //# Scaling to the reference unit MUST be a display concern only and MUST NOT alter the stored magnitude, so that `Qty.value` returns the number the source wrote in the unit the source named, and an explicit `as`/`in` conversion computes by the exact direct ratio off that stored magnitude with no intermediate reference rounding.
        let (num, den) = unit.scale();
        let inner_val = const_value_ast_scaled(db, b, id, &inner, num, den)?;
        let unit_ast = unit_value_ast(b, &unit.at_reference());
        // `((. Qty of) <value> <unit>)` — the member-access form the reader normalizes `(Qty.of …)` to
        // (a dotted name `Qty.of` desugars to `(. Qty of)`), so the baked value re-reads/re-prints to
        // the SAME canonical shape the corpus records.
        let qty_of = b.name("Qty.of");
        return Some(b.list(vec![qty_of, inner_val, unit_ast]));
    }
    // A SYMBOL value renders its CONSTRUCTION form `((. Symbol of) "text")` (17-symbols "a symbol is
    // constructed from a string"), NOT the bare string its `Core::ConstStr` rep would otherwise render.
    // A symbol shares the constant-string rep (its identity is its text — see `Resolved::SymbolConst`),
    // so the core is a `ConstStr`; recover the SYMBOL surface from the SOLVED TYPE `Ty::Symbol` and
    // re-materialize the `Symbol.of` construction as source, exactly as `Ty::Qty` recovers `Qty.of`.
    // Checked FIRST (before the core match) since the erased core would otherwise render as a bare String.
    if matches!(crate::infer::type_of(db, id), crate::ty::Ty::Symbol)
        && let Core::ConstStr(s) = core_of(db, id)
    {
        let symbol_of = member_access(b, "Symbol", "of");
        let text = b.atom_leaf(Leaf::Str(s));
        return Some(b.list(vec![symbol_of, text]));
    }
    // A TYPE-VALUE renders its concrete type's NAME surface — `(: Int64 Type)`, whose VALUE node is the
    // atom `Int64` (the type node is `Type`, built by `type_ast`'s `Ty::Type` arm). A type is a first-class
    // value that can be returned and inspected at run time (core-semantics.md §Types Are First-Class
    // Values), and it is FULLY compile-time-known, so its boundary form is baked from the reduced type
    // rather than a runtime representation — recover the concrete `Ty` with `typeval_of` (the same reducer
    // `Type.of`/`Type.eq`/an annotation position use) and render its type surface as the value node.
    // Checked FIRST (before the core match) because the erased core of a type expression is not a scalar.
    if matches!(crate::infer::type_of(db, id), crate::ty::Ty::Type)
        && let Some(concrete) = crate::eval::typeval_of(db, id)
    {
        return type_ast(b, &concrete, &db.name_ctx());
    }
    match core_of(db, id) {
        Core::ConstInt(v) => Some(b.atom_leaf(Leaf::Int {
            value: v,
            radix: Radix::Dec,
        })),
        // A constant RATIONAL bakes as the native rational NODE (seq-204): a payloadless `Leaf::Rational`
        // tag head + two ordinary `Leaf::Int` children (numerator, denominator) — the SAME structure the
        // runtime `value-encode` (op62) emits, so a CONST rational and a RUNTIME rational of the same value
        // are BYTE-IDENTICAL (one content-address; the operator's binary-AST canonical-exchange mandate).
        // The pair is already normalized (lowest terms, sign on the numerator, denom > 0). The tag is
        // interned FIRST to match op62's node leaf order (pre-order); v-runtime confirms the byte-match.
        Core::ConstRational(n, d) => {
            let tag = b.atom_leaf(Leaf::Rational);
            let num = b.atom_leaf(Leaf::Int {
                value: n,
                radix: Radix::Dec,
            });
            let den = b.atom_leaf(Leaf::Int {
                value: d,
                radix: Radix::Dec,
            });
            Some(b.list(vec![tag, num, den]))
        }
        // A constant float bakes as its exact decimal leaf — the codec encodes it (KIND_FLOAT), and the
        // host reader renders it back. A quantity over a Float64 magnitude reaches here through
        // `const_value_ast_at` (the inner-value render of `Qty.of`).
        Core::ConstFloat(d) => {
            // RE-CANONICALIZE via `from_f64`: the stored `Decimal` carries the SOURCE LITERAL's digit form,
            // but the canonical value form is the FULL exact expansion (scalar display_float + rust + the
            // runtime `float_leaf`), so a CONST compound-element float renders identically to a runtime one.
            let mut canon =
                crate::ast::Decimal::from_f64(f64::from_bits(d.to_f64_bits())).unwrap_or(d);
            // DEMOTE a Float32-typed constant through binary32 before baking, mirroring the runtime
            // `Core::ConstFloat` emit (`… as f32`), the float-arith/compare const folds, and the Rust
            // backend. Without this a const-folded `(: 0.1 Float32)` value (e.g. `(List.at (list (: 0.1
            // Float32)) 0)`, which folds to `(Some 0.1)` and bakes its value form here) keeps the UN-demoted
            // f64 0.1 — a wasm-vs-rust VALUE differential (rust demotes; witness rcdzc-wasm-list-float32-
            // element-not-demoted). A `Float64`/default-width const is byte-identical (the guard is false).
            if let crate::ty::Ty::Float(ft) = crate::infer::type_of(db, id)
                && ft.ground_width() == 32
            {
                // Render the SHORTEST-F32 decimal (`from_f32` = `{:e}` on the binary32), NOT the
                // f32→f64-PROMOTED shortest-f64: `from_f64(… as f32 as f64)` rendered `28.29` as
                // `28.290000915527344` (the shortest f64 of the promoted value — a DIFFERENT number;
                // operator ruling: "those are different values entirely"). Demote the VALUE to binary32
                // then render the f32's own shortest, mirroring the wasm value_codec `float32_leaf`, the
                // Rust backend's `from_f32`, and the runtime `float_leaf` — the one-canonical-render
                // (seq-283): a const / bare Float32 now displays identically to a runtime-heap one.
                let f32_val = f64::from_bits(canon.to_f64_bits()) as f32;
                canon = crate::ast::Decimal::from_f32(f32_val).unwrap_or(canon);
            }
            Some(b.atom_leaf(Leaf::Float(canon)))
        }
        Core::ConstBool(x) => Some(b.atom_leaf(Leaf::Bool(x))),
        // A constant string bakes as its `"…"` leaf — the codec encodes it (KIND_STR: len + UTF-8
        // bytes), and the host reader lifts it back to a string value.
        Core::ConstStr(s) => Some(b.atom_leaf(Leaf::Str(s))),
        // A constant char bakes as its `#\c` leaf — the codec encodes it (KIND_CHAR), and the host reader
        // renders it `#\c`. This lets a constant `(Some #\a)` (a `Char.from-int` fold) cross the boundary.
        Core::ConstChar(c) => Some(b.atom_leaf(Leaf::Char(c))),
        // The unit value bakes as the `unit` name leaf — ONE canonical byte form, distinct from every
        // other value's form (no other value renders as the bare `unit` name), so a program that produces
        // only its emitted events still has a serializable normal-termination value.
        //= spec/contracts/deterministic-value-form.md#the-unit-value-has-a-canonical-byte-form
        //# The unit value MUST have exactly one canonical byte encoding, so that a program that produces no value other than its emitted events has a serializable normal-termination value.
        //= spec/contracts/deterministic-value-form.md#the-unit-value-has-a-canonical-byte-form
        //# The canonical byte encoding of the unit value MUST be distinct from that of every other value, consistent with structural equality treating the unit value as equal only to itself.
        Core::Unit => Some(b.name("unit")),
        // Each compound element recurses through `const_value_ast`, whose Qty arm (above) scales any
        // quantity leaf to its dimension's reference unit in its own inner type — so a tuple/record/sum
        // payload/nested-compound carrying a quantity displays at reference independently, not only a bare
        // top-level quantity.
        //= spec/capabilities/units-of-measure.md#a-stored-quantity-displays-at-its-dimension-s-reference-unit
        //# The reference-unit display MUST recurse into every quantity leaf of a compound value, so that a tuple, a sum payload, a nested compound, or a record field carrying a quantity each displays scaled to its reference independently and in its own inner type, not only a bare top-level quantity.
        Core::Tuple { elems } => {
            // M2: head-first native TUPLE_CTOR head (recognized by kind), not the `tuple` name head.
            let mut children = Vec::with_capacity(elems.len());
            for e in elems.iter().copied() {
                children.push(const_value_ast(db, b, e)?);
            }
            Some(b.compound(crate::ast::CompoundCtor::Tuple, &children))
        }
        Core::Record { fields } => {
            // M2: head-first native RECORD_CTOR head; fields are FieldPair-leaf `(= k v)` (b.field_pair).
            let mut children = Vec::with_capacity(fields.len());
            // Canonical (sorted) field order — a `BTreeMap` iterates sorted, matching the type render.
            for (name, &v) in fields.iter() {
                let fname = b.name(&*name.name);
                let fval = const_value_ast(db, b, v)?;
                // `(= name value)` ascription form (record-type Phase B full-symmetry migration —
                // literals, patterns, AND value-output all spell `(= name value)`; operator-ruled
                // 2026-08-09). Distinct from a map's `(key value)` pairs, which stay pair-form.
                children.push(b.field_pair(fname, fval));
            }
            Some(b.compound(crate::ast::CompoundCtor::Record, &children))
        }
        // A CONSTANT list literal renders `(list e1 e2 …)` — its length is statically known (unlike a
        // grown/runtime list), so its bytes bake exactly like a constant tuple's. Each element is a
        // constant in turn (a non-constant element makes the whole value non-constant, so `core_of` would
        // not be a `ListNew` of constants and this returns `None`, declining the escape). A list is an
        // ORDERED aggregate — the render walks `elems` in order, so the canonical form preserves element
        // order (unlike the map/set render, which sorts an unordered aggregate).
        //= spec/contracts/deterministic-value-form.md#ordering-of-aggregate-members-is-fixed
        //# The canonical encoding of an ordered aggregate MUST preserve its element order.
        Core::ListNew { elems } => {
            // M2: head-first native LIST_CTOR head, not the `list` name head.
            let mut children = Vec::with_capacity(elems.len());
            for e in elems.iter().copied() {
                children.push(const_value_ast(db, b, e)?);
            }
            Some(b.compound(crate::ast::CompoundCtor::List, &children))
        }
        // A CONSTANT map value — `(map (k1 v1) (k2 v2) …)` — its entries rendered in CANONICAL KEY ORDER,
        // independent of insertion order and DISTINGUISHABLE from a record (`map` head, `(key value)`
        // pairs). The constant map already has each key at most once (the `Map.insert` fold replaced by
        // key value); sort the entries by their canonical KEY order (`const_key_order`), then render each
        // pair. A non-constant key/value makes an entry non-constant → `None`, declining the escape (a
        // genuinely runtime map's escape is the deferred looping walker). This is the constant-escape (R1)
        // companion of the map value — a fully-constant map crosses by baked bytes here.
        //= spec/contracts/deterministic-value-form.md#ordering-of-aggregate-members-is-fixed
        //# The canonical encoding of an unordered aggregate MUST place its members in a fixed order derived from the members themselves, not from the order in which they were inserted or discovered.
        //= spec/capabilities/collections-and-text.md#a-map-renders-as-its-entries-in-canonical-key-order
        //# A map's canonical form MUST present its entries as key-value pairs in the deterministic order of *Map Iteration Is Deterministic*, so that two equal maps have identical canonical forms regardless of the order their entries were added.
        //= spec/capabilities/collections-and-text.md#a-map-renders-as-its-entries-in-canonical-key-order
        //# The canonical form MUST be distinguishable from a record's, so that a map and a record are never confused by their rendered form even when they carry the same keys and values (a map's keys are values of one key type; a record's field names are fixed compile-time labels).
        // The canonical render is the only entry-visiting the seed exposes for a map, and it sorts by
        // `const_key_order` — a deterministic order derived from the KEYS, not insertion order — which is
        // exactly the order the canonical byte form places them in (this render IS that byte form).
        //= spec/capabilities/collections-and-text.md#map-iteration-is-deterministic
        //# Iterating a map MUST visit its entries in a deterministic order derived from the keys, not from insertion order.
        //= spec/capabilities/collections-and-text.md#map-iteration-is-deterministic
        //# The order in which a map's entries are visited MUST agree with the order its canonical byte form places them in.
        Core::MapNew { entries, .. } => {
            let mut sorted: Vec<(StructId, StructId)> = entries.to_vec();
            // Sort by canonical key order. A key that is not orderable-as-a-constant declines the whole
            // escape (the runtime walker path is deferred), so a failed comparison bails to `None`.
            let mut orderable = true;
            sorted.sort_by(|a, b| {
                const_key_order(db, a.0, b.0).unwrap_or_else(|| {
                    orderable = false;
                    std::cmp::Ordering::Equal
                })
            });
            if !orderable {
                return None;
            }
            // M2: head-first native MAP_CTOR head; entries are FieldPair-leaf `(= k v)` (map=field-pair
            // unification), not the legacy `(k v)` raw pair.
            let mut children = Vec::with_capacity(sorted.len());
            for (k, v) in sorted {
                let kv = const_value_ast(db, b, k)?;
                let vv = const_value_ast(db, b, v)?;
                children.push(b.field_pair(kv, vv));
            }
            Some(b.compound(crate::ast::CompoundCtor::Map, &children))
        }
        // A CONSTANT set value — `(Set.of (list e1 e2 …))` — its elements rendered in CANONICAL (sorted)
        // ORDER inside a `(list …)`, wrapped in a `(Set.of …)` form (collections-and-text.md §A Set …
        // canonical written form is `(Set.of (list …))`). The constant set already has each element at
        // most once (the `Set.of`/insert folds dedup by value); sort by `const_key_order` (reused — an
        // element orders exactly like a map key). A non-orderable element declines the escape.
        // This sorted render is the only element-visiting the seed exposes for a set: a deterministic order
        // derived from the ELEMENTS (not insertion order), agreeing with the canonical byte form it builds.
        //= spec/capabilities/collections-and-text.md#set-iteration-is-deterministic
        //# Iterating a set MUST visit its elements in a deterministic order derived from the elements, not from insertion order.
        //= spec/capabilities/collections-and-text.md#set-iteration-is-deterministic
        //# The order in which a set's elements are visited MUST agree with the order its canonical byte form places them in.
        Core::SetOf { elems, .. } => {
            let mut sorted: Vec<StructId> = elems.to_vec();
            let mut orderable = true;
            sorted.sort_by(|&x, &y| {
                const_key_order(db, x, y).unwrap_or_else(|| {
                    orderable = false;
                    std::cmp::Ordering::Equal
                })
            });
            if !orderable {
                return None;
            }
            // M2: head-first native SET_CTOR head with the sorted-deduped elements directly — the
            // `(. Set of)(list …)` member-path is REPLACED (operator: a set renders `#set(…)` natively).
            let mut children = Vec::with_capacity(sorted.len());
            for e in sorted {
                children.push(const_value_ast(db, b, e)?);
            }
            Some(b.compound(crate::ast::CompoundCtor::Set, &children))
        }
        // A CONSTANT sum value — `(Some 5)`, `(None unit)`, `(Some (Some 5))`. Its canonical form is
        // `(VariantName payload…)` with the variant TAG present (`deterministic-value-form.md`;
        // core-semantics.md §A Constructor Applied To An Argument Is A Sum Value). This holds regardless
        // of what the payload IS — a scalar, a tuple, or ANOTHER sum value — so a NESTED constant sum
        // (`(Some (Some 5))`) bakes recursively, both variant tags present. This is the constant-escape
        // (R1) companion of `sum_form_template`'s runtime walker: a fully-constant sum crosses by baked
        // bytes here, so it never needs the per-variant runtime template (which cannot express a nested
        // sum's variable-length inner shape). The variant NAME is recovered from the disc against this
        // node's solved sum type (its declaration's variant set); a nullary variant carries `unit`.
        Core::SumNew { disc, payloads } => {
            let ty = crate::infer::type_of(db, id);
            let crate::ty::Ty::Sum { decl, .. } = ty else {
                return None; // a SumNew whose solved type is not a sum is a compiler bug — decline
            };
            let head = variant_head_ast(db, b, decl, disc)?;
            let mut children = vec![head];
            match payloads.len() {
                // Nullary variant: `(VariantName unit)` — the corpus form (`(None unit)`).
                0 => children.push(b.name("unit")),
                // Single payload (the canonical variant shape — one payload type, a scalar / tuple /
                // nested sum): render it recursively.
                1 => children.push(const_value_ast(db, b, payloads[0])?),
                // Multiple application arguments (a `(V.Both a b)` multi-arg surface) — not a canonical
                // single-payload form; the escape declines rather than guess a rendering.
                _ => return None,
            }
            Some(b.list(children))
        }
        // A constant `Bytes.of` → a `Leaf::Bytes` value node (rendered `b"…"` by the host). Each element
        // is a constant Int in `0..=255` (range-checked at `lower_bytes_of`); collect the raw bytes. A
        // non-constant element would have declined at `lower_bytes_of` (no `Core::BytesOf` built), so
        // every element here folds to a `ConstInt` in range.
        Core::BytesOf { elems } => {
            let mut raw = Vec::with_capacity(elems.len());
            for e in elems.iter().copied() {
                match core_of(db, e) {
                    Core::ConstInt(v) => {
                        raw.push(v.to_i64().filter(|n| (0..=255).contains(n))? as u8)
                    }
                    _ => return None,
                }
            }
            Some(b.atom_leaf(Leaf::Bytes(raw.into())))
        }
        _ => None,
    }
}

/// Render the constant value at `id` treating it AS having type `expect` — the quantity-inner helper.
/// A `Qty.of` node erases (in `core_of`) to its inner value's core, so rendering the INNER value means
/// rendering that same core, but WITHOUT re-triggering the `Ty::Qty` branch of `const_value_ast` (which
/// reads `type_of(id)` = the whole quantity type). `expect` is the inner numeric type; for a scalar
/// (int/float/bool) the value form is the same as `const_value_ast`'s scalar arms, so match the core
/// directly here. (A quantity over a COMPOUND inner type is not a Layer-1 case — the numeric core is
/// scalar — so a non-scalar inner declines the escape by `None`.)
/// Render a quantity's magnitude SCALED to its dimension's reference unit — `value × (num/den)` in the
/// inner numeric type — for the value-form display of a `(Qty T u)` (`const_value_ast`'s Qty arm). The
/// scale `(num, den)` is the unit's exact ratio to its reference (`unit.scale()`); a reference unit is
/// `(1, 1)`, a no-op that delegates to the unscaled render. Mirrors the mixed-unit combine's constant
/// fold so a single quantity and a homogeneous combine display identically: Float multiplies then
/// rounds to a finite decimal, Rational scales exactly (cross-multiply + renormalize), Int multiplies
/// then truncates on a non-whole ratio (the numeric core's integer-division rule). Declines (None) on a
/// non-scalar inner or a Float scale with no finite decimal form.
pub(super) fn const_value_ast_scaled(
    db: &mut Db,
    b: &mut crate::ast::Builder,
    id: StructId,
    expect: &crate::ty::Ty,
    num: i128,
    den: i128,
) -> Option<StructId> {
    use crate::ast::{Leaf, Radix};
    // Reference unit (scale 1/1) — no scaling, render the magnitude as-is.
    if num == 1 && den == 1 {
        return const_value_ast_at(db, b, id, expect);
    }
    //= spec/capabilities/units-of-measure.md#a-stored-quantity-displays-at-its-dimension-s-reference-unit
    //# The display scale MUST be applied in the quantity's own inner numeric type, so that a Float rounds, a Rational stays exact, and an integer truncates toward zero on a non-whole ratio exactly as the numeric core's rules dictate — the dimensional layer introduces no arithmetic of its own beyond the source-denoted scale.
    match core_of(db, id) {
        Core::ConstInt(v) => {
            let scaled = v.to_i128()?.checked_mul(num)? / den;
            Some(b.atom_leaf(Leaf::Int {
                value: IntValue::from_i128(scaled),
                radix: Radix::Dec,
            }))
        }
        Core::ConstFloat(d) => {
            let scaled = f64::from_bits(d.to_f64_bits()) * (num as f64) / (den as f64);
            crate::ast::Decimal::from_f64(scaled).map(|dec| b.atom_leaf(Leaf::Float(dec)))
        }
        Core::ConstRational(n, dd) => {
            // Exact: (n/dd) × (num/den) = (n·num)/(dd·den), renormalized to lowest terms.
            let sn = n.mul(&IntValue::from_i128(num));
            let sd = dd.mul(&IntValue::from_i128(den));
            match normalized_rational(sn, sd) {
                Core::ConstRational(rn, rd) => {
                    // Native rational NODE (seq-204), byte-identical to op62 — tag head first, then the
                    // two normalized Int children (num, den). See the top-level `const_value_ast` arm.
                    let tag = b.atom_leaf(Leaf::Rational);
                    let num = b.atom_leaf(Leaf::Int {
                        value: rn,
                        radix: Radix::Dec,
                    });
                    let den = b.atom_leaf(Leaf::Int {
                        value: rd,
                        radix: Radix::Dec,
                    });
                    Some(b.list(vec![tag, num, den]))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

pub(super) fn const_value_ast_at(
    db: &mut Db,
    b: &mut crate::ast::Builder,
    id: StructId,
    expect: &crate::ty::Ty,
) -> Option<StructId> {
    use crate::ast::{Leaf, Radix};
    let _ = expect; // the inner is a scalar in Layer 1; the core discriminates directly
    match core_of(db, id) {
        Core::ConstInt(v) => Some(b.atom_leaf(Leaf::Int {
            value: v,
            radix: Radix::Dec,
        })),
        Core::ConstFloat(d) => {
            // RE-CANONICALIZE via `from_f64`: the stored `Decimal` carries the SOURCE LITERAL's digit form,
            // but the canonical value form is the FULL exact expansion (scalar display_float + rust + the
            // runtime `float_leaf`), so a CONST compound-element float renders identically to a runtime one.
            let mut canon =
                crate::ast::Decimal::from_f64(f64::from_bits(d.to_f64_bits())).unwrap_or(d);
            // DEMOTE a Float32-typed constant through binary32 before baking, mirroring the runtime
            // `Core::ConstFloat` emit (`… as f32`), the float-arith/compare const folds, and the Rust
            // backend. Without this a const-folded `(: 0.1 Float32)` value (e.g. `(List.at (list (: 0.1
            // Float32)) 0)`, which folds to `(Some 0.1)` and bakes its value form here) keeps the UN-demoted
            // f64 0.1 — a wasm-vs-rust VALUE differential (rust demotes; witness rcdzc-wasm-list-float32-
            // element-not-demoted). A `Float64`/default-width const is byte-identical (the guard is false).
            if let crate::ty::Ty::Float(ft) = crate::infer::type_of(db, id)
                && ft.ground_width() == 32
            {
                // Render the SHORTEST-F32 decimal (`from_f32` = `{:e}` on the binary32), NOT the
                // f32→f64-PROMOTED shortest-f64: `from_f64(… as f32 as f64)` rendered `28.29` as
                // `28.290000915527344` (the shortest f64 of the promoted value — a DIFFERENT number;
                // operator ruling: "those are different values entirely"). Demote the VALUE to binary32
                // then render the f32's own shortest, mirroring the wasm value_codec `float32_leaf`, the
                // Rust backend's `from_f32`, and the runtime `float_leaf` — the one-canonical-render
                // (seq-283): a const / bare Float32 now displays identically to a runtime-heap one.
                let f32_val = f64::from_bits(canon.to_f64_bits()) as f32;
                canon = crate::ast::Decimal::from_f32(f32_val).unwrap_or(canon);
            }
            Some(b.atom_leaf(Leaf::Float(canon)))
        }
        Core::ConstBool(x) => Some(b.atom_leaf(Leaf::Bool(x))),
        // A RATIONAL magnitude bakes as the native rational NODE (seq-204, same as the top-level
        // `const_value_ast` Rational arm), so a `(Qty Rational u)` value renders `(Qty.of n/d <unit>)`
        // byte-identical to op62 — tag head first, then the two normalized Int children.
        Core::ConstRational(n, d) => {
            let tag = b.atom_leaf(Leaf::Rational);
            let num = b.atom_leaf(Leaf::Int {
                value: n,
                radix: Radix::Dec,
            });
            let den = b.atom_leaf(Leaf::Int {
                value: d,
                radix: Radix::Dec,
            });
            Some(b.list(vec![tag, num, den]))
        }
        // A non-scalar inner value is not a Layer-1 quantity magnitude — decline the escape.
        _ => None,
    }
}

/// Materialize a compile-time `Unit` value as SOURCE structure — the `<unit>` position of a rendered
/// `(Qty.of <value> <unit>)` and of a `(Qty T <unit>)` type. The dimensionless unit renders `Unit.one`;
/// a single base to the first power renders `((. Unit base) #"name")`; a base to a power `(Unit.^ …
/// k)`; a product of positive factors a left-nested `(Unit.* …)`; and — crucially — a unit with
/// NEGATIVE exponents renders as a QUOTIENT `(Unit./ <numerator> <denominator>)`, the surface the corpus
/// records for a derived unit (`(Unit./ meter second)` for a velocity, NOT `(Unit.* meter (Unit.^ second
/// -1))`). The numerator is the positive-exponent factors (`Unit.one` if none); the denominator the
/// negative-exponent factors with their exponents made positive. Uses the `#"name"` SYMBOL leaf per base
/// so the rendered unit re-reads to the same `Unit`. `Unit.base` is member access; `Unit.^`/`Unit.*`/
/// `Unit./` stay BARE names (their segment is not alphabetic, so the reader does not desugar them).
pub(crate) fn unit_value_ast(b: &mut crate::ast::Builder, unit: &crate::ty::Unit) -> StructId {
    use crate::ast::Leaf;
    let entries: Vec<(String, i64)> = unit.entries().map(|(n, e)| (n.clone(), *e)).collect();
    if entries.is_empty() {
        // `Unit.one` — the dimensionless unit (bare dotted-name atom, printed verbatim → sugared).
        return b.name("Unit.one");
    }
    // One base factor at a (positive) exponent: `(Unit.base #"name")` or `(Unit.^ … k)` — the head is a
    // bare dotted-name atom (`Unit.base`), printed verbatim → sugared, matching the operator-symbol members
    // `Unit.^`/`Unit.*`/`Unit./` (seq-283 member-render consistency; re-reads to the same Leaf::Member).
    fn factor(b: &mut crate::ast::Builder, name: &str, exp: i64) -> StructId {
        let base_head = b.name("Unit.base");
        let sym = b.atom_leaf(Leaf::Sym(name.into()));
        let base = b.list(vec![base_head, sym]);
        if exp == 1 {
            base
        } else {
            let pow_head = b.name("Unit.^");
            let n = b.atom_leaf(Leaf::Int {
                value: crate::ast::IntValue::from_i64(exp),
                radix: crate::ast::Radix::Dec,
            });
            b.list(vec![pow_head, base, n])
        }
    }
    // Left-nested product of a factor list, or `Unit.one` when empty.
    fn product(b: &mut crate::ast::Builder, factors: &[(String, i64)]) -> StructId {
        if factors.is_empty() {
            return b.name("Unit.one");
        }
        let mut acc = factor(b, &factors[0].0, factors[0].1);
        for (name, exp) in &factors[1..] {
            let f = factor(b, name, *exp);
            let mul_head = b.name("Unit.*");
            acc = b.list(vec![mul_head, acc, f]);
        }
        acc
    }
    // Split into positive (numerator) and negative (denominator, exponents made positive) factors.
    let num: Vec<(String, i64)> = entries
        .iter()
        .filter(|(_, e)| *e > 0)
        .map(|(n, e)| (n.clone(), *e))
        .collect();
    let den: Vec<(String, i64)> = entries
        .iter()
        .filter(|(_, e)| *e < 0)
        .map(|(n, e)| (n.clone(), -*e))
        .collect();
    if den.is_empty() {
        // All positive — a plain product (or a single factor).
        return product(b, &num);
    }
    // A quotient `(Unit./ numerator denominator)` — the derived-unit surface.
    let numerator = product(b, &num);
    let denominator = product(b, &den);
    let div_head = b.name("Unit./");
    b.list(vec![div_head, numerator, denominator])
}
