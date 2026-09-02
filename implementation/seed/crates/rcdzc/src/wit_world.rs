//! §3c — reading a preparsed TARGET WIT WORLD (binary-AST) to drive emit-to-match.
//!
//! Per the compiler↔platform-separation end-state (`DESIGN-compiler-platform-separation.md` §3b, operator
//! override 2026-08-11), rcdzc emits a Cadenza program to a target WIT world by reading each member's
//! DECLARED canonical-ABI type and bridging (`value-encode`/`value-decode`) wherever the guest value-model
//! type differs. The world reaches rcdzc as a PREPARSED binary-AST artifact (from an external producer OR an
//! inline module declaration — both lower to the same structured world); **rcdzc never parses WIT text.**
//!
//! This module is the TYPE-DESCRIPTOR reader — it decodes one type descriptor occurrence into a
//! [`WitType`]. The shared type-node vocabulary: a PRIMITIVE is a lone NAME-head marker `(u8)` / `(string)`;
//! a COMPOUND is `(list <elem>)`, `(record (fieldname <ty>)…)`, `(tuple <ty>…)`, etc. Heads are NAMES like
//! everything else (operator seq-206); the legacy STRING-head spelling `("list" <elem>)` is also accepted
//! for back-compat (see [`parse_wit_type`]). So `list<u8>` (a "byte-list", all the reducer `apply` boundary
//! needs) decodes to `WitType::List(U8)`.
//!
//! The WORLD-STRUCTURE reader (world → import/export interfaces → members → func → params/result) is added
//! once v-agent-harness formalizes the exact world node encoding (their lane, §3b); the vocabulary is locked
//! but its precise heads/nesting are theirs to pin, so this lands the grounded half now.

use crate::ast::{Arenas, Struct, StructId};
use crate::ty::Ty;

/// A canonical-ABI type as declared in a target WIT world, decoded from a `build_type` descriptor. Covers
/// the scalars, `list`, `record`, `tuple`, `option`, `unit`, and the tagged types `variant` / `enum` /
/// `result` / `flags` — the full set a typed reducer world exercises (`cdz-platform/wit/world.wit`:
/// `message`/`step` records, the `outcome`/`error` variants, `result<payload, error>`, `option<u64>`). The
/// only component-model types still outside this set are resource handles (`own`/`borrow`), added when a
/// world needs them; [`parse_wit_type`] returns `None` for a not-yet-covered descriptor rather than guessing.
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
    /// `variant { case, case(T), … }` — each case a name and an optional payload type, declaration order
    /// (`None` payload = a payload-less case, so `outcome { continue, close(closed) }` is
    /// `[("continue", None), ("close", Some(Record…))]`).
    Variant(Vec<(String, Option<WitType>)>),
    /// `enum { case, … }` — all cases payload-less (the degenerate variant), declaration order.
    Enum(Vec<String>),
    /// `result<ok, err>` — either arm may be absent, spelling `result` / `result<T>` / `result<_, E>` /
    /// `result<T, E>`. `ok`/`err` are `None` when that arm is omitted (distinct from `Some(Unit)`, an arm
    /// whose type IS `unit`).
    Result {
        ok: Option<Box<WitType>>,
        err: Option<Box<WitType>>,
    },
    /// `flags { name, … }` — a set of named bits, declaration order.
    Flags(Vec<String>),
    /// `unit` — the payload-less / void marker (a member's `result` is `unit` for a no-return func).
    Unit,
}

/// Decode one type descriptor occurrence `id` in `a` into its [`WitType`]. `None` if the descriptor
/// is malformed or its shape is a component-model type this reader does not yet cover (a later-slice type).
/// A PRIMITIVE is a lone NAME-head marker `(kind)` (name-head ONLY — see below); a COMPOUND reads its
/// children and accepts a NAME head `(list <e>)` (canonical) OR the legacy STRING head `("list" <e>)`.
pub fn parse_wit_type(a: &Arenas, id: StructId) -> Option<WitType> {
    let Struct::List(items) = a.get(id) else {
        return None;
    };
    // PRIMITIVE — a lone marker `(kind)`; the children are ignored. Primitives were ALWAYS name-head
    // (`(bool)`/`(s64)`): seq-206 did NOT introduce a legacy quoted-STRING spelling for them (only COMPOUNDS
    // ever had one, `("list" …)`). So a primitive matches the NAME head ONLY — a string-head `("bool")` is
    // malformed, NOT back-compat, and must still be flagged (else a bad descriptor silently drops the whole
    // world → misleading unbound-import cascade). Fall through to the compound match on a non-primitive name.
    if let Some(name) = a.head_name(id) {
        let prim = match name {
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
            _ => None,
        };
        if prim.is_some() {
            return prim;
        }
    }
    // COMPOUND — reads its children. Accepts EITHER a NAME head `(list <e>)` (the canonical spelling — heads
    // are Names "like everything else", operator seq-206) OR the legacy quoted-STRING head `("list" <e>)`,
    // the actual back-compat spelling the corpus migrates from (so string-headed COMPOUND descriptors keep
    // parsing WITHOUT a flag-day). In this WIT-descriptor context a `(record …)`/`(list …)` is unambiguously
    // a TYPE (never a value literal), so name-heads carry no value/pattern ambiguity.
    let spelling = a.head_name(id).or_else(|| a.head_ctor(id))?;
    match spelling {
        // (list <elem>)  [or legacy ("list" <elem>)]
        "list" => {
            let elem = *items.get(1)?;
            Some(WitType::List(Box::new(parse_wit_type(a, elem)?)))
        }
        // (record (= fieldname <ty>)…) — each field in declaration order. The reader emits a native
        // FieldPair `(= name ty)` (the migrated `(record (= a (s64)) …)` corpus form, mirroring the
        // value-model record's `(= k v)` fields); a plain 2-list `(name ty)` is also accepted (the
        // hand-built descriptor form the unit tests use). Read the (name, ty) nodes either way.
        "record" => {
            let mut fields = Vec::with_capacity(items.len().saturating_sub(1));
            for &entry in &items[1..] {
                let (name_id, ty_id) = if let Some((k, v)) = a.field_pair_parts(entry) {
                    (k, v)
                } else if let Struct::List(pair) = a.get(entry) {
                    if pair.len() != 2 {
                        return None;
                    }
                    (pair[0], pair[1])
                } else {
                    return None;
                };
                let name = a.as_name(name_id)?.to_string();
                let ty = parse_wit_type(a, ty_id)?;
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
        // ("variant" <case>…) — each case a list: a 1-list (casename) is payload-less, a 2-list
        // (casename <ty>) carries a payload. Declaration order (a variant's case order is significant).
        "variant" => {
            let mut cases = Vec::with_capacity(items.len().saturating_sub(1));
            for &entry in &items[1..] {
                let Struct::List(case) = a.get(entry) else {
                    return None;
                };
                match case.len() {
                    1 => cases.push((a.as_name(case[0])?.to_string(), None)),
                    2 => {
                        let name = a.as_name(case[0])?.to_string();
                        cases.push((name, Some(parse_wit_type(a, case[1])?)));
                    }
                    _ => return None,
                }
            }
            Some(WitType::Variant(cases))
        }
        // ("enum" <name>…) — each case a bare NAME leaf, always payload-less.
        "enum" => {
            let mut cases = Vec::with_capacity(items.len().saturating_sub(1));
            for &c in &items[1..] {
                cases.push(a.as_name(c)?.to_string());
            }
            Some(WitType::Enum(cases))
        }
        // ("flags" <name>…) — a set of named bits; like enum, each a bare NAME leaf.
        "flags" => {
            let mut names = Vec::with_capacity(items.len().saturating_sub(1));
            for &c in &items[1..] {
                names.push(a.as_name(c)?.to_string());
            }
            Some(WitType::Flags(names))
        }
        // ("result" <ok-slot> <err-slot>) — each slot is a type descriptor OR the absent-marker ("none")
        // for an arm WIT omits (`result<_, E>` has an absent ok, `result<T>` an absent err).
        "result" => {
            let ok = parse_result_slot(a, *items.get(1)?)?;
            let err = parse_result_slot(a, *items.get(2)?)?;
            Some(WitType::Result { ok, err })
        }
        // ("unit") — payload-less; a 1-element Str-head form.
        "unit" => Some(WitType::Unit),
        // A not-yet-covered compound (a resource handle) → decline.
        _ => None,
    }
}

/// Decode one arm of a `result` descriptor: the absent-marker `("none")` → `None` (an arm WIT omits), else a
/// type descriptor → `Some(ty)`. The outer `Option` is the malformed-input signal (a descriptor that does
/// not decode), kept distinct from the inner `None` (a well-formed absent arm).
fn parse_result_slot(a: &Arenas, id: StructId) -> Option<Option<Box<WitType>>> {
    // The absent-arm marker — a NAME head `(none)` (canonical) or the legacy STRING head `("none")`.
    if a.head_name(id).or_else(|| a.head_ctor(id)) == Some("none") {
        return Some(None);
    }
    Some(Some(Box::new(parse_wit_type(a, id)?)))
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
        // `unit` is a first-class WIT type — the exact inverse of [`wit_type_to_ty`]'s `WitType::Unit →
        // Ty::Unit`. The imposed-world/host boundary handles a no-value return upstream, but a SYNTHESIZED
        // world whose op result is `Unit` (a guest annotation drives the decl) needs this outbound arm to
        // self-declare `unit` (WIT-shape-coverage matrix v1, v-rb-confirmed: emit owns no-value returns,
        // synth owns the declared-unit result).
        Ty::Unit => WitType::Unit,
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
        | Ty::Qty { .. }
        | Ty::Fn(_, _)
        | Ty::Type
        | Ty::Var(_)
        | Ty::Any
        | Ty::Cont { .. } => return None,
    })
}

/// The Cadenza value-model [`Ty`] a declared world-import [`WitType`] denotes — the FORWARD direction of
/// [`ty_natural_wit`], the canonical WIT→Ty mapping a guest's `perform` of a world import types against
/// (v-platform oracle ruling 2026-08-23, `world-import-call-surface-must-be-fully-general`). Generic over an
/// ARBITRARY import member's signature, ZERO per-interface arms:
///
/// - `list<u8>` → `Bytes` (every hash/payload/token/bytes alias collapses to `Bytes`, the inverse of
///   `ty_natural_wit`'s `Bytes → list<u8>`); any other `list<T>` → `List(⟦T⟧)`.
/// - `bool`/`char`/`string` → `Bool`/`Char`/`String`; `uN`/`sN` → an `Int` of that signedness+width;
///   `f32`/`f64` → a `Float` of that width; `unit` → `Unit`.
/// - `tuple<T…>` → `Tuple(⟦T⟧…)`; `record { f: T… }` → `Record(f: ⟦T⟧…)` (canonically SORTED, as every
///   `Ty::Record` is — the WIT-declared field order is a boundary/emit concern, not the type's identity).
/// - `option<T>` → the prelude `Option(⟦T⟧)`; `result<ok, err>` → the prelude `Result(⟦ok⟧, ⟦err⟧)`, an
///   absent arm filled with `Unit` (WIT `result` / `result<T>` / `result<_, E>`).
///
/// `⟦·⟧` = apply recursively. Returns `None` for a shape not yet mapped: `enum` / `variant` / `flags` (each
/// needs a SYNTHESIZED nominal sum decl — a Cadenza sum is nominal, carrying a decl identity, unlike a
/// structural WIT variant; a later increment covers `Dir`/`Error`), or any compound whose inner type does
/// not map. `db` is needed only to instantiate the prelude `Option`/`Result` sums (`normalize_sum` over the
/// declared occurrence), so a prelude-less compile yields `None` there too.
pub fn wit_type_to_ty(db: &crate::db::Db, t: &WitType) -> Option<Ty> {
    use crate::ty::{FloatTy, IntTy};
    Some(match t {
        WitType::Bool => Ty::Bool,
        WitType::Char => Ty::Char,
        WitType::String => Ty::String,
        WitType::U8 => Ty::Int(IntTy::fixed(false, 8)),
        WitType::U16 => Ty::Int(IntTy::fixed(false, 16)),
        WitType::U32 => Ty::Int(IntTy::fixed(false, 32)),
        WitType::U64 => Ty::Int(IntTy::fixed(false, 64)),
        WitType::S8 => Ty::Int(IntTy::fixed(true, 8)),
        WitType::S16 => Ty::Int(IntTy::fixed(true, 16)),
        WitType::S32 => Ty::Int(IntTy::fixed(true, 32)),
        WitType::S64 => Ty::Int(IntTy::fixed(true, 64)),
        WitType::F32 => Ty::Float(FloatTy::fixed(32)),
        WitType::F64 => Ty::Float(FloatTy::fixed(64)),
        WitType::Unit => Ty::Unit,
        // A byte-list collapses to `Bytes` (the inverse of `ty_natural_wit`); any other element → `List(⟦T⟧)`.
        WitType::List(elem) => {
            if matches!(**elem, WitType::U8) {
                Ty::Bytes
            } else {
                Ty::List(Box::new(wit_type_to_ty(db, elem)?))
            }
        }
        WitType::Tuple(elems) => {
            let mut out = Vec::with_capacity(elems.len());
            for e in elems {
                out.push(wit_type_to_ty(db, e)?);
            }
            Ty::Tuple(out.into())
        }
        WitType::Record(fields) => {
            let mut map = std::collections::BTreeMap::new();
            for (name, fty) in fields {
                map.insert(
                    crate::resolved::Symbol::plain(name.as_str()),
                    wit_type_to_ty(db, fty)?,
                );
            }
            Ty::Record(std::rc::Rc::new(map))
        }
        WitType::Option(inner) => {
            let elem = wit_type_to_ty(db, inner)?;
            prelude_sum(db, "Option", vec![elem])?
        }
        WitType::Result { ok, err } => {
            let ok_ty = match ok {
                Some(t) => wit_type_to_ty(db, t)?,
                None => Ty::Unit,
            };
            let err_ty = match err {
                Some(t) => wit_type_to_ty(db, t)?,
                None => Ty::Unit,
            };
            prelude_sum(db, "Result", vec![ok_ty, err_ty])?
        }
        // A nominal sum needs a SYNTHESIZED decl (Cadenza sums carry a decl identity) — a later increment.
        WitType::Variant(_) | WitType::Enum(_) | WitType::Flags(_) => return None,
    })
}

/// BEST-EFFORT [`wit_type_to_ty`] for the cadenza typed-WIT-export RESULT `expected` threading: it NEVER
/// returns `None` for a compound — an unmappable field/element (a `variant`/`enum`/`flags`, pending the
/// synthesized-nominal-decl increment) becomes `Ty::Any` rather than poisoning the WHOLE shape. This matters
/// for a result RECORD that mixes a mappable field beside an unmappable one — e.g. `record { requests:
/// list<record{ deadline-nanos: option<u64> }>, outcome: variant{…} }` (28-wit typed-reducer host-op-result
/// shapes): the strict `wit_type_to_ty` bails on `outcome` and drops the ENTIRE result type, so the bare
/// `Option.None` deep inside `requests` never recovers its `option<u64>` element type and DECLINES (CDZ0900).
/// Sound as an `expected` FALLBACK ONLY: `expected` refines an UNDER-DETERMINED value's type args, and
/// `Ty::Any` at a position is itself treated as under-determined ([`ty_has_free_arg`] is true for `Any`), so
/// it is NO WORSE than an absent `expected` there — while the mappable SIBLING fields DO get their resolved
/// args threaded down. The unmappable field's own VALUE (`Outcome.Continue`, a fully-determined user variant)
/// needs no `expected` and emits on its own. Do NOT use where an EXACT type is required (param derivation,
/// codec) — that stays on the strict `wit_type_to_ty` which honestly declines an unmapped shape.
pub fn wit_type_to_ty_lossy(db: &crate::db::Db, t: &WitType) -> crate::ty::Ty {
    use crate::ty::Ty;
    match t {
        WitType::List(elem) => {
            if matches!(**elem, WitType::U8) {
                Ty::Bytes
            } else {
                Ty::List(Box::new(wit_type_to_ty_lossy(db, elem)))
            }
        }
        WitType::Tuple(elems) => {
            Ty::Tuple(elems.iter().map(|e| wit_type_to_ty_lossy(db, e)).collect())
        }
        WitType::Record(fields) => {
            let mut map = std::collections::BTreeMap::new();
            for (name, fty) in fields {
                map.insert(
                    crate::resolved::Symbol::plain(name.as_str()),
                    wit_type_to_ty_lossy(db, fty),
                );
            }
            Ty::Record(std::rc::Rc::new(map))
        }
        WitType::Option(inner) => {
            prelude_sum(db, "Option", vec![wit_type_to_ty_lossy(db, inner)]).unwrap_or(Ty::Any)
        }
        WitType::Result { ok, err } => {
            let ok_ty = ok
                .as_ref()
                .map(|t| wit_type_to_ty_lossy(db, t))
                .unwrap_or(Ty::Unit);
            let err_ty = err
                .as_ref()
                .map(|t| wit_type_to_ty_lossy(db, t))
                .unwrap_or(Ty::Unit);
            prelude_sum(db, "Result", vec![ok_ty, err_ty]).unwrap_or(Ty::Any)
        }
        // A scalar / unit maps exactly; an unmappable tagged type (variant/enum/flags) over-approximates to
        // `Any` (harmless as an expected fallback — see the doc).
        _ => wit_type_to_ty(db, t).unwrap_or(Ty::Any),
    }
}

/// Instantiate a prelude sum type (`Option`/`Result`) at the given type args, or `None` if the declaration
/// is absent (a prelude-less compile). Mirrors `infer::option_ty`, the shared way the world-effect request
/// record spells its `Option Bytes` fields.
fn prelude_sum(db: &crate::db::Db, name: &str, args: Vec<Ty>) -> Option<Ty> {
    let occ = db.type_decls.iter().find(|t| t.name == name)?.occ;
    Some(db.normalize_sum(occ, args))
}

/// The resolvable SOURCE TYPE-EXPR AST node a world-import [`WitType`] denotes — the form a hand-written
/// type annotation takes (`Bytes` / `(List T)` / `(Record (: f T)…)` / `(Option T)` / `(Result T E)` /
/// `(Int W)` / `(-> …)`), which `eval::typeval_of` reduces to the SAME [`Ty`] that [`wit_type_to_ty`]
/// produces. This is the node the derive-from-`world.wit` synthesis injects into a synthesized `(effect
/// <iface> (op <op> (-> <param-expr>… <result-expr>)))` decl (v-syntax's b-prime plan: inject synthesized
/// effect decls BEFORE resolve, so all downstream sees ORDINARY effects with zero resolve special-casing —
/// the no-redeclare surface).
///
/// It builds SOURCE forms, NOT `eval::encode_ty`'s internal `(typeval …)` payload: the two DIVERGE for a
/// sum, where `encode_ty` emits the nominal `(Sum <name> <decl> …)` (a `decode_ty`-only shape `typeval_of`
/// does not read) while the injectable source form is the ctor application `(Option T)` / `(Result T E)`.
/// The scalar / list / tuple / record forms coincide with `encode_ty`'s. `None` for a WIT type
/// [`wit_type_to_ty`] does not map (`enum` / `variant` / `flags`, pending the synthesized-nominal-decl
/// increment), or whose inner type does not map. Builds into `db`'s arena; the synthesis pass reuses it for
/// the injected decls.
pub fn wit_type_to_type_expr(ast: &mut Arenas, t: &WitType) -> Option<StructId> {
    wit_type_to_type_expr_with_sums(ast, t, &std::collections::HashMap::new())
}

/// [`wit_type_to_type_expr`] with a NOMINAL-SUM map: a WIT `variant`/`enum` is resolved to a GUEST-declared
/// named sum whose case-name set matches (kebab-normalized), emitting that type's bare NAME. The reducer's
/// world declares its variants ANONYMOUSLY (`variant(timeout, …)`), but the guest declares a NAMED
/// `type Error = | Timeout | …` that mirrors it; Cadenza sums carry a DECL identity, so the derived boundary
/// type must reference the guest's named type (not an anonymous structural sum). `sums` maps a case-name set
/// → the guest type name (built by [`guest_sum_names`]); an EMPTY map reproduces the plain
/// [`wit_type_to_type_expr`] (variant/enum → `None`), which the import synthesis uses (it skips such members).
/// A `flags` type has no named-sum analogue and stays `None`.
fn wit_type_to_type_expr_with_sums(
    ast: &mut Arenas,
    t: &WitType,
    sums: &std::collections::HashMap<std::collections::BTreeSet<String>, String>,
) -> Option<StructId> {
    use crate::ast::Leaf;
    use crate::backend::common::export_name::kebab_extern_name;
    use crate::prelude::{push_atom, push_list};
    fn nm(ast: &mut Arenas, s: &str) -> StructId {
        push_atom(ast, Leaf::Name(s.into()))
    }
    // `(Int W)` / `(UInt W)` — the width-carrying head form `typeval_of` reads (mirrors `encode_ty`).
    fn int_expr(ast: &mut Arenas, signed: bool, w: i64) -> StructId {
        let ctor = nm(ast, if signed { "Int" } else { "UInt" });
        let width = push_atom(
            ast,
            Leaf::Int {
                value: crate::ast::IntValue::from_i64(w),
                radix: crate::ast::Radix::Dec,
            },
        );
        push_list(ast, vec![ctor, width])
    }
    fn float_expr(ast: &mut Arenas, w: i64) -> StructId {
        let ctor = nm(ast, "Float");
        let width = push_atom(
            ast,
            Leaf::Int {
                value: crate::ast::IntValue::from_i64(w),
                radix: crate::ast::Radix::Dec,
            },
        );
        push_list(ast, vec![ctor, width])
    }
    Some(match t {
        WitType::Bool => nm(ast, "Bool"),
        WitType::Char => nm(ast, "Char"),
        WitType::String => nm(ast, "String"),
        WitType::Unit => nm(ast, "Unit"),
        WitType::U8 => int_expr(ast, false, 8),
        WitType::U16 => int_expr(ast, false, 16),
        WitType::U32 => int_expr(ast, false, 32),
        WitType::U64 => int_expr(ast, false, 64),
        WitType::S8 => int_expr(ast, true, 8),
        WitType::S16 => int_expr(ast, true, 16),
        WitType::S32 => int_expr(ast, true, 32),
        WitType::S64 => int_expr(ast, true, 64),
        WitType::F32 => float_expr(ast, 32),
        WitType::F64 => float_expr(ast, 64),
        // A byte-list is `Bytes` (the inverse of `ty_natural_wit`); any other element → `(List T)`.
        WitType::List(elem) if matches!(**elem, WitType::U8) => nm(ast, "Bytes"),
        WitType::List(elem) => {
            let inner = wit_type_to_type_expr_with_sums(ast, elem, sums)?;
            let head = nm(ast, "List");
            push_list(ast, vec![head, inner])
        }
        WitType::Tuple(elems) => {
            let mut items = vec![nm(ast, "Tuple")];
            for e in elems {
                items.push(wit_type_to_type_expr_with_sums(ast, e, sums)?);
            }
            push_list(ast, items)
        }
        // `(Record (: fname T)…)` — the canonical ascription-field form (mirrors `encode_ty`).
        WitType::Record(fields) => {
            let mut items = vec![nm(ast, "Record")];
            for (name, fty) in fields {
                let colon = nm(ast, ":");
                let fname = nm(ast, name.as_str());
                let ft = wit_type_to_type_expr_with_sums(ast, fty, sums)?;
                items.push(push_list(ast, vec![colon, fname, ft]));
            }
            push_list(ast, items)
        }
        // Source ctor application `(Option T)` — NOT `encode_ty`'s nominal `(Sum …)`.
        WitType::Option(inner) => {
            let ie = wit_type_to_type_expr_with_sums(ast, inner, sums)?;
            let head = nm(ast, "Option");
            push_list(ast, vec![head, ie])
        }
        // `(Result ok err)`; an absent arm is `Unit` (WIT `result` / `result<T>` / `result<_, E>`).
        WitType::Result { ok, err } => {
            let oe = match ok {
                Some(t) => wit_type_to_type_expr_with_sums(ast, t, sums)?,
                None => nm(ast, "Unit"),
            };
            let ee = match err {
                Some(t) => wit_type_to_type_expr_with_sums(ast, t, sums)?,
                None => nm(ast, "Unit"),
            };
            let head = nm(ast, "Result");
            push_list(ast, vec![head, oe, ee])
        }
        // A WIT `variant`/`enum` is ANONYMOUS, but a Cadenza sum carries a DECL identity — so resolve it to a
        // GUEST-declared named sum whose case-name set matches (kebab), emitting that type's bare NAME. With an
        // empty `sums` map (the import-synthesis path) this stays `None` — the guest hand-declares such a
        // member. `flags` has no named-sum analogue → always `None`.
        WitType::Variant(cases) => {
            let set: std::collections::BTreeSet<String> =
                cases.iter().map(|(c, _)| kebab_extern_name(c)).collect();
            let name = sums.get(&set)?;
            nm(ast, &name.clone())
        }
        WitType::Enum(cases) => {
            let set: std::collections::BTreeSet<String> =
                cases.iter().map(|c| kebab_extern_name(c)).collect();
            let name = sums.get(&set)?;
            nm(ast, &name.clone())
        }
        WitType::Flags(_) => return None,
    })
}

/// Synthesize a source-AST `(effect <iface> (op <op> <arrow>)…)` DECL for each world-IMPORT interface,
/// deriving every member's arrow `(-> <param-expr>… <result-expr>)` from its declared WIT signature via
/// [`wit_type_to_type_expr`]. These are the decls the no-redeclare pre-pass injects into a guest's module
/// BEFORE resolve (v-syntax's b-prime plan): a guest that performs a NAMED world import — `(host (iface)
/// ((. iface op) args…))` — then resolves + types against the derived effect WITHOUT a hand-written
/// `(effect …)` decl, and all downstream (resolve / infer / lower / [`is_world_import_op`]) sees an ORDINARY
/// effect (which then re-derives it as a synchronous `Core::HostCall`), so genericity + no drift come for
/// free.
///
/// The decl NAME is the import interface's SHORT kebab name (`cadenza:agent-kernel/identity` → `identity`),
/// matching what a guest writes in `(host (identity) …)` and what [`is_world_import_op`] binds. A NULLARY op
/// is the single-element arrow `(-> R)` (the elided-unit convention). A member whose signature carries a
/// type [`wit_type_to_type_expr`] does not map (`enum`/`variant`/`flags`) is SKIPPED (the guest falls back to
/// a hand decl for it until that increment); an interface with no mappable member yields no decl. Returns
/// the decl nodes in `db`'s arena, in world-declaration order; empty when the world is absent/undecodable.
pub fn synthesize_world_import_effect_decls(
    ast: &mut Arenas,
    world_bytes: Option<&[u8]>,
) -> Vec<StructId> {
    use crate::ast::Leaf;
    use crate::backend::common::export_name::kebab_extern_name;
    use crate::prelude::{push_atom, push_list};
    fn nm(ast: &mut Arenas, s: &str) -> StructId {
        push_atom(ast, Leaf::Name(s.into()))
    }
    fn short(fq: &str) -> &str {
        fq.rsplit('/').next().unwrap_or(fq)
    }
    let Some(bytes) = world_bytes else {
        return Vec::new();
    };
    // The world decodes into its OWN arena; the synthesized decls are built into the target `ast` (the
    // guest module's arena the pre-pass injects into) — reading the world structure, writing source forms.
    let Some(world_arena) = crate::codec::decode(bytes) else {
        return Vec::new();
    };
    let Some(world) = parse_target_world(&world_arena, world_arena.root) else {
        return Vec::new();
    };
    // The guest's NAMED sums keyed by (kebab) case-name set → type name (from its top-level `(type …)`
    // decls, visible in `ast` at this pre-resolve pass). Threading it into the arrow derivation resolves a
    // WIT `variant`/`enum` in an import op's type to the guest's mirroring named sum (e.g. the world's
    // anonymous `error` variant → the guest's `type Error`), so an import op whose param/result CONTAINS a
    // variant (deliver-response's `answer: result<payload, error>`) maps + RESOLVES — the same
    // `_with_sums` machinery the EXPORT-param derivation already uses (#3137). Without it (an empty map) a
    // variant → `None` → the op is skipped (deliver-response silently dropped from the synthesized effect).
    let sums = guest_sum_names(ast);
    let mut decls = Vec::new();
    for iface in &world.imports {
        let mut ops = Vec::new();
        for member in &iface.members {
            // `(-> <param-expr>… <result-expr>)` — a nullary op is the single-element `(-> R)`. A member
            // with a not-yet-mappable param/result type is skipped (the guest hand-declares it meanwhile).
            let mut arrow_kids = vec![nm(ast, "->")];
            let mut mappable = true;
            for (_pname, pty) in &member.func.params {
                match wit_type_to_type_expr_with_sums(ast, pty, &sums) {
                    Some(n) => arrow_kids.push(n),
                    None => {
                        mappable = false;
                        break;
                    }
                }
            }
            if !mappable {
                continue;
            }
            let Some(result_expr) =
                wit_type_to_type_expr_with_sums(ast, &member.func.result, &sums)
            else {
                continue;
            };
            arrow_kids.push(result_expr);
            let arrow = push_list(ast, arrow_kids);
            let op_head = nm(ast, "op");
            let op_name = nm(ast, &kebab_extern_name(&member.name));
            ops.push(push_list(ast, vec![op_head, op_name, arrow]));
        }
        if ops.is_empty() {
            continue;
        }
        let effect_head = nm(ast, "effect");
        let iface_name = nm(ast, &kebab_extern_name(short(&iface.name)));
        let mut effect_kids = vec![effect_head, iface_name];
        effect_kids.extend(ops);
        decls.push(push_list(ast, effect_kids));
    }
    decls
}

/// The no-redeclare world-import PRE-PASS (v-syntax's b-prime plan): for an in-source top-level `(world …)`
/// decl, synthesize an `(effect <iface> (op …)…)` decl per import interface (via
/// [`synthesize_world_import_effect_decls`]) and APPEND each to the module's top-level members — BEFORE
/// `scan_top_level`/resolve. So a guest that performs a NAMED world import — `(host (iface) ((. iface op)
/// args…))` — resolves + types against the DERIVED effect with NO hand-written `(effect …)` decl: the
/// synthesized decl is byte-shaped like a hand-written one, so resolve/infer/lower/`is_world_import_op` all
/// see an ORDINARY effect (which re-derives as a synchronous `Core::HostCall`). Runs in `Db::load_linked`
/// alongside `param_sidecar::generate` — the same generate-before-resolve slot, at a locus BOTH compile AND
/// check reach, so the surface holds in `cdz check`/LSP too (no separate wit_world-population fix). A module
/// with no in-source `(world …)` (or a world with no mappable import) is left untouched. The EXTERNAL
/// `KIND_WIT_WORLD` artifact world (not present in the ast at load) is a follow-up (thread via the
/// `load_linked` linkage seam); reference reducers declare the world in-source, so this ships that path.
pub fn inject_world_import_effects(ast: &mut Arenas) {
    let Some(world_form) = top_world_form(ast) else {
        return;
    };
    // Encode the world subtree to the SAME bytes the `db.wit_world` population uses (compile.rs), then
    // inject the synthesized effect decls (the shared bytes-driven path also serves the external-artifact
    // world — see `inject_world_import_effects_from_bytes`).
    let bytes = crate::codec::encode(&crate::sidecar::extract_subtree(ast, world_form));
    inject_world_import_effects_from_bytes(ast, &bytes);
}

/// Inject the synthesized `(effect …)` decls for a world given its ENCODED bytes — the shared core of the
/// no-redeclare sidecar, driven either by an in-source `(world …)` (via [`inject_world_import_effects`], in
/// `Db::load_linked`) OR by an EXTERNAL `KIND_WIT_WORLD` artifact (called from `compile` BEFORE `Db::load`,
/// so a reducer delivered with an artifact world gets the same no-redeclare surface as an in-source one).
/// Synthesizes an `(effect <iface> (op …)…)` per import interface and appends it as a top-level module
/// member — SKIPPING any interface the guest ALREADY declares (a hand-written `(effect X …)` — the redeclare
/// path, or an `enum`/`variant`-carrying interface whose type name is not recoverable from the world
/// descriptor — WINS; a second `(effect X …)` would be a duplicate). This skip also makes the two drivers
/// composable: if `compile` injects the artifact effects first, a later in-source pre-pass sees them
/// declared and skips, so an interface is never synthesized twice.
pub fn inject_world_import_effects_from_bytes(ast: &mut Arenas, world_bytes: &[u8]) {
    use crate::backend::common::export_name::kebab_extern_name;
    // FIRST synthesize any nominal type decl a world `enum`/`variant` needs but the guest did not mirror —
    // injected as a top-level `(type …)` so the `guest_sum_names` re-scan INSIDE
    // `synthesize_world_import_effect_decls` below sees it and the sum-typed op maps (else it would be
    // skipped). No-op when the world has no enum/variant, or every one already has a guest mirror.
    synthesize_missing_nominal_decls(ast, world_bytes);
    // Effect names the module already declares (kebab-normalized) — hand-written OR already-synthesized.
    let declared: std::collections::HashSet<String> = top_level_items(ast)
        .into_iter()
        .filter_map(|it| ast.as_form(it, "effect").and_then(|t| t.first().copied()))
        .filter_map(|name_node| ast.as_name(name_node).map(kebab_extern_name))
        .collect();
    let decls = synthesize_world_import_effect_decls(ast, Some(world_bytes));
    for decl in decls {
        // The synthesized decl is `(effect <name> …)`; skip it if the module already declares that effect
        // (its nodes stay unreferenced in the arena — inert, not scanned as a top-level member).
        let name = ast
            .as_form(decl, "effect")
            .and_then(|t| t.first().copied())
            .and_then(|n| ast.as_name(n))
            .map(kebab_extern_name);
        if name.is_some_and(|n| declared.contains(&n)) {
            continue;
        }
        append_module_member(ast, decl);
    }
}

/// The no-annotation EXPORT-BOUNDARY pre-pass — the export-side mirror of [`inject_world_import_effects`].
/// For an in-source top-level `(world …)` decl, DERIVE each guest-export def's boundary PARAMETER types from
/// the matching world guest-export member and inject them as `(: <param> <type>)` annotations BEFORE
/// `scan_top_level`/resolve. So a reducer writes `(def (on-message msg) <step>)` with NO param annotation and
/// still type-checks: `msg` is typed from `world.exports…on-message`'s DECLARED param type
/// ([`wit_type_to_type_expr`]), exactly the annotation the author would otherwise hand-write. This removes the
/// one remaining boundary annotation a reducer needs (the entry-point param types); its own nominal `(type …)`
/// sums are inherent and stay.
///
/// A guest def binds to a world export member BY NAME (`kebab_extern_name` — the SAME binding the emit uses),
/// then params align POSITIONALLY. Only a BARE param binder is annotated — an author-written `(: p T)` WINS
/// (left untouched), a position past the member's params is left alone, and a param whose declared type
/// [`wit_type_to_type_expr`] cannot map (`enum`/`variant`/`flags`) is left bare (it falls back to the ordinary
/// CDZ0201 "annotate it" — no regression). Runs in `Db::load_linked` in the same generate-before-resolve slot
/// as the import pre-pass, at the locus BOTH compile AND check reach, so the surface holds in `cdz check`/LSP
/// too. A module with no in-source `(world …)`, or no def matching an export member, is left untouched. The
/// EXTERNAL `KIND_WIT_WORLD` artifact world is served by the sibling
/// [`derive_world_export_param_annotations_from_bytes`] (called from `compile` before `Db::load`).
pub fn derive_world_export_param_annotations(ast: &mut Arenas) {
    use crate::backend::common::export_name::kebab_extern_name;
    let Some(world_form) = top_world_form(ast) else {
        return;
    };
    // Build the (export-member kebab name → ordered param WitTypes) map OWNED, dropping the
    // `parse_target_world` borrow of `ast` before the mutating rewrite (WitType is Clone). The in-source
    // `(world …)` node is already the exact `world_schema_tree` shape `parse_target_world` reads, so no bytes
    // round-trip is needed (unlike the artifact path, which decodes its own arena).
    let member_params = {
        let Some(world) = parse_target_world(ast, world_form) else {
            return;
        };
        export_member_params(&world, kebab_extern_name)
    };
    apply_export_param_annotations(ast, &member_params);
}

/// Derive export-boundary param annotations from an EXTERNAL `KIND_WIT_WORLD` artifact's ENCODED bytes — the
/// artifact-side entry point of the no-annotation export boundary, called from `compile` BEFORE `Db::load`
/// (the same seam the import `inject_world_import_effects_from_bytes` uses). The flagship + identity +
/// provenance reducers target an external artifact world (`cdz compile <src> wit-world:reducer-world=…`), so
/// this is the real reducer path; the in-source [`derive_world_export_param_annotations`] covers a reducer
/// that declares its world inline. Both reuse the SAME rewrite ([`apply_export_param_annotations`]) — an
/// artifact world decodes to the same `TargetWorld`/`WitInterface` exports as an in-source one. If both a
/// compile-time artifact and an in-source world are present, the artifact runs first and the in-source pass
/// then no-ops on the already-annotated params (the `(: …)`-skip makes them composable, mirroring the import
/// side).
pub fn derive_world_export_param_annotations_from_bytes(ast: &mut Arenas, world_bytes: &[u8]) {
    use crate::backend::common::export_name::kebab_extern_name;
    // The world decodes into its OWN arena; the def-sig rewrite writes into the guest `ast`.
    let Some(world_arena) = crate::codec::decode(world_bytes) else {
        return;
    };
    let Some(world) = parse_target_world(&world_arena, world_arena.root) else {
        return;
    };
    let member_params = export_member_params(&world, kebab_extern_name);
    apply_export_param_annotations(ast, &member_params);
}

/// The (export-member kebab name → ordered param [`WitType`]s) map a guest def's boundary params are derived
/// against — the world's guest-EXPORT interface members, keyed by the SAME `kebab_extern_name` the emit binds
/// a def to an export member with, so front-end derivation and backend emit agree on which member types a def.
fn export_member_params(
    world: &TargetWorld,
    kebab_extern_name: impl Fn(&str) -> String,
) -> std::collections::HashMap<String, Vec<WitType>> {
    let mut m = std::collections::HashMap::new();
    for iface in &world.exports {
        for member in &iface.members {
            m.insert(
                kebab_extern_name(&member.name),
                member.func.params.iter().map(|(_n, t)| t.clone()).collect(),
            );
        }
    }
    m
}

/// Rewrite each top-level guest-export def's BARE boundary params to `(: <param> <derived-type>)` in place,
/// deriving the type from `member_params` (keyed by the def's `kebab_extern_name`). The shared core of both
/// the in-source and artifact export-param derivation. An author-written `(: p T)` WINS (skipped); a position
/// past the member's params, or a type [`wit_type_to_type_expr`] cannot map (`enum`/`variant`/`flags`), is
/// left bare (falls back to the ordinary CDZ0201). A def whose name matches no export member is untouched.
/// The guest's NAMED sums keyed by case-name set (kebab) → the type name, so a WIT `variant`/`enum` in a
/// derived boundary type resolves to the guest's mirroring `type` decl (Cadenza sums carry a decl identity —
/// see [`wit_type_to_type_expr_with_sums`]). Reads every top-level `(type <Name> <case>…)` (a bare-name case
/// or a `(Case payload…)` list); a `type` with no cases (a plain alias) is skipped. If two types share a
/// case-name set the entry is DROPPED (ambiguous — decline to guess rather than mis-bind).
fn guest_sum_names(
    ast: &Arenas,
) -> std::collections::HashMap<std::collections::BTreeSet<String>, String> {
    use crate::backend::common::export_name::kebab_extern_name;
    let mut by_set: std::collections::HashMap<std::collections::BTreeSet<String>, String> =
        std::collections::HashMap::new();
    let mut ambiguous: std::collections::HashSet<std::collections::BTreeSet<String>> =
        std::collections::HashSet::new();
    for it in top_level_items(ast) {
        let Some(tail) = ast.as_form(it, "type") else {
            continue;
        };
        let Some((&name_node, cases)) = tail.split_first() else {
            continue;
        };
        let Some(type_name) = ast.as_name(name_node) else {
            continue;
        };
        if cases.is_empty() {
            continue;
        }
        // Each case's NAME: a bare `Name` (nullary case) or the head of a `(Case payload…)` list.
        let mut set = std::collections::BTreeSet::new();
        let mut ok = true;
        for &c in cases {
            match ast.as_name(c).or_else(|| ast.head_name(c)) {
                Some(n) => {
                    set.insert(kebab_extern_name(n));
                }
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok || set.is_empty() || ambiguous.contains(&set) {
            continue;
        }
        if by_set.insert(set.clone(), type_name.to_string()).is_some() {
            by_set.remove(&set);
            ambiguous.insert(set);
        }
    }
    by_set
}

fn apply_export_param_annotations(
    ast: &mut Arenas,
    member_params: &std::collections::HashMap<String, Vec<WitType>>,
) {
    use crate::ast::Leaf;
    use crate::backend::common::export_name::kebab_extern_name;
    use crate::prelude::{push_atom, push_list};
    if member_params.is_empty() {
        return;
    }
    // The guest's named sums, so a WIT variant/enum in a param type (e.g. the `error` variant inside
    // `on-response`'s `result<…, error>`) resolves to the guest's `type Error` by matching case names.
    let sums = guest_sum_names(ast);
    for it in top_level_items(ast) {
        // `(def (<name> param…) body)` — the signature is the def's first child, a list whose head is the
        // def name and whose tail is the params.
        let Some(&sig) = ast.as_form(it, "def").and_then(|t| t.first()) else {
            continue;
        };
        let Struct::List(sig_items) = ast.get(sig) else {
            continue;
        };
        let sig_items = sig_items.clone();
        let Some(def_name) = sig_items
            .first()
            .and_then(|&n| ast.as_name(n))
            .map(kebab_extern_name)
        else {
            continue;
        };
        let Some(ptys) = member_params.get(&def_name).cloned() else {
            continue;
        };
        let mut new_sig = sig_items.clone();
        let mut changed = false;
        for (i, ptype) in ptys.iter().enumerate() {
            // `sig_items[0]` is the def name; param i sits at position i+1.
            let Some(&param_node) = sig_items.get(i + 1) else {
                break;
            };
            // Skip an already-annotated binder `(: p T)` — the author's annotation wins.
            if ast.as_form(param_node, ":").is_some() {
                continue;
            }
            let Some(pname) = ast.as_name(param_node).map(str::to_string) else {
                continue;
            };
            let Some(type_expr) = wit_type_to_type_expr_with_sums(ast, ptype, &sums) else {
                continue;
            };
            // Build `(: <param-name> <derived-type-expr>)` and replace the bare binder in place.
            let colon = push_atom(ast, Leaf::Name(":".into()));
            let name = push_atom(ast, Leaf::Name(pname.into()));
            let annotated = push_list(ast, vec![colon, name, type_expr]);
            new_sig[i + 1] = annotated;
            changed = true;
        }
        if changed {
            ast.structure[sig.0 as usize] = Struct::List(new_sig);
        }
    }
}

/// The module's DIRECT top-level members (`(module NAME item…)` → `item…`; a bare root → itself), mirroring
/// `db.top_world_forms`' root-member reckoning — a form nested inside a def is not a module member.
fn top_level_items(ast: &Arenas) -> Vec<StructId> {
    // A bare source file (no explicit `(module …)` wrapper — the natural reducer authoring form) parses to a
    // `(do item…)` root; a wrapped module to `(module NAME item…)`. Return the ITEMS in either case so the
    // world scan sees the direct top-level members (mirrors `proptest_gen` / `param_sidecar`, which also
    // treat a `(do …)` root as the top-level item list). Without the `do` arm an in-source `(world …)` in a
    // bare file was invisible → the sidecar synthesized nothing → every import cascaded to CDZ0101.
    if let Some(do_items) = ast.as_form(ast.root, "do") {
        return do_items.to_vec();
    }
    match ast.get(ast.root) {
        Struct::List(_) if ast.as_form(ast.root, "module").is_some() => ast
            .as_form(ast.root, "module")
            .and_then(|t| t.get(1..))
            .unwrap_or(&[])
            .to_vec(),
        _ => vec![ast.root],
    }
}

/// The FIRST in-source top-level `(world …)` form, if the module declares one. A `(world …)` nested inside a
/// def is not a module world target.
fn top_world_form(ast: &Arenas) -> Option<StructId> {
    top_level_items(ast)
        .into_iter()
        .find(|&it| ast.head_name(it) == Some("world"))
}

/// Append `member` as a top-level member of the `(module NAME …)` root (or a bare-root wrapped as a list).
/// The synthesized effect is then visible to the ordinary compile exactly like a hand-written declaration.
/// (Mirrors `param_sidecar::append_module_member`.)
fn append_module_member(ast: &mut Arenas, member: StructId) {
    let root = ast.root;
    if let Struct::List(items) = ast.get(root) {
        let mut new_items = items.clone();
        let insert_at = if ast.as_form(root, "module").is_some() && new_items.len() >= 2 {
            2
        } else {
            new_items.len()
        };
        new_items.insert(insert_at, member);
        ast.structure[root.0 as usize] = Struct::List(new_items);
    }
}

/// A kebab WIT case identifier (`not-found`) → the Cadenza constructor identifier (`NotFound`) — the inverse
/// of `kebab_extern_name`'s Pascal→kebab, so a `guest_sum_names` re-scan kebabs the synthesized constructor
/// back to the SAME case-set the WIT `enum` keys on.
fn pascal_of_kebab(s: &str) -> String {
    s.split('-')
        .filter(|seg| !seg.is_empty())
        .map(|seg| {
            let mut c = seg.chars();
            match c.next() {
                Some(f) => f.to_ascii_uppercase().to_string() + c.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

/// Collect every `WitType::Enum` case-set (kebab) appearing anywhere in `t` — recursing through
/// list/tuple/record/option/result element+field types and variant payloads — into `out`, keyed by the kebab
/// case-set → the original (raw) case-name list. Deduped by set (the same enum shape in >1 position is one
/// synthesized type). VARIANT case-sets are NOT collected here (payloaded variants are a later sub-slice; a
/// variant's payloads ARE walked for nested enums).
type NominalSumSpec = Vec<(String, Option<WitType>)>;

/// Collect every anonymous nominal-sum shape (`enum` OR `variant`) appearing anywhere in `t` — recursing
/// through list/tuple/record/option/result element+field types and variant payloads — into `out`, keyed by
/// the kebab case-set → the ordered `(raw case name, optional payload type)` list. Deduped by set (the same
/// shape in >1 position is one synthesized type). An `enum` case carries no payload (`None`); a `variant`
/// case carries its declared payload (or `None` for a payload-less case).
fn collect_nominal_sum_specs(
    t: &WitType,
    out: &mut std::collections::BTreeMap<std::collections::BTreeSet<String>, NominalSumSpec>,
) {
    use crate::backend::common::export_name::kebab_extern_name;
    match t {
        WitType::Enum(cases) => {
            let set: std::collections::BTreeSet<String> =
                cases.iter().map(|c| kebab_extern_name(c)).collect();
            out.entry(set)
                .or_insert_with(|| cases.iter().map(|c| (c.clone(), None)).collect());
        }
        WitType::Variant(cases) => {
            let set: std::collections::BTreeSet<String> =
                cases.iter().map(|(c, _)| kebab_extern_name(c)).collect();
            out.entry(set)
                .or_insert_with(|| cases.iter().map(|(c, p)| (c.clone(), p.clone())).collect());
            // A payload may itself contain a nested anonymous sum.
            for (_, p) in cases {
                if let Some(pt) = p {
                    collect_nominal_sum_specs(pt, out);
                }
            }
        }
        WitType::List(e) | WitType::Option(e) => collect_nominal_sum_specs(e, out),
        WitType::Tuple(es) => es.iter().for_each(|e| collect_nominal_sum_specs(e, out)),
        WitType::Record(fs) => fs
            .iter()
            .for_each(|(_, e)| collect_nominal_sum_specs(e, out)),
        WitType::Result { ok, err } => {
            if let Some(o) = ok {
                collect_nominal_sum_specs(o, out);
            }
            if let Some(e) = err {
                collect_nominal_sum_specs(e, out);
            }
        }
        _ => {}
    }
}

/// WIT NOMINAL-SUM SELF-DECLARATION (operator ruling 2026-08-28 — full WIT type algebra: an IMPOSED world's
/// anonymous `enum`/`variant` with NO guest mirror sum gets a SYNTHESIZED nominal type — an INTERNAL name +
/// NAMEABLE case constructors — so the guest performs the import with zero hand-decl). Scans the world's
/// IMPORT op param/result types for `enum`/`variant` case-sets not already covered by a guest `(type …)` decl
/// (`guest_sum_names`) and injects a `(type <SynthName> <Ctor>…)` / `(type <SynthName> (<Ctor> <payload>)…)`
/// per missing set. Injected BEFORE [`synthesize_world_import_effect_decls`], so its internal
/// `guest_sum_names` re-scan sees the synthesized decl and the sum-typed op MAPS instead of being SKIPPED
/// (`wit_type_to_type_expr_with_sums` resolves the case-set to the synthesized type's bare name). The type
/// name is INTERNAL (`Wit<Ctors>`, disambiguated against existing top-level types); per the ruling the guest
/// references the nameable case constructors, not this name. A variant whose PAYLOAD type does not map (a
/// nested anonymous sum — a later sub-slice) is skipped, leaving the op skipped as before; `flags` has no
/// Cadenza surface (out of scope).
fn synthesize_missing_nominal_decls(ast: &mut Arenas, world_bytes: &[u8]) {
    use crate::ast::Leaf;
    use crate::backend::common::export_name::kebab_extern_name;
    use crate::prelude::{push_atom, push_list};
    let Some(world_arena) = crate::codec::decode(world_bytes) else {
        return;
    };
    let Some(world) = parse_target_world(&world_arena, world_arena.root) else {
        return;
    };
    // Nominal-sum specs appearing in any IMPORT op param/result (deduped by kebab set → cases with payloads).
    let mut specs: std::collections::BTreeMap<std::collections::BTreeSet<String>, NominalSumSpec> =
        std::collections::BTreeMap::new();
    for iface in &world.imports {
        for member in &iface.members {
            for (_pname, pty) in &member.func.params {
                collect_nominal_sum_specs(pty, &mut specs);
            }
            collect_nominal_sum_specs(&member.func.result, &mut specs);
        }
    }
    if specs.is_empty() {
        return;
    }
    // Case-sets a guest type ALREADY covers → the mirror wins, do not synthesize (avoids the ambiguous
    // two-types-one-set drop that would then leave the op unmapped).
    let existing = guest_sum_names(ast);
    // Existing top-level type NAMES → disambiguate the internal synth name against them.
    let mut names: std::collections::HashSet<String> = top_level_items(ast)
        .into_iter()
        .filter_map(|it| ast.as_form(it, "type").and_then(|t| t.first().copied()))
        .filter_map(|n| ast.as_name(n).map(|s| s.to_string()))
        .collect();
    let empty_sums = std::collections::HashMap::new();
    for (set, cases) in &specs {
        if existing.contains_key(set) {
            continue;
        }
        let ctor_idents: Vec<String> = cases
            .iter()
            .map(|(c, _)| pascal_of_kebab(&kebab_extern_name(c)))
            .collect();
        // Build each case node — a bare `Ctor` (payload-less) or `(Ctor <payload-type-expr>)`. A payload
        // whose type-expr does not map (a nested anonymous sum: a later sub-slice) makes the WHOLE nominal
        // unbuildable → skip it, leaving the op skipped as before. (The unreferenced payload nodes built for
        // a skipped nominal stay inert in the arena — not scanned as members.)
        let mut case_nodes: Vec<StructId> = Vec::with_capacity(cases.len());
        let mut buildable = true;
        for ((_raw, payload), ctor) in cases.iter().zip(&ctor_idents) {
            match payload {
                None => case_nodes.push(push_atom(ast, Leaf::Name(ctor.as_str().into()))),
                Some(pt) => {
                    let Some(pe) = wit_type_to_type_expr_with_sums(ast, pt, &empty_sums) else {
                        buildable = false;
                        break;
                    };
                    let head = push_atom(ast, Leaf::Name(ctor.as_str().into()));
                    case_nodes.push(push_list(ast, vec![head, pe]));
                }
            }
        }
        if !buildable {
            continue;
        }
        let mut tyname = format!("Wit{}", ctor_idents.join(""));
        let mut n = 2;
        while names.contains(&tyname) {
            tyname = format!("Wit{}{}", ctor_idents.join(""), n);
            n += 1;
        }
        names.insert(tyname.clone());
        let mut kids = vec![
            push_atom(ast, Leaf::Name("type".into())),
            push_atom(ast, Leaf::Name(tyname.as_str().into())),
        ];
        kids.extend(case_nodes);
        let decl = push_list(ast, kids);
        append_module_member(ast, decl);
    }
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

/// The DERIVED signature of a world-IMPORT member — its `(param types, result type)` in Cadenza `Ty`,
/// mapped from the declared `world.wit` func signature via [`wit_type_to_ty`]. This is the piece that lets a
/// guest `perform` a NAMED world import WITHOUT a mirrored `(effect …)` decl (v-platform ruling 2026-08-23):
/// the compiler, compiling a guest FOR a world, already knows that world's import signatures, so it derives
/// the op's arg/result types straight from the WIT contract — one generic rule, ZERO per-interface arms, no
/// guest/WIT drift.
///
/// `iface`/`op` match the SAME way [`is_world_import_op`] does — the world's FQ import name's last
/// `/`-segment, kebab-normalized on both sides (so a source effect `Kv` binds `cadenza:agent-kernel/kv`).
/// A NULLARY op yields an EMPTY param vec (e.g. `identity.id : () -> reducer-id` → `([], Bytes)`), so the
/// caller need not commit to any curried-arrow encoding. Returns `None` when the world is absent/undecodable,
/// the member is not a declared import, or any param/result type is one [`wit_type_to_ty`] does not yet map
/// (a WIT `enum`/`variant` — e.g. `graph.neighbors`'s `dir` enum or `run.run`'s `error` — until the
/// synthesized-nominal-decl increment lands).
pub fn world_import_member_sig(
    db: &crate::db::Db,
    world_bytes: Option<&[u8]>,
    iface: &str,
    op: &str,
) -> Option<(Vec<Ty>, Ty)> {
    let bytes = world_bytes?;
    let arenas = crate::codec::decode(bytes)?;
    let world = parse_target_world(&arenas, arenas.root)?;
    use crate::backend::common::export_name::kebab_extern_name;
    fn short(fq: &str) -> &str {
        fq.rsplit('/').next().unwrap_or(fq)
    }
    let iface_k = kebab_extern_name(iface);
    let op_k = kebab_extern_name(op);
    let interface = world
        .imports
        .iter()
        .find(|i| kebab_extern_name(short(&i.name)) == iface_k)?;
    let member = interface
        .members
        .iter()
        .find(|m| kebab_extern_name(&m.name) == op_k)?;
    let mut params = Vec::with_capacity(member.func.params.len());
    for (_pname, pty) in &member.func.params {
        params.push(wit_type_to_ty(db, pty)?);
    }
    let result = wit_type_to_ty(db, &member.func.result)?;
    Some((params, result))
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
    fn a_migrated_record_field_pair_world_parses_through_the_real_reader() {
        // Regression: the 28-wit-abi record-result worlds (sp1/sp3/sp3n/sp6) reach the reader as
        // `(record (= a (s64)) …)` — name-head `record` (seq-206) with native FieldPair `(= name ty)` field
        // entries (the reader's compound form, mirroring a value-model record's fields). parse_wit_type's
        // record arm read a 2-list `(name ty)` and rejected the 3-element FieldPair → parse_target_world
        // returned None → the whole world silently dropped → `record_interface_export` bailed before its body
        // and every record-result export declined. Read through the REAL front-end reader (testkit::parse) so
        // the field entries are the ACTUAL FieldPair nodes the corpus produces, not hand-built 2-lists.
        for (label, src) in [
            (
                "sp1",
                "(world w (export iface (member f (func (param x (s64)) (result (record (= b1 (s64)) (= b2 (s64))))))))",
            ),
            (
                "sp3",
                "(world w (export iface (member f (func (param x (s64)) (result (record (= a (s64)) (= d (option (s64)))))))))",
            ),
            (
                "sp6",
                "(world w (export iface (member f (func (param x (s64)) (param y (s64)) (result (record (= b1 (s64)) (= b2 (s64))))))))",
            ),
        ] {
            let a = crate::testkit::parse(src);
            assert!(
                parse_target_world(&a, a.root).is_some(),
                "{label} record-result world must parse; got None (world drops → export declines)"
            );
        }
        // Pin the nested shape too (sp3): the result decodes to a record whose second field is an option —
        // proves the FieldPair read recurses into a nested name-head `(option (s64))`, declaration order kept.
        let a = crate::testkit::parse(
            "(world w (export iface (member f (func (param x (s64)) (result (record (= a (s64)) (= d (option (s64)))))))))",
        );
        let tw = parse_target_world(&a, a.root).expect("sp3 world parses");
        let result = &tw.exports[0].members[0].func.result;
        assert_eq!(
            result,
            &WitType::Record(vec![
                ("a".to_string(), WitType::S64),
                ("d".to_string(), WitType::Option(Box::new(WitType::S64))),
            ]),
            "sp3 result: record {{ a: s64, d: option<s64> }} in declaration order"
        );
    }

    #[test]
    fn the_descriptor_vocabulary_parses_through_the_real_reader() {
        // BOTH #5710-family parse bugs (string-head primitive #5751, record FieldPair fields #5763) slipped
        // through because the unit tests HAND-BUILD descriptor nodes via `Builder` (2-lists, str-heads) that
        // diverge from what the real front-end reader actually emits (native FieldPair, name-heads, ctor
        // leaves). This pins the WHOLE 28-wit-abi descriptor vocabulary — variant (payload + bare case), enum,
        // result-of/enum-err, list, option, unit, and DEEP record/variant/option nesting — as READ THROUGH the
        // real reader (testkit::parse), so a future reader-representation change that breaks any descriptor
        // shape fails HERE, not silently at a dropped world months later. Each world must parse (Some).
        let worlds = [
            // sp7: variant result with a payloaded case + a bare case.
            "(world w (export iface (member f (func (param x (s64)) (result (variant (small (s64)) (big)))))))",
            // result<list<u8>, variant<..>> — nested result/list/variant, the platform run member's shape.
            "(world w (export guest (member run (func (param program (list (u8))) (result (result (list (u8)) (variant (timeout) (faulted))))))))",
            // record param carrying a result<list<u8>, enum<..>> — result + enum + list nested in a record field.
            "(world w (export iface (member f (func (param m (record (= a (result (list (u8)) (enum timeout missing schema faulted))))) (result (s64))))))",
            // option param + unit result — the bare option + unit vocabulary.
            "(world w (export iface (member f (func (param m (record (= d (option (s64))))) (result (unit))))))",
            // Deeply nested: record → list → record → option field (the on-message request shape), exercising
            // the FieldPair read recursing several levels through name-head compounds.
            "(world w (export guest (member on-message (func (param m (record (= contract (list (u8))) (= payload (list (u8))))) (result (record (= items (list (record (= echo (list (u8))) (= deadline (option (u64)))))))))) ))",
        ];
        for (i, src) in worlds.iter().enumerate() {
            let a = crate::testkit::parse(src);
            assert!(
                parse_target_world(&a, a.root).is_some(),
                "descriptor-vocabulary world #{i} must parse through the real reader; got None:\n  {src}"
            );
        }
        // Pin one rich shape's decoded value: the variant result (sp7) — a payloaded case + a bare case, in
        // declaration order (a variant's case order is significant).
        let a = crate::testkit::parse(
            "(world w (export iface (member f (func (param x (s64)) (result (variant (small (s64)) (big)))))))",
        );
        let tw = parse_target_world(&a, a.root).expect("sp7 variant world parses");
        assert_eq!(
            tw.exports[0].members[0].func.result,
            WitType::Variant(vec![
                ("small".to_string(), Some(WitType::S64)),
                ("big".to_string(), None),
            ]),
            "sp7 result: variant {{ small(s64), big }} in declaration order"
        );
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
    fn parses_name_head_compounds_identically_to_string_head() {
        // seq-206: compound type heads are NAMES `(list <e>)` (like the scalar primitives `(s64)`), not
        // strings `("list" <e>)`. A name head must decode identically to the legacy string head (back-compat).
        let name_head = |b: &mut Builder, h: &str, kids: Vec<StructId>| {
            let head = b.name(h);
            let mut v = vec![head];
            v.extend(kids);
            b.list(v)
        };
        // (list (s64)) name-headed → List(S64), same as the legacy ("list" (s64)).
        let mut b = Builder::new();
        let s64 = prim(&mut b, "s64");
        let list = name_head(&mut b, "list", vec![s64]);
        let a = b.finish(list);
        assert_eq!(
            parse_wit_type(&a, list),
            Some(WitType::List(Box::new(WitType::S64))),
            "name-head (list …)"
        );
        // (record (a (s64)) (d (option (s64)))) name-headed, with a nested name-head (option …) — the sp3 shape.
        let mut b = Builder::new();
        let a_ty = prim(&mut b, "s64");
        let a_name = b.name("a");
        let a_field = b.list(vec![a_name, a_ty]);
        let opt_inner = prim(&mut b, "s64");
        let opt = name_head(&mut b, "option", vec![opt_inner]);
        let d_name = b.name("d");
        let d_field = b.list(vec![d_name, opt]);
        let rec = name_head(&mut b, "record", vec![a_field, d_field]);
        let a = b.finish(rec);
        assert_eq!(
            parse_wit_type(&a, rec),
            Some(WitType::Record(vec![
                ("a".to_string(), WitType::S64),
                ("d".to_string(), WitType::Option(Box::new(WitType::S64))),
            ])),
            "name-head (record …) with nested name-head (option …)"
        );
        // (none) name-head result-slot marker → absent arm, parity with the legacy ("none").
        let mut b = Builder::new();
        let none = {
            let h = b.name("none");
            b.list(vec![h])
        };
        let a = b.finish(none);
        assert_eq!(
            parse_result_slot(&a, none),
            Some(None),
            "name-head (none) is the absent result arm"
        );
    }

    #[test]
    fn a_string_head_primitive_is_malformed_not_back_compat() {
        // seq-206 back-compat accepts a legacy STRING head ONLY for COMPOUNDS (`("list" …)`) — the spelling
        // the corpus migrates from. PRIMITIVES were ALWAYS name-head (`(bool)`), so a string-head `("bool")`
        // was NEVER a legacy form: it must DECLINE (→ None), which lets `collect_faults` report the malformed
        // world instead of silently dropping it (regression guard for the #5710 over-broad head unification,
        // which briefly accepted `("bool")` as Bool and swallowed the malformed diagnostic).
        for kind in ["bool", "u8", "s64", "char", "string", "f64"] {
            let mut b = Builder::new();
            let head = str_head(&mut b, kind);
            let root = b.list(vec![head]);
            let a = b.finish(root);
            assert_eq!(
                parse_wit_type(&a, root),
                None,
                "string-head primitive (\"{kind}\") must be malformed, not accepted"
            );
        }
    }

    #[test]
    fn declines_a_not_yet_covered_compound() {
        // A resource handle ("own" …) is the one remaining uncovered component-model type → None, never a
        // misread. (variant/enum/result/flags ARE covered now — see the tests below.)
        let mut b = Builder::new();
        let inner = prim(&mut b, "u32");
        let head = str_head(&mut b, "own");
        let root = b.list(vec![head, inner]);
        let a = b.finish(root);
        assert_eq!(parse_wit_type(&a, root), None, "unknown compound declines");
    }

    #[test]
    fn parses_a_variant_with_a_payload_case_and_a_payload_less_case() {
        // ("variant" (continue) (close ("record" (n (u8))))) — outcome-shaped: a bare case + a payload case.
        let mut b = Builder::new();
        let cont_name = b.name("continue");
        let cont = b.list(vec![cont_name]); // 1-list → payload-less
        let n_ty = prim(&mut b, "u8");
        let n_name = b.name("n");
        let field = b.list(vec![n_name, n_ty]);
        let rec_head = str_head(&mut b, "record");
        let rec = b.list(vec![rec_head, field]);
        let close_name = b.name("close");
        let close = b.list(vec![close_name, rec]); // 2-list → payload case
        let head = str_head(&mut b, "variant");
        let root = b.list(vec![head, cont, close]);
        let a = b.finish(root);
        assert_eq!(
            parse_wit_type(&a, root),
            Some(WitType::Variant(vec![
                ("continue".to_string(), None),
                (
                    "close".to_string(),
                    Some(WitType::Record(vec![("n".to_string(), WitType::U8)])),
                ),
            ])),
            "variant keeps case order + optional payloads"
        );
    }

    #[test]
    fn parses_an_enum_of_bare_cases() {
        // ("enum" timeout missing-handler schema-violation) — the error-shaped enum, all payload-less.
        let mut b = Builder::new();
        let c0 = b.name("timeout");
        let c1 = b.name("missing-handler");
        let c2 = b.name("schema-violation");
        let head = str_head(&mut b, "enum");
        let root = b.list(vec![head, c0, c1, c2]);
        let a = b.finish(root);
        assert_eq!(
            parse_wit_type(&a, root),
            Some(WitType::Enum(vec![
                "timeout".to_string(),
                "missing-handler".to_string(),
                "schema-violation".to_string(),
            ])),
            "enum keeps case order"
        );
    }

    #[test]
    fn parses_flags_like_an_enum_but_distinct() {
        // ("flags" read write) — distinct WitType from the same-shaped enum.
        let mut b = Builder::new();
        let f0 = b.name("read");
        let f1 = b.name("write");
        let head = str_head(&mut b, "flags");
        let root = b.list(vec![head, f0, f1]);
        let a = b.finish(root);
        assert_eq!(
            parse_wit_type(&a, root),
            Some(WitType::Flags(vec![
                "read".to_string(),
                "write".to_string()
            ])),
        );
    }

    #[test]
    fn parses_result_of_bytes_and_an_enum_err() {
        // ("result" ("list" (u8)) ("enum" timeout)) — the response answer shape: result<payload, error>.
        let mut b = Builder::new();
        let u8p = prim(&mut b, "u8");
        let lhead = str_head(&mut b, "list");
        let ok = b.list(vec![lhead, u8p]);
        let ecase = b.name("timeout");
        let ehead = str_head(&mut b, "enum");
        let err = b.list(vec![ehead, ecase]);
        let head = str_head(&mut b, "result");
        let root = b.list(vec![head, ok, err]);
        let a = b.finish(root);
        assert_eq!(
            parse_wit_type(&a, root),
            Some(WitType::Result {
                ok: Some(Box::new(WitType::List(Box::new(WitType::U8)))),
                err: Some(Box::new(WitType::Enum(vec!["timeout".to_string()]))),
            }),
            "result<list<u8>, enum> decodes both arms"
        );
    }

    #[test]
    fn parses_result_with_an_absent_ok_arm() {
        // ("result" ("none") (u32)) — result<_, u32>: an omitted ok arm decodes to None, distinct from unit.
        let mut b = Builder::new();
        let none_head = str_head(&mut b, "none");
        let ok = b.list(vec![none_head]);
        let err = prim(&mut b, "u32");
        let head = str_head(&mut b, "result");
        let root = b.list(vec![head, ok, err]);
        let a = b.finish(root);
        assert_eq!(
            parse_wit_type(&a, root),
            Some(WitType::Result {
                ok: None,
                err: Some(Box::new(WitType::U32)),
            }),
            "absent ok arm is None, not Some(Unit)"
        );
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
    fn natural_wit_of_unit_is_wit_unit_and_round_trips() {
        // The unit OUTBOUND arm (WIT-shape-coverage matrix v1): a synthesized-world op result of `Unit`
        // self-declares `unit`, the exact inverse of `wit_type_to_ty`'s `WitType::Unit → Ty::Unit`.
        // Previously `Ty::Unit` fell into the None group (an asymmetry: inbound `unit` mapped, outbound
        // did not). Pin both the arm and the Ty→WIT→Ty round-trip so the inverse can't silently drift.
        assert_eq!(ty_natural_wit(&Ty::Unit), Some(WitType::Unit));
        let db = crate::db::Db::load(crate::testkit::parse(
            "(module m (def (main) 0) (export main))",
        ));
        assert_eq!(
            wit_type_to_ty(&db, &ty_natural_wit(&Ty::Unit).unwrap()),
            Some(Ty::Unit)
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

    /// The FORWARD canonical mapping `wit_type_to_ty` (v-platform oracle rule 2026-08-23) covers the whole
    /// structural + Option/Result span the platform `world.wit` uses — generic over any import member, zero
    /// per-interface arms. Pins each shape against the oracle target, and the deferred nominal shapes
    /// (enum/variant/flags → None, need a synthesized decl).
    #[test]
    fn wit_type_to_ty_maps_the_canonical_world_import_shapes() {
        use crate::db::Db;
        use crate::ty::FloatTy;
        // A Db carrying the prelude (so Option/Result decls resolve for normalize_sum).
        let db = Db::load(crate::testkit::parse(
            "(module m (def (main) 0) (export main))",
        ));
        let byte_list = || WitType::List(Box::new(WitType::U8));

        // Scalars + widths + the byte-list→Bytes collapse.
        assert_eq!(wit_type_to_ty(&db, &WitType::Bool), Some(Ty::Bool));
        assert_eq!(wit_type_to_ty(&db, &WitType::String), Some(Ty::String));
        assert_eq!(wit_type_to_ty(&db, &WitType::Unit), Some(Ty::Unit));
        assert_eq!(
            wit_type_to_ty(&db, &WitType::U64),
            Some(Ty::Int(IntTy::fixed(false, 64)))
        );
        assert_eq!(
            wit_type_to_ty(&db, &WitType::S32),
            Some(Ty::Int(IntTy::fixed(true, 32)))
        );
        assert_eq!(
            wit_type_to_ty(&db, &WitType::F64),
            Some(Ty::Float(FloatTy::fixed(64)))
        );
        // list<u8> → Bytes (identity.id / blobs / hashes / tokens all collapse here).
        assert_eq!(wit_type_to_ty(&db, &byte_list()), Some(Ty::Bytes));
        // A non-byte list stays a List of the mapped element.
        assert_eq!(
            wit_type_to_ty(&db, &WitType::List(Box::new(WitType::U32))),
            Some(Ty::List(Box::new(Ty::Int(IntTy::fixed(false, 32)))))
        );

        // record → Record (canonically sorted; the deliver `message` shape, all Bytes, maps fully and
        // recursively — nested sender record included).
        let message = WitType::Record(vec![
            ("contract".into(), byte_list()),
            (
                "sender".into(),
                WitType::Record(vec![
                    ("reducer".into(), byte_list()),
                    ("host".into(), byte_list()),
                ]),
            ),
            ("payload".into(), byte_list()),
            ("token".into(), byte_list()),
        ]);
        match wit_type_to_ty(&db, &message).expect("the deliver message record maps") {
            Ty::Record(fields) => {
                assert_eq!(fields.len(), 4, "four top-level fields");
                assert_eq!(
                    fields.get(&crate::resolved::Symbol::plain("payload")),
                    Some(&Ty::Bytes)
                );
                match fields.get(&crate::resolved::Symbol::plain("sender")) {
                    Some(Ty::Record(inner)) => assert_eq!(
                        inner.get(&crate::resolved::Symbol::plain("reducer")),
                        Some(&Ty::Bytes),
                        "the nested sender record maps recursively"
                    ),
                    other => panic!("sender must be a nested Record, got {other:?}"),
                }
            }
            other => panic!("a WIT record maps to Ty::Record, got {other:?}"),
        }

        // option<bytes> (state.get / blobs.get) → the prelude Option instantiated at Bytes.
        let want_opt = {
            let occ = db
                .type_decls
                .iter()
                .find(|t| t.name == "Option")
                .unwrap()
                .occ;
            db.normalize_sum(occ, vec![Ty::Bytes])
        };
        assert_eq!(
            wit_type_to_ty(&db, &WitType::Option(Box::new(byte_list()))),
            Some(want_opt)
        );
        // result<list<u8>, _> with an absent err → the prelude Result(Bytes, Unit).
        let want_res = {
            let occ = db
                .type_decls
                .iter()
                .find(|t| t.name == "Result")
                .unwrap()
                .occ;
            db.normalize_sum(occ, vec![Ty::Bytes, Ty::Unit])
        };
        assert_eq!(
            wit_type_to_ty(
                &db,
                &WitType::Result {
                    ok: Some(Box::new(byte_list())),
                    err: None,
                }
            ),
            Some(want_res)
        );

        // Deferred nominal shapes need a synthesized decl → None (the `Dir` enum, the `Error` variant).
        assert_eq!(
            wit_type_to_ty(
                &db,
                &WitType::Enum(vec!["outgoing".into(), "incoming".into()])
            ),
            None,
            "an enum needs a synthesized nominal sum decl (later increment)"
        );
        assert_eq!(
            wit_type_to_ty(
                &db,
                &WitType::Variant(vec![("timeout".into(), None), ("faulted".into(), None)])
            ),
            None,
            "a variant needs a synthesized nominal sum decl (later increment)"
        );
    }

    /// `world_import_member_sig` DERIVES a world-import member's `(param types, result type)` straight from
    /// the declared `world.wit` — the piece that lets a guest perform a NAMED import with no mirrored
    /// `(effect …)` decl. Pins the tricky shapes: a NULLARY op → empty params (identity.id), option/unit
    /// results (state.get/put), and the deferred enum → `None` (until the synthesized-decl increment).
    #[test]
    fn world_import_member_sig_derives_the_arg_and_result_types_from_world_wit() {
        use crate::db::Db;
        let db = Db::load(crate::testkit::parse(
            "(module m (def (main) 0) (export main))",
        ));
        let mut b = Builder::new();
        // import things { id: () -> list<u8>; get: (list<u8>) -> option<list<u8>>;
        //                 put: (list<u8>, list<u8>) -> unit; pick: (enum{outgoing,incoming}) -> unit }
        let id_res = byte_list(&mut b);
        let id = member(&mut b, "id", vec![], id_res);
        let get_k = byte_list(&mut b);
        let get_inner = byte_list(&mut b);
        let get_res = opt(&mut b, get_inner);
        let get = member(&mut b, "get", vec![("key", get_k)], get_res);
        let put_k = byte_list(&mut b);
        let put_v = byte_list(&mut b);
        let put_res = unit(&mut b);
        let put = member(
            &mut b,
            "put",
            vec![("key", put_k), ("value", put_v)],
            put_res,
        );
        let en = {
            let eh = str_head(&mut b, "enum");
            let a1 = b.name("outgoing");
            let a2 = b.name("incoming");
            b.list(vec![eh, a1, a2])
        };
        let pick_res = unit(&mut b);
        let pick = member(&mut b, "pick", vec![("dir", en)], pick_res);
        // set-edges: (list<u8>, list<u8>, list<list<u8>>) -> list<list<u8>> — the privileged
        // graph.set-edges shape: 3 args, a NESTED list<list<u8>> (list of reducer-ids) param + result. The
        // inner list<u8> collapses to Bytes, so list<list<u8>> derives to List(Bytes).
        let list_of_bytes = |b: &mut Builder| {
            let lh = str_head(b, "list");
            let inner = byte_list(b);
            b.list(vec![lh, inner])
        };
        let se_src = byte_list(&mut b);
        let se_kind = byte_list(&mut b);
        let se_edges = list_of_bytes(&mut b);
        let se_res = list_of_bytes(&mut b);
        let set_edges = member(
            &mut b,
            "set-edges",
            vec![("src", se_src), ("kind", se_kind), ("edges", se_edges)],
            se_res,
        );
        let ih = b.name("import");
        let inm = b.name("things");
        let things = b.list(vec![ih, inm, id, get, put, pick, set_edges]);
        let wh = b.name("world");
        let wn = b.name("reducer");
        let world = b.list(vec![wh, wn, things]);
        let a = b.finish(world);
        let bytes = crate::codec::encode(&a);
        let w = Some(bytes.as_slice());

        // NULLARY op → empty params, Bytes result (the identity.id : () -> reducer-id shape).
        assert_eq!(
            world_import_member_sig(&db, w, "things", "id"),
            Some((vec![], Ty::Bytes))
        );
        // (list<u8>) -> option<list<u8>> → ([Bytes], Option Bytes).
        let opt_bytes = {
            let occ = db
                .type_decls
                .iter()
                .find(|t| t.name == "Option")
                .unwrap()
                .occ;
            db.normalize_sum(occ, vec![Ty::Bytes])
        };
        assert_eq!(
            world_import_member_sig(&db, w, "things", "get"),
            Some((vec![Ty::Bytes], opt_bytes))
        );
        // (list<u8>, list<u8>) -> unit → ([Bytes, Bytes], Unit).
        assert_eq!(
            world_import_member_sig(&db, w, "things", "put"),
            Some((vec![Ty::Bytes, Ty::Bytes], Ty::Unit))
        );
        // 3-arg + nested list<list<u8>> param/result (graph.set-edges shape) →
        // ([Bytes, Bytes, List Bytes], List Bytes) — the inner list<u8> collapses to Bytes.
        assert_eq!(
            world_import_member_sig(&db, w, "things", "set-edges"),
            Some((
                vec![Ty::Bytes, Ty::Bytes, Ty::List(Box::new(Ty::Bytes))],
                Ty::List(Box::new(Ty::Bytes))
            ))
        );
        // An enum param is not yet mapped → None (deferred synthesized-decl increment).
        assert_eq!(world_import_member_sig(&db, w, "things", "pick"), None);
        // A non-member / non-interface / absent world → None.
        assert_eq!(world_import_member_sig(&db, w, "things", "nope"), None);
        assert_eq!(world_import_member_sig(&db, w, "other", "id"), None);
        assert_eq!(world_import_member_sig(&db, None, "things", "id"), None);
    }

    /// `wit_type_to_type_expr` produces a canonical type-AST node that `typeval_of` reads back to EXACTLY the
    /// `Ty` `wit_type_to_ty` derives — the round-trip that guarantees a synthesized `(effect …)` decl's
    /// injected arrow types resolve to the intended world-import signature (the b-prime no-redeclare
    /// foundation). Covers the full mappable span; enum/variant/flags → None (no type-expr yet).
    #[test]
    fn wit_type_to_type_expr_round_trips_to_the_derived_ty() {
        use crate::db::Db;
        let mut db = Db::load(crate::testkit::parse(
            "(module m (def (main) 0) (export main))",
        ));
        let byte = || WitType::List(Box::new(WitType::U8));
        let cases = vec![
            WitType::Bool,
            WitType::String,
            WitType::Unit,
            WitType::U64,
            WitType::S32,
            byte(),                                // list<u8> → Bytes
            WitType::List(Box::new(WitType::U32)), // list<u32> → List(Int)
            WitType::Option(Box::new(byte())),     // option<list<u8>> → Option Bytes
            WitType::Tuple(vec![WitType::Bool, byte()]),
            WitType::Record(vec![
                ("contract".into(), byte()),
                ("payload".into(), byte()),
            ]),
            WitType::Result {
                ok: Some(Box::new(byte())),
                err: None,
            },
        ];
        for wt in &cases {
            let want = wit_type_to_ty(&db, wt).expect("mappable");
            let node = wit_type_to_type_expr(&mut db.ast, wt).expect("type-expr builds");
            let got = crate::eval::typeval_of(&mut db, node).unwrap_or_else(|| {
                panic!("the injected type-expr for {wt:?} must resolve to a type")
            });
            assert_eq!(
                got, want,
                "type-expr round-trips to the derived Ty for {wt:?}"
            );
        }
        // enum/variant/flags have no type-expr yet (need a synthesized nominal decl).
        assert_eq!(
            wit_type_to_type_expr(&mut db.ast, &WitType::Enum(vec!["a".into(), "b".into()])),
            None
        );
    }

    /// `synthesize_world_import_effect_decls` builds one `(effect <short-name> (op <op> <arrow>)…)` decl per
    /// world-import interface, with each op's arrow derived from its WIT signature — the decls the
    /// no-redeclare pre-pass injects. Verifies the effect/op NAMES (FQ import → short kebab) and that the
    /// derived arrows RESOLVE to the intended effect-op types (nullary `(-> R)` = Unit->R; single-arg with
    /// an option result; the 3-node multi-arg arrow shape).
    #[test]
    fn synthesize_world_import_effect_decls_builds_resolvable_effects() {
        use crate::db::Db;
        let mut b = Builder::new();
        let byte_list = |b: &mut Builder| {
            let hh = b.atom_leaf(Leaf::Str("list".into()));
            let uh = b.name("u8");
            let u = b.list(vec![uh]);
            b.list(vec![hh, u])
        };
        // import cadenza:agent-kernel/identity { id: () -> list<u8> }
        let id_res = byte_list(&mut b);
        let id = member(&mut b, "id", vec![], id_res);
        let ih1 = b.name("import");
        let in1 = b.name("cadenza:agent-kernel/identity");
        let identity = b.list(vec![ih1, in1, id]);
        // import cadenza:agent-kernel/state { get: (list<u8>) -> option<list<u8>>;
        //   put: (list<u8>, list<u8>) -> unit }
        let gk = byte_list(&mut b);
        let gi = byte_list(&mut b);
        let gr = opt(&mut b, gi);
        let get = member(&mut b, "get", vec![("key", gk)], gr);
        let pk = byte_list(&mut b);
        let pv = byte_list(&mut b);
        let pr = unit(&mut b);
        let put = member(&mut b, "put", vec![("key", pk), ("value", pv)], pr);
        let ih2 = b.name("import");
        let in2 = b.name("cadenza:agent-kernel/state");
        let state = b.list(vec![ih2, in2, get, put]);
        let wh = b.name("world");
        let wn = b.name("reducer");
        let world = b.list(vec![wh, wn, identity, state]);
        let a = b.finish(world);
        let bytes = crate::codec::encode(&a);

        let mut db = Db::load(crate::testkit::parse(
            "(module m (def (main) 0) (export main))",
        ));
        let decls = synthesize_world_import_effect_decls(&mut db.ast, Some(&bytes));
        assert_eq!(decls.len(), 2, "one (effect …) decl per import interface");

        // Navigate a synthesized `(effect NAME (op OP ARROW)…)` decl.
        fn parts(db: &Db, d: StructId) -> (String, Vec<(String, StructId)>) {
            let Struct::List(kids) = db.ast.get(d) else {
                panic!("effect decl is a list")
            };
            assert_eq!(db.ast.as_name(kids[0]), Some("effect"));
            let name = db.ast.as_name(kids[1]).unwrap().to_string();
            let ops = kids[2..]
                .iter()
                .map(|&op| {
                    let Struct::List(ok) = db.ast.get(op) else {
                        panic!("op is a list")
                    };
                    assert_eq!(db.ast.as_name(ok[0]), Some("op"));
                    (db.ast.as_name(ok[1]).unwrap().to_string(), ok[2])
                })
                .collect();
            (name, ops)
        }

        let (n0, ops0) = parts(&db, decls[0]);
        assert_eq!(n0, "identity", "FQ import name → short kebab effect name");
        assert_eq!(ops0.len(), 1);
        assert_eq!(ops0[0].0, "id");
        // A nullary `(-> Bytes)` resolves to `Unit -> Bytes` (the elided-unit convention).
        let id_arrow = ops0[0].1;
        assert_eq!(
            crate::eval::typeval_of(&mut db, id_arrow),
            Some(Ty::Fn(Box::new(Ty::Unit), Box::new(Ty::Bytes))),
            "the derived identity.id arrow resolves to Unit -> Bytes"
        );

        let (n1, ops1) = parts(&db, decls[1]);
        assert_eq!(n1, "state");
        assert_eq!(
            ops1.iter().map(|o| o.0.as_str()).collect::<Vec<_>>(),
            vec!["get", "put"]
        );
        // get: (list<u8>) -> option<list<u8>> → Bytes -> Option Bytes.
        let opt_bytes = {
            let occ = db
                .type_decls
                .iter()
                .find(|t| t.name == "Option")
                .unwrap()
                .occ;
            db.normalize_sum(occ, vec![Ty::Bytes])
        };
        let get_arrow = ops1[0].1;
        assert_eq!(
            crate::eval::typeval_of(&mut db, get_arrow),
            Some(Ty::Fn(Box::new(Ty::Bytes), Box::new(opt_bytes))),
            "the derived state.get arrow resolves to Bytes -> Option Bytes"
        );
        // put: (list<u8>, list<u8>) -> unit — the multi-arg arrow `(-> Bytes Bytes Unit)` (4 nodes).
        let put_arrow = ops1[1].1;
        let Struct::List(ak) = db.ast.get(put_arrow) else {
            panic!("arrow is a list")
        };
        assert_eq!(db.ast.as_name(ak[0]), Some("->"));
        assert_eq!(ak.len(), 4, "(-> Bytes Bytes Unit) is a 4-node arrow");

        // An absent world → no decls.
        assert!(synthesize_world_import_effect_decls(&mut db.ast, None).is_empty());
    }

    /// A world-import op whose type CONTAINS a WIT `variant` (the `deliver-response` shape: `answer:
    /// result<payload, error>` where `error` is an anonymous variant) is now SYNTHESIZED — the arrow
    /// derivation threads the guest's NAMED sums, so the anonymous error variant resolves to the guest's
    /// mirroring `type Error` and the op MAPS. Previously the import synthesis used an EMPTY sums map →
    /// variant → `None` → the op was silently SKIPPED (deliver-response never resolved). Contrast: with no
    /// mirroring guest type the op stays skipped.
    #[test]
    fn a_variant_typed_import_op_is_synthesized_when_a_guest_named_sum_mirrors_it() {
        use crate::db::Db;
        let mut b = Builder::new();
        let byte_list = |b: &mut Builder| {
            let hh = b.atom_leaf(Leaf::Str("list".into()));
            let uh = b.name("u8");
            let u = b.list(vec![uh]);
            b.list(vec![hh, u])
        };
        // respond: (r: result<list<u8>, variant(timeout, faulted)>) -> unit
        let err_variant = {
            let vh = str_head(&mut b, "variant");
            let c1 = {
                let n = b.name("timeout");
                b.list(vec![n])
            };
            let c2 = {
                let n = b.name("faulted");
                b.list(vec![n])
            };
            b.list(vec![vh, c1, c2])
        };
        let result_ty = {
            let rh = str_head(&mut b, "result");
            let ok = byte_list(&mut b);
            b.list(vec![rh, ok, err_variant])
        };
        let rr = unit(&mut b);
        let respond = member(&mut b, "respond", vec![("r", result_ty)], rr);
        let ih = b.name("import");
        let inm = b.name("cadenza:agent-kernel/deliver");
        let deliver = b.list(vec![ih, inm, respond]);
        let wh = b.name("world");
        let wn = b.name("reducer");
        let world = b.list(vec![wh, wn, deliver]);
        let a = b.finish(world);
        let bytes = crate::codec::encode(&a);

        // Guest declares `type Error = | Timeout | Faulted` — case-set {timeout, faulted} mirrors the world's
        // anonymous error variant, so the variant-typed op maps + is synthesized.
        let mut db = Db::load(crate::testkit::parse(
            "(module m (type Error Timeout Faulted) (def (main) 0) (export main))",
        ));
        let decls = synthesize_world_import_effect_decls(&mut db.ast, Some(&bytes));
        assert_eq!(
            decls.len(),
            1,
            "the variant-typed op maps via the guest's named Error → the effect is synthesized"
        );

        // Contrast: a guest with NO mirroring sum → the variant → None → the op is skipped → empty effect dropped.
        let mut db2 = Db::load(crate::testkit::parse(
            "(module m (def (main) 0) (export main))",
        ));
        let decls2 = synthesize_world_import_effect_decls(&mut db2.ast, Some(&bytes));
        assert!(
            decls2.is_empty(),
            "with no mirroring guest sum the variant-typed op is skipped (hand-declared meanwhile)"
        );
    }

    #[test]
    fn an_enum_typed_import_op_is_synthesized_when_a_guest_named_sum_mirrors_it() {
        // The ENUM-arm twin of the variant-mirror test above: a WIT `enum` is a DISTINCT descriptor arm
        // (bare-NAME cases, not the variant's per-case lists) resolved by the SAME case-set→guest-sum-name
        // path (`guest_sum_names` + `wit_type_to_type_expr_with_sums`'s `WitType::Enum` arm). Pins the
        // enum-mirror path (previously only the variant arm was witnessed) so the enum
        // synthesized-nominal-decl increment can't silently regress the ALREADY-wired guest-mirror case.
        use crate::db::Db;
        let mut b = Builder::new();
        // set-status: (s: enum { active, closed }) -> unit
        let status_enum = {
            let eh = str_head(&mut b, "enum");
            let c1 = b.name("active");
            let c2 = b.name("closed");
            b.list(vec![eh, c1, c2])
        };
        let rr = unit(&mut b);
        let set_status = member(&mut b, "set-status", vec![("s", status_enum)], rr);
        let ih = b.name("import");
        let inm = b.name("cadenza:agent-kernel/lifecycle");
        let lifecycle = b.list(vec![ih, inm, set_status]);
        let wh = b.name("world");
        let wn = b.name("reducer");
        let world = b.list(vec![wh, wn, lifecycle]);
        let a = b.finish(world);
        let bytes = crate::codec::encode(&a);

        // Guest declares `type Status = | Active | Closed` — case-set {active, closed} mirrors the world's
        // anonymous enum, so the enum-typed op maps + is synthesized.
        let mut db = Db::load(crate::testkit::parse(
            "(module m (type Status Active Closed) (def (main) 0) (export main))",
        ));
        let decls = synthesize_world_import_effect_decls(&mut db.ast, Some(&bytes));
        assert_eq!(
            decls.len(),
            1,
            "the enum-typed op maps via the guest's named Status → the effect is synthesized"
        );

        // Contrast: a guest with NO mirroring sum → the enum → None → the op is skipped.
        let mut db2 = Db::load(crate::testkit::parse(
            "(module m (def (main) 0) (export main))",
        ));
        let decls2 = synthesize_world_import_effect_decls(&mut db2.ast, Some(&bytes));
        assert!(
            decls2.is_empty(),
            "with no mirroring guest sum the enum-typed op is skipped (hand-declared meanwhile)"
        );
    }

    #[test]
    fn an_imposed_world_enum_with_no_guest_mirror_synthesizes_a_nominal_and_maps_the_op() {
        // WIT ENUM SELF-DECLARATION, sub-slice 1 (operator ruling 2026-08-28, full WIT algebra): an IMPOSED
        // world declares an anonymous enum { active, closed } the guest does NOT mirror. Where the mirror test
        // above SKIPS such an op, inject_world_import_effects_from_bytes must now SYNTHESIZE a nominal
        // (type Wit… Active Closed) + inject it FIRST, so the enum-typed op MAPS (its effect is synthesized).
        // Per the ruling: internal synth type name (Wit-prefixed), nameable case constructors (Active/Closed).
        use crate::db::Db;
        let mut b = Builder::new();
        // status: () -> enum { active, closed }
        let status_enum = {
            let eh = str_head(&mut b, "enum");
            let c1 = b.name("active");
            let c2 = b.name("closed");
            b.list(vec![eh, c1, c2])
        };
        let status = member(&mut b, "status", vec![], status_enum);
        let ih = b.name("import");
        let inm = b.name("cadenza:agent-kernel/lifecycle");
        let lifecycle = b.list(vec![ih, inm, status]);
        let wh = b.name("world");
        let wn = b.name("reducer");
        let world = b.list(vec![wh, wn, lifecycle]);
        let a = b.finish(world);
        let bytes = crate::codec::encode(&a);

        // Guest module with NO mirror type.
        let mut db = Db::load(crate::testkit::parse(
            "(module m (def (main) 0) (export main))",
        ));
        inject_world_import_effects_from_bytes(&mut db.ast, &bytes);

        let items = top_level_items(&db.ast);
        // A nominal type was synthesized (Wit-prefixed name), with the payloadless cases Active + Closed.
        let synth_ctors = items.iter().find_map(|&it| {
            let tail = db.ast.as_form(it, "type")?;
            let (&nn, cases) = tail.split_first()?;
            if !db.ast.as_name(nn)?.starts_with("Wit") {
                return None;
            }
            Some(
                cases
                    .iter()
                    .filter_map(|&c| db.ast.as_name(c).map(|s| s.to_string()))
                    .collect::<Vec<_>>(),
            )
        });
        assert_eq!(
            synth_ctors.as_deref(),
            Some(&["Active".to_string(), "Closed".to_string()][..]),
            "an imposed-world anonymous enum with no guest mirror synthesizes a (type Wit… Active Closed)"
        );
        // And the enum-typed import op now MAPS → the lifecycle effect is injected (was skipped before).
        let has_effect = items.iter().any(|&it| {
            db.ast
                .as_form(it, "effect")
                .and_then(|t| t.first().copied())
                .and_then(|n| db.ast.as_name(n))
                .is_some_and(|n| n == "lifecycle")
        });
        assert!(
            has_effect,
            "the enum-typed import op maps once its nominal is synthesized → the effect is injected"
        );
    }

    #[test]
    fn an_imposed_world_payloaded_variant_no_mirror_synthesizes_a_nominal() {
        // WIT nominal-sum SELF-DECLARATION, sub-slice 3: an IMPOSED world imports an op whose result is an
        // anonymous VARIANT { active, closed(s64) } — a payload-LESS case + a payloaded case — the guest does
        // NOT mirror. synthesize_missing_nominal_decls must build (type Wit… Active (Closed (Int 64))) + inject
        // it so the variant-typed op MAPS. Extends the enum sub-slice (payloadless) to payloaded cases; the
        // payload type-expr is built via wit_type_to_type_expr.
        use crate::db::Db;
        let mut b = Builder::new();
        let s64 = |b: &mut Builder| {
            let h = b.name("s64");
            b.list(vec![h])
        };
        // status: () -> variant { active, closed(s64) }
        let status_variant = {
            let vh = str_head(&mut b, "variant");
            let active = {
                let n = b.name("active");
                b.list(vec![n])
            };
            let closed = {
                let n = b.name("closed");
                let p = s64(&mut b);
                b.list(vec![n, p])
            };
            b.list(vec![vh, active, closed])
        };
        let status = member(&mut b, "status", vec![], status_variant);
        let ih = b.name("import");
        let inm = b.name("cadenza:agent-kernel/lifecycle");
        let lifecycle = b.list(vec![ih, inm, status]);
        let wh = b.name("world");
        let wn = b.name("reducer");
        let world = b.list(vec![wh, wn, lifecycle]);
        let a = b.finish(world);
        let bytes = crate::codec::encode(&a);

        let mut db = Db::load(crate::testkit::parse(
            "(module m (def (main) 0) (export main))",
        ));
        inject_world_import_effects_from_bytes(&mut db.ast, &bytes);

        let items = top_level_items(&db.ast);
        // The synthesized nominal's case HEADS — a bare-name (payloadless) OR the head of a (Ctor payload) list.
        let synth_case_heads = items.iter().find_map(|&it| {
            let tail = db.ast.as_form(it, "type")?;
            let (&nn, cases) = tail.split_first()?;
            if !db.ast.as_name(nn)?.starts_with("Wit") {
                return None;
            }
            Some(
                cases
                    .iter()
                    .filter_map(|&c| {
                        db.ast
                            .as_name(c)
                            .or_else(|| db.ast.head_name(c))
                            .map(|s| s.to_string())
                    })
                    .collect::<Vec<_>>(),
            )
        });
        assert_eq!(
            synth_case_heads.as_deref(),
            Some(&["Active".to_string(), "Closed".to_string()][..]),
            "a payloaded variant synthesizes (type Wit… Active (Closed …)) — payloadless + payloaded cases"
        );
        let has_effect = items.iter().any(|&it| {
            db.ast
                .as_form(it, "effect")
                .and_then(|t| t.first().copied())
                .and_then(|n| db.ast.as_name(n))
                .is_some_and(|n| n == "lifecycle")
        });
        assert!(
            has_effect,
            "the payloaded-variant import op maps once its nominal is synthesized → the effect is injected"
        );
    }

    #[test]
    fn a_nested_enum_in_a_result_import_result_synthesizes_its_nominal() {
        // Full-WIT-algebra NESTING: the anonymous enum is not the whole result but NESTED inside a compound
        // (`result<s64, enum{active,closed}>`). collect_nominal_sum_specs recurses through the result arms, so
        // the nested enum is still collected + synthesized and the op maps (the result-expr becomes
        // (Result (Int 64) Wit…)). Pins that the synth reaches sums nested in list/tuple/record/option/result.
        use crate::db::Db;
        let mut b = Builder::new();
        let result_enum = {
            let rh = str_head(&mut b, "result");
            let ok = {
                let h = b.name("s64");
                b.list(vec![h])
            };
            let err = {
                let eh = str_head(&mut b, "enum");
                let c1 = b.name("active");
                let c2 = b.name("closed");
                b.list(vec![eh, c1, c2])
            };
            b.list(vec![rh, ok, err])
        };
        let status = member(&mut b, "status", vec![], result_enum);
        let ih = b.name("import");
        let inm = b.name("cadenza:agent-kernel/lifecycle");
        let lifecycle = b.list(vec![ih, inm, status]);
        let wh = b.name("world");
        let wn = b.name("reducer");
        let world = b.list(vec![wh, wn, lifecycle]);
        let a = b.finish(world);
        let bytes = crate::codec::encode(&a);

        let mut db = Db::load(crate::testkit::parse(
            "(module m (def (main) 0) (export main))",
        ));
        inject_world_import_effects_from_bytes(&mut db.ast, &bytes);

        let items = top_level_items(&db.ast);
        let synth = items.iter().find_map(|&it| {
            let tail = db.ast.as_form(it, "type")?;
            let (&nn, cases) = tail.split_first()?;
            if !db.ast.as_name(nn)?.starts_with("Wit") {
                return None;
            }
            Some(
                cases
                    .iter()
                    .filter_map(|&c| db.ast.as_name(c).map(|s| s.to_string()))
                    .collect::<Vec<_>>(),
            )
        });
        assert_eq!(
            synth.as_deref(),
            Some(&["Active".to_string(), "Closed".to_string()][..]),
            "an enum nested in a result import result is collected + synthesized (nesting reaches it)"
        );
        let has_effect = items.iter().any(|&it| {
            db.ast
                .as_form(it, "effect")
                .and_then(|t| t.first().copied())
                .and_then(|n| db.ast.as_name(n))
                .is_some_and(|n| n == "lifecycle")
        });
        assert!(
            has_effect,
            "the op whose result nests the enum maps once the nested nominal is synthesized"
        );
    }

    // Build a `status`-style nullary import op returning an `enum { active, closed }`, in interface
    // `cadenza:agent-kernel/lifecycle`, as a `(world reducer (import …))` — the shared shape for the
    // synthesis hardening tests below. `extra` adds a SECOND member with the SAME enum result.
    fn world_bytes_with_enum_import(mut b: Builder, extra_member: bool) -> Vec<u8> {
        let mk_enum = |b: &mut Builder| {
            let eh = str_head(b, "enum");
            let c1 = b.name("active");
            let c2 = b.name("closed");
            b.list(vec![eh, c1, c2])
        };
        let mut members = {
            let e = mk_enum(&mut b);
            vec![member(&mut b, "status", vec![], e)]
        };
        if extra_member {
            let e = mk_enum(&mut b);
            members.push(member(&mut b, "check", vec![], e));
        }
        let ih = b.name("import");
        let inm = b.name("cadenza:agent-kernel/lifecycle");
        let mut iface = vec![ih, inm];
        iface.extend(members);
        let lifecycle = b.list(iface);
        let wh = b.name("world");
        let wn = b.name("reducer");
        let world = b.list(vec![wh, wn, lifecycle]);
        let a = b.finish(world);
        crate::codec::encode(&a)
    }

    fn synth_wit_type_names(ast: &Arenas) -> Vec<String> {
        top_level_items(ast)
            .into_iter()
            .filter_map(|it| ast.as_form(it, "type").and_then(|t| t.first().copied()))
            .filter_map(|n| ast.as_name(n).map(|s| s.to_string()))
            .filter(|n| n.starts_with("Wit"))
            .collect()
    }

    #[test]
    fn the_same_enum_reused_across_two_import_ops_synthesizes_one_nominal() {
        // DEDUP: a world that uses the same anonymous enum { active, closed } in TWO import ops must
        // synthesize exactly ONE nominal (collect dedups by case-set) — two `(type Wit… …)` decls would be a
        // duplicate-declaration resolve error. Realistic: worlds reuse a status/outcome enum across members.
        use crate::db::Db;
        let bytes = world_bytes_with_enum_import(Builder::new(), true);
        let mut db = Db::load(crate::testkit::parse(
            "(module m (def (main) 0) (export main))",
        ));
        inject_world_import_effects_from_bytes(&mut db.ast, &bytes);
        assert_eq!(
            synth_wit_type_names(&db.ast).len(),
            1,
            "the same enum reused across two ops synthesizes exactly ONE nominal (deduped by case-set)"
        );
    }

    #[test]
    fn a_synth_name_colliding_with_a_guest_type_is_disambiguated() {
        // NAME COLLISION: the guest declares a type literally named `WitActiveClosed` (a DIFFERENT case-set),
        // which is exactly the internal name the enum { active, closed } would synthesize. The synthesis must
        // disambiguate (WitActiveClosed2) rather than emit a second `(type WitActiveClosed …)` (a duplicate
        // declaration). Guards the disambiguation loop.
        use crate::db::Db;
        let bytes = world_bytes_with_enum_import(Builder::new(), false);
        // Guest already has `type WitActiveClosed = Foo | Bar` (case-set {foo,bar} — does NOT mirror the enum).
        let mut db = Db::load(crate::testkit::parse(
            "(module m (type WitActiveClosed Foo Bar) (def (main) 0) (export main))",
        ));
        inject_world_import_effects_from_bytes(&mut db.ast, &bytes);
        // The synthesized enum nominal must carry a DISAMBIGUATED name (not the guest's WitActiveClosed), with
        // the Active/Closed cases.
        let synth = top_level_items(&db.ast).into_iter().find_map(|it| {
            let tail = db.ast.as_form(it, "type")?;
            let (&nn, cases) = tail.split_first()?;
            let name = db.ast.as_name(nn)?;
            let ctors: Vec<String> = cases
                .iter()
                .filter_map(|&c| db.ast.as_name(c).map(|s| s.to_string()))
                .collect();
            if ctors == ["Active".to_string(), "Closed".to_string()] {
                Some(name.to_string())
            } else {
                None
            }
        });
        assert_eq!(
            synth.as_deref(),
            Some("WitActiveClosed2"),
            "the synthesized enum nominal disambiguates its name off the colliding guest type"
        );
    }
}
