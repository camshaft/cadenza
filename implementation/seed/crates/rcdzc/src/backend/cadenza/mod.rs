//! The Cadenza backend — lower the OPTIMIZED Core back to Cadenza surface, emitting the BINARY AST.
//!
//! Unlike the wasm/rust backends (which emit a runnable artifact), this backend emits the PROGRAM
//! ITSELF back out: it walks the same target-neutral columns everything above the seam produced
//! (`core_of`/`type_of`/[`Layout`]) — i.e. the program AFTER resolution, inference, const-folding, and
//! optimization — and reconstructs a Cadenza surface AST from it, then serializes that tree with the
//! binary-AST codec ([`crate::codec::encode`]). The result is a `kind == "ast"` artifact.
//!
//! Three properties this enables (operator's mandate):
//! - **round-trip idempotence** — feeding the emitted binary AST back through `compile` yields the
//!   identical optimized program, hence a byte-identical re-emit (the correctness + optimization-
//!   inspection signal). A deterministic build order is what makes this hold: `crate::codec::encode`
//!   serializes BUILD ORDER (no canon pass), so this backend builds head-first, left-to-right, and
//!   the same Core always produces the same bytes.
//! - **syntax-system pipe** — the bytes decode straight into `cadenza-syntax` for sexpr/ML rendering,
//!   so a human can inspect what lowering + folding did.
//! - **lean-oracle input** — the oracle consumes binary AST directly.
//!
//! Like the other backends it consumes the structured Core DIRECTLY (no flat wasm `Lir`) and uses only
//! `backend::common`, never the sibling backends' internals. It DECLINES (attributed to this target) a
//! construct it does not yet reconstruct — the same decline-don't-miscompile discipline the wasm/rust
//! backends follow. Coverage so far:
//! - **B0**: whole-program shape (`(do (def …)… (export …)…)`) with CONSTANT-bodied definitions — the
//!   PLAIN constant leaves (Int/Bool/Str/Char/Float/Unit) as literals, and the WRAPPER-typed numeric-
//!   tower / nominal-leaf constants via a re-compilable surface: `BigInt`→`(: n BigInt)` (the direct
//!   ascription — `BigInt.of` widens an `Int64` so it can't hold a beyond-`Int64` literal),
//!   `Rational`→`(Rational.of n d)` when num/den fit `Int64` (else declines), `Symbol`→`(Symbol.of "…")`
//!   (emitting the bare scalar would drop the type and miscompile the value). `Ty::Qty` still declines
//!   (needs unit reconstruction — a later slice).
//! - **B1a**: PARAMETERS — a def signature `(<name> (: <p> <Ty>)…)` (param types via lower's canonical
//!   `type_ast`) and a `Core::Param`/`LocalRef` reference (the bare binder name). A parameter of a type
//!   with no value-form surface (function/unsolved) declines.
//! - **B1b**: OPERATORS + CONTROL — the runtime binary operators (`Arith`/`Compare`/`StrCmp`/
//!   `FloatCompare`, re-emitted `(<op> l r)` via the `Prim`→surface reverse-map), boolean `Not`
//!   (`(not x)`), short-circuit `And`/`or` (`(and|or l r)`), and the conditional `If` (`(if c t e)`).
//! - **B2**: BINDING — a kept multi-use `Core::Let` re-emits as `(let ((<n> <v>)…) <body>)` with
//!   DETERMINISTIC synthesized binding names (the source name is discarded at lowering), and a
//!   `Core::LocalRef` resolves to its binding's synthesized name via the threaded [`BinderEnv`].
//! - **B3**: CALLS — a `Core::Call` (a non-inlinable, i.e. recursive, application) re-emits as
//!   `(<callee-name> <arg>…)`, naming the callee by its source name (it is in `layout.order`, so its
//!   `(def …)` is emitted too).
//!
//! - **M1**: scalar `Core::Match` (Int/Bool probes + wildcard/binder) → an `if`-chain of `(= scrut lit)`
//!   probes (value-equivalent; the scrutinee is a pure scalar). A GUARDED arm desugars into the `if`
//!   condition — `(if (and (= scrut lit) <guard>) body rest)`, or `(if <guard> body rest)` for a guarded
//!   wildcard — since a guard's fall-through IS the `if`/else chain. A non-scalar probe
//!   (Str/Char/Bytes/list/map) declines.
//! - **M4a**: sum `Core::MatchSum` → surface `(match <scrutinee> (<Variant> <binder>…) <body>)…`
//!   ([`emit_match_sum`]): a root switch on the scrutinee's OWN discriminant with EXPLICIT-variant, bare
//!   `Leaf`-body arms. Each arm mints a fresh `_cdz_m<n>` binder per payload slot (recorded in
//!   `env.payloads` under the `SumPayload` `(scrutinee, path)` key the body reads); a `Core::SumPayload`
//!   resolves to its binder. NESTED matches (a `Leaf` body that is itself a `MatchSum`) recurse naturally.
//!   A disc-FOLDED / nested-`Switch` / `Guarded` / `LitTest` decision tree, or a DEFAULT (`disc: None`)
//!   arm, declines. A match over a user sum whose `(type …)` was not re-emitted declines; prelude sums
//!   (Option/Result) are ambient.
//! - **M4b**: list `Core::MatchList` → surface `(match <scrutinee> (<list-pattern> <body>)…)`
//!   ([`emit_match_list`]): a length-dispatch arm — `LenEq(n)`→`(list b0 … b_{n-1})`, `LenGe(lead)`→
//!   `(list b0 … b_{lead-1} .. rest)`, `Any`→`_`. Leading element binders register at `[Elem(i)]`, the
//!   rest binder at `[RestFrom(lead)]` (same `env.payloads` map M4a uses); a `Core::SumPayload` resolves
//!   to its binder, and nested list matches recurse. A GUARDED arm re-emits the `(guard <pattern> <cond>)`
//!   surface form (cond with the arm's binders in scope). A NESTED/variant element sub-pattern (a deeper
//!   `SumPayload` path this slice does not register) declines.
//! - **DATA**: runtime compound VALUES — `Core::Tuple`→`(tuple …)`, `Core::Record`→`(record (= k v)…)`
//!   (name-sorted), `Core::ListNew`→`(list …)`, `Core::MapNew`→`(map (<k> <v>)…)` and `Core::SetOf`→
//!   `((. Set of) (list …))` (map/set entries emit in STORED order — the value is unordered, so the
//!   round-trip is VALUE-equivalence, order-independent; the keys are runtime, no canonical sort applies);
//!   and a `Core::SumNew` variant →
//!   `(: (<Variant> <payload-or-unit>) <sum-type>)` (the type ascription pins an under-determined sum,
//!   e.g. a bare `(None unit)`). When the value's OWN solved type is under-determined (a bare `(None)` at a
//!   join, whose own type is `Option<?>`), the `<sum-type>` is recovered from the `expected` type its
//!   container passed down (see `emit_expr`'s `expected` / [`body_ctx`]); a still-free type declines.
//!   All mirror lower's value surface.
//!   A USER sum is re-declared: `emit` emits its `(type <Name> (<Variant> <PayloadTy>…)…)` decl (for a
//!   MONOMORPHIC, CLOSED, MULTI-variant sum — recursive payloads OK) and its values then round-trip; a
//!   GENERIC / OPEN / SINGLE-variant (optimizer-erased) user sum, and a user `Nominal` newtype, still
//!   DECLINE. PRELUDE sums (Option/Result/…) are ambient (no decl). A user-sum value emits ⇔ its decl was
//!   emitted (`emitted` set), so there is never an unbound-type recompile.
//!
//! Still declining, for later increments: closures (Closure/Captured/CallClosure), sequencing
//! (Seq/Block/Break), map/set OPERATIONS (insert/lookup/…), richer SUM decision trees (guarded /
//! literal-test / nested-switch / default sum arms — the `SumCont::Guarded` continuation), nested-element
//! list arms, non-scalar scalar-match probes, and a multi-argument variant CONSTRUCTOR (`SumNew` — the
//! match side already binds multi-payload slots).

use crate::ast::{Builder, Leaf, Radix, StructId};
use crate::core::Core;
use crate::db::Db;
use crate::diag::Reject;
use crate::layout::Layout;
use crate::lower::core_of;
use crate::ty::Ty;
use std::collections::HashMap;

/// The in-scope binding environment threaded (as `&mut`) through an emit walk. Two DISJOINT namespaces,
/// both mapping an anonymous-in-Core binder to the SYNTHESIZED surface name this backend mints for it (the
/// source name was discarded at lowering, so names are minted DETERMINISTICALLY — the same Core always
/// yields the same names, so a re-emit round-trips):
/// - `lets`: a kept `let` binding's initializer occurrence (the `StructId` a `Core::LocalRef` resolves to)
///   → its synthesized name (by binding order, see [`synth_binding_name`]).
/// - `payloads`: a sum-match payload — keyed by `(scrutinee-node, path)` (the same key a `Core::SumPayload`
///   in an arm body carries) → the binder name a `Core::MatchSum` arm minted for that payload slot (see
///   [`synth_payload_name`]). A match payload binder is anonymous in Core (a body reads
///   `sum-payload(scrutinee)` at a path), so the arm mints one fresh name per payload slot and records it
///   here for the body; `next_payload` keeps names globally unique within the def so a nested match's
///   binders never shadow an outer match's.
#[derive(Default)]
struct BinderEnv {
    lets: HashMap<StructId, std::rc::Rc<str>>,
    payloads: HashMap<(StructId, Vec<crate::core::PathStep>), std::rc::Rc<str>>,
    next_payload: usize,
}

/// The deterministic synthesized surface name for the `i`th kept `let` binding encountered in an emit
/// walk. Positional (not derived from the binder's `StructId`, which differs between the two arenas of a
/// round-trip), so compile-then-recompile mints the SAME name for the structurally-same binding. The
/// `_cdz_let` prefix keeps it out of the way of ordinary source identifiers.
fn synth_binding_name(i: usize) -> std::rc::Rc<str> {
    format!("_cdz_let{i}").into()
}

/// The deterministic synthesized surface name for the `i`th sum-match PAYLOAD binder minted in an emit
/// walk (monotone across the whole def, so binders of a nested match never collide with an outer match's).
/// The `_cdz_m` prefix keeps it clear of source identifiers AND silences the unused-binding warning
/// (CDZ0306, "prefix with `_` to silence") for a payload slot the arm body never reads.
fn synth_payload_name(i: usize) -> std::rc::Rc<str> {
    format!("_cdz_m{i}").into()
}

/// Emit the binary-AST artifact for the program in `db` under `layout`. Reconstructs a Cadenza surface
/// tree `(do (def …)… (export …)…)` over the same reachable definition set (`layout.order`) the other
/// backends emit, then serializes it with the binary-AST codec.
pub fn emit(db: &mut Db, layout: &Layout) -> Result<Vec<u8>, Reject> {
    let mut b = Builder::new();

    // A top-level `(do …)` root: link unwraps a `do`/`module` root and contributes its children, so a
    // `(do <def>… <export>…)` re-reads to the same program `(module m <def>… <export>…)` would. Build
    // the head first (head-first order is the fleet-wide build convention the no-canon codec relies on).
    let do_head = b.name("do");
    let mut root_children = vec![do_head];

    // Emit the user-declared TYPE declarations FIRST — a user sum's value re-reads to `(: (V p) T)`,
    // which needs `(type T …)` in scope (a prelude sum like Option/Result is ambient, needs none).
    // `emit_type_decl` handles only a monomorphic, closed, multi-variant sum (generic / open / single-
    // variant-erased are declined at the value site); the set of decls that landed gates which sum values
    // may emit, so the two agree (a value emits ⇔ its decl was emitted — no unbound-type recompile).
    let mut emitted: std::collections::HashSet<StructId> = std::collections::HashSet::new();
    for i in 0..db.type_decls.len() {
        let decl = db.type_decls[i].clone();
        if db.is_user_node(decl.occ)
            && let Some(node) = emit_type_decl(db, &mut b, &decl)
        {
            root_children.push(node);
            emitted.insert(decl.occ);
        }
    }

    // One `(def …)` per reachable definition, in layout order (a stable, target-neutral order).
    for &def in &layout.order {
        root_children.push(emit_def(db, &mut b, def, &emitted)?);
    }

    // Then the exports, so recompiling the emitted program reaches the SAME definition set (an export-
    // less program would close to an empty `layout.order` and re-emit `(do)` — not idempotent). Emit in
    // the same layout order for determinism.
    for &def in &layout.order {
        if let Some(e) = layout.export_plan(def) {
            root_children.push(emit_export(db, &mut b, def, e)?);
        }
    }

    let root = b.list(root_children);
    let arenas = b.finish(root);
    Ok(crate::codec::encode(&arenas))
}

/// Reconstruct a user sum's `(type <Name> (<Variant> <PayloadTy>…)…)` declaration, or `None` for a sum
/// this slice does not emit: a GENERIC sum (type parameters — the payload is a type variable), an OPEN
/// sum (row-variable tail), or a SINGLE-variant sum (optimizer-erased to its payload, so its value emits
/// without the nominal — a later slice). A variant's payload types are recovered from their declaration
/// occurrences via `typeval_of` + lower's `type_ast`; a nullary variant is `(<Variant>)`. `decl` is an
/// owned clone (so `typeval_of`'s `&mut db` does not alias a `db.type_decls` borrow).
fn emit_type_decl(db: &mut Db, b: &mut Builder, decl: &crate::db::TypeDecl) -> Option<StructId> {
    if !decl.params.is_empty() || decl.open_tail.is_some() || decl.variants.len() < 2 {
        return None;
    }
    let type_head = b.name("type");
    let name = b.name(decl.name.as_str());
    let mut children = vec![type_head, name];
    for v in &decl.variants {
        let vname = b.name(v.name.as_str());
        let mut vchildren = vec![vname];
        for &p in &v.payloads {
            let ty = crate::eval::typeval_of(db, p)?;
            let ncx = db.name_ctx();
            let ty_node = crate::lower::type_ast(b, &ty, &ncx)?;
            vchildren.push(ty_node);
        }
        children.push(b.list(vchildren));
    }
    Some(b.list(children))
}

/// Reconstruct `(def (<name> (: <p> <Ty>)…) <body>)` for definition `def`. B1a handles NULLARY defs and
/// parameterized defs whose parameters have a value-form-representable type; a parameter of a type with
/// no surface (a function/continuation/unsolved type — `type_ast` returns `None`) declines.
fn emit_def(
    db: &mut Db,
    b: &mut Builder,
    def: usize,
    emitted: &std::collections::HashSet<StructId>,
) -> Result<StructId, Reject> {
    let name = db.defs[def].name.clone();
    let body = db.defs[def].body.ok_or_else(|| {
        Reject::decline(format!(
            "definition `{name}` has no body to lower to Cadenza"
        ))
    })?;

    // Build the signature `(<name> (: <p0> <Ty0>) …)`. `def_params` returns each parameter's binder
    // occurrence (the identity a `Core::Param` reference resolves to) paired with its SOLVED type. The
    // parameter's surface name is read off that binder occurrence; its type ascription reuses lower's
    // canonical `type_ast` so the re-emitted `(: p Ty)` is byte-identical to the type surface everything
    // else in the program uses (round-trip identity). `def_params` returns an owned Vec, so it is taken
    // FIRST (a `&mut db`), before the immutable `name_ctx()` borrow the type rendering needs.
    let params = crate::layout::def_params(db, def);
    let def_head = b.name("def");
    let sig_name = b.name(name.as_str());
    let mut sig_children = vec![sig_name];
    {
        // Within this scope only the immutable `NameCtx` (a `&db` borrow) and the builder are used — no
        // `&mut db` — so the parameter name reads (`as_name`) and `type_ast` calls compose.
        let ncx = db.name_ctx();
        for (binder, ty) in params.iter() {
            let pname = db.ast.as_name(*binder).ok_or_else(|| {
                Reject::decline(format!(
                    "the Cadenza backend cannot recover a parameter name for `{name}`"
                ))
            })?;
            let pname_node = b.name(pname);
            let ty_node = crate::lower::type_ast(b, ty, &ncx).ok_or_else(|| {
                Reject::decline(format!(
                    "the Cadenza backend does not yet lower a parameter of type `{}` (`{name}`) — no \
                     value-form type surface (a function / unsolved type)",
                    ty.render_name(&ncx)
                ))
            })?;
            // `(: <pname> <Ty>)` — the ascription head `:` is a Name atom, matching the surface reader
            // and `type_ast`'s own record-field ascriptions.
            let colon = b.name(":");
            sig_children.push(b.list(vec![colon, pname_node, ty_node]));
        }
    }
    let sig = b.list(sig_children);
    // A fresh binding environment per definition — a `let` / match arm in the body populates it.
    let mut env = BinderEnv::default();
    let body_node = emit_expr(db, b, body, None, &mut env, emitted)?;
    Ok(b.list(vec![def_head, sig, body_node]))
}

/// Reconstruct `(export <name>)` for an exported definition. B0 handles only an export whose boundary
/// name equals the definition's source name (an unrenamed export); a renamed export (`export … as …`)
/// declines, since dropping the rename would not round-trip.
fn emit_export(
    db: &mut Db,
    b: &mut Builder,
    def: usize,
    e: &crate::layout::ExportPlan,
) -> Result<StructId, Reject> {
    let source_name = db.defs[def].name.clone();
    if e.name != source_name {
        return Err(Reject::decline(format!(
            "the Cadenza backend does not yet lower a RENAMED export (`{source_name}` exported as \
             `{}`) — B0 emits unrenamed exports only",
            e.name
        )));
    }
    let export_head = b.name("export");
    let name_ref = b.name(source_name.as_str());
    Ok(b.list(vec![export_head, name_ref]))
}

/// Reconstruct a Cadenza surface expression from the optimized `Core` at `id`. B0 covers the constant
/// leaves only; every other node declines (attributed to this target), to be filled in by later
/// increments (ops/control B1, binding B2, calls B3, data B4, …).
/// `expected` is the type this expression is REQUIRED to have by its surrounding context (the branch/body
/// position it occupies) — `Some` when a container passed one down (an `if` gives its branches the join
/// type; a `let`/match body inherits the whole form's type), else `None`. It is a FALLBACK, used only where
/// the node's own solved type is under-determined: a nullary/partially-applied `Core::SumNew` whose solved
/// type has a FREE type argument (`(None)` in `(if c (Some 1) (None))`, whose own type is `Option<?>`) reads
/// the CONCRETE join type from `expected` to ascribe `(: (None unit) (Option Int64))` — otherwise it would
/// decline. Threaded only to value/tail positions (branches, bodies); operand/scrutinee/guard positions
/// pass `None` (they impose no outer type). Passing `None` everywhere reproduces the pre-thread behavior.
fn emit_expr(
    db: &mut Db,
    b: &mut Builder,
    id: StructId,
    expected: Option<Ty>,
    env: &mut BinderEnv,
    emitted: &std::collections::HashSet<StructId>,
) -> Result<StructId, Reject> {
    // A value whose solved type is a USER-declared sum/nominal round-trips only if its `(type …)`
    // declaration is re-emitted. `emit` emits (and records in `emitted`) the declarations it can — a
    // MONOMORPHIC, CLOSED, MULTI-variant sum. So a user-sum value whose decl IS in `emitted` proceeds
    // (its `(type …)` is in scope). Otherwise DECLINE: a GENERIC/OPEN sum (no decl emitted), a
    // SINGLE-variant sum (optimizer-ERASED to its bare payload, so its value would emit without the
    // nominal — a value divergence), or a `Ty::Nominal` newtype (erased likewise). Prelude sums
    // (Option/Result — `is_user_node` false) are ambient and always proceed. (breaker-reported;
    // decline-don't-miscompile.)
    match crate::infer::type_of(db, id) {
        Ty::Sum { decl, .. } if db.is_user_node(decl) && !emitted.contains(&decl) => {
            return Err(Reject::decline(
                "the Cadenza backend does not yet re-emit this user sum value (generic / open / \
                 single-variant sum — its `(type …)` declaration is not emitted) — a later slice"
                    .to_string(),
            ));
        }
        Ty::Nominal { decl, .. } if db.is_user_node(decl) => {
            return Err(Reject::decline(
                "the Cadenza backend does not yet re-emit a user nominal (newtype) value — it erases \
                 to its payload, losing the nominal — a later slice"
                    .to_string(),
            ));
        }
        _ => {}
    }
    match core_of(db, id) {
        // A CONSTANT scalar leaf re-reads to its plain value+type when its solved type is the PLAIN scalar
        // type; a numeric-tower / nominal-leaf WRAPPER (`BigInt`/`Rational`/`Symbol`) shares a bare scalar
        // core (`ConstInt`/`ConstStr`/`ConstRational`) but re-reads to the WRAPPER only through its
        // CONSTRUCTOR surface — emitting the bare scalar would drop the type and MISCOMPILE the value (a
        // `Ty::Symbol` value came back a `String`, confirmed). So a plain scalar emits its literal and a
        // wrapper constant emits `(X.of …)`. (`Ty::Qty` — a scaled/unit-bearing wrapper — needs unit
        // reconstruction and still declines, a later slice.) `radix` is display-only (Core drops it).
        Core::ConstInt(v) if matches!(crate::infer::type_of(db, id), Ty::Int(_)) => Ok(b
            .atom_leaf(Leaf::Int {
                value: v,
                radix: Radix::Dec,
            })),
        // A BigInt constant is a `ConstInt` typed `Ty::BigInt` — re-emit the DIRECT ascription
        // `(: <n> BigInt)`, NOT `(BigInt.of <n>)`: `BigInt.of` WIDENS a fixed-size `Int64`, so it cannot
        // hold a beyond-`Int64` literal (`(BigInt.of 9223372036854775808)` fails CDZ0201 "out of range
        // for Int64 … write the literal directly as a BigInt with (: … BigInt)"). The ascription form
        // takes the literal directly as a BigInt and round-trips at every magnitude.
        Core::ConstInt(v) if matches!(crate::infer::type_of(db, id), Ty::BigInt) => {
            let colon = b.name(":");
            let n = b.atom_leaf(Leaf::Int {
                value: v,
                radix: Radix::Dec,
            });
            let ty = b.name("BigInt");
            Ok(b.list(vec![colon, n, ty]))
        }
        Core::ConstStr(s) if matches!(crate::infer::type_of(db, id), Ty::String) => {
            Ok(b.atom_leaf(Leaf::Str(s)))
        }
        // A Symbol constant shares a `ConstStr` core typed `Ty::Symbol` — re-emit `(Symbol.of "…")` so it
        // re-reads as a Symbol (a bare string would come back a `String`).
        Core::ConstStr(s) if matches!(crate::infer::type_of(db, id), Ty::Symbol) => {
            let head = member_access(b, "Symbol", "of");
            let text = b.atom_leaf(Leaf::Str(s));
            Ok(b.list(vec![head, text]))
        }
        Core::ConstFloat(d) if matches!(crate::infer::type_of(db, id), Ty::Float(_)) => {
            Ok(b.atom_leaf(Leaf::Float(d)))
        }
        // An exact RATIONAL constant — its value-form `num/den` is not valid expression syntax, so
        // re-emit the CONSTRUCTOR `(Rational.of <num> <den>)` over the normalized pair. `Rational.of`
        // takes two `Int64` arguments, so a numerator/denominator BEYOND `Int64` cannot be expressed this
        // way (same limit as `BigInt.of`); such a constant DECLINES (a beyond-`Int64` rational literal
        // surface is a later slice) rather than emit a non-re-compilable `(Rational.of <huge> …)`.
        Core::ConstRational(n, d) if n.to_i64().is_some() && d.to_i64().is_some() => {
            let head = member_access(b, "Rational", "of");
            let num = b.atom_leaf(Leaf::Int {
                value: n,
                radix: Radix::Dec,
            });
            let den = b.atom_leaf(Leaf::Int {
                value: d,
                radix: Radix::Dec,
            });
            Ok(b.list(vec![head, num, den]))
        }
        // Bool / Char / Unit have no wrapping type, so they always emit their one literal form.
        Core::ConstBool(bo) => Ok(b.atom_leaf(Leaf::Bool(bo))),
        Core::ConstChar(c) => Ok(b.atom_leaf(Leaf::Char(c))),
        Core::Unit => Ok(b.name("unit")),
        // A reference to a function PARAMETER — its surface is the bare name of the parameter's binder
        // occurrence (a `Name`), which re-resolves to the same parameter on recompile.
        Core::Param { binder } => {
            let nm = db.ast.as_name(binder).ok_or_else(|| {
                Reject::decline(
                    "the Cadenza backend cannot recover the name of a parameter reference"
                        .to_string(),
                )
            })?;
            Ok(b.name(nm))
        }
        // A reference to a kept `let` binding — its binder is the initializer occurrence (NOT a `Name`),
        // so its surface name comes from the environment the enclosing `Let` populated with the
        // synthesized binding name. A `LocalRef` always lives inside its `Let`'s body, so the binder is
        // in scope; an absent entry would be an emit bug (a `LocalRef` reached without its `Let`).
        Core::LocalRef { binder } => {
            let nm = env.lets.get(&binder).ok_or_else(|| {
                Reject::decline(
                    "the Cadenza backend reached a `let`-binding reference with no binding in scope"
                        .to_string(),
                )
            })?;
            Ok(b.name(nm.clone()))
        }
        // A runtime binary operator — arithmetic, integer/bool comparison, string ordering, or float
        // comparison. All four carry `{op, lhs, rhs}` (FloatCompare also a width, ignored — the surface
        // operator is width-agnostic), and all re-emit as `(<operator> <lhs> <rhs>)`. The surface
        // operator is recovered from the prim; the INTERNAL float prims (`FAdd`/`FEq`/…) share the same
        // one surface operator as their integer counterparts (the author writes one `+`/`=`/`<`, `lower`
        // picks the prim by solved type), so re-emitting the shared operator re-solves to the same prim.
        Core::Arith { op, lhs, rhs }
        | Core::Compare { op, lhs, rhs }
        | Core::StrCmp { op, lhs, rhs }
        | Core::FloatCompare {
            op,
            lhs,
            rhs,
            width: _,
        } => {
            let sym = prim_operator(op).ok_or_else(|| {
                Reject::decline(format!(
                    "the Cadenza backend does not yet lower the operator prim {op:?}"
                ))
            })?;
            let head = b.name(sym);
            // Operands FIRST would reverse head-first order — build the head atom, then each operand
            // sub-tree left-to-right, then the list (children hold the ids; the head is already pushed).
            let l = emit_expr(db, b, lhs, None, env, emitted)?;
            let r = emit_expr(db, b, rhs, None, env, emitted)?;
            Ok(b.list(vec![head, l, r]))
        }
        // Boolean negation `(not x)`.
        Core::Not { operand } => {
            let head = b.name("not");
            let x = emit_expr(db, b, operand, None, env, emitted)?;
            Ok(b.list(vec![head, x]))
        }
        // Short-circuiting conjunction / disjunction — `is_and` picks `and` vs `or`.
        Core::And { lhs, rhs, is_and } => {
            let head = b.name(if is_and { "and" } else { "or" });
            let l = emit_expr(db, b, lhs, None, env, emitted)?;
            let r = emit_expr(db, b, rhs, None, env, emitted)?;
            Ok(b.list(vec![head, l, r]))
        }
        // A two-way conditional `(if cond then else)`. Both BRANCHES are value/tail positions of the same
        // solved type — the `if`'s own type (the join of the branches). Pass it down as `expected` so a
        // branch that is an under-determined `Core::SumNew` (a bare `(None)` whose own type is `Option<?>`)
        // recovers the concrete join type to ascribe against. The condition is a Bool operand — no expected.
        Core::If { cond, then_, else_ } => {
            let head = b.name("if");
            let ctx = body_ctx(db, id, expected);
            let c = emit_expr(db, b, cond, None, env, emitted)?;
            let t = emit_expr(db, b, then_, ctx.clone(), env, emitted)?;
            let e = emit_expr(db, b, else_, ctx, env, emitted)?;
            Ok(b.list(vec![head, c, t, e]))
        }
        // A kept multi-use binding sequence `(let ((<n0> <v0>) …) <body>)`. Each binding is `(init, init)`
        // — keyed only by its initializer occurrence — so a fresh surface name is minted deterministically
        // (by binding order) and recorded in `env` for the `LocalRef`s that read it. Bindings are
        // SEQUENTIAL (`let*`): a later binding's value may reference an earlier binding, so each name is
        // registered right AFTER its own value is emitted and BEFORE the next; the body is emitted with
        // every binding in scope. Head-first: the `let` head and each binding's name atom are pushed
        // before their sub-trees.
        Core::Let { bindings, body } => {
            let let_head = b.name("let");
            let mut binding_nodes = Vec::with_capacity(bindings.len());
            for &(binder, value) in bindings.iter() {
                let name = synth_binding_name(env.lets.len());
                let name_atom = b.name(name.clone());
                // The value is emitted with only the PRIOR bindings in scope (a binding's initializer
                // cannot reference itself), then this binding is registered for the rest of the sequence.
                let value_node = emit_expr(db, b, value, None, env, emitted)?;
                env.lets.insert(binder, name);
                binding_nodes.push(b.list(vec![name_atom, value_node]));
            }
            let bindings_list = b.list(binding_nodes);
            // The body is the `let`'s value/tail position — it has the whole `let`'s type, so it inherits
            // this `let`'s `expected`; the bindings' initializers are operands (no expected).
            let body_node = emit_expr(db, b, body, expected, env, emitted)?;
            Ok(b.list(vec![let_head, bindings_list, body_node]))
        }
        // A runtime CALL to a top-level function — `(<callee-name> <arg>…)`. `Core::Call` is present only
        // for an application that could NOT be inlined-and-folded at compile time (i.e. a RECURSIVE
        // callee); `callee` is the `db.defs` index, whose source name re-resolves to the same definition
        // (it is in `layout.order`, so this backend also emits its `(def …)`). Args are lowered in the
        // caller's frame, left-to-right. Head-first: the callee name atom is pushed before the args.
        Core::Call { callee, args } => {
            let callee_name = db.defs[callee].name.clone();
            let head = b.name(callee_name.as_str());
            let mut children = Vec::with_capacity(1 + args.len());
            children.push(head);
            for arg in args {
                children.push(emit_expr(db, b, arg, None, env, emitted)?);
            }
            Ok(b.list(children))
        }
        // A scalar MATCH over a runtime Int/Bool scrutinee — re-emit as an `if`-CHAIN of literal-equality
        // probes: `(match s (l0 b0) … (_ bn))` → `(if (= s l0) b0 (if … bn))`. This is VALUE-equivalent
        // (the backend itself lowers a scalar match to a probe chain), reusing the `if`/`=` emit; the
        // scrutinee is a pure scalar, so re-emitting it per probe is side-effect-free. M1 handles LITERAL
        // probes (`Int`/`Bool`) + a wildcard tail, UNGUARDED; a guarded arm or a non-scalar probe
        // (`Str`/`Char`/`Bytes`/`ListLen`/`MapHasKeys`) declines (later slices).
        Core::Match { scrutinee, arms } => {
            if arms.is_empty() {
                return Err(Reject::decline(
                    "the Cadenza backend does not lower a zero-arm match".to_string(),
                ));
            }
            for arm in &arms {
                if !matches!(
                    arm.probe,
                    crate::core::Probe::Int(_)
                        | crate::core::Probe::Bool(_)
                        | crate::core::Probe::Wild
                ) {
                    return Err(Reject::decline(
                        "the Cadenza backend does not yet lower a non-scalar match probe \
                         (Str/Char/Bytes/list/map)"
                            .to_string(),
                    ));
                }
            }
            let ctx = body_ctx(db, id, expected);
            emit_match_chain(db, b, scrutinee, &arms, 0, ctx, env, emitted)
        }
        // A runtime TUPLE value `(tuple <e>…)` — a fixed-arity positional product built from runtime
        // operands (a projection of a compile-time-visible tuple folds away in `lower`, so a surviving
        // `Core::Tuple` is a runtime value). Mirrors lower's constant value surface.
        Core::Tuple { elems } => {
            let head = b.name("tuple");
            let mut children = Vec::with_capacity(1 + elems.len());
            children.push(head);
            for e in elems.iter().copied() {
                children.push(emit_expr(db, b, e, None, env, emitted)?);
            }
            Ok(b.list(children))
        }
        // A runtime RECORD value `(record (= <k> <v>)…)` — fields in canonical (name-sorted `BTreeMap`)
        // order, each an `(= name value)` ascription pair (matching lower's `const_value_ast` surface).
        Core::Record { fields } => {
            let head = b.name("record");
            let mut children = Vec::with_capacity(1 + fields.len());
            children.push(head);
            for (name, &v) in fields.iter() {
                let fname = b.name(&*name.name);
                let fval = emit_expr(db, b, v, None, env, emitted)?;
                children.push(b.field_pair(fname, fval));
            }
            Ok(b.list(children))
        }
        // A runtime LIST value `(list <e>…)` — an ordered homogeneous sequence built from runtime
        // operands; the walk preserves element order.
        Core::ListNew { elems } => {
            let head = b.name("list");
            let mut children = Vec::with_capacity(1 + elems.len());
            children.push(head);
            for e in elems.iter().copied() {
                children.push(emit_expr(db, b, e, None, env, emitted)?);
            }
            Ok(b.list(children))
        }
        // A runtime MAP value `(map (<k> <v>)…)` — the entries are runtime operands (a fully-constant map
        // bakes via lower's constant escape, so a surviving `Core::MapNew` is a runtime value). Entries are
        // emitted in their STORED order, NOT re-sorted into canonical key order: a map is UNORDERED, so the
        // reconstructed value equals the original regardless of entry order (and the keys are runtime, so no
        // compile-time canonical sort is available anyway) — the round-trip is VALUE-equivalence, which a
        // map's order-independent identity satisfies. Each entry is the pair-list `(<k> <v>)` (distinct from
        // a record's `(= k v)`), key then value emitted left-to-right. Mirrors lower's constant map surface.
        Core::MapNew { entries, .. } => {
            let head = b.name("map");
            let mut children = Vec::with_capacity(1 + entries.len());
            children.push(head);
            for &(k, v) in entries.iter() {
                let kv = emit_expr(db, b, k, None, env, emitted)?;
                let vv = emit_expr(db, b, v, None, env, emitted)?;
                children.push(b.list(vec![kv, vv]));
            }
            Ok(b.list(children))
        }
        // A runtime SET value `((. Set of) (list <e>…))` — the `Set.of` application over a `(list …)` of the
        // elements (a fully-constant set bakes via lower's constant escape; a surviving `Core::SetOf` is a
        // runtime value). Like the map, elements emit in STORED order (a set is unordered, so value-identity
        // is order-independent). `Set.of` is the member access `(. Set of)`, matching lower's set surface.
        Core::SetOf { elems, .. } => {
            let list_head = b.name("list");
            let mut list_children = Vec::with_capacity(1 + elems.len());
            list_children.push(list_head);
            for e in elems.iter().copied() {
                list_children.push(emit_expr(db, b, e, None, env, emitted)?);
            }
            let inner_list = b.list(list_children);
            let dot = b.name(".");
            let set_mod = b.name("Set");
            let of_key = b.name("of");
            let set_of = b.list(vec![dot, set_mod, of_key]);
            Ok(b.list(vec![set_of, inner_list]))
        }
        // A runtime SUM (variant) value `(<Variant> <payload>)` — a constructed variant built from a
        // runtime payload. The variant NAME is recovered from the discriminant against the node's solved
        // sum type (`variant_head_ast` — bare, or `(. Type Variant)` when the name would collide with a
        // non-ctor prelude binding). A nullary variant carries `unit` (`(None unit)`), a single-payload
        // variant its payload; a multi-argument variant surface is not canonical and declines. Mirrors
        // lower's constant value surface.
        Core::SumNew { disc, payloads } => {
            // The value's own solved type. When it is UNDER-DETERMINED (a free type argument — a bare
            // nullary `(None)` at a join whose element type only the sibling branch fixes, so this node's
            // own type is `Option<?>`), fall back to the `expected` type the surrounding context supplied
            // (the `if`/`let`/match position this value fills). Both are the SAME sum declaration; `expected`
            // just carries the RESOLVED type arguments, which is what the `(: … <sum-type>)` ascription needs.
            let own_ty = crate::infer::type_of(db, id);
            let ty = match (&own_ty, &expected) {
                // Under-determined own type + a concrete expected of the same sum decl → use expected.
                (Ty::Sum { decl: od, .. }, Some(ex @ Ty::Sum { decl: ed, .. }))
                    if od == ed && ty_has_free_arg(&own_ty) && !ty_has_free_arg(ex) =>
                {
                    ex.clone()
                }
                _ => own_ty,
            };
            let decl = match &ty {
                Ty::Sum { decl, .. } => *decl,
                _ => {
                    return Err(Reject::decline(
                        "the Cadenza backend cannot recover a variant head for a non-sum SumNew"
                            .to_string(),
                    ));
                }
            };
            // `(: <variant> <sum-type>)` — the ASCRIPTION is required: the optimizer often folds a sum
            // value to a bare variant with no surrounding type context (e.g. `main` = `(None unit)`), and
            // a nullary or partially-parameterized variant under-determines the sum's type parameters
            // (`(Option _)` / `(Result Int64 _)`) → CDZ0203 on recompile. Annotating with the full solved
            // sum type (via lower's `type_ast`) pins it. `type_ast` returns `None` for an under-determined
            // sum (a free type-arg), so a genuinely-ambiguous value DECLINES rather than emit a bad surface.
            let colon = b.name(":");
            let head = crate::lower::variant_head_ast(db, b, decl, disc).ok_or_else(|| {
                Reject::decline(
                    "the Cadenza backend could not recover the variant name for a SumNew"
                        .to_string(),
                )
            })?;
            let payload = match payloads.len() {
                0 => b.name("unit"),
                1 => emit_expr(db, b, payloads[0], None, env, emitted)?,
                _ => {
                    return Err(Reject::decline(
                        "the Cadenza backend does not yet lower a multi-argument variant"
                            .to_string(),
                    ));
                }
            };
            let variant = b.list(vec![head, payload]);
            let ncx = db.name_ctx();
            let ty_node = crate::lower::type_ast(b, &ty, &ncx).ok_or_else(|| {
                Reject::decline(
                    "the Cadenza backend does not yet lower a variant of an under-determined sum type"
                        .to_string(),
                )
            })?;
            Ok(b.list(vec![colon, variant, ty_node]))
        }
        // A match over a runtime SUM scrutinee — re-emit the surface `(match <scrutinee> (<pat> <body>)…)`.
        // M4a handles the SIMPLE decision-tree shape (delegated to [`emit_match_sum`]): a root switch on the
        // scrutinee's OWN discriminant, every arm an explicit variant with a bare LEAF body; a disc-folded /
        // nested / guarded / literal-test tree, or a default (wildcard) arm, declines (a later slice).
        Core::MatchSum { scrutinee, root } => {
            let ctx = body_ctx(db, id, expected);
            emit_match_sum(db, b, scrutinee, &root, ctx, env, emitted)
        }
        // A match over a runtime LIST scrutinee — re-emit `(match <scrutinee> (<list-pattern> <body>)…)`
        // ([`emit_match_list`]): a length-`LenEq`/`LenGe`/`Any` arm with PLAIN leading-element + rest binders.
        // A guarded arm, or a nested/variant element sub-pattern (a deeper `SumPayload` path), declines.
        Core::MatchList { scrutinee, arms } => {
            let ctx = body_ctx(db, id, expected);
            emit_match_list(db, b, scrutinee, &arms, ctx, env, emitted)
        }
        // A match PAYLOAD read — its surface is the binder name the enclosing `MatchSum`/`MatchList` arm
        // minted for this `(scrutinee, path)` and recorded in `env.payloads` (a sum variant payload at
        // `[Payload]`/`[Payload, Elem(i)]`, or a list element/rest at `[Elem(i)]`/`[RestFrom(k)]`). Reached
        // ONLY inside the arm body that bound it; a read whose binder is not in scope (a nested sub-pattern
        // this slice does not emit) declines.
        Core::SumPayload { scrutinee, path } => {
            let nm = env
                .payloads
                .get(&(scrutinee, path.to_vec()))
                .ok_or_else(|| {
                    Reject::decline(
                        "the Cadenza backend reached a sum-match payload with no binder in scope (a \
                         payload read outside a directly-emitted single-level match arm)"
                            .to_string(),
                    )
                })?;
            Ok(b.name(nm.clone()))
        }
        other => Err(Reject::decline(format!(
            "the Cadenza backend does not yet lower this Core node back to Cadenza: {}",
            core_node_kind(&other)
        ))),
    }
}

/// Reconstruct the surface `(match <scrutinee> (<pattern> <body>)…)` for a `Core::MatchSum`. M4a lowers the
/// SIMPLE decision-tree shape: the `root` is a [`SumCont::Switch`] on the scrutinee's OWN discriminant
/// (empty `path`), every arm dispatches on an EXPLICIT variant (`disc: Some`) to a bare [`SumCont::Leaf`]
/// body. Anything richer declines (a later slice): a disc-FOLDED / NESTED-switch / GUARDED / LITERAL-TEST
/// continuation, or a DEFAULT (`disc: None`) wildcard arm. Each arm's `(<Variant> <binder>…)` pattern mints
/// one fresh `_cdz_m<n>` binder per payload slot (recorded in `env.payloads` under the same `(scrutinee,
/// path)` key a `Core::SumPayload` in the body carries — `[Payload]` for a single-payload variant,
/// `[Payload, Elem(i)]` for slot `i` of a multi-payload variant, mirroring `select.rs`), so a payload read
/// resolves to its binder. A match over a USER sum whose `(type …)` was not re-emitted declines (its variant
/// heads must resolve on recompile); a prelude sum (`Option`/`Result`) is ambient. The scrutinee is emitted
/// ONCE (a match evaluates it once). Because every arm is an explicit variant, the emitted match covers the
/// same variant set the original did — it stays exhaustive (no CDZ0210), no synthesized wildcard needed.
fn emit_match_sum(
    db: &mut Db,
    b: &mut Builder,
    scrutinee: StructId,
    root: &crate::core::SumCont,
    expected: Option<Ty>,
    env: &mut BinderEnv,
    emitted: &std::collections::HashSet<StructId>,
) -> Result<StructId, Reject> {
    use crate::core::{PathStep, SumCont};
    let arms = match root {
        SumCont::Switch { path, arms } if path.is_empty() => arms,
        _ => {
            return Err(Reject::decline(
                "the Cadenza backend does not yet lower this sum match (a disc-folded / nested / \
                 guarded root — only a switch on the scrutinee's own discriminant)"
                    .to_string(),
            ));
        }
    };
    // The scrutinee's solved sum declaration — the source of each arm's variant name + payload arity.
    let decl = match crate::infer::type_of(db, scrutinee) {
        Ty::Sum { decl, .. } => decl,
        _ => {
            return Err(Reject::decline(
                "the Cadenza backend cannot recover a sum declaration for a MatchSum scrutinee"
                    .to_string(),
            ));
        }
    };
    if db.is_user_node(decl) && !emitted.contains(&decl) {
        return Err(Reject::decline(
            "the Cadenza backend does not yet re-emit a match over this user sum (its `(type …)` \
             declaration is not emitted — a generic / open / single-variant sum)"
                .to_string(),
        ));
    }
    let match_head = b.name("match");
    let scrut_node = emit_expr(db, b, scrutinee, None, env, emitted)?;
    let mut children = vec![match_head, scrut_node];
    for arm in arms {
        let disc = arm.disc.ok_or_else(|| {
            Reject::decline(
                "the Cadenza backend does not yet lower a DEFAULT (wildcard) sum-match arm"
                    .to_string(),
            )
        })?;
        let body = match &arm.cont {
            SumCont::Leaf(body) => *body,
            _ => {
                return Err(Reject::decline(
                    "the Cadenza backend does not yet lower a guarded / literal-test / nested-switch \
                     sum-match arm"
                        .to_string(),
                ));
            }
        };
        // Recover the variant head (bare or `(. Type Variant)`) and its payload arity from the sum decl.
        let head = crate::lower::variant_head_ast(db, b, decl, disc).ok_or_else(|| {
            Reject::decline(
                "the Cadenza backend could not recover the variant name for a sum-match arm"
                    .to_string(),
            )
        })?;
        let arity = db
            .type_decl_by_occ(decl)
            .and_then(|t| t.variants.get(disc as usize))
            .map(|v| v.payloads.len())
            .ok_or_else(|| {
                Reject::decline(
                    "the Cadenza backend could not recover the variant arity for a sum-match arm"
                        .to_string(),
                )
            })?;
        // Mint a binder per payload slot and register its `SumPayload` path for the arm body: a single-
        // payload variant reads `[Payload]`; a multi-payload variant's payload is a tuple, slot `i` at
        // `[Payload, Elem(i)]`. A nullary variant emits the bare `(<Variant>)` pattern.
        let mut pat_children = vec![head];
        for slot in 0..arity {
            let name = synth_payload_name(env.next_payload);
            env.next_payload += 1;
            let path: Vec<PathStep> = if arity == 1 {
                vec![PathStep::Payload]
            } else {
                vec![PathStep::Payload, PathStep::Elem(slot)]
            };
            env.payloads.insert((scrutinee, path), name.clone());
            pat_children.push(b.name(name));
        }
        let pattern = b.list(pat_children);
        // The body is emitted with this arm's payload binders in scope; it is the match's value/tail
        // position, so it inherits the match's `expected` type (for an under-determined `(None)` etc.).
        let body_node = emit_expr(db, b, body, expected.clone(), env, emitted)?;
        children.push(b.list(vec![pattern, body_node]));
    }
    Ok(b.list(children))
}

/// Reconstruct the surface `(match <scrutinee> (<list-pattern> <body>)…)` for a `Core::MatchList` — a match
/// dispatched by the list's LENGTH. Each arm's [`ListArmCond`] maps to a surface list pattern: `LenEq(n)` →
/// `(list b0 … b_{n-1})` (a fixed-arity pattern binding exactly `n` elements), `LenGe(lead)` →
/// `(list b0 … b_{lead-1} .. rest)` (a rest pattern binding `lead` leading elements + the tail sublist), and
/// `Any` → the bare wildcard `_` (a whole-list catch-all; a body that reads the whole list does so through
/// the scrutinee's OWN name, which A-normal form guarantees is a binder). A leading element binder is
/// registered under `[Elem(i)]` and the rest binder under `[RestFrom(lead)]` (the same `SumPayload` key the
/// body carries — see `resolve.rs`), so a `Core::SumPayload` read resolves to its binder. Only PLAIN binders
/// are emitted: a NESTED element sub-pattern (`(list (Mk x) …)` / `(list (list a ..) ..)`) resolves its
/// binder at a DEEPER path (`[Elem(i), Payload]` / `[Elem(i), Elem(j)]`) that this slice does not register,
/// so its body read misses the env and DECLINES rather than emit a wrong pattern. A GUARDED arm declines.
/// Arm order + conditions mirror the Core exactly, so the emitted match stays exhaustive (no CDZ0210).
fn emit_match_list(
    db: &mut Db,
    b: &mut Builder,
    scrutinee: StructId,
    arms: &[crate::core::ListArm],
    expected: Option<Ty>,
    env: &mut BinderEnv,
    emitted: &std::collections::HashSet<StructId>,
) -> Result<StructId, Reject> {
    use crate::core::{ListArmCond, PathStep};
    let match_head = b.name("match");
    let scrut_node = emit_expr(db, b, scrutinee, None, env, emitted)?;
    let mut children = vec![match_head, scrut_node];
    for arm in arms {
        // Build the arm's surface pattern, registering each binder's `SumPayload` path for the body.
        let pattern = match arm.cond {
            ListArmCond::LenEq(n) => {
                let list_head = b.name("list");
                let mut pat = vec![list_head];
                for i in 0..n {
                    let name = synth_payload_name(env.next_payload);
                    env.next_payload += 1;
                    env.payloads
                        .insert((scrutinee, vec![PathStep::Elem(i)]), name.clone());
                    pat.push(b.name(name));
                }
                b.list(pat)
            }
            ListArmCond::LenGe(lead) => {
                let list_head = b.name("list");
                let mut pat = vec![list_head];
                for i in 0..lead {
                    let name = synth_payload_name(env.next_payload);
                    env.next_payload += 1;
                    env.payloads
                        .insert((scrutinee, vec![PathStep::Elem(i)]), name.clone());
                    pat.push(b.name(name));
                }
                // The `..` separator, then the rest binder (the tail sublist from `lead` onward).
                pat.push(b.name(".."));
                let rest = synth_payload_name(env.next_payload);
                env.next_payload += 1;
                env.payloads
                    .insert((scrutinee, vec![PathStep::RestFrom(lead)]), rest.clone());
                pat.push(b.name(rest));
                b.list(pat)
            }
            // A whole-list catch-all — the bare wildcard `_`. The whole-list value, if the body reads it,
            // comes through the scrutinee's own name (not a `SumPayload`), so no binder is registered.
            ListArmCond::Any => b.name("_"),
        };
        // A GUARDED arm wraps its pattern in the `(guard <pattern> <cond>)` surface form (`resolve.rs`
        // Case 6lg): the arm fires only when its length condition AND `cond` hold, and otherwise FALLS
        // THROUGH to the next arm — the surface reader re-lowers `(guard …)` with that same fall-through.
        // The cond is emitted with this arm's element/rest binders IN SCOPE (registered above), so a guard
        // reading a bound element resolves to its binder. A guarded arm does not count toward exhaustiveness
        // (upstream guarantees an unguarded covering tail), so mirroring the arm keeps the match exhaustive.
        let pattern = match arm.guard {
            Some(g) => {
                let guard_head = b.name("guard");
                let cond = emit_expr(db, b, g, None, env, emitted)?;
                b.list(vec![guard_head, pattern, cond])
            }
            None => pattern,
        };
        let body_node = emit_expr(db, b, arm.body, expected.clone(), env, emitted)?;
        children.push(b.list(vec![pattern, body_node]));
    }
    Ok(b.list(children))
}

/// Re-emit a scalar match's arms as a nested `if`-chain, from arm `i` onward. The LAST arm (or an
/// UNGUARDED wildcard) is unconditional — its body IS the else: a scalar `Core::Match` is exhaustive
/// (checked upstream), so its final/wildcard arm covers the residual case. Each earlier arm wraps
/// `(if <cond> <body> <rest>)`, where `<cond>` is the probe test `(= <scrutinee> <lit>)`, the arm's GUARD,
/// or their conjunction `(and (= <scrutinee> <lit>) <guard>)`: a GUARDED arm fires only when its probe AND
/// guard hold and otherwise FALLS THROUGH to the rest — exactly the `if`/`else` chain, so a guard needs no
/// surface `match`, it desugars into the condition (a guarded Wild arm is just `(if <guard> body rest)`).
/// The guard's binder is the scrutinee (a bare-binder pattern binds the whole scalar), which lowering
/// resolves to the scrutinee's own core, so emitting the guard re-emits the scrutinee reference in scope.
/// The scrutinee is a pure scalar, re-emitted per probe. Precondition (caller): every probe is
/// `Int`/`Bool`/`Wild`. (A guarded arm does not count toward exhaustiveness, so the final covering arm is
/// always unguarded — a guarded LAST arm would be a non-exhaustive shape and declines defensively.)
// The recursion threads the shared emit state (db/builder/scrutinee/arms/index) plus the `expected` type
// and the binder env — each is load-bearing, so the arg count is intrinsic, not a bundling opportunity.
#[allow(clippy::too_many_arguments)]
fn emit_match_chain(
    db: &mut Db,
    b: &mut Builder,
    scrutinee: StructId,
    arms: &[crate::core::MatchArm],
    i: usize,
    expected: Option<Ty>,
    env: &mut BinderEnv,
    emitted: &std::collections::HashSet<StructId>,
) -> Result<StructId, Reject> {
    let arm = &arms[i];
    let is_last = i + 1 == arms.len();
    // An UNGUARDED last arm, or an UNGUARDED wildcard (always matches → any later arm is dead): the
    // unconditional else. A wildcard arm may BIND the scrutinee; its body reads that binder, which lowering
    // resolves to the scrutinee's own core, so emitting the body re-emits the scrutinee reference in scope.
    // The body is the match's value/tail position, so it inherits the match's `expected` type.
    let unguarded_wild = matches!(arm.probe, crate::core::Probe::Wild) && arm.guard.is_none();
    if (is_last && arm.guard.is_none()) || unguarded_wild {
        return emit_expr(db, b, arm.body, expected, env, emitted);
    }
    if is_last {
        // A guarded final arm cannot cover the residual case (its guard may fail) — a non-exhaustive shape
        // that should not arise from a well-formed match; decline rather than build a chain with no tail.
        return Err(Reject::decline(
            "the Cadenza backend does not lower a GUARDED final scalar-match arm (no covering tail)"
                .to_string(),
        ));
    }
    // The probe test `(= <scrutinee> <lit>)`, if this arm probes a literal (a Wild arm has no probe test —
    // only its guard gates it).
    let probe_cond = match &arm.probe {
        crate::core::Probe::Int(v) => {
            let eq = b.name("=");
            let scrut = emit_expr(db, b, scrutinee, None, env, emitted)?;
            let lit = b.atom_leaf(Leaf::Int {
                value: v.clone(),
                radix: Radix::Dec,
            });
            Some(b.list(vec![eq, scrut, lit]))
        }
        crate::core::Probe::Bool(x) => {
            let eq = b.name("=");
            let scrut = emit_expr(db, b, scrutinee, None, env, emitted)?;
            let lit = b.atom_leaf(Leaf::Bool(*x));
            Some(b.list(vec![eq, scrut, lit]))
        }
        crate::core::Probe::Wild => None,
        // The caller pre-scanned the arms to only Int/Bool/Wild.
        _ => {
            return Err(Reject::decline(
                "the Cadenza backend does not yet lower this match probe".to_string(),
            ));
        }
    };
    // The arm's guard (a boolean the scrutinee-binder is in scope for), if present.
    let guard_cond = match arm.guard {
        Some(g) => Some(emit_expr(db, b, g, None, env, emitted)?),
        None => None,
    };
    // The full condition: probe alone, guard alone, or `(and probe guard)`. At least one is present here
    // (an unguarded Wild was returned above as the unconditional else).
    let cond = match (probe_cond, guard_cond) {
        (Some(p), Some(g)) => {
            let and = b.name("and");
            b.list(vec![and, p, g])
        }
        (Some(p), None) => p,
        (None, Some(g)) => g,
        (None, None) => {
            unreachable!("an unguarded Wild arm is the unconditional else, handled above")
        }
    };
    let if_head = b.name("if");
    let body = emit_expr(db, b, arm.body, expected.clone(), env, emitted)?;
    let rest = emit_match_chain(db, b, scrutinee, arms, i + 1, expected, env, emitted)?;
    Ok(b.list(vec![if_head, cond, body, rest]))
}

/// The `expected` type to pass down to the value/tail children (branches, arm bodies) of a container node
/// `id` (an `if` / `match`): PREFER the container's OWN solved type (the join of its branches — usually
/// concrete, e.g. an `if` returning `Option<Int64>`), falling back to the `incoming` expected only when the
/// container's own type is itself under-determined (e.g. a match whose arms are all `(None)`, whose join is
/// `Option<?>` — then the concrete type comes from further out). This is what lets a bare `(None)` in a
/// branch/arm body recover its element type. Cheap `Ty` clone; called once per container.
fn body_ctx(db: &mut Db, id: StructId, incoming: Option<Ty>) -> Option<Ty> {
    let own = crate::infer::type_of(db, id);
    if ty_has_free_arg(&own) {
        incoming
    } else {
        Some(own)
    }
}

/// Whether a solved type is UNDER-DETERMINED — it contains a free type variable (`Ty::Var`) or the
/// unconstrained `Ty::Any`, so `lower::type_ast` cannot render it (it returns `None` for those). Used by the
/// `Core::SumNew` emit to decide whether the node's OWN solved type is usable, or whether it must fall back
/// to the `expected` type its context supplied (e.g. a bare `(None)` whose own type is `Option<?>`). Walks
/// the type's structure so a free arg NESTED inside a compound (`Option<List<?>>`) is caught too.
fn ty_has_free_arg(ty: &Ty) -> bool {
    match ty {
        Ty::Var(_) | Ty::Any => true,
        Ty::List(t) | Ty::Set(t) => ty_has_free_arg(t),
        Ty::Map(k, v) => ty_has_free_arg(k) || ty_has_free_arg(v),
        Ty::Tuple(ts) => ts.iter().any(ty_has_free_arg),
        Ty::Sum { args, .. } | Ty::Nominal { args, .. } => args.iter().any(ty_has_free_arg),
        Ty::Record(fs) => fs.values().any(ty_has_free_arg),
        Ty::Qty { inner, .. } => ty_has_free_arg(inner),
        Ty::Fn(a, r) => ty_has_free_arg(a) || ty_has_free_arg(r),
        Ty::Cont { resume, answer } => ty_has_free_arg(resume) || ty_has_free_arg(answer),
        _ => false,
    }
}

/// `(. <operand> <key>)` — the member-access form the reader normalizes a dotted `X.key` to. Used to
/// re-emit a wrapper constant's CONSTRUCTOR (`(Symbol.of …)`, `(BigInt.of …)`, `(Rational.of …)`), whose
/// value-form is not valid expression syntax. Mirrors `lower::member_access`.
fn member_access(b: &mut Builder, operand: &str, key: &str) -> StructId {
    let dot = b.name(".");
    let op = b.name(operand);
    let k = b.name(key);
    b.list(vec![dot, op, k])
}

/// The SURFACE operator a runtime-operator prim re-emits as, or `None` for a prim that is not a binary
/// operator (defensive — such a prim never appears in `Arith`/`Compare`/`StrCmp`/`FloatCompare`). The
/// reverse of `Prim::from_name` for the operator subset. Each INTERNAL float prim maps to the SAME
/// surface operator as its integer twin (`FAdd`→`+`, `FEq`→`=`, `FLt`→`<`, …): the author writes one
/// operator and `lower` selects the prim by the operands' solved type, so re-emitting the shared surface
/// operator re-solves to the same prim on recompile — the property round-trip idempotence rests on.
fn prim_operator(op: crate::resolved::Prim) -> Option<&'static str> {
    use crate::resolved::Prim::*;
    Some(match op {
        Add | FAdd => "+",
        Sub | FSub => "-",
        Mul | FMul => "*",
        Div | FDiv => "/",
        Rem => "%",
        Shl => "<<",
        Shr => ">>",
        BitAnd => "&",
        BitOr => "|",
        BitXor => "^",
        Lt | FLt => "<",
        Gt | FGt => ">",
        Le | FLe => "<=",
        Ge | FGe => ">=",
        Eq | FEq => "=",
        Compare => "compare",
        _ => return None,
    })
}

/// A short human-readable kind name for a `Core` node, for the decline message (so a decline says WHICH
/// construct is not yet lowered rather than an opaque debug dump).
fn core_node_kind(c: &Core) -> &'static str {
    match c {
        Core::ConstInt(_) => "ConstInt",
        Core::ConstRational(..) => "ConstRational",
        Core::ConstBool(_) => "ConstBool",
        Core::ConstStr(_) => "ConstStr",
        Core::ConstChar(_) => "ConstChar",
        Core::ConstFloat(_) => "ConstFloat",
        Core::ConstFloatNan => "ConstFloatNan",
        Core::Unit => "Unit",
        _ => "a non-constant node",
    }
}
// Behavioral coverage for this backend lives in the CORPUS round-trip check through the nix per-case
// pipeline (operator directive: e2e behavior belongs in the conformance/corpus suite, NEVER a Rust
// `#[test]`), not here — see the `corpus-cadenza` target (coordinated with v-nix).
