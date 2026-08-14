//! §3c — reading a preparsed TARGET WIT WORLD (binary-AST) to drive emit-to-match.
//!
//! Per the compiler↔platform-separation end-state (`DESIGN-compiler-platform-separation.md` §3b, operator
//! override 2026-08-11), rcdzc emits a Cadenza program to a target WIT world by reading each member's
//! DECLARED canonical-ABI type and bridging (`value-encode`/`value-decode`) wherever the guest value-model
//! type differs. The world reaches rcdzc as a PREPARSED binary-AST artifact (from an external producer OR an
//! inline module declaration — both lower to the same structured world); **rcdzc never parses WIT text.**
//!
//! This module is the TYPE-DESCRIPTOR reader — it decodes one `build_type` descriptor occurrence into a
//! [`WitType`]. It matches `cdz-kernel::ast_marshal::build_type` EXACTLY (the shared type-node vocabulary):
//! a PRIMITIVE is a lone NAME-head marker `(u8)` / `(string)`; a COMPOUND is a STRING-head form —
//! `("list" <elem>)`, `("record" (fieldname <ty>)…)`, `("tuple" <ty>…)`. So `list<u8>` (a "byte-list", all
//! the reducer `apply` boundary needs) decodes to `WitType::List(U8)`.
//!
//! The WORLD-STRUCTURE reader (world → import/export interfaces → members → func → params/result) is added
//! once v-agent-harness formalizes the exact world node encoding (their lane, §3b); the vocabulary is locked
//! but its precise heads/nesting are theirs to pin, so this lands the grounded half now.

use crate::ast::{Arenas, Struct, StructId};
use crate::ty::Ty;

/// A canonical-ABI type as declared in a target WIT world, decoded from a `build_type` descriptor. The set
/// the reducer slice needs is the scalars + `List` (byte-list = `List(U8)`); the remaining component-model
/// types (variant / enum / option / result / flags / own+borrow handles / unit) are added as later full-A
/// slices need them — [`parse_wit_type`] returns `None` for a not-yet-covered descriptor rather than
/// guessing.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum WitType {
    Bool,
    U8,
    U16,
    U32,
    U64,
    S8,
    S16,
    S32,
    S64,
    Char,
    String,
    F32,
    F64,
    /// `list<T>` — a byte-list is `List(Box::new(U8))`.
    List(Box<WitType>),
    /// `record { name: T, … }` in declaration order.
    Record(Vec<(String, WitType)>),
    /// `tuple<T, …>`, positional.
    Tuple(Vec<WitType>),
    /// `option<T>`.
    Option(Box<WitType>),
    /// `unit` — the payload-less / void marker (a member's `result` is `unit` for a no-return func).
    Unit,
}

/// Decode one `build_type` descriptor occurrence `id` in `a` into its [`WitType`]. `None` if the descriptor
/// is malformed or its shape is a component-model type this reader does not yet cover (a later-slice type).
/// EXACT mirror of `ast_marshal::build_type`: a NAME-head 1-list is a primitive; a STR-head form is a
/// compound (`list`/`record`/`tuple`).
pub fn parse_wit_type(a: &Arenas, id: StructId) -> Option<WitType> {
    // PRIMITIVE — a lone NAME-head marker `(kind)`. (A string head is a compound; handled below.)
    if let Some(kind) = a.head_name(id) {
        return match kind {
            "bool" => Some(WitType::Bool),
            "u8" => Some(WitType::U8),
            "u16" => Some(WitType::U16),
            "u32" => Some(WitType::U32),
            "u64" => Some(WitType::U64),
            "s8" => Some(WitType::S8),
            "s16" => Some(WitType::S16),
            "s32" => Some(WitType::S32),
            "s64" => Some(WitType::S64),
            "char" => Some(WitType::Char),
            "string" => Some(WitType::String),
            "f32" => Some(WitType::F32),
            "f64" => Some(WitType::F64),
            // A not-yet-covered primitive spelling → decline (never misread).
            _ => None,
        };
    }
    // COMPOUND — a STRING-head form. Read the raw children (the head is a Str leaf, so `as_form`, which
    // matches a NAME head, does not apply here).
    let ctor = a.head_ctor(id)?;
    let Struct::List(items) = a.get(id) else {
        return None;
    };
    match ctor {
        // ("list" <elem>)
        "list" => {
            let elem = *items.get(1)?;
            Some(WitType::List(Box::new(parse_wit_type(a, elem)?)))
        }
        // ("record" (fieldname <ty>)…) — each field a (name, type) 2-list, declaration order.
        "record" => {
            let mut fields = Vec::with_capacity(items.len().saturating_sub(1));
            for &entry in &items[1..] {
                let Struct::List(pair) = a.get(entry) else {
                    return None;
                };
                if pair.len() != 2 {
                    return None;
                }
                let name = a.as_name(pair[0])?.to_string();
                let ty = parse_wit_type(a, pair[1])?;
                fields.push((name, ty));
            }
            Some(WitType::Record(fields))
        }
        // ("tuple" <ty>…) — positional.
        "tuple" => {
            let mut elems = Vec::with_capacity(items.len().saturating_sub(1));
            for &t in &items[1..] {
                elems.push(parse_wit_type(a, t)?);
            }
            Some(WitType::Tuple(elems))
        }
        // ("option" <ty>)
        "option" => Some(WitType::Option(Box::new(parse_wit_type(
            a,
            *items.get(1)?,
        )?))),
        // ("unit") — payload-less; a 1-element Str-head form.
        "unit" => Some(WitType::Unit),
        // A not-yet-covered compound (variant/enum/result/flags/handle) → decline.
        _ => None,
    }
}

/// The natural canonical-ABI type a guest value-model [`Ty`] lowers to, when it HAS a single such type —
/// the identity the emit checks a declared [`WitType`] against. `None` for a Ty with no single natural
/// canonical form (a `Sum`/`Map`/`Set`/nominal/etc. crosses via the value-form document, not a direct
/// canonical type — so a declared `list<u8>` against it is a value-form bridge, not a match). This is the
/// EMIT side of the §3b value-bridging rule; a match means the ordinary canonical lower already serves.
pub fn ty_natural_wit(t: &Ty) -> Option<WitType> {
    Some(match t {
        Ty::Bool => WitType::Bool,
        Ty::Char => WitType::Char,
        Ty::String => WitType::String,
        // A runtime Bytes value IS a byte-list at the boundary.
        Ty::Bytes => WitType::List(Box::new(WitType::U8)),
        Ty::Int(it) => match (it.ground_signed(), it.ground_width()) {
            (true, 8) => WitType::S8,
            (true, 16) => WitType::S16,
            (true, 32) => WitType::S32,
            (true, 64) => WitType::S64,
            (false, 8) => WitType::U8,
            (false, 16) => WitType::U16,
            (false, 32) => WitType::U32,
            (false, 64) => WitType::U64,
            _ => return None,
        },
        Ty::Float(ft) => match ft.ground_width() {
            32 => WitType::F32,
            64 => WitType::F64,
            _ => return None,
        },
        Ty::List(e) => WitType::List(Box::new(ty_natural_wit(e)?)),
        Ty::Tuple(elems) => {
            let mut out = Vec::with_capacity(elems.len());
            for e in elems.iter() {
                out.push(ty_natural_wit(e)?);
            }
            WitType::Tuple(out)
        }
        // A record's natural canonical is a `record` of its fields' naturals, in canonical (sorted) field
        // order (the `BTreeMap` iterates sorted). If any field has no natural, the record has none either.
        Ty::Record(fields) => {
            let mut out = Vec::with_capacity(fields.len());
            for (name, fty) in fields.iter() {
                out.push((name.name.to_string(), ty_natural_wit(fty)?));
            }
            WitType::Record(out)
        }
        // No single natural canonical type — these cross via the value-form document when the declared
        // type is a byte-list, else they are incompatible.
        Ty::Sum { .. }
        | Ty::Nominal { .. }
        | Ty::Map(_, _)
        | Ty::Set(_)
        | Ty::BigInt
        | Ty::Rational
        | Ty::Symbol
        | Ty::Unit
        | Ty::Qty { .. }
        | Ty::Fn(_, _)
        | Ty::Type
        | Ty::Var(_)
        | Ty::Any
        | Ty::Cont { .. } => return None,
    })
}

/// Whether a guest [`Ty`] has a value-form document (so `value-encode`/`value-decode` can bridge it to a
/// declared `list<u8>`). Everything with a runtime value form qualifies; the non-value types (a function, a
/// type-value, an unresolved var, `Any`, a continuation, `unit`) do not.
pub fn ty_is_value_encodable(t: &Ty) -> bool {
    !matches!(
        t,
        Ty::Fn(_, _) | Ty::Type | Ty::Var(_) | Ty::Any | Ty::Cont { .. } | Ty::Unit
    )
}

/// The emit action for ONE boundary position (a param or result), given the WORLD's DECLARED canonical type
/// and the GUEST's value-model type — the core of emit-to-match (§3b value-bridging rule).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BridgeAction {
    /// The guest value already lowers to the declared canonical type — emit the ORDINARY canonical
    /// lift/lower (identity for a scalar, the existing S0 `bytes-len`/`bytes-get` copy for a Bytes/String).
    /// No value-form bridge.
    Direct,
    /// The declared type is `list<u8>` but the guest value is a value-encodable COMPOUND — bridge via the
    /// value-form document: `value-encode` a result out / `value-decode` a param in (the export-side bridge
    /// is the reducer `apply`'s Event/effect-list case; the import-side its inverse).
    ValueForm,
    /// No defined mapping — the guest value-model type cannot satisfy the declared type (a compile error,
    /// never a silent wrong emit). Later full-A slices widen this (e.g. a declared record vs a guest record
    /// via a structural marshal); for now anything past Direct/ValueForm declines.
    Incompatible,
}

/// Decide the emit for a boundary position: DIRECT when the guest already lowers to `declared`; VALUEFORM
/// when `declared` is a byte-list and the guest is a value-encodable compound; INCOMPATIBLE otherwise.
pub fn bridge_decision(declared: &WitType, guest: &Ty) -> BridgeAction {
    if ty_natural_wit(guest).as_ref() == Some(declared) {
        return BridgeAction::Direct;
    }
    let byte_list = matches!(declared, WitType::List(e) if **e == WitType::U8);
    if byte_list && ty_is_value_encodable(guest) {
        return BridgeAction::ValueForm;
    }
    BridgeAction::Incompatible
}

/// A member's function signature: ordered `(param-name, type)` params and a result type — decoded from a
/// `(func (param <name> <ty>)… (result <ty>))` node (v-syntax's `wit_func_sig`).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct WitFunc {
    pub params: Vec<(String, WitType)>,
    pub result: WitType,
}

/// One interface member: `(member <name> <func>)`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct WitMember {
    pub name: String,
    pub func: WitFunc,
}

/// One WIT interface, import or export: `(<import|export> <name> (member …)…)`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct WitInterface {
    pub name: String,
    pub members: Vec<WitMember>,
}

/// A decoded TARGET WIT WORLD — the structured world (v-syntax's `world_schema_tree` node) rcdzc reads to
/// drive emit-to-match. Split into `imports` (host-provided, marshalled per §4) and `exports`
/// (guest-provided, the export-side value-bridge fires here). The emit binds a guest def to a world export
/// member by NAME (v-ah: name match), reads that member's declared param/result types, and applies
/// [`bridge_decision`] per position.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TargetWorld {
    pub name: String,
    pub imports: Vec<WitInterface>,
    pub exports: Vec<WitInterface>,
}

/// Decode a `world_schema_tree` root `(world <name> <iface>…)` into a [`TargetWorld`]. `None` on a
/// malformed world or a member whose signature carries a type this build does not yet decode (a
/// later-slice type). Walks the EXACT node v-syntax's `world_schema_tree`/`wit_interface`/`wit_func_sig`
/// produce (all heads NAME atoms), composing [`parse_wit_type`] for each param/result descriptor — so the
/// emit reads the same structured world regardless of source (external artifact or inline decl).
pub fn parse_target_world(a: &Arenas, root: StructId) -> Option<TargetWorld> {
    if a.head_name(root)? != "world" {
        return None;
    }
    let Struct::List(items) = a.get(root) else {
        return None;
    };
    let name = a.as_name(*items.get(1)?)?.to_string();
    let mut imports = Vec::new();
    let mut exports = Vec::new();
    for &iface_id in items.get(2..)? {
        let dir = a.head_name(iface_id)?;
        // SURFACE DOC METADATA: a `///` doc on the inline `world` decl lowers to a `(doc …)` child — v-syntax's
        // `world_expr` interleaves doc nodes after the name (for round-trip). Docs are NOT part of world
        // IDENTITY: `world_schema_tree` (the canonical builder the external artifact + rcdzc's emit both route
        // through) takes no doc param, so a documented world MUST decode to the SAME `TargetWorld` as its
        // undocumented twin (v-syntax pinned that interface-structure identity in f1e3edc92). Skip doc heads
        // before reading interfaces; without this a doc'd top-level `world` decl fails to parse (its `(doc …)`
        // child is neither `import` nor `export`) → `world_bytes_crossing_export` returns None → the in-source
        // world-decl compile arm silently would not drive emit. MVP worlds carry no docs, so this is inert today.
        if dir == "doc" {
            continue;
        }
        let iface = parse_wit_interface(a, iface_id)?;
        match dir {
            "import" => imports.push(iface),
            "export" => exports.push(iface),
            _ => return None,
        }
    }
    Some(TargetWorld {
        name,
        imports,
        exports,
    })
}

/// Whether a performed effect operation binds to a world-IMPORT interface member — the schema-hash
/// phase-1a SYNC-vs-ASYNC perform discriminator (effects fold-purity boundary). The target WIT world
/// (`db.wit_world` bytes) declares the reducer's imports; a delegated effect whose op is a member of an
/// import interface (today `cadenza:agent-kernel/kv`) has a SYNCHRONOUS backing import to call, so it
/// stays a `Core::HostCall` (the kv path). A world-touching effect (Model/Tool/Emit) has NO import
/// binding — by the contract biconditional (v-ah, confirmed firm): a world-touching effect MUST defer
/// its result to a later `apply` for fold-purity, so it can NEVER be import-backed; import-backed ⟺ sync.
/// So a non-member perform is ASYNC and reifies into the returned effect-list (a `Core::EffectReify`
/// tuple), never a synchronous import call. This reads the DECLARED contract (zero hard-coded
/// Model/Tool/Emit vocabulary — GENERIC-COMPILER-clean): the emit observes that no synchronous target
/// exists, it does not know a capability taxonomy. `iface` is the effect's declaring name — the SHORT
/// name a source `(effect kv …)` declares (matching `backend::wasm::host::HostImport::effect`); the world
/// declares its import interface at the FULLY-QUALIFIED WIT name (`cadenza:agent-kernel/kv`), so the match
/// is on the FQ name's LAST `/`-segment (its interface short-name) — `kv` binds `cadenza:agent-kernel/kv`,
/// the same short-name↔FQ mapping the HostCall→component-import emit uses. `None`/false when the world is
/// absent or does not decode (no world ⟹ no host-fused imports ⟹ nothing is a sync import).
pub fn is_world_import_op(world_bytes: Option<&[u8]>, iface: &str, op: &str) -> bool {
    let Some(bytes) = world_bytes else {
        return false;
    };
    let Some(arenas) = crate::codec::decode(bytes) else {
        return false;
    };
    let Some(world) = parse_target_world(&arenas, arenas.root) else {
        return false;
    };
    // The world's import interface `name` is the FQ WIT name (`cadenza:agent-kernel/kv`); the perform's
    // `iface` is the SOURCE effect name (`Kv` — a Cadenza identifier, possibly capitalized). Match on the
    // FQ name's last `/`-segment, KEBAB-NORMALIZED on BOTH sides — the effect `Kv` fuses to the import
    // `cadenza:agent-kernel/kv` at the component boundary via `kebab_extern_name` (Kv → kv), so the
    // membership test must apply the same normalization or a capitalized source effect name (the real
    // `effect Kv`, host-fused to the lowercase WIT `kv` import) mis-misses and gets wrongly reified. Op
    // names normalize too (a `prefix-scan` op ↔ `prefix-scan` member — already kebab, identity). A bare
    // short-name world also matches (no `/` = itself, then kebab-normalized).
    use crate::backend::common::export_name::kebab_extern_name;
    fn short(fq: &str) -> &str {
        fq.rsplit('/').next().unwrap_or(fq)
    }
    let iface_k = kebab_extern_name(iface);
    let op_k = kebab_extern_name(op);
    world.imports.iter().any(|i| {
        kebab_extern_name(short(&i.name)) == iface_k
            && i.members.iter().any(|m| kebab_extern_name(&m.name) == op_k)
    })
}

fn parse_wit_interface(a: &Arenas, id: StructId) -> Option<WitInterface> {
    let Struct::List(items) = a.get(id) else {
        return None;
    };
    let name = a.as_name(*items.get(1)?)?.to_string();
    let mut members = Vec::new();
    for &m in items.get(2..)? {
        members.push(parse_wit_member(a, m)?);
    }
    Some(WitInterface { name, members })
}

fn parse_wit_member(a: &Arenas, id: StructId) -> Option<WitMember> {
    if a.head_name(id)? != "member" {
        return None;
    }
    let Struct::List(items) = a.get(id) else {
        return None;
    };
    let name = a.as_name(*items.get(1)?)?.to_string();
    let func = parse_wit_func(a, *items.get(2)?)?;
    Some(WitMember { name, func })
}

fn parse_wit_func(a: &Arenas, id: StructId) -> Option<WitFunc> {
    if a.head_name(id)? != "func" {
        return None;
    }
    let Struct::List(items) = a.get(id) else {
        return None;
    };
    let mut params = Vec::new();
    let mut result = None;
    for &kid in items.get(1..)? {
        match a.head_name(kid)? {
            // (param <name> <ty-desc>)
            "param" => {
                let Struct::List(p) = a.get(kid) else {
                    return None;
                };
                let pname = a.as_name(*p.get(1)?)?.to_string();
                let ty = parse_wit_type(a, *p.get(2)?)?;
                params.push((pname, ty));
            }
            // (result <ty-desc>) — always present, at most once.
            "result" => {
                if result.is_some() {
                    return None;
                }
                let Struct::List(r) = a.get(kid) else {
                    return None;
                };
                result = Some(parse_wit_type(a, *r.get(1)?)?);
            }
            _ => return None,
        }
    }
    Some(WitFunc {
        params,
        result: result?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Builder, Leaf};
    use crate::ty::IntTy;

    /// Build a primitive descriptor `(kind)` — a NAME-head 1-list, exactly as `ast_marshal::build_type`.
    fn prim(b: &mut Builder, kind: &str) -> StructId {
        let h = b.name(kind);
        b.list(vec![h])
    }
    fn str_head(b: &mut Builder, s: &str) -> StructId {
        b.atom_leaf(Leaf::Str(s.into()))
    }

    #[test]
    fn parses_each_scalar_primitive() {
        for (kind, want) in [
            ("bool", WitType::Bool),
            ("u8", WitType::U8),
            ("u32", WitType::U32),
            ("s64", WitType::S64),
            ("char", WitType::Char),
            ("string", WitType::String),
            ("f64", WitType::F64),
        ] {
            let mut b = Builder::new();
            let root = prim(&mut b, kind);
            let a = b.finish(root);
            assert_eq!(parse_wit_type(&a, root), Some(want), "primitive {kind}");
        }
    }

    #[test]
    fn parses_byte_list_as_list_of_u8() {
        // ("list" (u8)) — the reducer apply's byte boundary.
        let mut b = Builder::new();
        let u8p = prim(&mut b, "u8");
        let head = str_head(&mut b, "list");
        let root = b.list(vec![head, u8p]);
        let a = b.finish(root);
        assert_eq!(
            parse_wit_type(&a, root),
            Some(WitType::List(Box::new(WitType::U8))),
            "byte-list decodes to List(U8)"
        );
    }

    #[test]
    fn parses_a_record_of_scalars_in_declaration_order() {
        // ("record" (family (string)) (version (u32)))
        let mut b = Builder::new();
        let fam_ty = prim(&mut b, "string");
        let fam_name = b.name("family");
        let fam = b.list(vec![fam_name, fam_ty]);
        let ver_ty = prim(&mut b, "u32");
        let ver_name = b.name("version");
        let ver = b.list(vec![ver_name, ver_ty]);
        let head = str_head(&mut b, "record");
        let root = b.list(vec![head, fam, ver]);
        let a = b.finish(root);
        assert_eq!(
            parse_wit_type(&a, root),
            Some(WitType::Record(vec![
                ("family".to_string(), WitType::String),
                ("version".to_string(), WitType::U32),
            ])),
            "content-type-shaped record, declaration order"
        );
    }

    #[test]
    fn parses_nested_list_of_record() {
        // ("list" ("record" (n (u8)))) — nesting recurses.
        let mut b = Builder::new();
        let n_ty = prim(&mut b, "u8");
        let n_name = b.name("n");
        let field = b.list(vec![n_name, n_ty]);
        let rec_head = str_head(&mut b, "record");
        let rec = b.list(vec![rec_head, field]);
        let list_head = str_head(&mut b, "list");
        let root = b.list(vec![list_head, rec]);
        let a = b.finish(root);
        assert_eq!(
            parse_wit_type(&a, root),
            Some(WitType::List(Box::new(WitType::Record(vec![(
                "n".to_string(),
                WitType::U8
            )])))),
            "list-of-record nests"
        );
    }

    #[test]
    fn declines_a_not_yet_covered_compound() {
        // ("variant" …) is a later-slice type → None, never a misread.
        let mut b = Builder::new();
        let head = str_head(&mut b, "variant");
        let root = b.list(vec![head]);
        let a = b.finish(root);
        assert_eq!(parse_wit_type(&a, root), None, "unknown compound declines");
    }

    fn record_ty(fields: &[(&str, Ty)]) -> Ty {
        let mut m = std::collections::BTreeMap::new();
        for (n, t) in fields {
            m.insert(
                crate::resolved::Symbol {
                    namespace: None,
                    name: (*n).into(),
                },
                t.clone(),
            );
        }
        Ty::Record(std::rc::Rc::new(m))
    }

    #[test]
    fn natural_wit_of_a_record_is_sorted_fields() {
        // content-type-shaped guest record → its natural canonical `record`, fields in sorted order.
        let rec = record_ty(&[
            ("version", Ty::Int(IntTy::fixed(false, 32))),
            ("family", Ty::String),
        ]);
        assert_eq!(
            ty_natural_wit(&rec),
            Some(WitType::Record(vec![
                ("family".to_string(), WitType::String),
                ("version".to_string(), WitType::U32),
            ]))
        );
    }

    #[test]
    fn dispatch_scalar_and_bytes_are_direct_when_matching() {
        // A guest Int64 against a declared s64 → Direct (identity).
        assert_eq!(
            bridge_decision(&WitType::S64, &Ty::Int(IntTy::i64())),
            BridgeAction::Direct
        );
        // A guest u32 against declared u32 → Direct.
        assert_eq!(
            bridge_decision(&WitType::U32, &Ty::Int(IntTy::fixed(false, 32))),
            BridgeAction::Direct
        );
        // A guest Bytes against a declared byte-list → Direct (guest already is bytes).
        assert_eq!(
            bridge_decision(&WitType::List(Box::new(WitType::U8)), &Ty::Bytes),
            BridgeAction::Direct
        );
    }

    #[test]
    fn dispatch_bytelist_vs_compound_is_valueform() {
        // THE reducer case: declared byte-list, guest compound Event record → value-form bridge.
        let event = record_ty(&[("content_type", Ty::String), ("payload", Ty::Bytes)]);
        assert_eq!(
            bridge_decision(&WitType::List(Box::new(WitType::U8)), &event),
            BridgeAction::ValueForm
        );
    }

    #[test]
    fn dispatch_mismatch_is_incompatible() {
        // Declared scalar vs a guest that is not that scalar and not a byte-list bridge → Incompatible.
        assert_eq!(
            bridge_decision(&WitType::S64, &Ty::Bytes),
            BridgeAction::Incompatible
        );
        // A non-byte-list compound declared vs a mismatched guest is not yet a defined bridge → Incompatible.
        assert_eq!(
            bridge_decision(&WitType::String, &Ty::Bool),
            BridgeAction::Incompatible
        );
    }

    // A byte-list descriptor `("list" (u8))`, exactly as `build_type` emits it.
    fn byte_list(b: &mut Builder) -> StructId {
        let head = str_head(b, "list");
        let u8p = prim(b, "u8");
        b.list(vec![head, u8p])
    }
    fn opt(b: &mut Builder, inner: StructId) -> StructId {
        let head = str_head(b, "option");
        b.list(vec![head, inner])
    }
    fn unit(b: &mut Builder) -> StructId {
        let head = str_head(b, "unit");
        b.list(vec![head])
    }
    // (member <name> (func (param <pn> <ty>)… (result <ty>)))
    fn member(
        b: &mut Builder,
        name: &str,
        params: Vec<(&str, StructId)>,
        result: StructId,
    ) -> StructId {
        let fh = b.name("func");
        let mut func_kids = vec![fh];
        for (pn, ty) in params {
            let ph = b.name("param");
            let pnn = b.name(pn);
            func_kids.push(b.list(vec![ph, pnn, ty]));
        }
        let rh = b.name("result");
        let rnode = b.list(vec![rh, result]);
        func_kids.push(rnode);
        let func = b.list(func_kids);
        let mh = b.name("member");
        let mn = b.name(name);
        b.list(vec![mh, mn, func])
    }

    #[test]
    fn a_doc_node_on_the_world_is_skipped_and_the_world_parses() {
        // A `///`-documented inline `world` decl lowers to a `(doc …)` child interleaved after the name
        // (v-syntax's world_expr). Docs are NOT part of world identity, so `parse_target_world` must SKIP
        // them and decode the SAME `TargetWorld` as the undocumented twin (v-syntax pinned the syntax-side
        // interface-structure identity in f1e3edc92). Without the doc-skip this world would fail to parse (its
        // `(doc …)` child is neither `import` nor `export`) and the in-source world-decl arm silently would not
        // drive emit. This is the cross-side pin of the doc-independence guarantee.
        let mut b = Builder::new();
        let ev = byte_list(&mut b);
        let res = byte_list(&mut b);
        let apply = member(&mut b, "apply", vec![("event", ev)], res);
        let eh = b.name("export");
        let en = b.name("fold");
        let fold = b.list(vec![eh, en, apply]);
        // (doc "…") interleaved right after the world NAME, before the export — the world_expr doc position.
        let dh = b.name("doc");
        let dtext = str_head(&mut b, "the reducer world");
        let doc = b.list(vec![dh, dtext]);
        let wh = b.name("world");
        let wn = b.name("reducer");
        let world = b.list(vec![wh, wn, doc, fold]);
        let a = b.finish(world);

        let tw = parse_target_world(&a, world)
            .expect("a doc'd world still parses (the doc node is skipped)");
        assert_eq!(tw.name, "reducer");
        assert_eq!(tw.imports.len(), 0, "no imports");
        assert_eq!(
            tw.exports.len(),
            1,
            "the fold export is read past the doc node"
        );
        assert_eq!(tw.exports[0].name, "fold");
        assert_eq!(tw.exports[0].members.len(), 1);
        assert_eq!(tw.exports[0].members[0].name, "apply");
    }

    #[test]
    fn parses_the_reducer_target_world() {
        let mut b = Builder::new();
        // export fold { apply: func(event: list<u8>) -> list<u8> }
        let ev = byte_list(&mut b);
        let res = byte_list(&mut b);
        let apply = member(&mut b, "apply", vec![("event", ev)], res);
        let eh = b.name("export");
        let en = b.name("fold");
        let fold = b.list(vec![eh, en, apply]);
        // import kv { get: func(key: list<u8>) -> option<list<u8>>; set: func(key, value: list<u8>) -> unit }
        let gk = byte_list(&mut b);
        let ginner = byte_list(&mut b);
        let gres = opt(&mut b, ginner);
        let get = member(&mut b, "get", vec![("key", gk)], gres);
        let sk = byte_list(&mut b);
        let sv = byte_list(&mut b);
        let sres = unit(&mut b);
        let set = member(&mut b, "set", vec![("key", sk), ("value", sv)], sres);
        let ih = b.name("import");
        let inm = b.name("kv");
        let kv = b.list(vec![ih, inm, get, set]);
        // world reducer
        let wh = b.name("world");
        let wn = b.name("reducer");
        let world = b.list(vec![wh, wn, fold, kv]);
        let a = b.finish(world);

        let tw = parse_target_world(&a, world).expect("reducer world parses");
        assert_eq!(tw.name, "reducer");
        // export side — the reducer apply's byte boundary (what the export-side bridge reads).
        assert_eq!(tw.exports.len(), 1);
        assert_eq!(tw.exports[0].name, "fold");
        let bytes_list = WitType::List(Box::new(WitType::U8));
        assert_eq!(
            tw.exports[0].members,
            vec![WitMember {
                name: "apply".to_string(),
                func: WitFunc {
                    params: vec![("event".to_string(), bytes_list.clone())],
                    result: bytes_list.clone(),
                },
            }]
        );
        // import side — kv get (option) + set (unit), both parse.
        assert_eq!(tw.imports.len(), 1);
        assert_eq!(tw.imports[0].name, "kv");
        assert_eq!(tw.imports[0].members[0].name, "get");
        assert_eq!(
            tw.imports[0].members[0].func.result,
            WitType::Option(Box::new(bytes_list.clone()))
        );
        assert_eq!(tw.imports[0].members[1].name, "set");
        assert_eq!(tw.imports[0].members[1].func.result, WitType::Unit);
        assert_eq!(tw.imports[0].members[1].func.params.len(), 2);
    }

    #[test]
    fn is_world_import_op_discriminates_kv_sync_from_world_effect_async() {
        // Schema-hash phase-1a SYNC/ASYNC perform discriminator (the emit-side membership fork). Build the
        // reducer world (export fold{apply}, import kv{get,set}), codec-encode it to the `db.wit_world` byte
        // shape, and assert: a kv op (get/set) BINDS to the `kv` import interface → SYNC (stays Core::HostCall);
        // a world-touching op (no import binding) → ASYNC (reifies to output). Proves the membership logic
        // before the fork wires into `lower`.
        let mut b = Builder::new();
        let ev = byte_list(&mut b);
        let res = byte_list(&mut b);
        let apply = member(&mut b, "apply", vec![("event", ev)], res);
        let eh = b.name("export");
        let en = b.name("fold");
        let fold = b.list(vec![eh, en, apply]);
        let gk = byte_list(&mut b);
        let ginner = byte_list(&mut b);
        let gres = opt(&mut b, ginner);
        let get = member(&mut b, "get", vec![("key", gk)], gres);
        let sk = byte_list(&mut b);
        let sv = byte_list(&mut b);
        let sres = unit(&mut b);
        let set = member(&mut b, "set", vec![("key", sk), ("value", sv)], sres);
        let ih = b.name("import");
        let inm = b.name("kv");
        let kv = b.list(vec![ih, inm, get, set]);
        let wh = b.name("world");
        let wn = b.name("reducer");
        let world = b.list(vec![wh, wn, fold, kv]);
        let a = b.finish(world);
        // Encode to the `db.wit_world: Option<Vec<u8>>` byte shape the emit consults.
        let bytes = crate::codec::encode(&a);

        // kv ops BIND to the `kv` import interface → SYNC (Core::HostCall).
        assert!(
            is_world_import_op(Some(&bytes), "kv", "get"),
            "kv.get is a world-import member → sync"
        );
        assert!(
            is_world_import_op(Some(&bytes), "kv", "set"),
            "kv.set is a world-import member → sync"
        );
        // A world-touching effect op has NO import binding → ASYNC (reify-to-output, never a HostCall).
        assert!(
            !is_world_import_op(Some(&bytes), "model", "generate"),
            "a world-effect op is NOT an import member → async reify-to-output"
        );
        // An op name that exists on kv but under a DIFFERENT interface name does NOT match (interface-scoped).
        assert!(
            !is_world_import_op(Some(&bytes), "tool", "get"),
            "op membership is interface-scoped: `get` under `tool` (no such import) → async"
        );
        // No world at all → nothing is a sync import (a bare-perform program with no host-fused world).
        assert!(
            !is_world_import_op(None, "kv", "get"),
            "no target world → no sync imports"
        );
    }

    #[test]
    fn is_world_import_op_matches_a_fq_import_name_and_a_capitalized_effect_name() {
        // REGRESSION (cdz-agent-host e2e reject 036966): the REAL reducer world declares its import at the
        // FULLY-QUALIFIED WIT name `cadenza:agent-kernel/kv`, and the source effect is `effect Kv` (capital
        // K — a Cadenza identifier), host-fused to the lowercase WIT `kv` import via `kebab_extern_name`.
        // A case-sensitive last-`/`-segment match ("Kv" != "kv") wrongly missed → the perform fork
        // mis-reified `Kv.put` instead of leaving it a sync Core::HostCall → the real kv reducers folded to
        // 0 effects. The match must KEBAB-NORMALIZE both sides (Kv → kv), the same fusing the boundary does.
        let mut b = Builder::new();
        let ev = byte_list(&mut b);
        let res = byte_list(&mut b);
        let apply = member(&mut b, "apply", vec![("event", ev)], res);
        let eh = b.name("export");
        let en = b.name("fold");
        let fold = b.list(vec![eh, en, apply]);
        let pk = byte_list(&mut b);
        let pv = byte_list(&mut b);
        let pres = unit(&mut b);
        let put = member(&mut b, "put", vec![("key", pk), ("value", pv)], pres);
        let ih = b.name("import");
        // The FQ WIT import name the real reducer world declares (v-ah's reducer_world_artifact).
        let inm = b.name("cadenza:agent-kernel/kv");
        let kv = b.list(vec![ih, inm, put]);
        let wh = b.name("world");
        let wn = b.name("reducer");
        let world = b.list(vec![wh, wn, fold, kv]);
        let a = b.finish(world);
        let bytes = crate::codec::encode(&a);

        // The CAPITALIZED source effect `Kv` (as `effect Kv` declares it) fuses to the FQ lowercase `kv`
        // import → its ops are world-import members → SYNC HostCall (NOT reified). This is the exact case
        // the e2e reject exercised.
        assert!(
            is_world_import_op(Some(&bytes), "Kv", "put"),
            "capital `Kv` fuses to the FQ `cadenza:agent-kernel/kv` import (kebab Kv→kv) → sync import member"
        );
        // Lowercase `kv` also matches (kebab identity) — both source spellings fuse.
        assert!(
            is_world_import_op(Some(&bytes), "kv", "put"),
            "lowercase `kv` matches the FQ import too"
        );
        // A NON-import world-effect (capitalized `Model`) still does not match → reify (async).
        assert!(
            !is_world_import_op(Some(&bytes), "Model", "request"),
            "`Model` is not a world import → async reify (not mis-matched by the kebab normalization)"
        );
    }
}
