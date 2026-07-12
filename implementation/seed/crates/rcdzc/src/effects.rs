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
use crate::db::{Db, EffectDecl, OpDecl};
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
    /// Each discharged operation's `(decl-occ, op-index)` → the arm that discharges it.
    arms: HashMap<(u32, u32), HandleArm>,
}

/// Reduce a `(handle init arms body)` to a rewritten BODY occurrence with every perform of a discharged
/// operation resolved to its arm and rewritten tail-resumptively, or `None` to DECLINE (the case is not
/// in the tail-resumptive shipping surface — `lower` then declines cleanly, a Todo). `init`/`arms`/`body`
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
            if let Some(callee) =
                crate::eval::lambda_body(db, head).or_else(|| crate::eval::lambda_body_of_nullary(db, head))
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
            let added: Vec<u32> = effects
                .iter()
                .filter_map(|&e| {
                    // Each `effect` element is a name occurrence resolving to the effect record; recover
                    // its decl via the record's `(meta t)` = `(effect NAME <decl>)`.
                    host_effect_decl(db, e)
                })
                .collect();
            let before = handled.len();
            handled.extend(&added);
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
    let ctx = HandlerCtx { arms: map };
    // Thread the INIT state through the body in evaluation order. The handle's value is the body's
    // value (the accumulated state is observable only through the operations), so we return the
    // rewritten body; the final threaded state is discarded (the body never reads it directly).
    let (rewritten, _final_state) = thread(db, body, init, &ctx)?;
    Some(rewritten)
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

/// Rewrite `node` under handler context `ctx`, threading `state` (the current-state expression) through
/// it in EVALUATION ORDER. Returns `(rewritten-node, next-state)` — the node with performs resolved and
/// the state as it stands AFTER `node` evaluates — or `None` to decline (a shape not provably
/// tail-resumptive). `state` is an arena occurrence (an expression), substituted into an arm's `state`
/// binder when a perform fires.
fn thread(
    db: &mut Db,
    node: StructId,
    state: StructId,
    ctx: &HandlerCtx,
) -> Option<(StructId, StructId)> {
    match resolved_of(db, node) {
        // A PERFORM `(E.op args…)` of a discharged operation: resolve to its arm, substitute the arm's
        // params ↦ (rewritten) args and its state binder ↦ current state, and rewrite the arm body's
        // TAIL resume to the resume VALUE, threading the resume's next-STATE forward.
        Resolved::Apply { head, args } if is_perform(db, head, ctx).is_some() => {
            let (decl, idx) = is_perform(db, head, ctx).unwrap();
            let arm = ctx.arms.get(&(decl, idx))?.clone();
            // Thread state through each argument left-to-right (an argument may itself perform).
            let mut cur = state;
            let mut rewritten_args = Vec::with_capacity(args.len());
            for &a in args.iter() {
                let (ra, next) = thread(db, a, cur, ctx)?;
                rewritten_args.push(ra);
                cur = next;
            }
            // The arm binds its params to the args and its state binder to the CURRENT state. Substitute
            // both into the arm body (a capture-safe arena substitution), then extract the tail resume.
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
            subst.insert(arm.state, cur);
            let arm_body = crate::eval::beta_reduce(db, arm.body, &subst);
            // The arm body must be a TAIL `(resume value next-state)` — the value becomes the perform's
            // result; the next-state threads forward. Anything else (no resume / non-tail) declines.
            let (value, next_state) = tail_resume(db, arm_body)?;
            Some((value, next_state))
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
            let mut cur = state;
            let mut last = None;
            for it in items {
                let (r, next) = thread(db, it, cur, ctx)?;
                last = Some(r);
                cur = next;
            }
            Some((last.unwrap(), cur))
        }
        // A CROSS-FUNCTION perform: `(f args…)` where `f` is a NON-RECURSIVE function whose body reaches
        // an operation this handler discharges (`DESIGN-effects-rcdzc.md` §3, the new inline trigger). The
        // handler must be present in the callee, so INLINE `f` into the handled region — β-reduce the call
        // (substitute args for params) and thread state through the reduced body, exactly as if the
        // callee's body were written inline. This is what makes `(handle … (gen))` work when `gen`
        // performs the discharged op. A RECURSIVE such callee cannot be inlined (E3 specializes it); it
        // is not caught here (the reachability check excludes a recursive callee), so it declines below.
        Resolved::Apply { head, args } if call_reaches_discharged_effect(db, head, ctx) => {
            // Thread state through the arguments FIRST (they evaluate before the call, in order), then
            // inline the callee and thread its (reduced) body.
            let mut cur = state;
            let mut rargs = Vec::with_capacity(args.len());
            for &a in args.iter() {
                let (ra, next) = thread(db, a, cur, ctx)?;
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
            thread(db, reduced, cur, ctx)
        }
        // An ordinary application / arithmetic / comparison / connective / `not` over sub-expressions:
        // thread state through the operands in left-to-right order, rebuilding the same head. This
        // covers `(+ (E.op) 1)`, `(List.push s (E.op))`, etc. The head itself is not a perform (that
        // arm above caught it), so it is copied as-is.
        Resolved::Apply { head, args } => {
            let mut cur = state;
            let (rhead, next0) = thread_or_copy(db, head, cur, ctx)?;
            cur = next0;
            let mut children = vec![rhead];
            for &a in args.iter() {
                let (ra, next) = thread(db, a, cur, ctx)?;
                children.push(ra);
                cur = next;
            }
            Some((db.push_list(children), cur))
        }
        // A node that performs nothing — a literal, a bare reference, a param, unit, a type value, a
        // fully-non-effect subtree. It leaves the state unchanged. Copy it structurally so the rewritten
        // body is self-contained (a fresh occurrence re-resolving against the rewritten scope).
        _ if !subtree_performs(db, node, ctx) => {
            let copied = copy_pure(db, node);
            Some((copied, state))
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
    let Some(body) = crate::eval::lambda_body(db, head).or_else(|| crate::eval::lambda_body_of_nullary(db, head)) else {
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
        && let Some(callee) =
            crate::eval::lambda_body(db, head).or_else(|| crate::eval::lambda_body_of_nullary(db, head))
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

/// Thread `node` if it performs, else copy it (a head that is a pure function reference). Convenience for
/// an application head.
fn thread_or_copy(
    db: &mut Db,
    node: StructId,
    state: StructId,
    ctx: &HandlerCtx,
) -> Option<(StructId, StructId)> {
    if subtree_performs(db, node, ctx) {
        thread(db, node, state, ctx)
    } else {
        Some((copy_pure(db, node), state))
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
