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
//!   probes (value-equivalent; the scrutinee is a pure scalar). Guards desugar to `if` at lowering (so
//!   they never reach here); a non-scalar probe (Str/Char/Bytes/list/map) declines.
//! - **DATA**: runtime compound VALUES — `Core::Tuple`→`(tuple …)`, `Core::Record`→`(record (= k v)…)`
//!   (name-sorted), `Core::ListNew`→`(list …)`; and a `Core::SumNew` variant →
//!   `(: (<Variant> <payload-or-unit>) <sum-type>)` (the type ascription pins an under-determined sum,
//!   e.g. a bare `(None unit)`; `type_ast` declines a free-type-arg sum). All mirror lower's value surface.
//!   A USER sum is re-declared: `emit` emits its `(type <Name> (<Variant> <PayloadTy>…)…)` decl (for a
//!   MONOMORPHIC, CLOSED, MULTI-variant sum — recursive payloads OK) and its values then round-trip; a
//!   GENERIC / OPEN / SINGLE-variant (optimizer-erased) user sum, and a user `Nominal` newtype, still
//!   DECLINE. PRELUDE sums (Option/Result/…) are ambient (no decl). A user-sum value emits ⇔ its decl was
//!   emitted (`emitted` set), so there is never an unbound-type recompile.
//!
//! Still declining, for later increments: closures (Closure/Captured/CallClosure), sequencing
//! (Seq/Block/Break), map/set values, sum/list MATCHES (MatchSum/MatchList) + non-scalar match probes,
//! and a multi-argument variant.

use crate::ast::{Builder, Leaf, Radix, StructId};
use crate::core::Core;
use crate::db::Db;
use crate::diag::Reject;
use crate::layout::Layout;
use crate::lower::core_of;
use crate::ty::Ty;
use std::collections::HashMap;

/// The in-scope `let`-binding environment: a map from a kept binding's binder occurrence (the
/// initializer `StructId` a `Core::LocalRef` resolves to) to the SYNTHESIZED surface name this backend
/// gives it. A `Core::Let` binding carries only its initializer occurrence — the source binding name is
/// discarded at lowering — so the backend mints fresh names DETERMINISTICALLY (by binding order, see
/// [`synth_binding_name`]); the same Core always yields the same names, which is what makes the
/// re-emitted `let` round-trip byte-identically. Threaded (as `&mut`) through [`emit_expr`] so a
/// `LocalRef` in a `let` body resolves to the name its binding was minted.
type BinderEnv = HashMap<StructId, std::rc::Rc<str>>;

/// The deterministic synthesized surface name for the `i`th kept `let` binding encountered in an emit
/// walk. Positional (not derived from the binder's `StructId`, which differs between the two arenas of a
/// round-trip), so compile-then-recompile mints the SAME name for the structurally-same binding. The
/// `_cdz_let` prefix keeps it out of the way of ordinary source identifiers.
fn synth_binding_name(i: usize) -> std::rc::Rc<str> {
    format!("_cdz_let{i}").into()
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
    // A fresh binding environment per definition — a `let` in the body populates it.
    let mut env = BinderEnv::new();
    let body_node = emit_expr(db, b, body, &mut env, emitted)?;
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
fn emit_expr(
    db: &mut Db,
    b: &mut Builder,
    id: StructId,
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
            let nm = env.get(&binder).ok_or_else(|| {
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
            let l = emit_expr(db, b, lhs, env, emitted)?;
            let r = emit_expr(db, b, rhs, env, emitted)?;
            Ok(b.list(vec![head, l, r]))
        }
        // Boolean negation `(not x)`.
        Core::Not { operand } => {
            let head = b.name("not");
            let x = emit_expr(db, b, operand, env, emitted)?;
            Ok(b.list(vec![head, x]))
        }
        // Short-circuiting conjunction / disjunction — `is_and` picks `and` vs `or`.
        Core::And { lhs, rhs, is_and } => {
            let head = b.name(if is_and { "and" } else { "or" });
            let l = emit_expr(db, b, lhs, env, emitted)?;
            let r = emit_expr(db, b, rhs, env, emitted)?;
            Ok(b.list(vec![head, l, r]))
        }
        // A two-way conditional `(if cond then else)`.
        Core::If { cond, then_, else_ } => {
            let head = b.name("if");
            let c = emit_expr(db, b, cond, env, emitted)?;
            let t = emit_expr(db, b, then_, env, emitted)?;
            let e = emit_expr(db, b, else_, env, emitted)?;
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
                let name = synth_binding_name(env.len());
                let name_atom = b.name(name.clone());
                // The value is emitted with only the PRIOR bindings in scope (a binding's initializer
                // cannot reference itself), then this binding is registered for the rest of the sequence.
                let value_node = emit_expr(db, b, value, env, emitted)?;
                env.insert(binder, name);
                binding_nodes.push(b.list(vec![name_atom, value_node]));
            }
            let bindings_list = b.list(binding_nodes);
            let body_node = emit_expr(db, b, body, env, emitted)?;
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
                children.push(emit_expr(db, b, arg, env, emitted)?);
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
                if arm.guard.is_some() {
                    return Err(Reject::decline(
                        "the Cadenza backend does not yet lower a GUARDED match arm".to_string(),
                    ));
                }
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
            emit_match_chain(db, b, scrutinee, &arms, 0, env, emitted)
        }
        // A runtime TUPLE value `(tuple <e>…)` — a fixed-arity positional product built from runtime
        // operands (a projection of a compile-time-visible tuple folds away in `lower`, so a surviving
        // `Core::Tuple` is a runtime value). Mirrors lower's constant value surface.
        Core::Tuple { elems } => {
            let head = b.name("tuple");
            let mut children = Vec::with_capacity(1 + elems.len());
            children.push(head);
            for e in elems.iter().copied() {
                children.push(emit_expr(db, b, e, env, emitted)?);
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
                let fval = emit_expr(db, b, v, env, emitted)?;
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
                children.push(emit_expr(db, b, e, env, emitted)?);
            }
            Ok(b.list(children))
        }
        // A runtime SUM (variant) value `(<Variant> <payload>)` — a constructed variant built from a
        // runtime payload. The variant NAME is recovered from the discriminant against the node's solved
        // sum type (`variant_head_ast` — bare, or `(. Type Variant)` when the name would collide with a
        // non-ctor prelude binding). A nullary variant carries `unit` (`(None unit)`), a single-payload
        // variant its payload; a multi-argument variant surface is not canonical and declines. Mirrors
        // lower's constant value surface.
        Core::SumNew { disc, payloads } => {
            let ty = crate::infer::type_of(db, id);
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
                1 => emit_expr(db, b, payloads[0], env, emitted)?,
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
        other => Err(Reject::decline(format!(
            "the Cadenza backend does not yet lower this Core node back to Cadenza: {}",
            core_node_kind(&other)
        ))),
    }
}

/// Re-emit a scalar match's arms as a nested `if`-chain, from arm `i` onward. The LAST arm (or a
/// wildcard arm) is unconditional — its body IS the else: a scalar `Core::Match` is exhaustive (checked
/// upstream), so its final/wildcard arm covers the residual case. Each earlier literal-probe arm wraps
/// `(if (= <scrutinee> <lit>) <body> <rest>)`. The scrutinee is a pure scalar, re-emitted per probe.
/// Precondition (checked by the caller): every arm is unguarded with an `Int`/`Bool`/`Wild` probe.
fn emit_match_chain(
    db: &mut Db,
    b: &mut Builder,
    scrutinee: StructId,
    arms: &[crate::core::MatchArm],
    i: usize,
    env: &mut BinderEnv,
    emitted: &std::collections::HashSet<StructId>,
) -> Result<StructId, Reject> {
    let arm = &arms[i];
    // The last arm, or a wildcard (which always matches, making any later arm dead): unconditional else.
    // A wildcard arm may BIND the scrutinee; its body reads that binder, which lowering resolves to the
    // scrutinee's own core, so emitting the body re-emits the scrutinee reference in scope.
    if i + 1 == arms.len() || matches!(arm.probe, crate::core::Probe::Wild) {
        return emit_expr(db, b, arm.body, env, emitted);
    }
    let if_head = b.name("if");
    let eq = b.name("=");
    let scrut = emit_expr(db, b, scrutinee, env, emitted)?;
    let lit = match &arm.probe {
        crate::core::Probe::Int(v) => b.atom_leaf(Leaf::Int {
            value: v.clone(),
            radix: Radix::Dec,
        }),
        crate::core::Probe::Bool(x) => b.atom_leaf(Leaf::Bool(*x)),
        // The caller pre-scanned the arms to only Int/Bool/Wild; a non-last non-Wild arm is Int/Bool.
        _ => {
            return Err(Reject::decline(
                "the Cadenza backend does not yet lower this match probe".to_string(),
            ));
        }
    };
    let cond = b.list(vec![eq, scrut, lit]);
    let body = emit_expr(db, b, arm.body, env, emitted)?;
    let rest = emit_match_chain(db, b, scrutinee, arms, i + 1, env, emitted)?;
    Ok(b.list(vec![if_head, cond, body, rest]))
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
