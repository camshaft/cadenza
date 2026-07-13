//! Effect SYNTHESIS — an `(effect NAME (op f (-> A B)) …)` declaration realized as an ordinary record.
//!
//! The program-driven twin of a slice of `prelude::install`, and the effect analogue of
//! `crate::sums`: an effect declaration IS a record in scope, exactly as a sum type is. `(effect Diag
//! (op emit (-> Int64 Unit)) (op collect (-> Unit (List Int64))))` binds `Diag` to a RECORD whose
//! FIELDS ARE ITS OPERATIONS. Then `Diag.emit` is ORDINARY MEMBER ACCESS (the `Int64.max` /
//! `Option.Some` path) yielding an operation VALUE, and a performance `(Diag.emit code)` is an ORDINARY
//! APPLICATION typed against that value's `(meta t)` arrow — every one reusing the machinery already
//! built for the prelude's records and the sum synthesis, with NO name special-casing
//! (`prelude-and-resolution.md` §Nothing Is Privileged By Name).
//!
//! **What an effect record holds.** For `(effect Diag (op emit (-> Int64 Unit)) (op collect (-> Unit
//! (List Int64))))`:
//! ```text
//! Diag = (record ((meta t)      (effect Diag <decl>))            ; marks it an EFFECT; identity = decl
//!                 (emit    (record ((meta t)         (-> Int64 Unit))
//!                                   ((meta apply)     (intrinsic perform))
//!                                   ((meta effect-op) <decl>:0)))
//!                 (collect (record ((meta t)         (-> Unit (List Int64)))
//!                                   ((meta apply)     (intrinsic perform))
//!                                   ((meta effect-op) <decl>:1))))
//! ```
//! - The effect record's `(meta t)` is an EFFECT type-value (encoded `(effect NAME <decl>)`, decoded by
//!   `resolve::decode_ty`). It is not a value type — an effect is not a first-class value — but carrying
//!   it keeps the record a well-formed module-like record and lets a later pass recover the effect from
//!   a projected op (via `(meta effect-op)`).
//! - Each OPERATION is a field whose value is an operation record carrying THREE meta channels — the
//!   same shape a variant constructor / operator record has, so a performance rides the ordinary
//!   `(meta apply)` dispatch:
//!     - `(meta t)` — the operation's TYPE, the arrow `(-> Param Result)` written after the name. Read
//!       when the operation is applied, so a perform-argument mismatch is an ordinary type error
//!       (`capabilities-and-effects.md` §Performing An Operation Is Typed).
//!     - `(meta apply)` — the `(intrinsic perform)` marker. Applying an operation projects this; it is
//!       NOT a known `Prim` yet, so a perform that reaches lowering with no enclosing handler DECLINES
//!       (E0 recognizes the surface; E1 resolves a perform to its handler and rewrites it away).
//!     - `(meta effect-op)` — the operation's IDENTITY: its declaring effect's declaration occurrence
//!       and its operation index, encoded as an integer `decl.occ * K + op_index` is NOT used; instead a
//!       two-element `(effect-op <decl> <index>)` node, so a later pass recovers WHICH effect+operation a
//!       projected op value denotes without a name scan (the analogue of a variant ctor's
//!       `(meta variant)` discriminant).
//!
//! **Identity is the declaration, not the name** (as for sums): the `(meta t)` and each op's
//! `(meta effect-op)` carry the [`EffectDecl`]'s occurrence, so two effects that declare a same-named
//! operation never collide (`capabilities-and-effects.md` §An Operation Is Reached Through Its Declaring
//! Effect).
//!
//! **Deferred to E1.** These records carry the op TYPE + identity; the `(meta apply)` perform intrinsic
//! declines at lowering until E1 makes the compile-time evaluator handler-context-aware and rewrites a
//! tail-resumptive perform to plain code. Until then a program that performs an operation TYPES its
//! arguments but declines to run — the "present, but not yet realized" state a prelude `unrealized`
//! field or a construction-less sum constructor already has.

use crate::ast::{Arenas, IntValue, Leaf, Radix, Struct, StructId};
use crate::db::{Db, Def, EffectDecl, OpDecl};
use crate::fxhash::FxHashMap as HashMap;
use crate::prelude::{meta_field, push_atom, push_list};
use crate::resolve::resolved_of;
use crate::resolved::{HandleArm, Resolved};

/// Synthesize the record for every scanned `(effect …)` declaration, appending them to `ast` as
/// ordinary nodes and RECORDING each on its `EffectDecl.synth`. Called at load AFTER the scan (which
/// produced `decls`) and the sum synthesis, so the effect records take `StructId`s above the program's
/// and the parent index (built next) covers them. Deterministic: a fixed function of the declarations.
///
/// There is NO name→record map: an `(effect …)` resolves exactly like a `def`/`type` — by the ordinary
/// top-level lookup against `db.effect_decls` (an occurrence-keyed Vec), so two same-named declarations
/// each keep their own record and identity.
pub fn synthesize(ast: &mut Arenas, decls: &mut [EffectDecl]) {
    for decl in decls.iter_mut() {
        decl.synth = Some(effect_record(ast, decl));
    }
}

/// Desugar every CANONICAL handler form `(handle E seed (arm…) body)` — where the effect `E` and the
/// initial `seed` are PROMOTED into the head and each arm's operation is written BARE (`(op (p…) state
/// body)`) — into the INTERNAL form the rest of the compiler consumes: `(handle seed (arm'…) body)`
/// with `E` dropped from the head and each arm's op rewritten to its `(. E op)` projection. This lets
/// the surface (both s-expr and ML) name the effect once, on the `handle`, while `resolve_handle` /
/// `effects` / infer / lower / compile keep reading the projection-per-arm shape unchanged.
///
/// Runs at load BEFORE the parent index is built, so the rewritten `(. E op)` projections resolve like
/// hand-written member access. Mutates the arena IN PLACE (swapping a handle node's children vector),
/// mirroring `accum::introduce` / `binding_params::lower`.
///
/// The canonical shape also carries the language RULE that a `handle` discharges exactly ONE effect:
/// its head names that effect, and every arm is one of that effect's operations. That is checked
/// downstream — an arm op `(. E op)` that `E` does not declare is the ordinary undeclared-operation
/// rejection (CDZ0403) — so this pass only performs the mechanical rewrite. A form that is NOT the
/// canonical shape (already-internal `(handle seed (arm…) body)` with 4 children, or a malformed
/// handle) is left untouched, so a hand-authored internal-shape program still compiles.
pub fn desugar_handles(ast: &mut Arenas) {
    // Collect the rewrites first (an immutable scan), then apply — appends during apply must not
    // perturb the ids we scanned.
    let mut plans: Vec<HandlePlan> = Vec::new();
    for id in (0..ast.structure.len() as u32).map(StructId) {
        if let Some(plan) = plan_canonical_handle(ast, id) {
            plans.push(plan);
        }
    }
    for plan in plans {
        apply_handle_plan(ast, plan);
    }
}

/// A recognized canonical `(handle E seed (arm…) body)` and the in-place rewrite it needs. Applying it
/// (a) rewrites each arm's bare op `op` to the projection `(. E op)` IN PLACE (the arm keeps its own
/// `StructId`, so its source span survives) and (b) drops the effect child from the handle head (the
/// handle node keeps its `StructId` too). Preserving those ids matters: the CDZ0405 "add the missing
/// arm" fix anchors an insert on the ARMS-LIST node's source span, which only exists if the desugar did
/// not synthesize a fresh spanless arms list (`spec/capabilities/diagnostics.md` §A Rejection Carries A
/// Structural Fix).
struct HandlePlan {
    /// The `(handle …)` form occurrence — its children shrink from five to four (effect dropped).
    handle: StructId,
    /// The effect NAME each arm's op is projected on.
    effect_name: String,
    /// Each arm occurrence and its current bare-op child — the op is swapped for a `(. E op)` projection
    /// built at apply time, leaving the arm's params/state/body (and the arm's own id/span) untouched.
    arms: Vec<StructId>,
}

/// If `id` is a CANONICAL `(handle E seed (arm…) body)` — five children, the second a bare effect NAME,
/// every arm a 4-part `(bare-op (params…) state body)` — return the [`HandlePlan`] to rewrite it in
/// place. `None` for any other shape (already-internal 4-child handle, an arm already projected, or a
/// malformed handle), which is left untouched so a hand-authored internal-shape program still compiles.
fn plan_canonical_handle(ast: &Arenas, id: StructId) -> Option<HandlePlan> {
    let Struct::List(items) = ast.get(id) else {
        return None;
    };
    // Canonical head: `handle` with FIVE children (head, effect, seed, arms, body). The internal form
    // has four (head, seed, arms, body), so the arity distinguishes them with no ambiguity.
    if items.len() != 5 || ast.as_name(items[0]) != Some("handle") {
        return None;
    }
    // The head's second child MUST be a bare effect NAME for this to be the canonical (promoted) shape.
    let effect_name = ast.as_name(items[1])?.to_string();
    let arms_occ = items[3];
    let Struct::List(arm_nodes) = ast.get(arms_occ) else {
        return None;
    };
    // Confirm EVERY arm is `(bare-op (params…) state body)` — a 4-part list whose op is a bare NAME (not
    // an already-projected `(. E op)`). If any arm is not this shape, this is not the canonical form.
    for &arm in arm_nodes {
        let Struct::List(parts) = ast.get(arm) else {
            return None;
        };
        if parts.len() != 4 || ast.as_name(parts[0]).is_none() {
            return None;
        }
    }
    Some(HandlePlan {
        handle: id,
        effect_name,
        arms: arm_nodes.clone(),
    })
}

/// Apply a [`HandlePlan`]: swap each arm's bare op for a `(. E op)` projection IN PLACE and drop the
/// effect from the handle head. Both the arm nodes and the handle node keep their `StructId`s (and thus
/// their source spans); only the projection nodes are freshly appended.
fn apply_handle_plan(ast: &mut Arenas, plan: HandlePlan) {
    for arm in plan.arms {
        // The arm's current bare op (its first child) — replace it with `(. E op)`. `E` is a FRESH name
        // occurrence per arm so the parent walk anchors each projection independently; `op` is REUSED
        // (its own occurrence carries the arm's op-name span).
        let Struct::List(parts) = ast.get(arm) else {
            continue;
        };
        let op = parts[0];
        let rest: Vec<StructId> = parts[1..].to_vec();
        let dot = push_atom(ast, Leaf::Name(".".to_string()));
        let eff = push_atom(ast, Leaf::Name(plan.effect_name.clone()));
        let proj = push_list(ast, vec![dot, eff, op]);
        let mut new_children = vec![proj];
        new_children.extend(rest);
        ast.structure[arm.0 as usize] = Struct::List(new_children);
    }
    // Drop the effect (index 1) from the handle head: [handle, effect, seed, arms, body] -> [handle,
    // seed, arms, body]. The handle keeps its id/span.
    if let Struct::List(items) = ast.get(plan.handle) {
        let mut kept = items.clone();
        if kept.len() == 5 {
            kept.remove(1);
            ast.structure[plan.handle.0 as usize] = Struct::List(kept);
        }
    }
}

/// The occurrence of the operation field named `op_name` inside a synthesized effect `record` — so a
/// consumer can find an operation value by name. The record is `(record ((meta t) …) (emit <op>)
/// (collect <op>)…)`; an operation field is a 2-element `(name <op>)` list whose name matches. `None`
/// if not found (e.g. a meta field). The effect analogue of `sums::variant_ctor_field`.
pub fn op_field(ast: &Arenas, record: StructId, op_name: &str) -> Option<StructId> {
    let Struct::List(children) = ast.get(record) else {
        return None;
    };
    for &field in children.iter().skip(1) {
        if let Struct::List(pair) = ast.get(field)
            && pair.len() == 2
            && ast.as_name(pair[0]) == Some(op_name)
        {
            return Some(pair[1]);
        }
    }
    None
}

/// Build one effect's record: `(record ((meta t) <effect-typeval>) (<op> <op-value>)…)`. The `(meta t)`
/// marks it an effect (identity = the declaration occurrence); each operation is a field to its
/// operation-value record.
fn effect_record(ast: &mut Arenas, decl: &EffectDecl) -> StructId {
    // The record PRIMITIVE head is the STRING `"record"` (the NAME `record` is a shadowable alias).
    let head = push_atom(ast, Leaf::Str("record".to_string()));
    let mut children = vec![head];

    // `(meta t)` — the effect type-value, so a later pass can recover the effect's identity.
    let eff_ty = effect_typeval(ast, decl);
    children.push(meta_field(ast, "t", eff_ty));

    // One field per operation, its value the operation-value record. The operation's INDEX in
    // declaration order is its stable operation index.
    for (index, op) in decl.ops.iter().enumerate() {
        let value = op_value(ast, decl, op, index as u32);
        let k = push_atom(ast, Leaf::Name(op.name.clone()));
        children.push(push_list(ast, vec![k, value]));
    }
    push_list(ast, children)
}

/// An operation-value record — the SAME three-meta-channel shape a variant constructor has:
///  - `(meta t)` — the operation's TYPE, the user-written arrow `(-> Param Result)` (copied fresh so it
///    re-resolves in the synthesized scope). Read by `apply_type` when the operation is applied, so a
///    perform-argument mismatch is an ordinary type error.
///  - `(meta apply)` — the `(intrinsic perform)` marker. Applying the operation projects this; it is not
///    a known `Prim`, so a perform reaching lowering with no enclosing handler declines (E1 rewrites it).
///  - `(meta effect-op)` — the operation's IDENTITY `(effect-op <decl> <index>)`, so a later pass
///    recovers which effect+operation a projected op value denotes without a name scan.
fn op_value(ast: &mut Arenas, decl: &EffectDecl, op: &OpDecl, index: u32) -> StructId {
    let head = push_atom(ast, Leaf::Str("record".to_string()));
    // `(meta t)` — the operation's arrow type, wrapped in a ZERO-PARAM `(fn () (-> Param Result))`. The
    // wrapper is LOAD-BEARING (the same lesson the monomorphic String/Bytes ops learned): a BARE arrow
    // as `(meta t)` makes `typeval_of` collapse the whole op-value RECORD to `Ty::Type` (an arrow IS a
    // type-value), so `(. E op)` would type as `Type` and `(+ (E.op) 1)` faults "unify Int64 with Type".
    // The `(fn () …)` wrapper makes `scheme_of` read a monomorphic SCHEME (no quantified variables)
    // rather than a type-value, so the op has a function type and a performance types as an application.
    // A malformed `(op NAME)` with no type gets `Unit`.
    let arrow = match op.ty {
        Some(t) => copy_subtree(ast, t),
        None => push_atom(ast, Leaf::Name("Unit".to_string())),
    };
    let ty = {
        let fn_head = push_atom(ast, Leaf::Name("fn".to_string()));
        let params = push_list(ast, vec![]);
        push_list(ast, vec![fn_head, params, arrow])
    };
    let t_field = meta_field(ast, "t", ty);
    // `(meta apply)` = the perform marker (declines at lowering until E1).
    let apply = {
        let ih = push_atom(ast, Leaf::Name("intrinsic".to_string()));
        let who = push_atom(ast, Leaf::Name("perform".to_string()));
        push_list(ast, vec![ih, who])
    };
    let apply_field = meta_field(ast, "apply", apply);
    // `(meta effect-op)` = `(effect-op <decl> <index>)` — the operation's identity.
    let identity = {
        let eo = push_atom(ast, Leaf::Name("effect-op".to_string()));
        let d = int_atom(ast, decl.occ.0 as i64);
        let i = int_atom(ast, index as i64);
        push_list(ast, vec![eo, d, i])
    };
    let identity_field = meta_field(ast, "effect-op", identity);
    push_list(ast, vec![head, t_field, apply_field, identity_field])
}

/// The effect's type-value as an arena node: `(effect NAME <decl>)`. The dual of `resolve::decode_ty`'s
/// effect arm — the declaration occurrence is the identity (an integer literal in the wire form), the
/// name for rendering. (Not `(typeval …)`: an effect is not a value type; this node marks the record as
/// an effect and carries its identity.)
fn effect_typeval(ast: &mut Arenas, decl: &EffectDecl) -> StructId {
    let eff_head = push_atom(ast, Leaf::Name("effect".to_string()));
    let nm = push_atom(ast, Leaf::Name(decl.name.clone()));
    let d = int_atom(ast, decl.occ.0 as i64);
    push_list(ast, vec![eff_head, nm, d])
}

/// A decimal integer-literal atom for `value` — the encoding used for a declaration occurrence / an
/// operation index in the synthesized meta channels (mirrors `sums`).
fn int_atom(ast: &mut Arenas, value: i64) -> StructId {
    push_atom(
        ast,
        Leaf::Int {
            value: IntValue::from_i64(value),
            radix: Radix::Dec,
        },
    )
}

/// Structurally COPY the subtree rooted at `node` (a NAME atom copied fresh so it re-resolves against
/// the copy's scope; a constant atom shared; a list copied with its children copied). Lets a synthesized
/// operation type reference a user-written type without the shared occurrence acquiring a second parent.
/// Identical to `sums::copy_subtree`.
fn copy_subtree(ast: &mut Arenas, node: StructId) -> StructId {
    match ast.get(node).clone() {
        Struct::Atom(lid) => match ast.leaf(lid).clone() {
            Leaf::Name(_) => {
                let leaf = ast.leaf(lid).clone();
                push_atom(ast, leaf)
            }
            _ => node,
        },
        Struct::List(children) => {
            let copied: Vec<StructId> = children.iter().map(|&c| copy_subtree(ast, c)).collect();
            push_list(ast, copied)
        }
    }
}

// ============================================================================================
// E1c — TAIL-RESUMPTIVE HANDLER FOLD
// ============================================================================================
//
// A `(handle INIT (ARM…) BODY)` is reduced AWAY at lowering (`DESIGN-effects-rcdzc.md` §0/§4.1): each
// performance of a discharged operation in BODY is resolved to its concrete ARM (a compile-time
// constant — no runtime handler search), and a TAIL-RESUMPTIVE arm is rewritten to plain code — the
// perform becomes the arm's resume VALUE, and the arm's next-STATE threads forward to the rest of the
// handled region. `select` then sees only ordinary `Core`.
//
// This is realized as a SOURCE-TO-SOURCE rewrite in the arena: `reduce_handle` produces a new BODY
// occurrence with every perform replaced, which `lower` then lowers by the ordinary path. State is
// threaded by an EVALUATION-ORDER walk carrying the "current state expression" — a perform reads the
// current state (binds it to the arm's `state` binder) and updates it to the arm's next-state.
//
// CONSERVATIVE: the walk handles exactly the structural forms the tail-resumptive shipping surface uses
// (a perform, a `do` sequence, an application/arith over sub-expressions, an `if`), threading state in
// left-to-right evaluation order. Anything it cannot prove tail-resumptive — a non-tail `resume`, an
// arm with no `resume` (abortive), a `resume` shape it does not recognize, a cross-function perform (a
// perform reached only through a call) — makes it return `None`, and `lower` DECLINES (a Todo, never a
// miscompile). E1c's job is the self-contained tail surface; cross-function (inline trigger) is E1c-2,
// recursion is E3.

/// The handler context for one active `handle`: the discharged operations (by identity) mapped to
/// their arms, plus the STATE binders in scope. Threaded by value through the rewrite (not mutated),
/// so a nested handler / re-entry does not clobber an outer one.
struct HandlerCtx {
    /// Each discharged operation's `(decl-occ, op-index)` → the arm that discharges it. For a MERGED
    /// context (nested handlers whose shared recursive callee performs several effects), this holds the
    /// union of every enclosing handler's arms.
    arms: HashMap<(u32, u32), HandleArm>,
    /// A stable identity STRING for this handler context — the discharged ops + their arm occurrences,
    /// in sorted order — used as the specialization memo key (`db.effect_specializations`). A RESOLVED
    /// identity (occurrences), NOT `format!("{:?}", body)` — the old compiler's stringly-typed-syntax
    /// footgun (`DESIGN-effects-rcdzc.md` §4.3). Empty until built by `HandlerCtx::new`.
    key: String,
    /// One STATE SLOT per enclosing handler, OUTERMOST first — the threaded states the fold carries as a
    /// vector (`DESIGN-effects-rcdzc.md` §4.3: "two nested handlers → two trailing params"). A
    /// single-handler context has one slot; a MERGED nested context has one per handler. Each slot records
    /// which effect DECL it threads and that state's TYPE (to annotate a specialized fn's trailing param).
    /// A specialized recursive fn takes one trailing state parameter per slot, in this order.
    slots: Vec<StateSlot>,
    /// The ABORTIVE arms of this context (E4) — the `(decl, idx)` of every arm that NEVER resumes (its
    /// body has no `resume`). Performing such an operation ABANDONS the surrounding computation and makes
    /// the handle yield the arm body's value (`DESIGN-effects-rcdzc.md` §4.2). Empty for a purely
    /// tail-resumptive context.
    abortive: std::collections::HashSet<(u32, u32)>,
    /// The ABORT value captured during threading (interior-mutable so the immutable-`&ctx` thread walk can
    /// record it): when an abortive perform in a STRICT position fires, its arm value is set here and
    /// threading short-circuits — `reduce_handle` returns this value as the WHOLE handle body (the
    /// surrounding computation is dead). `None` until an abortive perform fires. This increment realizes
    /// the UNCONDITIONAL strict abort (the perform is reached before its enclosing op completes); a
    /// conditional abort (inside an `if`/`match` branch) is a later increment.
    abort_value: std::cell::Cell<Option<StructId>>,
}

/// One handler's state in a (possibly merged) [`HandlerCtx`]: the effect declaration whose operations
/// thread it, and the state's TYPE (from the handle's `init` seed; `None` if undetermined → a recursive
/// specialization declines). The slot's INDEX is its position in `HandlerCtx::slots` — the trailing
/// state-parameter position a specialized fn threads it through.
struct StateSlot {
    /// The effect declaration occurrence whose operations thread this slot's state.
    decl: u32,
    /// The state's type — annotates the specialized fn's trailing param. `None` if undetermined.
    state_ty: Option<crate::ty::Ty>,
}

impl HandlerCtx {
    /// Build a handler context from its operation→arm map and its state slots (one per enclosing handler,
    /// outermost first). The key is the discharged ops (`decl:idx`) plus each arm's occurrence, sorted — a
    /// stable RESOLVED identity. A single-handler context has one slot; a merged nested context (built by
    /// `merged_nested_ctx`) has one per handler.
    fn new(db: &mut Db, arms: HashMap<(u32, u32), HandleArm>, slots: Vec<StateSlot>) -> HandlerCtx {
        let mut parts: Vec<String> = arms
            .iter()
            .map(|((d, i), arm)| format!("{d}:{i}@{}", arm.op.0))
            .collect();
        parts.sort();
        let key = parts.join(",");
        // An arm is ABORTIVE when its body has NO `resume` (neither a bare `(resume …)` nor a
        // `(do … (resume …))`): performing it abandons the computation, yielding the arm body's value.
        let abortive = arms
            .iter()
            .filter(|(_, arm)| !arm_has_resume(db, arm.body))
            .map(|(&k, _)| k)
            .collect();
        HandlerCtx {
            arms,
            key,
            slots,
            abortive,
            abort_value: std::cell::Cell::new(None),
        }
    }

    /// The slot INDEX that effect declaration `decl` threads its state through — the trailing
    /// state-parameter position. `None` if this context does not thread that effect.
    fn slot_of(&self, decl: u32) -> Option<usize> {
        self.slots.iter().position(|s| s.decl == decl)
    }

    /// Whether this context threads at least one state (the presence gate the recursive-specialization arm
    /// checks — the successor of the old `single_state.is_some()`).
    fn has_state(&self) -> bool {
        !self.slots.is_empty()
    }
}

/// If an INNER handle with arms `inner_arms`, nested inside the OUTER context `outer`, must be MERGED with
/// the outer context to fold — build the merged context (`Some`), else `None` (use the inside-out path).
///
/// The merge is needed exactly when the inner handle's body reaches a RECURSIVE callee that performs BOTH
/// the inner effect AND an outer effect: neither handler alone can specialize such a callee (specializing
/// on the inner effect only would leave the outer performs unresolved inside the specialized body). The
/// merged context is the UNION of the outer arms and the inner arms, with the inner handler's slot
/// APPENDED after the outer slots (so the recursive fn threads `s_outer…, s_inner`, outermost-first). The
/// inner slot's state type is derived from the inner handle's own arms via `state_ty_of_arms` — but the
/// caller supplies `inner_init` when threading (the type is only for the specialized param annotation).
///
/// `None` (fall back to inside-out) when: the inner arms span more than one effect (a single `handle`
/// discharges one effect, so this should not happen — defensive), the inner effect is already an outer
/// slot (re-entrant same-effect nesting — the inside-out shadow path handles it), or the inner body does
/// not reach a recursive callee performing an outer effect (an ordinary nested handler — inside-out).
fn merged_nested_ctx(
    db: &mut Db,
    inner_arms: &[HandleArm],
    inner_body: StructId,
    outer: &HandlerCtx,
) -> Option<HandlerCtx> {
    // The single effect the inner handle discharges (all its arms share a decl).
    let inner_decl = inner_arms
        .first()
        .and_then(|a| crate::eval::effect_op_of(db, a.op))
        .map(|(d, _)| d.0)?;
    // Re-entrant same-effect nesting (the inner effect is already an outer slot) is the inside-out shadow
    // case — not a merge.
    if outer.slot_of(inner_decl).is_some() {
        return None;
    }
    // Build the candidate merged arm map (outer arms ∪ inner arms) and slots (outer slots ++ inner slot).
    let mut arms = outer.arms.clone();
    for arm in inner_arms {
        let (decl, idx) = crate::eval::effect_op_of(db, arm.op)?;
        // A malformed resume-vs-result type declines, as in `reduce_handle`.
        if !resume_result_type_ok(db, arm) {
            return None;
        }
        arms.insert((decl.0, idx), arm.clone());
    }
    let mut slots: Vec<StateSlot> = outer
        .slots
        .iter()
        .map(|s| StateSlot {
            decl: s.decl,
            state_ty: s.state_ty.clone(),
        })
        .collect();
    // The inner slot's state type — from the inner handle's arms' resume next-states.
    let inner_state_ty = inner_state_ty_from_arms(db, inner_arms);
    slots.push(StateSlot {
        decl: inner_decl,
        state_ty: inner_state_ty,
    });
    let merged = HandlerCtx::new(db, arms, slots);
    // Only MERGE if the inner body reaches a RECURSIVE callee that (under the merged context) performs an
    // OUTER effect too — the two-nested-states signature. When it does, the inside-out path can't fold it
    // (specializing on the inner effect alone leaves the outer performs unresolved); merging lets the
    // callee specialize ONCE against both. When it doesn't (an ordinary non-recursive nested handler like
    // `(+ (A.a) (B.b))`), the inside-out path is correct and cheaper — return `None`.
    if inner_body_needs_merge(db, inner_body, inner_decl, &merged) {
        Some(merged)
    } else {
        None
    }
}

/// Whether the inner handle's body needs the MERGE (vs the inside-out path): it reaches a RECURSIVE callee
/// (specializable under the merged context) whose body performs an effect the inner handler does NOT
/// discharge — an outer effect. That is exactly the shape the inside-out path cannot fold.
fn inner_body_needs_merge(
    db: &mut Db,
    node: StructId,
    inner_decl: u32,
    merged: &HandlerCtx,
) -> bool {
    // A syntactic call to a recursive callee the merged context discharges: does that callee's body reach
    // an OUTER (non-inner) discharged effect?
    if let Resolved::Apply { head, .. } = resolved_of(db, node)
        && recursive_call_reaches_discharged(db, &head, merged)
        && let Some(callee_def) = callee_def_index_of(db, head)
        && let Some(body) = db.defs[callee_def].body
        && callee_reaches_outer_effect(db, body, inner_decl, merged, 0)
    {
        return true;
    }
    // Otherwise descend structurally (the recursive callee may be nested in an `if`/`do`/etc.).
    match db.ast.get(node).clone() {
        Struct::List(children) => children
            .iter()
            .any(|&c| inner_body_needs_merge(db, c, inner_decl, merged)),
        Struct::Atom(_) => false,
    }
}

/// Whether `node` (a recursive callee's body) performs an effect the merged context discharges OTHER than
/// the inner effect `inner_decl` — i.e. an outer effect. Bounded structural walk following non-recursive
/// calls (mirrors `body_reaches_discharged`).
fn callee_reaches_outer_effect(
    db: &mut Db,
    node: StructId,
    inner_decl: u32,
    merged: &HandlerCtx,
    depth: u32,
) -> bool {
    if depth > 16 {
        return false;
    }
    if let Resolved::Apply { head, .. } = resolved_of(db, node)
        && let Some((decl, idx)) = is_perform(db, head, merged)
        && decl != inner_decl
    {
        let _ = idx;
        return true;
    }
    // Follow a non-recursive call into its body.
    if let Resolved::Apply { head, .. } = resolved_of(db, node)
        && let Some(callee) = crate::eval::lambda_body(db, head)
            .or_else(|| crate::eval::lambda_body_of_nullary(db, head))
        && !crate::eval::is_recursive(db, callee)
        && callee_reaches_outer_effect(db, callee, inner_decl, merged, depth + 1)
    {
        return true;
    }
    match db.ast.get(node).clone() {
        Struct::List(children) => children
            .iter()
            .any(|&c| callee_reaches_outer_effect(db, c, inner_decl, merged, depth)),
        Struct::Atom(_) => false,
    }
}

/// The inner handler's state type from its arms alone (the join of its tail arms' next-state types). Used
/// to annotate the merged context's inner slot. `None` if undetermined (blocks a recursive specialization
/// on that slot, a clean decline).
fn inner_state_ty_from_arms(db: &mut Db, arms: &[HandleArm]) -> Option<crate::ty::Ty> {
    let mut t: Option<crate::ty::Ty> = None;
    for arm in arms {
        if let Some(next) = tail_resume_next_state_of(db, arm.body) {
            let nt = crate::infer::type_of(db, next);
            t = Some(match t {
                Some(prev) => prev.join(&nt),
                None => nt,
            });
        }
    }
    match t {
        Some(ty) if !matches!(ty, crate::ty::Ty::Any) => Some(ty),
        _ => None,
    }
}

/// Reduce a `(handle init arms body)` to a rewritten BODY occurrence with every perform of a discharged
/// operation resolved to its arm and rewritten tail-resumptively, or `None` to DECLINE (the case is not
/// in the tail-resumptive shipping surface — `lower` then declines cleanly, a Todo). `init`/`arms`/`body`
/// Whether a handler arm's op occurrence `op` names an operation its effect does NOT declare — a
/// closed-set violation (CDZ0403). True when `op` is a member access `(. E k)` where `E` resolves to an
/// EFFECT record (its `(meta t)` is an `(effect …)` type-value) but `k` is not one of that effect's
/// declared operation names. A valid op (a declared operation) is `false`; an `op` that is not a
/// member-access on an effect at all (a malformed arm) is also `false` (its own fault surfaces).
pub fn arm_op_names_undeclared_operation(db: &mut Db, op: StructId) -> bool {
    // `op` must be a member access `(. operand key)`.
    let Resolved::Member { operand, key } = resolved_of(db, op) else {
        return false;
    };
    // The operand must resolve to an EFFECT record — recover its declaration occurrence from `(meta t)`
    // = `(effect NAME <decl>)`. A non-effect operand (an ordinary record/module) is not this check's
    // concern (a bad field there is the ordinary CDZ0201).
    let Some(decl) = effect_decl_of_value(db, operand) else {
        return false;
    };
    // Is `key` a declared operation of that effect? Look it up in the effect's declaration.
    match db.effect_decl_by_occ(crate::ast::StructId(decl)) {
        Some(eff) => !eff.ops.iter().any(|o| o.name == key.name),
        None => false, // no such effect declaration (should not happen for a resolved effect record)
    }
}

/// For an undeclared handler-arm op `(. E k)` (one `arm_op_names_undeclared_operation` flagged), the
/// nearest DECLARED operation name of the effect `E` to the mistyped `k` — the "did you mean?"
/// suggestion (`spec/capabilities/diagnostics.md` §A Diagnostic Carries A Route To A Fix), the effect-op
/// analogue of the absent-field suggestion. Draws candidates from the effect's own declared op set (via
/// the shared `diag::suggest::nearest`), so the suggestion is always a real operation of that effect.
/// Returns `(key-occurrence, nearest-op-name)` — the occurrence is the node a replace fix rewrites.
/// `None` if `op` is not `(. E k)` on an effect, or no declared op is close enough to `k`.
pub fn nearest_declared_op(db: &mut Db, op: StructId) -> Option<(StructId, String)> {
    let Resolved::Member { operand, key } = resolved_of(db, op) else {
        return None;
    };
    let decl = effect_decl_of_value(db, operand)?;
    let names: Vec<String> = db
        .effect_decl_by_occ(crate::ast::StructId(decl))?
        .ops
        .iter()
        .map(|o| o.name.clone())
        .collect();
    let candidate = crate::diag::suggest::nearest(&key.name, &names)?;
    // The key occurrence is the second child of the `(. operand key)` form — the node the fix rewrites.
    let key_occ = db.ast.as_form(op, ".").and_then(|t| t.get(1).copied())?;
    Some((key_occ, candidate))
}

/// The operations an exhaustive handler for the effect discharged by `arms` MUST bind but does NOT —
/// the effect's declared operation names minus the arms' operation names, in declaration order. Empty
/// when the handler is exhaustive (binds every operation) OR when the effect can't be determined (a
/// malformed handle whose own fault surfaces elsewhere). This realizes CDZ0405: a `handle E` names ONE
/// effect and, since an effect's operations are a closed set, must discharge the WHOLE set — the effect
/// analogue of match exhaustiveness.
///
/// The effect is read from the FIRST well-formed arm (every arm of a well-formed handle names the same
/// effect — a cross-effect arm is CDZ0403, checked independently). A duplicate operation among the arms
/// still counts once; a repeated op does not make the handler more exhaustive.
pub fn handler_missing_operations(db: &mut Db, arms: &[HandleArm]) -> Vec<MissingOp> {
    // The effect discharged: the declaring effect of the first arm whose op resolves to an effect op.
    let decl = arms
        .iter()
        .find_map(|a| crate::eval::effect_op_of(db, a.op).map(|(d, _)| d));
    let Some(decl) = decl else {
        return Vec::new();
    };
    // The operation names the arms bind (an arm op is `(. E k)` → its key name).
    let bound: std::collections::HashSet<String> = arms
        .iter()
        .filter_map(|a| match resolved_of(db, a.op) {
            Resolved::Member { key, .. } => Some(key.name.clone()),
            _ => None,
        })
        .collect();
    // The effect's full operation set, minus what the arms bind, in declaration order. Each carries its
    // arm ARITY (how many parameter binders its arm takes, elided-unit excluded) so a fix can render a
    // correctly-shaped template arm.
    let Some(eff) = db.effect_decl_by_occ(decl) else {
        return Vec::new();
    };
    let pending: Vec<(String, Option<StructId>)> = eff
        .ops
        .iter()
        .filter(|o| !bound.contains(&o.name))
        .map(|o| (o.name.clone(), o.ty))
        .collect();
    pending
        .into_iter()
        .map(|(name, ty)| MissingOp {
            arity: ty.map(|t| op_arm_arity(db, t)).unwrap_or(0),
            name,
        })
        .collect()
}

/// An operation an exhaustive handler must bind but does not — its name and the arm ARITY (parameter
/// count) a well-formed arm for it takes, so a fix can render `(op (p0 …) s (resume unit s))` with the
/// right number of parameter binders.
pub struct MissingOp {
    pub name: String,
    pub arity: usize,
}

/// The number of PARAMETER binders a handler arm for an operation of declared type `ty` takes — the arm
/// ARITY. `ty` is the operation's arrow `(-> P… R)`. A `(-> Unit R)` or a nullary-elided `(-> R)` takes
/// ZERO parameters (the unit is elided, matching a nullary perform `(E.op)` and the corpus arm
/// `(op () s …)`); otherwise the arity is the number of parameter positions before the result.
fn op_arm_arity(db: &Db, ty: StructId) -> usize {
    let Some(tail) = db.ast.as_form(ty, "->") else {
        return 0;
    };
    // The arrow is `(-> P0 … Pn R)` (flat): the last child is the result, the rest are parameters.
    let params = if tail.len() <= 1 {
        &[][..]
    } else {
        &tail[..tail.len() - 1]
    };
    // A single `Unit` parameter is the elided-unit nullary convention → zero arm binders.
    if params.len() == 1 && db.ast.as_name(params[0]) == Some("Unit") {
        return 0;
    }
    params.len()
}

/// The effect-declaration occurrence the value at `id` denotes, if it resolves to an EFFECT record — its
/// `(meta t)` is an `(effect NAME <decl>)` node. `None` for any value that is not an effect record.
fn effect_decl_of_value(db: &mut Db, id: StructId) -> Option<u32> {
    let field = crate::eval::project_meta(db, id, "t")?;
    let tail = db.ast.as_form(field, "effect")?;
    let decl_occ = tail.get(1).copied()?;
    match resolved_of(db, decl_occ) {
        Resolved::Int(v) => v.to_i64().and_then(|n| u32::try_from(n).ok()),
        _ => None,
    }
}

/// If the perform at `perform` (an application whose head is an effect operation) is DELEGATED to the host
/// by an enclosing `(host (E…) …)`, its host-call target: the declaring effect's NAME, the operation's
/// NAME, and its declared RESULT type. `None` if the perform is not enclosed by a `host` delegating its
/// effect (then it is handled in-program, or unhandled). Walks PARENTS from the perform to the nearest
/// enclosing `host` whose effect list names the op's declaring effect — the routing decision is the
/// nearest enclosing router (`capabilities-and-effects.md` §Host-Binding Is A Routing Decision Made At The
/// Entrypoint), and a nearer `handle` for the SAME effect would have reduced the perform away before
/// lowering, so reaching here means no such handler intervenes.
pub fn perform_host_target(
    db: &mut Db,
    perform: StructId,
    head: StructId,
) -> Option<(String, String, crate::ty::Ty)> {
    // The op's declaring effect + its name — the op head is a member access `(. E op)`.
    let (decl, _idx) = crate::eval::effect_op_of(db, head)?;
    let op_name = match resolved_of(db, head) {
        Resolved::Member { key, .. } => key.name.clone(),
        _ => return None,
    };
    let eff = db.effect_decl_by_occ(decl)?;
    let effect_name = eff.name.clone();
    // Walk PARENTS to find an enclosing `(host (E…) body)` whose effect list names this effect.
    let mut cur = perform;
    while let Some(parent) = db.parent_of(cur) {
        if let Resolved::Host { effects, .. } = resolved_of(db, parent)
            && effects
                .iter()
                .any(|&e| effect_decl_of_host_name(db, e) == Some(decl))
        {
            // The op's declared RESULT type — peel its `(meta t)` scheme's arrow to the final result.
            let result = op_result_type(db, head)?;
            return Some((effect_name, op_name, result));
        }
        cur = parent;
    }
    // FALLBACK — the perform has no `host` ANCESTOR in the parent chain. This happens when the perform is
    // in a SYNTHESIZED node (a `reduce_handle` rewrite of an intra-program handler that wraps a
    // host-delegated call — the rewritten body is fresh, re-parented under no `host`). The routing is
    // still sound: `check_no_home` (CDZ0401) guarantees every perform reaching emit HAS a home, and a
    // handler for THIS effect would have reduced the perform away — so a residual perform of an effect
    // some ENTRYPOINT delegates is genuinely host-bound. Consult the program-wide delegation set (the
    // union of the entrypoints' `host` clauses — "the manifest is the union of its delegations").
    if program_delegates_effect(db, decl) {
        let result = op_result_type(db, head)?;
        return Some((effect_name, op_name, result));
    }
    None
}

/// Whether ANY entrypoint (`export`) body delegates the effect `decl` to the host — a `(host (E…) …)`
/// whose effect list names it, anywhere in a reachable export body. The program-wide delegation set (the
/// manifest is the union of the entrypoints' delegations); used as the fallback when a perform's `host`
/// ancestor was erased by a `reduce_handle` node synthesis. Memoized-free but cheap (a handful of
/// exports, walked once per residual host perform).
fn program_delegates_effect(db: &mut Db, decl: crate::ast::StructId) -> bool {
    let export_bodies: Vec<StructId> = db
        .exports
        .iter()
        .filter_map(|e| e.def.and_then(|d| db.defs[d].body))
        .collect();
    export_bodies
        .into_iter()
        .any(|b| body_has_host_delegating(db, b, decl, 0))
}

/// Whether the subtree at `node` contains a `(host (E…) …)` delegating the effect `decl`. A structural
/// walk (bounded); a `host` node's effect list is checked, then the walk descends every child.
fn body_has_host_delegating(
    db: &mut Db,
    node: StructId,
    decl: crate::ast::StructId,
    depth: u32,
) -> bool {
    if depth > 128 {
        return false;
    }
    if let Resolved::Host { effects, .. } = resolved_of(db, node)
        && effects
            .iter()
            .any(|&e| effect_decl_of_host_name(db, e) == Some(decl))
    {
        return true;
    }
    match db.ast.get(node).clone() {
        Struct::List(children) => children
            .iter()
            .any(|&c| body_has_host_delegating(db, c, decl, depth + 1)),
        Struct::Atom(_) => false,
    }
}

/// The effect-declaration occurrence a `host`'s effect-list ENTRY names — a bare effect name occurrence
/// `E` that resolves to an effect record; recover its decl from the record's `(meta t)`. `None` if it does
/// not resolve to an effect.
fn effect_decl_of_host_name(db: &mut Db, name_occ: StructId) -> Option<crate::ast::StructId> {
    // The name resolves to a Ref to the effect record's value; read the record's effect decl.
    let value = match resolved_of(db, name_occ) {
        Resolved::Ref { value } => value,
        _ => name_occ,
    };
    effect_decl_of_value(db, value).map(crate::ast::StructId)
}

/// The declared RESULT type of the operation whose op-value projection is `head` — instantiate its
/// `(meta t)` scheme `(fn () (-> P… Result))` and peel to the final arrow result. `None` if malformed.
fn op_result_type(db: &mut Db, head: StructId) -> Option<crate::ty::Ty> {
    let mut fresh = crate::unify::Fresh::new();
    let scheme = crate::eval::scheme_of(db, head, &mut fresh)?;
    let mut result = crate::unify::instantiate(&scheme, &mut fresh);
    let mut peeled = false;
    while let crate::ty::Ty::Fn(_, r) = result {
        result = *r;
        peeled = true;
    }
    if peeled { Some(result) } else { None }
}

/// Report CDZ0401 for every effect operation reached from ENTRYPOINT body `node` with no home — neither
/// an enclosing handler discharging its effect nor a host delegation of it
/// (`capabilities-and-effects.md` §An Ungranted Effect Is A Compile-Time Error). Walks the resolved tree
/// tracking the set of effect-declaration occurrences currently HANDLED (by an enclosing `handle` arm)
/// or DELEGATED (by an enclosing `host`), following non-recursive calls into their callee bodies (a
/// perform may be cross-function). A perform whose effect is not in that set is ungranted → CDZ0401.
pub fn check_no_home(db: &mut Db, node: StructId, out: &mut Vec<crate::diag::Reject>) {
    let mut handled: Vec<u32> = Vec::new();
    check_no_home_walk(db, node, &mut handled, out, 0);
}

fn check_no_home_walk(
    db: &mut Db,
    node: StructId,
    handled: &mut Vec<u32>,
    out: &mut Vec<crate::diag::Reject>,
    depth: u32,
) {
    if depth > 64 {
        return; // backstop — a deep call chain is left to the ordinary decline
    }
    match resolved_of(db, node) {
        // A PERFORM `(E.op args…)`: if its effect is not currently handled/delegated, it has no home.
        Resolved::Apply { head, args } => {
            if let Some((decl, _idx)) = crate::eval::effect_op_of(db, head) {
                if !handled.contains(&decl.0) {
                    out.push(
                        crate::diag::Reject::coded(
                            crate::diag::Code::EffectNoHome,
                            "this effect operation is reached with neither an enclosing handler nor a \
                             host delegation, so it has no home (add a handler or delegate it at the \
                             entrypoint)",
                        )
                        .at(node),
                    );
                }
                // Still walk the args (they may perform other effects).
                for &a in args.iter() {
                    check_no_home_walk(db, a, handled, out, depth);
                }
                return;
            }
            // A CALL into a non-recursive callee — follow it (the perform may be cross-function). The
            // callee's body is checked under the SAME handled set (dynamic extent: the caller's handlers
            // enclose the callee's performs). A recursive callee is not followed (E3), so an ungranted
            // perform only reachable through recursion is not reported here — a conservative miss, safe
            // (it declines at lowering rather than mis-reporting).
            if let Some(callee) = crate::eval::lambda_body(db, head)
                .or_else(|| crate::eval::lambda_body_of_nullary(db, head))
                && !crate::eval::is_recursive(db, callee)
            {
                check_no_home_walk(db, callee, handled, out, depth + 1);
            }
            for &a in args.iter() {
                check_no_home_walk(db, a, handled, out, depth);
            }
        }
        // A `handle` — its arms DISCHARGE their effects for the BODY (dynamic extent). Push each arm's
        // effect decl onto the handled set while walking the body, then pop. The arm BODIES themselves
        // resolve their own performs at the arm's definition context (the under-frame) — but for the
        // no-home check, an arm body performing its own effect re-performs OUTWARD, so we walk arm bodies
        // under the OUTER handled set (without this handle's effects added), matching forwarding. The
        // init is evaluated in the outer context too.
        Resolved::Handle { init, arms, body } => {
            check_no_home_walk(db, init, handled, out, depth);
            // Arm bodies: outer context (a re-performed op forwards to the next-outer handler).
            for arm in &arms {
                check_no_home_walk(db, arm.body, handled, out, depth);
            }
            // Body: this handle's effects are now handled.
            let added: Vec<u32> = arms
                .iter()
                .filter_map(|a| crate::eval::effect_op_of(db, a.op).map(|(d, _)| d.0))
                .collect();
            let before = handled.len();
            handled.extend(&added);
            check_no_home_walk(db, body, handled, out, depth);
            handled.truncate(before);
        }
        // A `host` — its listed effects are DELEGATED for the body. Push each delegated effect's decl.
        Resolved::Host { effects, body } => {
            let added: Vec<(StructId, u32)> = effects
                .iter()
                .filter_map(|&e| {
                    // Each `effect` element is a name occurrence resolving to the effect record; recover
                    // its decl via the record's `(meta t)` = `(effect NAME <decl>)`.
                    host_effect_decl(db, e).map(|d| (e, d))
                })
                .collect();
            // LATENT AUTHORITY (CDZ0404). A delegation must grant EXACTLY the effects that escape — an
            // effect the body never reaches is a granted-but-unexercised capability, rejected
            // (`capabilities-and-effects.md` §Host Delegation Is An Entrypoint's Prerogative). Check each
            // delegated effect is reached by a perform in the body; if not, CDZ0404 (anchored at the
            // delegation's effect-name occurrence).
            for &(occ, decl) in &added {
                if !body_reaches_effect(db, body, decl, 0) {
                    // The repair is to DROP the unreached effect from the manifest — a delete edit on the
                    // effect-name occurrence (`spec/capabilities/diagnostics.md` §A Diagnostic Carries A
                    // Route To A Fix). The effect's name (for the label) comes from its declaration.
                    let eff_name = db
                        .effect_decl_by_occ(crate::ast::StructId(decl))
                        .map(|e| e.name.clone());
                    let label = match &eff_name {
                        Some(n) => format!("remove the unreached effect `{n}` from the delegation"),
                        None => "remove the unreached effect from the delegation".to_string(),
                    };
                    out.push(
                        crate::diag::Reject::coded(
                            crate::diag::Code::LatentAuthority,
                            "this entrypoint delegates an effect to the host that its body never \
                             reaches (latent authority); the manifest must be exactly the effects \
                             that escape",
                        )
                        .at(occ)
                        .with_fix(crate::diag::Fix::delete_heuristic(occ, label)),
                    );
                }
            }
            let before = handled.len();
            handled.extend(added.iter().map(|&(_, d)| d));
            check_no_home_walk(db, body, handled, out, depth);
            handled.truncate(before);
        }
        // A resume's value/next-state, and every other structural form: descend into children.
        _ => {
            if let Struct::List(children) = db.ast.get(node).clone() {
                for c in children {
                    check_no_home_walk(db, c, handled, out, depth);
                }
            }
        }
    }
}

/// Whether the resolved subtree at `node` performs an operation of the effect whose declaration
/// occurrence is `decl` — following calls into their callee bodies (the perform may be cross-function).
/// A RECURSIVE callee IS followed (its body walked ONCE), guarded by a `visited` set of callee-body
/// occurrences so a self-/mutual-recursive cycle terminates. Used by the CDZ0404 latent-authority check:
/// following a recursive callee is required so `(host (log) (go 1))` where `go` recursively performs
/// `log.emit` is NOT falsely flagged as latent authority (its perform IS reached, through the recursion).
fn body_reaches_effect(db: &mut Db, node: StructId, decl: u32, depth: u32) -> bool {
    let mut visited = std::collections::HashSet::new();
    body_reaches_effect_visited(db, node, decl, depth, &mut visited)
}

fn body_reaches_effect_visited(
    db: &mut Db,
    node: StructId,
    decl: u32,
    depth: u32,
    visited: &mut std::collections::HashSet<StructId>,
) -> bool {
    if depth > 64 {
        return false;
    }
    if let Resolved::Apply { head, .. } = resolved_of(db, node)
        && let Some((d, _idx)) = crate::eval::effect_op_of(db, head)
        && d.0 == decl
    {
        return true;
    }
    if let Resolved::Apply { head, .. } = resolved_of(db, node)
        && let Some(callee) = crate::eval::lambda_body(db, head)
            .or_else(|| crate::eval::lambda_body_of_nullary(db, head))
        // Walk the callee's body ONCE — `visited.insert` is false on a re-entry (a recursive cycle),
        // which stops the descent so a self-/mutual-recursive callee terminates.
        && visited.insert(callee)
        && body_reaches_effect_visited(db, callee, decl, depth + 1, visited)
    {
        return true;
    }
    match db.ast.get(node).clone() {
        Struct::List(children) => children
            .iter()
            .any(|&c| body_reaches_effect_visited(db, c, decl, depth, visited)),
        Struct::Atom(_) => false,
    }
}

/// The effect-declaration occurrence a `host` delegation's effect-name occurrence `e` names — resolve it
/// to its effect record and read the record's `(meta t)` = `(effect NAME <decl>)`. `None` if `e` does
/// not name an effect (a malformed delegation, reported elsewhere).
fn host_effect_decl(db: &mut Db, e: StructId) -> Option<u32> {
    let field = crate::eval::project_meta(db, e, "t")?;
    // `(effect NAME <decl>)` — decl is the third element (index 2), a decimal integer literal.
    let tail = db.ast.as_form(field, "effect")?;
    let decl_occ = tail.get(1).copied()?;
    match resolved_of(db, decl_occ) {
        Resolved::Int(v) => v.to_i64().and_then(|n| u32::try_from(n).ok()),
        _ => None,
    }
}

/// are the resolved handle's children.
pub fn reduce_handle(
    db: &mut Db,
    init: StructId,
    arms: &[HandleArm],
    body: StructId,
) -> Option<StructId> {
    // RE-ENTRY GUARD, held for the whole fold. `type_of` of a `handle` calls `reduce_handle` (E1c-2), and
    // `resume_result_type_ok`/`type_of(init)` below type nodes that can reach an enclosing handle — so
    // reducing a handle can re-enter this same `reduce_handle`. `db.enter_reduction()` bumps the shared
    // reduction-depth guard (the same backstop β-reduction uses); past the bound it returns `None` → we
    // decline. Held across the whole fold so every `type_of` inside is bounded. (The unbounded INLINE
    // loop — a recursive effectful callee β-reduced to a fresh non-self-referential copy each level —
    // is bounded separately by `THREAD_INLINE_LIMIT` in `thread_bounded`.)
    let mut guard = db.enter_reduction()?;
    let db = guard.db();
    // Build the operation→arm map, keyed by each arm's operation identity (read off the arm's op
    // projection's `(meta effect-op)`). An arm whose op is not an effect operation (a malformed arm) or
    // whose op the effect does not declare (CDZ0403 — reported elsewhere) makes the fold decline.
    let mut map = HashMap::default();
    for arm in arms {
        let (decl, idx) = crate::eval::effect_op_of(db, arm.op)?;
        // GUARD: the operation's declared RESULT type must be DETERMINED, and — for a tail resume — the
        // resume VALUE must agree with it. If the result type is undetermined (a malformed op arrow the
        // evaluator can't reduce, e.g. a single-arg `(-> T)`), the fold cannot safely substitute the
        // resume value as the perform's result — decline (a Todo), so the fold NEVER runs a program whose
        // resume value is unchecked against an unknown result type. A determined-but-MISMATCHED resume
        // value (`(resume true s)` for an Int64 op) is reported CDZ0201 by `check_resume_result_type`;
        // declining here as well keeps the fold from emitting the mistyped value (belt-and-suspenders —
        // the fault side rejects, the value side declines, so neither a wrong VALUE nor a wrong CODE ships).
        if !resume_result_type_ok(db, arm) {
            return None;
        }
        map.insert((decl.0, idx), arm.clone());
    }
    // This handle discharges ONE effect (all its arms share a decl — a handle's arms are for one effect),
    // so the context has ONE state slot. `state_ty_of_arms` derives the slot's state type from the init
    // seed joined with the arms' next-states. A NESTED handler whose shared recursive callee performs both
    // effects is MERGED into a multi-slot context by `thread_bounded`'s nested-`Handle` arm.
    let decl = arms
        .first()
        .and_then(|a| crate::eval::effect_op_of(db, a.op))
        .map(|(d, _)| d.0)?;
    let state_ty = state_ty_of_arms(db, init, arms);
    let slot = StateSlot { decl, state_ty };
    let ctx = HandlerCtx::new(db, map, vec![slot]);
    // ABORTIVE (E4) TYPE-CONSISTENCY GUARD. An abortive arm materializes its BODY as the abort value, which
    // becomes the value of the position the perform occupied — a position the type checker typed by the
    // op's declared RESULT type (a perform types as its result, never as the arm value). If the arm body's
    // type differs from that result type (`bail : Int64 -> Bool` but the arm yields `n : Int64`), the abort
    // value does not fit where it lands: in a conditional it disagrees with the sibling branch and emits an
    // ill-typed `if` (invalid wasm). The checker misses this gap, so guard it in the fold — decline when any
    // abortive arm's body type does not match its operation's result type. (A tail-resumptive arm is already
    // covered: `resume_result_type_ok` checked its resume value against the result type above.)
    let undetermined = |t: &crate::ty::Ty| matches!(t, crate::ty::Ty::Any | crate::ty::Ty::Var(_));
    let abortive_keys: Vec<(u32, u32)> = ctx.abortive.iter().copied().collect();
    for (d, i) in abortive_keys {
        let arm_op = ctx.arms.get(&(d, i))?.op;
        let arm_body = ctx.arms.get(&(d, i))?.body;
        let body_ty = crate::infer::type_of(db, arm_body);
        let result_ty = op_result_type(db, arm_op);
        // Compare structurally; an undetermined side (an `Any`/var) does not disqualify (the abort value
        // then flows unconstrained, matching the E4-a strict cases that already work). Only a DEFINITE
        // disagreement (two concrete, distinct ground types) declines.
        if let Some(rt) = result_ty
            && !undetermined(&body_ty)
            && !undetermined(&rt)
            && body_ty != rt
        {
            return None;
        }
    }
    // ABORTIVE (E4) NON-TAIL HOIST. An abort in a strict OPERAND under a conditional — `(+ 100 (if c
    // (Bail.bail 7) 50))` — is not directly foldable (the abort must escape the `+`). But an abort
    // ABANDONS the enclosing computation, so distributing the surrounding strict op INTO both `if` branches
    // is value-preserving: `(+ 100 (if c (Bail.bail 7) 50))` ≡ `(if c (+ 100 (Bail.bail 7)) (+ 100 50))`.
    // In the rewritten form the abort sits in an `if` BRANCH TAIL — the shape the per-branch fold
    // (`thread_branch_local_abort`) already handles (the abort branch's `(+ 100 (Bail.bail 7))` is an
    // unconditional-abort-in-a-branch, captured locally). Sound because the op's OTHER operands are pure
    // (the hoist requires it), so duplicating them across the two branches changes no observable effect.
    // Runs to a fixpoint (bounded) so a nested non-tail abort is lifted level by level; a shape it can't
    // lift is left as-is and the guard below declines it.
    let body = hoist_conditional_abort(db, body, &ctx);
    // ABORTIVE (E4) SOUNDNESS GUARD. Two sound abort shapes are realized below: (1) an UNCONDITIONAL abort
    // collapses the whole handle to the arm value; (2) an abort in the TAIL of a tail-position `if` branch
    // folds per-branch (the branch-local abort restores the cell so the other branch survives — see
    // `thread_branch_local_abort`). An abort at any OTHER conditional position the hoist above could not
    // lift to a branch tail (e.g. under an EFFECTFUL sibling operand, or a short-circuit connective) still
    // fires on one path with no realizable shape — decline cleanly rather than miscompile.
    if !ctx.abortive.is_empty() && body_has_unsound_abortive_perform(db, body, &ctx, true, false) {
        return None;
    }
    // Thread the INIT state through the body in evaluation order. The handle's value is the body's
    // value (the accumulated state is observable only through the operations), so we return the
    // rewritten body; the final threaded state is discarded (the body never reads it directly).
    let (rewritten, _final_states) = thread(db, body, vec![init], &ctx)?;
    // ABORTIVE (E4): if an abortive perform fired during threading, the handle's value is that arm's
    // value — the surrounding computation was abandoned, so the threaded body is dead. Return the abort
    // value directly. (Unconditional strict abort only; a conditional abort was declined above.)
    if let Some(abort) = ctx.abort_value.get() {
        return Some(abort);
    }
    Some(rewritten)
}

/// Whether the subtree at `node` contains a perform of an ABORTIVE operation (one in `ctx.abortive`) —
/// the trigger for the non-tail hoist. A structural walk mirroring `subtree_performs`, narrowed to
/// abortive ops only (a tail-resumptive perform is threaded, not hoisted).
fn subtree_has_abortive_perform(db: &mut Db, node: StructId, ctx: &HandlerCtx) -> bool {
    if let Resolved::Apply { head, .. } = resolved_of(db, node)
        && let Some(id) = is_perform(db, head, ctx)
        && ctx.abortive.contains(&id)
    {
        return true;
    }
    match db.ast.get(node).clone() {
        Struct::List(children) => children
            .iter()
            .any(|&c| subtree_has_abortive_perform(db, c, ctx)),
        Struct::Atom(_) => false,
    }
}

/// Rewrite a NON-TAIL conditional abort into a branch-tail one by distributing the enclosing strict op
/// into both `if` branches, to a fixpoint. `(+ 100 (if c (Bail.bail 7) 50))` → `(if c (+ 100 (Bail.bail
/// 7)) (+ 100 50))`. Sound because an abort ABANDONS the enclosing computation (so pushing the op into
/// the aborting branch, where it never completes, changes nothing) and the op's OTHER operands are PURE
/// (required below — duplicating a pure operand across the two branches is observably identical). After
/// the lift the abort sits in an `if` branch tail, the shape `thread_branch_local_abort` already folds.
///
/// Only a genuine application `(op a…)` that is NOT itself a perform / special form is a hoist site, and
/// only when EXACTLY the `if`-argument's subtree holds the abortive perform while every sibling operand
/// is perform-free. A shape that does not match is left unchanged (the guard then declines it). Bounded
/// by a rewrite budget so a pathological input cannot loop.
fn hoist_conditional_abort(db: &mut Db, node: StructId, ctx: &HandlerCtx) -> StructId {
    let mut cur = node;
    // A generous fixpoint bound: each pass lifts at least one `if` one level, and a body has far fewer
    // than this many nested strict operands. Prevents any accidental non-convergence.
    for _ in 0..256 {
        match hoist_once(db, cur, ctx) {
            Some(next) => cur = next,
            None => break,
        }
    }
    cur
}

/// One rewrite step of [`hoist_conditional_abort`]: find the FIRST (pre-order) application `(op a…)` with
/// an `if` operand carrying an abortive perform and all other operands pure, distribute the op into the
/// branches, and return the rewritten WHOLE tree. `None` if no such site exists (fixpoint reached).
fn hoist_once(db: &mut Db, node: StructId, ctx: &HandlerCtx) -> Option<StructId> {
    // Is THIS node a hoist site? A plain application `(op a0 … ak)` — head not a perform — with one arg an
    // `if` holding an abortive perform and every OTHER arg perform-free.
    if let Resolved::Apply { head, args } = resolved_of(db, node)
        && is_perform(db, head, ctx).is_none()
    {
        // The op must be a strict primitive/operator whose operands all evaluate — an `if`/`let`/`match`
        // head is a special form handled by its own thread arm, not a distributable op. `resolved_of` of
        // an operator head is a prim; we simply require the head itself to be perform-free and rebuild the
        // same head. (Being conservative: we only distribute when the head is perform-free.)
        if !subtree_performs(db, head, ctx) {
            for (i, &a) in args.iter().enumerate() {
                if let Resolved::If { cond, then_, else_ } = resolved_of(db, a)
                    && subtree_has_abortive_perform(db, a, ctx)
                {
                    // Every OTHER operand (and the head) must be perform-free, so distributing them into
                    // both branches duplicates only pure code. The `if`'s CONDITION must also be pure (it
                    // evaluates once before the branch; duplicating it is fine only if pure — an effectful
                    // condition would perform twice). If any sibling or the condition performs, this is not
                    // a sound hoist site — leave it (the guard declines).
                    let others_pure = args
                        .iter()
                        .enumerate()
                        .all(|(j, &b)| j == i || !subtree_performs(db, b, ctx));
                    let cond_pure = !subtree_performs(db, cond, ctx);
                    if others_pure && cond_pure {
                        // Build `(op a0 … <branch> … ak)` for each branch, then `(if cond then' else')`.
                        let rebuild = |db: &mut Db, branch: StructId| -> StructId {
                            let children: Vec<StructId> = std::iter::once(head)
                                .chain(
                                    args.iter()
                                        .enumerate()
                                        .map(|(j, &b)| if j == i { branch } else { b }),
                                )
                                .collect();
                            db.push_list(children)
                        };
                        let new_then = rebuild(db, then_);
                        let new_else = rebuild(db, else_);
                        let if_head = db.push_atom(Leaf::Name("if".to_string()));
                        return Some(db.push_list(vec![if_head, cond, new_then, new_else]));
                    }
                }
            }
        }
    }
    // A SHORT-CIRCUIT connective `(and lhs rhs)` / `(or lhs rhs)` whose RIGHT operand carries an abort is
    // itself a conditional in disguise — `rhs` runs on only one value of `lhs`. Desugar it to the
    // equivalent `if` so the abort lands in a branch tail the per-branch capture folds: `(and lhs rhs)` ≡
    // `(if lhs rhs false)` (rhs runs only when lhs is true), `(or lhs rhs)` ≡ `(if lhs true rhs)` (rhs runs
    // only when lhs is false). `lhs` becomes the `if` CONDITION — evaluated exactly once either way, so no
    // duplication and no purity constraint on it (a later hoist pass lifts any abort inside `lhs`, which is
    // an unconditional strict position — the connective always evaluates lhs). Only fires when `rhs` holds
    // an abortive perform (a plain short-circuit with no abort threads unchanged). The desugared `if`'s
    // aborting branch materializes the abort VALUE opposite a Bool constant, so it must be Bool-typed —
    // guaranteed by the TYPE-CONSISTENCY guard at the top of `reduce_handle` (an abortive arm whose body
    // type ≠ its op's result type declines before reaching here; a connective operand's op result IS Bool,
    // so a surviving abortive arm yields Bool). A plain short-circuit with no abort is not a site.
    if let Resolved::And { lhs, rhs, is_and } = resolved_of(db, node)
        && subtree_has_abortive_perform(db, rhs, ctx)
    {
        let if_head = db.push_atom(Leaf::Name("if".to_string()));
        let (then_, else_) = if is_and {
            let false_lit = db.push_atom(Leaf::Bool(false));
            (rhs, false_lit) // (and lhs rhs) ≡ (if lhs rhs false)
        } else {
            let true_lit = db.push_atom(Leaf::Bool(true));
            (true_lit, rhs) // (or lhs rhs) ≡ (if lhs true rhs)
        };
        return Some(db.push_list(vec![if_head, lhs, then_, else_]));
    }
    // Not a site here — recurse into children, rebuilding with the FIRST rewritten child. A special form
    // (`if`/`let`/`match`) is descended structurally too: a non-tail abort nested in a `let` init or an
    // `if` branch's operand is lifted within that sub-position, then the enclosing thread arm folds it.
    if let Struct::List(children) = db.ast.get(node).clone() {
        for (k, &c) in children.iter().enumerate() {
            if let Some(new_c) = hoist_once(db, c, ctx) {
                let mut new_children = children.clone();
                new_children[k] = new_c;
                return Some(db.push_list(new_children));
            }
        }
    }
    None
}

/// Whether `node` contains an abortive perform (of an op in `ctx.abortive`) the fold CANNOT realize
/// soundly — run AFTER `hoist_conditional_abort`, so every abort the hoist could lift to a branch tail
/// already has been. What remains SOUND: (1) an UNCONDITIONAL abort (`(+ 1 (Bail.bail 7))` — E4-a
/// collapses the whole handle to the arm value); (2) an abort anywhere inside a TAIL-position `if`/`let`
/// branch reached through only PURE STRICT OPS — `thread_branch_local_abort` captures the branch's abort
/// regardless of how deep the pure nesting is (`(if c (+ 100 (Bail.bail 7)) …)` folds: the branch value
/// IS the abort). UNSOUND (flagged → `reduce_handle` declines): an abort the hoist could not lift and the
/// per-branch capture cannot intercept — (a) under a SHORT-CIRCUIT connective `and`/`or` right operand
/// (threaded strictly, collapses the whole handle across a branch), (b) alongside an EFFECTFUL sibling
/// operand (the hoist requires pure siblings to distribute, so a perform-bearing sibling blocks the lift),
/// or (c) in a conditional's CONDITION. `tail` = value flows to the handle value along a pure-strict path
/// from the nearest enclosing branch; `under_cond` = descended into a conditional branch. Flag = an abort
/// at `under_cond && !tail`.
fn body_has_unsound_abortive_perform(
    db: &mut Db,
    node: StructId,
    ctx: &HandlerCtx,
    tail: bool,
    under_cond: bool,
) -> bool {
    // An abort reached under a conditional at a non-tail (non-capturable) position is the unsound shape.
    if under_cond
        && !tail
        && let Resolved::Apply { head, .. } = resolved_of(db, node)
        && let Some(id) = is_perform(db, head, ctx)
        && ctx.abortive.contains(&id)
    {
        return true;
    }
    // An `if`: the CONDITION is a NON-tail, NON-branch strict operand (an abort in a condition can't be
    // captured per-branch — treat it as `tail=false`). Each BRANCH is a conditional position:
    // `under_cond=true`, and `tail` carries the `if`'s own tail-ness (a tail `if` → tail branches, whose
    // aborts the per-branch capture intercepts; a non-tail `if` → non-tail branches, flagged).
    if let Resolved::If { cond, then_, else_ } = resolved_of(db, node) {
        return body_has_unsound_abortive_perform(db, cond, ctx, false, under_cond)
            || body_has_unsound_abortive_perform(db, then_, ctx, tail, true)
            || body_has_unsound_abortive_perform(db, else_, ctx, tail, true);
    }
    // A `(let ((n init)…) body)`: the let's VALUE is the BODY's value, so the body inherits THIS position's
    // tail-ness + `under_cond`. Each INIT is a strict operand — NON-tail, carrying the let's `under_cond`.
    if let Some(form) = db.ast.as_form(node, "let")
        && form.len() == 2
    {
        let (bindings_occ, body_occ) = (form[0], form[1]);
        if let Struct::List(pairs) = db.ast.get(bindings_occ).clone() {
            for pair in pairs {
                if let Struct::List(kv) = db.ast.get(pair).clone()
                    && kv.len() == 2
                    && body_has_unsound_abortive_perform(db, kv[1], ctx, false, under_cond)
                {
                    return true;
                }
            }
        }
        return body_has_unsound_abortive_perform(db, body_occ, ctx, tail, under_cond);
    }
    // Generic descent over a strict application `(op a0 … ak)`. KEY (post-hoist): an abort nested under a
    // PURE strict op INSIDE a tail branch is CAPTURABLE — `thread_branch_local_abort` takes the branch's
    // abort value regardless of the pure `op` wrapping it — so we PRESERVE `tail` for such an operand
    // rather than lowering it. An operand is capturable-tail iff (i) we are already on a capturable path
    // (`tail`), (ii) the op is NOT a short-circuit connective, and (iii) every OTHER operand (and the head)
    // is PERFORM-FREE (an effectful sibling would run before/after the abort and cannot be duplicated or
    // dropped — the hoist declines to lift across it, so its abort stays flagged). A short-circuit right
    // operand (index >= 2 of `and`/`or`) is conditional AND non-capturable (threaded strictly), so it is
    // marked `under_cond` and `tail=false`.
    match db.ast.get(node).clone() {
        Struct::List(children) => {
            let head_name = children
                .first()
                .and_then(|&c| db.ast.as_name(c).map(str::to_string));
            let is_short_circuit = matches!(head_name.as_deref(), Some("and") | Some("or"));
            // Whether every element (head + operands) OTHER than `i` is perform-free — so the abort in
            // operand `i` is the sole effect and its enclosing pure op is transparent to the per-branch
            // capture (the branch collapses to the abort value, discarding the pure wrapper).
            let siblings_pure = |db: &mut Db, i: usize| -> bool {
                (0..children.len()).all(|j| j == i || !subtree_performs(db, children[j], ctx))
            };
            (0..children.len()).any(|i| {
                let c = children[i];
                if i == 0 {
                    // The head: descend as a strict, non-capturable operand (a head rarely performs).
                    return body_has_unsound_abortive_perform(db, c, ctx, false, under_cond);
                }
                let sc_right = is_short_circuit && i >= 2;
                // Capturable-tail: on a tail path, not a short-circuit right operand, siblings all pure.
                let capturable = tail && !sc_right && siblings_pure(db, i);
                let child_tail = capturable;
                let child_cond = under_cond || sc_right;
                body_has_unsound_abortive_perform(db, c, ctx, child_tail, child_cond)
            })
        }
        Struct::Atom(_) => false,
    }
}

/// The state type for a handler whose arms are `arms` seeded with `init` — the init seed's type JOINED
/// with each tail-arm's NEXT-STATE type. The seed of an accumulating handler is often an empty collection
/// whose element is undetermined (`(list)` : `List ?`), while an arm that GROWS the state
/// (`(resume unit (List.push s code))`) has a concrete element (`code` types from the op's declared
/// parameter — `handle_arm_param_ty`). Joining fixes the empty seed's element from the growing arm; a
/// read-out arm whose next-state is a bare `s` passthrough types as a var and `join` yields the other
/// (more-defined) side, so it never poisons the element. Reads only the arm's NEXT-STATE (`resume`'s 2nd
/// arg), never its resume VALUE, so a `Unit` resume value cannot bleed into the state element. `None` if
/// the joined type is still undetermined (then a recursive specialization declines).
fn state_ty_of_arms(db: &mut Db, init: StructId, arms: &[HandleArm]) -> Option<crate::ty::Ty> {
    let init_t = crate::infer::type_of(db, init);
    let mut t = init_t.clone();
    for arm in arms {
        if let Some(next) = tail_resume_next_state_of(db, arm.body) {
            let nt = crate::infer::type_of(db, next);
            t = t.join(&nt);
        }
    }
    if matches!(t, crate::ty::Ty::Any) {
        None
    } else {
        Some(t)
    }
}

/// Whether the arm's tail resume VALUE agrees with the operation's declared RESULT type, AND that result
/// type is DETERMINED (not `Any`). `true` when: the arm has no tail resume (out of scope — the fold will
/// decline for other reasons), OR the result type is determined and the resume value's type agrees with
/// it. `false` when the result type is undetermined (a malformed op arrow) or the resume value's type
/// disagrees — either way the fold must decline rather than substitute an unverified/mistyped value.
fn resume_result_type_ok(db: &mut Db, arm: &HandleArm) -> bool {
    // The op's result type: peel the op value's `(meta t)` scheme `(fn () (-> P… Result))` to the final
    // result. An op whose arrow does not reduce (`(-> (List Int64))` — a single-arg arrow the FnCtor
    // can't build) yields no `Fn`, leaving a non-arrow / `Any` result → treat as UNDETERMINED (decline).
    let mut fresh = crate::unify::Fresh::new();
    let Some(scheme) = crate::eval::scheme_of(db, arm.op, &mut fresh) else {
        return false;
    };
    let mut result = crate::unify::instantiate(&scheme, &mut fresh);
    let mut peeled_any = false;
    while let crate::ty::Ty::Fn(_, r) = result {
        result = *r;
        peeled_any = true;
    }
    // If the op's `(meta t)` did not reduce to a function type at all (no arrow peeled), the operation's
    // result type is undetermined — decline. (A well-formed op is `(fn () (-> P Result))`, always an arrow.)
    if !peeled_any {
        return false;
    }
    // An undetermined result (`Any`) — the arrow reduced but its result is unknown — is not safe to
    // substitute against; decline.
    if matches!(result, crate::ty::Ty::Any) {
        return false;
    }
    // Check the tail resume value's type against the determined result type. No tail resume → not this
    // fold's concern (it will decline elsewhere), so do not block on it here.
    let Some(value) = tail_resume_value_of(db, arm.body) else {
        return true;
    };
    let value_ty = crate::infer::type_of(db, value);
    value_ty.agrees_with(&result)
}

/// The VALUE of a tail `(resume value next-state)` in the arm body, or `None` if the body is not a tail
/// resume. Reads the ORIGINAL (un-substituted) arm body.
fn tail_resume_value_of(db: &mut Db, node: StructId) -> Option<StructId> {
    match resolved_of(db, node) {
        Resolved::Resume { value, .. } => Some(value),
        _ => None,
    }
}

/// The declared type of a HANDLE-ARM operation parameter `binder` — read from the arm's operation
/// signature (`capabilities-and-effects.md` §Performing An Operation Is Typed: an operation's args are
/// typed against its declared parameter types). An arm `(E.op (p0 p1 …) state body)` binds `pk` to the
/// op's k-th parameter type: instantiate the op value's `(meta t)` scheme `(fn () (-> P0 (-> P1 …
/// Result)))` and peel to the k-th domain. `None` if `binder` is not a handle-arm op param (a bare param,
/// a def/fn param, the state binder). This is what lets `(List.push s code)` in a `Diag.emit` arm type
/// `code` as `Int64` (its declared param) rather than an unconstrained fresh variable.
pub fn handle_arm_param_ty(db: &mut Db, binder: StructId) -> Option<crate::ty::Ty> {
    // `binder` sits in the arm's params list: binder → params-list → arm. The arm must be a handle arm
    // `(op (params…) state body)` (4 elements), and `binder` its own occurrence in the params list.
    let params_list = db.parent_of(binder)?;
    let arm = db.parent_of(params_list)?;
    if !crate::resolve::is_handle_arm(db, arm) {
        return None;
    }
    let Struct::List(parts) = db.ast.get(arm).clone() else {
        return None;
    };
    // The params list is the arm's 2nd element (index 1); confirm and find `binder`'s position in it.
    if parts.get(1).copied() != Some(params_list) {
        return None;
    }
    let Struct::List(params) = db.ast.get(params_list).clone() else {
        return None;
    };
    let k = params.iter().position(|&p| p == binder)?;
    // The op's declared arrow: instantiate the op value's `(meta t)` scheme, then peel `k` domains to
    // reach the k-th parameter type. `(fn () (-> P0 (-> P1 Result)))` → peel k `Fn`s and take the domain.
    let op = parts[0];
    let mut fresh = crate::unify::Fresh::new();
    let scheme = crate::eval::scheme_of(db, op, &mut fresh)?;
    let mut cur = crate::unify::instantiate(&scheme, &mut fresh);
    for _ in 0..k {
        match cur {
            crate::ty::Ty::Fn(_, r) => cur = *r,
            _ => return None,
        }
    }
    match cur {
        crate::ty::Ty::Fn(p, _) => Some(*p),
        _ => None,
    }
}

fn tail_resume_next_state_of(db: &mut Db, node: StructId) -> Option<StructId> {
    match resolved_of(db, node) {
        Resolved::Resume { next_state, .. } => Some(next_state),
        _ => None,
    }
}

/// Rewrite `node` under handler context `ctx`, threading `states` (the current-state expression PER SLOT,
/// slot order) through it in EVALUATION ORDER. Returns `(rewritten-node, next-states)` — the node with
/// performs resolved and the states as they stand AFTER `node` evaluates — or `None` to decline (a shape
/// not provably tail-resumptive). Each state is an arena occurrence (an expression); a perform of an
/// operation in slot `k` reads/updates slot `k` (substituting slot `k`'s state into the arm's `state`
/// binder), leaving the other slots unchanged. A single-handler context has one slot; a merged nested
/// context has one per handler.
fn thread(
    db: &mut Db,
    node: StructId,
    states: Vec<StructId>,
    ctx: &HandlerCtx,
) -> Option<(StructId, Vec<StructId>)> {
    thread_bounded(db, node, states, ctx, 0)
}

/// Thread an `if` BRANCH, capturing an abortive perform as LOCAL to the branch (E4): if threading the
/// branch set the whole-handle `abort_value` cell, the branch's value is that arm value and the cell is
/// CLEARED (so the abort does not collapse the OTHER branch or the whole handle — a branch-tail abort
/// just yields the arm value for that branch, since the enclosing `if` is the handle body's value).
/// Returns the branch's rewritten value (the abort value if it aborted, else the threaded rewrite). The
/// branch's out-state is discarded (nothing after an `if` in the tail-fold shape reads it).
fn thread_branch_local_abort(
    db: &mut Db,
    branch: StructId,
    states: Vec<StructId>,
    ctx: &HandlerCtx,
    inline_depth: u32,
) -> Option<StructId> {
    let before = ctx.abort_value.get();
    let (rbranch, _) = thread_bounded(db, branch, states, ctx, inline_depth)?;
    let after = ctx.abort_value.get();
    // A NEW abort fired while threading THIS branch → it is local to the branch: use the abort value as
    // the branch's rewrite and restore the cell to its prior state (so a sibling branch / the handle is
    // not collapsed). If no new abort fired, keep the ordinary threaded rewrite.
    if after != before
        && let Some(abort) = after
    {
        ctx.abort_value.set(before);
        Some(abort)
    } else {
        Some(rbranch)
    }
}

/// Bound on cross-function INLINE depth during threading. A handled body that inlines callees deeper
/// than this DECLINES rather than unrolling without end (`reference-compiler.md` §An Unbounded Handler
/// Context Declines) — the safe backstop against a RECURSIVE effectful callee slipping past the
/// `is_recursive` exclusion (β-reduction produces a fresh non-self-referential copy each inline, so a
/// naive `is_recursive` on the copy reads false and would unroll forever). E3's proper per-context
/// specialization handles a recursive callee; anything the specialization does not cover, this bound
/// declines. Set well above any real non-recursive inline chain in the corpus.
const THREAD_INLINE_LIMIT: u32 = 64;

fn thread_bounded(
    db: &mut Db,
    node: StructId,
    states: Vec<StructId>,
    ctx: &HandlerCtx,
    inline_depth: u32,
) -> Option<(StructId, Vec<StructId>)> {
    if inline_depth > THREAD_INLINE_LIMIT {
        return None; // an unbounded inline chain — decline (a recursive callee the spec path missed)
    }
    match resolved_of(db, node) {
        // A PERFORM `(E.op args…)` of a discharged operation: resolve to its arm, substitute the arm's
        // params ↦ (rewritten) args and its state binder ↦ the CURRENT state OF THAT OP'S SLOT, and rewrite
        // the arm body's TAIL resume to the resume VALUE, threading the resume's next-STATE forward IN THAT
        // SLOT (the other slots pass through unchanged). This is what lets nested handlers over one
        // recursive callee thread each effect's state independently.
        Resolved::Apply { head, args } if is_perform(db, head, ctx).is_some() => {
            let (decl, idx) = is_perform(db, head, ctx).unwrap();
            let arm = ctx.arms.get(&(decl, idx))?.clone();
            let slot = ctx.slot_of(decl)?;
            // Thread state through each argument left-to-right (an argument may itself perform).
            let mut cur = states;
            let mut rewritten_args = Vec::with_capacity(args.len());
            for &a in args.iter() {
                let (ra, next) = thread_bounded(db, a, cur, ctx, inline_depth)?;
                rewritten_args.push(ra);
                cur = next;
            }
            // The arm binds its params to the args and its state binder to THIS SLOT's current state.
            // Substitute both into the arm body (a capture-safe arena substitution), then extract the tail
            // resume.
            let mut subst: HashMap<StructId, StructId> = HashMap::default();
            if arm.params.len() == rewritten_args.len() {
                for (&p, &a) in arm.params.iter().zip(&rewritten_args) {
                    // The `()` placeholder param of a nullary op binds nothing — skip it.
                    if !is_unit_param(db, p) {
                        subst.insert(p, a);
                    }
                }
            } else if arm.params.len() == 1 && rewritten_args.is_empty() {
                // A NULLARY perform `(E.op)` for a `(-> Unit T)` op: the arm's single parameter is the
                // ELIDED unit argument — either written `()` (binds nothing) or a named binder `u` (binds
                // to unit). Supply a synthesized `unit` so a named unit binder resolves.
                let p = arm.params[0];
                if !is_unit_param(db, p) {
                    let unit = db.push_list(vec![]); // `()` — the unit value
                    subst.insert(p, unit);
                }
            } else {
                // Any other arity mismatch — decline.
                return None;
            }
            subst.insert(arm.state, cur[slot]);
            let arm_body = crate::eval::beta_reduce(db, arm.body, &subst);
            // ABORTIVE arm (E4): the arm never resumes, so performing it ABANDONS the surrounding
            // computation — the arm body value becomes the handle's value. Record it in the ctx's abort
            // cell (interior-mutable) and return it as this node's value; the enclosing strict context is
            // dead. Two callers read the cell: `reduce_handle` (an UNCONDITIONAL abort collapses the whole
            // handle to this value) and `thread_branch_local_abort` (a TAIL abort in an `if` branch is
            // local — it uses this value for the branch and restores the cell). An UNSOUND conditional
            // abort (non-tail branch, condition) was declined by `body_has_unsound_abortive_perform` in
            // `reduce_handle` before threading, so it never reaches here. State does not thread (abandoned).
            if ctx.abortive.contains(&(decl, idx)) {
                let copied = copy_pure(db, arm_body);
                ctx.abort_value.set(Some(copied));
                return Some((copied, cur));
            }
            // The arm body must reduce to a TAIL `(resume value next-state)` — the value becomes the
            // perform's result; the next-state threads forward IN THIS SLOT. Two shapes:
            //   * a bare `(resume v s)` — the value is `v`.
            //   * a `(do stmt… (resume v s))` — an INTERPOSING/FORWARDING arm that runs side-effecting
            //     STATEMENTS (a perform of ANOTHER effect — an outer handler's, or a host op — recorded
            //     before forwarding) then resumes. The statements must RUN, so the perform's result is a
            //     `(do stmt… v)`: the statements sequenced (folded later under their own enclosing
            //     handler / emitted as host calls), then the resume value. This is what lets an inner
            //     handler INTERPOSE on a delegated effect — count it via an outer effect, then forward.
            let (value, next_state) = match tail_resume(db, arm_body) {
                Some(vs) => vs,
                None => {
                    // A `(do stmt… (resume v s))` arm body: peel the trailing resume, keep the stmts.
                    let items = db.ast.as_form(arm_body, "do")?.to_vec();
                    let (&last, stmts) = items.split_last()?;
                    let (v, s) = tail_resume(db, last)?;
                    // Rebuild `(do stmt… v)` — the statements run for effect, then `v` is the value. A
                    // pure-statement `do` would have no reason to wrap the resume, so the statements here
                    // carry the interposing side effects that must survive.
                    let do_head = db.push_name("do");
                    let mut children = vec![do_head];
                    children.extend_from_slice(stmts);
                    children.push(v);
                    (db.push_list(children), s)
                }
            };
            cur[slot] = next_state;
            Some((value, cur))
        }
        // A `do` sequence — `(do e0 e1 … en)`. Evaluate each in EVALUATION ORDER, threading state; the
        // sequence's value is the LAST expression's value, its state the last's next-state. (A `do` is
        // grammar; the resolver does not model a NESTED `do` as an expression at all — it declines — so we
        // cannot rebuild a `do` node. Instead we return the LAST item's rewrite: after folding, each
        // earlier item is a PURE value-expression (its effect was folded into the threaded state), so its
        // value being discarded is exactly `do`'s semantics — evaluate for effect, yield the last. If an
        // earlier item did NOT fold to a pure expression it would carry an unresolved perform and the
        // whole handler would already have declined; and a `do` item that is a bare literal/constant has
        // no effect to preserve. So dropping the earlier rewrites is sound for the tail-resumptive surface
        // this fold serves. (A `do` item with a residual RUNTIME trap is out of scope here — that needs
        // nested-`do` value support, an orthogonal feature.)
        _ if db.ast.as_form(node, "do").is_some() => {
            let items: Vec<StructId> = db.ast.as_form(node, "do").unwrap().to_vec();
            if items.is_empty() {
                return None;
            }
            let mut cur = states;
            let mut last = None;
            for it in items {
                let (r, next) = thread_bounded(db, it, cur, ctx, inline_depth)?;
                last = Some(r);
                cur = next;
            }
            Some((last.unwrap(), cur))
        }
        // A CROSS-FUNCTION perform: `(f args…)` where `f` is a NON-RECURSIVE function whose body reaches
        // A RECURSIVE effectful call `(f args…)` — `f` recurses AND reaches a discharged op, so it cannot
        // be inlined (it would not terminate). SPECIALIZE it: emit `f#ctx` once per handler context
        // (`DESIGN-effects-rcdzc.md` §4.3), threading this context's state as a trailing parameter, and
        // emit a call `(f#ctx args… <current-state>)`. Checked BEFORE the cross-function inline arm below,
        // because a β-reduced copy of `f`'s body is NOT self-referential (its self-call names the original
        // `f`), so `is_recursive` on the copy reads false — if the inline arm ran first it would unroll
        // the recursion one level per inline (bounded only by the depth backstop). Catching recursion
        // here first routes `f` to specialization, never to the unbounded inline.
        Resolved::Apply { head, args }
            if ctx.has_state() && recursive_call_reaches_discharged(db, &head, ctx) =>
        {
            // Thread state through the args first (they evaluate before the call), then the call takes the
            // current threaded state OF EVERY SLOT as its trailing arguments (in slot order — the order
            // `specialize_recursive` lays the trailing state params out in).
            let mut cur = states;
            let mut rargs = Vec::with_capacity(args.len() + ctx.slots.len());
            for &a in args.iter() {
                let (ra, next) = thread_bounded(db, a, cur, ctx, inline_depth)?;
                rargs.push(ra);
                cur = next;
            }
            let spec = specialize_recursive(db, head, ctx)?;
            rargs.extend(cur.iter().copied()); // one trailing state arg per slot, in slot order
            // Build the call `(<spec-name> args… state…)`. The specialized def is named, so a name atom
            // resolves to it (via `def_by_name`), and the ordinary recursive `Core::Call` + reachability
            // path emits it.
            let name_atom = db.push_atom(Leaf::Name(spec));
            let mut call = vec![name_atom];
            call.extend(rargs);
            // The call's VALUE is the specialized fn's result; the states after it are not observed (the
            // corpus never reads post-recursion state — the single-return shape).
            Some((db.push_list(call), cur))
        }
        // A CROSS-FUNCTION perform: `(f args…)` where `f` is a NON-RECURSIVE function whose body reaches
        // an operation this handler discharges (`DESIGN-effects-rcdzc.md` §3, the new inline trigger). The
        // handler must be present in the callee, so INLINE `f` into the handled region — β-reduce the call
        // (substitute args for params) and thread state through the reduced body, exactly as if the
        // callee's body were written inline. This is what makes `(handle … (gen))` work when `gen`
        // performs the discharged op. A RECURSIVE such callee is caught by the specialization arm ABOVE.
        Resolved::Apply { head, args } if call_reaches_discharged_effect(db, head, ctx) => {
            // Thread state through the arguments FIRST (they evaluate before the call, in order), then
            // inline the callee and thread its (reduced) body.
            let mut cur = states;
            let mut rargs = Vec::with_capacity(args.len());
            for &a in args.iter() {
                let (ra, next) = thread_bounded(db, a, cur, ctx, inline_depth)?;
                rargs.push(ra);
                cur = next;
            }
            // A PARAMETERIZED callee β-reduces (substitute args for params); a NULLARY def `(gen)` has no
            // lambda wrapper — its name resolves straight to its body, so `apply_lambda` yields nothing
            // and we thread the body directly.
            let reduced = match crate::eval::apply_lambda(db, head, &rargs).ok().flatten() {
                Some(r) => r,
                None => crate::eval::lambda_body_of_nullary(db, head)?,
            };
            thread_bounded(db, reduced, cur, ctx, inline_depth + 1)
        }
        // An `(if cond then else)` — the condition is evaluated (thread state through it), then BOTH
        // branches see the post-condition state (only one runs, but each is rewritten under the same
        // incoming state — a perform in a branch reads that state). The branch STATES are not merged
        // (the corpus never observes post-`if` state across a perform-in-a-branch), so the `if`'s
        // out-state is the post-CONDITION state — sound for the single-return shape (a recursive call
        // in a branch takes the branch-local threaded state as its argument; nothing after the `if`
        // reads state). This is what threads countdown's `(if (= (tick) 0) 0 (+ 1 (loop)))`.
        Resolved::If { cond, then_, else_ } => {
            let (rcond, cur) = thread_bounded(db, cond, states, ctx, inline_depth)?;
            // Each BRANCH is threaded under the post-condition state. An ABORTIVE perform in a branch's
            // TAIL is LOCAL to that branch: the branch's value is the arm value (the abort discards up to
            // the handle, but the `if` IS the handle body's value, so per-branch the abort just yields the
            // arm value). Capture it per-branch — thread the branch, and if it set the whole-handle abort
            // cell, take that value as the branch's rewrite and CLEAR the cell so the OTHER branch (and the
            // handle) are not collapsed. (Sound when the `if` is in the handle's tail position — a NON-tail
            // conditional abort is declined by `body_has_unsound_abortive_perform` in `reduce_handle`
            // before threading, so an `if` reached here is safe to fold per-branch.)
            let rthen = thread_branch_local_abort(db, then_, cur.clone(), ctx, inline_depth)?;
            let relse = thread_branch_local_abort(db, else_, cur.clone(), ctx, inline_depth)?;
            let if_head = db.push_atom(Leaf::Name("if".to_string()));
            Some((db.push_list(vec![if_head, rcond, rthen, relse]), cur))
        }
        // A `(let ((n init)…) body)` — thread state through each initializer in order, then the body.
        // This is what threads range-sum's `(let ((i (Idx.next))) (if (= i 0) …))`: the init `(Idx.next)`
        // performs (reads state, threads next), and `i` binds that resume value. Rebuild the `let` with
        // the rewritten inits + body so `i`'s binding survives (the binder name is copied structurally).
        _ if db.ast.as_form(node, "let").is_some() => {
            let tail: Vec<StructId> = db.ast.as_form(node, "let").unwrap().to_vec();
            if tail.len() != 2 {
                return None;
            }
            let bindings_occ = tail[0];
            let body_occ = tail[1];
            let Struct::List(pairs) = db.ast.get(bindings_occ).clone() else {
                return None;
            };
            let mut cur = states;
            let mut rpairs = Vec::with_capacity(pairs.len());
            for pair in pairs {
                let Struct::List(kv) = db.ast.get(pair).clone() else {
                    return None;
                };
                if kv.len() != 2 {
                    return None;
                }
                // The binder NAME is copied structurally (a binder, not a value to thread); the INIT is
                // threaded (it may perform).
                let name_copy = copy_pure(db, kv[0]);
                let (rinit, next) = thread_bounded(db, kv[1], cur, ctx, inline_depth)?;
                cur = next;
                rpairs.push(db.push_list(vec![name_copy, rinit]));
            }
            let (rbody, cur) = thread_bounded(db, body_occ, cur, ctx, inline_depth)?;
            let let_head = db.push_atom(Leaf::Name("let".to_string()));
            let rbindings = db.push_list(rpairs);
            Some((db.push_list(vec![let_head, rbindings, rbody]), cur))
        }
        // A NESTED `handle` in the handled body. TWO ways it composes:
        //
        // (1) MERGED context (the two-nested-states case): the inner handle's body reaches a RECURSIVE
        //     callee that performs BOTH the inner AND an outer effect — so neither handler alone can fold
        //     it (the inside-out path below would leave the outer performs inside a specialization keyed on
        //     the inner effect only). Instead MERGE the inner handler's slot into the outer context (the
        //     union of arms, the inner slot APPENDED after the outer slots), push the inner `init` as the
        //     new slot's incoming state, and thread the inner body under the merged context. The recursive
        //     callee then specializes ONCE against the combined context — `f#ctx(args…, s_outer…, s_inner)`
        //     — threading each effect's state as its own trailing param (`DESIGN-effects-rcdzc.md` §4.3).
        //     After threading, the outer slots' states are the merged vector's PREFIX (the inner slot's
        //     final state is discarded — the handle's value is its body's value).
        // (2) INSIDE-OUT (the existing non-merged path): reduce the inner handle in isolation
        //     (`reduce_handle` discharges ITS effect, rewriting its performs to plain code), then thread the
        //     reduced result — which may still perform an outer effect — under the OUTER context. So
        //     `(handle_B … (handle_A … body))` folds `A` away first, leaving `B` performs for `B`'s fold.
        //     Used when the inner body does NOT reach a recursive callee performing an outer effect.
        Resolved::Handle {
            init: inner_init,
            arms: inner_arms,
            body: inner_body,
        } => {
            if let Some(merged) = merged_nested_ctx(db, &inner_arms, inner_body, ctx) {
                // Thread the inner body under the merged context, with the inner slot seeded by its init
                // (appended after the outer states). The merged vector = outer states ++ [inner init].
                let mut merged_states = states.clone();
                merged_states.push(inner_init);
                let (rbody, out) =
                    thread_bounded(db, inner_body, merged_states, &merged, inline_depth)?;
                // Drop the inner slot's final state; return the OUTER slots' states (the prefix).
                let outer_states = out[..states.len()].to_vec();
                Some((rbody, outer_states))
            } else {
                let reduced = reduce_handle(db, inner_init, &inner_arms, inner_body)?;
                // Thread the reduced result (which may still perform an outer effect) under the outer ctx.
                thread_bounded(db, reduced, states, ctx, inline_depth)
            }
        }
        // An ordinary application / arithmetic / comparison / connective / `not` over sub-expressions:
        // thread state through the operands in left-to-right order, rebuilding the same head. This
        // covers `(+ (E.op) 1)`, `(List.push s (E.op))`, etc. The head itself is not a perform (that
        // arm above caught it), so it is copied as-is.
        Resolved::Apply { head, args } => {
            let mut cur = states;
            let (rhead, next0) = thread_or_copy(db, head, cur, ctx, inline_depth)?;
            cur = next0;
            let mut children = vec![rhead];
            for &a in args.iter() {
                let (ra, next) = thread_bounded(db, a, cur, ctx, inline_depth)?;
                children.push(ra);
                cur = next;
            }
            Some((db.push_list(children), cur))
        }
        // A node that performs nothing — a literal, a bare reference, a param, unit, a type value, a
        // fully-non-effect subtree. It leaves the states unchanged. Copy it structurally so the rewritten
        // body is self-contained (a fresh occurrence re-resolving against the rewritten scope).
        _ if !subtree_performs(db, node, ctx) => {
            let copied = copy_pure(db, node);
            Some((copied, states))
        }
        // Some other form that DOES contain a perform but is not one of the shapes we thread (e.g. an
        // `if`/`match`/`let` with a perform inside — E1c-2/E3 territory). Decline.
        _ => None,
    }
}

/// Whether applying `head` (an application's head) REACHES an operation this handler discharges — the
/// new inline trigger (`DESIGN-effects-rcdzc.md` §3). True when `head` is a NON-RECURSIVE function
/// (a lambda / top-level def) whose body transitively performs a discharged operation. A recursive
/// callee is EXCLUDED (it cannot be inlined — E3 specializes it), as is a head that is not a function
/// (a perform head is caught by the perform arm; an operator/ctor reaches no effect). Bounded: the walk
/// follows non-recursive calls, and `is_recursive` gates re-entry.
fn call_reaches_discharged_effect(db: &mut Db, head: StructId, ctx: &HandlerCtx) -> bool {
    // The callee body, without reducing — a lambda body (parameterized def) OR a nullary def body (whose
    // name resolves straight to its body, no lambda wrapper). `None` for a non-function head (an
    // operator, a perform, a bare value).
    let Some(body) = crate::eval::lambda_body(db, head)
        .or_else(|| crate::eval::lambda_body_of_nullary(db, head))
    else {
        return false;
    };
    // A RECURSIVE callee cannot be inlined (it would not terminate) — exclude it (E3 specializes it).
    if crate::eval::is_recursive(db, body) {
        return false;
    }
    body_reaches_discharged(db, body, ctx, 0)
}

/// Whether the resolved subtree at `node` performs a discharged operation, following NON-RECURSIVE calls
/// into their callee bodies (up to a small depth bound — a cross-function chain past it is left to E3's
/// specialization / a clean decline). A syntactic perform of a discharged op is the base case.
fn body_reaches_discharged(db: &mut Db, node: StructId, ctx: &HandlerCtx, depth: u32) -> bool {
    // Depth backstop — a cross-function chain deeper than this declines (the inline trigger stays
    // bounded, mirroring the evaluator's reduction guard).
    if depth > 16 {
        return false;
    }
    // A syntactic perform of a discharged operation.
    if let Resolved::Apply { head, .. } = resolved_of(db, node)
        && is_perform(db, head, ctx).is_some()
    {
        return true;
    }
    // A call to a NON-RECURSIVE function whose body reaches a discharged op — follow it (a parameterized
    // OR a nullary-def callee).
    if let Resolved::Apply { head, .. } = resolved_of(db, node)
        && let Some(callee) = crate::eval::lambda_body(db, head)
            .or_else(|| crate::eval::lambda_body_of_nullary(db, head))
        && !crate::eval::is_recursive(db, callee)
        && body_reaches_discharged(db, callee, ctx, depth + 1)
    {
        return true;
    }
    // Otherwise descend into children structurally.
    match db.ast.get(node).clone() {
        Struct::List(children) => children
            .iter()
            .any(|&c| body_reaches_discharged(db, c, ctx, depth)),
        Struct::Atom(_) => false,
    }
}

/// Whether `head` names a RECURSIVE top-level def whose body reaches a discharged operation — the E3
/// specialization trigger (the recursive counterpart of `call_reaches_discharged_effect`, which excludes
/// recursion). Only a NAMED top-level def can be specialized (it needs a stable identity to synthesize
/// `f#ctx` from and to name the recursive call); a computed/anonymous recursive head is not.
fn recursive_call_reaches_discharged(db: &mut Db, head: &StructId, ctx: &HandlerCtx) -> bool {
    let head = *head;
    let Some(callee_def) = callee_def_index_of(db, head) else {
        return false;
    };
    let Some(body) = db.defs[callee_def].body else {
        return false;
    };
    let rec = crate::eval::is_recursive(db, body);
    let reaches = body_reaches_discharged(db, body, ctx, 0);
    rec && reaches
}

/// Whether a type contains an undetermined `Ty::Any` component (structurally). Used to reject a
/// specialization whose state type is not fully determined — an `Any` most often an empty-list seed's
/// element type — since a synthesized state-param annotation must be a definite type.
fn ty_has_any(ty: &crate::ty::Ty) -> bool {
    use crate::ty::Ty;
    match ty {
        Ty::Any => true,
        Ty::List(elem) => ty_has_any(elem),
        Ty::Map(k, v) => ty_has_any(k) || ty_has_any(v),
        Ty::Set(elem) => ty_has_any(elem),
        Ty::Tuple(elems) => elems.iter().any(ty_has_any),
        Ty::Record(fields) => fields.values().any(ty_has_any),
        Ty::Sum { args, .. } => args.iter().any(ty_has_any),
        Ty::Fn(p, r) => ty_has_any(p) || ty_has_any(r),
        _ => false,
    }
}

/// The `db.defs` index the application head `head` names — following a `Ref` to a lambda/def body. `None`
/// for a head that is not a named top-level def. (A local copy of `lower::callee_def_index`, which is
/// private there.)
fn callee_def_index_of(db: &mut Db, head: StructId) -> Option<usize> {
    match resolved_of(db, head) {
        Resolved::Lambda { body, .. } => db.def_index_by_body(body),
        Resolved::Ref { value } => db
            .def_index_by_body(value)
            .or_else(|| callee_def_index_of(db, value)),
        _ => None,
    }
}

/// Specialize the recursive effectful def `head` names UNDER this handler context — emit `f#ctx` once
/// (memoized on `db.effect_specializations` by `(body-occ, ctx.key)`), returning its synthesized NAME.
/// The specialized def takes `f`'s original parameters plus a trailing STATE parameter; its body is
/// `f`'s body threaded under `ctx` (each perform → its arm's resume value against the state param; the
/// recursive self-call → a call to `f#ctx` with the threaded next-state). `None` if `head` is not a
/// specializable recursive def or its body cannot be threaded (declines cleanly).
fn specialize_recursive(db: &mut Db, head: StructId, ctx: &HandlerCtx) -> Option<String> {
    let callee_def = callee_def_index_of(db, head)?;
    let orig_body = db.defs[callee_def].body?;
    if !ctx.has_state() {
        return None;
    }
    // Each slot's state TYPE must be FULLY DETERMINED to annotate its trailing state param. An
    // UNDETERMINED component (an `Any` — most commonly an empty-list seed `(list)`, whose element type is
    // `Ty::Any` until an operation pins it) or a MISSING slot type would bake a wrong/loose annotation
    // (`(: s (List Any))`) that mistypes the threaded body. Decline cleanly rather than emit it.
    let slot_tys: Vec<crate::ty::Ty> = ctx
        .slots
        .iter()
        .map(|s| s.state_ty.clone())
        .collect::<Option<Vec<_>>>()?;
    if slot_tys.iter().any(ty_has_any) {
        return None;
    }

    // MEMO: the same recursive def under the same handler context specializes ONCE. Keyed by the def's
    // body occurrence + the context's resolved identity. A hit returns the existing synthesized name.
    let memo_key = (orig_body, ctx.key.clone());
    if let Some(&idx) = db.effect_specializations.get(&memo_key) {
        return Some(db.defs[idx].name.clone());
    }

    // The original parameters. A NULLARY def (countdown `loop`, range-sum `sum-down`) has none; a
    // PARAMETERIZED def (`walk n`) carries its param name occurrences, threaded through unchanged (the
    // self-call passes its own args). Each is copied fresh into the specialized signature so the threaded
    // body's references to it re-resolve against the new def's scope. Only BARE-name params are handled
    // (an annotated `(: n T)` original param is a later increment); a non-bare param declines. We capture
    // each param's NAME and its SOLVED type (from the original def — `type_of` runs the connected
    // recursive-param solve), so the synthesized param can be ANNOTATED: a bare synthesized param has no
    // call site to flow a type from and no connected solve of its own reliably reaching it, so it would
    // stay `Any` and mistype the body (`(Diag.emit n)` needs `n: Int64`). A param whose original type is
    // undetermined (`Any`) declines — the specialization needs a definite param type to emit.
    let orig_params = db.defs[callee_def].params.clone();
    let mut orig_param_specs: Vec<(String, crate::ty::Ty)> = Vec::with_capacity(orig_params.len());
    for &p in &orig_params {
        let name = match db.ast.as_name(p) {
            Some(n) => n.to_string(),
            None => return None,
        };
        let ty = crate::infer::type_of(db, p);
        if matches!(ty, crate::ty::Ty::Any) {
            return None; // an undetermined original param — cannot annotate the synthesized copy
        }
        orig_param_specs.push((name, ty));
    }

    // The specialized NAME — unique per (def, context). The `#` makes it unspellable in source (no user
    // collision); the def-count suffix keeps distinct specializations distinct.
    let base = db.defs[callee_def].name.clone();
    let spec_name = format!("{base}#eff{}", db.defs.len());

    // Build the specialized def as a REAL AST form `(def (spec (: n Tn)… (: s0 Ts0) (: s1 Ts1)…) <body>)`,
    // so its parameters resolve (via `is_param_occurrence`, which walks to a `def` form) and each types by
    // its annotation. Every param — original AND each trailing STATE (one per handler slot, in slot order)
    // — is an ANNOTATED binder `(: name T)`. The state params come LAST, since the self-call appends the
    // slot states last (in slot order).
    let spec_name_atom = db.push_atom(Leaf::Name(spec_name.clone()));
    let mut sig_children = vec![spec_name_atom];
    for (n, ty) in &orig_param_specs {
        let name_atom = db.push_atom(Leaf::Name(n.clone()));
        let ty_expr = crate::eval::encode_typeval(db, ty);
        let colon = db.push_atom(Leaf::Name(":".to_string()));
        sig_children.push(db.push_list(vec![colon, name_atom, ty_expr]));
    }
    // The trailing state params — one per slot, named `{spec}$s{k}`, annotated with the slot's state type.
    let state_names: Vec<String> = (0..slot_tys.len())
        .map(|k| format!("{spec_name}$s{k}"))
        .collect();
    for (k, ty) in slot_tys.iter().enumerate() {
        let state_name = db.push_atom(Leaf::Name(state_names[k].clone()));
        let state_type_expr = crate::eval::encode_typeval(db, ty);
        let colon = db.push_atom(Leaf::Name(":".to_string()));
        sig_children.push(db.push_list(vec![colon, state_name, state_type_expr]));
    }
    let sig = db.push_list(sig_children.clone());
    let spec_params: Vec<StructId> = sig_children[1..].to_vec();

    // RESERVE the def NOW (with the sig, body filled later) + MEMOIZE — so the recursive self-call inside
    // the body re-enters `specialize_recursive`, hits the memo, and names THIS `spec_name`.
    let spec_index = db.defs.len();
    db.push_specialized_def(Def {
        name: spec_name.clone(),
        sig_occ: sig,
        params: spec_params.clone(),
        body: None,
    });
    db.effect_specializations.insert(memo_key, spec_index);

    // Thread `orig_body` under `ctx`, with each slot's incoming state = a REFERENCE to its state param. A
    // perform's resume value references the arm's state binder, which `thread`'s perform arm substitutes
    // with that slot's state expression; the recursive self-call re-enters and (via the memo) rewrites to
    // `(spec_name args… <threaded-states>)`. Each state name atom must re-resolve to its param, so we pass
    // FRESH occurrences of the names (bare `s{k}` references), not the binder occurrences.
    let state_refs: Vec<StructId> = state_names
        .iter()
        .map(|n| db.push_atom(Leaf::Name(n.clone())))
        .collect();
    let (spec_body, _out) = thread(db, orig_body, state_refs, ctx)?;

    // Wrap in a REAL `(def (spec params… (: s T)) spec_body)` arena node so the parent index links
    // param → sig → def: `is_param_occurrence` walks that chain to classify each param, and `binder_in`
    // Case 4 resolves a body reference against the def signature. Without this the synthesized params
    // would not resolve. The `db.defs` entry's `body` points at `spec_body` (the def-form node is for
    // scope/param resolution, not the emitted body — emission reads `db.defs[i].body`).
    let def_head = db.push_atom(Leaf::Name("def".to_string()));
    let _def_form = db.push_list(vec![def_head, sig, spec_body]);

    db.fill_specialized_def(spec_index, spec_params, spec_body);
    Some(spec_name)
}

/// Thread `node` if it performs, else copy it (a head that is a pure function reference). Convenience for
/// an application head.
fn thread_or_copy(
    db: &mut Db,
    node: StructId,
    states: Vec<StructId>,
    ctx: &HandlerCtx,
    inline_depth: u32,
) -> Option<(StructId, Vec<StructId>)> {
    if subtree_performs(db, node, ctx) {
        thread_bounded(db, node, states, ctx, inline_depth)
    } else {
        Some((copy_pure(db, node), states))
    }
}

/// If `head` is (resolves to) an effect operation this context discharges, its `(decl, index)` identity;
/// else `None`. A perform is an application whose head value carries a `(meta effect-op)` for an
/// operation in `ctx.arms`.
fn is_perform(db: &mut Db, head: StructId, ctx: &HandlerCtx) -> Option<(u32, u32)> {
    let (decl, idx) = crate::eval::effect_op_of(db, head)?;
    if ctx.arms.contains_key(&(decl.0, idx)) {
        Some((decl.0, idx))
    } else {
        None
    }
}

/// Whether the arm body `node` is a TAIL `(resume VALUE NEXT-STATE)` — returning `(value, next_state)`.
/// `None` if it is not a resume at the tail (an abortive arm with no resume, or a non-tail resume the
/// tail path cannot serve — those are E4/E5). The body is the ALREADY-SUBSTITUTED arm body, so the
/// value/next-state are concrete expressions.
fn tail_resume(db: &mut Db, node: StructId) -> Option<(StructId, StructId)> {
    match resolved_of(db, node) {
        Resolved::Resume { value, next_state } => Some((value, next_state)),
        _ => None,
    }
}

/// Whether the arm body at `node` contains a `resume` anywhere (structural walk). An arm with NO resume
/// is ABORTIVE (E4): performing it abandons the computation and yields the arm body's value. Used to
/// classify a `HandlerCtx`'s arms; the tail-resume EXTRACTION (bare or do-wrapped) is separate.
fn arm_has_resume(db: &mut Db, node: StructId) -> bool {
    if matches!(resolved_of(db, node), Resolved::Resume { .. }) {
        return true;
    }
    match db.ast.get(node).clone() {
        Struct::List(children) => children.iter().any(|&c| arm_has_resume(db, c)),
        Struct::Atom(_) => false,
    }
}

/// Whether `param` is the unit placeholder `()` (a nullary operation's single "parameter", which binds
/// nothing). `()` resolves to `Resolved::Unit`.
fn is_unit_param(db: &mut Db, param: StructId) -> bool {
    matches!(resolved_of(db, param), Resolved::Unit)
}

/// Whether the subtree at `node` performs an operation `ctx` discharges — a fast pre-check so a
/// perform-free subtree is copied wholesale rather than threaded position-by-position. Structural walk.
fn subtree_performs(db: &mut Db, node: StructId, ctx: &HandlerCtx) -> bool {
    if let Resolved::Apply { head, .. } = resolved_of(db, node)
        && is_perform(db, head, ctx).is_some()
    {
        return true;
    }
    // A `resume` reached here (outside an arm's tail rewrite) is treated as effectful so it is not
    // silently copied as pure.
    if matches!(resolved_of(db, node), Resolved::Resume { .. }) {
        return true;
    }
    // A CALL whose callee REACHES a discharged operation — cross-function (inlined) OR recursive
    // (specialized). Its performs are behind the call, not syntactic here, so without this a `(walk 3)`
    // whose `walk` performs the effect would be copied as "pure" and never threaded — leaving the perform
    // unhandled. Following the callee is what makes the fold thread INTO such a call (the inline /
    // specialization arms then handle it). This mirrors the two Apply arms' guards.
    if let Resolved::Apply { head, .. } = resolved_of(db, node)
        && (call_reaches_discharged_effect(db, head, ctx)
            || recursive_call_reaches_discharged(db, &head, ctx))
    {
        return true;
    }
    match db.ast.get(node).clone() {
        Struct::List(children) => children.iter().any(|&c| subtree_performs(db, c, ctx)),
        Struct::Atom(_) => false,
    }
}

/// Copy a perform-FREE subtree so it is self-contained in the rewritten body (a fresh occurrence
/// re-resolving against the rewritten scope). A constant leaf is shared; a name atom is copied fresh; a
/// list is copied with its children copied. (This is `beta_reduce` with an empty substitution — reused
/// so the copy discipline is identical.)
fn copy_pure(db: &mut Db, node: StructId) -> StructId {
    crate::eval::beta_reduce(db, node, &HashMap::default())
}

#[cfg(test)]
mod desugar_tests {
    use super::*;
    use crate::testkit::parse;

    /// The CANONICAL handle `(handle E seed (bare-arm…) body)` desugars to the INTERNAL
    /// `(handle seed ((. E op)-arm…) body)` the resolver consumes: `E` leaves the head and each arm's
    /// bare op becomes its `(. E op)` projection, with params/state/body preserved.
    #[test]
    fn desugars_canonical_handle_to_internal_shape() {
        let mut ast = parse("(handle Fresh 0 ((next (u) s (resume s (+ s 1)))) (Fresh.next))");
        desugar_handles(&mut ast);
        // The root is now the internal 4-child handle: (handle 0 (arm…) body).
        let tail = ast.as_form(ast.root, "handle").expect("still a handle");
        assert_eq!(
            tail.len(),
            3,
            "internal handle = seed (arms) body (head excluded)"
        );
        // tail[0] = seed (the int 0, no longer the effect name).
        assert_eq!(
            ast.as_name(tail[0]),
            None,
            "first child is the seed, not the effect"
        );
        let Struct::List(arms) = ast.get(tail[1]) else {
            panic!("arms list")
        };
        let Struct::List(arm0) = ast.get(arms[0]) else {
            panic!("one arm")
        };
        assert_eq!(arm0.len(), 4, "arm = op-proj (params) state body");
        // The op is now the projection (. Fresh next).
        let proj = ast
            .as_form(arm0[0], ".")
            .expect("op rewritten to a `.` projection");
        assert_eq!(ast.as_name(proj[0]), Some("Fresh"));
        assert_eq!(ast.as_name(proj[1]), Some("next"));
    }

    /// A handle ALREADY in internal shape (4 children, dotted arm op) is left untouched — so a
    /// hand-authored internal-shape program still compiles unchanged.
    #[test]
    fn leaves_internal_shape_handle_untouched() {
        let mut ast = parse("(handle 0 (((. Fresh next) (u) s (resume s (+ s 1)))) (Fresh.next))");
        let before = ast.structure.len();
        desugar_handles(&mut ast);
        assert_eq!(
            ast.structure.len(),
            before,
            "no nodes appended for an internal-shape handle"
        );
        let tail = ast.as_form(ast.root, "handle").unwrap();
        assert_eq!(tail.len(), 3);
    }
}
