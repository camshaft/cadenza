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
//!       when the operation is applied, so a performance `(Diag.emit code)` checks `code` against the
//!       declared parameter type and yields the declared result type — typed exactly as an ordinary
//!       function application, a perform-argument mismatch being an ordinary type error
//!       (`capabilities-and-effects.md` §Performing An Operation Is Typed).
//= spec/capabilities/capabilities-and-effects.md#performing-an-operation-is-typed-and-contributes-to-the-row
//# Performing an operation MUST check its arguments against the operation's declared parameter types and yield the operation's declared result type, so that an effect operation is typed exactly as an ordinary function application is.
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
//! An `(effect NAME (op f (-> A B)) …)` names the effect and binds each of its operations to an
//! operation type, so an effect's operation set is a CLOSED, statically-known set of fields (not an open
//! collection of ad-hoc names). Each operation is reached only THROUGH its declaring effect record
//! (`Diag.emit` is member access off `Diag`), keyed by the declaration occurrence, so two effects may
//! declare a same-named operation without collision and every performance names an unambiguous op.
//= spec/capabilities/capabilities-and-effects.md#an-effect-declaration-names-the-effect-and-types-its-operations
//# A program MUST be able to declare an effect that names it and binds each of its operations to an operation type, so that the set of operations an effect offers is a closed, statically-known set rather than an open collection of ad-hoc names.
//= spec/capabilities/capabilities-and-effects.md#an-effect-declaration-names-the-effect-and-types-its-operations
//# An operation MUST be reached through its declaring effect, so that two effects may each declare an operation of the same name without collision and the effect an operation belongs to is unambiguous at every performance and every handler arm.
//!
//! The synthesized record is ROUTING-AGNOSTIC: it binds operation names to types and identities but
//! carries NO host binding, so declaring (or performing) an effect grants no capability on its own — a
//! reached operation with no enclosing handler and no entrypoint delegation declines (the no-home
//! check), and authority enters only where an entrypoint delegates. A library that declares or performs
//! an effect therefore cannot enlarge the authority of a program that uses it.
//= spec/capabilities/capabilities-and-effects.md#an-entrypoint-delegates-the-capabilities-it-grants-to-the-host
//# Declaring an effect and its operations MUST NOT by itself grant any host capability: an effect declaration is a routing-agnostic contract, and only an entrypoint's delegation routes an effect's operations to the host, so that a library that declares or performs an effect cannot enlarge the authority of a program that uses it.
//= spec/capabilities/capabilities-and-effects.md#host-binding-is-a-routing-decision-made-at-the-entrypoint
//# An effect declaration MUST NOT determine whether the effect is discharged in-program or at the host boundary, so that an effect is a routing-agnostic contract and the same declared effect may be handled in one program and delegated to the host in another.
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

/// The head of the INTERNAL handler node the desugar produces — DISTINCT from the source keyword
/// `handle`, so the two shapes never share a spelling. `handle` (source) is EXCLUSIVELY the canonical
/// 5-child `(handle E seed (bare-op-arm…) body)`; this internal 4-child `(handle-internal seed
/// ((. E op)-arm…) body)` is what `resolve`/`infer`/`lower`/`compile` consume, and it can only ever be
/// produced by this pass — a source program cannot write it (a `-` makes it un-lexable as a bare token,
/// and it is not a keyword), so there is exactly ONE way to write a handler.
pub(crate) const HANDLE_INTERNAL: &str = "handle-internal";

/// Desugar every CANONICAL handler form `(handle E seed (arm…) body)` — where the effect `E` and the
/// initial `seed` are PROMOTED into the head and each arm's operation is written BARE (`(op (p…) state
/// body)`) — into the INTERNAL form the rest of the compiler consumes: `(handle-internal seed (arm'…)
/// body)` with `E` dropped from the head, the head RE-SPELLED [`HANDLE_INTERNAL`], and each arm's op
/// rewritten to its `(. E op)` projection. This lets the surface (both s-expr and ML) name the effect
/// once, on the `handle`, while `resolve_handle` / `effects` / infer / lower / compile keep reading the
/// projection-per-arm shape unchanged — under a head only this pass produces.
///
/// Runs at load BEFORE the parent index is built, so the rewritten `(. E op)` projections resolve like
/// hand-written member access. Mutates the arena IN PLACE (swapping a handle node's children vector),
/// mirroring `accum::introduce` / `binding_params::lower`.
///
/// The canonical shape also carries the language RULE that a `handle` discharges exactly ONE effect:
/// its head names that effect, and every arm is one of that effect's operations. That is checked
/// downstream — an arm op `(. E op)` that `E` does not declare is the ordinary undeclared-operation
/// rejection (CDZ0403) — so this pass only performs the mechanical rewrite. A source form still headed
/// `handle` AFTER this pass is NOT the canonical shape (an effect-name-less legacy handle, or a
/// malformed/too-short one); it is left untouched here and REJECTED downstream (`resolve_handle`) —
/// there is one canonical way to write a handler, and the old effect-name-less shape is no longer it.
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
    /// The SOURCE effect-name occurrence (the handle head's `items[1]`). The FIRST arm's projection
    /// reuses it (rather than a fresh minted atom) so the effect name keeps a source span — an UNBOUND
    /// effect (`handle Nope …`) then anchors its CDZ0101 to the real `Nope` token instead of a spanless
    /// synthesized atom. Remaining arms mint fresh atoms (each projection needs its own occurrence for
    /// independent parent-walk anchoring).
    effect_occ: StructId,
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
        effect_occ: items[1],
        arms: arm_nodes.clone(),
    })
}

/// Apply a [`HandlePlan`]: swap each arm's bare op for a `(. E op)` projection IN PLACE and drop the
/// effect from the handle head. Both the arm nodes and the handle node keep their `StructId`s (and thus
/// their source spans); only the projection nodes are freshly appended.
fn apply_handle_plan(ast: &mut Arenas, plan: HandlePlan) {
    for (i, arm) in plan.arms.iter().enumerate() {
        let arm = *arm;
        // The arm's current bare op (its first child) — replace it with `(. E op)`. `op` is REUSED (its
        // own occurrence carries the arm's op-name span). For `E`: the FIRST arm REUSES the source
        // effect-name occurrence (so an unbound effect keeps its span — a CDZ0101 anchors to the real
        // token, M31), the REST mint a fresh atom each (a projection needs its own occurrence for
        // independent parent-walk anchoring). The dropped head effect-name slot is otherwise orphaned, so
        // re-parenting it here also keeps it reachable.
        let Struct::List(parts) = ast.get(arm) else {
            continue;
        };
        let op = parts[0];
        let rest: Vec<StructId> = parts[1..].to_vec();
        let dot = push_atom(ast, Leaf::Name(".".to_string()));
        let eff = if i == 0 {
            plan.effect_occ
        } else {
            push_atom(ast, Leaf::Name(plan.effect_name.clone()))
        };
        let proj = push_list(ast, vec![dot, eff, op]);
        let mut new_children = vec![proj];
        new_children.extend(rest);
        ast.structure[arm.0 as usize] = Struct::List(new_children);
    }
    // Rewrite the head to `handle-internal` and drop the effect (index 1): [handle, effect, seed, arms,
    // body] -> [handle-internal, seed, arms, body]. The node keeps its id/span; only the head atom and
    // the removed effect change. The re-spelled head is what marks this the desugared internal form —
    // distinct from a leftover source `handle`, which downstream rejects.
    if let Struct::List(items) = ast.get(plan.handle) {
        let mut kept = items.clone();
        if kept.len() == 5 {
            kept.remove(1);
            kept[0] = push_atom(ast, Leaf::Name(HANDLE_INTERNAL.to_string()));
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
// A performance in BODY is discharged by the ENCLOSING handler active in dynamic extent (a perform its
// caller wraps, or the same code under two handlers, is discharged by each in turn), and WHICH handler
// is fixed STATICALLY here — by monomorphizing the enclosing handler context over the closed effect row
// into a compile-time constant — so handler resolution is dynamic in extent yet a deterministic function
// of the source with no runtime handler search:
//= spec/capabilities/capabilities-and-effects.md#handler-resolution-is-dynamic-in-extent-and-statically-determined
//# A raised effect operation MUST be discharged by the nearest handler enclosing it in dynamic extent — the nearest handler active along the run's call chain, not the nearest handler lexically enclosing the performing function's definition — so that a function may perform an operation its caller discharges and the same function called under two different handlers is discharged by each in turn.
//= spec/capabilities/capabilities-and-effects.md#handler-resolution-is-dynamic-in-extent-and-statically-determined
//# Which handler discharges each performance MUST be determined statically at compile time by monomorphizing the enclosing handler context over the closed effect row, so that handler resolution is dynamic in extent yet a deterministic function of the source (constitution III) with no runtime handler search.
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
    /// The self-call temps accumulated while threading a leaf TAIL expression in multi-value mode: each
    /// `(temp-name, spec-call)` becomes a `(let ((temp spec-call)) …)` wrapping the tail's tuple, innermost
    /// = last-pushed (so a later temp's init may read an earlier temp's `(. t 1)` out-state). Drained by
    /// `thread_returning_tuple` at each leaf. Interior-mutable (the `&ctx` walk pushes to it).
    pending: std::cell::RefCell<Vec<(String, StructId)>>,
    /// A monotonic counter for fresh self-call temp names (`{spec}$t{k}`), unique within one specialization.
    temp_ctr: std::cell::Cell<u32>,
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
            pending: std::cell::RefCell::new(Vec::new()),
            temp_ctr: std::cell::Cell::new(0),
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
    inner_init: StructId,
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
    // The inner slot's state type — seeded by the inner handle's INIT type and joined with each arm's
    // resume next-state type. Using the INIT as the seed is load-bearing: an arm that RE-THREADS its bound
    // state `resume(v, s)` has `next-state = s`, and `type_of` of the bare state binder alone is `Ty::Any`
    // (the binder carries no standalone type — its type is the seed's). Deriving the slot type from the
    // arms' next-states ONLY (the old `inner_state_ty_from_arms`) then yielded `Any` and DECLINED the merge,
    // so a stateful inner handler under a nested context (`handle Model … in handle Tools(0) … | step(a,s)
    // => resume(a, s)`) failed to fold while the same handler STANDALONE folded (single-handler
    // `reduce_handle` already seeds from the init via `state_ty_of_arms`). Reusing `state_ty_of_arms` here
    // makes the merged path seed identically — the init `Tools(0)` pins the slot to `Int64` regardless of
    // whether an arm re-threads `s` or hands back a fresh value. (Reported by v-agent-harness Inc-2.)
    let inner_state_ty = state_ty_of_arms(db, inner_init, inner_arms);
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

/// The KEY occurrence of a handler-arm op projection `(. E k)` — the `k` child, which carries the arm's
/// op-NAME source span (the desugar REUSES the surface op-name occurrence for `k`, `apply_handle_plan`).
/// The projection `(. E k)` itself is a node the desugar freshly SYNTHESIZES (spanless), so a diagnostic
/// anchored to the projection maps to no source text; anchoring to this key occ instead gives the arm's
/// op a real `file:line:col`. `None` if `op` is not a `(. …)` form with a key child.
pub fn arm_op_key_occ(db: &Db, op: StructId) -> Option<StructId> {
    db.ast.as_form(op, ".").and_then(|t| t.get(1).copied())
}

/// If a handler arm's op projection `(. E k)` has a head `E` that names a VALUE DEFINITION rather than an
/// effect — `(handle foo 0 …)` where `foo` is a `(def foo …)` — the operand occurrence `E`. A `handle`'s
/// HEAD must name an effect (the arms ARE that effect's operations); a value head is a malformed handle,
/// but with the head desugared into `(. E k)` projections it surfaces as a leaky cascade ("member access
/// requires a record, found Int64" from `(. foo k)`, plus an uncoded fold-decline) instead of naming the
/// real problem. `None` when the head is an effect, an UNBOUND name (the resolver's own CDZ0101 is
/// primary — mirrors the host-delegation check's conservatism), or `op` is not a projection. CONSERVATIVE
/// like `check_no_home`'s host-delegation check: flags ONLY a head unambiguously bound to a value def
/// (`def_by_name`), never a nested-module effect (absent from the top-level registry).
pub fn arm_op_head_names_a_value(db: &mut Db, op: StructId) -> Option<(StructId, &'static str)> {
    let Resolved::Member { operand, .. } = resolved_of(db, op) else {
        return None;
    };
    // Already a real effect head → fine.
    if effect_decl_of_value(db, operand).is_some() {
        return None;
    }
    // Flag a head that is unambiguously a top-level VALUE def or a TYPE (never a nested-module effect /
    // unbound name). Both are the "handle head is not an effect" root cause — a value head leaks "member
    // access requires a record" from the arm's `(. head op)`, a type head leaks "record has no field `op`"
    // (a sum's variants are its fields) plus the fold-decline; naming the CATEGORY says what to fix.
    let name = db.ast.as_name(operand)?.to_string();
    if db.def_by_name(&name).is_some() {
        Some((operand, "a value definition"))
    } else if db.type_decl_by_name(&name).is_some() {
        Some((operand, "a type"))
    } else if crate::eval::typeval_of(db, operand).is_some() {
        // A PRELUDE type-value head — `(handle Int64 …)`, `(handle Option …)`. `typeval_of` recognizes any
        // type-value GENERICALLY (a prelude scalar/collection/ctor, no hard-coded name list — the
        // no-keys-outside-the-prelude rule), so a prelude type is classified the same as a user `(type …)`
        // head above. Without this, a prelude-type head is neither a `def_by_name` nor a
        // `type_decl_by_name` (a prelude type has no user decl), so it slipped through to the leaky
        // "not yet reducible by the tail-resumptive fold" fold-decline instead of naming the real problem.
        Some((operand, "a type"))
    } else {
        None
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

/// The TWO-TIER hint for a handler arm whose op `(. E k)` names an operation `E` does not declare — the
/// effect-op analogue of `no_field_reject`'s member-access enrichment. Returns `(key-occurrence, hint,
/// confident-single)`: `hint` is the `did_you_mean` suffix (a confident `` — did you mean `k`? `` when a
/// declared op is a plausible typo, ELSE `` — closest matches: `a`, `b` `` listing the effect's declared
/// operations — a CLOSED, small set, so listing is signal); `confident-single` is `Some(op)` ONLY when
/// there is a confident typo, driving the REPLACE fix (a tier-2 list is not one mechanical edit). `None`
/// if `op` is not `(. E k)` on an effect, or the effect declares no operations. Unlike `nearest_declared_op`
/// (which yields nothing on a far miss), this always produces a hint when the effect has ops to list.
pub fn declared_op_hint(db: &mut Db, op: StructId) -> Option<(StructId, String, Option<String>)> {
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
    if names.is_empty() {
        return None;
    }
    let key_occ = db.ast.as_form(op, ".").and_then(|t| t.get(1).copied())?;
    let single = crate::diag::suggest::nearest(&key.name, names.iter().map(String::as_str));
    let hint = crate::diag::suggest::did_you_mean(&key.name, names.iter().map(String::as_str), 3);
    Some((key_occ, hint, single))
}

/// The CANONICAL identity of a handler arm's operation `(. E k)` — `(effect-declaration-occurrence,
/// op-name)`. Two arms discharge the SAME operation exactly when their identities are equal, so this is
/// the key a duplicate-arm check dedups on. Keyed by the effect's DECLARATION (not just the name) so two
/// effects each declaring `emit` never collide — the same closed-set identity `handler_missing_operations`
/// and the reduction plan use. `None` if `op` is not `(. E k)` on an effect (an undeclared/malformed arm,
/// whose own fault CDZ0403 is reported instead). The op key's OCCURRENCE (for a delete fix's anchor) is
/// read separately via `arm_op_key_occ`.
pub fn arm_op_identity(db: &mut Db, op: StructId) -> Option<(u32, String)> {
    let Resolved::Member { operand, key } = resolved_of(db, op) else {
        return None;
    };
    let decl = effect_decl_of_value(db, operand)?;
    Some((decl, key.name.to_string()))
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

/// For a handler arm whose op names a DECLARED operation, whether the arm binds the WRONG number of
/// parameter binders — the arm-arity mismatch. Returns `(op_name, expected_description, actual)` when
/// `arm.params`'s length is not an accepted binder count for the operation's declared `(-> P… R)` type,
/// else `None`. An arm that binds too few parameters was SILENTLY ACCEPTED (the fold substituted an
/// unbound-or-defaulted binder); one that binds too many surfaced only the leaky "not yet reducible by
/// the tail-resumptive fold" decline — neither said the arm's parameter count is wrong. This is the
/// handler-arm analogue of a function applied at the wrong arity.
///
/// ⚠ The ELIDED-UNIT convention: a `(-> Unit R)` operation accepts BOTH a 0-binder arm (`(op () s …)`,
/// unit elided) AND a 1-binder arm (`(op (u) s …)`, unit bound explicitly) — the corpus uses both — so a
/// unit op's accepted set is `{0, 1}`, and only an arm outside it (2+ binders) is a mismatch. A genuine
/// N-parameter op (`N ≥ 1`, no elided unit) requires EXACTLY N. `op_arm_arity` gives the elided count
/// (0 for a unit op); the raw parameter count (1 for a unit op) is the other accepted value.
///
/// Returns `None` for an UNDECLARED op (its own CDZ0403 is the fault to report) or a malformed op with no
/// declared type.
pub fn arm_param_arity_mismatch(db: &mut Db, arm: &HandleArm) -> Option<(String, String, usize)> {
    let (decl, index) = crate::eval::effect_op_of(db, arm.op)?;
    let op_name = match resolved_of(db, arm.op) {
        Resolved::Member { key, .. } => key.name.clone(),
        _ => return None,
    };
    let eff = db.effect_decl_by_occ(decl)?;
    let ty = eff.ops.get(index as usize)?.ty?;
    let elided = op_arm_arity(db, ty); // 0 for a `(-> Unit R)` op, else the parameter count.
    let raw = raw_param_count(db, ty); // 1 for a `(-> Unit R)` op — the unit-bound-explicitly spelling.
    let actual = arm.params.len();
    // A unit op (`elided == 0 && raw == 1`) accepts both spellings; every other op requires exactly its
    // parameter count (`elided == raw`).
    if actual == elided || actual == raw {
        return None;
    }
    // The message describes the accepted count(s): "0 or 1" for a unit op, otherwise the single count.
    let expected = if elided == raw {
        elided.to_string()
    } else {
        format!("{elided} or {raw}")
    };
    Some((op_name, expected, actual))
}

/// The RAW parameter count of an operation's arrow `(-> P… R)` — the number of parameter positions
/// before the result, WITHOUT the elided-unit collapse `op_arm_arity` applies. For `(-> Unit R)` this is
/// 1 (the explicit `Unit` position), for `(-> R)` it is 0, for `(-> A B R)` it is 2.
fn raw_param_count(db: &Db, ty: StructId) -> usize {
    let Some(tail) = db.ast.as_form(ty, "->") else {
        return 0;
    };
    if tail.len() <= 1 { 0 } else { tail.len() - 1 }
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
//= spec/capabilities/capabilities-and-effects.md#host-binding-is-a-routing-decision-made-at-the-entrypoint
//# The concrete form by which an entrypoint delegates a set of effects to the host MUST be pinned at the declared-default location and MUST resolve an operation it delegates exactly as the nearest enclosing handler would, so that host delegation is the boundary member of the same nearest-enclosing resolution as in-program handling and two builds agree on the surface a delegation takes.
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
    // The delegation SET is a pure function of the export bodies, but this is a FALLBACK consulted once per
    // residual host-perform. Keying a cache by `decl` was still O(N²) for N DISTINCT delegated effects:
    // each decl missed once → N full export-body walks. Instead materialize the WHOLE set in ONE walk on
    // first query, then answer every query (including for effects NOT delegated) by O(1) membership.
    if db.delegated_effects.is_none() {
        let export_bodies: Vec<StructId> = db
            .exports
            .iter()
            .filter_map(|e| e.def.and_then(|d| db.defs[d].body))
            .collect();
        let mut set = crate::fxhash::FxHashSet::default();
        for b in export_bodies {
            collect_host_delegated(db, b, 0, &mut set);
        }
        db.delegated_effects = Some(set);
    }
    db.delegated_effects.as_ref().unwrap().contains(&decl)
}

/// Collect into `out` every effect-declaration occurrence delegated by a `(host (E…) …)` in the subtree at
/// `node` — the set-building twin of the old per-`decl` `body_has_host_delegating` probe, run ONCE over the
/// export bodies so N distinct delegated effects cost one walk, not N. A structural walk (bounded); a
/// `host` node's effect list contributes its decls, then the walk descends every child.
fn collect_host_delegated(
    db: &mut Db,
    node: StructId,
    depth: u32,
    out: &mut crate::fxhash::FxHashSet<crate::ast::StructId>,
) {
    if depth > 128 {
        return;
    }
    if let Resolved::Host { effects, .. } = resolved_of(db, node) {
        for e in effects.iter() {
            if let Some(decl) = effect_decl_of_host_name(db, *e) {
                out.insert(decl);
            }
        }
    }
    if let Struct::List(children) = db.ast.get(node).clone() {
        for c in children {
            collect_host_delegated(db, c, depth + 1, out);
        }
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
    // Dedup CALLEE-BODY re-walks: a nullary/pure helper called from N sites (`(mk)` × N in a wide record
    // projection) would otherwise have its whole body re-walked once PER call site → O(sites × body) =
    // O(N²). Keying on `(callee_body, handled-set)` is SOUND — a callee walked under an identical handled
    // set yields identical CDZ0401s (a re-walk is redundant), while a DIFFERENT handled set (an effect
    // granted at one call site, ungranted at another) is a distinct key and IS re-walked. Mirrors the
    // sibling `body_reached_effects_walk`'s `visited` guard, which this walk was missing.
    let mut followed: crate::fxhash::FxHashSet<(StructId, u64)> =
        crate::fxhash::FxHashSet::default();
    // `node` is the ENTRYPOINT body — the node a host-delegation fix wraps (`(host (E) <body>)`), which is
    // constant across the walk (a perform deep in the body, or in a called function, still delegates at
    // the entrypoint). Thread it through unchanged so the CDZ0401 fix can anchor its wrap there.
    check_no_home_walk(db, node, node, &mut handled, &mut followed, out, 0);
}

/// A hash of the currently-HANDLED effect-decl set — the second component of the callee-follow dedup key.
/// The handled set is a small `Vec<u32>` grown/truncated as `handle`/`host` frames are entered/left; its
/// CONTENTS (not order — a handled set is a set) determine a callee's CDZ0401 verdict, so hash the sorted
/// unique decls. Cheap: the set is tiny (one entry per enclosing handler/delegation, typically 0).
fn handled_key(handled: &[u32]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut v: Vec<u32> = handled.to_vec();
    v.sort_unstable();
    v.dedup();
    let mut h = crate::fxhash::FxHasher::default();
    v.hash(&mut h);
    h.finish()
}

fn check_no_home_walk(
    db: &mut Db,
    node: StructId,
    entrypoint: StructId,
    handled: &mut Vec<u32>,
    followed: &mut crate::fxhash::FxHashSet<(StructId, u64)>,
    out: &mut Vec<crate::diag::Reject>,
    depth: u32,
) {
    if depth > 64 {
        return; // backstop — a deep call chain is left to the ordinary decline
    }
    #[cfg(test)]
    crate::db::CHECK_NO_HOME_VISITS.with(|c| c.set(c.get() + 1));
    match resolved_of(db, node) {
        // A PERFORM `(E.op args…)`: if its effect is not currently handled/delegated, it has no home.
        Resolved::Apply { head, args } => {
            if let Some((decl, _idx)) = crate::eval::effect_op_of(db, head) {
                if !handled.contains(&decl.0) {
                    // The mechanical repair is the host-delegation route the message names: WRAP the
                    // entrypoint body in `(host (E) <body>)`, which grants the effect at the boundary and
                    // clears the fault (verified in one shot). The effect's name comes from its
                    // declaration (`decl`), derived not hard-coded (`spec/capabilities/diagnostics.md` §A
                    // Diagnostic Carries A Route To A Fix). Heuristic: delegating at the entrypoint is ONE
                    // of the two routes (the other is adding a `handle` with real arms — a semantic choice
                    // the compiler must not make for the author); the wrap resolves the fault but whether
                    // the author meant to delegate vs. handle is theirs to decide. The reject anchors at
                    // the PERFORM site (where the fault is), while the wrap targets the entrypoint body.
                    let mut reject = crate::diag::Reject::coded(
                        crate::diag::Code::EffectNoHome,
                        "this effect operation is reached with neither an enclosing handler nor a \
                         host delegation, so it has no home (add a handler or delegate it at the \
                         entrypoint)",
                    )
                    .at(node);
                    if let Some(eff) = db.effect_decl_by_occ(decl) {
                        let name = eff.name.clone();
                        reject = reject.with_fix(crate::diag::Fix::wrap_heuristic(
                            entrypoint,
                            format!("(host ({name}) "),
                            ")",
                            format!("delegate `{name}` at the entrypoint with `(host ({name}) …)`"),
                        ));
                    }
                    out.push(reject);
                }
                // Still walk the args (they may perform other effects).
                for &a in args.iter() {
                    check_no_home_walk(db, a, entrypoint, handled, followed, out, depth);
                }
                return;
            }
            // A CALL into a non-recursive callee — follow it (the perform may be cross-function). The
            // callee's body is checked under the SAME handled set (dynamic extent: the caller's handlers
            // enclose the callee's performs). A recursive callee is not followed (E3), so an ungranted
            // perform only reachable through recursion is not reported here — a conservative miss, safe
            // (it declines at lowering rather than mis-reporting). Following is DEDUPED per
            // `(callee, handled)`: N call sites of the same pure/nullary helper re-walk its body ONCE per
            // distinct handler context, not once per site (the O(N²)→O(N) fix).
            if let Some(callee) = crate::eval::lambda_body(db, head)
                .or_else(|| crate::eval::lambda_body_of_nullary(db, head))
                && !crate::eval::is_recursive(db, callee)
                && followed.insert((callee, handled_key(handled)))
            {
                check_no_home_walk(db, callee, entrypoint, handled, followed, out, depth + 1);
            }
            for &a in args.iter() {
                check_no_home_walk(db, a, entrypoint, handled, followed, out, depth);
            }
        }
        // A `handle` — its arms DISCHARGE their effects for the BODY (dynamic extent). Push each arm's
        // effect decl onto the handled set while walking the body, then pop. The arm BODIES themselves
        // resolve their own performs at the arm's definition context (the under-frame) — but for the
        // no-home check, an arm body performing its own effect re-performs OUTWARD, so we walk arm bodies
        // under the OUTER handled set (without this handle's effects added), matching forwarding. The
        // init is evaluated in the outer context too. This is the interpose contract: a perform in the
        // BODY resolves to THIS handler (a nearer handler wins over any enclosing `host` delegation, so an
        // otherwise-delegated effect is intercepted); an arm re-performing its own op forwards to the
        // next-OUTER handler/delegation, never back into this handler.
        //= spec/capabilities/capabilities-and-effects.md#a-handler-may-interpose-on-an-effect-an-entrypoint-would-delegate
        //# A program MUST be able to enclose, in a handler that discharges its operations, an effect an entrypoint would otherwise delegate to the host, so that the operation resolves to that handler rather than reaching the boundary, making it possible to observe, mock, cache, or otherwise stand in for a host capability without the performing code being aware — a handler nearer the perform wins over the delegation that encloses it.
        //= spec/capabilities/capabilities-and-effects.md#a-handler-may-interpose-on-an-effect-an-entrypoint-would-delegate
        //# A handler arm that re-performs the operation it is discharging MUST resolve that re-performance against the handlers and delegations enclosing the handler's own declaration, not against the handler itself, so that an arm forwards to the next-outer handler — up to and including the host delegation at the entrypoint — rather than recursing into itself.
        Resolved::Handle { init, arms, body } => {
            check_no_home_walk(db, init, entrypoint, handled, followed, out, depth);
            // Arm bodies: outer context (a re-performed op forwards to the next-outer handler).
            for arm in arms.iter() {
                check_no_home_walk(db, arm.body, entrypoint, handled, followed, out, depth);
            }
            // Body: this handle's effects are now handled.
            let added: Vec<u32> = arms
                .iter()
                .filter_map(|a| crate::eval::effect_op_of(db, a.op).map(|(d, _)| d.0))
                .collect();
            let before = handled.len();
            handled.extend(&added);
            check_no_home_walk(db, body, entrypoint, handled, followed, out, depth);
            handled.truncate(before);
        }
        // A `host` — its listed effects are DELEGATED for the body. Push each delegated effect's decl.
        // The `host` clause is where the entrypoint ENUMERATES, at the entrypoint itself, the effects it
        // grants to the boundary — authority enters here, not from any effect's declaration.
        //= spec/capabilities/capabilities-and-effects.md#an-entrypoint-delegates-the-capabilities-it-grants-to-the-host
        //# An entrypoint MUST enumerate, at the entrypoint itself, every effect whose operations it delegates to the host, so that granting a capability is a decision made where authority enters the program rather than a property an effect's declaration carries.
        Resolved::Host { effects, body } => {
            let added: Vec<(StructId, u32)> = effects
                .iter()
                .filter_map(|&e| {
                    // Each `effect` element is a name occurrence resolving to the effect record; recover
                    // its decl via the record's `(meta t)` = `(effect NAME <decl>)`.
                    host_effect_decl(db, e).map(|d| (e, d))
                })
                .collect();
            // A DELEGATED NAME THAT RESOLVES TO A VALUE DEFINITION. `(host (foo) …)` where `foo` is a
            // top-level `(def foo …)` names a VALUE, not an effect — a malformed grant (a `host` delegates
            // EFFECTS, `capabilities-and-effects.md` §Host Delegation Is An Entrypoint's Prerogative), not a
            // silently-ignored no-op. Reject it CDZ0201, anchored at the name. ⚠ CONSERVATIVE — flags ONLY a
            // name that is UNAMBIGUOUSLY a value def (`def_by_name`): a nested-module effect
            // (`(module m (effect log …) (def (main) (host (log) …)))`) is NOT in the TOP-LEVEL
            // `effect_decls`/`def_by_name` registries (it lives in the module's own scope), so testing "is a
            // declared effect" would false-flag it; testing "is a value def" instead only fires on a genuine
            // non-effect binding. An unbound delegated name is left to the resolver's own unbound-name check.
            for &e in effects.iter() {
                // A real effect (top-level or nested-module) is fine — skip it.
                if host_effect_decl(db, e).is_some() {
                    continue;
                }
                let Some(name) = db.ast.as_name(e).map(str::to_string) else {
                    continue;
                };
                // Classify a NON-effect delegated name — only the UNAMBIGUOUS top-level cases, so a
                // nested-module effect (referenced from an outer scope as a bare name, but a real effect
                // the compiler brings program-wide) is NEVER flagged:
                //  - a top-level VALUE def (`(def foo …)`) → "a value definition" (the original case),
                //  - a top-level TYPE (`(type C …)`) → "a type" (the host twin of the handle-head type case).
                // A genuinely-unbound name (`(host (Nope) …)`) is deliberately NOT flagged here: a
                // nested-module effect name ALSO fails to resolve in the outer scope (it resolves to a
                // Poison), so a "resolves to nothing" test would false-flag it — left to the resolver.
                let category = if db.def_by_name(&name).is_some() {
                    Some("is a value definition, not an effect")
                } else if db.type_decl_by_name(&name).is_some() {
                    Some("is a type, not an effect")
                } else {
                    None
                };
                if let Some(cat) = category {
                    out.push(crate::diag::Reject::coded(
                        crate::diag::Code::Malformed,
                        format!(
                            "a host delegates EFFECTS to the boundary, but this delegated name `{name}` \
                             {cat} — a `host` grants a declared effect, e.g. `(effect E …)`"
                        ),
                    )
                    .at(e));
                }
            }
            // A DUPLICATE DELEGATED EFFECT. `(host (A A) …)` names the same effect twice — a delegation's
            // effect list is a SET (the manifest is the union of escaping effects), so naming one twice is
            // the same fixed-set-no-duplicates ill-formedness a duplicate effect operation and a duplicate
            // handler arm are rejected for (CDZ0201). Left unchecked it double-imports at the boundary and
            // traps at run time. Report each occurrence after the first (by name, anchored at the redundant
            // occurrence), with a delete fix.
            let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
            for &e in effects.iter() {
                let Some(name) = db.ast.as_name(e).map(str::to_string) else {
                    continue;
                };
                if !seen.insert(name.clone()) {
                    out.push(
                        crate::diag::Reject::coded(
                            crate::diag::Code::Malformed,
                            format!(
                                "effect `{name}` is delegated more than once in this host — a host's \
                                 effect list is a set (the manifest is the union of escaping effects)"
                            ),
                        )
                        .at(e)
                        .with_fix(crate::diag::Fix::delete_heuristic(
                            e,
                            format!("remove the duplicate `{name}` delegation"),
                        )),
                    );
                }
            }
            // LATENT AUTHORITY (CDZ0404). A delegation must grant EXACTLY the effects that escape — an
            // effect the body never reaches is a granted-but-unexercised capability, rejected
            // (`capabilities-and-effects.md` §Host Delegation Is An Entrypoint's Prerogative). Check each
            // delegated effect is reached by a perform in the body; if not, CDZ0404 (anchored at the
            // delegation's effect-name occurrence).
            // Compute the SET of effects the body reaches ONCE (one walk), then test each delegated effect
            // by O(1) membership — not one full-body walk per delegated effect (which was O(N²) for an
            // N-effect delegation: `body_reaches_effect` re-walked the whole O(N) body N times).
            let reached = body_reached_effects(db, body);
            for &(occ, decl) in &added {
                // Suppress CDZ0404 when the body has a MEMBER ACCESS on this effect that just does not
                // resolve as a perform — a MISSPELLED op (`(E.emitt …)`). That is a cascade of the typo's
                // primary CDZ0201 ("did you mean `emit`?"), not a genuine unreached-effect: the author
                // DID intend to reach `E`. Fixing the typo makes both vanish, so report only the root.
                // (`body_has_effect_member_access` runs only for an UNREACHED effect — rare, so its per-
                // effect body walk is not on the hot path a valid all-reached delegation takes.)
                if !reached.contains(&decl) && !body_has_effect_member_access(db, body, decl) {
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
            check_no_home_walk(db, body, entrypoint, handled, followed, out, depth);
            handled.truncate(before);
        }
        // A resume's value/next-state, and every other structural form: descend into children.
        _ => {
            if let Struct::List(children) = db.ast.get(node).clone() {
                for c in children {
                    check_no_home_walk(db, c, entrypoint, handled, followed, out, depth);
                }
            }
        }
    }
}

/// The SET of effect declarations the body subtree at `node` reaches by a perform — following calls into
/// their callee bodies (the perform may be cross-function), and following a RECURSIVE callee's body ONCE
/// (guarded by a `visited` set of callee-body occurrences so a self-/mutual-recursive cycle terminates).
/// Following a recursive callee is required so `(host (log) (go 1))` where `go` recursively performs
/// `log.emit` is NOT falsely flagged as latent authority (its perform IS reached, through the recursion).
/// The latent-authority check (CDZ0404) uses this so a `(host (E0 … EN) body)` delegating N effects does
/// ONE body walk + N O(1) set lookups, not N full body walks — which was O(N²) (an N-effect delegation
/// over an O(N) body = 2s at 1600 effects, ~81% in the per-effect walk).
///
/// This SET is the function's inferred EFFECT ROW: each performed operation contributes its declaring
/// effect, following calls so a cross-function perform still counts — and the manifest of delegated
/// effects is a projection of this row (`host.rs::collect_host_imports`).
//= spec/capabilities/capabilities-and-effects.md#performing-an-operation-is-typed-and-contributes-to-the-row
//# Performing an operation MUST add its declaring effect to the effect row of the function that performs it, so that a function's inferred row is the set of effects its operations reach and the manifest of delegated effects is a projection of that row.
fn body_reached_effects(db: &mut Db, node: StructId) -> std::collections::HashSet<u32> {
    let mut reached = std::collections::HashSet::new();
    let mut visited = std::collections::HashSet::new();
    body_reached_effects_walk(db, node, 0, &mut reached, &mut visited);
    reached
}

fn body_reached_effects_walk(
    db: &mut Db,
    node: StructId,
    depth: u32,
    reached: &mut std::collections::HashSet<u32>,
    visited: &mut std::collections::HashSet<StructId>,
) {
    if depth > 64 {
        return;
    }
    if let Resolved::Apply { head, .. } = resolved_of(db, node) {
        if let Some((d, _idx)) = crate::eval::effect_op_of(db, head) {
            reached.insert(d.0);
        }
        // Follow a (possibly recursive) callee's body ONCE — `visited.insert` false on re-entry stops a
        // cycle. Mirrors `body_reaches_effect_visited`'s call-following so the reached set is identical.
        if let Some(callee) = crate::eval::lambda_body(db, head)
            .or_else(|| crate::eval::lambda_body_of_nullary(db, head))
            && visited.insert(callee)
        {
            body_reached_effects_walk(db, callee, depth + 1, reached, visited);
        }
    }
    if let Struct::List(children) = db.ast.get(node).clone() {
        for c in children {
            body_reached_effects_walk(db, c, depth, reached, visited);
        }
    }
}

/// Whether the body subtree at `node` contains a MEMBER ACCESS `(. E k)` whose operand resolves to the
/// effect `decl` — REGARDLESS of whether `k` is a declared operation. A MISSPELLED op (`(E.emitt …)` for
/// declared `emit`) does not resolve as a perform, so `body_reaches_effect` returns false and the effect
/// would be falsely flagged as latent authority (CDZ0404) — a CASCADE of the typo, which is already the
/// primary CDZ0201 "record has no field `emitt` — did you mean `emit`?". Recognizing the mis-typed
/// `E`-member here suppresses the derived CDZ0404 so the author sees ONE root error (the typo), whose fix
/// makes both vanish (`reference-compiler.md` §Outcomes Are Ordered By Safety — one primary "no" per root
/// cause). Only a member access ON THIS EFFECT counts; an unrelated typo elsewhere does not mask a
/// genuine latent-authority.
fn body_has_effect_member_access(db: &mut Db, node: StructId, decl: u32) -> bool {
    // The `(. operand key)` form: if `operand` resolves to this effect's record, it is an E-member —
    // a perform the author intends (a valid op reaches via `body_reaches_effect`; a typo lands here).
    if let Some(tail) = db.ast.as_form(node, ".")
        && let Some(&operand) = tail.first()
        && effect_decl_of_value(db, operand) == Some(decl)
    {
        return true;
    }
    match db.ast.get(node).clone() {
        Struct::List(children) => children
            .iter()
            .any(|&c| body_has_effect_member_access(db, c, decl)),
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
    // The HANDLE BODY's type. An abort makes its arm value the WHOLE handle's value, so the abort value
    // must have this type. When an abort sits inside a COMPOUND-typed body — `(tuple 1 (Bail.bail 7))` or
    // `(tuple 1 (if c (Bail.bail 7) 5))` — a scalar abort value disagrees with the compound: the whole-
    // collapse path types the handle as the scalar (Int64) while the hoist/threading path substitutes the
    // scalar into the compound (a tuple `(1,7)`) — the two syntactic shapes even INFER different handle
    // types, and the hoist emits an ill-typed `if` (invalid wasm). The type checker misses the gap (a
    // perform types by its op result). Decline when an abortive arm's value type differs from the handle
    // body's type — the abort value simply doesn't fit where the handle promises to return it.
    let handle_body_ty = crate::infer::type_of(db, body);
    let abortive_keys: Vec<(u32, u32)> = ctx.abortive.iter().copied().collect();
    for (d, i) in abortive_keys {
        let arm_op = ctx.arms.get(&(d, i))?.op;
        let arm_body = ctx.arms.get(&(d, i))?.body;
        let body_ty = crate::infer::type_of(db, arm_body);
        let result_ty = op_result_type(db, arm_op);
        // Compare by COMPATIBILITY (`agrees_with`), NOT structural `==`: two `Int` types that differ only
        // in DEFERRED-vs-Fixed sign/width are compatible (an undetermined Int unifies with Int64), so `==`
        // would spuriously flag `(if (< (Bail.bail 7) 5) 1 2)` — the handle body infers `Int{Deferred}` and
        // the abort arm value `Int64{Fixed}`. Only a genuine MISMATCH (Int64 vs Bool, scalar vs tuple)
        // declines. An undetermined side (`Any`/var) never disqualifies. TWO checks: the abort value must
        // agree with (a) its operation's declared RESULT type, and (b) the HANDLE BODY's type.
        if let Some(rt) = result_ty
            && !undetermined_ty(&body_ty)
            && !undetermined_ty(&rt)
            && !body_ty.agrees_with(&rt)
        {
            return None;
        }
        if !undetermined_ty(&body_ty)
            && !undetermined_ty(&handle_body_ty)
            && !body_ty.agrees_with(&handle_body_ty)
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
    // GUARD-CONDITION PERFORM. A perform inside a match-arm GUARD condition — `(match k ((guard p (E.op)) b)
    // …)` — is a position the distribution/fold walks do NOT descend (they route the scrutinee + arm bodies,
    // not the guard conds). For the SOUND, NARROW shape — a two-arm match whose first arm has an IRREFUTABLE
    // inner pattern guarded by a performing cond, and an irrefutable catch-all second arm — desugar it to an
    // `if` on the guard (`desugar_performing_guard_match`): the arm is selected iff the guard holds, so
    // `(match k ((guard <irrefutable> g) b) (<irrefutable> b2))` ≡ `(if g b b2)` (each binder let-bound to
    // the scrutinee). The guard becomes an `if` CONDITION — a strict-first position the if-condition fold
    // already routes through the enclosing handle. A shape this does NOT cover (multiple guarded arms, which
    // sequence handler state per arm-test — the per-branch-sees-the-seed distribution does not model that; or
    // a refutable guarded pattern) is left to DECLINE cleanly (the honest "not yet reducible" todo), never
    // the factually-wrong "no enclosing handler" error.
    let body = match desugar_performing_guard_match(db, body, &ctx) {
        Some(rewritten) => rewritten,
        None => {
            if body_has_performing_match_guard(db, body, &ctx) {
                return None;
            }
            body
        }
    };
    // RESUMPTIVE (tail-resume) NON-TAIL CONDITIONAL HOIST. A `perform` inside an `if`/`match` BRANCH
    // advances the handler state LOCALLY, but the `if` thread arm returns the post-CONDITION state as its
    // out-state — so the state advance is LOST to any CONTINUATION after the conditional (`(do (if c
    // (E.op) x) (E.op))` runs the second `(E.op)` against the pre-branch state). The branch-out-state is
    // genuinely a runtime PHI of the two branches, which the tail fold does not represent. Fix by the same
    // distribution the abortive hoist uses: lift the `if`/`match` OUT of the strict continuation into a
    // TAIL position, distributing the continuation into both branches — `(do (if c t e) k)` ≡ `(if c (do
    // t k) (do e k))`, `(+ (if c t e) rhs)` ≡ `(if c (+ t rhs) (+ e rhs))`. Then each branch threads its
    // own state THROUGH its copy of the continuation (the `if` is now in tail position, where the fold is
    // correct). Sound: the CONDITION stays the single `if` condition (evaluated exactly once, never
    // duplicated), and the continuation is duplicated across both branches but only one branch runs at
    // runtime — so every effect in it happens exactly once, in the same relative order, as long as no
    // EFFECTFUL sibling is evaluated BEFORE the `if` (that would jump the condition ahead of it); the
    // hoist requires every preceding sibling pure. Runs to a fixpoint (bounded). A shape it cannot lift
    // (a perform under a conditional the hoist could not raise to tail) is left as-is and declines below.
    let body = hoist_resumptive_conditional(db, body, &ctx);
    // E5 HANDLER DISTRIBUTION over a pure-conditioned tail conditional (a commuting conversion). When the
    // handle BODY is an `if`/`match` whose CONDITION/SCRUTINEE is strongly pure but a BRANCH / ARM BODY
    // performs a discharged op, the pure one-hole fold below declines (a branch perform is a NON-uniform
    // continuation — it runs only on the taken path). But the conditional IS the whole handle body (tail
    // position), so the handler distributes into each branch — `(handle E s arms (if c t e))` ≡ `(if c
    // (handle E s arms t) (handle E s arms e))`, `(handle E s arms (match k (p b)…))` ≡ `(match k (p (handle
    // E s arms b))…)`: the condition/scrutinee runs exactly ONCE (pure, evaluated first, advancing no
    // state), and each branch becomes a SMALLER handle body the fold already serves (only one runs at
    // runtime, seeing the seed state — nothing advanced it). Recurse `reduce_handle` on each; if any branch
    // is a shape the fold cannot serve, the whole distribution declines (`?`) and we fall through to the
    // ordinary decline. GATED to the NON-tail-resumptive regime (all arms non-tail, none abortive) — a
    // tail-resumptive branch perform is already handled by the threading path, and an abortive one by
    // `hoist_conditional_abort`; distributing there would only duplicate working paths.
    if ctx.abortive.is_empty()
        && ctx.arms.values().all(|a| tail_resume(db, a.body).is_none())
        && let Some(distributed) = distribute_handler_over_conditional(db, init, arms, body, &ctx)
    {
        return Some(distributed);
    }
    // NESTED-HANDLE PRE-REDUCTION (for a NON-tail-resumptive outer handler). The inside-out `thread` path
    // reduces a nested inner `handle` only while THREADING — which requires this (outer) handler's arms to
    // be tail-resumptive. When they are NOT, a body like `(handle B tail (+ (A.a) (B.b)))` reaches the E5
    // pure-one-hole check with the raw inner `handle` node still present (`pure_hole` sees a nested handle →
    // Impure → declines). Reduce the inner handle(s) FIRST so the body becomes `(+ (A.a) 20)` — a single
    // outer-effect perform in a pure one-hole context the E5 fold below serves (→ `(+ 1 (+ 20 10))` = 31).
    // GATED to the non-tail-resumptive, non-abortive regime (the tail path already reduces inner handles via
    // `thread`, and the merge path handles a recursive callee spanning both effects); only run it when the
    // body actually contains a nested handle, so the common no-nested-handle body is untouched.
    let body = if ctx.abortive.is_empty()
        && ctx.arms.values().all(|a| tail_resume(db, a.body).is_none())
        && body_contains_nested_handle(db, body)
    {
        reduce_inner_handles(db, body)
    } else {
        body
    };
    // APPLIED-LAMBDA PRE-REDUCTION. A handle body that wraps its perform in a lambda APPLICATION —
    // `((fn (x) (+ x (Amb.flip))) 100)` or a `let`-bound `(f 100)` where `f`'s body performs — reaches the
    // pure-one-hole classifier with the raw `Apply` node still present (`pure_hole` does not β-reduce a
    // lambda-head call, so it sees a non-uniform / effect-reaching call → declines). The `thread`/one-shot
    // path DOES inline such a call (its `call_reaches_discharged_effect` arm), so the one-shot case folds;
    // but the MULTI-shot path goes through `pure_hole` and declined. β-reduce these redexes FIRST so the
    // body becomes `(+ 100 (Amb.flip))` — a single perform in a pure one-hole context the E5 fold serves
    // under a multi-shot arm too. Same β-reduction the inline arm performs, only sequenced earlier; a
    // recursive callee is excluded by `call_reaches_discharged_effect` (specialized, not inlined). Gated on
    // a cheap syntactic check so the common no-such-redex body is untouched.
    let body = if body_contains_applied_performing_lambda(db, body, &ctx) {
        reduce_applied_lambdas(db, body, &ctx)
    } else {
        body
    };
    // E5 PURE ONE-HOLE-CONTINUATION fold (general one-shot, the pure-continuation case). When the handle
    // BODY reaches EXACTLY ONE discharged perform `P` through STRICT, UNCONDITIONAL, effect-free positions
    // (`pure_hole`), its delimited continuation is the PURE one-hole context `C = body[P := □]`. Resuming
    // returns into that context, so `(resume v s)` yields `C[v]` — a copy of the body with `P` replaced by
    // `v`. The handle's value is then the arm body with every `(resume v s)` rewritten to `C[v]`:
    //   `(handle Amb 0 ((flip (u) s (+ 1 (resume 10 s)))) (+ 100 (Amb.flip)))`
    //     → arm `(+ 1 (resume 10 0))`, `C = (+ 100 □)` → `(+ 1 (+ 100 10))` = 111.
    // Because `C` is STRONGLY pure (no effect of any kind), a MULTI-shot arm may duplicate it safely:
    //   `(+ (resume 1 s) (resume 2 s))` over `(+ 100 (Amb.flip))` → `(+ (+ 100 1) (+ 100 2))` = 203.
    // The IDENTITY slice (`body` IS the bare perform, `C = □`, so `C[v] = v`) is the special case. A
    // perform under a CONDITIONAL (`if`/`match`/`and`/`or` — a non-uniform continuation), a SECOND
    // perform, a `resume`, a nested `handle`, or a non-primitive call disqualifies (`pure_hole` → this
    // block does not fire and threading declines below); those need the full captured-continuation
    // machinery (defunctionalized frames — a later increment).
    if let PureHole::Hole(perform) = pure_hole(db, body, &ctx)
        && let Resolved::Apply { head, args } = resolved_of(db, perform)
        && let Some((decl, idx)) = is_perform(db, head, &ctx)
        && let Some(arm) = ctx.arms.get(&(decl, idx)).cloned()
        && !ctx.abortive.contains(&(decl, idx))
        && tail_resume(db, arm.body).is_none()
    {
        // Substitute the arm's params ↦ (pure-copied) perform args and its state binder ↦ the init seed
        // (nothing runs before the perform on a pure spine, so the state seen at the perform is the seed),
        // then rewrite every `(resume v s)` → `C[v]`. The perform's args are pure (`pure_hole` admits only
        // strongly-pure arguments), so evaluating them has no effect and they need no state threading.
        let mut subst: HashMap<StructId, StructId> = HashMap::default();
        if arm.params.len() == args.len() {
            for (&p, &a) in arm.params.iter().zip(args.iter()) {
                if !is_unit_param(db, p) {
                    subst.insert(p, copy_pure(db, a));
                }
            }
        } else if arm.params.len() == 1 && args.is_empty() {
            let p = arm.params[0];
            if !is_unit_param(db, p) {
                let unit = db.push_list(vec![]);
                subst.insert(p, unit);
            }
        } else {
            return None;
        }
        subst.insert(arm.state, init);
        let substituted = crate::eval::beta_reduce(db, arm.body, &subst);
        // Rewrite every `(resume v s)` → `C[v]` (the pure delimited continuation applied to the resume
        // value). The arm body's free names keep their pinned resolution through `beta_reduce`; `C`'s free
        // names resolve against the handle scope (the structural splice copy re-parents them). The
        // next-state is dead — nothing after the perform reads state on a pure spine.
        let folded = rewrite_resume_to_context(db, substituted, body, perform);
        // Graft the synthesized `folded` UNDER the original handle's site BEFORE the type-check below, so a
        // FREE name inside it — an enclosing function's parameter or a match-arm binder used in the handle
        // body, `(handle … (+ x (Amb.flip)))` — resolves up the original lexical chain instead of reading
        // unbound. `push_list` leaves a synthesized root's parent `None`; the lowering site re-parents the
        // final result, but the `type_errors` guard here runs FIRST, so without this an outer-param body
        // would spuriously fault (unbound name) and OVER-DECLINE. Parent to the `handle` node itself (the
        // same discipline the lowering-site reparent uses — see lower.rs) so the scope walk ascends
        // folded → handle → … and a binder form above still recognizes the handle as its body child.
        reparent_under_handle_site(db, folded, body);
        // TYPE-CONSISTENCY GUARD (the resumptive analogue of the abortive guard above). A `resume` node
        // types as `Ty::Any` (infer.rs), so an arm body `(+ 1 (resume 10 s))` type-checks LENIENTLY — the
        // `+` accepts the `Any`-typed resume. But `(resume v s)` yields `C[v]`, which has the handle BODY's
        // type: for a Bool-typed body `(< (Amb.flip) 5)` the fold produces `(+ 1 (< 10 5))` — an integer
        // `+` over a Bool — which the ORIGINAL program was ill-typed to express (the arm consumes the
        // continuation result at a type the body cannot supply). `reduce_handle` runs at LOWERING, after
        // inference, so nothing re-checks the folded term — it would reach codegen as invalid wasm. Re-run
        // the type checker on the folded result and DECLINE if it faults, so an ill-typed composition is
        // rejected (the whole program errors — the handle has no valid fold), never miscompiled.
        if !crate::infer::type_errors(db, folded).is_empty() {
            return None;
        }
        return Some(folded);
    }
    // E5 TWO-HOLE (general one-shot) fold: a NON-tail one-shot arm whose LEADING discharged perform sits on
    // the strict spine but whose continuation ITSELF performs (a second hole) — `(+ (Amb.flip) (Amb.flip))`
    // under `(flip (u) s (+ 1 (resume 10 s)))`. The pure one-hole block above declined it (`C` is not pure).
    // In a DEEP handler, `resume v s'` returns into `C[v]` WITH THE HANDLER STILL ACTIVE, so the second
    // perform in `C[v]` is handled too: `resume v s' = reduce_handle(s', arms, C[v])`. Each refold removes
    // one perform → terminates. GATED to a ONE-SHOT arm (`count_resumes == 1`): the resume value flows into
    // `C` exactly once, so the inner perform in `C` runs exactly once (a multi-shot arm would duplicate it —
    // the frame vertical's job). The leading perform's ARGS are strongly pure (`leading_strict_hole` checks),
    // so they need no state threading; the state at the leading perform is the seed (nothing runs before it).
    if let Some(perform) = leading_strict_hole(db, body, &ctx)
        && let Resolved::Apply { head, args } = resolved_of(db, perform)
        && let Some((decl, idx)) = is_perform(db, head, &ctx)
        && let Some(arm) = ctx.arms.get(&(decl, idx)).cloned()
        && !ctx.abortive.contains(&(decl, idx))
        // A tail-resumptive arm (bare OR do-wrapped interpose/forward) is served by the `thread` path — do
        // NOT steal it here (it would decline a forwarding arm whose resume value is a foreign perform).
        && !is_tail_resumptive_arm(db, arm.body)
        // MULTI-SHOT is sound only when the continuation `C` (re-reduced per resume) reaches NO FOREIGN
        // perform — i.e. only THIS handler's discharged ops, which the refold folds away into pure code.
        // A ONE-SHOT arm splices `C` once, so any foreign perform in it runs once (sound). But a MULTI-shot
        // arm would re-run a foreign/HOST perform in `C` once per resume — the host-composition invariant
        // (DESIGN §4.4: a reified continuation must not span a host call) forbids that, so require the body
        // to be free of any undischarged (foreign/host) perform when the arm resumes more than once.
        && (count_resumes(db, arm.body) == 1 || !body_reaches_foreign_perform(db, body, &ctx))
    {
        let mut subst: HashMap<StructId, StructId> = HashMap::default();
        if arm.params.len() == args.len() {
            for (&p, &a) in arm.params.iter().zip(args.iter()) {
                if !is_unit_param(db, p) {
                    subst.insert(p, copy_pure(db, a));
                }
            }
        } else if arm.params.len() == 1 && args.is_empty() {
            let p = arm.params[0];
            if !is_unit_param(db, p) {
                let unit = db.push_list(vec![]);
                subst.insert(p, unit);
            }
        } else {
            return None;
        }
        subst.insert(arm.state, init);
        let substituted = crate::eval::beta_reduce(db, arm.body, &subst);
        // Rewrite the arm's single `(resume v s')` to `reduce_handle(s', arms, C[v])` — the re-reduced
        // continuation (a further discharged perform in `C` is folded by the recursive call). Declines
        // cleanly (`?`) if any recursive refold cannot be served.
        let folded = rewrite_resume_to_refolded_context(db, substituted, body, perform, arms)?;
        reparent_under_handle_site(db, folded, body);
        // Same type-consistency guard as the pure one-hole block — the synthesized term is not otherwise
        // re-checked before codegen.
        if !crate::infer::type_errors(db, folded).is_empty() {
            return None;
        }
        return Some(folded);
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
    // MULTI-VALUE (repro-1): if the handle BODY was itself a self-call to a multi-value spec — `(handle …
    // (relabel tree))` — the self-call arm pushed a pending temp and returned `(. t 0)` (the value
    // projection); the temp is not yet bound. Drain any pending temps into wrapping `let`s so the handle
    // value is `(let ((t (f#ctx … init))) (. t 0))`. (The self-call arm already discards each spec's
    // OUT-state at the top level — the handle observes only the value.) Nothing pending → returns `rewritten`.
    let wrapped = drain_and_wrap(db, &ctx, 0, rewritten);
    Some(wrapped)
}

/// Reduce every NESTED inner `handle` found in `node` to its folded form, IN PLACE (returning a rewritten
/// copy). An inner handle that reduces (`reduce_handle`) is replaced by its result; one that does not is
/// left untouched. This runs in `reduce_handle` BEFORE the E5 pure-one-hole check so an OUTER handler whose
/// arm is NON-tail-resumptive can serve a body that contains a reducible inner handle of a DIFFERENT
/// effect: the inside-out `thread` path reduces an inner handle only while threading (which needs the OUTER
/// arm tail-resumptive), so `(handle A non-tail (handle B tail (+ (A.a) (B.b))))` used to decline — B never
/// got reduced before A's E5 fold ran and saw the raw inner `handle` node (a non-uniform continuation).
/// Reducing B FIRST turns the body into `(+ (A.a) 20)`, a single A-perform in a pure one-hole context the
/// outer E5 fold folds to `(+ 1 (+ 20 10))` = 31. SOUND + frame-free: reducing the inner handle is the
/// same already-proven-safe reduction the threading path performs, only sequenced earlier. Only the
/// SHALLOWEST inner handles are reduced (`reduce_handle` recurses into its own body), and a handle that is
/// itself the whole `node` is NOT reduced here (the caller is mid-reducing it). Bounded by `reduce_handle`'s
/// own re-entry guard.
fn reduce_inner_handles(db: &mut Db, node: StructId) -> StructId {
    // A `handle` node: reduce IT (its body's own nested handles are reduced by that call recursing here),
    // and use the result. If it declines, fall through to a structural copy so the node is unchanged.
    if let Resolved::Handle { init, arms, body } = resolved_of(db, node)
        && let Some(reduced) = reduce_handle(db, init, &arms, body)
    {
        return reduced;
    }
    // Otherwise descend structurally, reducing any inner handle in a child.
    match db.ast.get(node).clone() {
        Struct::List(children) => {
            let rebuilt: Vec<StructId> = children
                .iter()
                .map(|&c| reduce_inner_handles(db, c))
                .collect();
            db.push_list(rebuilt)
        }
        Struct::Atom(_) => node,
    }
}

/// Whether `node` IS or CONTAINS a nested `handle` (of an inner effect). The gate for the nested-handle
/// pre-reduction — a cheap syntactic check so a body with no inner handle skips the pre-reduction pass. The
/// handle body may BE the inner handle directly (`(handle A … (handle B …))`) or contain it inside an
/// operator (`(+ (handle B …) x)`), so both `node` itself and its descendants count.
fn body_contains_nested_handle(db: &mut Db, node: StructId) -> bool {
    if matches!(resolved_of(db, node), Resolved::Handle { .. }) {
        return true;
    }
    match db.ast.get(node).clone() {
        Struct::List(children) => children.iter().any(|&c| body_contains_nested_handle(db, c)),
        Struct::Atom(_) => false,
    }
}

/// Whether the handle body has a MATCH whose arm PATTERN performs a discharged op — i.e. a `(guard
/// <pattern> <cond>)` whose GUARD CONDITION performs (a plain pattern is a binder/literal that never
/// performs, so a perform in the PATTERN position is always a guard cond). The effect-routing/distribution
/// walks (`distribute_handler_over_conditional`, the fold) descend a match's SCRUTINEE and ARM BODIES but
/// NOT the arm guard conditions, so a perform in a guard is left unrouted and reaches lowering as a bare
/// perform → the misleading "performed with no enclosing handler here" (a handler DOES enclose it). Routing
/// a guard perform through the handler is a genuine extension (the guard runs before the arm, advancing
/// handler state per arm-test — a stateful sequencing the current per-branch-sees-the-seed distribution
/// does not model), so until it is wired, DECLINE cleanly here (`reduce_handle` → `None` →
/// `HANDLER_NOT_REDUCIBLE_DECLINE`, an honest "not yet reducible" todo) rather than letting the unrouted
/// perform surface the factually-wrong "no enclosing handler" error. Recurses under `let`/`if`/`do`/nested
/// `match` (a guard in any nested match under the handle body is equally unrouted). A NESTED `handle`'s own
/// body is NOT descended (its guards are its own handler's concern — reduced when that inner handle folds).
fn body_has_performing_match_guard(db: &mut Db, node: StructId, ctx: &HandlerCtx) -> bool {
    if matches!(resolved_of(db, node), Resolved::Handle { .. }) {
        return false; // an inner handle's guards belong to that handle's own reduction
    }
    if let Resolved::Match { arms, .. } = resolved_of(db, node)
        && arms.iter().any(|&(pat, _)| subtree_performs(db, pat, ctx))
    {
        return true;
    }
    match db.ast.get(node).clone() {
        Struct::List(children) => children
            .iter()
            .any(|&c| body_has_performing_match_guard(db, c, ctx)),
        Struct::Atom(_) => false,
    }
}

/// Whether `pat` is an IRREFUTABLE inner pattern — a bare name binder or `_` wildcard — which matches ANY
/// scrutinee. Used by the guard desugar: a guarded arm whose inner pattern is irrefutable is selected iff
/// its GUARD holds, so it desugars to an `if` on the guard (the pattern narrows nothing).
fn is_irrefutable_pattern(db: &mut Db, pat: StructId) -> bool {
    match db.ast.as_name(pat) {
        Some("_") => true,
        Some(_) => true, // a bare name binds the whole scrutinee — irrefutable
        None => false,   // a literal / compound / annotated pattern refutes
    }
}

/// Rewrite a match with a PERFORMING GUARD into an `if` so the existing if-condition fold routes the guard
/// perform — the SOUND, NARROW case: a two-arm match `(match scrut ((guard <irrefutable> g) b) (<irrefutable>
/// b2))` where the FIRST arm's inner pattern is irrefutable (so the arm is selected exactly when `g` holds)
/// and the SECOND is an irrefutable catch-all. Such a match is equivalent to `(if g b b2)` with each arm's
/// binder let-bound to the scrutinee — the guard becomes an `if` CONDITION, a strict-first position the fold
/// serves. Returns the rewritten `(if …)` (wrapped in a `let` if the first arm binds a name), or `None` when
/// the body is not this exact shape (leaving the honest guard-perform decline for the general case: multiple
/// guarded arms sequence state per arm-test, which this narrow rewrite does not model). Walks structurally
/// so the match may be nested in the body. Does NOT descend into an inner `handle` (its guards are that
/// handle's concern).
fn desugar_performing_guard_match(
    db: &mut Db,
    node: StructId,
    ctx: &HandlerCtx,
) -> Option<StructId> {
    if matches!(resolved_of(db, node), Resolved::Handle { .. }) {
        return None;
    }
    if let Resolved::Match { scrutinee, arms } = resolved_of(db, node)
        && arms.len() == 2
    {
        let (pat0, body0) = arms[0];
        let (pat1, body1) = arms[1];
        // First arm must be `(guard <irrefutable> <cond>)` with a PERFORMING cond; second an irrefutable
        // catch-all with NO guard.
        if let Some(g) = db.ast.as_form(pat0, "guard").map(|t| t.to_vec())
            && g.len() == 2
            && is_irrefutable_pattern(db, g[0])
            && subtree_performs(db, g[1], ctx)
            && db.ast.as_form(pat1, "guard").is_none()
            && is_irrefutable_pattern(db, pat1)
        {
            let cond = g[1];
            // Bind the scrutinee to each arm's binder (if a name, not `_`) so the arm bodies + guard read it.
            // Both arms bind the SAME scrutinee value; if either inner pattern is a name, wrap the `if` in a
            // `let` for that name. (The two arms' binders may differ in name; a name in arm 2's body reads
            // the same scrutinee, so bind whichever names appear. Here we bind arm 0's name — the guard and
            // body0 reference it; arm 1's binder, if a distinct name, is handled by binding it too.)
            let if_head = db.push_name("if");
            let if_node = db.push_list(vec![if_head, cond, body0, body1]);
            // Wrap in `let` bindings for any named (non-`_`) inner patterns, so the guard/bodies resolve them
            // to the scrutinee. A wildcard `_` binds nothing.
            let mut binders: Vec<StructId> = Vec::new();
            for &p in &[g[0], pat1] {
                if let Some(name) = db.ast.as_name(p)
                    && name != "_"
                {
                    let name_atom = db.push_atom(Leaf::Name(name.to_string()));
                    let scrut_copy = copy_pure(db, scrutinee);
                    binders.push(db.push_list(vec![name_atom, scrut_copy]));
                }
            }
            if binders.is_empty() {
                return Some(if_node);
            }
            let let_head = db.push_name("let");
            let bindings = db.push_list(binders);
            return Some(db.push_list(vec![let_head, bindings, if_node]));
        }
    }
    // Recurse structurally: the match may be nested inside the body.
    match db.ast.get(node).clone() {
        Struct::List(children) => {
            let mut changed = false;
            let mut rebuilt = Vec::with_capacity(children.len());
            for c in children {
                match desugar_performing_guard_match(db, c, ctx) {
                    Some(r) => {
                        rebuilt.push(r);
                        changed = true;
                    }
                    None => rebuilt.push(c),
                }
            }
            if changed {
                Some(db.push_list(rebuilt))
            } else {
                None
            }
        }
        Struct::Atom(_) => None,
    }
}

/// β-reduce every APPLIED-LAMBDA REDEX in `node` whose callee reaches a discharged operation AND whose
/// arguments are all STRONGLY PURE — an `((fn (x) …(E.op)…) 100)` or a `let`-bound `(f 100)` where `f`'s
/// body performs — into its substituted body, recursively. Runs in `reduce_handle` BEFORE the pure-one-hole
/// classifier so a body that wraps its single perform in a lambda application reaches the fold in reduced
/// form: `((fn (x) (+ x (Amb.flip))) 100)` becomes `(+ 100 (Amb.flip))`, a single perform in a pure one-hole
/// context the E5 fold serves — folding under a MULTI-shot arm too (the `thread`/one-shot path already
/// inlines such a call via its `call_reaches_discharged_effect` arm, but the pure-one-hole/multi-shot path
/// does not). SOUND: this is a β-reduction with PURE arguments — substituting a pure argument (even into a
/// param used MANY times) duplicates no effect. A PERFORMING argument is NOT pre-reduced here (the
/// strongly-pure guard fails): β-substituting it into a multiply-used param would duplicate its effect
/// (`(mixed (Amb.flip))`, `mixed x = x + x + …` → `flip` per `x`, a miscompile), so it is left for the
/// `thread` path, which threads the argument's state exactly once before inlining. A RECURSIVE callee is
/// EXCLUDED (`call_reaches_discharged_effect` returns false — it is specialized, not inlined) so the
/// reduction terminates. A non-redex node is descended structurally. Bounded by `reduce_handle`'s re-entry
/// guard (each reduced body is re-walked once).
fn reduce_applied_lambdas(db: &mut Db, node: StructId, ctx: &HandlerCtx) -> StructId {
    // An application whose head is a (non-recursive) lambda/ref-to-lambda reaching the discharged effect,
    // AND whose arguments are all strongly pure (the soundness guard — see the doc comment): β-reduce it
    // (substitute args for params), then re-walk the reduced body for further nested redexes.
    if let Resolved::Apply { head, args } = resolved_of(db, node)
        && is_perform(db, head, ctx).is_none()
        && args.iter().all(|&a| strongly_pure(db, a, ctx))
        && call_reaches_discharged_effect(db, head, ctx)
    {
        // Reduce each argument's own redexes first (a pure arg won't itself be a performing redex, but keep
        // the recursion uniform), then β-reduce the call. A parameterized callee substitutes; a nullary def
        // has no lambda wrapper (its name resolves straight to its body), so fall back to that body.
        let rargs: Vec<StructId> = args
            .iter()
            .map(|&a| reduce_applied_lambdas(db, a, ctx))
            .collect();
        let reduced = match crate::eval::apply_lambda(db, head, &rargs).ok().flatten() {
            Some(r) => r,
            None => match crate::eval::lambda_body_of_nullary(db, head) {
                Some(b) => b,
                None => return node, // not actually reducible — leave it
            },
        };
        return reduce_applied_lambdas(db, reduced, ctx);
    }
    // Otherwise descend structurally, reducing any redex in a child.
    match db.ast.get(node).clone() {
        Struct::List(children) => {
            let rebuilt: Vec<StructId> = children
                .iter()
                .map(|&c| reduce_applied_lambdas(db, c, ctx))
                .collect();
            db.push_list(rebuilt)
        }
        Struct::Atom(_) => node,
    }
}

/// Whether `node` IS or CONTAINS an applied-lambda redex the pre-reduction will reduce — a call whose
/// callee reaches a discharged operation and whose arguments are all strongly pure. The gate for the
/// applied-lambda pre-reduction, so a body with no such redex skips the pass (the common case). Mirrors
/// `reduce_applied_lambdas`'s own guard (incl. the strongly-pure-args soundness condition) so the gate and
/// the reducer agree on exactly which redexes fire.
fn body_contains_applied_performing_lambda(db: &mut Db, node: StructId, ctx: &HandlerCtx) -> bool {
    if let Resolved::Apply { head, args } = resolved_of(db, node)
        && is_perform(db, head, ctx).is_none()
        && args.iter().all(|&a| strongly_pure(db, a, ctx))
        && call_reaches_discharged_effect(db, head, ctx)
    {
        return true;
    }
    match db.ast.get(node).clone() {
        Struct::List(children) => children
            .iter()
            .any(|&c| body_contains_applied_performing_lambda(db, c, ctx)),
        Struct::Atom(_) => false,
    }
}

/// Graft the synthesized `folded` body into the lexical slot the (post-hoist) handle `body` occupies, so a
/// free name inside `folded` — an enclosing function parameter or a match-arm binder the handle body
/// references — resolves up the SAME scope chain the original `body` did. `body`'s parent is the handle
/// node (or, in a distributed branch, the `if`/arm above it), which itself ascends to the enclosing
/// `def`/`fn`/`match`; parenting `folded` at `body`'s parent + child index reproduces exactly that chain.
/// This lets the `type_errors` guard in `reduce_handle` type the folded term with its free names bound
/// (else an outer-param body over-declines with a spurious unbound-name fault). A no-op if `body` is the
/// arena root (no parent) — a top-level handle body has no enclosing scope to reach anyway.
fn reparent_under_handle_site(db: &mut Db, folded: StructId, body: StructId) {
    let Some(parent) = db.parent_of(body) else {
        return; // a top-level handle body — no enclosing scope to reach
    };
    // When the handle body being reduced is the SECOND child of a 2-element `(x body)` pair — a `match`
    // ARM `(pattern body)` (the distribution case) or a `let` binding `(name init)` — the binder scope
    // check (`resolve::match_arm_binds`) reads the pair's RECORDED body child (`pb[1]`) and demands the
    // reference ascend from THAT node. Parenting `folded` directly under the pair leaves `pb[1]` as the
    // ORIGINAL body, so the ascended-from child (`folded`) would not match and a binder-referencing arm
    // body would over-decline. Rebuild a fresh `(x folded)` pair in the original pair's slot so `folded`
    // IS the recorded body child — then the binder resolves against the reduced body. (A non-pair parent —
    // the `handle` node itself, or an `if` node in a distributed branch — is reached by ascending THROUGH
    // it to the enclosing binder form, so it needs no rebuild: parent `folded` directly.)
    if let Struct::List(children) = db.ast.get(parent).clone()
        && children.len() == 2
        && children[1] == body
        && let Some(grandparent) = db.parent_of(parent)
    {
        let fresh_pair = db.push_list(vec![children[0], folded]);
        db.reparent(fresh_pair, Some(grandparent), db.child_ix_of(parent) as u32);
        return;
    }
    db.reparent(folded, Some(parent), db.child_ix_of(body) as u32);
}

/// HANDLER DISTRIBUTION over a pure-conditioned tail conditional (a commuting conversion). If `body` is an
/// `(if c t e)` or a `(match scrut arms…)` whose CONDITION/SCRUTINEE is strongly pure (runs once, advances
/// no state, so it need not thread through the handler) but whose fold otherwise declines because a
/// BRANCH / ARM BODY performs a discharged op, distribute the handler into each branch:
///   `(handle E s arms (if c t e))`     ≡ `(if c (handle E s arms t) (handle E s arms e))`
///   `(handle E s arms (match k (p b)…))` ≡ `(match k (p (handle E s arms b))…)`
/// Each branch / arm body is re-`reduce_handle`d with the SAME init/arms (only one runs at runtime, seeing
/// the seed state — the condition/scrutinee advanced nothing). Returns the rebuilt conditional, or `None`
/// if the body is neither shape, the condition/scrutinee is not strongly pure, or any branch's fold
/// declines (so the caller falls through to the ordinary decline — never a partial rewrite). A `match`
/// PATTERN is reused verbatim in the rebuilt arm, so its binder still scopes the (re-anchored) reduced arm
/// body — `reduce_handle`'s `reparent_under_handle_site` anchors the reduced body under the ORIGINAL arm
/// pair while it type-checks, so a binder-referencing arm body resolves; the final rebuild then re-parents
/// each reduced body under its new arm pair.
fn distribute_handler_over_conditional(
    db: &mut Db,
    init: StructId,
    arms: &[HandleArm],
    body: StructId,
    ctx: &HandlerCtx,
) -> Option<StructId> {
    match resolved_of(db, body) {
        Resolved::If { cond, then_, else_ } => {
            // The condition must be strongly pure — it runs once, before either branch. A performing
            // condition is the `pure_hole` if-cond case (folds below) or a threading shape; distributing it
            // would risk moving a perform. Only distribute when a BRANCH performs (else the fold serves the
            // body directly).
            if !strongly_pure(db, cond, ctx) {
                return None;
            }
            if !subtree_performs(db, then_, ctx) && !subtree_performs(db, else_, ctx) {
                return None;
            }
            // Reduce each branch as its own handle body (init/arm occurrences are only READ + copied on
            // substitution, so sharing them across the branch reductions is safe). Either branch declining
            // makes the whole distribution decline — no partial rewrite.
            let then_r = reduce_handle(db, init, arms, then_)?;
            let else_r = reduce_handle(db, init, arms, else_)?;
            let if_head = db.push_name("if");
            Some(db.push_list(vec![if_head, cond, then_r, else_r]))
        }
        Resolved::Match {
            scrutinee,
            arms: match_arms,
        } => {
            // The SCRUTINEE must be strongly pure (evaluated once, before any arm). Only distribute when an
            // ARM BODY performs. A pattern is a binder position (no perform), so `subtree_performs` on the
            // arm bodies is the trigger.
            if !strongly_pure(db, scrutinee, ctx) {
                return None;
            }
            if !match_arms
                .iter()
                .any(|&(_, arm_body)| subtree_performs(db, arm_body, ctx))
            {
                return None;
            }
            // Rebuild `(match scrutinee (pat body')…)`: reduce each arm body under the same init/arms, reuse
            // each pattern verbatim (its binder scopes the reduced body — see `reparent_under_handle_site`).
            // Any arm's fold declining declines the whole distribution.
            let match_head = db.push_name("match");
            let mut children = vec![match_head, scrutinee];
            for &(pat, arm_body) in match_arms.iter() {
                let reduced = reduce_handle(db, init, arms, arm_body)?;
                children.push(db.push_list(vec![pat, reduced]));
            }
            Some(db.push_list(children))
        }
        _ => None,
    }
}

/// Whether applying `head` (a non-perform application head) reaches an abortive operation through the
/// callee's body FROM INSIDE A CONDITIONAL — the unsound cross-function case. Follows a NON-RECURSIVE
/// callee (the inline arm's target) and checks whether an abortive perform sits under an `if`/`match`/
/// short-circuit within it. Such an abort surfaces, after the inline arm β-reduces the callee, as a
/// non-tail conditional the hoist never saw (it was opaque behind the call) — and `thread`'s `if` arm
/// would then capture it PER-BRANCH as if the `if` were the handle's tail, dropping the enclosing op
/// (`(+ 10 (check -1))` → 109, a MISCOMPILE). An UNCONDITIONAL cross-fn abort (the callee's body is a bare
/// abort, `(+ 10 (boom 99))`) is NOT flagged — inlining yields a plain strict abort the E4-a machinery
/// collapses soundly. A recursive callee is not followed (not inlined). Depth-bounded.
fn call_reaches_conditional_abortive(db: &mut Db, head: StructId, ctx: &HandlerCtx) -> bool {
    let Some(body) = crate::eval::lambda_body(db, head)
        .or_else(|| crate::eval::lambda_body_of_nullary(db, head))
    else {
        return false;
    };
    if crate::eval::is_recursive(db, body) {
        return false; // a recursive callee is not inlined — not this arm's concern
    }
    subtree_has_conditional_abortive(db, body, ctx, false)
}

/// Whether `node` contains an abortive perform reached from UNDER a conditional (`if`/`match`/short-circuit
/// connective) — `under_cond` tracks whether we have descended into a conditional position. A bare abort
/// not under any conditional is NOT reported (it is the sound unconditional-collapse case).
fn subtree_has_conditional_abortive(
    db: &mut Db,
    node: StructId,
    ctx: &HandlerCtx,
    under_cond: bool,
) -> bool {
    if under_cond
        && let Resolved::Apply { head, .. } = resolved_of(db, node)
        && let Some(id) = is_perform(db, head, ctx)
        && ctx.abortive.contains(&id)
    {
        return true;
    }
    // A conditional marks its branches / shielded operand `under_cond`.
    if let Resolved::If { cond, then_, else_ } = resolved_of(db, node) {
        return subtree_has_conditional_abortive(db, cond, ctx, under_cond)
            || subtree_has_conditional_abortive(db, then_, ctx, true)
            || subtree_has_conditional_abortive(db, else_, ctx, true);
    }
    if let Resolved::And { lhs, rhs, .. } = resolved_of(db, node) {
        return subtree_has_conditional_abortive(db, lhs, ctx, under_cond)
            || subtree_has_conditional_abortive(db, rhs, ctx, true);
    }
    match db.ast.get(node).clone() {
        Struct::List(children) => children
            .iter()
            .any(|&c| subtree_has_conditional_abortive(db, c, ctx, under_cond)),
        Struct::Atom(_) => false,
    }
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

/// Whether `t` is an UNDETERMINED type (an `Any` or a free `Var`) — an abort-type comparison over an
/// undetermined side is inconclusive, so callers treat it as "allow" rather than a definite mismatch.
fn undetermined_ty(t: &crate::ty::Ty) -> bool {
    matches!(t, crate::ty::Ty::Any | crate::ty::Ty::Var(_))
}

/// The VALUE TYPE an abortive perform in `node` collapses to — the type of the abortive arm's BODY (which
/// becomes the abort value). Returns `Some(ty)` when `node` contains exactly one abortive op whose arm
/// body type is determinable; `None` if it finds none (or the arm/type is unavailable). Used by the hoist
/// to check that distributing an enclosing op keeps the produced `if` branches type-consistent.
fn abortive_perform_value_ty(
    db: &mut Db,
    node: StructId,
    ctx: &HandlerCtx,
) -> Option<crate::ty::Ty> {
    if let Resolved::Apply { head, .. } = resolved_of(db, node)
        && let Some(id) = is_perform(db, head, ctx)
        && ctx.abortive.contains(&id)
    {
        let arm_body = ctx.arms.get(&id)?.body;
        return Some(crate::infer::type_of(db, arm_body));
    }
    match db.ast.get(node).clone() {
        Struct::List(children) => children
            .iter()
            .find_map(|&c| abortive_perform_value_ty(db, c, ctx)),
        Struct::Atom(_) => None,
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
                    // TYPE-SAFE distribution. After the hoist, the aborting branch `(op … (Bail v) …)`
                    // COLLAPSES to the abort value (its type = the abortive arm's body type), while the
                    // other branch `(op … e …)` keeps the op's RESULT type. Those must AGREE or the produced
                    // `if` is ill-typed → invalid wasm (`(tuple 1 (if c (Bail.bail 7) 5))`: op result is a
                    // tuple, abort value is Int64). Only distribute when the op's result type equals the
                    // abort value type; otherwise leave it (the guard declines). Undetermined either side →
                    // allow (the scalar cases where inference hasn't ground both, matching prior behavior).
                    let op_result = crate::infer::type_of(db, node);
                    let abort_ty = abortive_perform_value_ty(db, a, ctx);
                    let types_agree = match abort_ty {
                        Some(at) => {
                            undetermined_ty(&op_result) || undetermined_ty(&at) || op_result == at
                        }
                        None => true, // no single abort value type found — fall through, guard decides
                    };
                    if others_pure && cond_pure && types_agree {
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
    // A `(let (b0… (k (if c t e)) b1…) body)` whose binding INIT is an `(if …)` carrying an abort: the init
    // is a NON-tail strict position (its value feeds the binder `k`), so the per-branch capture can't reach
    // it in place. But an abort ABANDONS everything, so lift the `if` OUT of the let, distributing the whole
    // let into each branch with the init replaced by that branch's value: `(let (… (k (if c t e)) …) body)`
    // ≡ `(if c (let (… (k t) …) body) (let (… (k e) …) body))`. Sound when `c` and the PRECEDING binding
    // inits (b0…) are pure — they are duplicated across the two branches (a later binding sees earlier ones,
    // so we cannot reorder past an effectful earlier init). The aborting branch's `(k t)` init is then an
    // unconditional abort the fold collapses; the other branch keeps the let. Only the FIRST such init is
    // lifted per pass (the fixpoint handles the rest).
    if let Some(form) = db.ast.as_form(node, "let").map(|t| t.to_vec())
        && form.len() == 2
    {
        let (bindings_occ, body_occ) = (form[0], form[1]);
        if let Struct::List(pairs) = db.ast.get(bindings_occ).clone() {
            for (bi, &pair) in pairs.iter().enumerate() {
                if let Struct::List(kv) = db.ast.get(pair).clone()
                    && kv.len() == 2
                    && let Resolved::If { cond, then_, else_ } = resolved_of(db, kv[1])
                    && subtree_has_abortive_perform(db, kv[1], ctx)
                {
                    // `cond` and every PRECEDING init must be pure (duplicated across the two branches; an
                    // effectful earlier binding cannot be run twice). A later init / the body may perform —
                    // it is copied whole into each branch unchanged (run once per branch, as before).
                    let cond_pure = !subtree_performs(db, cond, ctx);
                    let preceding_pure = pairs[..bi].iter().all(|&p| match db.ast.get(p).clone() {
                        Struct::List(pkv) if pkv.len() == 2 => !subtree_performs(db, pkv[1], ctx),
                        _ => true,
                    });
                    if cond_pure && preceding_pure {
                        // Rebuild the whole `let` with binding `bi`'s init replaced by `branch`.
                        let rebuild = |db: &mut Db, branch: StructId| -> StructId {
                            let name = kv[0];
                            let new_pair = db.push_list(vec![name, branch]);
                            let new_pairs: Vec<StructId> = pairs
                                .iter()
                                .enumerate()
                                .map(|(j, &p)| if j == bi { new_pair } else { p })
                                .collect();
                            let new_bindings = db.push_list(new_pairs);
                            let let_head = db.push_atom(Leaf::Name("let".to_string()));
                            db.push_list(vec![let_head, new_bindings, body_occ])
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

/// Lift a RESUMPTIVE `if`/`match` (whose taken branch performs a discharged op) out of a strict
/// continuation position, distributing the continuation into both branches to a fixpoint, so the
/// conditional ends up in TAIL position where the tail-resume fold threads state correctly. See the
/// call site in `reduce_handle` for the value-preservation argument. `None`-free (returns the rewritten
/// tree, or the input unchanged when no site is found).
fn hoist_resumptive_conditional(db: &mut Db, node: StructId, ctx: &HandlerCtx) -> StructId {
    let mut cur = node;
    // Each pass lifts at least one conditional one level toward tail position; a body nests far fewer
    // than this many strict positions. The bound prevents any accidental non-convergence.
    for _ in 0..256 {
        match hoist_resumptive_once(db, cur, ctx) {
            Some(next) => cur = next,
            None => break,
        }
    }
    cur
}

/// Whether `node` is an `if`/`match` whose SELECTED-branch position (a branch of an `if`, or an arm body
/// of a `match`) contains a perform of an op this handler discharges — the shape whose branch-local state
/// advance the tail fold drops to a continuation. The condition/scrutinee performing is irrelevant here
/// (that threads fine); only a BRANCH/ARM-BODY perform is the hoist trigger.
fn conditional_branch_performs(db: &mut Db, node: StructId, ctx: &HandlerCtx) -> bool {
    match resolved_of(db, node) {
        Resolved::If { then_, else_, .. } => {
            subtree_performs(db, then_, ctx) || subtree_performs(db, else_, ctx)
        }
        Resolved::Match { arms, .. } => arms
            .iter()
            .any(|&(_, body)| subtree_performs(db, body, ctx)),
        _ => false,
    }
}

/// Rebuild `node`'s branch/arm-body positions by mapping each through `f` (which wraps a branch in the
/// distributed continuation). The condition/scrutinee is left UNTOUCHED (evaluated once, not duplicated).
fn map_conditional_branches(
    db: &mut Db,
    node: StructId,
    mut f: impl FnMut(&mut Db, StructId) -> StructId,
) -> StructId {
    match resolved_of(db, node) {
        Resolved::If { cond, then_, else_ } => {
            let nt = f(db, then_);
            let ne = f(db, else_);
            let if_head = db.push_atom(Leaf::Name("if".to_string()));
            db.push_list(vec![if_head, cond, nt, ne])
        }
        Resolved::Match { scrutinee, arms } => {
            let match_head = db.push_atom(Leaf::Name("match".to_string()));
            let mut children = vec![match_head, scrutinee];
            for (pat, body) in arms {
                let nbody = f(db, body);
                children.push(db.push_list(vec![pat, nbody]));
            }
            db.push_list(children)
        }
        _ => node,
    }
}

/// One rewrite step of [`hoist_resumptive_conditional`]. Finds the FIRST (pre-order) strict position
/// holding an `if`/`match` whose branch performs, distributes the enclosing strict context into the
/// branches, and returns the rewritten WHOLE tree. Two site shapes (mirroring the abortive hoist):
///   * a NON-TAIL `do` item `(do a0 … (if c t e) … an)` → `(do a0 … (if c (do t … an) (do e … an)))`,
///     the conditional and everything AFTER it moved into each branch (it was a strict item, so it and
///     its continuation always run). Every item BEFORE it must be pure (they run before the condition;
///     duplicating them is not needed — they stay in the outer `do` prefix — but the conditional must
///     become the do's tail, so items before it that PERFORM would still be threaded strictly, which is
///     fine; the real constraint is only that we don't reorder — handled by keeping the prefix intact).
///   * a strict application operand `(op a0 … (if c t e) … ak)` → `(if c (op … t …) (op … e …))`, with
///     every operand BEFORE the conditional pure (else the condition would jump ahead of an earlier
///     perform) and the head perform-free. Operands AFTER may perform (duplicated, run once per taken
///     branch, in order).
fn hoist_resumptive_once(db: &mut Db, node: StructId, ctx: &HandlerCtx) -> Option<StructId> {
    // Site 1: a `do` whose a NON-LAST item is a branch-performing conditional. Move that item and every
    // following item into each branch as a `(do <branch> rest…)`; keep the preceding items as the do's
    // prefix. Reduces to the tail-position shape one conditional at a time.
    if let Some(items) = db.ast.as_form(node, "do").map(|t| t.to_vec())
        && items.len() >= 2
    {
        for (i, &it) in items.iter().enumerate() {
            if i + 1 < items.len() && conditional_branch_performs(db, it, ctx) {
                let rest: Vec<StructId> = items[i + 1..].to_vec();
                // Wrap a branch in `(do <branch> rest…)` — the continuation after the conditional.
                let wrap = |db: &mut Db, branch: StructId| -> StructId {
                    let do_head = db.push_name("do");
                    let mut ch = vec![do_head, branch];
                    ch.extend_from_slice(&rest);
                    db.push_list(ch)
                };
                let new_cond = map_conditional_branches(db, it, wrap);
                // The rewritten `do` keeps items[..i] as a prefix, with the distributed conditional as its
                // new tail. If i==0 there is no prefix and the conditional IS the whole do's value.
                if i == 0 {
                    return Some(new_cond);
                }
                let do_head = db.push_name("do");
                let mut ch = vec![do_head];
                ch.extend_from_slice(&items[..i]);
                ch.push(new_cond);
                return Some(db.push_list(ch));
            }
        }
    }
    // Site 2: a strict application `(op a0 … ak)` — head not a perform — with a branch-performing
    // conditional operand and every PRECEDING operand pure. Distribute the op into the branches.
    if let Resolved::Apply { head, args } = resolved_of(db, node)
        && is_perform(db, head, ctx).is_none()
        && !subtree_performs(db, head, ctx)
    {
        for (i, &a) in args.iter().enumerate() {
            if conditional_branch_performs(db, a, ctx) {
                // Every operand BEFORE the conditional must be pure — else distributing moves the `if`
                // condition ahead of an earlier perform, reordering effects. Operands AFTER may perform
                // (they are duplicated into each branch but only one branch runs, preserving order/count).
                let preceding_pure = args[..i].iter().all(|&b| !subtree_performs(db, b, ctx));
                if preceding_pure {
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
                    return Some(map_conditional_branches(db, a, rebuild));
                }
            }
        }
    }
    // Site 3: a SHORT-CIRCUIT connective `(and lhs rhs)` / `(or lhs rhs)` whose RHS performs is itself a
    // conditional in disguise — `rhs` runs only on one value of `lhs`, so its perform's state advance is a
    // branch-local one that the strict two-operand threading would drop to a continuation. Desugar it to
    // the equivalent `if` (`(and lhs rhs)` ≡ `(if lhs rhs false)`, `(or lhs rhs)` ≡ `(if lhs true rhs)`)
    // so the next pass sees a branch-performing `if` and lifts it out of its continuation (Site 1/2). `lhs`
    // becomes the `if` CONDITION — evaluated exactly once either way, so no duplication and no purity
    // constraint on it. Only fires when `rhs` performs (a pure connective threads/copies wholesale).
    if let Resolved::And { lhs, rhs, is_and } = resolved_of(db, node)
        && subtree_performs(db, rhs, ctx)
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
    // Not a site here — recurse into children, rebuilding with the FIRST rewritten child (so a
    // conditional nested inside a `let` init / branch / arm is lifted within that sub-position, then the
    // enclosing pass lifts it further if needed).
    if let Struct::List(children) = db.ast.get(node).clone() {
        for (k, &c) in children.iter().enumerate() {
            if let Some(new_c) = hoist_resumptive_once(db, c, ctx) {
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
    // A CROSS-FUNCTION call reaching a CONDITIONAL abort in a NON-TAIL position is unsound. The abort lives
    // inside the callee's body under an `if`/`match`/connective (opaque here — the hoist lifts only
    // SYNTACTIC conditionals, so it cannot see it), so after the inline arm β-reduces the callee the abort
    // surfaces as a non-tail conditional the hoist never lifted — and `thread`'s `if` arm would capture it
    // PER-BRANCH as if the `if` were the handle's tail, dropping the enclosing op (`(+ 10 (check -1))` →
    // 109 instead of 99, a MISCOMPILE). Flag it so `reduce_handle` DECLINES: this needs the non-local-exit
    // calling convention (a later vertical). An UNCONDITIONAL cross-fn abort (`(+ 10 (boom 99))`, the
    // callee is a bare abort) is NOT flagged — inlining yields a plain strict abort E4-a collapses soundly.
    if !tail
        && let Resolved::Apply { head, .. } = resolved_of(db, node)
        && is_perform(db, head, ctx).is_none()
        && call_reaches_conditional_abortive(db, head, ctx)
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
    // tail-ness + `under_cond`. Each INIT is a strict operand. KEY (post-hoist, mirroring the generic-op
    // case): an UNCONDITIONAL abort in an init INSIDE a tail branch is CAPTURABLE — when the init aborts the
    // whole let collapses to that value, which `thread_branch_local_abort` takes per-branch — so init `bi`
    // is capturable-tail iff we are on a `tail` path AND every PRECEDING init is perform-free (an effectful
    // earlier init runs before the abort and cannot be dropped; a conditional abort in an init was already
    // lifted out of the let by the hoist, so a surviving one is unconditional). The BODY inherits the let's
    // tail-ness + `under_cond`.
    if let Some(form) = db.ast.as_form(node, "let").map(|t| t.to_vec())
        && form.len() == 2
    {
        let (bindings_occ, body_occ) = (form[0], form[1]);
        if let Struct::List(pairs) = db.ast.get(bindings_occ).clone() {
            for (bi, &pair) in pairs.iter().enumerate() {
                if let Struct::List(kv) = db.ast.get(pair).clone()
                    && kv.len() == 2
                {
                    let preceding_pure = pairs[..bi].iter().all(|&p| match db.ast.get(p).clone() {
                        Struct::List(pkv) if pkv.len() == 2 => !subtree_performs(db, pkv[1], ctx),
                        _ => true,
                    });
                    let init_tail = tail && preceding_pure;
                    if body_has_unsound_abortive_perform(db, kv[1], ctx, init_tail, under_cond) {
                        return true;
                    }
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
                // BUT a CROSS-FUNCTION CONDITIONAL-abortive operand is NOT capturable even so — its abort is
                // opaque to the hoist (hidden under an `if` in the callee), so after the inline arm surfaces
                // it the per-branch capture would wrongly drop THIS enclosing op (`(+ 10 (check -1))` →
                // 109). Deny it capturable-tail so the cross-fn-reach check above flags it (`!tail`).
                let cross_fn_abort = matches!(resolved_of(db, c), Resolved::Apply { head, .. }
                    if is_perform(db, head, ctx).is_none() && call_reaches_conditional_abortive(db, head, ctx));
                let capturable = tail && !sc_right && !cross_fn_abort && siblings_pure(db, i);
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
/// Pop every pending self-call temp pushed at or above `mark` (LIFO) and wrap `inner` in one `(let ((temp
/// call)) …)` per temp — LAST-pushed = INNERMOST, so a later temp's init (which may read an earlier temp's
/// `(. t 1)` out-state) is in scope. Returns the wrapped node (just `inner` when nothing was pending).
/// The multi-value counterpart of the ordinary let-init threading: a self-call's out-state is a RUNTIME
/// value, so each self-call must be bound before its projections can be used.
fn drain_and_wrap(db: &mut Db, ctx: &HandlerCtx, mark: usize, inner: StructId) -> StructId {
    let entries: Vec<(String, StructId)> = ctx.pending.borrow_mut().split_off(mark);
    let mut acc = inner;
    for (name, init) in entries.into_iter().rev() {
        let let_head = db.push_atom(Leaf::Name("let".to_string()));
        let name_atom = db.push_atom(Leaf::Name(name));
        let pair = db.push_list(vec![name_atom, init]);
        let bindings = db.push_list(vec![pair]);
        acc = db.push_list(vec![let_head, bindings, acc]);
    }
    acc
}

/// Package a leaf tail's `(value, out-states…)` as a tuple constructor `("tuple" value s0' s1' …)` — the
/// multi-value return shape `f#ctx` yields so a caller's self-call can project the value (`.0`) and thread
/// each slot's advanced out-state (`.{slot+1}`) forward.
fn build_value_state_tuple(db: &mut Db, value: StructId, out_states: &[StructId]) -> StructId {
    let head = db.push_atom(Leaf::Str("tuple".to_string()));
    let mut children = vec![head, value];
    children.extend_from_slice(out_states);
    db.push_list(children)
}

/// Whether any self-call to `callee_def` in `node` sits UNDER a conditional (`if`/`match`/`and`/`or`) — a
/// position `thread_returning_tuple`'s leaf threader cannot bind soundly (a self-call in a branch pushes a
/// pending temp the branch-local `if`/`match` threading would not drain into that branch's scope). Such a
/// leaf DECLINES (multi-value v1 handles self-calls only on the UNCONDITIONAL strict spine — operator
/// operands, `let` inits/body, tuple/list/record elements, perform args). A self-call reached only through
/// those strict forms is fine; one gated behind a conditional is not (yet).
fn selfcall_under_conditional(db: &mut Db, node: StructId, callee_def: usize) -> bool {
    match resolved_of(db, node) {
        Resolved::If { cond, then_, else_ } => {
            // A self-call in the CONDITION is still on the strict spine (evaluated unconditionally); one in
            // a BRANCH is gated. Recurse the condition normally; flag a self-call anywhere in a branch.
            selfcall_under_conditional(db, cond, callee_def)
                || contains_self_call(db, then_, callee_def)
                || contains_self_call(db, else_, callee_def)
        }
        Resolved::Match { scrutinee, arms } => {
            selfcall_under_conditional(db, scrutinee, callee_def)
                || arms
                    .iter()
                    .any(|&(_, body)| contains_self_call(db, body, callee_def))
        }
        Resolved::And { lhs, rhs, .. } => {
            // `and`/`or` short-circuit: the rhs is conditional. A self-call in lhs is unconditional.
            selfcall_under_conditional(db, lhs, callee_def)
                || contains_self_call(db, rhs, callee_def)
        }
        _ => match db.ast.get(node).clone() {
            Struct::List(children) => children
                .iter()
                .any(|&c| selfcall_under_conditional(db, c, callee_def)),
            Struct::Atom(_) => false,
        },
    }
}

/// Whether `body` is threadable by `thread_returning_tuple` — a PRE-CHECK mirroring its structure so the
/// mode decision can decline UP FRONT (before `specialize_recursive` reserves the def), avoiding an orphan
/// bodyless spec. Descends `if`/`match` to their leaf tail expressions exactly as `thread_returning_tuple`
/// does; at each LEAF, a self-call gated behind a conditional is unthreadable (v1 handles self-calls only on
/// the unconditional strict spine). Returns `false` if any leaf has such a gated self-call.
fn multivalue_leaves_threadable(db: &mut Db, body: StructId, callee_def: usize) -> bool {
    match resolved_of(db, body) {
        // An `if`/`match` DISPATCH: `thread_returning_tuple` descends each branch/arm body (its own tail) AND
        // threads the CONDITION/SCRUTINEE with `thread_bounded`, then `drain_and_wrap` wraps any pending
        // self-call temps from the cond/scrutinee AROUND the whole `if`/`match`. Hoisting a temp out is sound
        // ONLY when the self-call is on the cond/scrutinee's UNCONDITIONAL strict spine — the cond always
        // evaluates, so a self-call directly in it always runs and lifting its binding changes nothing. But a
        // self-call GATED behind a NESTED conditional/short-circuit inside the cond (`(if (if g (walk …) 0)
        // …)`) runs only on some paths; hoisting its temp makes it run UNCONDITIONALLY and thread state as if
        // always taken — an eval-order MISCOMPILE (PR #456, Copilot). So the cond/scrutinee must have no
        // self-call under a conditional. (A self-call directly on the cond's strict spine, e.g. `(< (walk …)
        // 100)`, is fine — `selfcall_under_conditional` returns false for it.)
        Resolved::If { cond, then_, else_ } => {
            !selfcall_under_conditional(db, cond, callee_def)
                && multivalue_leaves_threadable(db, then_, callee_def)
                && multivalue_leaves_threadable(db, else_, callee_def)
        }
        Resolved::Match { scrutinee, arms } => {
            !selfcall_under_conditional(db, scrutinee, callee_def)
                && arms
                    .iter()
                    .all(|&(_, arm_body)| multivalue_leaves_threadable(db, arm_body, callee_def))
        }
        _ => !selfcall_under_conditional(db, body, callee_def),
    }
}

/// Thread `body` in MULTI-VALUE mode (repro-1): the synthesized `f#ctx` returns `(value, out-state-per-slot)`
/// at every tail, so a caller's self-call can thread the recursion's advanced state to a LATER sibling. The
/// walk descends through `if`/`match` (each branch/arm body is its own tail, producing its own tuple under
/// its own out-state); at a LEAF tail expression it threads with `thread_bounded` (self-calls there push
/// pending temps via the multi-value self-call arm), then drains those temps into wrapping `let`s and
/// packages `("tuple" value out-states…)`. Returns `None` (clean decline) for a leaf whose self-call sits
/// under a conditional — a position v1 does not bind soundly. `ctx.multivalue` must be set by the caller.
fn thread_returning_tuple(
    db: &mut Db,
    body: StructId,
    states: Vec<StructId>,
    ctx: &HandlerCtx,
    callee_def: usize,
) -> Option<StructId> {
    match resolved_of(db, body) {
        // An `if`: thread the CONDITION (a perform there threads state to both branches; a self-call there
        // is unconditional and drained around the whole `if`), then each branch is its OWN tail — recurse,
        // giving each a fresh copy of the post-condition state (single-parent-arena discipline, as the
        // ordinary `if` thread arm does). Rebuild `(if rcond then-tuple else-tuple)`, wrapping any
        // condition-level self-call temps around it.
        Resolved::If { cond, then_, else_ } => {
            let mark = ctx.pending.borrow().len();
            let (rcond, cur) = thread_bounded(db, cond, states, ctx, 0)?;
            let then_states: Vec<StructId> = cur.iter().map(|&s| copy_pure(db, s)).collect();
            let else_states: Vec<StructId> = cur.iter().map(|&s| copy_pure(db, s)).collect();
            let rthen = thread_returning_tuple(db, then_, then_states, ctx, callee_def)?;
            let relse = thread_returning_tuple(db, else_, else_states, ctx, callee_def)?;
            let if_head = db.push_atom(Leaf::Name("if".to_string()));
            let if_node = db.push_list(vec![if_head, rcond, rthen, relse]);
            Some(drain_and_wrap(db, ctx, mark, if_node))
        }
        // A `match`: thread the SCRUTINEE, then each arm BODY is its own tail — recurse under a fresh copy of
        // the post-scrutinee state. The PATTERN is a binder position (copied structurally). Rebuild `(match
        // rscrut (rpat body-tuple)…)`, wrapping any scrutinee-level self-call temps around it.
        Resolved::Match { scrutinee, arms } => {
            let mark = ctx.pending.borrow().len();
            let (rscrut, cur) = thread_bounded(db, scrutinee, states, ctx, 0)?;
            let match_head = db.push_atom(Leaf::Name("match".to_string()));
            let mut children = vec![match_head, rscrut];
            for (pat, arm_body) in arms {
                let rpat = copy_pure(db, pat);
                let arm_states: Vec<StructId> = cur.iter().map(|&s| copy_pure(db, s)).collect();
                let rbody = thread_returning_tuple(db, arm_body, arm_states, ctx, callee_def)?;
                children.push(db.push_list(vec![rpat, rbody]));
            }
            let match_node = db.push_list(children);
            Some(drain_and_wrap(db, ctx, mark, match_node))
        }
        // A LEAF tail expression — an operator/operand spine, a `let`, a tuple/ctor, a bare value, or a
        // perform. Thread it: any self-call on its unconditional strict spine pushes a pending temp (the
        // multi-value self-call arm), and a later sibling threads against that temp's `(. t 1)` out-state.
        // A self-call GATED behind a conditional here is unhandled in v1 — decline cleanly.
        _ => {
            if selfcall_under_conditional(db, body, callee_def) {
                return None;
            }
            let mark = ctx.pending.borrow().len();
            let (value, out_states) = thread_bounded(db, body, states, ctx, 0)?;
            let tuple = build_value_state_tuple(db, value, &out_states);
            Some(drain_and_wrap(db, ctx, mark, tuple))
        }
    }
}

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
            // EFFECT-DUPLICATION GUARD. The arm body β-reduces by SUBSTITUTING each argument for its
            // parameter, so a param used more than once COPIES its argument. If that argument reaches a
            // perform — of THIS handler's op OR a FOREIGN one (`arg_reaches_any_perform`) — duplicating it
            // would run the effect once per use: `(E.op (tuple (A.get) (A.get)))` whose arm reads `(. p 0)`
            // AND `(. p 1)` would thread four gets, not two (a miscompile). Decline instead — this fold
            // cannot represent it without a let-binding the tail surface does not model. A param whose arg
            // reaches NO perform (a literal, a name, a PURE compound like `(record (a 3) (b 4))`) duplicates
            // no effect, so multi-use is fine; a single-use param likewise runs its arg exactly once. This
            // mirrors the applied-lambda pre-reduction's pure-args soundness guard. (`arg_reaches_any_perform`
            // is used, NOT `body_reaches_foreign_perform`, because the latter over-reports a record literal's
            // field-pair as an unresolvable call — spuriously declining a pure record argument.)
            if arm.params.len() == rewritten_args.len() {
                for (&p, &a) in arm.params.iter().zip(&rewritten_args) {
                    if !is_unit_param(db, p)
                        && arg_reaches_any_perform(db, a, ctx)
                        && count_param_refs(db, arm.body, p) > 1
                    {
                        return None;
                    }
                }
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
                // FIRST abort wins. Threading proceeds in EVALUATION ORDER (left-to-right), so the first
                // abortive perform to fire is the leftmost — and it ABANDONS the rest of the computation,
                // including any later abortive perform on the same strict spine. If the cell is ALREADY set
                // (an earlier operand aborted), that earlier value stands: do NOT overwrite it, and return
                // it so this dead position carries the surviving value. `(+ (Bail.bail 7) (Bail.bail 9))`
                // must yield 7 (the first), never 9 — without this guard the second perform overwrote the
                // cell as threading continued past the first abort (a miscompile).
                if let Some(existing) = ctx.abort_value.get() {
                    return Some((existing, cur));
                }
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
            let (value, next_state) = peel_resume_from_arm_body(db, arm_body)?;
            // `value`/`next_state` are the resume node's own CHILDREN, so their `parent_of` still points at
            // that (now-discarded) `resume` node — an orphan whose parent chain does NOT reach the threaded
            // body's enclosing `(def …)`. If either carries a NAME reference (e.g. a state-threading arm's
            // `(resume s (+ s 1))`, where the substituted state `s` is a reference to the specialization's
            // `$s{k}` state param), splicing it elsewhere leaves that reference resolving against the dead
            // resume node's scope → a spurious CDZ0101 leaking the internal `walk#eff2$s0` name. COPY them
            // (a re-parenting structural copy) so each is detached from the dead resume and receives fresh
            // parentage when spliced into the value/next-state position. (The `do`-wrapped `value` is
            // already a fresh `push_list`; copying it again is a harmless re-parent.)
            //
            // DISTINCT DEEP copies are load-bearing when `value` and `next_state` are the SAME node — a
            // `resume(a, a)` arm (dispatch/done handing the op's arg back AND as the next state) has both
            // children equal, AND when that shared child (or a leaf within it) is a RESOLVE-PINNED bare name
            // (`a` substituted by an inlined helper's arg — `turn(fuel)` → `a`↦`fuel`, a pinned occurrence),
            // `copy_pure`/`beta_reduce` SHARES it (its pinned-name fast-path returns the node as-is to avoid
            // exponential re-resolution on deep inline chains). Sharing puts ONE node in TWO positions (the
            // `+` operand AND the self-call's trailing state arg) — a single-parent-arena orphan: `parent_of`
            // points at only one, so the other occurrence resolves against a foreign scope → CDZ0101 unbound
            // (the effectful-helper-in-a-self-call-arg bug, v-agent-harness Inc-3). `deep_fresh_copy` re-pushes
            // EVERY node fresh (no shared leaf), so each splice gets its own subtree that re-resolves against
            // the specialized def's sig (which carries the driver's own params).
            let value = deep_fresh_copy(db, value);
            let next_state = deep_fresh_copy(db, next_state);
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
            // CAPTURED enclosing-fn params come AFTER the original args and BEFORE the state args (the sig
            // layout). Each is passed as a fresh bare-name reference: inside `f#ctx` it resolves to the new
            // capture param; at the INITIAL call from the handle body it resolves to the enclosing fn's param
            // (`run-with`'s `tool`). They are CONSTANT across the recursion, so the same name is passed every
            // call — no threading. This one arm handles both the internal self-calls and the initial call.
            if let Some(captures) = db.effect_spec_captures.get(&spec).cloned() {
                for name in captures {
                    rargs.push(db.push_atom(Leaf::Name(name)));
                }
            }
            rargs.extend(cur.iter().copied()); // one trailing state arg per slot, in slot order
            // Build the call `(<spec-name> args… state…)`. The specialized def is named, so a name atom
            // resolves to it (via `def_by_name`), and the ordinary recursive `Core::Call` + reachability
            // path emits it.
            let name_atom = db.push_atom(Leaf::Name(spec.clone()));
            let mut call = vec![name_atom];
            call.extend(rargs);
            let call_node = db.push_list(call);
            if db.multivalue_specs.contains(&spec) {
                // MULTI-VALUE MODE (repro-1): `f#ctx` returns `(value, out-state-per-slot)`. The call's
                // OUT-state is a RUNTIME value (not a symbolic expr of the incoming state, as a perform's
                // is), so it must be LET-BOUND before it can be projected and threaded forward. Bind the
                // call to a fresh temp `t`, register the binding in `ctx.pending` (a leaf tail-expr drains
                // it into wrapping `let`s), and RETURN `(. t 0)` as the call's VALUE with `[(. t 1)…]` as
                // the NEW state per slot. This is what makes a LATER sibling self-call / perform thread
                // against THIS call's advanced out-state (`(. t 1)`), not the un-advanced incoming state.
                let k = ctx.temp_ctr.get();
                ctx.temp_ctr.set(k + 1);
                let tname = format!("{spec}$t{k}");
                ctx.pending.borrow_mut().push((tname.clone(), call_node));
                let value = tuple_proj(db, &tname, 0);
                let new_states: Vec<StructId> = (0..cur.len())
                    .map(|slot| tuple_proj(db, &tname, (slot + 1) as u32))
                    .collect();
                return Some((value, new_states));
            }
            // The call's VALUE is the specialized fn's result; the states after it are not observed (the
            // corpus never reads post-recursion state — the single-return shape).
            Some((call_node, cur))
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
            // DEEP-FRESH the reduced body before threading it. `apply_lambda`/`beta_reduce` SUBSTITUTES the
            // (threaded) args for the callee's params by returning each arg node AS-IS (its pinned-name fast
            // path), so a substituted arg that is a RESOLVE-PINNED reference to a DRIVER param — the caller's
            // `acc` spliced into an inlined helper `turn(a, acc) = acc + …` — keeps its pin to the caller's
            // (now-dead) scope. When this inline happens INSIDE a recursive self-call's arg, the reduced body
            // lands in the synthesized `f#ctx` def, where that pinned `acc` no longer resolves → CDZ0101
            // unbound (v-agent-harness Inc-3, the helper-references-a-driver-param sub-case). A fully-fresh
            // copy drops the stale pins so every name re-resolves against the specialized def's sig (which
            // carries the driver's params). Harmless for the non-nested inline (a fresh copy of a body whose
            // refs already resolve re-resolves to the same bindings).
            let reduced = deep_fresh_copy(db, reduced);
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
            //
            // Each branch gets its OWN FRESH COPY of the incoming state-ref nodes. Both branches EMBED the
            // state (a perform substitutes it into a resume value; a recursive/mutual call appends it as a
            // trailing state arg), and the arena is single-parent — so sharing one state-ref node across
            // both branches orphans whichever is parented second, leaking the internal `f#ctx$s0` name in a
            // CDZ0101 (the mutually-recursive-effect case where the perform is in one branch and the mutual
            // call in the other). Copying per branch gives each its own node.
            let then_states: Vec<StructId> = cur.iter().map(|&s| copy_pure(db, s)).collect();
            let else_states: Vec<StructId> = cur.iter().map(|&s| copy_pure(db, s)).collect();
            let rthen = thread_branch_local_abort(db, then_, then_states, ctx, inline_depth)?;
            let relse = thread_branch_local_abort(db, else_, else_states, ctx, inline_depth)?;
            let if_head = db.push_atom(Leaf::Name("if".to_string()));
            Some((db.push_list(vec![if_head, rcond, rthen, relse]), cur))
        }
        // A `(match scrutinee (pattern body)…)` — the analogue of `if` for the pattern engine. Thread the
        // SCRUTINEE (a perform there reads/threads state, `(match (Get.next) …)`), then rewrite each arm:
        // the PATTERN is a binder position (copied structurally, never threaded — like a `let` binder), the
        // BODY is threaded under the post-scrutinee state (only one arm runs, so each sees the same incoming
        // state, mirroring the `if` branches). An abortive perform in an arm BODY tail is branch-local — the
        // `match` IS the handle body's value, so per-arm the abort yields the arm value — captured by
        // `thread_branch_local_abort` (which restores the cell so a sibling arm / the handle is not
        // collapsed). The out-state is the post-scrutinee state (the single-return shape does not observe a
        // per-arm out-state). Rebuild the same `(match rscrut (pat rbody)…)` form so the pattern engine
        // lowers it by the ordinary path.
        Resolved::Match { scrutinee, arms } => {
            let (rscrut, cur) = thread_bounded(db, scrutinee, states, ctx, inline_depth)?;
            let match_head = db.push_atom(Leaf::Name("match".to_string()));
            let mut children = vec![match_head, rscrut];
            for (pat, body) in arms {
                // The pattern binds names for the arm body (a binder position) — copy it structurally so it
                // is self-contained, exactly as a `let` binder name is copied (never substituted/threaded).
                let rpat = copy_pure(db, pat);
                // Each arm gets its OWN FRESH COPY of the incoming state-refs — the same single-parent-arena
                // reason as the `if` branches: an arm body EMBEDS the state (a perform substitutes it into a
                // resume value; a recursive/mutual call appends it as a trailing state arg), so sharing one
                // state-ref node across arms orphans whichever is parented second, leaking the internal
                // `f#ctx$s0` name (a mutual group dispatched by `match` with the perform in one arm and the
                // mutual call in another). Copying per arm gives each its own node.
                let arm_states: Vec<StructId> = cur.iter().map(|&s| copy_pure(db, s)).collect();
                let rbody = thread_branch_local_abort(db, body, arm_states, ctx, inline_depth)?;
                children.push(db.push_list(vec![rpat, rbody]));
            }
            Some((db.push_list(children), cur))
        }
        // A short-circuit connective `(and lhs rhs)` / `(or lhs rhs)` whose rhs runs only conditionally on
        // `lhs`. Threading it as a strict two-operand form would evaluate rhs's perform even when `lhs`
        // short-circuits — an observable-effect change. Instead DESUGAR to the equivalent `if` (`(and lhs
        // rhs)` ≡ `(if lhs rhs false)`, `(or lhs rhs)` ≡ `(if lhs true rhs)`) and re-thread, so rhs is a
        // branch (threaded under the post-`lhs` state, run only on the taken path). `lhs` becomes the `if`
        // condition — evaluated exactly once either way. Only reached when a perform is inside (a pure
        // connective is copied wholesale by the pure-subtree arm below).
        Resolved::And { lhs, rhs, is_and } => {
            let if_head = db.push_atom(Leaf::Name("if".to_string()));
            let (then_, else_) = if is_and {
                let f = db.push_atom(Leaf::Bool(false));
                (rhs, f)
            } else {
                let t = db.push_atom(Leaf::Bool(true));
                (t, rhs)
            };
            let desugared = db.push_list(vec![if_head, lhs, then_, else_]);
            thread_bounded(db, desugared, states, ctx, inline_depth)
        }
        // A negation `(not operand)` — a STRICT one-operand form. Thread the operand (a perform there
        // reads/threads state, `(not (= (Get.next) 0))`), then rebuild `(not roperand)`.
        Resolved::Not { operand } => {
            let (roperand, cur) = thread_bounded(db, operand, states, ctx, inline_depth)?;
            let not_head = db.push_atom(Leaf::Name("not".to_string()));
            Some((db.push_list(vec![not_head, roperand]), cur))
        }
        // A tuple PROJECTION `(. operand index)` — STRICT one-operand. Thread the operand (a perform there
        // reads/threads state, `(. (tuple (Get.next) (Get.next)) 1)`), rebuild the same projection. The
        // index is a literal (copied structurally). `push_list` with the same `.`-head + index re-forms it.
        Resolved::Proj { operand, index } => {
            let (roperand, cur) = thread_bounded(db, operand, states, ctx, inline_depth)?;
            let dot = db.push_atom(Leaf::Name(".".to_string()));
            let idx_atom = db.push_atom(Leaf::Int {
                value: IntValue::from_i64(index as i64),
                radix: Radix::Dec,
            });
            Some((db.push_list(vec![dot, roperand, idx_atom]), cur))
        }
        // Member access `(. operand key)` — STRICT one-operand (the key is a label, not a value). Thread
        // the operand; rebuild `(. roperand key)` with the key copied as a bare name atom.
        Resolved::Member { operand, key } => {
            let (roperand, cur) = thread_bounded(db, operand, states, ctx, inline_depth)?;
            let dot = db.push_atom(Leaf::Name(".".to_string()));
            let key_atom = db.push_atom(Leaf::Name(key.name.clone()));
            Some((db.push_list(vec![dot, roperand, key_atom]), cur))
        }
        // An annotation `(: expr T)` — STRICT one-operand (the type is not runtime code). Thread `expr`,
        // rebuild `(: rexpr T)` with the type expression copied structurally.
        Resolved::Annot { expr, ty_expr } => {
            let (rexpr, cur) = thread_bounded(db, expr, states, ctx, inline_depth)?;
            let colon = db.push_atom(Leaf::Name(":".to_string()));
            let rty = copy_pure(db, ty_expr);
            Some((db.push_list(vec![colon, rexpr, rty]), cur))
        }
        // A TUPLE / LIST CONSTRUCTOR `("tuple" e0 e1 …)` / `("list" …)` — a STRICT compound constructor:
        // every element is evaluated exactly once, left to right, before the compound is built. So a perform
        // in an element reads/threads state exactly like an arithmetic operand or a call argument (the fold
        // already handles those). Thread each element in order, then rebuild the same constructor with the
        // rewritten elements. This is what threads `(let ((p ("tuple" (Fresh.next) (Fresh.next)))) …)` — the
        // ML tuple/list literal, whose head is the STRING-LITERAL ctor primitive `"tuple"`/`"list"` (a bare
        // `tuple` NAME reduces via `(meta apply)` and threads through the Apply arm; the string-head ctor
        // reaches HERE). The ctor string is re-pushed as a `Leaf::Str` head so the resolver re-recognizes it.
        Resolved::Tuple { elems } | Resolved::List { elems } => {
            let ctor = match resolved_of(db, node) {
                Resolved::List { .. } => "list",
                _ => "tuple",
            };
            let elems: Vec<StructId> = elems.iter().copied().collect();
            let mut cur = states;
            let mut relems = Vec::with_capacity(elems.len());
            for e in elems {
                let (re, next) = thread_bounded(db, e, cur, ctx, inline_depth)?;
                relems.push(re);
                cur = next;
            }
            let head = db.push_atom(Leaf::Str(ctor.to_string()));
            let mut children = vec![head];
            children.extend(relems);
            Some((db.push_list(children), cur))
        }
        // A RECORD CONSTRUCTOR `("record" (label value)…)` — a STRICT compound constructor whose field
        // VALUES are evaluated in written order before the record is built. Thread each field's value (a
        // perform there reads/threads state), keeping the label, and rebuild the same `("record" (label
        // rvalue)…)` form. The companion of the tuple/list arm above for the ML record literal `{ a = …, b
        // = … }`, which lowers to this string-headed ctor with `(label value)` pair args. (Matched on the
        // RAW form via `as_ctor_form` — like the `let` arm — because `Resolved::Record` holds a sorted
        // BTreeMap that loses the written order the source evaluates in.)
        _ if db.ast.as_ctor_form(node, "record").is_some() => {
            let fields: Vec<StructId> = db.ast.as_ctor_form(node, "record").unwrap().to_vec();
            let mut cur = states;
            let mut rfields = Vec::with_capacity(fields.len());
            for field in fields {
                let Struct::List(kv) = db.ast.get(field).clone() else {
                    return None;
                };
                if kv.len() != 2 {
                    return None;
                }
                // The label is copied structurally (a field name, not a value to thread); the VALUE is
                // threaded (it may perform).
                let label_copy = copy_pure(db, kv[0]);
                let (rvalue, next) = thread_bounded(db, kv[1], cur, ctx, inline_depth)?;
                cur = next;
                rfields.push(db.push_list(vec![label_copy, rvalue]));
            }
            let head = db.push_atom(Leaf::Str("record".to_string()));
            let mut children = vec![head];
            children.extend(rfields);
            Some((db.push_list(children), cur))
        }
        // A MAP CONSTRUCTOR `("map" (key value)…)` — a STRICT compound whose entries are evaluated in
        // written order, and within each entry the KEY then the VALUE (both may perform). Thread the key,
        // then the value, per entry, and rebuild the same `("map" (rkey rvalue)…)` form. The map companion
        // of the record arm above (a `(key value)` pair like a record field, but the KEY is a value to
        // thread, not a copied label). Matched on the RAW form (`Resolved::Map`'s `entries` slice is fine
        // too, but the raw form keeps the written order uniformly with the other ctor arms).
        _ if db.ast.as_ctor_form(node, "map").is_some() => {
            let entries: Vec<StructId> = db.ast.as_ctor_form(node, "map").unwrap().to_vec();
            let mut cur = states;
            let mut rentries = Vec::with_capacity(entries.len());
            for entry in entries {
                let Struct::List(kv) = db.ast.get(entry).clone() else {
                    return None;
                };
                if kv.len() != 2 {
                    return None;
                }
                // KEY then VALUE, both threaded (either may perform), in evaluation order.
                let (rkey, next_k) = thread_bounded(db, kv[0], cur, ctx, inline_depth)?;
                let (rvalue, next_v) = thread_bounded(db, kv[1], next_k, ctx, inline_depth)?;
                cur = next_v;
                rentries.push(db.push_list(vec![rkey, rvalue]));
            }
            let head = db.push_atom(Leaf::Str("map".to_string()));
            let mut children = vec![head];
            children.extend(rentries);
            Some((db.push_list(children), cur))
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
            if let Some(merged) = merged_nested_ctx(db, inner_init, &inner_arms, inner_body, ctx) {
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

/// Whether the resolved subtree at `node` performs a discharged operation, following calls into their
/// callee bodies. A syntactic perform of a discharged op is the base case. Follows BOTH non-recursive AND
/// recursive callees — a `visited` set of callee bodies bounds the walk over a (possibly MUTUALLY-recursive)
/// call cycle so it terminates. Following recursive callees is what lets a MUTUALLY-recursive effectful
/// group be detected as reaching the effect: `ev` reaches `Ctr.tick` only THROUGH its recursive partner
/// `od`, so without following `od` the specialize trigger `recursive_call_reaches_discharged(ev)` would read
/// false and `ev` would be copied unthreaded (its perform then hitting the no-home check). Mirrors
/// `body_reached_effects`'s visited-set call-following.
fn body_reaches_discharged(db: &mut Db, node: StructId, ctx: &HandlerCtx, depth: u32) -> bool {
    let mut visited = std::collections::HashSet::new();
    body_reaches_discharged_walk(db, node, ctx, depth, &mut visited)
}

fn body_reaches_discharged_walk(
    db: &mut Db,
    node: StructId,
    ctx: &HandlerCtx,
    depth: u32,
    visited: &mut std::collections::HashSet<StructId>,
) -> bool {
    // Depth backstop — a cross-function chain deeper than this declines (the trigger stays bounded,
    // mirroring the evaluator's reduction guard). The `visited` set already bounds cycles; the depth bound
    // caps a long non-cyclic chain.
    if depth > 64 {
        return false;
    }
    // A syntactic perform of a discharged operation.
    if let Resolved::Apply { head, .. } = resolved_of(db, node)
        && is_perform(db, head, ctx).is_some()
    {
        return true;
    }
    // A call whose callee body reaches a discharged op — follow it (a parameterized OR nullary-def callee),
    // recursive or not. `visited.insert` returns false on re-entry, stopping a self-/mutual-recursive cycle.
    if let Resolved::Apply { head, .. } = resolved_of(db, node)
        && let Some(callee) = crate::eval::lambda_body(db, head)
            .or_else(|| crate::eval::lambda_body_of_nullary(db, head))
        && visited.insert(callee)
        && body_reaches_discharged_walk(db, callee, ctx, depth + 1, visited)
    {
        return true;
    }
    // Otherwise descend into children structurally.
    match db.ast.get(node).clone() {
        Struct::List(children) => children
            .iter()
            .any(|&c| body_reaches_discharged_walk(db, c, ctx, depth, visited)),
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

/// Whether the subtree at `node` MIGHT transitively reach ANY effect operation — a perform of ANY declared
/// effect (`effect_op_of`), a `resume`, or a nested `handle`, following NON-RECURSIVE calls into their
/// bodies (bounded depth). CTX-INDEPENDENT — unlike `subtree_performs` (which asks only about THIS
/// handler's discharged ops), this asks whether the computation is effect-free in the ABSOLUTE sense, the
/// predicate an effect-free continuation `C` needs: `C` may be spliced many times, so it must contain no
/// observable effect of ANY kind. CONSERVATIVE (over-reports, NEVER under-reports — a wrong "effect-free"
/// would duplicate a hidden effect): a RECURSIVE callee, an UNRESOLVABLE call head (a higher-order
/// function-valued parameter, an indirect call — its body is unknown, so it MIGHT perform), or a chain
/// deeper than the depth bound all report `true`. Used to admit a non-recursive, transitively-pure USER
/// call inside `C` — the frame-free generalization of the pure one-hole continuation beyond
/// primitive-only operands (`call_is_effect_free_nonrecursive`).
fn reaches_any_effect(db: &mut Db, node: StructId, depth: u32) -> bool {
    // Depth backstop — a cross-function chain deeper than this is treated as possibly-effectful (a safe
    // over-report), mirroring `body_reaches_discharged`'s bound.
    if depth > 16 {
        return true;
    }
    // A `resume` or a nested `handle` is a control-flow effect.
    if matches!(
        resolved_of(db, node),
        Resolved::Resume { .. } | Resolved::Handle { .. }
    ) {
        return true;
    }
    if let Resolved::Apply { head, args } = resolved_of(db, node) {
        // A perform of ANY declared effect operation (not just a discharged one).
        if crate::eval::effect_op_of(db, head).is_some() {
            return true;
        }
        // A PURE primitive operator (arith/cmp/ctor): its head is effect-free; an effect can only come
        // from an ARGUMENT.
        if is_pure_operator_head(db, head) {
            return args.iter().any(|&a| reaches_any_effect(db, a, depth + 1));
        }
        // A USER call. Resolve the callee to a known def body; an UNRESOLVABLE head (a function-valued
        // parameter / indirect call) MIGHT perform — over-report. A RECURSIVE callee is conservatively
        // effectful (its body cannot be cheaply proven pure). Otherwise the call reaches an effect iff
        // the callee body does OR any argument does.
        let Some(callee) = crate::eval::lambda_body(db, head)
            .or_else(|| crate::eval::lambda_body_of_nullary(db, head))
        else {
            return true; // unknown callee — assume it may perform
        };
        if crate::eval::is_recursive(db, callee) {
            return true;
        }
        return reaches_any_effect(db, callee, depth + 1)
            || args.iter().any(|&a| reaches_any_effect(db, a, depth + 1));
    }
    // Any other shape (`let`/`if`/`match`/tuple/list/…) — descend into children structurally (an
    // over-approximation for a conditional, which is safe: an effect in EITHER branch counts).
    match db.ast.get(node).clone() {
        Struct::List(children) => children
            .iter()
            .any(|&c| reaches_any_effect(db, c, depth + 1)),
        Struct::Atom(_) => false,
    }
}

/// Whether the application `head` names a NON-RECURSIVE user function whose body is transitively
/// EFFECT-FREE — so a call to it may appear in a pure one-hole continuation `C`. Cadenza is strict, so the
/// call's arguments are evaluated (checked separately, at their own positions); this asks only that the
/// CALLEE itself introduces no effect when `C` is spliced (once or, for a multi-shot resume, many times).
/// `false` for a non-function head (an operator — handled elsewhere — or a bare value), a recursive
/// callee, or a callee whose body might perform (`reaches_any_effect`). Sound because `reaches_any_effect`
/// over-reports: a callee this admits provably reaches no effect on any resolvable path.
fn call_is_effect_free_nonrecursive(db: &mut Db, head: StructId) -> bool {
    let Some(body) = crate::eval::lambda_body(db, head)
        .or_else(|| crate::eval::lambda_body_of_nullary(db, head))
    else {
        return false;
    };
    if crate::eval::is_recursive(db, body) {
        return false;
    }
    !reaches_any_effect(db, body, 0)
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
/// Whether every RECURSIVE self-call in `body` (a call to `callee_def`) sits in TAIL position — the
/// precondition for specializing a recursive callee under an ABORTIVE handler. When an abort fires inside
/// the recursive body it becomes the specialized function's plain RETURN value; that is only correct when
/// no self-call has PENDING computation wrapped around it (a non-tail `(+ 1 (walk …))` would let the abort
/// value flow back through the `+ 1`, a miscompile). Tail carriers: an `if`/`match` branch, a `let` body.
/// A self-call anywhere else (an operator operand, an `if` condition, a `let` init) is NON-tail.
fn recursive_self_calls_all_tail(db: &mut Db, body: StructId, callee_def: usize) -> bool {
    self_calls_tail(db, body, callee_def, true)
}

/// Whether `body` (the def `callee_def`'s body) calls a RECURSIVE def OTHER than itself — the signature of
/// MUTUAL recursion (`ev`'s body calls the recursive `od`). Used by the abortive guard: `self_calls_tail`
/// validates only THIS def's own self-calls, so a mutually-recursive callee needs this extra decline (its
/// partner may hold non-tail pending frames an abort must abandon — the non-local-exit vertical). A bounded
/// structural walk; only DIRECT calls in the body are inspected (a transitive partner is reached through
/// one of them, so the direct check suffices to flag the group).
fn callee_calls_other_recursive_def(db: &mut Db, body: StructId, callee_def: usize) -> bool {
    if let Resolved::Apply { head, .. } = resolved_of(db, body)
        && let Some(other) = callee_def_index_of(db, head)
        && other != callee_def
        && let Some(other_body) = db.defs[other].body
        && crate::eval::is_recursive(db, other_body)
    {
        return true;
    }
    match db.ast.get(body).clone() {
        Struct::List(children) => children
            .iter()
            .any(|&c| callee_calls_other_recursive_def(db, c, callee_def)),
        Struct::Atom(_) => false,
    }
}

/// Whether `node` contains a call resolving to `callee_def` (a recursive self-call), anywhere.
fn contains_self_call(db: &mut Db, node: StructId, callee_def: usize) -> bool {
    if let Resolved::Apply { head, .. } = resolved_of(db, node)
        && callee_def_index_of(db, head) == Some(callee_def)
    {
        return true;
    }
    match db.ast.get(node).clone() {
        Struct::List(children) => children
            .iter()
            .any(|&c| contains_self_call(db, c, callee_def)),
        Struct::Atom(_) => false,
    }
}

/// Whether `node` contains a call that RE-ENTERS the recursion — a self-call to `callee_def` OR a call to
/// a MUTUAL-recursive PARTNER (a DIFFERENT def whose own body is recursive, so it cycles back), anywhere.
/// Used by the out-state spine-order guard: a LATER `let` init / `do` item that is itself a recursive call
/// reads the recursion's OUT-state exactly as a perform would — the SIBLING-recursive-calls shape `(let ((a
/// (walk l))) (let ((b (walk r))) …))`, where the second `(walk r)` would thread against the INCOMING state
/// (a silent state-reset miscompile). `contains_any_perform` misses this because a nested `let` obscures
/// the perform from its reachability walk, so the guard checks this too.
fn contains_recursive_call(db: &mut Db, node: StructId, callee_def: usize) -> bool {
    if let Resolved::Apply { head, .. } = resolved_of(db, node)
        && let Some(other) = callee_def_index_of(db, head)
    {
        if other == callee_def {
            return true;
        }
        if let Some(other_body) = db.defs[other].body
            && crate::eval::is_recursive(db, other_body)
        {
            return true;
        }
    }
    match db.ast.get(node).clone() {
        Struct::List(children) => children
            .iter()
            .any(|&c| contains_recursive_call(db, c, callee_def)),
        Struct::Atom(_) => false,
    }
}

/// Whether `node` reaches ANY perform — discharged by `ctx` OR foreign — anywhere in the subtree.
/// `arg_reaches_any_perform` already detects both (a bare `effect_op_of` head is a perform regardless of
/// which handler owns it), so it is the precise detector reused for the spine-order guard.
fn contains_any_perform(db: &mut Db, node: StructId, ctx: &HandlerCtx) -> bool {
    arg_reaches_any_perform(db, node, ctx)
}

/// Whether the recursive body has a strict application in which a SELF-CALL operand PRECEDES a
/// PERFORM operand (a discharged or foreign effect) — the shape the single-return specialization CANNOT
/// thread soundly: a strict operator evaluates its operands left-to-right, so a perform AFTER the self-call
/// reads the recursion's OUT-state, which the specialized `f#ctx` does not return (it threads only the
/// INCOMING state to each perform). Threading it anyway produces a body that resumes the later perform
/// against the incoming state — the wrong state — and leaves a stray unresolvable resume that surfaces the
/// internal `f#ctx$s0` name in a confusing CDZ0101. Declining UP FRONT keeps the decline clean (a "not yet
/// compiled" todo) and PROTECTS against the wrong-state miscompile. The associative `+`/`*` cases never
/// reach here — accumulator-introduction rewrites them to tail form before the effect fold. See
/// `rcdzc-specialize-recursive-operand-nested-selfcall-state-ref-gap`: a perform BEFORE the self-call
/// (reads pre-recursion state = the incoming state) folds fine; only self-call-THEN-perform is the gap.
fn selfcall_precedes_perform_in_operands(
    db: &mut Db,
    node: StructId,
    callee_def: usize,
    ctx: &HandlerCtx,
) -> bool {
    // At a strict application, scan operands left-to-right: once an operand contains a self-call, any LATER
    // operand that reaches a perform is the offending shape. (A perform in the SAME or an EARLIER operand
    // than the self-call is fine — it reads pre-recursion state.)
    if let Resolved::Apply { args, .. } = resolved_of(db, node) {
        let mut seen_self_call = false;
        for &a in args.iter() {
            if seen_self_call && contains_any_perform(db, a, ctx) {
                return true;
            }
            if contains_self_call(db, a, callee_def) {
                seen_self_call = true;
            }
        }
    }
    // A `let` sequences its inits left-to-right and THEN its body, so it is a strict spine just like an
    // operator's operands: once an INIT contains a self-call, any LATER init or the BODY that reaches a
    // perform reads the recursion's OUT-state — the same offending shape as `(+ (walk …) (E.op))`, just
    // hidden behind a binder (`(let ((rest (walk …))) (+ rest (E.op)))`). Without this arm the operand scan
    // never sees the self-call (it is buried in the init, and the body's `(+ rest (E.op))` has only a `Ref`
    // and a perform as operands), so specialization proceeds and leaks the internal `f#ctx$s0` name in a
    // confusing CDZ0101. Positions after the self-call init are: the remaining inits, then the body.
    if let Some(form) = db.ast.as_form(node, "let").map(|t| t.to_vec())
        && form.len() == 2
        && let Struct::List(pairs) = db.ast.get(form[0]).clone()
    {
        let mut seen_self_call = false;
        for pair in pairs {
            if let Struct::List(kv) = db.ast.get(pair).clone()
                && kv.len() == 2
            {
                // A LATER init that reaches a perform OR is itself (contains) a recursive/mutual call reads
                // the recursion's out-state. Checking `contains_recursive_call` alongside the perform reach
                // is what catches the SIBLING-recursive-calls shape `(let ((a (walk l))) (let ((b (walk r)))
                // (+ a b)))`: the second init `(walk r)` is a recursive call whose specialization would
                // thread against the INCOMING state, not the state `(walk l)` advanced — a silent state-reset
                // miscompile. (`contains_any_perform` alone misses it: the nested `let` obscures the perform
                // from the reachability walk.)
                if seen_self_call
                    && (contains_any_perform(db, kv[1], ctx)
                        || contains_recursive_call(db, kv[1], callee_def))
                {
                    return true;
                }
                if contains_self_call(db, kv[1], callee_def) {
                    seen_self_call = true;
                }
            }
        }
        if seen_self_call
            && (contains_any_perform(db, form[1], ctx)
                || contains_recursive_call(db, form[1], callee_def))
        {
            return true;
        }
    }
    // A `do` sequences its items left-to-right (each runs for effect, the last is the value) — the same
    // strict spine as operands / `let` inits. Once an ITEM contains a self-call, any LATER item that
    // reaches a perform reads the recursion's OUT-state. Without this arm `(do (walk …) (E.op))` folds the
    // `E.op` against the INCOMING state (the recursive-call thread arm returns `cur` unchanged as the
    // out-state — "post-recursion state is not observed"), so the sequence-following perform MISCOMPILES to
    // the wrong value (a source-fold miscompile both backends share). Decline it here instead.
    if let Some(items) = db.ast.as_form(node, "do").map(|t| t.to_vec()) {
        let mut seen_self_call = false;
        for &it in items.iter() {
            if seen_self_call && contains_any_perform(db, it, ctx) {
                return true;
            }
            if contains_self_call(db, it, callee_def) {
                seen_self_call = true;
            }
        }
    }
    // A `match` evaluates its SCRUTINEE first, then the selected ARM BODY — a strict spine (scrutinee before
    // body). If the SCRUTINEE contains a self-call, any arm BODY that reaches a perform reads the recursion's
    // OUT-state (`(match (walk …) (_ (E.op)))` threads the arm body against the post-scrutinee `cur`, which
    // the recursive-call arm returns UNCHANGED as the incoming state). Same out-state gap as operands / `let`
    // / `do`; without this the scrutinee-self-call shape leaks the internal `f#ctx$s0` name in a CDZ0101.
    if let Resolved::Match { scrutinee, arms } = resolved_of(db, node)
        && contains_self_call(db, scrutinee, callee_def)
        && arms
            .iter()
            .any(|&(_, body)| contains_any_perform(db, body, ctx))
    {
        return true;
    }
    // An `if` evaluates its CONDITION first, then the taken BRANCH — the condition is a strict-first
    // position (like a match scrutinee). A self-call in the condition followed by a perform in EITHER branch
    // reads the recursion's OUT-state (the branch threads against the post-condition `cur`, which the
    // recursive-call arm returns unchanged). Same out-state gap; without this the cond-self-call shape leaks
    // the internal `f#ctx$s0` name in a CDZ0101. (A self-call in a BRANCH, not the condition, is a different
    // position — handled by the generic structural recurse below where it belongs.)
    if let Resolved::If { cond, then_, else_ } = resolved_of(db, node)
        && contains_self_call(db, cond, callee_def)
        && (contains_any_perform(db, then_, ctx) || contains_any_perform(db, else_, ctx))
    {
        return true;
    }
    // Recurse structurally — the shape can be nested inside a branch/let/operand.
    match db.ast.get(node).clone() {
        Struct::List(children) => children
            .iter()
            .any(|&c| selfcall_precedes_perform_in_operands(db, c, callee_def, ctx)),
        Struct::Atom(_) => false,
    }
}

/// Whether the recursive body has a SELF-CALL whose ARGUMENT contains a call to a cross-function
/// effect-performing HELPER whose own body performs a discharged op UNDER A CONDITIONAL (`if`/`match`).
/// This is the residual of the effectful-helper-in-a-self-call-arg family the deep-fresh-copy fixes do NOT
/// cover: threading the self-call arg inlines the helper's `if`, and a perform in a branch produces a state
/// reference (`f#ctx$s0`) that the branch-local `if` threading does not bind into the synthesized def →
/// CDZ0101 leaking the internal `$s0` name. A helper that performs on its UNCONDITIONAL spine (no `if`
/// gating the perform) folds via the inline path (`deep_fresh_copy`) — that is NOT flagged here. Declining
/// this shape UP FRONT turns the confusing `$s0`-leaking CDZ0101 into a clean "not yet reducible" todo.
fn selfcall_arg_inlines_conditional_perform(
    db: &mut Db,
    node: StructId,
    callee_def: usize,
    ctx: &HandlerCtx,
) -> bool {
    if let Resolved::Apply { head, args } = resolved_of(db, node)
        && callee_def_index_of(db, head) == Some(callee_def)
    {
        // A self-call: does any ARG reach a cross-fn helper that performs under a conditional?
        for &a in args.iter() {
            if arg_inlines_conditional_perform(db, a, ctx) {
                return true;
            }
        }
    }
    match db.ast.get(node).clone() {
        Struct::List(children) => children
            .iter()
            .any(|&c| selfcall_arg_inlines_conditional_perform(db, c, callee_def, ctx)),
        Struct::Atom(_) => false,
    }
}

/// Whether `node` contains a call to a NON-RECURSIVE cross-function helper whose body performs a discharged
/// op UNDER A CONDITIONAL (`if`/`match`). Used by [`selfcall_arg_inlines_conditional_perform`].
fn arg_inlines_conditional_perform(db: &mut Db, node: StructId, ctx: &HandlerCtx) -> bool {
    if let Resolved::Apply { head, .. } = resolved_of(db, node)
        && call_reaches_discharged_effect(db, head, ctx)
        && let Some(body) = crate::eval::lambda_body(db, head)
            .or_else(|| crate::eval::lambda_body_of_nullary(db, head))
        && perform_under_conditional(db, body, ctx)
    {
        return true;
    }
    match db.ast.get(node).clone() {
        Struct::List(children) => children
            .iter()
            .any(|&c| arg_inlines_conditional_perform(db, c, ctx)),
        Struct::Atom(_) => false,
    }
}

/// Whether `node` contains an `if`/`match` that GATES a discharged perform — either in a BRANCH/arm body
/// or in the `if` CONDITION / `match` SCRUTINEE (both flow into the branch-local threading that leaks the
/// internal `f#ctx$s0` state-param when the inlined form lands in a self-call arg). A perform on a fully
/// unconditional spine (no `if`/`match` anywhere over it) is NOT flagged — that folds via the inline path.
fn perform_under_conditional(db: &mut Db, node: StructId, ctx: &HandlerCtx) -> bool {
    match resolved_of(db, node) {
        Resolved::If { cond, then_, else_ } => {
            contains_any_perform(db, cond, ctx)
                || contains_any_perform(db, then_, ctx)
                || contains_any_perform(db, else_, ctx)
        }
        Resolved::Match { scrutinee, arms } => {
            contains_any_perform(db, scrutinee, ctx)
                || arms
                    .iter()
                    .any(|&(_, body)| contains_any_perform(db, body, ctx))
        }
        _ => match db.ast.get(node).clone() {
            Struct::List(children) => children
                .iter()
                .any(|&c| perform_under_conditional(db, c, ctx)),
            Struct::Atom(_) => false,
        },
    }
}

/// Recursive worker for [`recursive_self_calls_all_tail`]: verify every self-call (a call resolving to
/// `callee_def`) occurs only at a `tail` position. Returns `false` at the first off-tail self-call.
fn self_calls_tail(db: &mut Db, node: StructId, callee_def: usize, tail: bool) -> bool {
    if let Resolved::Apply { head, args } = resolved_of(db, node) {
        let is_self = callee_def_index_of(db, head) == Some(callee_def);
        if is_self {
            if !tail {
                return false; // a self-call off the tail path — the non-local-exit case
            }
            // A tail self-call's ARGUMENTS are non-tail (they evaluate before the call) — check them.
            return args
                .iter()
                .all(|&a| self_calls_tail(db, a, callee_def, false));
        }
    }
    // An `if`: the condition is non-tail; each branch inherits THIS position's tail-ness.
    if let Resolved::If { cond, then_, else_ } = resolved_of(db, node) {
        return self_calls_tail(db, cond, callee_def, false)
            && self_calls_tail(db, then_, callee_def, tail)
            && self_calls_tail(db, else_, callee_def, tail);
    }
    // A `let`: inits non-tail, body inherits tail-ness.
    if let Some(form) = db.ast.as_form(node, "let").map(|t| t.to_vec())
        && form.len() == 2
    {
        if let Struct::List(pairs) = db.ast.get(form[0]).clone() {
            for pair in pairs {
                if let Struct::List(kv) = db.ast.get(pair).clone()
                    && kv.len() == 2
                    && !self_calls_tail(db, kv[1], callee_def, false)
                {
                    return false;
                }
            }
        }
        return self_calls_tail(db, form[1], callee_def, tail);
    }
    // Generic descent: children are non-tail operands (a self-call there is off the tail path). Treating
    // every generic child as non-tail only ever DECLINES (never wrongly accepts), the safe direction.
    match db.ast.get(node).clone() {
        Struct::List(children) => children
            .iter()
            .all(|&c| self_calls_tail(db, c, callee_def, false)),
        Struct::Atom(_) => true,
    }
}

fn specialize_recursive(db: &mut Db, head: StructId, ctx: &HandlerCtx) -> Option<String> {
    let callee_def = callee_def_index_of(db, head)?;
    let orig_body = db.defs[callee_def].body?;
    if !ctx.has_state() {
        return None;
    }
    // ABORTIVE + NON-TAIL RECURSION SOUNDNESS GUARD. When an ABORTIVE op is discharged and the recursive
    // callee has a self-call OFF the tail path — `(def (walk n) (if (= n 0) (Bail 99) (+ 1 (walk (- n
    // 1)))))` — an abort at the base must ABANDON the pending `+ 1` frames on the recursion stack. But the
    // specialized `walk#ctx` returns the abort value 99 as an ORDINARY return, which then flows back up
    // through each caller's `+ 1` → 99+1+1+1 = 102, a MISCOMPILE. This needs the non-local-exit calling
    // convention (a stack of `+ 1` frames the abort unwinds — a later vertical). A TAIL self-call is fine:
    // the abort is the tail value, propagating up with no pending frame. Decline the non-tail abortive case.
    if !ctx.abortive.is_empty() && !recursive_self_calls_all_tail(db, orig_body, callee_def) {
        return None;
    }
    // ABORTIVE + MUTUAL RECURSION: decline. `recursive_self_calls_all_tail` above checks only THIS def's
    // OWN self-calls, so a MUTUALLY-recursive callee (`ev` calls `od` calls `ev`) whose partner has a
    // NON-tail call to it passes that check yet still has pending frames an abort must abandon — the same
    // miscompile (`(def (ev n) (if (= n 0) (Bail 99) (+ 1 (od …)))) (def (od n) (+ 1 (ev …)))` → 103, not
    // 99). Verifying cross-def tail-ness over the whole recursive group is the non-local-exit vertical;
    // until then, an abortive context over a MUTUALLY-recursive callee (one that calls ANOTHER recursive
    // def) declines cleanly. (A self-recursive callee is handled by the tail check above.)
    if !ctx.abortive.is_empty() && callee_calls_other_recursive_def(db, orig_body, callee_def) {
        return None;
    }
    // STATE-THREADING + MUTUAL RECURSION with the perform SPLIT from the mutual call across branches now
    // FOLDS (no longer declined). The former leak — a cycle def performing a discharged op in ONE `if`/
    // `match` branch while the mutual call is in a DIFFERENT branch (`(def (ev n) (if (= n 0) (Fresh.next)
    // (od …)))`) left the internal `f#ctx$s0` state reference dangling — was fixed at its ROOT: the `if`/
    // `match` thread arms give each branch/arm its OWN copy of the incoming state-ref nodes (a single-parent
    // arena orphaned a shared node when two siblings both embedded it). With that fix the memo knot ties and
    // the branch-distributed state threads correctly, so the earlier syntactic decline guard is obsolete and
    // removed — the shape specializes and runs (verified →43 / →3 on both backends, full suite green).
    // Each slot's state TYPE must be FULLY DETERMINED to annotate its trailing state param. An
    // UNDETERMINED component (an `Any` — most commonly an empty-list seed `(list)`, whose element type is
    // `Ty::Any` until an operation pins it) or a MISSING slot type would bake a wrong/loose annotation
    // (`(: s (List Any))`) that mistypes the threaded body. Decline cleanly rather than emit it.
    // EFFECTFUL-HELPER-WITH-A-CONDITIONAL-PERFORM IN A SELF-CALL ARG: decline cleanly. The deep-fresh-copy
    // of an inlined helper body (the `call_reaches_discharged_effect` arm) folds a helper that performs on
    // its UNCONDITIONAL spine, but a helper whose perform sits inside an `if`/`match` branch threads that
    // branch state-locally and leaks the internal `f#ctx$s0` state-param name (an unresolved reference the
    // branch-local `if` threading does not bind into the synthesized def) — a confusing CDZ0101. Until the
    // branch-state threading of an inlined conditional-perform is handled, decline UP FRONT so the shape is
    // an honest "not yet reducible" todo rather than a leaking coded error. (v-agent-harness Inc-3 residual;
    // the non-conditional effectful-helper family folds via the deep-fresh-copy fixes.)
    if selfcall_arg_inlines_conditional_perform(db, orig_body, callee_def, ctx) {
        return None;
    }
    let slot_tys: Vec<crate::ty::Ty> = ctx
        .slots
        .iter()
        .map(|s| s.state_ty.clone())
        .collect::<Option<Vec<_>>>()?;
    if slot_tys.iter().any(ty_has_any) {
        return None;
    }
    // OUT-STATE-OBSERVING SIBLING PERFORM/SELF-CALL. The SINGLE-return specialization threads each perform
    // against the INCOMING state, which is correct only when the perform is evaluated no later than the
    // self-call on a strict spine. When a SELF-CALL operand PRECEDES a later PERFORM or SIBLING SELF-CALL
    // (`(+ (relabel l) (relabel r))`, `(- (build …) (Idx.next))`), that later operand reads the recursion's
    // OUT-state — which single-return does not return. MULTI-VALUE mode (repro-1) handles exactly this: it
    // makes `f#ctx` return `(value, out-states…)` and LET-BINDS each self-call so a later operand threads
    // against its `(. t 1)` out-state. So instead of declining outright, decide the mode:
    //   * If the offending shape's self-calls all sit on the UNCONDITIONAL strict spine of their tail leaf
    //     (the shape `thread_returning_tuple` can bind), take MULTI-VALUE mode.
    //   * Otherwise (a self-call gated behind a conditional feeding a later reader, abortive, etc.) DECLINE
    //     as before — the wrong-state miscompile protection stands.
    // The associative `+`/`*` cases are rewritten to tail form by accumulator-introduction before the fold,
    // so a bare `(+ (relabel l) (relabel r))` only reaches here when accumulator-intro could not linearize
    // it (two DISTINCT recursive operands — the genuine tree walk).
    // (The per-LEAF check that a self-call sits on the unconditional strict spine is done inside
    // `thread_returning_tuple`, which cleanly declines a leaf it cannot bind — so the mode decision here only
    // needs the offending shape + a non-abortive context. A top-level `match`/`if` DISPATCH whose arm bodies
    // hold the sibling self-calls is exactly what `thread_returning_tuple` recurses into, so it must NOT be
    // treated as "self-call under a conditional" at this level.)
    let multivalue = selfcall_precedes_perform_in_operands(db, orig_body, callee_def, ctx)
        && ctx.abortive.is_empty()
        && multivalue_leaves_threadable(db, orig_body, callee_def);
    if selfcall_precedes_perform_in_operands(db, orig_body, callee_def, ctx) && !multivalue {
        return None; // an out-state-observing shape the multi-value path does not cover yet (abortive, or
        // a self-call gated behind a conditional inside a leaf) — decline BEFORE reserving the def.
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
        // A param is either a BARE name `n` or an ANNOTATED binder `(: n T)`. Extract the NAME from either:
        // a bare name atom directly, or the FIRST operand of a `(: name T)` form. The synthesized copy is
        // always re-annotated with the SOLVED type below, so the original annotation itself is not reused
        // (a `(: n T)` param and a bare `n` param produce the identical `(: n <solved>)` in the spec sig).
        let name = match db.ast.as_name(p) {
            Some(n) => n.to_string(),
            None => match db.ast.as_form(p, ":").and_then(|t| t.first().copied()) {
                Some(name_occ) => match db.ast.as_name(name_occ) {
                    Some(n) => n.to_string(),
                    None => return None, // a non-name binder in the annotation — unsupported
                },
                None => return None, // neither a bare name nor a `(: name T)` binder
            },
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
    // CAPTURED enclosing-fn params: a handler arm may reference a name bound by an ENCLOSING function (not
    // the arm's own params/state, not the recursive def's params) — e.g. `converse(q,s) => resume(tool,0)`
    // where `tool` is `run-with`'s param. That free name, spliced into the synthesized `f#ctx` body by the
    // fold, would re-resolve against `f#ctx`'s sig (which lacks it) → a spurious CDZ0101. Thread each such
    // capture as an EXTRA param (after the originals, before the trailing states) and pass it UNCHANGED at
    // every call (it is constant across the recursion). `own_binders` = the binders bound WITHIN the
    // specialization: the recursive def's own params + each arm's params and state binder. A reference to
    // any of these is NOT a capture. (v-agent-harness merged-nested multi-arm dogfood.)
    let mut own_binders: std::collections::HashSet<StructId> =
        orig_params.iter().copied().collect();
    for arm in ctx.arms.values() {
        own_binders.extend(arm.params.iter().copied());
        own_binders.insert(arm.state);
    }
    let captured_specs = captured_enclosing_params(db, ctx, &own_binders);
    // A capture with an undetermined type cannot annotate its extra param — decline the whole specialization
    // (mirrors the `orig_params` `Ty::Any` guard), so the shape stays a clean todo rather than emitting a
    // loosely-typed param.
    if captured_specs.iter().any(|(_, ty)| ty_has_any(ty)) {
        return None;
    }
    let capture_names: Vec<String> = captured_specs.iter().map(|(n, _)| n.clone()).collect();

    let spec_name_atom = db.push_atom(Leaf::Name(spec_name.clone()));
    let mut sig_children = vec![spec_name_atom];
    for (n, ty) in &orig_param_specs {
        let name_atom = db.push_atom(Leaf::Name(n.clone()));
        let ty_expr = crate::eval::encode_typeval(db, ty);
        let colon = db.push_atom(Leaf::Name(":".to_string()));
        sig_children.push(db.push_list(vec![colon, name_atom, ty_expr]));
    }
    // The captured enclosing-fn params — annotated with each capture's solved type, AFTER the originals and
    // BEFORE the trailing states (the layout every call site appends args in: orig, captured, states).
    for (n, ty) in &captured_specs {
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
        internal: false,
    });
    db.effect_specializations.insert(memo_key, spec_index);
    // Register the multi-value calling convention BEFORE threading the body, so the recursive self-call arm
    // (which re-enters here, hits the memo, and reads `db.multivalue_specs`) knows THIS spec returns a
    // `(value, out-states…)` tuple and rewrites its own self-calls to destructure + thread the out-state.
    if multivalue {
        db.multivalue_specs.insert(spec_name.clone());
    }
    // Register the captured enclosing-fn param names so the self-call rewrite arm (and the initial call from
    // the handle body) passes them through as extra args, in the same order the sig lays them out.
    if !capture_names.is_empty() {
        db.effect_spec_captures
            .insert(spec_name.clone(), capture_names.clone());
    }

    // Thread `orig_body` under `ctx`, with each slot's incoming state = a REFERENCE to its state param. A
    // perform's resume value references the arm's state binder, which `thread`'s perform arm substitutes
    // with that slot's state expression; the recursive self-call re-enters and (via the memo) rewrites to
    // `(spec_name args… <threaded-states>)`. Each state name atom must re-resolve to its param, so we pass
    // FRESH occurrences of the names (bare `s{k}` references), not the binder occurrences.
    let state_refs: Vec<StructId> = state_names
        .iter()
        .map(|n| db.push_atom(Leaf::Name(n.clone())))
        .collect();
    // MULTI-VALUE mode: the body's every tail leaf yields `("tuple" value out-states…)`, and each self-call
    // is let-bound (out-state projected + threaded). SINGLE-return mode: the ordinary `thread` (unchanged).
    let spec_body = if multivalue {
        ctx.temp_ctr.set(0);
        ctx.pending.borrow_mut().clear();
        thread_returning_tuple(db, orig_body, state_refs, ctx, callee_def)?
    } else {
        let (b, _out) = thread(db, orig_body, state_refs, ctx)?;
        b
    };

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

/// Build a tuple projection `(. <name> index)` — a fresh bare-name reference to `name` projected at
/// `index`. Used by the multi-value self-call rewrite to read a let-bound self-call temp's value (`.0`)
/// and each slot's out-state (`.{slot+1}`).
fn tuple_proj(db: &mut Db, name: &str, index: u32) -> StructId {
    let dot = db.push_atom(Leaf::Name(".".to_string()));
    let name_atom = db.push_atom(Leaf::Name(name.to_string()));
    let idx_atom = db.push_atom(Leaf::Int {
        value: IntValue::from_i64(index as i64),
        radix: Radix::Dec,
    });
    db.push_list(vec![dot, name_atom, idx_atom])
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

/// Extract `(value, next_state)` from a tail-resumptive handler arm body, handling the CONTROL-shaped bodies
/// the thread path serves — the perform arm reduces the arm body to its resume VALUE (the perform's result)
/// + its NEXT-STATE (threaded forward). Shapes:
///   * a bare `(resume v s)` → `(v, s)`.
///   * a `(do stmt… (resume v s))` (interposing/forwarding) → `((do stmt… v), s)` — the statements run for
///     effect, then `v` is the value.
///   * a `(match scrut (pat (resume v s))…)` (the arm DESTRUCTURES its op arg, resuming per branch) → the
///     VALUE is the match rebuilt around each branch's resume value `(match scrut (pat v)…)`, and the
///     NEXT-STATE is the branches' shared next-state. Every branch must itself peel to a resume; the
///     branches must AGREE on the next-state (structurally identical) — a branch-VARYING next-state is a
///     match-valued state the tail fold cannot thread as a single slot value, so DECLINE (`None`). The common
///     case — destructure the arg (`(k, v)`) but thread ONE state (all branches `resume(…, Map.insert(s,k,v))`
///     or `resume(…, s)`) — agrees, and folds. (v-compiler-ml's get/put memoized-DB shape: a `put` arm
///     `(match kv (| (k,v) => resume(unit, Map.insert(s,k,v))))` performed in a `;`-sequence with a `get`.)
///
/// `None` if the arm body is not one of these (the honest "not yet reducible" decline).
fn peel_resume_from_arm_body(db: &mut Db, arm_body: StructId) -> Option<(StructId, StructId)> {
    // Bare `(resume v s)`.
    if let Some(vs) = tail_resume(db, arm_body) {
        return Some(vs);
    }
    // `(do stmt… (resume v s))` — peel the trailing resume, keep the leading statements around the value.
    if let Some(items) = db.ast.as_form(arm_body, "do").map(|t| t.to_vec()) {
        let (&last, stmts) = items.split_last()?;
        let (v, s) = peel_resume_from_arm_body(db, last)?;
        let do_head = db.push_name("do");
        let mut children = vec![do_head];
        children.extend_from_slice(stmts);
        children.push(v);
        return Some((db.push_list(children), s));
    }
    // `(match scrut (pat body)…)` — the arm DESTRUCTURES its op arg and resumes per branch. Peel each
    // branch's resume, then rebuild TWO matches over the SAME scrutinee: the VALUE match `(match scrut (pat
    // branch-value)…)` (the perform's result) and the NEXT-STATE match `(match scrut (pat branch-next-state)
    // …)` (threaded forward). Keeping BOTH match-wrapped is load-bearing: a branch's next-state may reference
    // the branch's PATTERN BINDERS (the DB `put` arm's `Map.insert(s, k, v)` uses `k`,`v` bound by `(k, v)`),
    // so the next-state CANNOT be hoisted out of the match (the binders would go unbound) — it must stay
    // inside its branch. The state then threads forward as a match-VALUED expression (sound because the
    // scrutinee is the pure op arg, so evaluating it in both matches duplicates no effect; the match-distrib
    // path likewise requires a pure scrutinee). A subsequent perform reading the state sees a well-formed
    // `(match arg (pat …))` that evaluates to the new state. Each branch must itself peel to a resume.
    if let Resolved::Match { scrutinee, arms } = resolved_of(db, arm_body) {
        if arms.is_empty() {
            return None;
        }
        let vhead = db.push_name("match");
        let shead = db.push_name("match");
        let mut value_children = vec![vhead, scrutinee];
        let mut state_children = vec![shead, scrutinee];
        for (pat, body) in arms {
            let (v, s) = peel_resume_from_arm_body(db, body)?;
            value_children.push(db.push_list(vec![pat, v]));
            state_children.push(db.push_list(vec![pat, s]));
        }
        return Some((db.push_list(value_children), db.push_list(state_children)));
    }
    None
}

/// Whether the arm body `node` is TAIL-RESUMPTIVE in the sense the `thread` path serves — either a bare
/// `(resume v s)` (its resume IS the tail) or a `(do stmt… (resume v s))` INTERPOSING/FORWARDING arm whose
/// LAST statement is the resume (the leading statements run for effect, then it resumes). Such an arm is
/// handled by threading, NOT by the E5 non-tail continuation folds — the two-hole fold must EXCLUDE it so
/// it does not steal (and then decline) an interpose arm the thread path folds correctly.
fn is_tail_resumptive_arm(db: &mut Db, node: StructId) -> bool {
    if tail_resume(db, node).is_some() {
        return true;
    }
    if let Some(items) = db.ast.as_form(node, "do").map(|t| t.to_vec())
        && let Some((&last, _)) = items.split_last()
    {
        return tail_resume(db, last).is_some();
    }
    false
}

/// Whether the application `head` is a PURE primitive operator — one whose `(meta apply)` channel names a
/// `Prim` (arithmetic, comparison, a constructor, …). Such an operator is STRICT and effect-free: its
/// operands evaluate exactly once, unconditionally, with no side effect. This is exactly the head-set the
/// pure one-hole-context walk descends into. A USER function head is not a record with a `(meta apply)`
/// prim (it resolves to a lambda), so it yields `None` and is treated as opaque/possibly-effectful. A
/// PERFORM head's `(meta apply)` is `(intrinsic perform)`, which is NOT a `Prim` (`Prim::from_name`
/// doesn't know "perform"), so it too yields `None` — so this predicate excludes EVERY perform, discharged
/// or not, which is what strong purity needs (a non-discharged perform is still an effect to guard against).
fn is_pure_operator_head(db: &mut Db, head: StructId) -> bool {
    crate::eval::meta_apply_of(db, head).is_some()
}

/// The result of classifying a handle body as a PURE one-hole continuation context (`pure_hole`).
enum PureHole {
    /// The body reaches EXACTLY ONE discharged perform `P` through STRICT, UNCONDITIONAL, effect-free
    /// positions, and everything ELSE in the body is strongly pure — so `C = body[P := □]` is a pure
    /// one-hole context. Carries the perform occurrence `P` (the hole).
    Hole(StructId),
    /// No discharged perform on a pure spine — the body is fully pure (nothing for THIS handler to
    /// discharge). Not a fold this block handles.
    Pure,
    /// Not a pure one-hole context: a SECOND discharged perform, a perform under a conditional
    /// (`if`/`match`/`and`/`or`), a `resume`, a nested `handle`, or any other effect — needs the full
    /// captured-continuation machinery, so this block declines and threading is attempted / declines.
    Impure,
}

/// Classify the handle `body` as a PURE one-hole continuation context: does it reach EXACTLY ONE
/// discharged perform `P` through strict, unconditional, effect-free positions, with everything else
/// strongly pure? If so, `C = body[P := □]` is pure and `(resume v s)` folds to `C[v]` (see the E5
/// pure-continuation block in `reduce_handle`). The walk is CONSERVATIVE — it admits only the shapes it
/// can prove pure-and-uniform and returns `Impure` for anything else, so a mis-fold (duplicating or
/// reordering an effect) is impossible; an over-decline just leaves the case to the (later) frame vertical.
///
/// STRICT UNCONDITIONAL positions (the continuation is uniform — the perform's result flows into exactly
/// one deterministic downstream computation): an operator/primitive-call OPERAND, a strict one-operand
/// form's operand (`not`/projection/member/annotation), a tuple/list element. NOT admitted (a non-uniform
/// or effect-shielding continuation): `if`/`match`/`and`/`or` (a conditional — the continuation differs by
/// branch / the perform may not run), a `let` (the binding could be duplicated by a multi-shot resume in a
/// way this simple splice does not model), a nested `handle`, a user/recursive call (its body may perform).
fn pure_hole(db: &mut Db, node: StructId, ctx: &HandlerCtx) -> PureHole {
    // A discharged perform IS the hole. Its ARGS must be strongly pure (a nested perform in an arg would be
    // a second effect whose own continuation is non-trivial) — checked by the caller-side spine below via
    // `strongly_pure`. Here, at the perform node, verify the args are strongly pure and this is the hole.
    if let Resolved::Apply { head, args } = resolved_of(db, node)
        && is_perform(db, head, ctx).is_some()
    {
        if args.iter().all(|&a| strongly_pure(db, a, ctx)) {
            return PureHole::Hole(node);
        }
        return PureHole::Impure; // a perform whose arg is itself effectful — not a simple one-hole context
    }
    // A `resume`, a nested `handle`, an `if`/`match`/`and`/`or` with a perform inside, a `let`, or a
    // user/recursive call are all NON-uniform or effect-shielding — decline (Impure) if they carry an
    // effect, else they are pure and fall through to the strongly-pure check below.
    match resolved_of(db, node) {
        // An `if`: the CONDITION is a STRICT, always-evaluated-first position, so a hole there has a
        // UNIFORM continuation `C = (if <cond[□]> then else)` — foldable (`(if (< (Amb.flip) 5) 1 2)` →
        // resume 10 gives `(if (< 10 5) 1 2)`). The BRANCHES run CONDITIONALLY after the condition; a
        // perform in a branch is a NON-uniform continuation (it may not run, or its continuation differs by
        // branch) — NOT this fold (the resumptive-conditional hoist lifts a branch-performing conditional
        // in a strict position elsewhere; a branch perform the hoist can't lift declines). So: admit a hole
        // in `cond` only when BOTH branches are strongly pure (they are copied verbatim into `C`, and a
        // multi-shot resume duplicates them — safe iff effect-free); a branch that is not strongly pure →
        // Impure. When `cond` has no hole and both branches are pure, the whole `if` is Pure.
        Resolved::If { cond, then_, else_ } => {
            if !strongly_pure(db, then_, ctx) || !strongly_pure(db, else_, ctx) {
                return PureHole::Impure;
            }
            pure_hole(db, cond, ctx)
        }
        // A `match`: the SCRUTINEE is a STRICT, always-evaluated-first position (like an `if` condition), so
        // a hole there has a UNIFORM continuation `C = (match <scrut[□]> (pat body)…)` — the arms run only
        // AFTER the scrutinee. Admit a hole in `scrutinee` only when EVERY arm BODY is strongly pure (they
        // are copied verbatim into `C`, and a multi-shot resume duplicates them — safe iff effect-free); an
        // arm body that performs is a non-uniform continuation → Impure. A pattern is a binder position (not
        // a value), so it holds no discharged perform. When the scrutinee has no hole and every arm body is
        // pure, the whole `match` is Pure.
        Resolved::Match { scrutinee, arms } => {
            if !arms.iter().all(|&(_, body)| strongly_pure(db, body, ctx)) {
                return PureHole::Impure;
            }
            pure_hole(db, scrutinee, ctx)
        }
        // A short-circuit connective `(and lhs rhs)`/`(or lhs rhs)`: the LHS is a STRICT, always-evaluated-
        // first position, so a hole there has a UNIFORM continuation `C = (and <lhs[□]> rhs)`. The RHS runs
        // only CONDITIONALLY on `lhs`, so admit a hole in `lhs` only when `rhs` is strongly pure (it is
        // copied verbatim into `C` and runs on the taken path, possibly duplicated by a multi-shot resume —
        // safe iff effect-free); an `rhs` that performs is a non-uniform continuation → Impure. A hole in
        // the RHS (a conditionally-run position) is NOT this fold — declines. When `lhs` has no hole and
        // `rhs` is pure, the whole connective is Pure.
        Resolved::And { lhs, rhs, .. } => {
            if !strongly_pure(db, rhs, ctx) {
                return PureHole::Impure;
            }
            pure_hole(db, lhs, ctx)
        }
        // A `let ((n0 i0) …) body`: the inits and the body all run UNCONDITIONALLY, in sequence (an init,
        // then later inits/the body see it) — so every INIT and the BODY is a strict-spine position, and a
        // hole in exactly one of them has a uniform continuation `C = (let ((n0 i0[□?]) …) body[□?])`. The
        // binder NAMES are label positions (never a value hole). `splice_context` copies the WHOLE `let` per
        // resume, so each copy gets a fresh independent binder (the same re-parenting that makes a match
        // binder-arm re-resolve) — a multi-shot resume is safe. Find ≤1 hole across the init VALUES then the
        // body; two holes or a hole beside an impure sibling → Impure (via `pure_hole_seq`).
        Resolved::Let {
            bindings,
            body: let_body,
        } => {
            let mut positions: Vec<StructId> = bindings.iter().map(|&(_n, i)| i).collect();
            positions.push(let_body);
            pure_hole_seq(db, positions.into_iter(), ctx)
        }
        // A nested handle / resume: if a discharged perform is anywhere inside, the continuation is
        // non-uniform (a nested control effect) → Impure; else the whole form is pure and admissible as
        // opaque context (the strongly-pure fall-through).
        Resolved::Handle { .. } | Resolved::Resume { .. } => {
            if strongly_pure(db, node, ctx) {
                PureHole::Pure
            } else {
                PureHole::Impure
            }
        }
        // A STRICT form: descend into its operands in order, finding at most one hole. `not`/projection/
        // member/annotation (one operand), a tuple/list (positional elements), and an ORDINARY application
        // (a primitive operator over operands — NOT a user/recursive call, which `strongly_pure` rejects)
        // are strict — every operand is evaluated exactly once, unconditionally, before the form. So a hole
        // in one operand has a uniform continuation.
        Resolved::Not { operand } => pure_hole(db, operand, ctx),
        Resolved::Proj { operand, .. } | Resolved::Member { operand, .. } => {
            pure_hole(db, operand, ctx)
        }
        Resolved::Annot { expr, .. } => pure_hole(db, expr, ctx),
        Resolved::Tuple { elems } | Resolved::List { elems } => {
            pure_hole_seq(db, elems.iter().copied(), ctx)
        }
        Resolved::Apply { head, args } => {
            // A STRICT, effect-free head — a PRIMITIVE operator (arith/cmp/ctor) OR a NON-RECURSIVE user
            // function whose body reaches no effect (`call_is_effect_free_nonrecursive`) — evaluates each
            // argument exactly once, unconditionally, before the call, and adds no effect of its own. So a
            // hole in one argument has a UNIFORM continuation `C = (f a0 … □ … an)` (splicing `C` re-runs
            // the pure call), foldable like a primitive operator. A perform head was handled above.
            if is_pure_operator_head(db, head) || call_is_effect_free_nonrecursive(db, head) {
                // The head is pure; find the single hole across the args, left to right.
                return pure_hole_seq(db, args.iter().copied(), ctx);
            }
            // A possibly-effectful call (recursive / unresolvable / reaches an effect): if it reaches an
            // effect it is Impure, else opaque-pure.
            if strongly_pure(db, node, ctx) {
                PureHole::Pure
            } else {
                PureHole::Impure
            }
        }
        // Any other shape (a literal, a bare ref, a param, a record, a type value, …) has no discharged
        // perform reachable on a pure spine here — classify by strong purity.
        _ => {
            if strongly_pure(db, node, ctx) {
                PureHole::Pure
            } else {
                PureHole::Impure
            }
        }
    }
}

/// Find the LEADING discharged perform on the strict evaluation spine of `node` — the FIRST one reached
/// left-to-right — where every position evaluated STRICTLY BEFORE it is strongly pure. UNLIKE `pure_hole`,
/// the continuation AFTER the hole MAY itself perform (a second hole): this is the two-hole (general
/// one-shot) case, folded frame-free by RE-REDUCING the spliced continuation `C[v]` under the same handler
/// (each refold removes one perform, so it terminates). Returns the leading perform occurrence, or `None`
/// if the leading effect on the spine is not at a clean strict-first, UNIFORM position (a conditional
/// BRANCH / connective RHS — the frame vertical's job). Descends the same STRICT-FIRST positions
/// `pure_hole` does: operator/call operands, tuple/list elements, `not`/proj/member/annotation, a `let`
/// (its inits then body, in order), and a `match` SCRUTINEE (evaluated first). SOUND ONLY for a ONE-SHOT
/// arm (the caller checks `count_resumes == 1`): a multi-shot arm would splice a performing `C` more than
/// once, duplicating the inner effect.
fn leading_strict_hole(db: &mut Db, node: StructId, ctx: &HandlerCtx) -> Option<StructId> {
    // A discharged perform IS the leading hole — provided its own ARGS are strongly pure (an effectful arg
    // would be an even-earlier hole this simple spine walk does not thread).
    if let Resolved::Apply { head, args } = resolved_of(db, node)
        && is_perform(db, head, ctx).is_some()
    {
        return if args.iter().all(|&a| strongly_pure(db, a, ctx)) {
            Some(node)
        } else {
            None
        };
    }
    match resolved_of(db, node) {
        Resolved::Not { operand } => leading_strict_hole(db, operand, ctx),
        Resolved::Proj { operand, .. } | Resolved::Member { operand, .. } => {
            leading_strict_hole(db, operand, ctx)
        }
        Resolved::Annot { expr, .. } => leading_strict_hole(db, expr, ctx),
        Resolved::Tuple { elems } | Resolved::List { elems } => {
            leading_strict_hole_seq(db, elems.iter().copied(), ctx)
        }
        Resolved::Apply { head, args } => {
            if is_pure_operator_head(db, head) || call_is_effect_free_nonrecursive(db, head) {
                return leading_strict_hole_seq(db, args.iter().copied(), ctx);
            }
            None
        }
        // A `let`: inits then body all run UNCONDITIONALLY in sequence — a strict spine. The leading hole
        // is the first performing position across (inits ++ body); the WHOLE `let` is the continuation `C`
        // (the refold copies it, so each binder re-binds independently). Mirrors `pure_hole`'s `let` arm.
        Resolved::Let { bindings, body } => {
            let mut positions: Vec<StructId> = bindings.iter().map(|&(_n, i)| i).collect();
            positions.push(body);
            leading_strict_hole_seq(db, positions.into_iter(), ctx)
        }
        // A `match`: the SCRUTINEE is a strict, always-evaluated-first position (the arms run only after);
        // the leading hole may be in the scrutinee. The continuation `C = (match □ arms…)` is re-reduced by
        // the refold, so the arms need not be pure here (unlike `pure_hole`, which needs a pure `C`). A
        // perform in an ARM BODY is a non-uniform (conditionally-run) position — NOT a leading strict hole
        // (handler distribution handles a pure-scrutinee arm perform; a scrutinee hole is this path).
        Resolved::Match { scrutinee, .. } => leading_strict_hole(db, scrutinee, ctx),
        // An `if`: the CONDITION is strict, evaluated FIRST, so a leading hole may sit there. The
        // continuation `C = (if <cond[□]> then else)` is re-reduced by the refold — where the condition is
        // now a concrete VALUE, so the taken branch is selected (a performing branch is then served by the
        // recursive fold / distribution). Unlike `pure_hole`'s if-cond arm, the branches need NOT be pure
        // (the refold re-reduces `C`). A leading hole in a BRANCH (a conditionally-run position) is NOT
        // returned — only the condition is a strict-first uniform position.
        Resolved::If { cond, .. } => leading_strict_hole(db, cond, ctx),
        // A short-circuit `and`/`or`: the LHS is strict, evaluated FIRST — a leading hole may sit there. `C
        // = (and <lhs[□]> rhs)` is re-reduced by the refold (lhs now a value → short-circuit resolves; a
        // performing rhs on the taken path is served by the fold). The RHS (conditionally-run) is not a
        // strict-first position, so only the LHS is descended.
        Resolved::And { lhs, .. } => leading_strict_hole(db, lhs, ctx),
        // Any other shape (`handle`/`resume`) or an already-pure leaf: not a clean strict leading hole
        // here — decline.
        _ => None,
    }
}

/// Find the LEADING hole across a sequence of strict operands (evaluated left-to-right, each exactly once
/// before the enclosing form): return the first operand that CONTAINS a leading strict hole, requiring
/// every EARLIER operand to be strongly pure (evaluated first, so an earlier perform would be the real
/// leading hole). An earlier operand that performs (but is not a clean strict hole) → `None` (decline).
fn leading_strict_hole_seq(
    db: &mut Db,
    items: impl Iterator<Item = StructId>,
    ctx: &HandlerCtx,
) -> Option<StructId> {
    for it in items {
        if strongly_pure(db, it, ctx) {
            continue; // no effect here — the hole is later
        }
        // The first operand that is NOT strongly pure must itself begin with a strict leading hole; if it
        // does not (it performs through a non-uniform position), we cannot fold — decline.
        return leading_strict_hole(db, it, ctx);
    }
    None
}

/// Find at most ONE hole across a sequence of strict operands (evaluated left-to-right, each exactly
/// once). Each operand is either strongly pure (no hole) or the single hole; two holes, or a hole
/// alongside an impure operand, is `Impure`.
fn pure_hole_seq(db: &mut Db, items: impl Iterator<Item = StructId>, ctx: &HandlerCtx) -> PureHole {
    let mut hole: Option<StructId> = None;
    for it in items {
        match pure_hole(db, it, ctx) {
            PureHole::Pure => {}
            PureHole::Hole(p) => {
                if hole.is_some() {
                    return PureHole::Impure; // a SECOND discharged perform — not a one-hole context
                }
                hole = Some(p);
            }
            PureHole::Impure => return PureHole::Impure,
        }
    }
    match hole {
        Some(p) => PureHole::Hole(p),
        None => PureHole::Pure,
    }
}

/// Whether the subtree at `node` is STRONGLY pure — free of ANY effect (not just this handler's discharged
/// ops): no discharged perform (`subtree_performs`) AND no `resume`, nested `handle`, or NON-primitive
/// call anywhere (a user/recursive call's body could perform an effect this handler does NOT discharge,
/// and a multi-shot resume would duplicate it). This is the guard the pure one-hole context needs: `C` is
/// spliced possibly-many times, so it must contain no observable effect of any kind. Conservative — a
/// call to a provably-pure function is rejected too (safe: an over-decline, never a mis-fold).
fn strongly_pure(db: &mut Db, node: StructId, ctx: &HandlerCtx) -> bool {
    // A discharged perform, a `resume`, or a call reaching a discharged effect — not pure. (Reuses the
    // discharged-op detector, which also follows calls into discharged effects.)
    if subtree_performs(db, node, ctx) {
        return false;
    }
    // A nested `handle` or a `resume` — control-flow effects. (`subtree_performs` catches a bare `resume`,
    // but be explicit so intent is clear and a future `subtree_performs` change cannot silently admit one.)
    if matches!(
        resolved_of(db, node),
        Resolved::Handle { .. } | Resolved::Resume { .. }
    ) {
        return false;
    }
    // An APPLICATION whose head is neither a pure primitive operator NOR an effect-free non-recursive user
    // function is a possibly-effectful call — reject it (a multi-shot splice would duplicate any hidden
    // effect). A primitive operator (arith/comparison/ctor/…) is pure; a user call whose callee is
    // non-recursive and transitively reaches NO effect (`call_is_effect_free_nonrecursive`) is also pure —
    // splicing the CALL (once or many times) re-runs an effect-free computation, observationally identical
    // to running it once. In BOTH cases the head is safe and the operands (checked by the structural
    // descent below) must themselves be strongly pure.
    if let Resolved::Apply { head, .. } = resolved_of(db, node)
        && !is_pure_operator_head(db, head)
        && !call_is_effect_free_nonrecursive(db, head)
    {
        return false;
    }
    // A LAMBDA VALUE is strongly pure — CONSTRUCTING a closure performs nothing; its body's effects fire
    // only when it is APPLIED (a separate `Apply` node, checked at ITS site). Do NOT descend into the
    // lambda body (the same reasoning as `subtree_performs`): a `let`-bound performing lambda left behind
    // after its application was pre-reduced (`reduce_applied_lambdas`) is a pure binding, so the pure
    // one-hole context around it stays strongly pure. Splicing a closure VALUE many times (a multi-shot
    // resume) duplicates only the closure, not any effect — the effect is at the application, which lives
    // in the context and is classified there.
    if matches!(resolved_of(db, node), Resolved::Lambda { .. }) {
        return true;
    }
    // Descend structurally — every child must be strongly pure. For an admitted application the head is a
    // name atom (pure) and this checks each argument; the effect-free callee's own body is NOT spliced (the
    // call stays a call), so it is validated by `call_is_effect_free_nonrecursive`, not descended here.
    match db.ast.get(node).clone() {
        Struct::List(children) => children.iter().all(|&c| strongly_pure(db, c, ctx)),
        Struct::Atom(_) => true,
    }
}

/// Rewrite every `(resume value next-state)` in `node` to `C[value]` — the E5 pure one-hole-continuation
/// reduction. `C = handle_body[perform := □]` is the pure delimited continuation the resume returns into,
/// realized as a copy of `handle_body` with the sole discharged `perform` node replaced by the resume's
/// VALUE. When `C` is the identity (`handle_body` IS the bare `perform`, so `C = □`), this is `value` in
/// place — the identity slice. `C` is STRONGLY pure (`pure_hole` admits only effect-free one-hole
/// contexts), so a MULTI-shot arm may splice a fresh copy of it per resume with no effect duplication.
/// Non-resume nodes are copied structurally so the result is self-contained. Each spliced copy detaches
/// from the dead `resume` node's scope (fresh parentage), and `C`'s free names re-parent under the splice.
fn rewrite_resume_to_context(
    db: &mut Db,
    node: StructId,
    handle_body: StructId,
    perform: StructId,
) -> StructId {
    if let Resolved::Resume { value, .. } = resolved_of(db, node) {
        // Splice a fresh copy of `C` with the hole filled by (a fresh copy of) the resume value: copy the
        // handle body, replacing the sole `perform` occurrence with the resume value.
        return splice_context(db, handle_body, perform, value);
    }
    match db.ast.get(node).clone() {
        Struct::List(children) => {
            let rewritten: Vec<StructId> = children
                .iter()
                .map(|&c| rewrite_resume_to_context(db, c, handle_body, perform))
                .collect();
            db.push_list(rewritten)
        }
        Struct::Atom(_) => copy_pure(db, node),
    }
}

/// Rewrite every `(resume value next-state)` in `node` to the RE-REDUCTION of the continuation `C[value]`
/// under the SAME handler, re-seeded with the resume's `next-state` — the E5 two-hole (general one-shot)
/// reduction. In a DEEP handler, `resume v s'` returns into `C[v]` with the handler still active for the
/// rest of the computation, so when `C[v]` itself performs a discharged op (a second hole), that inner
/// perform must ALSO be handled: `resume v s' = reduce_handle(s', arms, C[v])`. Each refold removes one
/// perform, so the recursion terminates (bounded further by `reduce_handle`'s re-entry guard). `C =
/// handle_body[leading_perform := □]`. Returns `None` if any recursive refold declines (the whole two-hole
/// fold then declines cleanly — never a partial rewrite). SOUND ONLY for a ONE-SHOT arm (the caller checks
/// `count_resumes == 1`): a single `resume` occurrence means `C` is spliced once, so the inner perform in
/// `C` runs exactly once — no duplication.
fn rewrite_resume_to_refolded_context(
    db: &mut Db,
    node: StructId,
    handle_body: StructId,
    perform: StructId,
    arms: &[HandleArm],
) -> Option<StructId> {
    if let Resolved::Resume { value, next_state } = resolved_of(db, node) {
        // Build `C[value]` (the continuation with the hole filled by the resume value), then re-reduce it
        // under the same handler seeded with the resume's next-state — so a further discharged perform in
        // `C` is handled by the recursive fold.
        let filled = splice_context(db, handle_body, perform, value);
        return reduce_handle(db, next_state, arms, filled);
    }
    match db.ast.get(node).clone() {
        Struct::List(children) => {
            let mut rewritten = Vec::with_capacity(children.len());
            for &c in &children {
                rewritten.push(rewrite_resume_to_refolded_context(
                    db,
                    c,
                    handle_body,
                    perform,
                    arms,
                )?);
            }
            Some(db.push_list(rewritten))
        }
        Struct::Atom(_) => Some(copy_pure(db, node)),
    }
}

/// Copy the one-hole context `handle_body` (the pure delimited continuation), replacing the sole hole
/// occurrence `perform` with (a fresh copy of) `filler` — i.e. build `C[filler]`. The hole `perform` is a
/// UNIQUE occurrence in the arena (`pure_hole` verified exactly one discharged perform reaches on a pure
/// spine), so a by-identity match locates it. Everything else is copied structurally so the result is
/// self-contained and re-parents its free names against the splice site.
fn splice_context(db: &mut Db, node: StructId, perform: StructId, filler: StructId) -> StructId {
    if node == perform {
        return copy_pure(db, filler);
    }
    match db.ast.get(node).clone() {
        Struct::List(children) => {
            let spliced: Vec<StructId> = children
                .iter()
                .map(|&c| splice_context(db, c, perform, filler))
                .collect();
            db.push_list(spliced)
        }
        Struct::Atom(_) => copy_pure(db, node),
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

/// The number of `resume` occurrences in the arm body at `node` (structural walk). ONE = a one-shot arm
/// (the resume value flows into the continuation exactly once, so splicing the continuation duplicates
/// NOTHING); >1 = multi-shot. The nested-continuation refold (a continuation that itself performs — the
/// two-hole case) is admitted ONLY for a one-shot arm: a multi-shot arm would splice a continuation that
/// PERFORMS more than once, duplicating (and re-ordering) that inner effect, which the frame-free refold
/// cannot represent (it needs the defunctionalized-frame vertical).
fn count_resumes(db: &mut Db, node: StructId) -> u32 {
    let here = u32::from(matches!(resolved_of(db, node), Resolved::Resume { .. }));
    let below = match db.ast.get(node).clone() {
        Struct::List(children) => children.iter().map(|&c| count_resumes(db, c)).sum(),
        Struct::Atom(_) => 0,
    };
    here + below
}

/// The number of references to the parameter `binder` in the arm body at `node` — how many times a
/// substituted argument would be COPIED when the arm body β-reduces. A reference resolves to
/// `Resolved::Param { binder }` (or a `Ref` transitively to it). Used to guard the perform-threading arm
/// against duplicating a PERFORMING argument: substituting an arg that reaches an effect into a param used
/// more than once would run that effect once per use (a miscompile — `(E.op (tuple (A.get) (A.get)))` whose
/// arm reads `(. p 0)` AND `(. p 1)` duplicated the two inner gets, threading four reads instead of two).
/// Collect the CAPTURED enclosing-fn param NAMES that a handler context's arm bodies reference free — a
/// name that resolves to a `Param` binder which is NOT one of the arm's own params/state (those are bound
/// by the arm) and is NOT a param of the recursive def being specialized (those are the `orig_params`,
/// already threaded). Such a name is captured from an enclosing function (e.g. `tool` from `run-with` in a
/// `converse(q,s) => resume(tool,0)` arm) and must be threaded as an extra specialized param, else it
/// re-resolves against the synthesized `f#ctx` sig (which lacks it) → a spurious CDZ0101. Returns each
/// capture's (name, solved-type) in first-seen order, deduped by name. `own_binders` is the set of binders
/// bound WITHIN the specialization (the arm params/state + the recursive def's params) — a reference to any
/// of these is NOT a capture.
fn captured_enclosing_params(
    db: &mut Db,
    ctx: &HandlerCtx,
    own_binders: &std::collections::HashSet<StructId>,
) -> Vec<(String, crate::ty::Ty)> {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out: Vec<(String, crate::ty::Ty)> = Vec::new();
    // DETERMINISM: `ctx.arms` is a HashMap, whose `.values()` iteration order is nondeterministic. The
    // first-seen capture order decides the synthesized spec signature's extra-param order AND the
    // `effect_spec_captures` arg order — so an unstable walk order would produce a run-to-run-varying
    // signature / arg order → non-reproducible wasm (violates the frozen-hash discipline). Walk the arm
    // bodies in a STABLE order, sorted by each arm's `(decl, op-index)` key. (Copilot PR #504.)
    let mut arm_bodies: Vec<((u32, u32), StructId)> =
        ctx.arms.iter().map(|(&k, a)| (k, a.body)).collect();
    arm_bodies.sort_by_key(|&(k, _)| k);
    for (_, body) in arm_bodies {
        collect_captures(db, body, own_binders, &mut seen, &mut out);
    }
    out
}

/// Walk `node`, recording each free `Param`-resolving name whose binder is not in `own_binders` (a capture).
fn collect_captures(
    db: &mut Db,
    node: StructId,
    own_binders: &std::collections::HashSet<StructId>,
    seen: &mut std::collections::HashSet<String>,
    out: &mut Vec<(String, crate::ty::Ty)>,
) {
    // A captured reference resolves either directly to `Param { binder }` or via a `Ref` chain reaching a
    // param binder (mirrors `count_param_refs`). Either way, if that binder is NOT bound within the
    // specialization it is an enclosing-fn capture.
    let capture_binder = match resolved_of(db, node) {
        Resolved::Param { binder } => Some(binder),
        Resolved::Ref { value } => {
            let mut target = value;
            loop {
                match resolved_of(db, target) {
                    Resolved::Param { binder } => break Some(binder),
                    Resolved::Ref { value: next } => target = next,
                    _ => break None,
                }
            }
        }
        _ => None,
    };
    if let Some(binder) = capture_binder
        && !own_binders.contains(&binder)
        && let Some(name) = db.ast.as_name(node).map(str::to_string)
        && !seen.contains(&name)
    {
        let ty = crate::infer::type_of(db, node);
        // Only thread a capture with a DETERMINED type — an undetermined one cannot annotate the extra
        // spec param (mirrors the `orig_params` `Ty::Any` decline). A capture we cannot type makes the
        // whole specialization decline (returned as an empty marker the caller checks).
        seen.insert(name.clone());
        out.push((name, ty));
    }
    if let Struct::List(children) = db.ast.get(node).clone() {
        for c in children {
            collect_captures(db, c, own_binders, seen, out);
        }
    }
}

fn count_param_refs(db: &mut Db, node: StructId, binder: StructId) -> u32 {
    // A reference matches the way `beta_reduce` substitutes: either a `Param { binder }` whose binder IS
    // the arm param, OR a `Ref { value }` whose chain reaches that binder transitively (an op-arm param
    // `p` used as `(. p 0)` resolves to a `Ref` reaching `p`'s declaration occurrence, not a `Param`).
    let here = match resolved_of(db, node) {
        Resolved::Param { binder: b } => u32::from(b == binder),
        Resolved::Ref { value } => {
            let mut target = value;
            let mut hit = false;
            loop {
                if target == binder {
                    hit = true;
                    break;
                }
                match resolved_of(db, target) {
                    Resolved::Ref { value: next } => target = next,
                    _ => break,
                }
            }
            u32::from(hit)
        }
        _ => 0,
    };
    let below = match db.ast.get(node).clone() {
        Struct::List(children) => children
            .iter()
            .map(|&c| count_param_refs(db, c, binder))
            .sum(),
        Struct::Atom(_) => 0,
    };
    here + below
}

/// Whether the argument subtree at `node` reaches ANY perform — of THIS handler's op (discharged) OR a
/// FOREIGN one (an outer handler's / host op) — following NON-RECURSIVE callee bodies (bounded depth). The
/// duplication guard's precise "does this argument carry an effect that must not be copied" test. UNLIKE
/// `body_reaches_foreign_perform`, it does NOT over-report an unresolvable/record-field-pair head as an
/// effect: a record literal `(record (a 3) (b 4))` resolves its field pairs as `Apply` nodes whose "head"
/// is the label `a` (no `meta_apply`, no lambda body), which the conservative foreign-walk misreads as an
/// unresolvable call and flags — spuriously declining a PURE record argument. Here an `Apply` whose head is
/// not an effect op and not a followable function simply DESCENDS into its args (a perform can only hide in
/// a sub-expression, never in a bare label head), so a pure compound argument is correctly effect-free. A
/// recursive callee is over-reported (it may perform; bounded, safe — a recursive performing arg is rare
/// and declining it is sound). Combines the discharged-op detection (`is_perform`) with the foreign-op one
/// (`effect_op_of` outside `ctx.arms`).
fn arg_reaches_any_perform(db: &mut Db, node: StructId, ctx: &HandlerCtx) -> bool {
    // The inner walk takes NO `HandlerCtx`: an `effect_op_of` head is a perform regardless of which handler
    // owns it, so this ANY-perform detector never consults `ctx` (unlike its sibling
    // `body_reaches_foreign_perform`, which needs it to distinguish a FOREIGN op). Dropping the pass-through
    // parameter clears clippy's only-used-in-recursion lint; `ctx` stays in the outer signature for a
    // uniform call shape with the sibling detector.
    let _ = ctx;
    fn walk(db: &mut Db, node: StructId, depth: u32) -> bool {
        if depth > 32 {
            return true; // too deep — assume it may perform (safe over-report)
        }
        if let Resolved::Apply { head, args } = resolved_of(db, node) {
            // A perform of ANY effect operation — this handler's (discharged) or another's (foreign).
            if crate::eval::effect_op_of(db, head).is_some() {
                return true;
            }
            // A user function call: follow a NON-RECURSIVE callee body; a recursive one over-reports.
            // (A non-function head — a compound constructor, a record field-pair label — is NOT followed:
            // it hides no perform in the head, only its args, which the descent below covers.)
            if let Some(callee) = crate::eval::lambda_body(db, head)
                .or_else(|| crate::eval::lambda_body_of_nullary(db, head))
            {
                if crate::eval::is_recursive(db, callee) {
                    return true;
                }
                if walk(db, callee, depth + 1) {
                    return true;
                }
            }
            return args.iter().any(|&a| walk(db, a, depth + 1));
        }
        // A bare `resume` reached in an argument is an effect too.
        if matches!(resolved_of(db, node), Resolved::Resume { .. }) {
            return true;
        }
        match db.ast.get(node).clone() {
            Struct::List(children) => children.iter().any(|&c| walk(db, c, depth + 1)),
            Struct::Atom(_) => false,
        }
    }
    walk(db, node, 0)
}

/// Whether the subtree at `node` transitively reaches a FOREIGN perform — an effect operation NOT
/// discharged by THIS handler `ctx` (an outer handler's effect, or a host-delegated op), following
/// NON-RECURSIVE calls into their bodies (bounded depth). CONSERVATIVE (over-reports, never under-reports):
/// a recursive/unresolvable call, or a chain deeper than the bound, reports `true`. Used to gate the
/// MULTI-shot two-hole refold: re-running the continuation per resume must not re-issue a foreign/HOST
/// effect (the host-composition invariant), so a body reaching a foreign perform stays one-shot-only.
fn body_reaches_foreign_perform(db: &mut Db, node: StructId, ctx: &HandlerCtx) -> bool {
    fn walk(db: &mut Db, node: StructId, ctx: &HandlerCtx, depth: u32) -> bool {
        if depth > 16 {
            return true; // too deep — assume it may reach a foreign perform (safe over-report)
        }
        if let Resolved::Apply { head, args } = resolved_of(db, node) {
            // An effect op NOT in this handler's arms is FOREIGN (a perform of it re-issues outside).
            if let Some((decl, idx)) = crate::eval::effect_op_of(db, head)
                && !ctx.arms.contains_key(&(decl.0, idx))
            {
                return true;
            }
            // A user call: follow a non-recursive callee; a recursive/unresolvable head over-reports.
            if crate::eval::effect_op_of(db, head).is_none() && !is_pure_operator_head(db, head) {
                match crate::eval::lambda_body(db, head)
                    .or_else(|| crate::eval::lambda_body_of_nullary(db, head))
                {
                    Some(callee) if !crate::eval::is_recursive(db, callee) => {
                        if walk(db, callee, ctx, depth + 1) {
                            return true;
                        }
                    }
                    _ => return true, // recursive / unresolvable — over-report
                }
            }
            return args.iter().any(|&a| walk(db, a, ctx, depth + 1));
        }
        match db.ast.get(node).clone() {
            Struct::List(children) => children.iter().any(|&c| walk(db, c, ctx, depth + 1)),
            Struct::Atom(_) => false,
        }
    }
    walk(db, node, ctx, 0)
}

/// Whether `param` is the unit placeholder `()` (a nullary operation's single "parameter", which binds
/// nothing). `()` resolves to `Resolved::Unit`.
fn is_unit_param(db: &mut Db, param: StructId) -> bool {
    matches!(resolved_of(db, param), Resolved::Unit)
}

/// Whether the subtree at `node` performs an operation `ctx` discharges — a fast pre-check so a
/// perform-free subtree is copied wholesale rather than threaded position-by-position. Structural walk.
fn subtree_performs(db: &mut Db, node: StructId, ctx: &HandlerCtx) -> bool {
    // MEMOIZE per `(node, ctx.key)`: the classifiers `strongly_pure`/`pure_hole` call this at MANY nodes
    // as they descend a handle body, and `strongly_pure` in particular re-ran this WHOLE-subtree walk at
    // every node it visited — so a deep body (an N-perform nested-`let` chain) recomputed the same node's
    // verdict O(depth) times, making the scan O(N²) and the fold O(N³). Whether a subtree performs is a
    // pure function of the node and the DISCHARGED-OP SET (`ctx.key`, the resolved-identity string), so a
    // memo collapses the repeats to O(1). (Node ids are never reused with a different meaning; a synthesized
    // node gets a fresh id, so a stale entry cannot mislead.)
    let cache_key = (node, ctx.key.clone());
    if let Some(&v) = db.subtree_performs_cache.get(&cache_key) {
        return v;
    }
    let v = subtree_performs_uncached(db, node, ctx);
    db.subtree_performs_cache.insert(cache_key, v);
    v
}

fn subtree_performs_uncached(db: &mut Db, node: StructId, ctx: &HandlerCtx) -> bool {
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
    // A LAMBDA VALUE performs nothing when CONSTRUCTED — its body's effects fire only when it is APPLIED
    // (an `Apply` node the perform/inline arms handle separately). So do NOT descend into a lambda body
    // here: `(let ((f (fn (x) (+ x (E.op))))) (f 10))` binds a pure lambda value, then the application
    // `(f 10)` is where the discharged op surfaces (via the inline arm). Descending into the body would
    // misclassify the pure binding as effectful, declining a foldable let-bound-performing-lambda. (A
    // lambda that ESCAPES unapplied genuinely performs nothing.)
    if matches!(resolved_of(db, node), Resolved::Lambda { .. }) {
        return false;
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

/// A DEEP structural copy that re-pushes EVERY node fresh — no sharing anywhere in the subtree. Unlike
/// [`copy_pure`] (`beta_reduce`), which returns a RESOLVE-PINNED name node as-is (its pinned-name
/// fast-path, avoiding exponential re-resolution on deep inline chains), this forces a genuinely fresh atom
/// for every leaf, so the result shares NO node with the original — nor with a SIBLING deep copy of the
/// same source.
///
/// Needed at the resume splice: a `resume(v, s)` arm whose value and next-state are the SAME source node —
/// `resume(a, a)` (dispatch/done handing the op's arg back AND as the next state), or an annotated-param
/// helper's substituted arg `(: fuel Int64)` — must become TWO INDEPENDENT nodes (one lands in the value
/// position, one as the self-call's trailing state arg). `copy_pure` alone shares them: for a bare pinned
/// name it returns the same id; for a compound `(: fuel Int64)` it re-pushes the `(:` list fresh but its
/// pinned inner `fuel` leaf is still shared. Either way ONE leaf ends up under TWO parents — a single-parent-
/// arena orphan → CDZ0101 (the effectful-helper-in-a-self-call-arg bug). A fully-fresh copy gives each splice
/// its own subtree; each fresh name occurrence re-resolves against the scope it lands in (the specialized
/// def's sig, which carries the driver's own params).
fn deep_fresh_copy(db: &mut Db, node: StructId) -> StructId {
    match db.ast.get(node).clone() {
        Struct::Atom(lid) => {
            let leaf = db.ast.leaf(lid).clone();
            db.push_atom(leaf)
        }
        Struct::List(children) => {
            let copied: Vec<StructId> = children.iter().map(|&c| deep_fresh_copy(db, c)).collect();
            db.push_list(copied)
        }
    }
}

#[cfg(test)]
mod desugar_tests {
    use super::*;
    use crate::testkit::parse;

    /// The CANONICAL handle `(handle E seed (bare-arm…) body)` desugars to the INTERNAL
    /// `(handle-internal seed ((. E op)-arm…) body)` the resolver consumes: the head is RE-SPELLED, `E`
    /// leaves the head, and each arm's bare op becomes its `(. E op)` projection, params/state/body kept.
    #[test]
    fn desugars_canonical_handle_to_internal_shape() {
        let mut ast = parse("(handle Fresh 0 ((next (u) s (resume s (+ s 1)))) (Fresh.next))");
        desugar_handles(&mut ast);
        // The root is now the internal node `(handle-internal 0 (arm…) body)` — a distinct head, so a
        // source `handle` and the desugared node never share a spelling.
        assert_eq!(ast.head_name(ast.root), Some(HANDLE_INTERNAL));
        let tail = ast
            .as_form(ast.root, HANDLE_INTERNAL)
            .expect("re-spelled to the internal head");
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

    /// The retired effect-name-less shape `(handle seed (arm…) body)` is NOT canonical, so the desugar
    /// (which fires only on the 5-child form) leaves its head as the source `handle` — it is NOT
    /// re-spelled to the internal head, and downstream `resolve_noncanonical_handle` rejects it. This is
    /// what makes the old form unwritable: only the desugar produces `handle-internal`.
    #[test]
    fn leaves_noncanonical_handle_headed_handle_for_rejection() {
        let mut ast = parse("(handle 0 (((. Fresh next) (u) s (resume s (+ s 1)))) (Fresh.next))");
        let before = ast.structure.len();
        desugar_handles(&mut ast);
        assert_eq!(
            ast.structure.len(),
            before,
            "no nodes appended for a non-canonical handle"
        );
        // Head is STILL the source `handle` (not re-spelled) — the marker the resolver rejects on.
        assert_eq!(ast.head_name(ast.root), Some("handle"));
    }
}
