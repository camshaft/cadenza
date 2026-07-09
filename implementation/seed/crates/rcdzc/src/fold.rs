//! `eval` — the compile-time evaluator (constant folding), the ONE compile-time tier.
//!
//! metaprogramming.md §"Compile-Time Evaluation Is One Tier": *"Macro expansion, generic reduction,
//! monomorphization, and constant folding MUST be the same compile-time evaluation mechanism."* This
//! pass is that mechanism; today it does constant folding + call β-reduction, and the same reduction
//! extends to generics/macros later. It is pure and deterministic (metaprogramming.md §Compile-Time
//! Evaluation Is Pure) — it reads no host, no clock, no randomness.
//!
//! ## Three-way fold, over `Mir` itself (no separate value type)
//! Each node folds to one of three shapes, all expressed IN `Mir`:
//!  - a **const** — `Int`/`Bool`/`Unit`, or a `Tuple` of consts (recognized by [`is_const`]);
//!  - a **poison** — `Mir::Error(reject)`: a constant operation that has NO value (it overflows /
//!    divides by zero / shifts out of range). Poison propagates through strict operators like a value,
//!    but is DROPPED by dead-code elimination at an unreached branch;
//!  - **dynamic** — anything else (a `Local`, a runtime `Call`, or a node with a dynamic child), left
//!    as runtime code with its children folded.
//!
//! ## Traps are compile-time diagnostics, but only when REACHED (operator rulings)
//! A constant operation that would trap at run time is instead POISON carrying a `CDZ0304` diagnostic
//! — the compiler fails compilation rather than shipping a component that traps
//! (core-semantics.md §Member Access sets the precedent: a compile-time-knowable error is a
//! compile-time rejection, never a deferred runtime trap). But a poison in a branch that folding
//! proves is never taken is DROPPED: folding `(if false <poison> 42)` selects `42` and discards the
//! poison. So reachability falls out of the fold — a poison that SURVIVES to a reached position is
//! surfaced as the compilation's diagnostic (by the pipeline), and one that does not simply vanishes.
//! A trap over RUNTIME operands is never folded and stays a runtime trap (`select` emits the guard) —
//! folding neither manufactures a trap the source did not denote nor erases one it did
//! (spec/learnings/2026-07-06-constant-folding-must-preserve-runtime-traps.md).

use crate::diag::Code;
use crate::ir::{ArithOp, BitOp, CmpOp, Mir, MirModule, Reject, ShiftOp};

/// Run the compile-time evaluator over a whole module. Module scope (not per-function) so a `Call` to
/// a constant-argument non-recursive callee can β-reduce against the callee's body.
pub fn eval_module(module: MirModule) -> MirModule {
    // Snapshot each function's body + arity, and which functions are (transitively) recursive — a
    // recursive callee is never inlined (it would not terminate). The bodies are the PRE-fold lowered
    // bodies; inlining folds a fresh copy, so a callee's own folding is independent.
    let bodies: Vec<Mir> = module.funcs.iter().map(|f| f.body.clone()).collect();
    let arities: Vec<usize> = module.funcs.iter().map(|f| f.params.len()).collect();
    let recursive = recursive_set(&bodies);
    // The fresh-local supply for α-renaming, seeded ABOVE every local id used across ALL bodies (+
    // params). β-reduction / inlining splices one function's body (whose local ids start at 0) into
    // another scope, so a spliced binder id can COLLIDE with the host scope's ids; `select` keys its
    // wasm slot by resolve-id with no scope restore, so a collision MISCOMPILES. α-renaming rewrites
    // every spliced binding occurrence to a fresh id (from this supply) before substituting.
    let seed = bodies
        .iter()
        .zip(&arities)
        .map(|(b, &ar)| max_local_id(b).map_or(0, |m| m + 1).max(ar as u32))
        .max()
        .unwrap_or(0);
    let ctx = Ctx {
        bodies: &bodies,
        arities: &arities,
        recursive: &recursive,
        fresh: std::cell::Cell::new(seed),
    };

    let mut funcs = module.funcs;
    for f in &mut funcs {
        let body = std::mem::replace(&mut f.body, Mir::Unit);
        f.body = ctx.fold(body);
    }
    MirModule { funcs, exports: module.exports }
}

/// Check the erasure fence (Layer 1 first-class types): a compile-time-only value must never survive
/// folding. Walk the Mir tree; on the first type-value leak, return a CDZ0305 reject. This is the
/// STRUCTURAL check that catches a type-value smuggled inside a heap compound — either as a bare
/// `Mir::TypeVal` node, OR as a value carried in a slot whose SOLVED type `is_comptime_only` (a
/// type-value bound to a local, a compound field/element/payload whose type contains `Ty::Type`).
/// Checking the whole `Ty` (not just leaf `Mir::TypeVal`) is the load-bearing item (A) of the design.
pub fn check_erasure_fence(mir: &Mir) -> Option<Reject> {
    use crate::ty::is_comptime_only;
    // A bare TypeVal node is a direct leak.
    if matches!(mir, Mir::TypeVal(_)) {
        return Some(leak_reject());
    }
    // Recurse into compounds to find any nested TypeVal, checking each slot's SOLVED type too.
    match mir {
        Mir::Tuple(elems) => {
            for (ety, e) in elems {
                if is_comptime_only(ety) {
                    return Some(leak_reject());
                }
                if let Some(r) = check_erasure_fence(e) {
                    return Some(r);
                }
            }
        }
        Mir::List(elems) => {
            for (ety, e) in elems {
                if is_comptime_only(ety) {
                    return Some(leak_reject());
                }
                if let Some(r) = check_erasure_fence(e) {
                    return Some(r);
                }
            }
        }
        Mir::Map(entries) => {
            for ((kt, k), (vt, v)) in entries {
                if is_comptime_only(kt) || is_comptime_only(vt) {
                    return Some(leak_reject());
                }
                if let Some(r) = check_erasure_fence(k) {
                    return Some(r);
                }
                if let Some(r) = check_erasure_fence(v) {
                    return Some(r);
                }
            }
        }
        Mir::Set(elems) => {
            for (ety, e) in elems {
                if is_comptime_only(ety) {
                    return Some(leak_reject());
                }
                if let Some(r) = check_erasure_fence(e) {
                    return Some(r);
                }
            }
        }
        Mir::HeapOp { args, .. } => {
            for (aty, a) in args {
                if is_comptime_only(aty) {
                    return Some(leak_reject());
                }
                if let Some(r) = check_erasure_fence(a) {
                    return Some(r);
                }
            }
        }
        Mir::Sum { payload_ty, payload, .. } => {
            if is_comptime_only(payload_ty) {
                return Some(leak_reject());
            }
            if let Some(r) = check_erasure_fence(payload) {
                return Some(r);
            }
        }
        Mir::Let { value_ty, value, body, .. } => {
            if is_comptime_only(value_ty) {
                return Some(leak_reject());
            }
            if let Some(r) = check_erasure_fence(value) {
                return Some(r);
            }
            if let Some(r) = check_erasure_fence(body) {
                return Some(r);
            }
        }
        Mir::If { cond, then_, else_, .. } => {
            if let Some(r) = check_erasure_fence(cond) {
                return Some(r);
            }
            if let Some(r) = check_erasure_fence(then_) {
                return Some(r);
            }
            if let Some(r) = check_erasure_fence(else_) {
                return Some(r);
            }
        }
        Mir::Arith(_, a, b) | Mir::Bit(_, a, b) | Mir::Shift(_, a, b) => {
            if let Some(r) = check_erasure_fence(a) {
                return Some(r);
            }
            if let Some(r) = check_erasure_fence(b) {
                return Some(r);
            }
        }
        Mir::Cmp { a, b, .. } => {
            if let Some(r) = check_erasure_fence(a) {
                return Some(r);
            }
            if let Some(r) = check_erasure_fence(b) {
                return Some(r);
            }
        }
        Mir::Proj { operand, .. } => {
            if let Some(r) = check_erasure_fence(operand) {
                return Some(r);
            }
        }
        Mir::Call { args, .. } => {
            for a in args {
                if let Some(r) = check_erasure_fence(a) {
                    return Some(r);
                }
            }
        }
        Mir::Apply { func, args } => {
            if let Some(r) = check_erasure_fence(func) {
                return Some(r);
            }
            for a in args {
                if let Some(r) = check_erasure_fence(a) {
                    return Some(r);
                }
            }
        }
        Mir::Match { scrutinee, arms, .. } => {
            if let Some(r) = check_erasure_fence(scrutinee) {
                return Some(r);
            }
            for (_pat, body) in arms {
                if let Some(r) = check_erasure_fence(body) {
                    return Some(r);
                }
            }
        }
        _ => {}
    }
    None
}

/// The erasure-fence reject — CDZ0305, a compile-time-only value reached the runtime boundary.
fn leak_reject() -> Reject {
    Reject::coded(
        crate::diag::Code::ComptimeErasure,
        "a compile-time-only value (a type-value) reached the runtime boundary".to_string(),
    )
}

/// Collect EVERY poison that survives to an UNCONDITIONALLY-REACHED position in a folded body — each
/// one the function always executes if it is called. Such a poison is a compile-time diagnostic (the
/// operator's ruling: a compile-time-knowable trap fails compilation, it is not shipped). Appends each
/// `Reject` to `out` — the compiler reports ALL of them, not the first (build-tool-interface.md,
/// Amendment 0.8.0; compiler-pipeline.md §Phases Recover From Errors).
///
/// "Reached" descends only STRICT positions: the operands of arithmetic/comparison/projection, the
/// elements of a product, a call's arguments, and BOTH a `let`'s value and body (both always evaluate).
/// It does NOT descend an `if`'s branches — a branch is conditionally reached, so a poison there is a
/// SHIELDED runtime trap (it stays a runtime `unreachable`, per the short-circuit-shielding contract:
/// folding never lifts a branch's trap out to unconditional position). After folding, a poison has
/// already propagated up through strict operators, so it only ever survives at the root, in a `let`
/// value/body, or inside an `if` branch — the first two are reached, the third is shielded.
pub fn collect_reached_poisons<'a>(mir: &'a Mir, out: &mut Vec<&'a Reject>) {
    match mir {
        Mir::Error(reject) => out.push(reject),
        Mir::Arith(_, a, b) | Mir::Bit(_, a, b) | Mir::Shift(_, a, b) => {
            collect_reached_poisons(a, out);
            collect_reached_poisons(b, out);
        }
        Mir::Cmp { a, b, .. } => {
            collect_reached_poisons(a, out);
            collect_reached_poisons(b, out);
        }
        Mir::Proj { operand, .. } => collect_reached_poisons(operand, out),
        Mir::Tuple(elems) => elems.iter().for_each(|(_, e)| collect_reached_poisons(e, out)),
        Mir::List(elems) => elems.iter().for_each(|(_, e)| collect_reached_poisons(e, out)),
        Mir::Map(entries) => entries.iter().for_each(|((_, k), (_, v))| {
            collect_reached_poisons(k, out);
            collect_reached_poisons(v, out);
        }),
        Mir::Set(elems) => elems.iter().for_each(|(_, e)| collect_reached_poisons(e, out)),
        Mir::HeapOp { args, .. } => args.iter().for_each(|(_, e)| collect_reached_poisons(e, out)),
        Mir::Call { args, .. } => args.iter().for_each(|a| collect_reached_poisons(a, out)),
        Mir::Apply { func, args } => {
            collect_reached_poisons(func, out);
            args.iter().for_each(|a| collect_reached_poisons(a, out));
        }
        Mir::Let { value, body, .. } => {
            collect_reached_poisons(value, out);
            collect_reached_poisons(body, out);
        }
        // The `if` CONDITION is strict (always evaluated); the BRANCHES are shielded — do not descend
        // them, so a poison in a runtime-conditional branch stays a runtime trap.
        Mir::If { cond, .. } => collect_reached_poisons(cond, out),
        // A sum's payload is strict (always built). A match's SCRUTINEE is strict; its ARM bodies are
        // shielded (conditionally reached) — like `if` branches, do not descend them.
        Mir::Sum { payload, .. } => collect_reached_poisons(payload, out),
        Mir::Match { scrutinee, .. } => collect_reached_poisons(scrutinee, out),
        // A lambda body is SHIELDED (conditional on application) — do NOT descend it (an un-applied
        // lambda's trap is conditional on application, like an `if` branch). A lambda body poison stays a
        // shielded runtime trap, not an unconditional build failure.
        Mir::Lambda { .. } => {}
        Mir::FuncRef(_) | Mir::Intrinsic(_) | Mir::Ctor { .. } | Mir::Wildcard | Mir::Trap(_)
        | Mir::Int(_) | Mir::Bool(_) | Mir::Str(_) | Mir::Unit | Mir::Local(_) | Mir::TypeVal(_) | Mir::TypeCtor(_) => {}
    }
}

/// The module context a fold needs to β-reduce a call. `fresh` is the α-renaming fresh-local supply
/// (a `Cell` so the otherwise-read-only fold can hand out fresh ids); it is seeded above every id used
/// across all bodies so a renamed id never collides with any function's original ids.
struct Ctx<'a> {
    bodies: &'a [Mir],
    arities: &'a [usize],
    recursive: &'a [bool],
    fresh: std::cell::Cell<u32>,
}

impl Ctx<'_> {
    /// Hand out a fresh local id (for α-renaming a spliced body's binders). Seeded above every id used
    /// across all bodies, so a fresh id never collides with any original id.
    fn fresh_local(&self) -> u32 {
        let id = self.fresh.get();
        self.fresh.set(id + 1);
        id
    }

    /// α-rename a body about to be spliced (inlined callee / β-reduced lambda): rewrite EVERY binding
    /// occurrence — `Let.id`, `Lambda.params`, and `Match`-arm pattern `Local` binders — to a FRESH id
    /// (threading a remap into their bound uses), so the spliced binders never collide with the host
    /// scope's ids. A `Local` use is rewritten iff a binder above it was renamed (in `remap`); a free
    /// `Local` (a callee param slot being substituted, or a captured caller local) is UNTOUCHED — its
    /// substitution happens separately. This is applied on every inline AND every β-reduction, which
    /// also hardens the const-inline path (nested same-id `let`s after two inlines are latent hazards).
    fn alpha_rename(&self, mir: Mir, remap: &mut std::collections::HashMap<u32, u32>) -> Mir {
        match mir {
            // A `Local` USE: if a binder above renamed this id, follow the remap; else leave it (a free
            // local — a param slot or captured outer local, handled by substitution, not renaming).
            Mir::Local(id) => Mir::Local(remap.get(&id).copied().unwrap_or(id)),
            Mir::Int(_) | Mir::Bool(_) | Mir::Str(_) | Mir::Unit | Mir::Error(_) | Mir::FuncRef(_)
            | Mir::Intrinsic(_) | Mir::Ctor { .. } | Mir::Wildcard | Mir::Trap(_) | Mir::TypeVal(_) | Mir::TypeCtor(_) => mir,
            Mir::Let { id, value_ty, value, body } => {
                // The bound VALUE is in the OUTER scope (the binder is not visible in it) — rename it
                // under the current remap. Then give the binder a fresh id and rename the body under the
                // extended remap. A shadow of an already-remapped id is overwritten for the body's extent
                // (correct lexical scoping) and restored after.
                let value = Box::new(self.alpha_rename(*value, remap));
                let fresh = self.fresh_local();
                let prev = remap.insert(id, fresh);
                let body = Box::new(self.alpha_rename(*body, remap));
                match prev {
                    Some(p) => { remap.insert(id, p); }
                    None => { remap.remove(&id); }
                }
                Mir::Let { id: fresh, value_ty, value, body }
            }
            Mir::Lambda { params, body } => {
                // Fresh id per param; rename the body under the extended remap; restore shadowed entries.
                let saved: Vec<(u32, Option<u32>)> = params
                    .iter()
                    .map(|&p| {
                        let fresh = self.fresh_local();
                        (p, remap.insert(p, fresh))
                    })
                    .collect();
                let new_params: Vec<u32> = params.iter().map(|p| remap[p]).collect();
                let body = Box::new(self.alpha_rename(*body, remap));
                for (p, prev) in saved.into_iter().rev() {
                    match prev {
                        Some(v) => { remap.insert(p, v); }
                        None => { remap.remove(&p); }
                    }
                }
                Mir::Lambda { params: new_params, body }
            }
            Mir::Match { scrutinee, scrut_ty, arms, ty } => {
                let scrutinee = Box::new(self.alpha_rename(*scrutinee, remap));
                // Each arm introduces its OWN pattern binders (scoped to that arm) — rename the pattern's
                // `Local` binders to fresh ids and its body under the same extended remap, then restore.
                let arms = arms
                    .into_iter()
                    .map(|(pat, body)| {
                        let mut introduced: Vec<(u32, Option<u32>)> = Vec::new();
                        let pat = self.alpha_rename_pattern(pat, remap, &mut introduced);
                        let body = self.alpha_rename(body, remap);
                        for (id, prev) in introduced.into_iter().rev() {
                            match prev {
                                Some(v) => { remap.insert(id, v); }
                                None => { remap.remove(&id); }
                            }
                        }
                        (pat, body)
                    })
                    .collect();
                Mir::Match { scrutinee, scrut_ty, arms, ty }
            }
            Mir::Sum { def, disc, payload_ty, payload } => Mir::Sum {
                def,
                disc,
                payload_ty,
                payload: Box::new(self.alpha_rename(*payload, remap)),
            },
            Mir::Call { func, args } => Mir::Call {
                func,
                args: args.into_iter().map(|a| self.alpha_rename(a, remap)).collect(),
            },
            Mir::Apply { func, args } => Mir::Apply {
                func: Box::new(self.alpha_rename(*func, remap)),
                args: args.into_iter().map(|a| self.alpha_rename(a, remap)).collect(),
            },
            Mir::Tuple(elems) => Mir::Tuple(
                elems.into_iter().map(|(t, e)| (t, self.alpha_rename(e, remap))).collect(),
            ),
            Mir::List(elems) => Mir::List(
                elems.into_iter().map(|(t, e)| (t, self.alpha_rename(e, remap))).collect(),
            ),
            Mir::Map(entries) => Mir::Map(
                entries
                    .into_iter()
                    .map(|((kt, k), (vt, v))| ((kt, self.alpha_rename(k, remap)), (vt, self.alpha_rename(v, remap))))
                    .collect(),
            ),
            Mir::Set(elems) => Mir::Set(
                elems.into_iter().map(|(t, e)| (t, self.alpha_rename(e, remap))).collect(),
            ),
            Mir::HeapOp { op, args } => Mir::HeapOp {
                op,
                args: args.into_iter().map(|(t, e)| (t, self.alpha_rename(e, remap))).collect(),
            },
            Mir::Proj { slot, elem_ty, operand } => Mir::Proj {
                slot,
                elem_ty,
                operand: Box::new(self.alpha_rename(*operand, remap)),
            },
            Mir::Arith(op, a, b) => Mir::Arith(
                op,
                Box::new(self.alpha_rename(*a, remap)),
                Box::new(self.alpha_rename(*b, remap)),
            ),
            Mir::Bit(op, a, b) => Mir::Bit(
                op,
                Box::new(self.alpha_rename(*a, remap)),
                Box::new(self.alpha_rename(*b, remap)),
            ),
            Mir::Shift(op, a, b) => Mir::Shift(
                op,
                Box::new(self.alpha_rename(*a, remap)),
                Box::new(self.alpha_rename(*b, remap)),
            ),
            Mir::Cmp { op, operand_ty, a, b } => Mir::Cmp {
                op,
                operand_ty,
                a: Box::new(self.alpha_rename(*a, remap)),
                b: Box::new(self.alpha_rename(*b, remap)),
            },
            Mir::If { cond, then_, else_, ty } => Mir::If {
                cond: Box::new(self.alpha_rename(*cond, remap)),
                then_: Box::new(self.alpha_rename(*then_, remap)),
                else_: Box::new(self.alpha_rename(*else_, remap)),
                ty,
            },
        }
    }

    /// α-rename a lowered PATTERN's `Local` binders to fresh ids, recording each `(orig, prev-remap)`
    /// in `introduced` so the caller can restore the remap after the arm. A pattern is a structural
    /// tree of `Sum`/`Tuple` with `Local`/`Wildcard`/literal leaves — the `Local`s are BINDERS here.
    fn alpha_rename_pattern(
        &self,
        pat: Mir,
        remap: &mut std::collections::HashMap<u32, u32>,
        introduced: &mut Vec<(u32, Option<u32>)>,
    ) -> Mir {
        match pat {
            Mir::Local(id) => {
                let fresh = self.fresh_local();
                introduced.push((id, remap.insert(id, fresh)));
                Mir::Local(fresh)
            }
            Mir::Sum { def, disc, payload_ty, payload } => Mir::Sum {
                def,
                disc,
                payload_ty,
                payload: Box::new(self.alpha_rename_pattern(*payload, remap, introduced)),
            },
            Mir::Tuple(elems) => Mir::Tuple(
                elems
                    .into_iter()
                    .map(|(t, e)| (t, self.alpha_rename_pattern(e, remap, introduced)))
                    .collect(),
            ),
            // Wildcard / literal leaves bind nothing.
            other => other,
        }
    }

    /// Fold one `Mir` node bottom-up. Returns a const, a poison (`Mir::Error`), or a dynamic node with
    /// folded children.
    fn fold(&self, mir: Mir) -> Mir {
        match mir {
            // Leaves — already fully reduced. `Error` is already a poison. `Str` is a heap-const leaf.
            // TypeVal/TypeCtor are compile-time-only leaves — they should never reach here (the fence
            // rejects them). Lambda is a transient compile-time value — already reduced (its application
            // is β-reduction).
            Mir::Int(_) | Mir::Bool(_) | Mir::Str(_) | Mir::Unit | Mir::Local(_) | Mir::Error(_) | Mir::TypeVal(_) | Mir::TypeCtor(_) | Mir::Lambda { .. } => mir,

            Mir::Arith(op, a, b) => self.fold_arith(op, self.fold(*a), self.fold(*b)),
            Mir::Bit(op, a, b) => self.fold_bit(op, self.fold(*a), self.fold(*b)),
            Mir::Shift(op, a, b) => self.fold_shift(op, self.fold(*a), self.fold(*b)),

            Mir::Cmp { op, operand_ty, a, b } => {
                let a = self.fold(*a);
                let b = self.fold(*b);
                // A poison operand poisons the comparison.
                if let Some(p) = first_poison(&a, &b) {
                    return p;
                }
                match (as_int(&a), as_int(&b), as_bool(&a), as_bool(&b)) {
                    (Some(x), Some(y), _, _) => Mir::Bool(cmp_int(op, x, y)),
                    (_, _, Some(x), Some(y)) => Mir::Bool(cmp_bool(op, x, y)),
                    // Unit == Unit is always true (one unit value); Unit has no other ordering.
                    _ if is_unit(&a) && is_unit(&b) && op == CmpOp::Eq => Mir::Bool(true),
                    _ => Mir::Cmp { op, operand_ty, a: Box::new(a), b: Box::new(b) },
                }
            }

            Mir::If { cond, then_, else_, ty } => {
                let cond = self.fold(*cond);
                match as_bool(&cond) {
                    // Constant condition = dead-code elimination: fold ONLY the taken branch, DROP the
                    // other (this is what discards a poison in an unreached branch).
                    Some(true) => self.fold(*then_),
                    Some(false) => self.fold(*else_),
                    None => {
                        // A poison CONDITION poisons the whole `if`.
                        if is_poison(&cond) {
                            return cond;
                        }
                        // Dynamic condition: keep the `if`, fold both branches. Each branch's own
                        // poison stays as runtime code (a runtime-conditional trap stays runtime).
                        Mir::If {
                            cond: Box::new(cond),
                            then_: Box::new(self.fold(*then_)),
                            else_: Box::new(self.fold(*else_)),
                            ty,
                        }
                    }
                }
            }

            Mir::Let { id, value_ty, value, body } => {
                let value = self.fold(*value);
                if is_poison(&value) {
                    return value;
                }
                // β-reduce a `let` whose value is a CONST or a TRANSIENT compile-time value (a `Ctor`/
                // `FuncRef`/`Intrinsic`/`TypeVal` — a function/constructor/type value that cannot be a
                // runtime local): substitute it into the body so `(let ((c None)) (c unit))` becomes
                // `Apply(Ctor, [unit])` → `Mir::Sum`, and `(let ((t Int64)) e)` substitutes the TypeVal.
                // A transient value MUST inline (it is never runtime-emittable as a local).
                if is_const(&value) || is_transient(&value) {
                    let body = substitute(*body, id, &value);
                    self.fold(body)
                } else {
                    Mir::Let {
                        id,
                        value_ty,
                        value: Box::new(value),
                        body: Box::new(self.fold(*body)),
                    }
                }
            }

            Mir::Tuple(elems) => {
                let folded: Vec<(crate::ty::Ty, Mir)> =
                    elems.into_iter().map(|(t, e)| (t, self.fold(e))).collect();
                // A poison element poisons the whole product.
                if let Some(p) = folded.iter().find_map(|(_, e)| if is_poison(e) { Some(e.clone()) } else { None }) {
                    return p;
                }
                Mir::Tuple(folded)
            }

            Mir::List(elems) => {
                // Fold each element; a poison element poisons the whole list (a strict construction).
                // A list is a runtime heap value (built by `vec-push`), so — like a data tuple — it is
                // NOT a const even when every element is: it stays a dynamic build for `select`.
                let folded: Vec<(crate::ty::Ty, Mir)> =
                    elems.into_iter().map(|(t, e)| (t, self.fold(e))).collect();
                if let Some(p) = folded.iter().find_map(|(_, e)| if is_poison(e) { Some(e.clone()) } else { None }) {
                    return p;
                }
                Mir::List(folded)
            }

            Mir::Map(entries) => {
                // A map is a runtime CHAMP value (built by `map-insert`) — NOT a const; a poison key or
                // value poisons the whole literal.
                let folded: Vec<((crate::ty::Ty, Mir), (crate::ty::Ty, Mir))> = entries
                    .into_iter()
                    .map(|((kt, k), (vt, v))| ((kt, self.fold(k)), (vt, self.fold(v))))
                    .collect();
                if let Some(p) = folded.iter().find_map(|((_, k), (_, v))| {
                    if is_poison(k) { Some(k.clone()) } else if is_poison(v) { Some(v.clone()) } else { None }
                }) {
                    return p;
                }
                Mir::Map(folded)
            }

            Mir::Set(elems) => {
                // A set is a runtime CHAMP value (built by `set-insert`) — NOT a const; a poison element
                // poisons the whole literal.
                let folded: Vec<(crate::ty::Ty, Mir)> =
                    elems.into_iter().map(|(t, e)| (t, self.fold(e))).collect();
                if let Some(p) = folded.iter().find_map(|(_, e)| if is_poison(e) { Some(e.clone()) } else { None }) {
                    return p;
                }
                Mir::Set(folded)
            }

            Mir::HeapOp { op, args } => {
                // Fold each argument (a strict build); a poison in any poisons the whole op.
                let folded: Vec<(crate::ty::Ty, Mir)> =
                    args.into_iter().map(|(t, e)| (t, self.fold(e))).collect();
                if let Some(p) = folded.iter().find_map(|(_, e)| if is_poison(e) { Some(e.clone()) } else { None }) {
                    return p;
                }
                Mir::HeapOp { op, args: folded }
            }

            Mir::Proj { slot, elem_ty, operand } => {
                let operand = self.fold(*operand);
                if is_poison(&operand) {
                    return operand;
                }
                // Projecting a slot of a LITERAL product node is the element itself — whether the
                // product is a data tuple/record `(. (tuple a b) 0)` or a MODULE record `(. m f)`
                // (which folded to a `Tuple` of `FuncRef`/value fields). This is sound for any literal
                // `Tuple` operand (the elements are in hand), so the product never reaches the heap
                // (`Layout::body_uses_heap`, run after this pass, then sees no heap use) — a module
                // record vanishes entirely, and even a data tuple projected in place needs no `arr`.
                if let Mir::Tuple(elems) = &operand {
                    if let Some((_, e)) = elems.get(slot) {
                        return e.clone();
                    }
                    // Slot out of range on a literal product — a compiler-internal inconsistency
                    // (infer proved the arity); leave it for select to handle rather than panic.
                }
                Mir::Proj { slot, elem_ty, operand: Box::new(operand) }
            }

            Mir::Call { func, args } => {
                let args: Vec<Mir> = args.into_iter().map(|a| self.fold(a)).collect();
                if let Some(p) = args.iter().find(|a| is_poison(a)) {
                    return p.clone();
                }
                self.try_inline(func, args)
            }

            // A bare function value / intrinsic / constructor — reduced only in an `Apply` (or projected
            // out of a record). A survivor is not runtime-emittable; `select` declines it. (TypeCtor is
            // already handled in the leaf catch-all above, alongside TypeVal/Lambda.)
            Mir::FuncRef(i) => Mir::FuncRef(i),
            Mir::Intrinsic(op) => Mir::Intrinsic(op),
            Mir::Ctor { def, index } => Mir::Ctor { def, index },
            Mir::Wildcard => Mir::Wildcard,
            Mir::Trap(msg) => Mir::Trap(msg),

            // A constructed sum — fold its payload; a poison payload poisons the sum. (A sum is a
            // runtime heap value, never a scalar const, so it stays a build.)
            Mir::Sum { def, disc, payload_ty, payload } => {
                let payload = self.fold(*payload);
                if is_poison(&payload) {
                    return payload;
                }
                Mir::Sum { def, disc, payload_ty, payload: Box::new(payload) }
            }

            // A match — fold the scrutinee and every arm body (pattern trees are left as lowered). A
            // poison scrutinee poisons the match. (Const-arm-selection is a later optimization; a const
            // scrutinee stays a runtime match — still correct, it builds+matches on the heap.)
            Mir::Match { scrutinee, scrut_ty, arms, ty } => {
                let scrutinee = self.fold(*scrutinee);
                if is_poison(&scrutinee) {
                    return scrutinee;
                }
                let arms = arms
                    .into_iter()
                    .map(|(p, b)| (p, self.fold(b)))
                    .collect();
                Mir::Match { scrutinee: Box::new(scrutinee), scrut_ty, arms, ty }
            }

            Mir::Apply { func, args } => {
                let func = self.fold(*func);
                let args: Vec<Mir> = args.into_iter().map(|a| self.fold(a)).collect();
                if is_poison(&func) {
                    return func;
                }
                if let Some(p) = args.iter().find(|a| is_poison(a)) {
                    return p.clone();
                }

                // SPINE COLLAPSE for curried application: if `func` is itself a residual `Apply` of a
                // FuncRef/Lambda OR an under-arity `Call`, gather the argument spine and re-dispatch at
                // the combined arity. `((add 3) 4)` where `add` is 2-arity → `Apply(Call{add,[3]},[4])`
                // → gather `[3,4]`, re-dispatch `Apply(FuncRef(add),[3,4])` → at full arity, inline or
                // reduce to `Call{add,[3,4]}`.
                let (callee, all_args) = match &func {
                    Mir::Apply { func: inner_func, args: inner_args }
                        if matches!(inner_func.as_ref(), Mir::FuncRef(_) | Mir::Lambda { .. }) => {
                        // Flatten: concat inner_args ++ outer args, extract the core callee.
                        let mut combined = inner_args.clone();
                        combined.extend(args);
                        (inner_func.as_ref().clone(), combined)
                    }
                    // An under-arity Call becomes a partial application — collapse it by gathering args.
                    Mir::Call { func: f, args: inner_args } => {
                        let mut combined = inner_args.clone();
                        combined.extend(args);
                        (Mir::FuncRef(*f), combined)
                    }
                    // No spine to collapse — proceed with the func as-is.
                    _ => (func.clone(), args),
                };

                // Apply of a resolved function value IS a direct call — reduce and fold it (β-reducing
                // const args, else a runtime call). A module `(. m f)` folds (via `Proj` of the module
                // record) to a `FuncRef`, so `((. m f) args)` collapses to a `Call` here.
                match callee {
                    Mir::FuncRef(i) => {
                        // NULLARY-as-value convention: `((. m answer) unit)` applies a nullary function
                        // value to `unit`. The callee takes no parameters, so drop a sole `unit` arg
                        // before the direct call (the unit is the calling convention, not a parameter).
                        let arity = self.arities.get(i).copied().unwrap_or(usize::MAX);
                        let final_args = if arity == 0 && all_args.len() == 1 && matches!(all_args[0], Mir::Unit) {
                            Vec::new()
                        } else {
                            all_args
                        };
                        // If under-arity after spine collapse, keep the flattened partial application.
                        if final_args.len() < arity {
                            return Mir::Apply { func: Box::new(Mir::FuncRef(i)), args: final_args };
                        }
                        self.try_inline(i, final_args)
                    }
                    // Apply of a LAMBDA — β-reduce: α-rename the body's bound locals to fresh ids (so a
                    // spliced binder never collides with the host scope's ids), then substitute each
                    // param := arg and fold. Exact arity only (over-application is a type error upstream).
                    Mir::Lambda { params, body } => {
                        if all_args.len() != params.len() {
                            // Under-arity partial application after collapse — keep it flattened.
                            return Mir::Apply { func: Box::new(Mir::Lambda { params, body }), args: all_args };
                        }
                        // α-rename the body's INNER binders (Let/Lambda/Match) to fresh ids; the lambda's
                        // OWN params stay (they are free in the body and about to be substituted away).
                        let mut remap = std::collections::HashMap::new();
                        let renamed = self.alpha_rename(*body, &mut remap);
                        let mut reduced = renamed;
                        for (param_id, arg) in params.iter().zip(&all_args) {
                            reduced = substitute(reduced, *param_id, arg);
                        }
                        self.fold(reduced)
                    }
                    // Apply of an INTRINSIC over all-CONSTANT args folds to the op's constant result
                    // (the op matches the value shapes it expects — not assumed integer); otherwise the
                    // applied intrinsic stays for `select` to lower to wasm instructions.
                    Mir::Intrinsic(op) => {
                        if all_args.iter().all(is_const) {
                            if let Some(v) = op.fold_const(&all_args) {
                                return v;
                            }
                        }
                        Mir::Apply { func: Box::new(Mir::Intrinsic(op)), args: all_args }
                    }
                    // Apply of a CONSTRUCTOR builds a heap sum `(disc, payload)` — the same reduction
                    // `lower` does for a directly-applied ctor, reached here when the ctor was `let`-bound
                    // and inlined (`(let ((c None)) (c unit))`). The single arg is the payload; its type
                    // is recovered from the arg's shape (enough to box it — a scalar/unit/compound).
                    Mir::Ctor { def, index } => {
                        let disc = index as u32;
                        let payload = all_args.into_iter().next().unwrap_or(Mir::Unit);
                        let payload_ty = mir_shape_ty(&payload);
                        Mir::Sum { def, disc, payload_ty, payload: Box::new(payload) }
                    }
                    // Apply of a TYPE CONSTRUCTOR builds a compound TypeVal — Layer 2 parametric types.
                    // `(List Int64)` → `TypeVal(Ty::List(Int))`. The args must ALL be TypeVals (type-value
                    // arguments); extract their `Ty`s and construct the compound type. This is the β-reduction
                    // that makes `(: e (List Int64))` work: the annotation's RHS folds to a TypeVal.
                    Mir::TypeCtor(kind) => {
                        use crate::ir::TypeCtorKind;
                        // Extract the Ty from each TypeVal argument.
                        let arg_tys: Vec<crate::ty::Ty> = all_args.iter().filter_map(|a| match a {
                            Mir::TypeVal(ty) => Some(ty.clone()),
                            _ => None,
                        }).collect();
                        // If not all args are TypeVals, decline (malformed / not-a-type-ctor-application).
                        if arg_tys.len() != all_args.len() {
                            return Mir::Apply { func: Box::new(Mir::TypeCtor(kind)), args: all_args };
                        }
                        // Build the compound TypeVal based on the constructor kind and argument types.
                        let compound_ty = match (kind, arg_tys.as_slice()) {
                            (TypeCtorKind::List, [elem]) => crate::ty::Ty::List(Box::new(elem.clone())),
                            (TypeCtorKind::Set, [elem]) => crate::ty::Ty::Set(Box::new(elem.clone())),
                            (TypeCtorKind::Map, [k, v]) => crate::ty::Ty::Map(Box::new(k.clone()), Box::new(v.clone())),
                            (TypeCtorKind::Tuple2, [a, b]) => crate::ty::Ty::Tuple(vec![a.clone(), b.clone()]),
                            (TypeCtorKind::Option, [a]) => crate::ty::Ty::Sum {
                                def: crate::ty::prelude_option(),
                                args: vec![a.clone()],
                            },
                            (TypeCtorKind::Result, [a, e]) => crate::ty::Ty::Sum {
                                def: crate::ty::prelude_result(),
                                args: vec![a.clone(), e.clone()],
                            },
                            // Arity mismatch or unsupported form — leave as Apply; `select` declines it.
                            _ => return Mir::Apply { func: Box::new(Mir::TypeCtor(kind)), args: all_args },
                        };
                        Mir::TypeVal(compound_ty)
                    }
                    // The applied value did not resolve to a function reference or intrinsic — not
                    // supported. Leave the `Apply`; `select` declines it.
                    other => Mir::Apply { func: Box::new(other), args: all_args },
                }
            }
        }
    }

    /// β-reduce a call when it is safe and useful: a NON-recursive callee, EVERY argument a compile-time
    /// value (const OR transient — a Lambda/FuncRef/Ctor/Intrinsic arg), and a result worth keeping.
    /// Otherwise leave a real runtime call with folded args. Widened from the old const-only guard to
    /// make HOFs reduce: a helper with a `Ty::Fn` parameter (a lambda/function arg) can now inline, letting
    /// `(ap (fn …) 7)` inline `ap` and β-reduce `(g v)`. A helper with a fn-typed parameter that is NOT
    /// fully inlined (exported directly, or recursive) correctly declines at select (Increment B).
    ///
    /// TODO (dead-function elimination, deferred): when a call is inlined here, the callee may lose its
    /// last caller and become unreachable — but it is still emitted (a function in the component nobody
    /// calls). A later pass should mark a function reachable iff it is exported OR called by a reachable
    /// function AFTER folding, and drop the unreachable ones (so an inlined-away helper adds no bytes).
    /// It is only wasted space today, never a correctness issue, so it is left for a DCE pass.
    fn try_inline(&self, func: usize, args: Vec<Mir>) -> Mir {
        let recursive = self.recursive.get(func).copied().unwrap_or(true);
        let arity = self.arities.get(func).copied().unwrap_or(usize::MAX);
        // Inline guard: inline a non-recursive callee when every arg is a compile-time value —
        // `is_const` OR `is_transient`. This is what lets HOFs reduce: `(ap (fn …) 7)` inlines `ap`.
        let all_comptime = args.iter().all(|a| is_const(a) || is_transient(a));
        if recursive || arity != args.len() || !all_comptime {
            return Mir::Call { func, args };
        }
        // α-rename the callee body's INNER binders (Let/Lambda/Match) to fresh ids so a spliced binder
        // never collides with the host scope's ids (`select` keys its wasm slot by resolve-id with no
        // scope restore — a collision would MISCOMPILE). The callee's PARAMS (`Local(0..arity)`) are
        // free in its body (not binders), so `alpha_rename` leaves them for substitution below. Then
        // substitute each arg for its param local and fold a fresh copy.
        let mut remap = std::collections::HashMap::new();
        let mut body = self.alpha_rename(self.bodies[func].clone(), &mut remap);
        for (i, arg) in args.iter().enumerate() {
            body = substitute(body, i as u32, arg);
        }
        let folded = self.fold(body);
        // KEEP the inlined result when it reduced to a scalar constant, a POISON, a MODULE RECORD, or a
        // TRANSIENT function value (a Lambda/FuncRef — so `((mk-adder 10) 5)` keeps the returned `Lambda`
        // for the outer `Apply(Lambda,[5])` to β-reduce). A poison here is a constant trap the compiler
        // PROVED by propagating the constant arguments through the callee — and the operator's ruling is
        // that a compile-time-knowable trap FAILS THE BUILD, wherever the compiler can prove it (this is
        // where the language shines). A dynamic result needs runtime data we do not have; leave the call
        // so a genuinely-runtime trap stays runtime (the emitted guard still exists for real input).
        // Do NOT keep a residual runtime value — that stays a `Call`, which then declines correctly if it
        // carries a fn-typed value.
        if is_scalar_const(&folded) || is_poison(&folded) || is_module_record(&folded) || is_transient(&folded) {
            folded
        } else {
            Mir::Call { func, args }
        }
    }

    fn fold_arith(&self, op: ArithOp, a: Mir, b: Mir) -> Mir {
        if let Some(p) = first_poison(&a, &b) {
            return p;
        }
        match (as_int(&a), as_int(&b)) {
            (Some(x), Some(y)) => {
                // Checked — matches select's ideal trapping sequence (select.rs:270). Overflow → a
                // constant with no value → poison (a compile-time diagnostic when reached).
                let r = match op {
                    ArithOp::Add => x.checked_add(y),
                    ArithOp::Sub => x.checked_sub(y),
                    ArithOp::Mul => x.checked_mul(y),
                };
                match r {
                    Some(v) => Mir::Int(v),
                    None => poison("integer overflow in a constant operation"),
                }
            }
            _ => Mir::Arith(op, Box::new(a), Box::new(b)),
        }
    }

    fn fold_bit(&self, op: BitOp, a: Mir, b: Mir) -> Mir {
        if let Some(p) = first_poison(&a, &b) {
            return p;
        }
        match (as_int(&a), as_int(&b)) {
            (Some(x), Some(y)) => {
                let r = match op {
                    BitOp::And => Some(x & y),
                    BitOp::Or => Some(x | y),
                    BitOp::Xor => Some(x ^ y),
                    // div_s / rem_s trap on divide-by-zero and on Int64.min / -1 — EXCEPT `% MIN -1`
                    // which wasm rem_s defines as 0 (never manufacture that trap — the learning's
                    // "modulo by -1 is zero even at the minimum integer").
                    BitOp::Div => x.checked_div(y),
                    BitOp::Rem => {
                        if y == -1 {
                            Some(0) // MIN % -1 = 0, and n % -1 = 0 for all n
                        } else {
                            x.checked_rem(y)
                        }
                    }
                };
                match r {
                    Some(v) => Mir::Int(v),
                    None => poison("division by zero (or Int64.min / -1) in a constant operation"),
                }
            }
            _ => Mir::Bit(op, Box::new(a), Box::new(b)),
        }
    }

    fn fold_shift(&self, op: ShiftOp, a: Mir, b: Mir) -> Mir {
        if let Some(p) = first_poison(&a, &b) {
            return p;
        }
        match (as_int(&a), as_int(&b)) {
            (Some(v), Some(count)) => {
                // Count guard: an out-of-range count (>= 64, or negative — a huge unsigned) traps
                // (select.rs:333). Constant → poison.
                if !(0..64).contains(&count) {
                    return poison("shift count out of range in a constant operation");
                }
                let c = count as u32;
                match op {
                    ShiftOp::Right => Mir::Int(v >> c),
                    ShiftOp::Left => {
                        // A left shift traps on overflow (a bit shifted past the sign) —
                        // `(v << c) >> c != v` (select.rs:344). Constant → poison.
                        let shifted = ((v as i128) << c) as i64;
                        if (shifted >> c) != v {
                            poison("left shift overflow in a constant operation")
                        } else {
                            Mir::Int(shifted)
                        }
                    }
                }
            }
            _ => Mir::Shift(op, Box::new(a), Box::new(b)),
        }
    }
}

// ─── const-shape readers ───────────────────────────────────────────────────────────────

/// Whether a node is a fully-known compile-time constant: a scalar literal, or a tuple all of whose
/// elements are constants. (A poison is NOT a const — it has no value.)
fn is_const(m: &Mir) -> bool {
    match m {
        Mir::Int(_) | Mir::Bool(_) | Mir::Unit => true,
        Mir::Tuple(elems) => elems.iter().all(|(_, e)| is_const(e)),
        _ => false,
    }
}

/// A TRANSIENT compile-time value — a constructor / function / intrinsic / type-value that is not
/// runtime-emittable and so must be INLINED at its binding site (a `let`), never left as a runtime
/// local. `(let ((c None)) (c unit))` substitutes the `Ctor` into the body so it reduces to a
/// `Mir::Sum`. `(let ((t Int64)) e)` substitutes the `TypeVal` into the body. These are the same
/// values `select` declines if a bare survivor reaches it.
///
/// A residual PARTIAL APPLICATION — an under-arity `Apply` whose (possibly nested) callee is a
/// `FuncRef`/`Lambda` — is ALSO transient: it is a compile-time function value awaiting its remaining
/// args. `(let ((add3 (add 3))) (add3 7))` binds `add3` to `Apply(Lambda,[3])`; inlining it lets the
/// outer `Apply(add3,[7])` spine-collapse to `[3,7]` and β-reduce. It is never runtime-emittable as a
/// local, so it MUST inline (a survivor to `select` declines — Increment B, an escaping partial app).
///
/// **Layer 2 addition:** a `TypeVal` is also transient — it is a compile-time-only value. Widening the
/// fold's compile-time-value predicate to admit TypeVal enables type-constructor applications to reduce.
fn is_transient(m: &Mir) -> bool {
    match m {
        Mir::Ctor { .. } | Mir::FuncRef(_) | Mir::Intrinsic(_) | Mir::TypeVal(_) | Mir::TypeCtor(_) | Mir::Lambda { .. } => true,
        // A residual partial application: `Apply(<transient fn value>, args)` — its callee is a
        // FuncRef/Lambda (directly, or itself a residual partial application via spine).
        Mir::Apply { func, .. } => matches!(func.as_ref(), Mir::FuncRef(_) | Mir::Lambda { .. })
            || is_transient(func),
        _ => false,
    }
}

/// Recover the boxing-relevant `Ty` of a `Mir` value from its SHAPE — used when the fold builds a
/// `Mir::Sum` from an inlined `Apply(Ctor)` and the payload's solved type is not at hand (the typed
/// tree is gone). Enough to pick the box/unbox accessor: `Int`/`Bool`/`Unit` scalars, or a compound
/// handle (a `Sum`/`Tuple`/`List` payload is already a handle — no box). A shape we cannot classify
/// defaults to `Unit` (no box), correct for the nullary-payload case this path most serves.
fn mir_shape_ty(m: &Mir) -> crate::ty::Ty {
    use crate::ty::Ty;
    match m {
        Mir::Int(_) => Ty::Int,
        Mir::Bool(_) => Ty::Bool,
        Mir::Sum { def, .. } => Ty::Sum { def: def.clone(), args: def.params.iter().map(|_| Ty::Unit).collect() },
        // A tuple/list payload is already a heap handle; any handle-typed shape needs no boxing, so a
        // placeholder compound (an empty tuple type) selects `box_op`/`unbox_op` = None correctly.
        Mir::Tuple(_) | Mir::List(_) | Mir::Map(_) | Mir::Set(_) | Mir::HeapOp { .. } => Ty::Tuple(vec![]),
        _ => Ty::Unit,
    }
}

/// A scalar (non-heap) constant — an Int/Bool/Unit literal. A const tuple is excluded (it is a heap
/// product, so an inlined body reducing to one still needs the heap construction path).
fn is_scalar_const(m: &Mir) -> bool {
    matches!(m, Mir::Int(_) | Mir::Bool(_) | Mir::Unit)
}

/// A MODULE RECORD — a `Tuple` whose every field is a function reference, a scalar constant, or a
/// nested module record. This shape is purely compile-time: it is only ever projected-and-applied and
/// never a runtime value, so inlining a call that yields it (then folding the `Proj`) makes the module
/// vanish. A data tuple with a runtime element (a `Local`, a `Call`) is NOT a module record — it must
/// stay a runtime heap product built by a real call.
fn is_module_record(m: &Mir) -> bool {
    match m {
        Mir::Tuple(fields) => fields.iter().all(|(_, e)| {
            matches!(e, Mir::FuncRef(_)) || is_scalar_const(e) || is_module_record(e)
        }),
        _ => false,
    }
}

fn is_poison(m: &Mir) -> bool {
    matches!(m, Mir::Error(_))
}

fn is_unit(m: &Mir) -> bool {
    matches!(m, Mir::Unit)
}

fn as_int(m: &Mir) -> Option<i64> {
    if let Mir::Int(n) = m {
        Some(*n)
    } else {
        None
    }
}

fn as_bool(m: &Mir) -> Option<bool> {
    if let Mir::Bool(b) = m {
        Some(*b)
    } else {
        None
    }
}

/// The first of two operands that is a poison, cloned (so a poison propagates through a strict op).
fn first_poison(a: &Mir, b: &Mir) -> Option<Mir> {
    if is_poison(a) {
        Some(a.clone())
    } else if is_poison(b) {
        Some(b.clone())
    } else {
        None
    }
}

/// A fresh poison node carrying the constant-trap diagnostic.
fn poison(message: &str) -> Mir {
    Mir::Error(Reject::coded(Code::ConstTrap, message))
}

fn cmp_int(op: CmpOp, x: i64, y: i64) -> bool {
    match op {
        CmpOp::Lt => x < y,
        CmpOp::Gt => x > y,
        CmpOp::Le => x <= y,
        CmpOp::Ge => x >= y,
        CmpOp::Eq => x == y,
    }
}

fn cmp_bool(op: CmpOp, x: bool, y: bool) -> bool {
    // Bool's total order is false < true (numeric-model.md); compare as 0/1.
    cmp_int(op, x as i64, y as i64)
}

// ─── substitution + recursion analysis ───────────────────────────────────────────────────

/// The maximum local id that appears ANYWHERE in a body — as a binder (`Let.id`, `Lambda.params`, a
/// pattern `Local`) or a use (`Local(id)`). Used to seed the α-renaming fresh-local supply above every
/// id in every body. `None` for a body that uses no locals.
fn max_local_id(mir: &Mir) -> Option<u32> {
    fn go(mir: &Mir, max: &mut Option<u32>) {
        let mut bump = |id: u32, max: &mut Option<u32>| {
            *max = Some(max.map_or(id, |m: u32| m.max(id)));
        };
        match mir {
            Mir::Local(id) => bump(*id, max),
            Mir::Let { id, value, body, .. } => {
                bump(*id, max);
                go(value, max);
                go(body, max);
            }
            Mir::Lambda { params, body } => {
                for p in params {
                    bump(*p, max);
                }
                go(body, max);
            }
            Mir::Match { scrutinee, arms, .. } => {
                go(scrutinee, max);
                for (p, b) in arms {
                    go(p, max); // a pattern's `Local` binders are counted by the `Local` arm
                    go(b, max);
                }
            }
            Mir::Sum { payload, .. } => go(payload, max),
            Mir::Call { args, .. } => args.iter().for_each(|a| go(a, max)),
            Mir::Apply { func, args } => {
                go(func, max);
                args.iter().for_each(|a| go(a, max));
            }
            Mir::Tuple(elems) | Mir::List(elems) | Mir::Set(elems) => {
                elems.iter().for_each(|(_, e)| go(e, max));
            }
            Mir::Map(entries) => entries.iter().for_each(|((_, k), (_, v))| {
                go(k, max);
                go(v, max);
            }),
            Mir::HeapOp { args, .. } => args.iter().for_each(|(_, e)| go(e, max)),
            Mir::Proj { operand, .. } => go(operand, max),
            Mir::Arith(_, a, b) | Mir::Bit(_, a, b) | Mir::Shift(_, a, b) => {
                go(a, max);
                go(b, max);
            }
            Mir::Cmp { a, b, .. } => {
                go(a, max);
                go(b, max);
            }
            Mir::If { cond, then_, else_, .. } => {
                go(cond, max);
                go(then_, max);
                go(else_, max);
            }
            Mir::Int(_) | Mir::Bool(_) | Mir::Str(_) | Mir::Unit | Mir::Error(_) | Mir::FuncRef(_)
            | Mir::Intrinsic(_) | Mir::Ctor { .. } | Mir::Wildcard | Mir::Trap(_) | Mir::TypeVal(_) | Mir::TypeCtor(_) => {}
        }
    }
    let mut max = None;
    go(mir, &mut max);
    max
}

/// Replace every `Local(id)` in `mir` with `value` (a constant). Used to β-reduce a `let` binding and
/// to bind a call's arguments to the callee's parameters. `value` is a constant, so it carries no
/// locals of its own — no capture/renaming is needed.
fn substitute(mir: Mir, id: u32, value: &Mir) -> Mir {
    match mir {
        Mir::Local(i) if i == id => value.clone(),
        Mir::Local(_) | Mir::Int(_) | Mir::Bool(_) | Mir::Str(_) | Mir::Unit | Mir::Error(_) | Mir::FuncRef(_)
        | Mir::Intrinsic(_) | Mir::Ctor { .. } | Mir::Wildcard | Mir::Trap(_) | Mir::TypeVal(_) | Mir::TypeCtor(_) => mir,
        Mir::Sum { def, disc, payload_ty, payload } => Mir::Sum {
            def,
            disc,
            payload_ty,
            payload: Box::new(substitute(*payload, id, value)),
        },
        Mir::Match { scrutinee, scrut_ty, arms, ty } => Mir::Match {
            scrutinee: Box::new(substitute(*scrutinee, id, value)),
            scrut_ty,
            // Substitute in the arm bodies; a pattern is a structural tree of ctors/tuples/literals plus
            // its OWN fresh binders (different ids), so the substituted `id` never occurs in a pattern.
            arms: arms.into_iter().map(|(p, b)| (p, substitute(b, id, value))).collect(),
            ty,
        },
        Mir::Call { func, args } => Mir::Call {
            func,
            args: args.into_iter().map(|a| substitute(a, id, value)).collect(),
        },
        Mir::Apply { func, args } => Mir::Apply {
            func: Box::new(substitute(*func, id, value)),
            args: args.into_iter().map(|a| substitute(a, id, value)).collect(),
        },
        Mir::Tuple(elems) => Mir::Tuple(
            elems.into_iter().map(|(t, e)| (t, substitute(e, id, value))).collect(),
        ),
        Mir::List(elems) => Mir::List(
            elems.into_iter().map(|(t, e)| (t, substitute(e, id, value))).collect(),
        ),
        Mir::Map(entries) => Mir::Map(
            entries
                .into_iter()
                .map(|((kt, k), (vt, v))| ((kt, substitute(k, id, value)), (vt, substitute(v, id, value))))
                .collect(),
        ),
        Mir::Set(elems) => Mir::Set(
            elems.into_iter().map(|(t, e)| (t, substitute(e, id, value))).collect(),
        ),
        Mir::HeapOp { op, args } => Mir::HeapOp {
            op,
            args: args.into_iter().map(|(t, e)| (t, substitute(e, id, value))).collect(),
        },
        Mir::Proj { slot, elem_ty, operand } => Mir::Proj {
            slot,
            elem_ty,
            operand: Box::new(substitute(*operand, id, value)),
        },
        Mir::Arith(op, a, b) => {
            Mir::Arith(op, Box::new(substitute(*a, id, value)), Box::new(substitute(*b, id, value)))
        }
        Mir::Bit(op, a, b) => {
            Mir::Bit(op, Box::new(substitute(*a, id, value)), Box::new(substitute(*b, id, value)))
        }
        Mir::Shift(op, a, b) => {
            Mir::Shift(op, Box::new(substitute(*a, id, value)), Box::new(substitute(*b, id, value)))
        }
        Mir::Cmp { op, operand_ty, a, b } => Mir::Cmp {
            op,
            operand_ty,
            a: Box::new(substitute(*a, id, value)),
            b: Box::new(substitute(*b, id, value)),
        },
        Mir::If { cond, then_, else_, ty } => Mir::If {
            cond: Box::new(substitute(*cond, id, value)),
            then_: Box::new(substitute(*then_, id, value)),
            else_: Box::new(substitute(*else_, id, value)),
            ty,
        },
        // A nested `let` may SHADOW `id` (rebind the same local index). Substitution stops at the
        // shadow's binding — but the bound VALUE is still in the outer scope, so substitute there;
        // only the body is shielded when the inner id equals `id`.
        Mir::Let { id: inner, value_ty, value: v, body } => {
            let v = Box::new(substitute(*v, id, value));
            let body = if inner == id {
                body // shadowed: the body's `id` refers to the inner binding, not ours
            } else {
                Box::new(substitute(*body, id, value))
            };
            Mir::Let { id: inner, value_ty, value: v, body }
        }
        // A lambda may SHADOW `id` (its params are binders). Substitute into the body, respecting the
        // lambda's own params as shadowing binders — like the `Let` shadow rule: do not substitute an
        // id a lambda re-binds.
        Mir::Lambda { params, body } => {
            let body = if params.contains(&id) {
                body // shadowed: the body's `id` refers to a lambda param, not ours
            } else {
                Box::new(substitute(*body, id, value))
            };
            Mir::Lambda { params, body }
        }
    }
}

/// For each module function, whether it is (transitively) recursive — part of a call cycle. A
/// recursive function is never inlined. Computed by transitive closure over the direct-call graph.
fn recursive_set(bodies: &[Mir]) -> Vec<bool> {
    let n = bodies.len();
    // Direct calls per function.
    let mut calls: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (i, b) in bodies.iter().enumerate() {
        collect_calls(b, &mut calls[i]);
    }
    // Transitive closure: reachable[i] = every function i can reach through calls. `i` is recursive
    // iff it can reach itself.
    let mut recursive = vec![false; n];
    for start in 0..n {
        let mut seen = vec![false; n];
        let mut stack = calls[start].clone();
        while let Some(f) = stack.pop() {
            if f >= n || seen[f] {
                continue;
            }
            seen[f] = true;
            if f == start {
                recursive[start] = true;
            }
            stack.extend(calls[f].iter().copied());
        }
    }
    recursive
}

/// Collect the module-function indices a body directly calls.
fn collect_calls(mir: &Mir, out: &mut Vec<usize>) {
    match mir {
        Mir::Call { func, args } => {
            out.push(*func);
            for a in args {
                collect_calls(a, out);
            }
        }
        // A function referenced as a value (and later applied) can close a cycle — count it as a call
        // to that function for recursion detection (so a recursive module export is not inlined).
        Mir::FuncRef(func) => out.push(*func),
        Mir::Apply { func, args } => {
            collect_calls(func, out);
            for a in args {
                collect_calls(a, out);
            }
        }
        Mir::Tuple(elems) => elems.iter().for_each(|(_, e)| collect_calls(e, out)),
        Mir::List(elems) => elems.iter().for_each(|(_, e)| collect_calls(e, out)),
        Mir::Map(entries) => entries.iter().for_each(|((_, k), (_, v))| {
            collect_calls(k, out);
            collect_calls(v, out);
        }),
        Mir::Set(elems) => elems.iter().for_each(|(_, e)| collect_calls(e, out)),
        Mir::HeapOp { args, .. } => args.iter().for_each(|(_, e)| collect_calls(e, out)),
        Mir::Proj { operand, .. } => collect_calls(operand, out),
        Mir::Arith(_, a, b) | Mir::Bit(_, a, b) | Mir::Shift(_, a, b) => {
            collect_calls(a, out);
            collect_calls(b, out);
        }
        Mir::Cmp { a, b, .. } => {
            collect_calls(a, out);
            collect_calls(b, out);
        }
        Mir::If { cond, then_, else_, .. } => {
            collect_calls(cond, out);
            collect_calls(then_, out);
            collect_calls(else_, out);
        }
        Mir::Let { value, body, .. } => {
            collect_calls(value, out);
            collect_calls(body, out);
        }
        // A sum's payload + a match's scrutinee and arm bodies may call.
        Mir::Sum { payload, .. } => collect_calls(payload, out),
        Mir::Match { scrutinee, arms, .. } => {
            collect_calls(scrutinee, out);
            arms.iter().for_each(|(_, b)| collect_calls(b, out));
        }
        // A lambda body can close a call cycle (a recursive closure) → descend so a lambda-closed
        // recursive call is detected (and the callee not wrongly inlined into non-termination).
        Mir::Lambda { body, .. } => collect_calls(body, out),
        // An intrinsic / ctor / wildcard / trap / str-literal / type-value / type-ctor references no module function.
        Mir::Intrinsic(_) | Mir::Ctor { .. } | Mir::Wildcard | Mir::Trap(_)
        | Mir::Int(_) | Mir::Bool(_) | Mir::Str(_) | Mir::Unit | Mir::Local(_) | Mir::Error(_) | Mir::TypeVal(_) | Mir::TypeCtor(_) => {}
    }
}
