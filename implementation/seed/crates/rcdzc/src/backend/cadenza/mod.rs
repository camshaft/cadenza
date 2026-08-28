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
//! - **B0**: whole-program shape (`(do (def …)… (export …)…)`) with CONSTANT-bodied definitions —
//!   the constant leaves (Int/Bool/Str/Char/Float).
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
//! Still declining, for later increments: closures (Closure/Captured/CallClosure), sequencing
//! (Seq/Block/Break), and data (Record/Tuple/sums/collections — B4), plus scalar `Match`.

use crate::ast::{Builder, Leaf, Radix, StructId};
use crate::core::Core;
use crate::db::Db;
use crate::diag::Reject;
use crate::layout::Layout;
use crate::lower::core_of;
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

    // One `(def …)` per reachable definition, in layout order (a stable, target-neutral order).
    for &def in &layout.order {
        root_children.push(emit_def(db, &mut b, def)?);
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

/// Reconstruct `(def (<name> (: <p> <Ty>)…) <body>)` for definition `def`. B1a handles NULLARY defs and
/// parameterized defs whose parameters have a value-form-representable type; a parameter of a type with
/// no surface (a function/continuation/unsolved type — `type_ast` returns `None`) declines.
fn emit_def(db: &mut Db, b: &mut Builder, def: usize) -> Result<StructId, Reject> {
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
    let body_node = emit_expr(db, b, body, &mut env)?;
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
) -> Result<StructId, Reject> {
    match core_of(db, id) {
        // An integer constant re-reads to the same value regardless of the base its text used (the base
        // is display-only and Core does not retain it), so emit the canonical decimal spelling.
        Core::ConstInt(v) => Ok(b.atom_leaf(Leaf::Int {
            value: v,
            radix: Radix::Dec,
        })),
        Core::ConstBool(bo) => Ok(b.atom_leaf(Leaf::Bool(bo))),
        Core::ConstStr(s) => Ok(b.atom_leaf(Leaf::Str(s))),
        Core::ConstChar(c) => Ok(b.atom_leaf(Leaf::Char(c))),
        // A finite float constant carries its exact `Decimal` (no `f64` rounding), which re-reads to the
        // same leaf. (`ConstFloatNan` has no finite `Decimal` and no plain written form — a later slice.)
        Core::ConstFloat(d) => Ok(b.atom_leaf(Leaf::Float(d))),
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
            let l = emit_expr(db, b, lhs, env)?;
            let r = emit_expr(db, b, rhs, env)?;
            Ok(b.list(vec![head, l, r]))
        }
        // Boolean negation `(not x)`.
        Core::Not { operand } => {
            let head = b.name("not");
            let x = emit_expr(db, b, operand, env)?;
            Ok(b.list(vec![head, x]))
        }
        // Short-circuiting conjunction / disjunction — `is_and` picks `and` vs `or`.
        Core::And { lhs, rhs, is_and } => {
            let head = b.name(if is_and { "and" } else { "or" });
            let l = emit_expr(db, b, lhs, env)?;
            let r = emit_expr(db, b, rhs, env)?;
            Ok(b.list(vec![head, l, r]))
        }
        // A two-way conditional `(if cond then else)`.
        Core::If { cond, then_, else_ } => {
            let head = b.name("if");
            let c = emit_expr(db, b, cond, env)?;
            let t = emit_expr(db, b, then_, env)?;
            let e = emit_expr(db, b, else_, env)?;
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
                let value_node = emit_expr(db, b, value, env)?;
                env.insert(binder, name);
                binding_nodes.push(b.list(vec![name_atom, value_node]));
            }
            let bindings_list = b.list(binding_nodes);
            let body_node = emit_expr(db, b, body, env)?;
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
                children.push(emit_expr(db, b, arg, env)?);
            }
            Ok(b.list(children))
        }
        other => Err(Reject::decline(format!(
            "the Cadenza backend does not yet lower this Core node back to Cadenza: {}",
            core_node_kind(&other)
        ))),
    }
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
