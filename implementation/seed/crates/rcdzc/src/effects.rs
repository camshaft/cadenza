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
    // Confirm EVERY arm is a bare-op arm: `(bare-op (params…) state body)` (4-part tail/abortive) or
    // `(bare-op (params…) state k body)` (5-part general `ctl`-style), whose op is a bare NAME (not an
    // already-projected `(. E op)`). If any arm is not this shape, this is not the canonical form. Both
    // arities re-project identically (`apply_handle_plan` keeps `parts[1..]`), so a `k`-binding arm desugars
    // like any other.
    for &arm in arm_nodes {
        let Struct::List(parts) = ast.get(arm) else {
            return None;
        };
        if !matches!(parts.len(), 4 | 5) || ast.as_name(parts[0]).is_none() {
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
        let dot = push_atom(ast, Leaf::Name(".".into()));
        let eff = if i == 0 {
            plan.effect_occ
        } else {
            push_atom(ast, Leaf::Name(plan.effect_name.clone().into()))
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
            kept[0] = push_atom(ast, Leaf::Name(HANDLE_INTERNAL.into()));
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
    let head = push_atom(ast, Leaf::Str("record".into()));
    let mut children = vec![head];

    // `(meta t)` — the effect type-value, so a later pass can recover the effect's identity.
    let eff_ty = effect_typeval(ast, decl);
    children.push(meta_field(ast, "t", eff_ty));

    // One field per operation, its value the operation-value record. The operation's INDEX in
    // declaration order is its stable operation index.
    for (index, op) in decl.ops.iter().enumerate() {
        let value = op_value(ast, decl, op, index as u32);
        let k = push_atom(ast, Leaf::Name(op.name.clone().into()));
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
    let head = push_atom(ast, Leaf::Str("record".into()));
    // `(meta t)` — the operation's arrow type, wrapped in a ZERO-PARAM `(fn () (-> Param Result))`. The
    // wrapper is LOAD-BEARING (the same lesson the monomorphic String/Bytes ops learned): a BARE arrow
    // as `(meta t)` makes `typeval_of` collapse the whole op-value RECORD to `Ty::Type` (an arrow IS a
    // type-value), so `(. E op)` would type as `Type` and `(+ (E.op) 1)` faults "unify Int64 with Type".
    // The `(fn () …)` wrapper makes `scheme_of` read a monomorphic SCHEME (no quantified variables)
    // rather than a type-value, so the op has a function type and a performance types as an application.
    // A malformed `(op NAME)` with no type gets `Unit`.
    let arrow = match op.ty {
        Some(t) => copy_subtree(ast, t),
        None => push_atom(ast, Leaf::Name("Unit".into())),
    };
    let ty = {
        let fn_head = push_atom(ast, Leaf::Name("fn".into()));
        let params = push_list(ast, vec![]);
        push_list(ast, vec![fn_head, params, arrow])
    };
    let t_field = meta_field(ast, "t", ty);
    // `(meta apply)` = the perform marker (declines at lowering until E1).
    let apply = {
        let ih = push_atom(ast, Leaf::Name("intrinsic".into()));
        let who = push_atom(ast, Leaf::Name("perform".into()));
        push_list(ast, vec![ih, who])
    };
    let apply_field = meta_field(ast, "apply", apply);
    // `(meta effect-op)` = `(effect-op <decl> <index>)` — the operation's identity.
    let identity = {
        let eo = push_atom(ast, Leaf::Name("effect-op".into()));
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
    let eff_head = push_atom(ast, Leaf::Name("effect".into()));
    let nm = push_atom(ast, Leaf::Name(decl.name.clone().into()));
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
    /// FINDING-24: whether the growing-state per-dispatch `#st` let-bind may fire on THIS thread pass. TRUE on
    /// the paths that DRAIN `pending` — the straight-line handle body (`reduce_handle`'s `drain_and_wrap` at
    /// the top) and multi-value specialization (`thread_returning_tuple` drains per leaf). FALSE for
    /// SINGLE-RETURN specialization (`specialize_recursive`'s bare `thread`, which has no `drain_and_wrap`): a
    /// `#st` pushed there would never materialize → an orphan `#st..` CDZ0101 reference in the specialized
    /// body. That path is ALSO immune to the exponential (finding-24 tick 1369: a LOOP-driven single static
    /// call site threads the state as a fn PARAMETER, not by re-substituting a growing expr per dispatch), so
    /// suppressing the bind there loses no size win — it is pure harm to fire. Interior-mutable so the
    /// specialize path can flip it around its own thread without rebuilding the ctx.
    bind_growing_state: std::cell::Cell<bool>,
    /// [tpwJ Option A-tight] TRUE while `specialize_recursive` is threading a RECURSIVE-DRIVER body. The
    /// cross-scope tuple COLLAPSE (a straight-line per-dispatch `#st`-bind + tuple-projection transform) is
    /// NOT sound under recursive specialization: `thread_returning_tuple` infers the recursive result type
    /// from the threaded shape, and a tuple-projected next-state leaves that result type undetermined (rq3).
    /// Excluding recursive drivers from the collapse is principled (a separate larger unit would teach
    /// `thread_returning_tuple` about the tuple-projected state); such an arm stays on the distribute path.
    in_recursive_specialize: std::cell::Cell<bool>,
    /// [tpwJ Option A-tight, per-handler ALL-OR-NOTHING] TRUE only when EVERY arm of this handler MESHES with
    /// the cross-scope tuple COLLAPSE: each arm either (a) HAS the shared-let-across-resume-slots shape (a
    /// collapse candidate), (b) is ABORTIVE (no resume — threads no state), or (c) threads the state UNCHANGED
    /// (its tail resume's next-state IS the arm's state binder — a trivial reader). A per-dispatch collapse
    /// threads state as `(. #st_vs 1)`; a DISTRIBUTE arm of the SAME handler that MODIFIES the state cannot
    /// read that projected state without orphaning (rrb1 binder 10055: a distribute arm reads a
    /// collapse-threaded projected state → cross-path mismatch). Deciding ONCE over `ctx.arms` and enabling the
    /// collapse only when all arms mesh keeps tpwJ (K.type shared-let + K.fin trivial reader) on the collapse
    /// path while rrb1/lru1/xh1 stay ALL on distribute → zero regression. The per-dispatch gate (hoistable /
    /// drain-safe / non-growing / single-slot / non-recursive) still applies on top for each firing dispatch.
    collapse_enabled: std::cell::Cell<bool>,
    /// [pyfb3] THIS handler's discharged ops drawn >= 2 times (statically) in the handle body — the
    /// multi-dispatch ops. Gates the nullary-foreign-perform-let collapse candidate: fires only for an op
    /// drawn >=2 (a single dispatch folds the same shape strict via distribute, no heap slot — as7). Empty at
    /// `new` (no body there); `reduce_handle` populates it from the body and re-evaluates `collapse_enabled`.
    multi_dispatch_ops: std::cell::RefCell<std::collections::HashSet<(u32, u32)>>,
}

/// Decide `collapse_enabled` (the tpwJ per-handler ALL-OR-NOTHING mesh) over the op→arm map: enable the
/// cross-scope tuple COLLAPSE only when >=1 arm is a collapse candidate AND EVERY arm MESHES (is itself a
/// candidate, is abortive, or threads its state UNCHANGED). Iterates the MAP so each arm's candidate test gets
/// ITS op identity (`multi_dispatch_ops` is keyed per-op). Called by `HandlerCtx::new` (empty multi set — body
/// unavailable) and re-called by `reduce_handle` once the body's per-op draw counts are known.
fn collapse_enabled_for(
    db: &mut Db,
    arms: &HashMap<(u32, u32), HandleArm>,
    multi_dispatch_ops: &std::collections::HashSet<(u32, u32)>,
) -> bool {
    let any_candidate = arms
        .iter()
        .any(|(&op, a)| arm_is_collapse_candidate(db, a.body, op, multi_dispatch_ops, arms));
    let all_mesh = arms.iter().all(|(&op, a)| {
        if arm_is_collapse_candidate(db, a.body, op, multi_dispatch_ops, arms)
            || !arm_has_resume(db, a.body)
        {
            return true;
        }
        // Threads the state UNCHANGED: EVERY resume next-state (peeled through match/if branches) is the arm's
        // state binder — a distribute arm that reads a collapse-threaded `(. #st_vs 1)` projection and passes
        // it straight through (safe, tpwJ's `fin`). A next-state that MODIFIES the state does not mesh (rrb1).
        let mut nexts: Vec<StructId> = Vec::new();
        let got = arm_resume_next_states(db, a.body, &mut nexts).is_some();
        let state = a.state;
        got && !nexts.is_empty() && nexts.iter().all(|&next| node_refs_binder(db, next, state))
    });
    any_candidate && all_mesh
}

/// One handler's state in a (possibly merged) [`HandlerCtx`]: the effect declaration whose operations
/// thread it, and the state's TYPE (from the handle's `init` seed; `None` if undetermined → a recursive
/// specialization declines). The slot's INDEX is its position in `HandlerCtx::slots` — the trailing
/// state-parameter position a specialized fn threads it through.
#[derive(Clone)]
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
        // EXCLUDE a general `ctl`-style arm that BINDS `k` (`cont: Some`): it has no `resume` (it invokes
        // the continuation via `k`, not `resume`), but it is E5-GENERAL, not abortive — the escaping-k
        // reification consumes it. Misclassifying it abortive makes the fold treat the arm body (which uses
        // `k` as a value) as an abort value → a "value is not applyable" lowering error on the unreified `k`.
        let abortive = arms
            .iter()
            .filter(|(_, arm)| arm.cont.is_none() && !arm_has_resume(db, arm.body))
            .map(|(&k, _)| k)
            .collect();
        // [tpwJ A-tight per-handler ALL-OR-NOTHING] Enable the cross-scope tuple COLLAPSE for THIS handler
        // only when >=1 arm is a collapse candidate (shared-let-across-resume-slots) AND EVERY arm MESHES with
        // it: each arm either has that shared-let shape, is abortive (no resume — no state thread), or threads
        // the state UNCHANGED (its tail resume's next-state IS the arm's state binder — a trivial reader like
        // tpwJ's `fin`). A distribute arm that MODIFIES the state cannot safely read a collapse-threaded
        // `(. #st_vs 1)` projection (rrb1 cross-path orphan), so if any arm fails to mesh, disable the collapse
        // and keep the whole handler on the distribute path. Decided ONCE here over all arms.
        // At construction the handle BODY is not available, so the multi-dispatch set is EMPTY here — the
        // nullary-foreign-perform-let candidate (pyfb3) needs the body's per-op draw count. `reduce_handle`
        // populates `multi_dispatch_ops` from the body and RE-EVALUATES `collapse_enabled` right after `new`
        // (before any thread reads it); this initial value covers the existing (body-independent) candidates.
        let no_multi = std::collections::HashSet::default();
        let collapse_enabled = collapse_enabled_for(db, &arms, &no_multi);
        HandlerCtx {
            arms,
            key,
            slots,
            abortive,
            abort_value: std::cell::Cell::new(None),
            pending: std::cell::RefCell::new(Vec::new()),
            temp_ctr: std::cell::Cell::new(0),
            bind_growing_state: std::cell::Cell::new(true),
            in_recursive_specialize: std::cell::Cell::new(false),
            multi_dispatch_ops: std::cell::RefCell::new(std::collections::HashSet::default()),
            collapse_enabled: std::cell::Cell::new(collapse_enabled),
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
    {
        // Scan the callee body AND — when accum rewrote it into a seed wrapper + copy `f$acc` — the COPY's
        // body (the seed wrapper no longer holds the recursion / inner-op call; those moved to `f$acc`).
        let reaches = callee_reaches_outer_effect(db, body, inner_decl, merged, 0)
            || db
                .transformed
                .get(&callee_def)
                .copied()
                .and_then(|acc| db.defs[acc].body)
                .is_some_and(|acc_body| {
                    callee_reaches_outer_effect(db, acc_body, inner_decl, merged, 0)
                });
        if reaches {
            return true;
        }
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
    {
        // A DIRECT outer-effect perform in the callee body.
        if decl != inner_decl {
            return true;
        }
        // An INNER-op perform whose ARM resume-value performs the OUTER effect (part 2): follow the arm body
        // — `(step (u) t (resume (A.tick) t))` performs the outer `A`, hidden from a callee-body-only scan.
        if let Some(arm) = merged.arms.get(&(decl, idx))
            && callee_reaches_outer_effect(db, arm.body, inner_decl, merged, depth + 1)
        {
            return true;
        }
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
        Some(eff) => !eff.ops.iter().any(|o| *o.name == *key.name),
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

/// Whether the operation `key` — absent from the effect NAMED `name` that a bare `E` reference resolves to
/// (the FIRST-declared same-named effect) — IS declared on a DIFFERENT, LATER `(effect name …)` of the same
/// name. Two same-named effects are DISTINCT (an effect's identity is its declaration, not its name —
/// `capabilities-and-effects.md` §An Effect's Operations Are A Closed Set; pinned by `14-effects:3129`), so
/// a bare `E` resolves the first and `E.key` where `key` lives only on a later same-named `E` fails "no
/// operation `key`" — baffling, since `key`'s declaration is visibly present. This detects exactly that
/// case so `no_field_reject` can explain it (the diagnostic-quality half of the works-as-specified
/// duplicate-effect finding). `resolved_first_occ` is the declaration occurrence the bare name resolved to
/// (so we only flag OTHER declarations). Returns true iff some same-named effect with a DIFFERENT occurrence
/// declares an op called `key`.
pub fn op_on_other_same_named_effect(
    db: &Db,
    name: &str,
    resolved_first_occ: StructId,
    key: &str,
) -> bool {
    db.effect_decls.iter().any(|e| {
        e.name == name && e.occ != resolved_first_occ && e.ops.iter().any(|o| o.name == key)
    })
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

/// A SHADOWED-OP hint for a handler arm `(. E k)` whose op `k` is NOT declared on the effect a bare `E`
/// resolves to (the FIRST same-named declaration) but IS declared on a DIFFERENT, LATER `(effect E …)` —
/// the handler-arm twin of `no_field_reject`'s perform-site shadow hint. Two same-named effects are DISTINCT
/// (an effect's identity is its declaration, not its name — `14-effects:3129`), so a handler on a bare `E`
/// discharges only the FIRST declaration's ops; an arm naming a LATER `E`'s op is out of reach. Returns the
/// explanatory hint suffix (leading ` — `, so a downstream dedup keying on the sentence core is unaffected),
/// or `None` if `op` is not `(. E k)` on an effect, or `k` is a genuine typo (not on any same-named later
/// `E`) — in which case the ordinary `declared_op_hint` did-you-mean stands.
pub fn arm_op_shadow_hint(db: &mut Db, op: StructId) -> Option<String> {
    let Resolved::Member { operand, key } = resolved_of(db, op) else {
        return None;
    };
    let name = db.ast.as_name(operand)?.to_string();
    let first_occ = db.effect_decl_by_name(&name)?;
    if op_on_other_same_named_effect(db, &name, first_occ, &key.name) {
        Some(format!(
            " — operation `{}` is declared on a LATER `(effect {name} …)`; a handler on a bare `{name}` \
             discharges the FIRST declaration (an effect's identity is its declaration, not its name), so \
             that operation is out of reach here — merge the operations into one `(effect {name} …)` or \
             handle the intended declaration",
            key.name
        ))
    } else {
        None
    }
}

/// The CANONICAL identity of a handler arm's operation `(. E k)` — `(effect-declaration-occurrence,
/// op-name)`. Two arms discharge the SAME operation exactly when their identities are equal, so this is
/// the key a duplicate-arm check dedups on. Keyed by the effect's DECLARATION (not just the name) so two
/// effects each declaring `emit` never collide — the same closed-set identity `handler_missing_operations`
/// and the reduction plan use. `None` if `op` is not `(. E k)` on an effect (an undeclared/malformed arm,
/// whose own fault CDZ0403 is reported instead). The op key's OCCURRENCE (for a delete fix's anchor) is
/// read separately via `arm_op_key_occ`.
pub fn arm_op_identity(db: &mut Db, op: StructId) -> Option<(u32, std::sync::Arc<str>)> {
    let Resolved::Member { operand, key } = resolved_of(db, op) else {
        return None;
    };
    let decl = effect_decl_of_value(db, operand)?;
    // `key.name` is `Arc<str>` — a duplicate-arm check only compares/hashes this identity, so carry the
    // `Arc<str>` (a refcount bump) rather than materializing a fresh `String` at each call.
    Some((decl, key.name.clone()))
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
    // The operation names the arms bind (an arm op is `(. E k)` → its key name). `key.name` is `Arc<str>`;
    // this is a membership set (`bound.contains` below), so carry the `Arc<str>` (a refcount bump) rather
    // than materializing a fresh `String` per arm.
    let bound: std::collections::HashSet<std::sync::Arc<str>> = arms
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
        .filter(|o| !bound.contains(o.name.as_str()))
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
/// WARNING: The ELIDED-UNIT convention: a `(-> Unit R)` operation accepts BOTH a 0-binder arm (`(op () s …)`,
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
        Resolved::Member { key, .. } => key.name.to_string(),
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
/// Canonical dotted key of a host operation — the single source of truth for the `e.op` name a delegated
/// host call is named/observed/matched by. The EFFECT name is KEBAB-NORMALIZED (the component/WIT extern
/// convention — `kebab_extern_name`: `Env`→`env`, `Log`→`log`, `Param`→`param`, `E`→`e`), the OP name is
/// kept VERBATIM. This is the observed name cdz-run binds/records; the corpus `(host-responses …)` fixtures
/// are written in mixed casing (`env.width` normalized, `Param.width` source), but cdz-run matches after
/// normalizing BOTH sides — so a consumer of this key MUST likewise normalize the effect part of a recorded
/// response key before comparing (the rust gate driver does: it splits the response key, kebab-normalizes
/// the effect, then derives the same shim ident this backend emits). Spec: host-interface-binding.md
/// #a-host-import-is-a-wit-typed-function. (Landed with its first rcdzc caller — the rust `Core::HostCall`
/// emit — since an uncalled helper trips clippy `dead_code`.)
pub(crate) fn canonical_host_op_key(effect: &str, op: &str) -> String {
    let eff = crate::backend::common::export_name::kebab_extern_name(effect);
    format!("{eff}.{op}")
}

pub fn perform_host_target(
    db: &mut Db,
    perform: StructId,
    head: StructId,
) -> Option<(String, String, crate::ty::Ty)> {
    // The op's declaring effect + its name — the op head is a member access `(. E op)`.
    let (decl, _idx) = crate::eval::effect_op_of(db, head)?;
    let op_name = match resolved_of(db, head) {
        Resolved::Member { key, .. } => key.name.to_string(),
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

/// For a call whose head resolves to the lambda `callee_body`, return — per parameter position (0..`arity`)
/// — the effect decls under which the callee APPLIES that parameter, i.e. the handlers/delegations
/// enclosing an application `(param_i …)` in the callee body. Used by [`check_no_home_walk`] to home a
/// LAMBDA ARGUMENT at its APPLICATION site rather than its definition site: if `callee_body` applies its
/// param `i` under a `handle E` (`with-seed(body) = handle E … (body unit)`), a lambda arg passed there
/// has its `E`-performs homed, so it must be walked with `E` added to the handled set — else a legitimate
/// `handler-runs-a-passed-closure` idiom is falsely rejected CDZ0401. Returns an empty inner vec for a
/// param the callee applies under NO extra handler (a bare `(body unit)` — `apply-fn`), so a genuinely-
/// ungranted effect in the lambda STAYS reported (soundness). Conservative: only a direct `(param_i …)`
/// application inside the callee's own `handle`/`host` scopes contributes; a param passed onward to a
/// further call is not chased (a miss → declines/reports as before, never a wrong grant).
/// If `head` (an application head) resolves to one of the lambda's `params` — directly as a `Param`, or
/// through a `Ref` chain that reaches a param binder — return that param's index. (A `let`/param reference
/// resolves to `Ref { value }` whose chain reaches the binder; mirrors `ctl_arm_lexical_k_to_resume`'s
/// `refs_to_k`.) Used to detect `(param_i …)` applications inside a callee body.
fn param_index_of_head(db: &mut Db, head: StructId, params: &[StructId]) -> Option<usize> {
    // The binder each param occurrence names.
    let param_binders: Vec<Option<StructId>> = params
        .iter()
        .map(|&p| match resolved_of(db, p) {
            Resolved::Param { binder } => Some(binder),
            _ => None,
        })
        .collect();
    let target = match resolved_of(db, head) {
        Resolved::Param { binder } => binder,
        Resolved::Ref { value } => {
            let mut t = value;
            loop {
                match resolved_of(db, t) {
                    Resolved::Param { binder } => break binder,
                    Resolved::Ref { value: n } => t = n,
                    _ => return None,
                }
            }
        }
        _ => return None,
    };
    param_binders.iter().position(|&b| b == Some(target))
}

fn param_apply_extra_handled(
    db: &mut Db,
    head: StructId,
    callee_body: StructId,
    arity: usize,
    // INTER-PROCEDURAL recursion budget: this fn follows a KNOWN sub-callee (line ~`sub_extra`) by
    // RE-ENTERING itself, and `is_recursive` cannot break every cycle — a self-call hidden inside a nested
    // fold closure (`(count e)` under `(List.fold es _ (fn (acc e) (+ acc (count e))))`) is invisible to the
    // call graph (`collect_callees` stops at a nested-`fn` boundary), so the callee reads as non-recursive
    // and is chased forever. This `depth` bounds that inter-procedural chase — it seeds the inner `walk` and
    // grows by one per sub-callee follow, so the `depth < 32` gate on the transitive follow terminates the
    // walk (a compiler must never overflow its stack — `self-hosting-and-bootstrap.md`). SEEDED FROM THE
    // CALLER'S ACTIVE `check_no_home_walk` WALK DEPTH (the sole external caller, ~line 1377), not reset to 0:
    // this fn is invoked from within that walk's own recursion, so SHARING the budget bounds the COMBINED
    // stack (outer walk + this inter-procedural follow) by the single 32-gate — strictly more conservative
    // against overflow than an independent fresh-0 budget, and still sound (a shared budget only trips the
    // follow-gate EARLIER, never later). The internal recursive follow re-enters at `depth + 1`.
    depth: u32,
) -> Vec<Vec<u32>> {
    let Some(params) = crate::eval::lambda_params_of(db, head) else {
        return Vec::new();
    };
    let mut out: Vec<Vec<u32>> = vec![Vec::new(); arity];
    // Walk the callee body tracking the handled set (mirrors `check_no_home_walk`'s handle/host threading);
    // at an application whose head resolves to callee param `i`, record the current handled set for `i`.
    fn walk(
        db: &mut Db,
        node: StructId,
        params: &[StructId],
        handled: &mut Vec<u32>,
        out: &mut [Vec<u32>],
        depth: u32,
    ) {
        if depth > 64 {
            return;
        }
        match resolved_of(db, node) {
            Resolved::Apply { head, args } => {
                // Is the head a reference to one of the callee's params? If so, that param is APPLIED here,
                // under the current handled set — record it (only the extra grants matter to the caller).
                if let Some(i) = param_index_of_head(db, head, params)
                    && i < out.len()
                    && out[i].is_empty()
                {
                    out[i] = handled.clone();
                }
                // TRANSITIVE homing: if the head is a KNOWN (non-recursive) callee and one of THIS callee's
                // params is passed as an argument to it, that param is applied wherever the SUB-callee applies
                // its own corresponding param — so it inherits the sub-callee's extra-handled set (plus the
                // handlers active here). `(def (outer b) (inner b))` with `inner` applying its param under
                // `handle R`: `outer`'s `b` inherits `{R}`, so a lambda passed to `outer` homes its `R`
                // perform. Bounded by `depth` (guards a recursive/cyclic sub-callee); a recursive sub-callee
                // is not chased (its `lambda_body` is a fixpoint we do not unfold). Only fills an as-yet-empty
                // slot (first-seen application wins, matching the direct case).
                if depth < 32
                    && param_index_of_head(db, head, params).is_none()
                    && let Some(sub_body) = crate::eval::lambda_body(db, head)
                        .or_else(|| crate::eval::lambda_body_of_nullary(db, head))
                    && !crate::eval::is_recursive(db, sub_body)
                {
                    // Re-enter at `depth + 1`: the follow itself is one inter-procedural step, and the
                    // sub-call's inner walk must SEE the accumulated depth (it re-seeds at this value), or a
                    // graph-invisible cycle (a self-call hidden in a nested fold closure) never trips the gate.
                    let sub_extra =
                        param_apply_extra_handled(db, head, sub_body, args.len(), depth + 1);
                    for (j, &a) in args.iter().enumerate() {
                        if let Some(i) = param_index_of_head(db, a, params)
                            && i < out.len()
                            && out[i].is_empty()
                            && sub_extra.get(j).is_some_and(|e| !e.is_empty())
                        {
                            let mut set = handled.clone();
                            set.extend(sub_extra[j].iter().copied());
                            out[i] = set;
                        }
                    }
                }
                walk(db, head, params, handled, out, depth);
                for &a in args.iter() {
                    walk(db, a, params, handled, out, depth);
                }
            }
            Resolved::Handle { init, arms, body } => {
                walk(db, init, params, handled, out, depth);
                for arm in arms.iter() {
                    walk(db, arm.body, params, handled, out, depth);
                }
                let added: Vec<u32> = arms
                    .iter()
                    .filter_map(|a| crate::eval::effect_op_of(db, a.op).map(|(d, _)| d.0))
                    .collect();
                let before = handled.len();
                handled.extend(&added);
                walk(db, body, params, handled, out, depth);
                handled.truncate(before);
            }
            Resolved::Host { effects, body } => {
                let added: Vec<u32> = effects
                    .iter()
                    .filter_map(|&e| host_effect_decl(db, e))
                    .collect();
                let before = handled.len();
                handled.extend(&added);
                walk(db, body, params, handled, out, depth);
                handled.truncate(before);
            }
            _ => {
                if let Struct::List(children) = db.ast.get(node).clone() {
                    for c in children {
                        walk(db, c, params, handled, out, depth);
                    }
                }
            }
        }
    }
    let mut handled: Vec<u32> = Vec::new();
    // Seed at the INTER-PROCEDURAL `depth`, not 0: the inner `walk` holds `depth` constant across ONE body
    // (a finite arena walk that always terminates) and the transitive follow re-enters at `depth + 1`, so
    // seeding here at the accumulated depth is what lets the `depth < 32` follow-gate actually fire on a
    // graph-invisible cycle — otherwise every re-entry restarts at 0 and the chase never terminates.
    walk(db, callee_body, &params, &mut handled, &mut out, depth);
    out
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
            // APPLY-SITE HOMING for a LAMBDA argument: a `(fn … (E.op))` passed as a fn-param is homed
            // where the CALLEE APPLIES that param, not at its definition site here. If the callee applies
            // param `i` under a handler discharging `E` — `with-seed(body) = handle E … (body unit)` — then
            // the lambda's perform IS homed, and walking its body under THIS call's (E-less) handled set
            // would falsely report CDZ0401. So, before the recursion, compute for the known callee the
            // EXTRA effects each param is applied under (`param_apply_extra_handled`); a lambda arg is then
            // walked below with those effects ADDED to the handled set. A bare `(body unit)` callee (no
            // handler — `apply-fn`) adds nothing, so a genuinely-ungranted effect in the lambda STAYS
            // reported (soundness). CONSERVATIVE: only a KNOWN, non-recursive callee contributes extra
            // grants; an opaque/param/recursive head adds nothing (the lambda is walked under the plain set,
            // matching the pre-fix behaviour). (Root-caused from v-cad's passed-closure-under-handler bug.)
            let callee = crate::eval::lambda_body(db, head)
                .or_else(|| crate::eval::lambda_body_of_nullary(db, head))
                .filter(|&c| !crate::eval::is_recursive(db, c));
            let param_extra: Vec<Vec<u32>> = match callee {
                Some(callee_body) => {
                    param_apply_extra_handled(db, head, callee_body, args.len(), depth)
                }
                None => Vec::new(),
            };
            if let Some(callee_body) = callee
                && followed.insert((callee_body, handled_key(handled)))
            {
                check_no_home_walk(
                    db,
                    callee_body,
                    entrypoint,
                    handled,
                    followed,
                    out,
                    depth + 1,
                );
            }
            for (i, &a) in args.iter().enumerate() {
                // A lambda arg whose callee-param is applied under extra handlers: walk it under `handled`
                // PLUS those extra effect decls (pushed, then popped), so its perform is homed at the apply
                // site. A non-lambda arg, or a param with no extra grant, is walked under `handled` as before.
                let extra = param_extra.get(i).filter(|e| !e.is_empty());
                if let Some(extra) = extra
                    && matches!(resolved_of(db, a), Resolved::Lambda { .. })
                {
                    let before = handled.len();
                    handled.extend(extra.iter().copied());
                    check_no_home_walk(db, a, entrypoint, handled, followed, out, depth);
                    handled.truncate(before);
                } else {
                    check_no_home_walk(db, a, entrypoint, handled, followed, out, depth);
                }
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
            // silently-ignored no-op. Reject it CDZ0201, anchored at the name. WARNING: CONSERVATIVE — flags ONLY a
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
///
/// The row is computed PURELY FROM THE OPERATIONS THE BODY REACHES — no written annotation is consulted
/// or required; a program compiles with none, the inferred escaping row being the mandatory floor.
//= spec/capabilities/capabilities-and-effects.md#effect-row-annotation-is-opt-in
//# A program MUST compile without any effect-row annotation, the compiler inferring each function's effect row from the operations it reaches, so that the mandatory floor is the escaping row itself, not a written annotation.
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

/// Selectively RESOLVE-PIN every name occurrence in `node` whose resolution chain reaches one of `binders`
/// (an arm's state/param/cont binders), so a following `copy_pure`/`substitute_nodes` of the arm body SHARES
/// those occurrences (eval.rs's `resolved_subtrees` captured-free-variable share-path) with their memoized
/// resolution intact — the ref keeps reaching its arm binder even in the detached rebuilt tree. Deliberately
/// does NOT pin a ref to a BODY-LOCAL binder (a `let`/`do`-def inside `node`): that must re-resolve against
/// the rebuilt tree's own (possibly rewritten) local init. Fills `db.resolved` (via `resolved_of`) AND marks
/// the node in `db.resolved_subtrees` — both are required for the share-path to fire. Bounded to `node`.
fn pin_refs_to_binders(db: &mut Db, node: StructId, binders: &[StructId]) {
    // A bare name whose resolution reaches one of `binders` → pin it (memo + walk-guard membership).
    if db.ast.as_name(node).is_some() {
        let reaches = match resolved_of(db, node) {
            Resolved::Param { binder } => binders.contains(&binder),
            Resolved::Ref { value } => {
                let mut t = value;
                loop {
                    if binders.contains(&t) {
                        break true;
                    }
                    match resolved_of(db, t) {
                        Resolved::Ref { value: n } => t = n,
                        _ => break false,
                    }
                }
            }
            _ => false,
        };
        if reaches {
            // `resolved_of` above already filled `db.resolved`; add the walk-guard membership so
            // `beta_reduce`'s share-path (eval.rs) returns this pinned occurrence as-is.
            db.resolved_subtrees.insert(node);
        }
        return;
    }
    if let Struct::List(children) = db.ast.get(node).clone() {
        for c in children {
            pin_refs_to_binders(db, c, binders);
        }
    }
}

/// E5 STEP 2 — WITHIN-ACTIVATION lexical `k`. A `ctl`-style arm binds the continuation `k` (`arm.cont =
/// Some(k)`) and applies it as `(k v)`. When `k` is used ONLY as the HEAD of applications — never bare,
/// never passed as an argument, never stored — the continuation does NOT escape: each `(k v)` returns into
/// the delimited context lexically, exactly as `(resume v)` does. So such an arm is semantically an
/// ORDINARY (non-tail) resumptive arm, and the existing pure-one-hole / two-hole `resume` folds serve it
/// with NO new machinery (no `Ty::Cont` heap rep, no defunctionalized frames — those are step 3, for an
/// ESCAPING `k`). This returns a rewritten arm body with every `(k v)` replaced by `(resume v <state>)`
/// (the state binder threads unchanged), or `None` if `k` ESCAPES (any non-application-head occurrence) —
/// in which case the caller declines cleanly, deferring to step 3.
///
/// Soundness: `(k v)` = the delimited continuation applied to `v`, which is precisely the meaning of
/// `(resume v)` in a handler arm; the state binder is threaded identically. `k` used strictly as an
/// application head cannot be captured/stored, so the continuation is single-use-in-extent (the folds
/// already handle one-shot AND the pure multi-shot case). A `k` that appears anywhere else (bare, an arg,
/// a let-init) MIGHT escape the handle's dynamic extent, which the lexical folds cannot represent → decline.
fn ctl_arm_lexical_k_to_resume(db: &mut Db, arm: &HandleArm) -> Option<StructId> {
    let k_binder = arm.cont?;
    // Confirm every `k` reference in the body is the HEAD of an `(k arg…)` application, and collect those
    // application nodes to rewrite. A reference resolves to `Ref { value }` whose chain reaches `k_binder`
    // (or a `Param { binder: k_binder }`). Walk the body; at each `Apply`, its HEAD may be a `k` ref (good
    // — record it); any OTHER position resolving to `k` is an escape.
    fn refs_to_k(db: &mut Db, node: StructId, k_binder: StructId) -> bool {
        match resolved_of(db, node) {
            Resolved::Param { binder } => binder == k_binder,
            Resolved::Ref { value } => {
                let mut t = value;
                loop {
                    if t == k_binder {
                        break true;
                    }
                    match resolved_of(db, t) {
                        Resolved::Ref { value: n } => t = n,
                        _ => break false,
                    }
                }
            }
            _ => false,
        }
    }
    // Gather the `(k arg)` application nodes (head resolves to k). Any k-ref NOT in such a head → escape.
    fn collect(db: &mut Db, node: StructId, k_binder: StructId, apps: &mut Vec<StructId>) -> bool {
        // A `k`-headed application: record it, and check its ARGS do not themselves escape `k` (a nested
        // `(k (k v))` — the inner is an arg, an escape; conservatively that fails the arg walk below).
        if let Resolved::Apply { head, args } = resolved_of(db, node)
            && refs_to_k(db, head, k_binder)
        {
            apps.push(node);
            // The args must be k-free (a `k` passed as an argument escapes). Walk each arg for any k-ref.
            return args.iter().all(|&a| !contains_k_ref(db, a, k_binder));
        }
        // Not a k-headed app: this node itself must not BE a k-ref (bare `k` — an escape), and recurse.
        if refs_to_k(db, node, k_binder) {
            return false; // bare `k` in a non-head position — escapes
        }
        match db.ast.get(node).clone() {
            crate::ast::Struct::List(children) => {
                children.iter().all(|&c| collect(db, c, k_binder, apps))
            }
            crate::ast::Struct::Atom(_) => true,
        }
    }
    fn contains_k_ref(db: &mut Db, node: StructId, k_binder: StructId) -> bool {
        if refs_to_k(db, node, k_binder) {
            return true;
        }
        match db.ast.get(node).clone() {
            crate::ast::Struct::List(children) => {
                children.iter().any(|&c| contains_k_ref(db, c, k_binder))
            }
            crate::ast::Struct::Atom(_) => false,
        }
    }
    let mut apps: Vec<StructId> = Vec::new();
    if !collect(db, arm.body, k_binder, &mut apps) {
        return None; // `k` escapes (bare / passed as an arg / stored) — defer to step 3
    }
    if apps.is_empty() {
        // `k` is bound but NEVER referenced (collect found no escape AND no application). The binder is
        // vacuous — the arm resumes via its own `resume` in the body (a `ctl`-form arm written with an
        // unused `k`, as the DES `sleep` distillation does: `(sleep (d) s k (let ((wake …)) (resume unit
        // wake)))`). Treat it as an ordinary arm: return the body UNCHANGED (the caller drops `cont`), so
        // the tail-resume / thread path serves it exactly as the same arm without the `k` binder would be.
        return Some(arm.body);
    }
    // Rewrite: each `(k arg)` → `(resume arg <state>)`. `k` is single-arg (Cont takes one resume value);
    // a different arity is not the lexical-resume shape. Build fresh `resume` nodes; the state binder is
    // referenced by a fresh name occurrence of the arm's state name (copied structurally, resolves to the
    // arm's state binder via `handle_arm_binds`).
    let mut sub: HashMap<StructId, StructId> = HashMap::default();
    for &app in &apps {
        let Resolved::Apply { args, .. } = resolved_of(db, app) else {
            continue;
        };
        if args.len() != 1 {
            return None; // `(k)` or `(k a b)` — not a single-value resume
        }
        let resume_head = db.push_name("resume");
        let state_ref = copy_pure(db, arm.state);
        let resume = db.push_list(vec![resume_head, args[0], state_ref]);
        sub.insert(app, resume);
    }
    // Splice the rewritten resume nodes in place of the `(k v)` applications (a node→node substitution).
    // SELECTIVELY PIN the arm body's ARM-BINDER references before `substitute_nodes`'s `copy_pure` re-pushes
    // every non-`(k v)` atom into the DETACHED rebuilt tree (root parent `None`). Two classes of name ref
    // need OPPOSITE treatment:
    //   • A ref to an ARM binder — the state binder `s`, an op-param `x` — resolves OUTSIDE the arm body
    //     (up the parent chain to `handle_arm_binds`). In the detached copy that parent walk is gone, so the
    //     ref must keep its pinned resolution (else the pure-one-hole reify's `beta_reduce({state↦init,
    //     params↦args})` can't match it and it leaks `unbound name s` — the bare-position `(+ s (k x))`
    //     leak, breaker ek1).
    //   • A ref to a BODY-LOCAL binder — a `let`/`do`-def `r` in `(let ((r (k x))) (+ r s))` — resolves
    //     INSIDE the arm body (down to the local `(def r …)`). It must NOT be pinned: after the rewrite the
    //     init changes `(k x)`→`(resume x s)`, and a pinned `r` would keep pointing at the ORIGINAL, now-
    //     orphaned `(k x)` init (a bare k-application) → "value is not applyable" (breaker ek5). Left
    //     unpinned, `r` re-resolves within the rebuilt tree to the REWRITTEN `(def r (resume x s))` init —
    //     correct (the explicit-resume twin `(let ((r (resume x s))) (+ r s))` already folds this way).
    // A blunt `resolve_subtree` pins BOTH (breaks ek5); a blunt `forget` pins NEITHER (re-leaks ek1). So
    // pin ONLY refs whose resolution chain reaches one of THIS arm's binders. Idempotent + bounded.
    let mut arm_binders: Vec<StructId> = vec![arm.state];
    arm_binders.extend(arm.params.iter().copied());
    arm_binders.push(k_binder);
    pin_refs_to_binders(db, arm.body, &arm_binders);
    Some(substitute_nodes(db, arm.body, &sub))
}

/// [cp4] CTX-FREE creation-wrapper capture-closure detector — whether `node` is a `(let (binds)
/// <returns-lambda>)` where SOME binding's init reaches an EFFECT-OP application AND the returned lambda
/// references that binder's NAME (the draw happens at closure CREATION, outside the returned lambda, and is
/// captured). CTX-FREE (any op, run before the discharged set is known). Excludes cx8 (perform INSIDE the
/// returned lambda → its body is a bare `fn`, not a `let` → false) and a list-returning factory (case-5 →
/// `body_returns_lambda` false). Mirrors `init_is_performing_capture_closure`'s direct-`let` case.
fn inlined_body_is_performing_capture_creation_wrapper(db: &mut Db, node: StructId) -> bool {
    let Some(form) = db.ast.as_form(node, "let").map(|t| t.to_vec()) else {
        return false;
    };
    if form.len() != 2 || !body_returns_lambda(db, form[1]) {
        return false;
    }
    let Struct::List(pairs) = db.ast.get(form[0]).clone() else {
        return false;
    };
    fn reaches_effect_op(db: &mut Db, n: StructId) -> bool {
        if crate::eval::effect_op_of(db, n).is_some() {
            return true;
        }
        if let Resolved::Apply { head, .. } = resolved_of(db, n)
            && crate::eval::effect_op_of(db, head).is_some()
        {
            return true;
        }
        match db.ast.get(n).clone() {
            Struct::List(ch) => ch.iter().any(|&c| reaches_effect_op(db, c)),
            Struct::Atom(_) => false,
        }
    }
    fn refs_name(db: &Db, n: StructId, name: &str) -> bool {
        if db.ast.as_name(n) == Some(name) {
            return true;
        }
        match db.ast.get(n) {
            Struct::List(ch) => ch.iter().any(|&c| refs_name(db, c, name)),
            Struct::Atom(_) => false,
        }
    }
    pairs.iter().any(|&pair| match db.ast.get(pair).clone() {
        Struct::List(kv) if kv.len() == 2 => {
            let performs = reaches_effect_op(db, kv[1]);
            let refs = db
                .ast
                .as_name(kv[0])
                .map(str::to_string)
                .is_some_and(|nm| refs_name(db, form[1], &nm));
            performs && refs
        }
        _ => false,
    })
}

/// [cp4 CAPTURE-ONCE FOLD, v-inference×v-effects pair] When the handle body binds a MULTI-USE `let`-local to
/// a NULLARY factory-call `(mk)` whose VERBATIM body is a performing-capture creation-wrapper closure
/// `(let ((a <perform>)) (fn …a…))`, rebind the local to a fresh COPY of that verbatim body — PRESERVING the
/// capture-let, turning the body into the DIRECT performing-closure-let (ca1m) shape the #3894 capture-once
/// hoist already folds to the capture-once value (cp4 → 150). Without this, reducing `(f X)` inlines `(mk)`
/// per use via `apply_lambda`/`lambda_of`, whose `Let`-capture arm SUBSTITUTES the performing init
/// `(St.next)` INTO the returned lambda body (creation-time capture → per-application perform), so a local
/// used ≥2× re-draws per use (breaker cp4: silent 170 instead of 150). Binding the VERBATIM body (not the
/// `apply_lambda`-reduced one, which collapses the capture) + the #4006 `deep_fresh_copy` re-anchor (below)
/// keeps the capture. NARROW: multi-use only (a single application draws once = correct, cpf1 folds 50),
/// NULLARY factory only (the arg-factory face cc3 is v-effects' `reduce_handle` layer, folded by #4006),
/// gated on the creation-wrapper shape (excludes cx8 [perform inside the lambda, folds 24] and pure
/// factories idc1/idc2 [callee returns non-lambda / no perform]).
fn bind_once_performing_factory(db: &mut Db, body: StructId) -> StructId {
    let Some(form) = db.ast.as_form(body, "let").map(|t| t.to_vec()) else {
        return body;
    };
    if form.len() != 2 {
        return body;
    }
    let (bindings_occ, inner) = (form[0], form[1]);
    let Struct::List(pairs) = db.ast.get(bindings_occ).clone() else {
        return body;
    };
    let mut changed = false;
    let mut new_pairs: Vec<StructId> = Vec::with_capacity(pairs.len());
    for pair in pairs.iter().copied() {
        let rebuilt = (|| -> Option<StructId> {
            let Struct::List(kv) = db.ast.get(pair).clone() else {
                return None;
            };
            if kv.len() != 2 || count_param_refs(db, inner, kv[1]) < 2 {
                return None; // multi-use only (a single application draws once — cpf1 folds 50)
            }
            let Struct::List(items) = db.ast.get(kv[1]).clone() else {
                return None;
            };
            if items.len() != 1 {
                return None; // NULLARY factory call `(mk)` only; the arg-factory (cc3) is v-effects' layer
            }
            let head = items[0];
            let mk_body = crate::eval::lambda_body_of_nullary(db, head)?;
            if !inlined_body_is_performing_capture_creation_wrapper(db, mk_body) {
                return None;
            }
            let empty: HashMap<StructId, StructId> = HashMap::default();
            let copy = crate::eval::copy_structural_pub(db, mk_body, &[], &empty);
            Some(db.push_list(vec![kv[0], copy]))
        })();
        match rebuilt {
            Some(np) => {
                new_pairs.push(np);
                changed = true;
            }
            None => new_pairs.push(pair),
        }
    }
    if !changed {
        return body;
    }
    let new_bindings = db.push_list(new_pairs);
    let let_head = db.push_name("let");
    let new_body = db.push_list(vec![let_head, new_bindings, inner]);
    // [#4006 recipe] The reused `inner` SHARES load-time atoms whose single parent slot points into the
    // pre-rewrite tree, so a plain `resolve_subtree` leaves `(f …)` refs anchored to the OLD `(mk)` binding
    // (folds 200) and a blunt `forget_subtree` over-forgets the fresh copy's own op resolution (spurious
    // decline). `deep_fresh_copy` gives every node a fresh id with `push_list`-set parents → coherent scope
    // chains; `forget_subtree` + `force_structural_resolution_subtree` then re-resolve every reference
    // against the rebuilt binding (the copied capture closure), yielding the DIRECT performing-closure-let
    // (ca1m) shape the #3894 capture-once hoist folds to the capture-once value (cp4 → 150).
    let fresh = deep_fresh_copy(db, new_body);
    crate::resolve::forget_subtree(db, fresh);
    db.force_structural_resolution_subtree(fresh);
    fresh
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
    // [cp4] Bind-once a multi-use nullary performing-factory let-local to the VERBATIM factory body (the
    // preserved capture-let = ca1m shape #3894 folds to 150), before the fold's per-use inline collapses
    // the creation-time capture into a per-application perform (silent 170). No-op unless the exact narrow
    // shape is present (see `bind_once_performing_factory`).
    let body = bind_once_performing_factory(db, body);
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
        // E5 GENERAL / `ctl`-style arm: an arm that binds the continuation `k` explicitly (`cont: Some`).
        // STEP 2 (within-activation, lexical `k`): if `k` is used ONLY as an application head `(k v)` — never
        // bare, stored, or passed as an arg — the continuation does not escape, so `(k v)` is semantically
        // `(resume v)`; `ctl_arm_lexical_k_to_resume` rewrites the arm body accordingly, turning it into an
        // ordinary non-tail resumptive arm the existing pure-one-hole / two-hole folds serve (no new heap
        // rep). STEP 3 (deferred): a `k` that ESCAPES (bare / stored in a list-map / passed onward — the DES
        // scheduler's sleep/store/resume-later) needs the defunctionalized-frame `Ty::Cont` machinery; such
        // an arm still DECLINES cleanly here (a Todo, never a valid-but-wrong fold). Classified distinctly
        // from tail-resumptive (has `resume`, no `k`) and abortive (no `resume`, no `k`).
        let arm = if arm.cont.is_some() {
            match ctl_arm_lexical_k_to_resume(db, arm) {
                Some(rewritten_body) => HandleArm {
                    op: arm.op,
                    params: arm.params.clone(),
                    state: arm.state,
                    cont: None,
                    body: rewritten_body,
                },
                // `k` ESCAPES (passed onward / stored) — carry the arm through WITH `cont: Some` so the
                // pure-one-hole block below can REIFY `k` as a closure `(fn (#kv) C)` when `C` is pure
                // (step-3 inc-2a). If that block does not serve it, the fold declines at the end (never a
                // mis-fold — the pure-one-hole block is the ONLY consumer of a `cont: Some` arm).
                None => arm.clone(),
            }
        } else {
            arm.clone()
        };
        // FINDING-24 COVERAGE-GAP (sft1): canonicalize a single-bare-name-binder match over a COMPOUND
        // scrutinee in the arm body — `(match <compound> (h2 <body>))` → `(let ((h2 <compound>)) <body>)` —
        // so the resumptive fold binds the scrutinee ONCE per dispatch (the existing `#st` state-bind path)
        // instead of copying the compound scrutinee into every continuation copy (super-linear emit → invalid
        // wasm). A pure syntactic identity; see `hoist_single_binder_match_scrutinee`.
        let arm = match hoist_single_binder_match_scrutinee(db, arm.body) {
            Some(hoisted) => HandleArm {
                op: arm.op,
                params: arm.params.clone(),
                state: arm.state,
                cont: arm.cont,
                body: hoisted,
            },
            None => arm,
        };
        map.insert((decl.0, idx), arm);
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
    // CROSS-ARM COLLECTION-KEY PROPAGATION (v-inference, breaker tuple-key-map-rust-e0308 2026-08-09). The
    // initial-state seed's `type_of` bottoms an empty collection at OPEN vars — `Map.empty` is `Map(Var,Var)`,
    // the enclosing `(tuple n Map.empty)` `Tuple([Int64, Map(Var,Var)])`. A handler arm fixes the collection's
    // key/value/element (the `rec` arm's `(Map.insert m (tuple s …) …)` pins the key to `(Int64,Int64)`), and
    // `state_ty_of_arms` above JOINS that onto the slot type — but the join result is never written back onto
    // the INIT node's own type memo, and `beta_reduce` substitutes the init node (its id, reused not copied)
    // into the arm bodies, so emit reads `type_of(init map node) = Map(Var,Var)` and grounds it to the default
    // `BTreeMap<i64,i64>`. When the real key is a `(i64,i64)` tuple that is rust/rust-async E0308 (`__m.insert`
    // of a tuple key into an `<i64,i64>` map); wasm's tagless heap needs no spelled key type, so it is a
    // wasm-masked backend divergence, not a decline. Propagate the SOLVED collection type from the joined slot
    // back onto the init subtree's open-var collection nodes so `type_of(init)` reflects the real key/value.
    refine_init_collection_ty(db, init, arms);
    // WIDTH-CONSISTENCY GUARD (F1, corpus-bugfix/breaker 2026-07-28; refined for pyu8t1/pyu8r1 + the 14b
    // `two do-def-bound performs` case). A tail resume value is substituted into the op's RESULT position;
    // if its EMITTED width is narrower than the op result width, emit puts an i32 where i64 is demanded and
    // the module is INVALID ("expected i64, found i32"; rust widens and runs — a backend divergence). The
    // canonical hazard is a bare `(next (u) s (resume s (+ s x)))` with a UInt8 `x`: `s` infers UInt8 (i32)
    // via the next-state, the op result is Int64 (i64), so the state read is emitted i32 into an i64 slot.
    // DECLINE cleanly (a "not yet reducible" todo) rather than emit invalid wasm — the safe floor; a
    // widening-coercion fold is a later increment. See the emit-width computation below for the exact test.
    for arm in arms {
        if let Some(rv) = tail_resume_value_of(db, arm.body)
            && let Some(crate::ty::Ty::Int(rt)) = op_result_type(db, arm.op)
            && let crate::ty::Width::Fixed(rw) = rt.width
        {
            // The width the resume value is EMITTED at, compared to the op result width it is
            // substituted into: a narrower emit puts an i32 where i64 is demanded → invalid wasm.
            // The emit width is the value's OWN concrete fixed-int width when it has one — an
            // explicitly-widened answer `(resume (Int64.of s) …)` is Int64 = matches the result,
            // SAFE (pyu8t1); a narrow op-result whose answer is narrow-typed matches too (pyu8r1).
            // When the value type is NOT a concrete fixed int (an `Any`/deferred-width bare
            // `(resume s …)` state read whose type is not yet pinned), fall back to the STATE slot
            // width — the value IS the state, emitted at slot width. This supersedes the original
            // state-width-only test (which OVER-declined a widened narrow-state thread) and the
            // undetermined-value gate (which UNDER-declined a bare state read whose type is refined
            // to the concrete narrow state type BY THIS POINT → the 14b `two do-def-bound performs`
            // case regressed to an invalid module). Fires only on a determined width MISMATCH; a
            // matching width, a non-int result, or no width info on either side is untouched.
            let emit_w = match crate::infer::type_of(db, rv) {
                crate::ty::Ty::Int(vt) if matches!(vt.width, crate::ty::Width::Fixed(_)) => {
                    vt.width
                }
                _ => match &state_ty {
                    Some(crate::ty::Ty::Int(st)) => st.width,
                    _ => continue,
                },
            };
            if let crate::ty::Width::Fixed(ew) = emit_w
                && ew != rw
            {
                return None;
            }
        }
    }
    let slot = StateSlot { decl, state_ty };
    let ctx = HandlerCtx::new(db, map, vec![slot.clone()]);
    // DES multi-task reach: expose a DEFERRED resume that is stored in a compound and applied through a
    // helper — `(sleep (wake) s (unbox-apply (Box.Box (fn (_u) (resume unit wake)))))` where `unbox-apply(b)
    // = match b ((Box.Box th) (th unit))` — by reducing each arm body's store→match-extract→apply round-trip
    // (β-reduce + `eval::fold_ctor_match`, via `reduce_arm_deferred_resume`) to the resume-in-place form the
    // fold serves. Only an arm whose reduction EXPOSES a tail resume is rewritten; every other arm reduces to
    // itself (`==`), so tail/abortive/already-foldable arms are byte-identical. Rebuild the ctx only when some
    // arm changed. (v-discrete-event-sim inc-4's pqueue store→pop→apply reach; co-built with v-inference, who
    // owns `fold_ctor_match`'s SumPayload-aware substitution.)
    let exposed: HashMap<(u32, u32), HandleArm> = ctx
        .arms
        .iter()
        .map(|(&k, a)| {
            // ONLY a plain 4-part arm (no explicit continuation binder `k`): an escaping-k arm (`cont:
            // Some`, e.g. b2-min's `(a () s k (use-k k))`) is served by the escaping-k reify below, which
            // needs its body UN-reduced — β-reducing `(use-k k)` here to `(k 10)` breaks that reify ("value
            // is not applyable"). The deferred-resume-thunk shape this pass targets is a 4-part arm whose
            // resume is buried in a compound-stored closure, never a `cont: Some` arm.
            let rb = if a.cont.is_none() {
                reduce_arm_deferred_resume(db, a.body, &ctx)
            } else {
                a.body
            };
            let arm = if rb == a.body {
                a.clone()
            } else {
                HandleArm {
                    op: a.op,
                    params: a.params.clone(),
                    state: a.state,
                    cont: a.cont,
                    body: rb,
                }
            };
            (k, arm)
        })
        .collect();
    let ctx = if exposed
        .iter()
        .any(|(k, a)| ctx.arms.get(k).is_none_or(|o| o.body != a.body))
    {
        HandlerCtx::new(db, exposed, vec![slot])
    } else {
        ctx
    };
    // [pyfb3] Now that the handle BODY is available, compute this handler's per-op multi-dispatch set and
    // RE-EVALUATE collapse_enabled. The nullary-foreign-perform-let collapse candidate (pyfb3/pyfb1-let) fires
    // ONLY for an op drawn >=2 times in the body — a SINGLE dispatch folds the same shape strict via distribute
    // (as7) with no heap slot, so collapsing it there would re-land the 07e85af7c heap-collapse regression.
    // `HandlerCtx::new` decided collapse_enabled with an empty multi-set (body unavailable); redo it here.
    {
        let multi = ops_drawn_ge2(db, body, &ctx.arms);
        *ctx.multi_dispatch_ops.borrow_mut() = multi;
        let mdo = ctx.multi_dispatch_ops.borrow().clone();
        let ce = collapse_enabled_for(db, &ctx.arms, &mdo);
        ctx.collapse_enabled.set(ce);
    }
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
    // DO-LOCAL VALUE-DEF → LET normalization. A handle body `(do (def v e) … rest)` binds `v` LOCAL to the
    // `do`. Several downstream folds (the abortive one-hole splice, the tail-resume thread's collapse-to-last)
    // DROP non-final do items and re-splice only the surviving expression — orphaning any `(def v e)` whose
    // `v` a LATER item references (notably a perform's ARGUMENT, `(Bail.bail v)` / `(Ask.ask v)`), which then
    // reads UNBOUND → spurious CDZ0101 (the do-def-in-perform-argument false-reject, corpus-bugfix 2026-07-24;
    // the `let`-twin never hit it because `let` rebuilds its scope). Rewrite a LEADING CHAIN of value defs
    // to nested `let`s wrapping the continuation up front — `(do (def v e) rest…)` ≡ `(let ((v e)) (do
    // rest…))`, recursively — so EVERY consumer below (abortive/pure-hole/thread) sees the scoped form.
    // Sound: `e` is evaluated once, in the SAME position and order it held in the `do` (its RHS may itself
    // perform — the `let` init runs exactly where the `do` item did); only `v`'s visibility widens to the
    // continuation. A FUNCTION def `(def (f p…) body)` (sig is a list, not a bare name) is left untouched —
    // it resolves to a lambda. (Rewrites only the LEADING chain, not every non-final def: a def AFTER a
    // non-def item is reached by re-threading the `do` tail, which normalizes it in turn.)
    // CAPTURE-ONCE normalization (finding #10 increment). A let-bound closure whose value is a
    // `(let ((a <perform>)…) LAMBDA)` — the capture `a` is a performing draw the returned lambda reads —
    // must discharge that draw ONCE and share it across every application; but a downstream β-reduction of
    // the closure's uses inlines `a`'s init at each use site, DUPLICATING the perform (multi-app ca1 read a
    // fresh draw per use). HOIST the performing init OUT of the closure-value-let to WRAP the binding here,
    // on the raw body BEFORE any use is reduced: `(let (… (f (let ((a P)…) LAMBDA)) …) BODY)` becomes
    // `(let ((a P)…) (let (… (f LAMBDA) …) BODY))`. Now `a` is a plain outer let-init the fold threads ONCE,
    // and every `(f v)` β-reduces against the PURE lambda closing over that single-threaded `a` (verified:
    // the hoisted single- and multi-app forms fold, sharing the one draw). Sound: the init runs once, before
    // the body, same order — only the binder's visibility widens. Fixpoint-loop so several such closures all
    // hoist; each hoist removes its own match, so it terminates.
    let mut body = body;
    while let Some(hoisted) = hoist_performing_capture_closure(db, body, &ctx) {
        // The hoist REUSES sub-nodes (the returned lambda, the let body) and re-parents them under the new
        // wrapping `let`s. A REUSED subtree can SHARE a load-time atom (e.g. a name occurrence) whose SINGLE
        // parent slot points into a stale/orphaned prior copy — so the exhaustive lexical-scope walk from such
        // a ref DEAD-ENDS before reaching the new binder → a false CDZ0101 "unbound" that no memo hygiene
        // (`forget`/`force`) can repair, because the parent CHAIN itself is broken (not just the memo). So
        // `deep_fresh_copy` the whole rewritten subtree: every node becomes a fresh synth id whose parent is
        // set correctly by `push_list`, giving a coherent parent chain for the exhaustive walk. Then
        // `forget_subtree` clears any stale memo and `force_structural_resolution_subtree` routes the
        // (now-synth) refs through the exhaustive walk against the CURRENT parents. This resolves a
        // nested-closure capture (cp1: `h` wrapping the hoisted `g`) correctly — no re-draw, no unbound.
        let hoisted = deep_fresh_copy(db, hoisted);
        crate::resolve::forget_subtree(db, hoisted);
        db.force_structural_resolution_subtree(hoisted);
        body = hoisted;
    }
    let body = lift_do_local_value_defs(db, body);
    // CAPTURE-AVOIDING HYGIENE (body side). Alpha-rename the handle body's LOCAL value binders (`let` pairs,
    // `do`-local `(def NAME v)`) to fresh `#`-names before the fold threads a resume VALUE into the perform
    // position inside this body. Without it, a body-local `(let ((x 7)) (+ x (E.get)))` (or its `do`-def
    // twin) CAPTURES a spliced-in free `x` carried from the arm — the arm's `(resume x s)` reads the OUTER
    // global x=100, but landing inside the body's `x=7` scope rebinds it (F2 → 14 not 107). `freshen_local_
    // binders` SHARES every free-name subtree (returns the node untouched when no local binder is inside), so
    // an enclosing-param / global reference the body legitimately reads keeps its pinned resolution — only
    // local binders and their in-scope references are rewritten. (breaker's silent-miscompile finding,
    // corpus-bugfix 2026-07-28.)
    let body = freshen_local_binders(db, body);
    // ABORTIVE-OUTER + NESTED-HANDLE PRE-REDUCTION (finding #11). When THIS handler is abortive and its body
    // contains a NESTED inner handle of a DIFFERENT effect that itself wraps a perform of THIS handler's op —
    // `(handle A abortive (+ (* 100 (handle B tail (if c (A.out n) n))) 7))` — the `A.out` is buried under
    // BOTH the inner `handle-internal B` node AND (here) a conditional. `hoist_conditional_abort` below and
    // the abort-reach guards do NOT descend into a nested handle node, so the buried `A.out` is invisible:
    // the inner B-handle later reduces during A's `thread` (the inside-out Handle arm), surfacing `(if c
    // (A.out n) n)` — but by then the hoist has already run, so the conditional abort is captured BRANCH-LOCAL
    // (yielding 9003 as the `if`'s value) and the enclosing `* 100`/`+ 7` continue → 900307, a silent
    // miscompile (finding #11: conditional foreign abort homes to the wrong inner boundary). REDUCE the inner
    // handles FIRST — B keeps `A.out` residual (foreign to B, `copy_pure`'d) and its wrapper is removed — so
    // the body becomes `(+ (* 100 (if c (A.out n) n)) 7)`, where `hoist_conditional_abort` CAN lift the
    // conditional abort to a branch tail A discharges. The UNCONDITIONAL case (abmin) already worked (the
    // inside-out thread reduces B then discharges the bare `A.out`); this makes the CONDITIONAL case take the
    // same homing path. GATED to abortive + a genuinely nested handle so the common body is untouched; sound
    // because reducing an inner handle is the same reduction the thread path performs, only sequenced earlier.
    let body = if !ctx.abortive.is_empty() && body_contains_nested_handle(db, body) {
        reduce_inner_handles(db, body)
    } else {
        body
    };
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
    // RE-HOIST after inlining. `reduce_applied_lambdas` above β-reduces a performing helper CALL into its
    // body — surfacing a conditional that was HIDDEN behind the call (`(let ((a (demand 5 25))) cont)` →
    // `(let ((a (match (Db.get k) … (do (Db.put …) …)))) cont)`). The first `hoist_resumptive_conditional`
    // ran BEFORE that inline, so it saw only the opaque `(demand …)` call and could not lift the branch-
    // performing conditional now exposed in the `let` init. Without re-hoisting, the tail fold threads the
    // surfaced conditional's out-state as its post-scrutinee state — the branch's `put` advance is dropped
    // and a later `(Db.get k)` in the continuation reads the stale pre-branch state (the helper-call
    // out-state silent miscompile). Re-running the hoist lifts the exposed conditional to tail position (via
    // Site 4, the `let`-init distribution) where per-arm threading carries the advance through the
    // continuation. Idempotent + cheap: a no-op (returns `body` unchanged) when the inline surfaced no
    // branch-performing conditional, so a body with no such helper is untouched.
    let body = hoist_resumptive_conditional(db, body, &ctx);
    // adv-69 SAFE-DECLINE FLOOR (HIGH silent miscompile, breaker + corpus-bugfix 2026-08-04). The hoist above
    // lifts a branch-performing conditional that is DIRECTLY a `let`-init to tail position (Site 4), where
    // per-branch threading carries its state advance. But a conditional wrapped in a BLOCK (`(let ((v (let
    // ((b true)) (if b (E.op) x)))) cont)` — inner-let/do around the `if`/`match`) is opaque to Site 4: the
    // block's exit state reverts to block-ENTRY, so the branch perform's advance is DROPPED and a later
    // perform in `cont` resumes the stale pre-branch state → a WRONG VALUE, no error (worst class). Until the
    // full through-block distribution lands (a commuting conversion needing alpha-safe binder handling — a
    // separate careful increment), DECLINE this residual shape so it grades a clean Todo, never a silent
    // miscompile. Detects a `let`-init whose value is a block-wrapped branch-performing conditional the hoist
    // left in place. (The direct-init case the hoist DID lift no longer matches — it's now a tail conditional,
    // not a `let`-init block — so this never over-declines the working Site-4 path.)
    if body_has_block_wrapped_let_init_branch_perform(db, body, &ctx) {
        return None;
    }
    // adv-69 a3 sub-face: the SAME block-boundary out-state drop at a NESTED handle's arm RESUME-VALUE
    // position (performing THIS handler's op), which the let-init scan above does not reach (it stops at a
    // nested `Handle`). Decline cleanly rather than folding the dropped-advance wrong value (probe-a3 ran 33,
    // correct 34). Precisely keyed on `Resume{value}` so it never over-declines a threaded position the fold
    // serves (a position-agnostic block-wrapped-perform scan over-declines 5 working cases).
    if body_has_nested_arm_resume_value_block_wrapped_branch_perform(db, body, &ctx) {
        return None;
    }
    // adv-69 g3 + c3 sub-faces: the SAME block-boundary out-state drop at a MATCH-SCRUTINEE (g3) or a non-tail
    // `do`-STATEMENT (c3) consuming a block-wrapped branch perform — positions Site 5 / Site 1 lift only when
    // the conditional is DIRECT, not block-wrapped. Decline cleanly rather than folding the dropped-advance
    // wrong value (probe-g3 ran 33/34, probe-c3 ran 33/73). Keyed on the WRAPPED shape only, so a direct
    // conditional in either position (the passing d2/e1 twins) still folds.
    if body_has_block_wrapped_scrutinee_or_statement_branch_perform(db, body, &ctx) {
        return None;
    }
    // finding #10 (breaker, MED-HIGH silent miscompile — the bind-once/closure-capture face). A `let`-bound
    // CLOSURE whose value is `(let ((a <perform>)) (fn … a …))` — the capture `a` is a PERFORMING inner-let
    // init — re-performs that draw PER APPLICATION: the closure re-derives its body from source through
    // `apply_lambda`/`beta_reduce` at each `(f x)`, discarding the once-evaluated draw and re-running it
    // inside the body (ca1c: single app 60 not 50; ca1: two apps 122 not 80). The `capture_subst` closes only
    // a STRONGLY-PURE init into the body; a performing init is left a live reference the re-derivation
    // re-fires. Correctly folding it (thread the draw ONCE before the closure value, close the closure over
    // the RESULT) is the eval-count/state-commitment capture-once increment; until then DECLINE the exact
    // shape (a silent wrong value → an honest "not yet reducible" todo). NARROW by construction (see
    // `body_has_closure_over_performing_capture`): only a let-bound closure whose init-let binds a discharged/
    // foreign perform referenced by the returned lambda — the direct handle-body captures (corpus-8688) and
    // the let-outside-the-closure workaround (d1) bind the draw in a PLAIN let, not the closure's init-let, so
    // they are untouched; a PURE inner init (d2fix) is not a perform, so it folds.
    if body_has_closure_over_performing_capture(db, body, &ctx) {
        // A performing-capture closure the early capture-once normalization did NOT hoist (a shape neither
        // FORM A — a let-bound closure-value-let — nor FORM B — an arg'd factory call — matches; e.g. a
        // conditionally-selected closure). DECLINE cleanly rather than fold a wrong value; the closure would
        // re-derive its body per application, re-firing the captured draw. (The common let-bound + arg'd
        // factory faces ARE hoisted above and never reach here.)
        return None;
    }
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
        // CONDITIONALLY-RESUMING ARM GUARD (corpus-bugfix/breaker 2026-07-28): decline an arm that resumes in
        // some branches but aborts (returns a bare value) in others — the reify below would mis-fold it (see
        // `arm_partially_resumes`). A `cont: Some` escaping-k arm is exempt (reified as a closure, not peeled).
        && (arm.cont.is_some() || !arm_partially_resumes(db, arm.body))
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
        // E5 ESCAPING-K REIFICATION (step-3 inc-2a): a general `ctl`-style arm that BINDS `k` (`cont: Some`)
        // and lets it ESCAPE (passes it to a fn / stores it — `ctl_arm_lexical_k_to_resume` returned None,
        // so the classifier carried it here). Over a PURE continuation `C` (`pure_hole` succeeded), reify
        // `k` as the CLOSURE `(fn (#kv) C)` where `C = body[perform := #kv]` — a lambda taking the resume
        // value `#kv` and running the delimited continuation. Substituting that lambda for the arm's `k`
        // binder makes the arm body carry `k` as an ordinary closure value; the runtime-closure machinery
        // lifts it (`Core::Closure`) and applies it (`Core::CallClosure`) at each `(k v)` — so `(use-k k)`
        // over `(+ 1 (A.f))` folds to `(use-k (fn (#kv) (+ 1 #kv)))` → 11. NO bespoke frame chain: a reified
        // continuation over a pure `C` IS a closure. (A `C` that re-performs the handled effect needs
        // handler-re-entry-at-apply, inc-2b — `pure_hole` fails on the second perform, so this never fires
        // for it.) The `#kv` binder is fresh (keyed on the perform node); the lambda captures `C`'s free
        // names, which the closure lift resolves into its env.
        let folded = if let Some(k_binder) = arm.cont {
            let kv_name = format!("#kv{}", perform.0);
            let kv_binder = db.push_name(&kv_name);
            let kv_ref = db.push_name(&kv_name);
            let cont_body = splice_context(db, body, perform, kv_ref);
            let fn_head = db.push_name("fn");
            let params_list = db.push_list(vec![kv_binder]);
            let k_lambda = db.push_list(vec![fn_head, params_list, cont_body]);
            let mut subst_k = subst.clone();
            subst_k.insert(k_binder, k_lambda);
            crate::eval::beta_reduce(db, arm.body, &subst_k)
        } else {
            // Pin the ARM BODY's enclosing captures BEFORE beta_reduce detaches them (mv-class arm-side).
            // A two-site resume arm `(+ (resume (+ n 1) s) (resume 2 s))` whose resume VALUE reads an
            // enclosing-fn param `n`: beta_reduce COPIES the arm body per the subst, and the copied `n` loses
            // its binding (ref_binder None) so each per-resume splice re-resolves it unbound. Pinning `n`
            // here (still parented to the def) records its resolution so beta_reduce's capture-share arm
            // preserves it. `lam_body = arm.body`, so the arm's own op-params/state binder (bound within) stay
            // unpinned + substitute normally. (breaker mv-class, arm-side/resume-value locus.)
            crate::eval::pin_free_vars(db, arm.body, arm.body, &[]);
            // adv-20: if the arm body is a pure LET-WRAPPED resume — `(let ((o (if (> s 100) … s))) (resume
            // (match o …) s))` — FLATTEN it (splice each pure let-init for its binder refs) so a let-init that
            // reads the arm state does not survive the `{arm.state → init}` subst below UNSUBSTITUTED (→
            // slotless `Core::Param{arm.state}` decline). The flatten PINS the arm's state/op-param uses first
            // so the flatten copy SHARES them (keeps `Ref{arm.state}`) rather than orphaning a bare sibling `s`
            // in the resume value; the subst below then substitutes the shared occurrences. Non-let arms fall
            // through unchanged. (v-inference-pinned locus; Option A.)
            let arm_binders: Vec<StructId> = arm
                .params
                .iter()
                .copied()
                .chain(std::iter::once(arm.state))
                .collect();
            let arm_body =
                flatten_pure_let_wrapped_resume(db, arm.body, &arm_binders).unwrap_or(arm.body);
            let substituted = crate::eval::beta_reduce(db, arm_body, &subst);
            // Also pin the SEED-derived captures now in `substituted`: when `n` is the handle SEED
            // (`(handle Amb n …)`), it enters the arm via the state-binder subst, so it appears in
            // `substituted` (not `arm.body`); pin it here so the per-resume splice shares it too.
            crate::eval::pin_free_vars(db, substituted, substituted, &[]);
            // CAPTURE-AVOIDING HYGIENE. Freshen the substituted arm body's LOCAL value binders before the
            // resume rewrite splices the continuation `C` (the handle body, carrying OUTER free names) into
            // its tail. Without this, an arm-local `(do (def x 5) (resume (+ x s) s))` CAPTURES `C`'s free `x`
            // (F1 → 10 not 105). Done AFTER `beta_reduce` (so the arm's op-param/state binder references are
            // already substituted away — freshening rebuilds the tree, which would otherwise orphan those
            // node-identity-keyed refs). (breaker's silent-miscompile finding, corpus-bugfix 2026-07-28.)
            let substituted = freshen_local_binders(db, substituted);
            // PIN `C`'S ENCLOSING CAPTURES before the (possibly multi-)splice. A MULTI-SHOT arm
            // (`(pick (u) s k (+ (k 1) (k 2)))`) has `k` applied ≥2×, so `rewrite_resume_to_context`
            // splices a FRESH copy of `C` per resume. When `C` (the handle body) reads an ENCLOSING-fn
            // param — `(let ((y 3)) (+ n (Amb.pick)))` free in `n` — each `splice_context` copy would
            // re-resolve `n` against its own orphan and report a false CDZ0101 "unbound n" (the reparent
            // at the fold tail only re-anchors the single folded root, not both C-copies). Pinning `C`'s
            // free captures here records their resolution so the per-resume copies SHARE the resolved
            // `n` — exactly the `apply_lambda_uncached` capture-share idiom. `C`'s own local binders (the
            // `let`'s `y`) are bound WITHIN `body`, so they stay unpinned and copy fresh. (breaker mv-class.)
            crate::eval::pin_free_vars(db, body, body, &[]);
            // Rewrite every `(resume v s)` → `C[v]` (the pure delimited continuation applied to the resume
            // value). The arm body's free names keep their pinned resolution through `beta_reduce`; `C`'s
            // free names resolve against the handle scope (the structural splice copy re-parents them). The
            // next-state is dead — nothing after the perform reads state on a pure spine.
            rewrite_resume_to_context(db, substituted, body, perform)
        };
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
    // E5 ESCAPING-K over a RE-PERFORMING continuation (step-3 inc-2b / FACE-1 B2). A general `ctl`-style arm
    // that lets `k` ESCAPE (`cont: Some` carried here) whose delimited continuation `C` ITSELF re-performs
    // the handled effect — `(handle A 5 ((a () s k (use-k k))) (+ (A.a) (A.a)))`, where after the leading
    // `(A.a)` hole `C = (+ □ (A.a))` re-performs `A.a`. The pure-one-hole block above did NOT fire (`pure_
    // hole` fails on the second perform), and the two-hole refold below is keyed on a `resume` node this arm
    // has NONE of — so without this block it declines. It CANNOT be a compile-time in-place fold: `apply(k,
    // v)` runs `C[v]` in a SEPARATE activation, so the re-performed `A.a` in `C` must RE-ENTER the handler.
    // REIFY `k` as a SELF-RE-INSTALLING handler-wrapped closure: `k = (fn (#kv) H[perform := #kv])` where
    // `H` is the WHOLE handle node — splicing the leading perform inside `H` yields `(handle A 5 (arm) (+
    // #kv (A.a)))`, a copy of the handler wrapped around the continuation. Applying it (`(use-k k)` → `(k
    // 10)`) re-enters that handle, whose remaining `(A.a)` now has a home; the re-installed handle has ONE
    // FEWER perform, so `reduce_handle`'s natural re-entry folds it (recursively, N→N-1) — bottoming out at
    // the pure-one-hole reify when a single perform remains. So `(+ (A.a) (A.a))` → `(k 10)` = `(handle A 5
    // (arm) (+ 10 (A.a)))` → (+ 10 10) = 20. NO frame chain / `br_table` — a re-performing continuation is a
    // handler-wrapped closure; the recursion is the existing fold, bounded by the re-entry guard.
    //
    // SOUND SUBSET (this increment): the arm must be STATE-OBLIVIOUS — its body does not reference the state
    // binder `s`. Re-installing with the ORIGINAL `init` seed is only correct when the arm never advanced
    // the state (b2-min's `(use-k k)` never mentions `s`). A STATE-ADVANCING arm (the DES `sleep`'s `(at s
    // d)` wake computation) needs the advanced state threaded into the re-installed handle's seed — the
    // follow-on increment — so it still DECLINES cleanly here (never a wrong value). Also require exactly ONE
    // escaping application shape (single-value `k`), and the continuation must reach only THIS handler's
    // performs (no foreign/host perform in `C`, `body_reaches_foreign_perform` false) — a reified
    // continuation must not span a host boundary (§4.4), and a foreign perform in `C` has no home under the
    // re-installed same-effect handler.
    if let Some(perform) = do_aware_leading_hole(db, body, &ctx)
        && let Resolved::Apply { head, args } = resolved_of(db, perform)
        && let Some((decl, idx)) = is_perform(db, head, &ctx)
        && let Some(arm) = ctx.arms.get(&(decl, idx)).cloned()
        // ESCAPING-K: the arm binds `k` and let it escape (`cont: Some` — carried past the classifier).
        && let Some(k_binder) = arm.cont
        && !ctx.abortive.contains(&(decl, idx))
        // EXACTLY TWO discharged performs in the body: the leading hole + ONE remaining in the continuation
        // `C`. The self-re-installing reify drives ONE re-entry (removing the leading perform), leaving a
        // single-perform handle the pure-one-hole fold bottoms out. A body with >2 performs would need
        // repeated re-installs this single-level reify does not complete (it produces a residual non-
        // applyable continuation), so decline cleanly there — the deeper-recursion increment's job.
        && count_discharged_performs(db, body, &ctx) == 2
        // STATE-OBLIVIOUS: the arm body must not read the state binder (else re-install with `init` is stale).
        && count_param_refs(db, arm.body, arm.state) == 0
        // The continuation must not span a foreign/host perform (host-composition invariant §4.4).
        && !body_reaches_foreign_perform(db, body, &ctx)
        // The whole handle node (H) must be reconstructable. By the time the fold runs, the canonical
        // `(handle E seed arms body)` has been desugared to the INTERNAL `(handle-internal seed arms body)`
        // (its head re-spelled, effect dropped) — that is `body`'s parent. Splicing the leading perform
        // inside it yields `(handle-internal seed arms C[#kv])`, the self-re-installing wrapped handle.
        && let Some(handle_node) = db.parent_of(body)
        && db.ast.head_name(handle_node) == Some(HANDLE_INTERNAL)
    {
        // Substitute the arm's params ↦ (pure-copied) leading-perform args and its state binder ↦ the init
        // seed (nothing runs before the leading perform on the strict spine, so the state there is the seed).
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
        // Reify `k = (fn (#kv) H[leading-perform := #kv])`: splice the leading perform inside the WHOLE
        // handle node (not just the body) so the continuation carries the handler around itself.
        let kv_name = format!("#kv{}", perform.0);
        let kv_binder = db.push_name(&kv_name);
        let kv_ref = db.push_name(&kv_name);
        let cont_body = splice_context(db, handle_node, perform, kv_ref);
        let fn_head = db.push_name("fn");
        let params_list = db.push_list(vec![kv_binder]);
        let k_lambda = db.push_list(vec![fn_head, params_list, cont_body]);
        subst.insert(k_binder, k_lambda);
        let folded = crate::eval::beta_reduce(db, arm.body, &subst);
        reparent_under_handle_site(db, folded, body);
        // Type-consistency guard (as the pure-one-hole block) — the synthesized term is not re-checked
        // before codegen, so an ill-typed composition must decline, never miscompile.
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
    if let Some(perform) = do_aware_leading_hole(db, body, &ctx)
        && let Resolved::Apply { head, args } = resolved_of(db, perform)
        && let Some((decl, idx)) = is_perform(db, head, &ctx)
        && let Some(arm) = ctx.arms.get(&(decl, idx)).cloned()
        && !ctx.abortive.contains(&(decl, idx))
        // A tail-resumptive arm (bare OR do-wrapped interpose/forward) is served by the `thread` path — do
        // NOT steal it here (it would decline a forwarding arm whose resume value is a foreign perform).
        && !is_tail_resumptive_arm(db, arm.body)
        // A MATCH-SHAPED resumptive arm (`(match s ((Some n) (resume n s)) …)`) is served by the thread
        // path's match-shaped-resume-PEEL (`peel_resume_from_arm_body`), which folds a `do`-sequenced
        // multi-perform body over it. The `do`-aware leading-hole above would otherwise let THIS block reach
        // such a `do`-bodied case (`(do (St.get) (+ 1 (St.get)))` over a match-arm) and steal-then-decline it
        // (the refold does not serve a match-peel arm). So defer any peelable arm to the thread path — only a
        // NON-peelable arm (e.g. the DES deferred-resume-thunk `(set (w) s (run-thunk (fn (_u) (resume w w))))`)
        // is this block's. Without the `do`-aware finder this guard was unnecessary (the global finder never
        // reached a `do` body); it becomes load-bearing exactly because `do_aware_leading_hole` now does.
        && peel_resume_from_arm_body(db, arm.body).is_none()
        // MULTI-SHOT is sound only when the continuation `C` (re-reduced per resume) reaches NO FOREIGN
        // perform — i.e. only THIS handler's discharged ops, which the refold folds away into pure code.
        // A ONE-SHOT arm splices `C` once, so any foreign perform in it runs once (sound). But a MULTI-shot
        // arm would re-run a foreign/HOST perform in `C` once per resume — the host-composition invariant
        // (DESIGN §4.4: a reified continuation must not span a host call) forbids that, so require the body
        // to be free of any undischarged (foreign/host) perform when the arm resumes more than once.
        && (count_resumes(db, arm.body) == 1 || !body_reaches_foreign_perform(db, body, &ctx))
        // CONDITIONALLY-RESUMING ARM GUARD (twin of the pure-one-hole block's): decline a partial-resume arm
        // (`(if cond ABORT (resume …))`) — the refold would rewrite only the resuming branch and mis-splice.
        && !arm_partially_resumes(db, arm.body)
    {
        // SILENT-MISCOMPILE GUARD (breaker pyth1). A nested closed HANDLE in the POST-RESUME TOLL position —
        // `(+ (resume v s') (handle E 40 … (+ (E.tick) 2)))` — is NOT reduced to its value by the refold; its
        // dispatches leak into the outer fold instead of folding to a self-contained 42, so a closed handle
        // = 42 in the toll produced 1414 not the correct 196 (uniform wasm+rust+rust-async, distinct-effect
        // identical → a genuine VALUE bug, not routing). The referentially-equal literal 42 in the toll folds
        // correctly. DECLINE (reject-not-miscompile) when the arm body reaches a nested handle OUTSIDE a
        // resume's own args — the resume subtree is skipped because its ANSWER value folds correctly via
        // `splice_context` (pyre6, landed) and its NEXT-STATE is guarded separately in
        // `rewrite_resume_to_refolded_context` (pyre3). Narrow: only a toll/context-position handle trips it.
        if arm_toll_reaches_nested_handle(db, arm.body) {
            return None;
        }
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
        // PIN the substituted arm body's resolution before the refold rebuilds it (breaker ts1, false-CDZ0101
        // unbound body-free-var). A two-site arm whose op-ARG is an enclosing binder — the body performs
        // `(St.feed n)`, `n` = main's param, substituted for `v` into the arm's `(if (> v 10) …)` CONDITION —
        // carries that `n` in the condition. `rewrite_resume_to_refolded_context` rebuilds the `if`-shaped arm
        // via `push_list` (recursing every child, incl. the condition), which OVERWRITES the shared `n` node's
        // parent to the fresh `>` → detaches `n` from main (single-parent arena) → the rebuilt `(if (> n 10) …)`
        // is a root, `n`'s chain dead-ends → spurious CDZ0101 'unbound n' on a VALID program. Anchor
        // `substituted` under the handle site FIRST so `n` resolves up the live chain to main, THEN
        // `resolve_subtree` memoizes every node's resolution against that CURRENT position — so the rebuild
        // can't change how `n` resolves (the apply_lambda pin-before-copy idiom, resolve.rs; the same fix
        // v-inference used for the guard-desugar arm-copy sibling ag5). Idempotent → no hang; a constant-arg
        // arm (no enclosing-binder ref) is byte-identical.
        reparent_under_handle_site(db, substituted, body);
        crate::resolve::resolve_subtree(db, substituted);
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
    // CALLER-OBSERVED OUT-STATE (task #15). Before threading, scan the handle body for a recursive-effectful
    // call whose FINAL out-state a LATER spine item observes (`(do (run-ops …) (Prim.run 0))` — the trailing
    // perform reads the state `run-ops` advanced). Record each such callee so `specialize_recursive` emits it
    // in MULTI-VALUE mode (return `(value, out-state)`); the single-return convention drops the advance and
    // silently miscompiles the observer to the pre-recursion state. The mode decision reads only the callee's
    // OWN body, so this caller-side observation must be recorded up front. Purely additive: it only UPGRADES a
    // multi-value-threadable callee — a non-threadable one stays single-return.
    mark_caller_observed_outstate(db, body, &ctx);
    // RECURSION-BOUNDARY caller-observed out-state (finding #19): a recursive-effectful callee whose out-state
    // flows into an enclosing recursive def's self-call argument must thread across the recursion. Scans the
    // reachable recursive-effectful defs' bodies (not just the handle body) and marks both the callee and the
    // enclosing def multi-value. Additive: gated by `multivalue_leaves_threadable` at the mode decision.
    mark_recursion_boundary_observed_outstate(db, body, &ctx);
    // CALLER-OBSERVED OUT-STATE via a same-effect ABORT arm (breaker sr5). The task-#15 machinery above
    // upgrades a recursive-effectful callee to MULTI-VALUE mode when a LATER spine perform observes its
    // out-state — which threads correctly to a RESUMING observer (sr4 → 2). But when the observing perform's
    // arm is ABORTIVE (a `(fin (u) s s)` with no `resume`), the abort COLLAPSE materializes the arm value
    // (reading the state binder `s`) against the pre-recursion SEED slot rather than the callee's threaded
    // out-state — so `(do (def _g (grow k)) (Acc.fin))` reads 0 instead of the advanced 2: a SILENT
    // MISCOMPILE on both backends (breaker sr5, HIGH). Folding it correctly needs the abort collapse to read
    // the multi-value out-state the recursion advanced (a later increment, same family as the resuming
    // thread). Until then DECLINE cleanly (→ Todo) rather than emit the wrong value. NARROW: fires only when
    // a recursive-effectful callee advancing THIS handler's state is followed on the body spine by an
    // ABORTIVE same-handler perform that reads state — the resuming observer (sr4), the observer as the
    // recursion BASE CASE (sr2), and a plain same-op observer (sr1) all thread/fold correctly and are NOT
    // flagged.
    if !ctx.abortive.is_empty() && body_recursive_advance_observed_by_abort(db, body, &ctx) {
        return None;
    }
    // SEED LET-LIFT for a NON-CONSTANT init carrying a live CAPTURE. The `thread` perform arm splices the
    // threaded state (which starts as `init`) at every `s` reference and `deep_fresh_copy`s each splice to
    // break value/next-state sharing (the resume(a,a) bug). That fresh copy re-pushes each leaf UNPINNED, so
    // an internal state param re-resolves against the specialized def's sig — correct — but a leaf that was
    // RESOLVE-PINNED to a LIVE enclosing binder (a caller runtime arg `k` substituted into the seed by an
    // inlining `(f k)`, arm body `(resume s (+ s 1))` reading `s` twice) loses its pin and re-resolves against
    // the folded orphan → a spurious CDZ0101 "unbound k" (the let-wrapped-handle-seed bug). Rather than teach
    // `deep_fresh_copy` to tell a live capture from a to-be-respecialized param (a fragile global change), bind
    // the seed ONCE here to a fresh INTERNAL name and thread THAT: each `s` splice is then a fresh UNPINNED
    // `#seed` occurrence (nothing to lose on copy) that re-resolves to the wrapping `let`, and the capture `k`
    // sits in exactly ONE place — the let-init, grafted under the handle site below so it resolves up the live
    // chain. GATED to a non-constant seed (a bare int/bool/etc. leaf is scope-independent, shared safely, and
    // stays byte-identical — the common `(handle St 0 …)` corpus case is untouched); the wrap fires only for
    // the rare runtime-arg seed the bug needs.
    let seed_wrap: Option<(StructId, StructId)> = if !seed_is_shareable_constant(db, init) {
        let nm = format!("#seed{}", init.0);
        let binder = db.push_name(&nm);
        let sref = db.push_name(&nm);
        Some((binder, sref))
    } else {
        None
    };
    let thread_seed = seed_wrap.map(|(_, r)| r).unwrap_or(init);
    // Apply the SEED LET-LIFT to a to-be-returned value: `(let ((#seed init)) value)` when the seed was
    // let-bound (non-constant), else the value unchanged. Every `#seed` occurrence threaded into `value`
    // re-resolves to this binding. Used by BOTH the resumptive return (below) AND the abortive returns: an
    // abort arm that READS the state binder — `(halt (u) s (* 1000 (+ (Map.len s) a)))` over a HEAP seed
    // `Map.empty` — carries `#seed` refs (the state binder was threaded as `#seed`), so the collapsed abort
    // value must be wrapped in the same `let` or `#seed` reads UNBOUND (CDZ0101 — the abort × seed-let-lift
    // seam; scalar seeds are shareable constants → no `#seed`, so they never hit it; heap seeds do). breaker
    // heap-abort-state issue 2026-08-04.
    // Use `binder`/`init` DIRECTLY (not copies): `init` is the ORIGINAL seed node — it may carry a free
    // capture (an enclosing fn's param, `(handle St k …)` where `k` is main's param), and the whole `let` is
    // grafted under the handle site by the caller's `reparent_under_handle_site`, restoring `init`'s lexical
    // chain. A `copy_pure(init)` would re-push it DETACHED, orphaning that capture → CDZ0101. Only ONE return
    // path fires per `reduce_handle` call, so reusing `binder`/`init` in one place is safe (no arena aliasing).
    let apply_seed_wrap = |db: &mut Db, value: StructId| -> StructId {
        if let Some((binder, _)) = seed_wrap {
            let let_head = db.push_name("let");
            let pair = db.push_list(vec![binder, init]);
            let bindings = db.push_list(vec![pair]);
            db.push_list(vec![let_head, bindings, value])
        } else {
            value
        }
    };
    // Thread the INIT state through the body in evaluation order. The handle's value is the body's
    // value (the accumulated state is observable only through the operations), so we return the
    // rewritten body; the final threaded state is discarded (the body never reads it directly).
    let (rewritten, _final_states) = thread(db, body, vec![thread_seed], &ctx)?;
    // ABORTIVE (E4): if an abortive perform fired during threading, the handle's value is that arm's
    // value — the surrounding computation was abandoned, so the threaded body is dead. (Unconditional
    // strict abort only; a conditional abort was declined above.)
    if let Some(abort) = ctx.abort_value.get() {
        // ABORT-FOLD, do-SHAPE (preserve a pre-abort FOREIGN advance). An abort collapses the handle to the
        // arm value — but a FOREIGN perform (an OUTER handler's effect, or a host op) evaluated on the strict
        // spine BEFORE the abort has ALREADY committed its state advance to the ENCLOSING computation and must
        // survive (the inner abort is the inner handle's control; it cannot roll back an outer effect's
        // committed step). For a `(do <foreign…> <abortive-perform>)` body, the do-arm above ALREADY keeps the
        // pre-abort foreign items and appends the abort value as the tail — so `rewritten` is a sound
        // `(do <foreign…> <abort-value>)` whose foreign prefix the ENCLOSING fold discharges (advancing the
        // outer state), then yields the abort value. Returning it (instead of the BARE abort) preserves that
        // advance: `(do (A.tick) (B.bail 99))` under B → `(do (A.tick) 99)`, so an outer `(A.get)` reads the
        // advanced state (110, not the 109 the bare-abort collapse produced — breaker ao1-ao4, incl. heap ao3
        // + multi-advance ao4; the do-arm's `kept` already threads the FULL prefix trace per ao4).
        //
        // GATED to `rewritten` being a `do`-FORM. This is what SEPARATES the sound do-shape from the STILL-
        // AMBIGUOUS strict-operand shape (`(+ (A.tick) (B.bail 99))` → `rewritten` `(+ (A.tick) 99)`, a `+`
        // form): a bare-abort `+` case that is CORRECT-because-UNOBSERVED (the corpus case "an abortive
        // perform under THREE nested handlers abandons the two resumptive frames above it", `(+ (A.a) (+ (B.b)
        // (Bail.bail 99)))` = 99) is indistinguishable at THIS (inner) handler from the miscompiling one (the
        // difference lives in the OUTER continuation, invisible here) — so the strict-op lift is a SEPARATE
        // increment. Only the do-form, where the do-arm already produced the sound for-effect sequencing, is
        // safe to return now. The foreign-reach check reads the ORIGINAL `body` (parented — no orphan
        // resolve-pin poison, unlike `rewritten` whose tail is the orphan abort value). ZERO corpus regressions
        // (a `+`/`let`/`match`-shaped abort body is untouched; only a `do` body with a kept foreign prefix
        // flips todo→pass).
        if db.ast.as_form(rewritten, "do").is_some() && body_reaches_foreign_perform(db, body, &ctx)
        {
            // DRAIN before returning (github-liaison review on #2002, HIGH-class). The normal path below
            // runs `drain_and_wrap` to wrap `rewritten` in the binding-`let`s for any pending MULTIVALUE
            // self-call temps (a `f#ctx` self-call arm pushes `#t` to `ctx.pending` and returns `(. t 0)`;
            // the temp is bound here). This do-form early-return must drain TOO: a multivalue self-call in
            // the KEPT pre-abort foreign prefix — `(do (relabel-self…) (A.tick) (B.bail 99))` — would leave
            // its `(. t 0)` referencing an unbound `#t` if we returned `rewritten` undrained → spurious
            // CDZ0101 / no-machine-representation. `drain_and_wrap` is a no-op when nothing is pending (the
            // common case — a plain `(do (A.tick) (B.bail 99))` has no self-call temp), so this is a strict
            // hardening: byte-identical when `ctx.pending` is empty, correct when it is not.
            // ONLY the `#st` drained inits (the FINDING-24 per-dispatch state binds) — NOT every pending
            // entry. `ctx.pending` ALSO holds self-call `#t` temps (a `f#ctx` self-call arm pushes `(#t,
            // call_node)`); forgetting THOSE re-resolves a `call_node` subtree that can hold a BODY binder
            // (a match/let binder like `root`) → CDZ0101 unbound `root` (the compiler-ml self-host reject:
            // db-query-diff where NO `#st` fires, only self-call temps). The forget is FOR the `#st` seed-memo
            // only; filter by the `#st` name prefix so a non-`#st` temp is never touched.
            let do_pending_inits: Vec<StructId> = ctx
                .pending
                .borrow()
                .iter()
                .filter(|(name, _)| name.starts_with("#st") || name.starts_with("#fa"))
                .map(|(_, init)| *init)
                .collect();
            let drained = drain_and_wrap(db, &ctx, 0, rewritten);
            // Wrap in the seed let-lift (heap seed → the do-prefix / abort value may read `#seed`).
            let drained = apply_seed_wrap(db, drained);
            reparent_under_handle_site(db, drained, body);
            // FINDING-24 stale-memo clear (see the resumptive-return twin below): forget ONLY the drained `#st`
            // init subtrees so their `#seed`/`#st` refs re-resolve against the grafted chain (a blanket
            // `forget_subtree(drained)` regressed a live pin — see the twin). Empty unless an `#st` bind drained.
            for &init in &do_pending_inits {
                crate::resolve::forget_subtree(db, init);
            }
            return Some(drained);
        }
        // Re-anchor the abort value under the handle site BEFORE returning — the SAME reparent the normal
        // path does below (line ~1848). The abort value is the arm body `copy_pure`d off the (now-dead)
        // resume/perform node, a synthesized orphan with parent `None`. When it is (or contains) a BARE
        // NAME referencing an ENCLOSING binder — the arm `(bail (n) s n)` returns `n`, bound to the perform
        // arg, which for a RUNTIME arg `(Bail.bail k)` is a reference to the caller's param `k` — the
        // orphan copy has no lexical chain, so `k` reads UNBOUND → Poison → the handle types `Any` and
        // lowering declines "return type has no machine representation" (wasm) while rust computes (a
        // backend split: corpus-bugfix 2026-07-18). A CONST arg `(Bail.bail 7)` folds to a literal (no free
        // name) so it never exhibited this — only a runtime arg leaves a free reference. Re-parenting under
        // the handle restores the chain abort → handle → def so `k` re-resolves to the param and types
        // Int64. (Mirrors the identity-arm pass-through reparent for the resumptive path.)
        // Wrap in the seed let-lift: an abort arm that READS a heap-typed state binder carries `#seed` refs
        // (the state was threaded as `#seed`); without the wrapping `let` they read unbound (CDZ0101).
        let abort = apply_seed_wrap(db, abort);
        reparent_under_handle_site(db, abort, body);
        return Some(abort);
    }
    // MULTI-VALUE (repro-1): if the handle BODY was itself a self-call to a multi-value spec — `(handle …
    // (relabel tree))` — the self-call arm pushed a pending temp and returned `(. t 0)` (the value
    // projection); the temp is not yet bound. Drain any pending temps into wrapping `let`s so the handle
    // value is `(let ((t (f#ctx … init))) (. t 0))`. (The self-call arm already discards each spec's
    // OUT-state at the top level — the handle observes only the value.) Nothing pending → returns `rewritten`.
    // Capture the pending drain-init node ids BEFORE draining (FINDING-24 stale-memo clear, below): these are
    // the `#st` per-dispatch state binds, pre-existing resolved subtrees whose `#seed` refs need re-resolution
    // once the seed-wrap grafts the binder. Empty in the common case (nothing pending → no-op forget).
    // ONLY the `#st` drained inits — NOT every pending entry (see the do-form twin above): `ctx.pending`
    // also holds self-call `#t` temps whose `call_node` can transitively hold a body binder (`root`);
    // forgetting a non-`#st` temp re-resolves it → CDZ0101 unbound `root` (the self-host reject where no
    // `#st` fires). Filter by the `#st` name prefix.
    let pending_inits: Vec<StructId> = ctx
        .pending
        .borrow()
        .iter()
        .filter(|(name, _)| name.starts_with("#st"))
        .map(|(_, init)| *init)
        .collect();
    let wrapped = drain_and_wrap(db, &ctx, 0, rewritten);
    // Apply the SEED LET-LIFT decided above: `(let ((#seed init)) wrapped)`. Every `#seed` occurrence
    // threaded into the body re-resolves to this binding; `init` (carrying the live capture) is evaluated
    // once here, and the whole `let` is grafted under the handle site below so `init`'s free names resolve
    // up the original chain. `None` (a constant seed) leaves `wrapped` untouched — byte-identical to before.
    // (Same helper the abortive returns above use.)
    let wrapped = apply_seed_wrap(db, wrapped);
    // Graft the threaded result UNDER the original handle's site (the same re-anchoring the E5 pure-one-hole
    // and multi-shot blocks above do). The `thread` perform arm returns the resume VALUE as the perform's
    // result via `deep_fresh_copy` — a freshly-pushed subtree whose root parent is `None`. When that value is
    // a BARE NAME referencing an ENCLOSING binder — the IDENTITY-arm pass-through `(handle St k ((get (u) s
    // (resume s s))) (St.get))`, whose folded body is just `k` (main's param) — the copy has no lexical chain,
    // so the reference reads UNBOUND → `Poison` → the handle types as `Any` and lowering declines "return type
    // has no machine representation" (a check/compile divergence: infer's export solve grounds it Int64, but
    // `type_of(rewritten)` here reads `Any`). Re-parenting `wrapped` under the handle node restores the chain
    // wrapped → handle → def so `k` re-resolves to main's param and types Int64. (A folded body forced through
    // an arithmetic op — `(+ s 1)` or a compound body — already re-parents its operands, which is why those
    // faces did not exhibit the leak; the bare pass-through is the gap.)
    reparent_under_handle_site(db, wrapped, body);
    // FINDING-24 stale-memo clear: when the seed was let-lifted (`seed_wrap` present) AND the fold drained a
    // per-dispatch `#st` state bind (v-effects' fold half), that `#st` let-init is a PRE-EXISTING resolved
    // program subtree (e.g. `(List.push #seed22 t)`) whose `#seed` reference was memoized at THREADING time —
    // before `apply_seed_wrap` (above) minted the `#seed` binder. `resolved_of` memoizes against the position
    // at first resolve, so that `#seed` occurrence stays memoized UNBOUND and the just-grafted `#seed` let is
    // never consulted → CDZ0101 `unbound #seed`. Forget ONLY the drained `#st` init subtrees (`pending_inits`)
    // so the next `resolved_of` recomputes their `#seed`/`#st` references against the FINAL `#seed`-outer /
    // `#st`-inner chain this function just built. Same stale-memo-after-reparent class the copied-binder paths
    // clear via `forget_subtree`. TARGETED (not a blanket `forget_subtree(wrapped)`): a whole-tree forget
    // re-resolves correctly-pinned nodes too and regressed 1 case (the `il2` data-driven-interleave-width
    // handler — a live pin shifted). Forgetting only the drained inits clears exactly the stale `#st`/`#seed`
    // memos and leaves every other pin intact → the seed-wrap corpus stays byte-identical (verified 0-regress).
    // `pending_inits` is empty unless the fold drained an `#st` bind, so this is a no-op for every existing
    // handle (constant OR heap seed) — it engages only once v-effects' fold half emits an `#st` state bind.
    for &init in &pending_inits {
        crate::resolve::forget_subtree(db, init);
    }
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

/// FINDING-24 COVERAGE-GAP (breaker sft1, the min-heap-sift face). Canonicalize a SINGLE-ARM match whose
/// only arm is a bare-NAME binder over a COMPOUND scrutinee — `(match <compound> (h2 <body>))` — into the
/// equivalent `let`: `(let ((h2 <compound>)) <body>)`. A single bare-name arm is UNCONDITIONALLY irrefutable
/// (a nullary/payload constructor pattern is a 2-element LIST `(Ctor sub)`, an atom name never is — see
/// `eval::fold_ctor_match` slice 0/1), so the match always succeeds and binds `h2` to the whole scrutinee;
/// the `let` is a pure syntactic identity. WHY it matters: the resumptive fold threads a `match` by copying
/// the SCRUTINEE into every continuation copy (one per dispatch), so a compound scrutinee referencing handler
/// state duplicated per dispatch makes emit grow SUPER-LINEARLY — the same continuation-duplication class as
/// finding-24, exposed through a match-scrutinee instead of a `let`-init (sft1: exponential, 47015 locals in
/// one function → invalid wasm "too many locals"). Routing the scrutinee through a `let` binds it ONCE via
/// the existing per-dispatch `#st` state-bind machinery `drain_and_wrap` already covers, collapsing the
/// growth back to LINEAR (sft1: 1.47MB→38820 bytes, valid, byte-for-byte correct outputs). Gated to a
/// COMPOUND scrutinee (an atom scrutinee is already a single slot — case E in the diagnosis is LINEAR — so
/// leave it untouched to minimize churn) and to a SINGLE arm (a multi-arm match is a real runtime dispatch,
/// not an irrefutable bind). Walks structurally (the match may be nested under `if`/`let`/`do` in an arm
/// body); copy-on-change so an unaffected subtree keeps its node identity (the scrutinee-identity-keyed
/// `fold_ctor_match`/`SumPayload` passes must not see a needlessly-rebuilt spine).
fn hoist_single_binder_match_scrutinee(db: &mut Db, node: StructId) -> Option<StructId> {
    if let Resolved::Match { scrutinee, arms } = resolved_of(db, node)
        && arms.len() == 1
    {
        let (pat, body) = arms[0];
        // The arm pattern must be a bare NAME binder (not `_`, not a `(Ctor …)`/tuple/literal list pattern),
        // and the scrutinee must be COMPOUND (a list — an application/op/ctor expr), not an atom already in
        // one slot. A `_` wildcard is left alone: it binds nothing, so there is no continuation-copied use to
        // collapse, and the scrutinee (if pure) folds away regardless.
        if let Some(name) = db.ast.as_name(pat).map(|s| s.to_string())
            && name != "_"
            && matches!(db.ast.get(scrutinee), Struct::List(_))
        {
            // First hoist any single-binder match NESTED in the body, then rebuild this one as a `let`.
            let body = hoist_single_binder_match_scrutinee(db, body).unwrap_or(body);
            let name_atom = db.push_name(&name);
            let let_head = db.push_name("let");
            let binding = db.push_list(vec![name_atom, scrutinee]);
            let bindings = db.push_list(vec![binding]);
            return Some(db.push_list(vec![let_head, bindings, body]));
        }
    }
    // Recurse structurally: the match may be nested inside an `if`/`let`/`do` arm body.
    match db.ast.get(node).clone() {
        Struct::List(children) => {
            let mut changed = false;
            let mut rebuilt = Vec::with_capacity(children.len());
            for c in children {
                match hoist_single_binder_match_scrutinee(db, c) {
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
            // REJECT-DON'T-MISCOMPILE (v-effects finding #9): this desugar binds the scrutinee to each named
            // arm binder via `copy_pure(scrutinee)` (below) — a fresh COPY per binder. When the scrutinee
            // itself PERFORMS and there are ≥2 named binders (arm-0's guard binder AND a named arm-1 fallback
            // binder, e.g. `((guard x cond) x)` + `(_o _o)`), each copy RE-EVALUATES the performing scrutinee:
            // `(match (St.next) ((guard x (> x (St.next))) …) (_o _o))` emits `(let ((x (St.next)) (_o
            // (St.next))) …)` → the fallback `_o` reads a RE-DRAWN scrutinee (breaker f1: 6 not 3; f2
            // dispatch-witness confirms the third hidden draw). Correctness is INPUT-DEPENDENT — the SAME
            // program is correct when the guard HITS (c2: -400) and wrong when it MISSES (f1) — so the whole
            // shape must reject. The correct fix (bind the performing scrutinee ONCE to a temp, then ALIAS
            // both binders to it — `(let ((k S)) (let ((x k) (_o k)) …))`) is the fresh-context bind-once /
            // let-threading arc (shared root with findings #8/lb/sh). A SINGLE named binder is sound (one copy
            // = one eval — c1's pure-guard shape never enters here, and a lone-binder performing scrutinee
            // folds correctly), so gate ONLY on ≥2 named binders over a performing scrutinee. Names the
            // `let`-lift workaround (c3: 3 — bind the draw once outside the match).
            let named_binder_count = [g[0], pat1]
                .iter()
                .filter(|&&p| db.ast.as_name(p).is_some_and(|n| n != "_"))
                .count();
            if named_binder_count >= 2 && subtree_performs(db, scrutinee, ctx) {
                // DECLINE (return None) rather than rewrite: the caller's `None` arm sees
                // `body_has_performing_match_guard` = true (this same performing-guard shape) and returns
                // `None` for the whole fold — the honest HANDLER_NOT_REDUCIBLE "not yet reducible" todo
                // decline, not the re-evaluation miscompile the copy-per-binder rewrite would produce. Flips to
                // a fold when the bind-once arc materializes the performing scrutinee once and aliases both
                // binders to it.
                return None;
            }
            let if_head = db.push_name("if");
            let if_node = db.push_list(vec![if_head, cond, body0, body1]);
            // Wrap in `let` bindings for any named (non-`_`) inner patterns, so the guard/bodies resolve them
            // to the scrutinee. A wildcard `_` binds nothing.
            let mut binders: Vec<StructId> = Vec::new();
            for &p in &[g[0], pat1] {
                if let Some(name) = db.ast.as_name(p).map(|s| s.to_string())
                    && name != "_"
                {
                    let name_atom = db.push_name(&name);
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
        // REFUTABLE first-arm pattern (a destructuring `(bin …)`/`(tuple …)`/ctor pattern) guarded by a
        // PERFORMING cond, with an irrefutable catch-all second arm. The irrefutable rewrite above (→ `(if g
        // b b2)`) is UNSOUND here: it drops the pattern-match, so a scrutinee that FAILS `P` would still run
        // the guard `g` and pick `b`/`b2` on `g` alone. The sound rewrite KEEPS the pattern match and hoists
        // the performing guard into an `if` INSIDE the matched arm: `(match k ((guard P g) b) (_ b2))` ≡
        // `(match k (P (if g b b2)) (_ b2))`. When `P` matches, `g` runs and selects `b` (guard holds) or
        // `b2` (guard fails); when `P` does not match, the catch-all yields `b2` WITHOUT running `g` — exactly
        // the guarded-match semantics. `g` is now an if-condition WITHIN a match arm, a strict-first position
        // the arm-body fold routes through the enclosing handle (the match-scrutinee/arm fold already
        // descends arm bodies). The catch-all `b2` is copied into the matched arm's else so a failed guard
        // falls through to the same value. Gated identically: second arm irrefutable + no guard, first-arm
        // guard cond performs. (breaker bg-family, refutable-pattern face.)
        if let Some(g) = db.ast.as_form(pat0, "guard").map(|t| t.to_vec())
            && g.len() == 2
            && !is_irrefutable_pattern(db, g[0])
            && subtree_performs(db, g[1], ctx)
            && db.ast.as_form(pat1, "guard").is_none()
            && is_irrefutable_pattern(db, pat1)
        {
            let inner_pat = g[0];
            let cond = g[1];
            // `(if g b0 <copy of b2>)` — the matched arm's body: guard holds → b0, else the catch-all value.
            let if_head = db.push_name("if");
            let b2_for_else = copy_pure(db, body1);
            let if_node = db.push_list(vec![if_head, cond, body0, b2_for_else]);
            // Rebuild the match: first arm keeps the REFUTABLE pattern with the guard-hoisted `if` body; the
            // catch-all is unchanged. `(match scrut (inner_pat if_node) (pat1 body1))`.
            let match_head = db.push_name("match");
            let scrut_copy = copy_pure(db, scrutinee);
            let arm0 = db.push_list(vec![inner_pat, if_node]);
            let arm1 = db.push_list(vec![pat1, body1]);
            return Some(db.push_list(vec![match_head, scrut_copy, arm0, arm1]));
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
/// Reduce a handler-arm body that DEFERS its resume inside a closure STORED IN A COMPOUND and applied
/// through a helper — the DES multi-task pqueue's store→pop→apply shape: `(sleep (wake) s (unbox-apply
/// (Box.Box (fn (_u) (resume unit wake)))))` where `unbox-apply(b) = match b ((Box.Box th) (th unit))`.
/// The `resume` is buried behind the `Box.Box` constructor + `unbox-apply`'s match, so the fold's
/// classifiers see no TAIL resume and the arm declines. Compose reductions until it surfaces:
///  (a) β-reduce a one-shot non-recursive helper call [`apply_lambda`]; ONE-SHOT guard `count_param_refs ≤ 1`
///      so a resume-bearing closure argument is not DUPLICATED (v-discrete-event-sim CONFIRMED every stored
///      continuation applies EXACTLY ONCE — pop removes before apply — so this guard IS the DES contract);
///  (b) case-of-known-constructor fold a `match` over a visible ctor [`eval::fold_ctor_match`, v-inference's
///      SumPayload-aware substitution — the `(Ctor v)` binder's uses resolve to `SumPayload{scrutinee}` not
///      `Ref`, so a targeted resolve-time rewrite, not `beta_reduce`];
/// repeating until a tail `resume` surfaces or no progress. Unchanged body ⇒ returned as-is (byte-identical),
/// so tail/abortive/already-foldable arms are untouched. Bounded iteration. Conservative: any shape it
/// cannot cleanly reduce is left as-is (the arm then declines exactly as before — never a mis-fold).
/// A ONE-SHOT non-recursive helper call reducible by the deferred-resume fold: `head` is a non-recursive
/// lambda/def, arity matches, EACH param is used at most once in the body (so β-reduction never DUPLICATES a
/// deferred resume-closure — the one-shot continuation contract), and every arg is either strongly pure or a
/// lambda (a deferred resume-closure). Returns the β-reduced body (NOT yet re-resolved — the caller
/// `forget_subtree`s it), or `None` if `node` is not such a call. Shared by the deferred-resume fold's arm
/// (a) (the call is the whole term) and arm (a′) (the call is a match SCRUTINEE) so the two gates cannot
/// drift.
fn reduce_one_shot_helper_call(db: &mut Db, node: StructId, ctx: &HandlerCtx) -> Option<StructId> {
    if let Resolved::Apply { head, args } = resolved_of(db, node)
        && is_perform(db, head, ctx).is_none()
        && let Some((params, hbody)) = crate::eval::lambda_params_and_body(db, head)
        && !crate::eval::is_recursive(db, hbody)
        && params.len() == args.len()
        && params
            .iter()
            .all(|&p| count_param_refs(db, hbody, crate::eval::param_name_occ(db, p)) <= 1)
        && args.iter().all(|&a| {
            strongly_pure(db, a, ctx) || matches!(resolved_of(db, a), Resolved::Lambda { .. })
        })
    {
        return crate::eval::apply_lambda(db, head, &args).ok().flatten();
    }
    None
}

/// ESCAPED-CLOSURE-LEAK RECOVERY (cx5d). When `reduce_handle`'s output LEAKS a discharged perform — a
/// closure performing an arm op, passed to a NON-recursive ONE-SHOT helper that APPLIES it OUTSIDE the fold's
/// reach (the closure lifts standalone, its perform un-homed → `reduced_body_leaks_escaped_perform` fires) —
/// β-INLINE each such helper call in the ORIGINAL handle `body`, so the closure is applied INLINE, within
/// the fold's reach. A subsequent re-run of `reduce_handle` then threads the now-in-reach perform. Returns
/// the rewritten body if it inlined ≥ 1 such call (already re-resolved), else `None`.
///
/// CTX-FREE (matches the discharged ops directly from `arms`). SAFETY is the caller's: it re-runs
/// `reduce_handle` and RE-CHECKS the leak, using the result only if it now folds cleanly — so this can only
/// turn a decline into a fold, NEVER change a folding case (every case that folds today reaches no leak, so
/// the caller never invokes this) nor a value (one-shot: each helper param is used `<= 1` time, so the
/// inlined perform runs exactly once). A recursive / multi-use helper, or a non-lambda-non-simple arg, is
/// left alone (the honest decline stands, as before).
pub fn inline_escaped_one_shot_perform_call(
    db: &mut Db,
    body: StructId,
    arms: &[HandleArm],
) -> Option<StructId> {
    let arm_ops: Vec<(u32, u32)> = arms
        .iter()
        .filter_map(|a| crate::eval::effect_op_of(db, a.op).map(|(d, i)| (d.0, i)))
        .collect();
    if arm_ops.is_empty() {
        return None;
    }
    let (rewritten, changed) = inline_escaped_worker(db, body, &arm_ops);
    if changed {
        // `apply_lambda` copies the callee body (a copied binder ref keeps a stale memoized resolution) and
        // the `push_list` rebuild produces fresh unresolved parents — forget the subtree so the re-run
        // `reduce_handle`'s `resolved_of` recomputes every ref against the inlined structure (the same
        // re-resolve hygiene `reduce_arm_deferred_resume` applies per inlined call). KEEP-PINNED: a capture
        // the escaping closure carries (a free var `pin_free_vars` resolved + `apply_lambda` SHARED into the
        // reduced body — e.g. an outer `let ((a 7))` the closure reads) is a shared node with a STALE parent
        // (the dead original closure), so forgetting + force-structural-re-resolving it walks that dead chain
        // into an orphan → a spurious CDZ0101 (breaker sk4c). Its pinned memo is already correct, so preserve
        // it; only the freshly re-parented (non-pinned) refs need recompute.
        crate::resolve::forget_subtree_keep_pinned(db, rewritten);
        // A LOAD-TIME reference re-parented by the rebuild (e.g. a `let`-body ref when the inlined helper
        // call sat in the binding-init) keeps its stale load-time `scope_skip` entry, so the fast-path would
        // resolve it against its ORIGINAL scope (the pre-rebuild init) rather than the rebuilt one — leaving
        // a later hygiene rename to miss it and the ref dangling (a false CDZ0101, breaker iso-b). Force the
        // subtree's load-time nodes onto the exhaustive walk so every ref resolves against the CURRENT
        // (rebuilt) parent chain. `forget_subtree` alone is insufficient — it clears the resolution memo but
        // not the load-time skip coverage the recompute then re-consults.
        db.force_structural_resolution_subtree(rewritten);
        Some(rewritten)
    } else {
        None
    }
}

/// Recursive worker for [`inline_escaped_one_shot_perform_call`]: rewrite `node`, β-inlining an eligible
/// one-shot helper call that passes an escaping performing closure. Returns `(rewritten, changed)`.
fn inline_escaped_worker(db: &mut Db, node: StructId, arm_ops: &[(u32, u32)]) -> (StructId, bool) {
    if let Resolved::Apply { head, args } = resolved_of(db, node)
        && crate::eval::effect_op_of(db, head).is_none() // the head is not itself a perform
        && let Some((params, hbody)) = crate::eval::lambda_params_and_body(db, head)
        && !crate::eval::is_recursive(db, hbody)
        && params.len() == args.len()
        // DUPLICATION-SAFETY. Originally a strict one-shot bound (every param referenced `<= 1`), which
        // rejected a closure APPLIED MULTIPLE TIMES through the helper (`apply-twice(g) = (+ (g 1) (g 2))`,
        // pclos) — yet that duplication is exactly correct: each `(g i)` is an independent application, so
        // inlining to `(+ (Src.read 1) (Src.read 2))` reproduces the two per-application performs + state
        // advances the un-inlined program has. Relax it: a param may be referenced more than once when its
        // argument is safe to duplicate at each reference — a LAMBDA closure (each reference is an
        // application) or a SIMPLE-PURE name/atom (no effect to duplicate). The following clause already
        // constrains EVERY arg to exactly those two shapes, and the recovery's post-fold re-check (no
        // residual leak + non-poison core, `lower.rs`) is the correctness net that rejects any misfold.
        && params.iter().zip(args.iter()).all(|(&p, &a)| {
            count_param_refs(db, hbody, crate::eval::param_name_occ(db, p)) <= 1
                || arg_is_lambda_valued(db, a)
                || arg_is_simple_pure(db, a)
        })
        && args
            .iter()
            .all(|&a| arg_is_lambda_valued(db, a) || arg_is_simple_pure(db, a))
        && args.iter().any(|&a| lambda_body_reaches_op(db, a, arm_ops))
        && let Ok(Some(reduced)) = crate::eval::apply_lambda(db, head, &args)
    {
        // The inlined body may expose another such call (finite AST bounds the recursion).
        let (r, _) = inline_escaped_worker(db, reduced, arm_ops);
        return (r, true);
    }
    match db.ast.get(node).clone() {
        Struct::List(children) => {
            let mut new_children = Vec::with_capacity(children.len());
            let mut any = false;
            for c in children {
                let (rc, ch) = inline_escaped_worker(db, c, arm_ops);
                any |= ch;
                new_children.push(rc);
            }
            if any {
                (db.push_list(new_children), true)
            } else {
                (node, false)
            }
        }
        Struct::Atom(_) => (node, false),
    }
}

/// A syntactically-simple PURE argument: a bare name or an atom leaf (no perform to preserve/reorder). Used
/// by the escaped-closure recovery's one-shot gate for the NON-lambda args (the lambda arg is the closure).
fn arg_is_simple_pure(db: &Db, node: StructId) -> bool {
    db.ast.as_name(node).is_some() || matches!(db.ast.get(node), Struct::Atom(_))
}

/// Whether `node` is a LAMBDA VALUE — a `(fn …)`, SEEN THROUGH a type ANNOTATION `(: (fn …) (-> A B))`.
/// A one-shot helper's β-reduction (hop 1) carries the parameter's declared type onto the substituted
/// argument (`substituted_arg` → `(: closure (-> …))`), so at the NEXT hop the same closure arrives as an
/// `Annot`, not a bare `Lambda` — a raw `matches!(resolved_of, Lambda)` misses it and the multi-hop inline
/// stalls after one hop (cx6). `lambda_of` (which the reach/apply gates use) already sees through `Annot`;
/// this mirrors that so the arg-eligibility gate agrees, letting the recovery chain through every hop. Only
/// an annotated/bare lambda passes — an `Annot` wrapping a non-lambda does not (its `expr` recurses to
/// `false`), keeping the one-shot gate as tight as before for non-closure args.
fn arg_is_lambda_valued(db: &mut Db, node: StructId) -> bool {
    match resolved_of(db, node) {
        Resolved::Lambda { .. } => true,
        Resolved::Annot { expr, .. } => arg_is_lambda_valued(db, expr),
        _ => false,
    }
}

/// Whether `node` is a lambda whose BODY reaches a perform of one of `arm_ops` (a discharged op this handle
/// routes) — the escaping closure whose perform the recovery re-homes by inlining its applying helper.
fn lambda_body_reaches_op(db: &mut Db, node: StructId, arm_ops: &[(u32, u32)]) -> bool {
    let Some((_params, hbody)) = crate::eval::lambda_params_and_body(db, node) else {
        return false;
    };
    subtree_reaches_arm_op(db, hbody, arm_ops)
}

fn subtree_reaches_arm_op(db: &mut Db, node: StructId, arm_ops: &[(u32, u32)]) -> bool {
    if let Resolved::Apply { head, .. } = resolved_of(db, node)
        && let Some((d, i)) = crate::eval::effect_op_of(db, head)
        && arm_ops.contains(&(d.0, i))
    {
        return true;
    }
    match db.ast.get(node).clone() {
        Struct::List(children) => children
            .iter()
            .any(|&c| subtree_reaches_arm_op(db, c, arm_ops)),
        Struct::Atom(_) => false,
    }
}

/// Rebuild a `(match SCRUTINEE arm…)` node with a NEW scrutinee, preserving the arms. The match AST is
/// `(match <scrutinee> <arm>…)` — child 0 is the `match` head, child 1 is the scrutinee, the rest are arms
/// — so this replaces child 1. `arms` is passed for a shape assertion only (the arm children are copied from
/// the original node verbatim). Falls back to the original node if it is not a well-formed match list.
fn rebuild_match_scrutinee(
    db: &mut Db,
    node: StructId,
    _arms: &[(StructId, StructId)],
    new_scrutinee: StructId,
) -> StructId {
    if let Struct::List(children) = db.ast.get(node).clone()
        && children.len() >= 2
    {
        let mut new_children = children;
        new_children[1] = new_scrutinee;
        return db.push_list(new_children);
    }
    node
}

fn reduce_arm_deferred_resume(db: &mut Db, node: StructId, ctx: &HandlerCtx) -> StructId {
    let mut cur = node;
    for _ in 0..32 {
        if tail_resume(db, cur).is_some() {
            break; // exposed — done
        }
        // (b) a match over a known ctor → its arm body with SumPayload-binder refs substituted.
        if let Some(folded) = crate::eval::fold_ctor_match(db, cur) {
            cur = folded;
            continue;
        }
        // (a) a one-shot non-recursive helper call whose args are pure or deferred resume-closures.
        if let Some(reduced) = reduce_one_shot_helper_call(db, cur, ctx) {
            // RE-RESOLVE the β-reduced body: `apply_lambda` COPIES the callee's arm bodies, and a copied
            // `(Ctor v)` binder ref keeps its memoized `SumPayload { scrutinee }` pointing at the callee's
            // ORIGINAL match scrutinee — so a following `fold_ctor_match` (whose gate is `s == scrutinee`,
            // node-identity) would skip the binder, leaving it unsubstituted (the stale-scrutinee skip
            // v-inference diagnosed for the recursion-unfold — the SAME hazard applies here when a helper
            // whose body matches its arg is inlined, e.g. `sched-step` popping the pqueue). Forgetting the
            // memoized resolution makes `resolved_of` recompute each binder ref against the copy's own
            // scrutinee occurrence — the `fuse_match_into_if` clone+re-resolve hygiene.
            crate::resolve::forget_subtree(db, reduced);
            cur = reduced;
            continue;
        }
        // (a′) the one-shot helper call sits in the SCRUTINEE of a `(match (helper …) arms…)` — the pop of a
        //      DIRECTLY-built entry constructed by a helper: `(sched-step (mk1 wake kb))` first reduces the
        //      outer `sched-step` (arm a) to `(match (mk1 wake kb) …)`, whose scrutinee is still the
        //      UNREDUCED `(mk1 …)` call, so `fold_ctor_match` (which needs a VISIBLE ctor) can't fire and the
        //      arm binders never surface (they resolve Poison). Reduce that nested scrutinee call in place —
        //      same one-shot predicate + re-resolve hygiene as arm (a) — turning the scrutinee into the
        //      helper's `(PQCons …)` body so the next iteration's `fold_ctor_match` pops it. Distinct from
        //      arm (c) (a RECURSIVE callee in the scrutinee, one-level-unfolded); this is the non-recursive
        //      direct-helper analogue, the shape v-inference isolated (a NON-recursive `mk1` reproduces the
        //      same decline the recursive `pins` does — the blocker is a nested-scrutinee call not reduced
        //      before the pop, NOT the recursion).
        if let Resolved::Match { scrutinee, arms } = resolved_of(db, cur) {
            // The scrutinee may carry a leading annotation `(: (helper …) T)` (the handler-arm value is
            // annotated at its declared type) — peel it to reach the call before testing/reducing.
            let call = match resolved_of(db, scrutinee) {
                Resolved::Annot { expr, .. } | Resolved::ConstBlock { expr } => expr,
                _ => scrutinee,
            };
            if let Some(reduced_scrut) = reduce_one_shot_helper_call(db, call, ctx) {
                crate::resolve::forget_subtree(db, reduced_scrut);
                cur = rebuild_match_scrutinee(db, cur, &arms, reduced_scrut);
                continue;
            }
        }
        // (c) a RECURSIVE helper call whose concrete argument selects a NON-recursive (base) arm — the
        //     time-ordered pqueue's `pins` (sorted-insert): `(pins PQNil t kb)` matches `q` = the visible
        //     `PQNil` ctor, takes the base arm `(PQCons (tuple t kb PQNil))`, and never recurses. Such a call
        //     is typically NESTED in an argument (`(sched-step (pins PQNil …))`) — the pop applies to the
        //     insert's result — so rewrite the FIRST such nested call in place: unfold `pins` ONE level (the
        //     recursion-permitting variant, since `apply_lambda` refuses a recursive callee up front), let
        //     `fold_ctor_match` resolve its internal `(match q …)` on the now-visible ctor, and splice the
        //     base-arm result back. Turning the `pins` arg into a directly-constructed `PQCons` lets the
        //     existing arm (a) reduce the surrounding `sched-step` on the next iteration. ACCEPT the unfold
        //     only if the folded body has NO residual self-call to the callee (base arm taken); a non-base
        //     concrete arg (recursion actually needed) leaves the self-call and is DISCARDED — a clean
        //     decline. "Peek one level, accept only if base-arm" is the loop's recursion budget;
        //     `fold_ctor_match` stays a pure one-step fold (v-inference's co-owned split).
        if let Some(rewritten) = rewrite_recursive_base_arm_call(db, cur, ctx) {
            cur = rewritten;
            continue;
        }
        break; // no further reduction applies
    }
    cur
}

/// Find the FIRST call to a recursive helper whose concrete argument selects a NON-recursive base arm and
/// rewrite it to that arm's (folded) body in place, rebuilding the surrounding structure. Returns `Some`
/// with the rewritten `node` if such a call was found + folded, else `None` (no progress). The call is
/// usually nested in an argument — `(sched-step (pins PQNil t kb))` — so this walks structurally and
/// rewrites the innermost-first match. See the (c) block in [`reduce_arm_deferred_resume`] for the policy;
/// the accept gate is `!body_calls_def(folded, callee)` (base arm taken → no residual self-call).
fn rewrite_recursive_base_arm_call(
    db: &mut Db,
    node: StructId,
    ctx: &HandlerCtx,
) -> Option<StructId> {
    // Try to rewrite THIS node if it is the recursive-base-arm call.
    if let Resolved::Apply { head, args } = resolved_of(db, node)
        && is_perform(db, head, ctx).is_none()
        && let Some(callee) = callee_def_index_of(db, head)
        && let Some((params, hbody)) = crate::eval::lambda_params_and_body(db, head)
        && crate::eval::is_recursive(db, hbody)
        && params.len() == args.len()
        // ONE-SHOT guard, scoped to the params whose ARGUMENT is a resume-closure (a lambda). Such an arg
        // must not be DUPLICATED (a deferred continuation applies exactly once — pop removes before apply),
        // so its param may appear at most once in the callee body. A PURE arg (`PQNil`, `wake`) is freely
        // duplicable — a sorted-insert references its queue/element params multiply while rebuilding the
        // order, harmless for pure values; enforcing ≤1 on those would reject every real recursive insert.
        && params.iter().zip(args.iter()).all(|(&p, &a)| {
            if matches!(resolved_of(db, a), Resolved::Lambda { .. }) {
                count_param_refs(db, hbody, crate::eval::param_name_occ(db, p)) <= 1
            } else {
                true
            }
        })
        && args.iter().all(|&a| {
            strongly_pure(db, a, ctx) || matches!(resolved_of(db, a), Resolved::Lambda { .. })
        })
        && let Some(unfolded) = crate::eval::apply_lambda_one_level_recursive(db, head, &args)
            .ok()
            .flatten()
        // RE-RESOLVE the unfolded node before folding. `apply_lambda`'s β-reduce COPIES the callee's arm
        // bodies, but a copied `(Ctor v)` binder ref keeps its memoized `SumPayload { scrutinee }` pointing
        // at the callee's ORIGINAL match-scrutinee occurrence — not the fresh one in the unfolded copy. So
        // `fold_ctor_match`/`rewrite_sum_payload` (whose substitution gate is `s == scrutinee`, node-identity)
        // would silently skip those refs, leaving the binder (`kb`) unsubstituted and the inner `(match kb …)`
        // pointing at a stale scrutinee → the continuation never surfaces. Forgetting the memoized resolution
        // (both the `resolved` column and the `resolved_subtrees` walk-guard) makes `resolved_of` recompute
        // each binder ref against the copy's OWN scrutinee occurrence — the same clone+re-resolve hygiene
        // `fuse_match_into_if` (lower.rs) applies when it clones match arms into `if` branches.
        && {
            crate::resolve::forget_subtree(db, unfolded);
            true
        }
        // Unfolding a callee whose param carries a resume-closure LET-BINDS that arg once (eval-once), so
        // the unfolded body is `(let ((kb …)) (match q …))` — fold the match THROUGH the leading `let`
        // wrappers (kept around the folded arm, preserving the one-shot binding).
        && let Some(folded) = fold_ctor_match_through_lets(db, unfolded)
        // ACCEPT only if the base arm was taken — no residual self-call to the callee. A non-base concrete
        // arg (recursion actually needed) leaves the self-call in the folded body → DISCARD (clean decline).
        && !body_calls_def(db, folded, callee)
    {
        return Some(folded);
    }
    // Otherwise recurse into children, rebuilding the first branch that rewrites.
    if let Struct::List(children) = db.ast.get(node).clone() {
        for (i, &c) in children.iter().enumerate() {
            if let Some(rc) = rewrite_recursive_base_arm_call(db, c, ctx) {
                let mut new_children = children.clone();
                new_children[i] = rc;
                return Some(db.push_list(new_children));
            }
        }
    }
    None
}

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
    // A RECURSIVE callee is not inlined but IS specialized (`specialize_recursive`), and that path returns
    // the callee's value as an ordinary return — so an abortive op inside a SELF-recursive callee, reached
    // through a NON-tail position in the handle body, has its abort value flow into the PENDING continuation
    // instead of abandoning it (`(+ (go 2) 999999)` where `go` bails → 500+999999 not 500 — adv-52, a silent
    // wrong value on all backends). The `specialize_recursive` abortive guard only refuses NON-tail
    // *recursion*; a TAIL-recursive callee like `go` passes it, so the unsoundness must be caught HERE: an
    // abortive op inside a recursive callee's body is opaque to the syntactic hoist exactly as a non-recursive
    // one is, so it is flagged the same way (denying the enclosing operand capturable-tail → `reduce_handle`
    // declines to the safe floor; a full fold needs the br-out-of-handle non-local-exit convention). We still
    // walk the body ONCE for a conditional abort (the self-call inside it is just another `Apply` node the
    // structural walk descends past — `subtree_has_conditional_abortive` never re-enters this fn, so a
    // recursive body is not a non-termination risk).
    subtree_has_conditional_abortive(db, body, ctx, false)
}

/// finding #11-B (oamin4/oa3): whether `init` is a call `(helper arg…)` whose callee ABORTS INSIDE A MATCH
/// ARM and one of whose ARGUMENTS reaches a FOREIGN perform — the def-boundary conditional-abort shape that
/// slips past the syntactic hoist and the ordinary cross-fn guard (which walks `if`/`and` conditionals but
/// not `match` arms). The `unwrap`-shape `(unwrap (E.fetch) tag)`: `unwrap` aborts in its `(None) => Bail.out
/// tag` arm (a CONDITIONAL abort — the `(Some v) => v` arm returns a value that flows into the continuation),
/// and the `E.fetch` argument performs an OUTER handler's op. Under such a call the abort must home to the
/// Bail boundary, but the fold captures it per-branch and threads the abort value into the continuation (a
/// silent wrong value: single call → 10223, chained pair → 2667423). GATED on the foreign argument so the
/// PURE-argument controls (oamin1/oamin5: `(unwrap (if …) tag)`) — where the abort DOES home correctly —
/// keep folding. Narrow by construction: it fires only for a call carrying a foreign perform into a
/// match-aborting helper, the exact def-boundary shape the non-local-exit convention will later fold.
fn init_is_foreign_arg_match_abort_call(db: &mut Db, init: StructId, ctx: &HandlerCtx) -> bool {
    let Resolved::Apply { head, args } = resolved_of(db, init) else {
        return false;
    };
    if is_perform(db, head, ctx).is_some() {
        return false; // a direct perform, not a helper call
    }
    let Some(body) = crate::eval::lambda_body(db, head)
        .or_else(|| crate::eval::lambda_body_of_nullary(db, head))
    else {
        return false;
    };
    if crate::eval::is_recursive(db, body) {
        return false; // a recursive callee is specialized, not inlined — covered elsewhere
    }
    // The callee aborts inside a match arm (a conditional abort the `if`-only hoist/guard cannot see), AND an
    // ARGUMENT of the call DIRECTLY performs a foreign op (the outer `E.fetch` — the sound-capture blocker).
    // Test the ARGUMENTS narrowly (`next_state_directly_performs_foreign` = a literal `(Outer.op …)` not
    // followed through user calls), NOT `body_reaches_foreign_perform`, which follows the callee and would
    // OVER-report on the helper's own match/pattern heads (flagging the pure-arg controls oamin1/oamin5).
    if !callee_aborts_in_match_arm(db, body, ctx) {
        return false;
    }
    args.iter()
        .any(|&a| next_state_directly_performs_foreign(db, a, ctx))
}

/// Whether `node` contains an abortive perform sitting inside a `match` ARM BODY — the match-arm analogue of
/// the `if`-branch conditional abort `subtree_has_conditional_abortive` already detects. Kept SEPARATE from
/// that walk so the existing cross-fn guard sites (which correctly fold a pure-scrutinee match-arm abort per
/// branch) are unperturbed; only the finding #11-B let-init gate consults this.
fn callee_aborts_in_match_arm(db: &mut Db, node: StructId, ctx: &HandlerCtx) -> bool {
    if let Resolved::Match { arms, .. } = resolved_of(db, node)
        && arms
            .iter()
            .any(|&(_, arm_body)| subtree_has_abortive_perform(db, arm_body, ctx))
    {
        return true;
    }
    match db.ast.get(node).clone() {
        Struct::List(children) => children
            .iter()
            .any(|&c| callee_aborts_in_match_arm(db, c, ctx)),
        Struct::Atom(_) => false,
    }
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
                        let if_head = db.push_name("if");
                        return Some(db.push_list(vec![if_head, cond, new_then, new_else]));
                    }
                }
            }
        }
    }
    // A strict application `(op … operand …)` whose OPERAND is a `(let (binds) body)` — pure bindings — whose
    // BODY carries an abortive perform (typically `(let ((d n)) (if c (A.out n) n))`, the LET-WRAPPED-PREDICATE
    // face of finding #11: the conditional abort sits under a `let`, so the `if`-operand hoist above does not
    // see it). LIFT the `let` OUT to wrap the whole application: `(op … (let (b) body) …)` ≡ `(let (b) (op …
    // body …))`. The next hoist pass then sees `(op … body …)` with the `if`-abort DIRECT in the operand and
    // distributes it. Sound when the `let`'s binding inits AND every OTHER operand (and the head) are pure —
    // then the lifted bindings run in the same observable order (nothing effectful is reordered), and the
    // bindings scope over the rebuilt application (they were only used inside `body`). Only fires when the let
    // body carries an abort (a plain value-let operand threads normally). Distinct from the let-INIT-carries-if
    // case below (there the `if` is a binding init; here it is the let BODY, one level out).
    if let Resolved::Apply { head, args } = resolved_of(db, node)
        && is_perform(db, head, ctx).is_none()
        && !subtree_performs(db, head, ctx)
    {
        for (i, &a) in args.iter().enumerate() {
            if let Some(letform) = db.ast.as_form(a, "let").map(|t| t.to_vec())
                && letform.len() == 2
                && subtree_has_abortive_perform(db, letform[1], ctx)
                && let Struct::List(pairs) = db.ast.get(letform[0]).clone()
            {
                let binds_pure = pairs.iter().all(|&p| match db.ast.get(p).clone() {
                    Struct::List(pkv) if pkv.len() == 2 => !subtree_performs(db, pkv[1], ctx),
                    _ => true,
                });
                let others_pure = args
                    .iter()
                    .enumerate()
                    .all(|(j, &b)| j == i || !subtree_performs(db, b, ctx));
                if binds_pure && others_pure {
                    // `(op a0 … (let (b) body) … ak)` → `(let (b) (op a0 … body … ak))`.
                    let inner_app: Vec<StructId> = std::iter::once(head)
                        .chain(
                            args.iter()
                                .enumerate()
                                .map(|(j, &b)| if j == i { letform[1] } else { b }),
                        )
                        .collect();
                    let app = db.push_list(inner_app);
                    let let_head = db.push_name("let");
                    return Some(db.push_list(vec![let_head, letform[0], app]));
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
        let if_head = db.push_name("if");
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
                            let let_head = db.push_name("let");
                            db.push_list(vec![let_head, new_bindings, body_occ])
                        };
                        let new_then = rebuild(db, then_);
                        let new_else = rebuild(db, else_);
                        let if_head = db.push_name("if");
                        return Some(db.push_list(vec![if_head, cond, new_then, new_else]));
                    }
                }
            }
        }
    }
    // An `(if COND t e)` whose CONDITION is itself an `(if c2 t2 e2)` carrying an abort — the shape the
    // connective desugar above LEAVES when an `and`/`or` with an abortive rhs is an ENCLOSING if's condition:
    // `(if (and b (Bail 7)) 100 200)` → (connective desugar) → `(if (if b (Bail 7) false) 100 200)`. The
    // abort is now buried in the condition's branch, where neither the operand-distribution nor the
    // branch-tail capture reaches it. Distribute the OUTER if THROUGH its condition: `(if (if c2 t2 e2) t e)`
    // ≡ `(if c2 (if t2 t e) (if e2 t e))`. Sound: `c2` is evaluated once either way (no duplication); the
    // outer branches `t`/`e` are duplicated into each inner branch, so they must be PURE (an effectful `t`/`e`
    // would run on a path it shouldn't, or twice). After the lift, an aborting `t2`/`e2` sits as an inner
    // `if`'s CONDITION `(if (Bail 7) t e)` — an abort in a strict-first (condition) position the next pass /
    // the `if`-condition thread handles (an abort in a condition abandons before branching, already folded).
    // Only fires when the condition-if carries an abort (a pure nested condition-if is left alone).
    if let Resolved::If { cond, then_, else_ } = resolved_of(db, node)
        && let Resolved::If {
            cond: c2,
            then_: t2,
            else_: e2,
        } = resolved_of(db, cond)
        && subtree_has_abortive_perform(db, cond, ctx)
        && !subtree_performs(db, then_, ctx)
        && !subtree_performs(db, else_, ctx)
    {
        let mk_if = |db: &mut Db, c: StructId, t: StructId, e: StructId| -> StructId {
            let if_head = db.push_name("if");
            db.push_list(vec![if_head, c, t, e])
        };
        // Copy the duplicated outer branches per inner arm (single-parent arena — each use needs its own node).
        let t_a = copy_pure(db, then_);
        let e_a = copy_pure(db, else_);
        let t_b = copy_pure(db, then_);
        let e_b = copy_pure(db, else_);
        let inner_then = mk_if(db, t2, t_a, e_a);
        let inner_else = mk_if(db, e2, t_b, e_b);
        return Some(mk_if(db, c2, inner_then, inner_else));
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

/// Whether `body` is served by the E5 ONE-SHOT refold (the pure-one-hole OR two-hole fold in `reduce_handle`)
/// — a positive servability check SHARED with the Site-5 `#cv`-lift so the two never contend (specificity
/// ordering, concierge-steered 2026-08-04). The refold serves a body whose leading discharged perform sits on
/// a strict spine and whose arm is a NON-tail-resumptive one-shot resumptive arm (a tail-resumptive arm is
/// served by the `thread` path, NOT the refold). Mirrors the two-hole refold's gate (`reduce_handle`, the
/// `do_aware_leading_hole` block): a leading hole + a discharged, non-abortive, NON-tail-resumptive,
/// non-peelable, non-partially-resuming arm. The DECISIVE conjunct separating the ao10 shape from the refold
/// cases is `is_tail_resumptive_arm`: ao10's outer arm `(tick (u) s (resume s (+ s 1)))` IS tail-resumptive
/// (refold declines → the `#cv`-lift is the right transform → 111), while the refold-test arm `(+ 1 (resume 10
/// s))` is NON-tail (refold serves → don't lift, defer → 1/13). When this returns true, the Site-5 5b lift
/// stands down. (`pure_hole`/`do_aware_leading_hole` alone don't separate them — both shapes have a leading
/// hole; the arm shape is the separator.)
fn body_served_by_oneshot_refold(db: &mut Db, body: StructId, ctx: &HandlerCtx) -> bool {
    let Some(perform) = do_aware_leading_hole(db, body, ctx) else {
        return false;
    };
    let Resolved::Apply { head, .. } = resolved_of(db, perform) else {
        return false;
    };
    let Some((decl, idx)) = is_perform(db, head, ctx) else {
        return false;
    };
    let Some(arm) = ctx.arms.get(&(decl, idx)).cloned() else {
        return false;
    };
    !ctx.abortive.contains(&(decl, idx))
        && !is_tail_resumptive_arm(db, arm.body)
        && peel_resume_from_arm_body(db, arm.body).is_none()
        && !arm_partially_resumes(db, arm.body)
        // EXACT-MATCH the refold's ONE-SHOT/foreign conjunct (reduce_handle, the two-hole block): the refold
        // only serves a MULTI-shot arm when the continuation reaches NO foreign perform (a one-shot arm is
        // always fine — it splices `C` once). Without this, `body_served_by_oneshot_refold` was STRICTLY
        // LOOSER than the refold: for a multi-shot arm reaching a foreign perform it returned true (Site-5 5b
        // stood down "refold serves it") while the refold DECLINED → NEITHER transform ran → the lost-advance
        // ao10 exists to prevent could recur for that shape (github-liaison/Copilot #2147 review, source-
        // verified). Adding it makes the shared predicate EXACTLY the refold's gate, so the `#cv`-lift and the
        // refold can never both stand down. `body` is the whole conditional the lift/refold sees (same node
        // the refold's `body_reaches_foreign_perform(body, ctx)` inspects).
        && (count_resumes(db, arm.body) == 1 || !body_reaches_foreign_perform(db, body, ctx))
}

/// Lift a branch-performing conditional out of its strict continuation into TAIL position, one level per
/// pass, to a fixpoint (see `hoist_resumptive_once` for the per-site transforms). Called from the perform
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

/// Whether `node` is a BLOCK (`let`/`do`) whose TAIL VALUE — through nested `let`/`do` wrappers — is a
/// branch-performing conditional that Site 4's hoist does NOT reach. Site 4 (`conditional_branch_performs`)
/// only lifts a conditional that is DIRECTLY the `let`-init value; a conditional wrapped in a block
/// (`(let ((v (let ((b true)) (if b (E.op) x)))) cont)`) is opaque to it, so the block's exit state falls
/// back to the block-ENTRY state and the branch perform's advance is DROPPED at the block boundary — a
/// silent wrong-value (adv-69, HIGH: `+ (* 10 v) (E.op)` reads the stale pre-branch state). Detecting this
/// residual shape (AFTER the hoist ran, so a liftable direct-init case is already in tail position) lets
/// `reduce_handle` DECLINE cleanly (honest Todo) rather than fold a wrong value. Peels `let`/`do` block
/// wrappers to reach the tail conditional. (The `let` arm peels its BODY regardless of whether the binding
/// PERFORMS — the `..` discards the init: a performing binding is threaded by the ordinary `let` arm, so it
/// does not itself trigger this decline, but this scanner does not need to distinguish it — declining on the
/// tail conditional is sound either way; a performing binding just means the peel keeps going to the body.)
/// Peel nested PURE `let` wrappers off `node`, returning `(collected wrapper binding-pairs, innermost tail)`
/// — the through-block commuting-conversion peel for adv-69 Site 6. `(let (w0…) (let (w1…) tail))` yields
/// `([w0…, w1…], tail)`. Returns `None` if `node` is not a `let` OR any wrapper binding INIT PERFORMS (an
/// effectful wrapper binding cannot be floated earlier in the enclosing `let` — reordering it past a later
/// perform would change effect order/count; leave those to the ordinary `let` thread arm / a later increment).
/// The peel stops at the first non-`let` tail; the caller checks that tail is a branch-performing conditional.
/// A pure wrapper binding is safe to float because the enclosing `let`'s bindings evaluate sequentially and a
/// PURE init commutes freely earlier in that sequence (it reads only already-bound names, which stay in order).
fn peel_pure_let_wrapper(db: &mut Db, node: StructId) -> Option<(Vec<StructId>, StructId)> {
    let mut collected: Vec<StructId> = Vec::new();
    let mut cur = node;
    while let Some(form) = db.ast.as_form(cur, "let").map(|t| t.to_vec()) {
        if form.len() != 2 {
            break;
        }
        let Struct::List(pairs) = db.ast.get(form[0]).clone() else {
            break;
        };
        // Every wrapper binding init must be PURE (no perform of ANY effect) to float it out safely.
        for &p in &pairs {
            match db.ast.get(p).clone() {
                Struct::List(pkv) if pkv.len() == 2 => {
                    if reaches_any_perform(db, pkv[1]) {
                        return None;
                    }
                }
                _ => return None, // malformed binding pair — do not peel
            }
        }
        collected.extend(pairs);
        cur = form[1];
    }
    if collected.is_empty() {
        return None; // `node` was not a `let` wrapper at all
    }
    Some((collected, cur))
}

fn block_wrapped_branch_performs(db: &mut Db, node: StructId, ctx: &HandlerCtx) -> bool {
    match resolved_of(db, node) {
        // A `let` block: its value is the body; recurse through it. (A performing binding is threaded by
        // the ordinary `let` arm — this drop only bites when the wrapper's BODY is the branch conditional.)
        Resolved::Let { body, .. } => {
            conditional_branch_performs(db, body, ctx)
                || block_wrapped_branch_performs(db, body, ctx)
        }
        // A `do` block resolves to a `Ref` at its last form (its value); recurse through it.
        Resolved::Ref { value } => {
            conditional_branch_performs(db, value, ctx)
                || block_wrapped_branch_performs(db, value, ctx)
        }
        _ => false,
    }
}

/// adv-69 **a3 sub-face** (breaker `probe-a3`, block-outstate battery). A block-wrapped branch-performing
/// conditional in a NESTED handle's arm RESUME-VALUE position, performing the OUTER (this) handler's
/// discharged op: `(handle St x ((get …)) (handle Up 0 ((ask (u) t (resume (let ((b true)) (if b (St.get)
/// 99)) t))) …))`. The outer `St` handler threads its state through the inner `Up` handle, but the block
/// boundary INSIDE the inner arm's resume-VALUE drops the outer `St.get`'s state advance (runs 33, correct
/// 34) — the same class of block-boundary out-state drop as the let-init face, but at a DIFFERENT position
/// (`Resume{value}`) that the let-init scanner deliberately does not reach (it stops at a nested `Handle`).
/// Detect a `Resume` whose VALUE is a branch-performing conditional performing THIS handler's op — whether
/// DIRECT (`(resume (if b (St.get) 99) t)`) or BLOCK-WRAPPED (`(resume (let ((b true)) (if b (St.get) 99))
/// t)`) — so `reduce_handle` declines cleanly (honest Todo) instead of folding the dropped-advance wrong
/// value. BOTH forms drop the advance at this position (verified: dropping the direct-conditional disjunct
/// makes `(resume (if true (St.get) 99) t)` miscompile to 33, not fold to 34) — unlike the let-init face,
/// where a DIRECT init is lifted by Site 4 and only the block-wrapped residue remains; a `resume`-value is
/// never hoisted (it lives in a nested handle's arm the outer reduction doesn't rewrite), so a direct
/// conditional there is a genuine miscompile too. PRECISELY POSITIONAL — keyed on the `Resume{value}` slot,
/// NOT a position-agnostic branch-perform scan (that over-declines 5 working cases where the perform sits in
/// a threaded position the fold DOES serve). The through-block fold that flips a3 → PASS is the same deferred
/// commuting conversion as the let-init face. Related:
/// [[adv69-block-wrapped-branch-perform-drops-state-advance-at-block-boundary]].
fn body_has_nested_arm_resume_value_block_wrapped_branch_perform(
    db: &mut Db,
    node: StructId,
    ctx: &HandlerCtx,
) -> bool {
    // A `Resume` whose VALUE is a branch perform of THIS handler's op — direct (`conditional_branch_performs`)
    // OR block-wrapped (`block_wrapped_branch_performs`) — is the a3 drop; both forms revert the out-state at
    // this position. (A `Resume` reached while scanning the outer handle's BODY belongs to a nested handle's
    // arm — the outer body itself has no bare resume; the outer handler's own arms are not in `body`.)
    if let Resolved::Resume { value, .. } = resolved_of(db, node)
        && (conditional_branch_performs(db, value, ctx)
            || block_wrapped_branch_performs(db, value, ctx))
    {
        return true;
    }
    match db.ast.get(node).clone() {
        Struct::List(children) => children
            .iter()
            .any(|&c| body_has_nested_arm_resume_value_block_wrapped_branch_perform(db, c, ctx)),
        Struct::Atom(_) => false,
    }
}

/// Scan `node` for a `let` whose ANY binding init is a BLOCK-WRAPPED branch-performing conditional (the
/// adv-69 shape the hoist cannot lift). Walks the whole subtree via `child_ids` — the miscompiling `let`
/// may sit anywhere the fold would thread. Returns true iff such a `let`-init exists, so `reduce_handle`
/// declines cleanly rather than folding the dropped-advance wrong value.
fn body_has_block_wrapped_let_init_branch_perform(
    db: &mut Db,
    node: StructId,
    ctx: &HandlerCtx,
) -> bool {
    // A NESTED handle: its ARMS belong to THAT inner handler's reduction (a block-wrapped perform in an arm
    // resume-value is the a3 guard's territory), so don't scan them here. But its INIT and BODY both run UNDER
    // this (outer) handler — a block-wrapped branch perform of THIS handler's OUTER op in a `let`-init in the
    // inner body OR the inner handle's INIT (the init is evaluated as part of the handle expression, in the
    // outer extent — `eval.rs` passes `init` to `reduce_handle` alongside `body`) drops the outer advance
    // exactly like the top-level face (the intervening inner handle does not rewrite an outer-effect perform),
    // and the outer reduction's scan previously stopped dead at the inner `Handle` and MISSED it (a silent
    // 33-vs-34 miscompile, v-effects self-probe 2026-08-04; the INIT position was the a4-init sub-face,
    // liaison/Copilot on merged #1933). Descend into BOTH the inner init and body, keeping THIS outer `ctx`:
    // `block_wrapped_branch_performs` is ctx-keyed, so it fires only on a perform of the OUTER discharged op —
    // an inner-effect perform never matches, so this cannot over-decline the inner handler's own shapes.
    if let Resolved::Handle { init, body, .. } = resolved_of(db, node) {
        // The inner handle's INIT may ITSELF be a block-wrapped branch perform (`(handle B (let ((k true))
        // (if k (A.ga) 9)) …)` — the block IS the init, not a let-binding within it), so check the init node
        // directly too; then recurse into both init and body for a `let`-init nested anywhere inside them.
        return block_wrapped_branch_performs(db, init, ctx)
            || body_has_block_wrapped_let_init_branch_perform(db, init, ctx)
            || body_has_block_wrapped_let_init_branch_perform(db, body, ctx);
    }
    // A `let` at THIS node: check each init for the block-wrapped branch-performing shape.
    if let Some(parts) = db.ast.as_form(node, "let").map(<[_]>::to_vec)
        && parts.len() == 2
        && let Struct::List(pairs) = db.ast.get(parts[0]).clone()
    {
        for pair in pairs {
            if let Struct::List(kv) = db.ast.get(pair).clone()
                && kv.len() == 2
                && block_wrapped_branch_performs(db, kv[1], ctx)
            {
                return true;
            }
        }
    }
    // Recurse structurally — the miscompiling `let` may be nested anywhere in the body.
    match db.ast.get(node).clone() {
        Struct::List(children) => children
            .iter()
            .any(|&c| body_has_block_wrapped_let_init_branch_perform(db, c, ctx)),
        Struct::Atom(_) => false,
    }
}

/// adv-69 **g3 + c3 sub-faces** (breaker block-outstate battery, post-floor 39-probe run). The SAME block-
/// boundary out-state drop as the let-init floor, but at two more consuming positions the hoist does NOT
/// reach:
///   * **g3 — MATCH-SCRUTINEE**: `(match (let ((b true)) (if b (St.get) 99)) (v (+ (* 10 v) (St.get))))` — a
///     block-wrapped branch perform in the scrutinee. Site 5 lifts a scrutinee that is DIRECTLY a branch-
///     performing conditional, but a block wrapper is opaque to it, so the scrutinee's out-state reverts to
///     entry (ran 33, correct 34).
///   * **c3 — DO-STATEMENT (non-last, discarded)**: `(do (let ((x true)) (if x (St.put 7) unit)) (+ (* 10
///     (St.get)) x))` — a block-wrapped branch perform as a non-tail `do` item. Site 1 hoists a non-last item
///     that is DIRECTLY a branch-performing conditional, but a block wrapper defeats its `conditional_branch_
///     performs` match, so the statement's `put` advance is dropped (ran 33, correct 73; the minimal twins
///     d2/e1 — a BARE `if` in the statement, or a def-bound cond — hoist fine and PASS).
///
/// Both key on `block_wrapped_branch_performs` (the WRAPPED shape ONLY): a DIRECT `if`/`match` in either
/// position is lifted by Site 1/5 and still folds — so this never over-declines the working hoist paths.
/// Until the through-block fold lands (the deferred commuting conversion), DECLINE these residual shapes so
/// they grade a clean Todo, never the silent wrong value. Related:
/// [[adv69-block-wrapped-branch-perform-drops-state-advance-at-block-boundary]].
fn body_has_block_wrapped_scrutinee_or_statement_branch_perform(
    db: &mut Db,
    node: StructId,
    ctx: &HandlerCtx,
) -> bool {
    // A NESTED handle: its ARMS are the inner handler's concern (a3's `Resume{value}` scanner covers the
    // nested-arm position), but its BODY runs UNDER this (outer) handler — a block-wrapped OUTER-effect perform
    // in a match-scrutinee or non-tail do-statement in the inner body drops the outer advance exactly like the
    // top-level face, and a scan that stopped at the inner `Handle` MISSED it (the a4-class miscompile, but for
    // the scrutinee/do-statement positions instead of the let-init: v-effects self-probe 2026-08-04, 33 vs 73).
    // Descend into the inner body ONLY, keeping THIS outer `ctx` — `block_wrapped_branch_performs` is ctx-keyed,
    // so only an OUTER-effect perform fires (an inner-effect perform never matches → no over-decline of the
    // inner handler's own shapes). Recurses through depth-N nesting via this same descent.
    if let Resolved::Handle { body, .. } = resolved_of(db, node) {
        return body_has_block_wrapped_scrutinee_or_statement_branch_perform(db, body, ctx);
    }
    // g3: a MATCH whose SCRUTINEE is a block-wrapped branch-performing conditional.
    if let Resolved::Match { scrutinee, .. } = resolved_of(db, node)
        && block_wrapped_branch_performs(db, scrutinee, ctx)
    {
        return true;
    }
    // c3: a `do` with a NON-LAST item that is a block-wrapped branch-performing conditional (a discarded
    // statement whose branch performs — its advance is dropped at the block boundary). The LAST item is the
    // do's value (tail) — a block-wrapped conditional there is the let-init/body shape the other scanners /
    // hoist handle; only the non-tail statement position is this face.
    if let Some(items) = db.ast.as_form(node, "do").map(<[_]>::to_vec)
        && items.len() >= 2
    {
        for &it in &items[..items.len() - 1] {
            if block_wrapped_branch_performs(db, it, ctx) {
                return true;
            }
        }
    }
    // Recurse structurally — the miscompiling position may be nested anywhere in the body.
    match db.ast.get(node).clone() {
        Struct::List(children) => children
            .iter()
            .any(|&c| body_has_block_wrapped_scrutinee_or_statement_branch_perform(db, c, ctx)),
        Struct::Atom(_) => false,
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
            let if_head = db.push_name("if");
            db.push_list(vec![if_head, cond, nt, ne])
        }
        Resolved::Match { scrutinee, arms } => {
            let match_head = db.push_name("match");
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
        let if_head = db.push_name("if");
        let (then_, else_) = if is_and {
            let false_lit = db.push_atom(Leaf::Bool(false));
            (rhs, false_lit) // (and lhs rhs) ≡ (if lhs rhs false)
        } else {
            let true_lit = db.push_atom(Leaf::Bool(true));
            (true_lit, rhs) // (or lhs rhs) ≡ (if lhs true rhs)
        };
        return Some(db.push_list(vec![if_head, lhs, then_, else_]));
    }
    // Site 4: a `let` whose BINDING INIT is a branch-performing conditional — the shape an inlined helper
    // leaves (`reduce_applied_lambdas` turns `(let ((a (demand 5 25))) cont)` into `(let ((a (match (Db.get
    // k) … (do (Db.put …) c)))) cont)`). The `let`-init threading (the `let` arm in `thread_bounded`) threads
    // the init and takes its out-state as the post-INIT state — but for a branch-performing conditional init
    // that out-state is the post-SCRUTINEE state (the `Match`/`If` thread arms drop each branch's advance),
    // so the branch's `put` advance never reaches the following bindings / body: a later `(Db.get k)` reads
    // the stale pre-branch state (the helper-call out-state silent miscompile). Distribute the CONTINUATION
    // (this binder + every following binding + the body) INTO each branch, keeping the PRECEDING bindings as
    // an outer `let` prefix so the conditional's own condition/scrutinee still sees them:
    //   `(let (p… (nk (if c t e)) r…) body)`
    //     ≡ `(let (p…) (if c (let ((nk t) r…) body) (let ((nk e) r…) body)))`
    // Now the conditional is in TAIL position of the outer `let` and each branch binds `nk` to the branch
    // value and threads the continuation UNDER the branch-advanced state (the per-branch `let` threading
    // carries the advance). Sound: the preceding inits stay in place and in order (a performing one is still
    // threaded once, before the condition — same order as the original); the condition/scrutinee is evaluated
    // exactly once (it is the single distributed `if`/`match` head); the continuation is duplicated across the
    // branches but only one runs at runtime, so every effect in it happens exactly once, in order. Only the
    // FIRST branch-performing init is distributed per pass (the fixpoint loop lifts a second one next pass).
    if let Some(pairs_form) = db.ast.as_form(node, "let").map(|t| t.to_vec())
        && pairs_form.len() == 2
        && let Struct::List(pairs) = db.ast.get(pairs_form[0]).clone()
    {
        let body_occ = pairs_form[1];
        for (k, &pair) in pairs.iter().enumerate() {
            let Struct::List(kv) = db.ast.get(pair).clone() else {
                continue;
            };
            if kv.len() == 2 && conditional_branch_performs(db, kv[1], ctx) {
                let binder = kv[0];
                let init = kv[1];
                // The continuation = this binder (bound to the branch value) + the remaining bindings +
                // the body, rebuilt as a `let` around each branch.
                let rest_pairs: Vec<StructId> = pairs[k + 1..].to_vec();
                // `map_conditional_branches` calls `wrap` ONCE PER BRANCH (both `if` arms, or every `match`
                // arm), so the continuation — the binder, the remaining bindings, and the body — is spliced
                // into EACH branch. The arena is single-parent (`push_list` overwrites `parent`/`child_ix`),
                // so reusing the SAME `binder`/`rest_pairs`/`body_occ` nodes across branches would parent
                // them to whichever branch is built LAST, orphaning the earlier branch's copy — the "one leaf
                // under two parents" class `deep_fresh_copy` guards (same as the If/Match thread arms copying
                // their state-refs per branch). DEFENSIVE: today this sharing is latent-benign — the `thread`
                // pass runs after this hoist and re-processes each branch (rebuilding via `push_list`/
                // `copy_pure`), re-freshing the shared nodes before resolution matters (verified: the fold
                // yields the same value with or without this copy). But relying on a later pass to launder a
                // structurally-invalid shared subtree is fragile, so give each branch its OWN fresh copy of
                // every continuation node here — making the "continuation duplicated across branches" comment
                // structurally true and robust to a future thread-pass change. (Copilot flag on PR #534.)
                let wrap = |db: &mut Db, branch: StructId| -> StructId {
                    let let_head = db.push_name("let");
                    let binder_c = deep_fresh_copy(db, binder);
                    let this_pair = db.push_list(vec![binder_c, branch]);
                    let mut bindings = vec![this_pair];
                    for &rp in &rest_pairs {
                        let rp_c = deep_fresh_copy(db, rp);
                        bindings.push(rp_c);
                    }
                    let bindings_list = db.push_list(bindings);
                    let body_c = deep_fresh_copy(db, body_occ);
                    db.push_list(vec![let_head, bindings_list, body_c])
                };
                let distributed = map_conditional_branches(db, init, wrap);
                // Preceding bindings (pure or performing — both stay in place, in order) become the outer
                // `let` prefix. With none, the distributed conditional IS the whole value.
                if k == 0 {
                    return Some(distributed);
                }
                let let_head = db.push_name("let");
                let prefix_bindings = db.push_list(pairs[..k].to_vec());
                return Some(db.push_list(vec![let_head, prefix_bindings, distributed]));
            }
        }
    }
    // Site 5: an `if`/`match` whose CONDITION/SCRUTINEE is ITSELF a branch-performing conditional — the shape
    // a connective in the condition leaves after Site 3 desugars it: `(if (and b (> (St.tick) 0)) t e)` →
    // Site 3 → `(if (if b (> (St.tick) 0) false) t e)`, an outer `if` whose CONDITION is a branch-performing
    // `if`. The `If`/`Match` thread arms take the post-CONDITION state as the whole conditional's out-state
    // (they do not observe a per-branch advance in the CONDITION), so the condition's `tick` advance is
    // DROPPED — the outer branches thread against the pre-condition seed and a branch `(St.tick)` reads the
    // stale state (the connective-in-scrutinee silent miscompile: `→ 1` where `→ 2` is correct). Bind the
    // performing condition/scrutinee to a fresh `let` so the outer conditional reads a plain scalar and the
    // performing conditional becomes a `let`-INIT — exactly the shape Site 4 distributes (which threads each
    // branch's advance through the continuation). One binding away is the WORKING let-bound form (verified:
    // a hand let-bound connective threads correctly). The bound name is `#cv{…}` (a `#`-prefixed name is
    // unbindable in source — CDZ0210 — so it can't be captured). Only fires
    // when the condition/scrutinee itself performs in a branch (a pure condition needs no lift).
    {
        let cond_scrut = match resolved_of(db, node) {
            Resolved::If { cond, .. } => Some(cond),
            Resolved::Match { scrutinee, .. } => Some(scrutinee),
            _ => None,
        };
        // 5a: the condition/scrutinee is itself a branch-performing conditional (always lift).
        // 5b (ao10): the condition performs AND a branch performs AND the one-shot REFOLD does NOT serve this
        // body. SPECIFICITY ORDERING (concierge-steered 2026-08-04): the E5 refold is the MORE-SPECIFIC
        // transform for a performing condition (it re-reduces the leading hole, constant-folding the cond to
        // select a branch), and wins where it applies; the `#cv`-lift is the general branch-advance-
        // preservation fallback. Gate 5b on `body_served_by_oneshot_refold` being FALSE (the shared
        // servability predicate, so the two transforms can never both fire): a refold-served body like `(if (<
        // (Amb.flip) 5) (+ 1 (Amb.flip)) 0)` (non-tail arm `(+ 1 (resume 10 s))`) → refold serves → DON'T lift
        // → 1/13; ao10's `(if (> (A.tick) 5) (do (A.tick) 99) 5)` (tail-resumptive outer arm `(resume s (+ s
        // 1))`) → refold declines → LIFT → 111. The DECISIVE separator is `is_tail_resumptive_arm` inside that
        // predicate. Without this gate, 5b `#cv`-lifted the refold cases before the refold ran, breaking their
        // leading-hole logic (4 lib-test regression). Pass the WHOLE `node` (the conditional) as the body — the
        // refold checks the leading hole through it.
        let refold_serves = body_served_by_oneshot_refold(db, node, ctx);
        if let Some(cs) = cond_scrut
            && (conditional_branch_performs(db, cs, ctx)
                || (subtree_performs(db, cs, ctx)
                    && conditional_branch_performs(db, node, ctx)
                    && !refold_serves))
        {
            // `(if CS t e)` ≡ `(let ((#cv CS)) (if #cv t e))`; `(match CS arms)` ≡ `(let ((#cv CS)) (match
            // #cv arms))`. Rebuild the conditional with the condition/scrutinee replaced by a fresh `#cv`
            // reference, wrapped in a `let` binding `#cv` to the original (performing) condition/scrutinee.
            // The next fixpoint pass sees the `let`-init branch-performing conditional and Site 4 distributes
            // it. The bound name is a fresh `#cv{node}` — unbindable in source (a `#`-name is CDZ0210 in binder
            // position) so it cannot be captured by a user binder.
            let cv_name = format!("#cv{}", node.0);
            let cv_binder = db.push_name(&cv_name);
            let cv_ref = db.push_name(&cv_name);
            let rebuilt = match resolved_of(db, node) {
                Resolved::If { then_, else_, .. } => {
                    let if_head = db.push_name("if");
                    db.push_list(vec![if_head, cv_ref, then_, else_])
                }
                Resolved::Match { arms, .. } => {
                    let match_head = db.push_name("match");
                    let mut children = vec![match_head, cv_ref];
                    for (pat, body) in arms {
                        children.push(db.push_list(vec![pat, body]));
                    }
                    db.push_list(children)
                }
                _ => unreachable!("cond_scrut is Some only for If/Match"),
            };
            let pair = db.push_list(vec![cv_binder, cs]);
            let bindings = db.push_list(vec![pair]);
            let let_head = db.push_name("let");
            return Some(db.push_list(vec![let_head, bindings, rebuilt]));
        }
    }
    // Site 6 (adv-69 through-block commuting conversion): a `let` binding whose INIT is a branch-performing
    // conditional WRAPPED in pure `let` blocks — `(let (… (v (let (w…) (if c (E.op) x))) …) body)`. Site 4
    // only fires on a DIRECT conditional init (`conditional_branch_performs(kv[1])` is false when `kv[1]` is a
    // `let`, not an `if`/`match`), so a block-wrapped init reached `block_wrapped_branch_performs`'s safe-floor
    // decline instead of folding — the branch's state advance dropped at the block boundary (adv-69: 33 not
    // 34). FLOAT the inner pure wrapper bindings `w…` OUT into the enclosing `let`, leaving the conditional as
    // `v`'s DIRECT init: `(let (… w… (v (if c (E.op) x)) …) body)`. This is a commuting conversion — sound
    // because the wrapper bindings are PURE (no perform: `wrapper_is_pure` below), so hoisting them earlier in
    // the same `let`'s (sequential, non-recursive) binding list changes no evaluation order or effect count;
    // `v`'s init now evaluates the conditional against the same values. Site 4 then fires on the now-direct
    // conditional init on the next fixpoint pass (threading each branch's advance through the continuation).
    // Only the wrapper is peeled here (one binding, first found); Site 4 does the actual branch distribution.
    // GATED: only when the inner conditional performs THIS handler's discharged op in a branch (the adv-69
    // trigger) — a pure-wrapped pure conditional needs no lift. A `do`-wrapper is left to Site 1 (it peels
    // do-statements); this handles the `let`-wrapper commuting conversion the direct-init Site 4 can't reach.
    if let Some(let_form) = db.ast.as_form(node, "let").map(|t| t.to_vec())
        && let_form.len() == 2
        && let Struct::List(outer_pairs) = db.ast.get(let_form[0]).clone()
    {
        let outer_body = let_form[1];
        for (k, &pair) in outer_pairs.iter().enumerate() {
            let Struct::List(kv) = db.ast.get(pair).clone() else {
                continue;
            };
            if kv.len() != 2 {
                continue;
            }
            // The init must be a `let`-block whose (through-nested-pure-lets) tail is a branch-performing
            // conditional, with every peeled wrapper binding PURE.
            if let Some((wrapper_pairs, inner_cond)) = peel_pure_let_wrapper(db, kv[1])
                && !wrapper_pairs.is_empty()
                && conditional_branch_performs(db, inner_cond, ctx)
            {
                // Rebuild: outer prefix `p…` + the floated wrapper bindings `w…` + `(v inner_cond)` + the
                // remaining outer bindings `r…`, under the same body. Fresh-copy the floated + rebound nodes
                // (single-parent arena) so nothing is shared across the rebuild.
                let mut new_pairs: Vec<StructId> =
                    Vec::with_capacity(outer_pairs.len() + wrapper_pairs.len());
                for &p in &outer_pairs[..k] {
                    new_pairs.push(deep_fresh_copy(db, p));
                }
                for &wp in &wrapper_pairs {
                    new_pairs.push(deep_fresh_copy(db, wp));
                }
                let binder_c = deep_fresh_copy(db, kv[0]);
                let cond_c = deep_fresh_copy(db, inner_cond);
                let v_pair = db.push_list(vec![binder_c, cond_c]);
                new_pairs.push(v_pair);
                for &p in &outer_pairs[k + 1..] {
                    new_pairs.push(deep_fresh_copy(db, p));
                }
                let new_bindings = db.push_list(new_pairs);
                let body_c = deep_fresh_copy(db, outer_body);
                let let_head = db.push_name("let");
                return Some(db.push_list(vec![let_head, new_bindings, body_c]));
            }
        }
    }
    // A NESTED `handle-internal`'s ARM LIST is opaque to THIS outer hoist WHEN no arm reaches an operation
    // the OUTER handler discharges — those arms are the INNER handler's concern, folded under ITS ctx by the
    // inside-out `reduce_handle(inner)`. Recursing into such an arm treated its shape `((. B op) (p) s (match
    // …))` as an ordinary strict application `(op a0 a1 <match>)` and let Site 2 distribute the arm-op HEAD
    // into the `match` branches — inverting the arm so the `match` sat in the op-slot, `effect_op_of` → None,
    // the inner op-map went EMPTY, and the nested fold declined (nv1f). BUT an inner arm that itself performs
    // the OUTER effect — the arg-arm `(cut (b) t (do … (set-ty …) …))` where `set-ty` is the OUTER DbState
    // handler's own op demanded across `compute-type` — MUST stay in this recursion, because the outer hoist
    // legitimately needs to lift its branch-performing conditional (skipping it re-broke the arg-arm:
    // wasm-unreachable, the ec113e84e over-scope). So skip the arms child ONLY when EVERY arm body is free of
    // an outer-ctx perform (`subtree_performs(.., ctx)` — ctx here is the OUTER handler, and that predicate is
    // exactly "reaches an op THIS ctx discharges"). nv1f's inner B arm performs only B (inner) → skipped; the
    // arg-arm's set-ty reaches the outer op → recursed. (Mirrors `thread_bounded`, which treats a nested
    // handle as opaque; this is the finer-grained rule the hoist needs since an inner arm can re-perform the
    // outer effect.)
    let arms_to_skip = if db.ast.head_name(node) == Some(HANDLE_INTERNAL) {
        db.ast
            .as_form(node, HANDLE_INTERNAL)
            .and_then(|t| t.get(1).copied())
            .filter(|&arms| {
                // arms = `((op (params) state body)…)`; each arm's BODY is its last child. Skip the arms
                // list only if NONE of the arm bodies reaches an outer-ctx perform.
                let arm_bodies: Vec<StructId> = match db.ast.get(arms) {
                    Struct::List(arm_nodes) => arm_nodes
                        .iter()
                        .filter_map(|&a| match db.ast.get(a) {
                            Struct::List(parts) => parts.last().copied(),
                            Struct::Atom(_) => None,
                        })
                        .collect(),
                    Struct::Atom(_) => Vec::new(),
                };
                // Use `subtree_reaches_discharged_op` (a perform of an op the OUTER ctx discharges), NOT
                // `subtree_performs`: the latter treats a bare `resume` as effectful, so an inner arm's own
                // `(resume …)` would spuriously read as "performs the outer effect" and defeat the skip. We
                // want ONLY "reaches a perform of the OUTER handler's op" — the arg-arm's `set-ty`, not B's
                // inner resume.
                !arm_bodies
                    .iter()
                    .any(|&b| subtree_reaches_discharged_op(db, b, ctx))
            })
    } else {
        None
    };
    // Not a site here — recurse into children, rebuilding with the FIRST rewritten child (so a
    // conditional nested inside a `let` init / branch / arm is lifted within that sub-position, then the
    // enclosing pass lifts it further if needed).
    if let Struct::List(children) = db.ast.get(node).clone() {
        for (k, &c) in children.iter().enumerate() {
            if Some(c) == arms_to_skip {
                continue; // an inner handler's outer-effect-free arms fold under their own reduce_handle
            }
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
    // An `if`: the CONDITION evaluates BEFORE branching, so an abort there abandons the whole computation
    // regardless of which branch would run. When the `if` is on a TAIL path (`tail=true` — it is the handle
    // body, or the tail of a branch the per-branch capture intercepts), that condition-abort IS capturable:
    // `thread`'s `if` arm threads the condition first, the abort sets the cell, and `reduce_handle` (body) /
    // `thread_branch_local_abort` (branch) takes the abort value — the enclosing computation is dead, so
    // nothing is dropped. This is what lets a connective-in-condition abort fold after the hoist distributes
    // the outer `if` through it (`(if (and b (Bail 7)) 100 200)` → `(if b (if (Bail 7) 100 200) …)`, the
    // condition-abort in the tail branch). A NON-tail `if` (`tail=false` — an operand `(+ 1 (if (Bail 7) …))`
    // the hoist could not lift) keeps the condition non-capturable → flagged. So the condition inherits the
    // `if`'s own `tail`. Each BRANCH is a conditional position (`under_cond=true`, `tail` carried).
    if let Resolved::If { cond, then_, else_ } = resolved_of(db, node) {
        return body_has_unsound_abortive_perform(db, cond, ctx, tail, under_cond)
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
                    // finding #11-B (oamin4/oa3): a def-boundary conditional abort with a foreign-performing
                    // argument — `(let ((a (unwrap (E.fetch) tag))) …)` where `unwrap` aborts in a match arm and
                    // `E.fetch` performs an outer op. The abort must home to the handler boundary, but the fold
                    // captures it per-branch and threads the abort value into the let continuation (a silent
                    // wrong value). It is opaque to the `if`-only hoist AND to the ordinary cross-fn guard (which
                    // walks `if`/`and`, not `match` arms). Flag it here so `reduce_handle` declines to the safe
                    // floor. GATED on the foreign argument so the PURE-arg controls (oamin1/oamin5) still fold.
                    if init_is_foreign_arg_match_abort_call(db, kv[1], ctx) {
                        return true;
                    }
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

/// Propagate a handler's SOLVED state collection type back onto the initial-state seed subtree, so an empty
/// collection built at the seed (`Map.empty`, an empty `(set)`/`(list)`) whose key/value/element is fixed
/// only in a LATER handler arm reflects that solved type at its OWN construction node — the cross-arm
/// key-propagation the per-node `type_of` cannot reach (an empty collection bottoms at open vars; the arm
/// that inserts a tuple key is a different subtree).
///
/// The slot type [`state_ty_of_arms`] computes is NOT reused here: its `tail_resume_next_state_of` only sees
/// a DIRECT `(resume v next)` arm body, but the common arm wraps the resume in a `(match st ((tuple s m)
/// (resume … next)))`, so the joined slot leaves the map key/value OPEN. We derive the solved type
/// independently: join the seed's own `type_of` with the `type_of` of EVERY resume next-state reached by
/// walking each arm body (stopping at a nested `handle`, whose resumes belong to the inner handler). The
/// `rec` arm's `(Map.insert m (tuple s …) …)` next-state then reflects `Map((Int64,Int64), Int64)`, fixing
/// the seed's open key. Then walk the seed in parallel with the joined type, filling the memo of an open-var
/// collection node with the solved collection type; recurses through a tuple position-wise (the common
/// `(tuple counter Map.empty)` state).
///
/// GENERIC over the collection's element vars (Map key AND value, Set element, List element) — one walk
/// clears all four (breaker's tk/tv/sk/lv family), not a Map-key special case.
///
/// NARROW + SAFE, mirroring [`crate::infer::ground_open_var_arms_to_collection`]:
/// - only writes when the joined type at that position is a collection with NO free var in its
///   key/value/element (a genuinely-solved type, never one guess feeding another), AND the node's current
///   `type_of` is that same collection KIND but OPEN (`is_open` — a free var like `Map.empty`'s `Map(Var,Var)`,
///   OR `Ty::Any` like an empty `(list)`/`(set)` literal's element). A node already solved, or a solved slot
///   still open on both sides, is left untouched (a no-op — byte-identical emit).
/// - never runs mid-scheme-solve (`solving_schemes` non-empty): a provisional fixpoint var must stay unfrozen.
///
/// The container kind and arity are unchanged, so the machine slot (an i32 heap handle) is identical; only the
/// spelled key/value/element type the rust/rust-async backend annotates is refined from open to solved.
fn refine_init_collection_ty(db: &mut Db, init: StructId, arms: &[HandleArm]) {
    if !db.solving_schemes.is_empty() {
        return;
    }
    // The solved state type: the seed joined with every resume next-state reached in the arm bodies.
    let mut solved = crate::infer::type_of(db, init);
    let mut next_states: Vec<StructId> = Vec::new();
    for arm in arms {
        collect_resume_next_states(db, arm.body, &mut next_states);
    }
    for next in next_states {
        let nt = crate::infer::type_of(db, next);
        solved = solved.join(&nt);
    }
    fill_open_collection_from_solved(db, init, &solved);
}

/// Collect the NEXT-STATE occurrence of every `(resume value next)` reachable in `node`, descending the
/// arena structure but NOT into a nested `handle` (its resumes target the inner handler's state). Used by
/// [`refine_init_collection_ty`] to see resumes buried under a `(match st …)` arm wrapper.
fn collect_resume_next_states(db: &mut Db, node: StructId, out: &mut Vec<StructId>) {
    if let Resolved::Resume { next_state, .. } = resolved_of(db, node) {
        out.push(next_state);
    }
    if matches!(resolved_of(db, node), Resolved::Handle { .. }) {
        return;
    }
    if let Struct::List(children) = db.ast.get(node).clone() {
        for c in children.iter() {
            collect_resume_next_states(db, *c, out);
        }
    }
}

/// Fill the type memo of an open-var collection node under `node` with the corresponding fully-determined
/// collection type from `solved`, recursing through a tuple position-wise. See [`refine_init_collection_ty`]
/// for the safety contract (only refines an open node whose kind matches a free-var-free solved collection).
fn fill_open_collection_from_solved(db: &mut Db, node: StructId, solved: &crate::ty::Ty) {
    use crate::ty::Ty;
    match solved {
        // Recurse through a tuple state position-wise. `positional_value_nodes` reads the element occurrences
        // from BOTH the `Resolved::Tuple` literal and the `tuple`-headed `Apply` spelling (the init seed
        // `(tuple n Map.empty)` resolves as the latter), so a non-tuple node or arity mismatch is skipped.
        Ty::Tuple(solved_elems) => {
            if let Some(elems) =
                crate::infer::positional_value_nodes(db, node, crate::resolved::Prim::TupleNew)
                && elems.len() == solved_elems.len()
            {
                for (child, sub) in elems.into_iter().zip(solved_elems.iter()) {
                    fill_open_collection_from_solved(db, child, sub);
                }
            }
        }
        Ty::Map(k, v) if !k.has_free_var() && !v.has_free_var() => {
            if matches!(crate::infer::type_of(db, node), Ty::Map(nk, nv) if is_open(&nk) || is_open(&nv))
            {
                db.types.fill(node, solved.clone());
            }
        }
        Ty::Set(e) if !e.has_free_var() => {
            if matches!(crate::infer::type_of(db, node), Ty::Set(ne) if is_open(&ne)) {
                db.types.fill(node, solved.clone());
            }
        }
        Ty::List(e) if !e.has_free_var() => {
            if matches!(crate::infer::type_of(db, node), Ty::List(ne) if is_open(&ne)) {
                db.types.fill(node, solved.clone());
            }
        }
        _ => {}
    }
}

/// Whether a collection element/key/value type is UNSOLVED for emit purposes — a free `Ty::Var` (an
/// unconstrained empty `Map.empty` → `Map(Var,Var)`) OR `Ty::Any` (an empty `(list)`/`(set)` literal bottoms
/// its element at `Any`, not a var). Both leave the rust/rust-async backend nothing concrete to annotate, so
/// both are candidates for refinement from a solved sibling-arm type.
fn is_open(t: &crate::ty::Ty) -> bool {
    matches!(t, crate::ty::Ty::Any) || t.has_free_var()
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
    if !value_ty.agrees_with(&result) {
        return false;
    }
    // WIDTH GUARD: `agrees_with` treats two fixed-width ints of DIFFERENT widths as compatible (UInt8
    // "agrees with" Int64), but the fold substitutes the resume value into the op's RESULT position with NO
    // coercion. A CONCRETELY-typed narrow resume value (`(resume x s)` with x:UInt8, op result Int64) would
    // emit an i32 where i64 is expected → invalid wasm; DECLINE cleanly instead (the safe floor; a coercion
    // fold is a later increment). The STATE-typed resume value (`(resume s …)`, value types `Any`) is caught
    // by the state-slot width guard in `reduce_handle` (this catches the value-is-a-narrow-EXPRESSION twin).
    if let (crate::ty::Ty::Int(v), crate::ty::Ty::Int(r)) = (&value_ty, &result)
        && let (crate::ty::Width::Fixed(vw), crate::ty::Width::Fixed(rw)) = (v.width, r.width)
        && vw != rw
    {
        return false;
    }
    true
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

/// The STATE binder's type for a handler arm — the companion of [`handle_arm_param_ty`] for the arm's
/// element-2 state binder (which is NOT in the params list, so `handle_arm_param_ty` returns `None` for
/// it and it would otherwise fall through to `Ty::Any`). The state's type is fixed by the handle's SEED:
/// a handler folds a state whose type the `init` seed establishes, so a body reference to `s` reads a
/// value of the SEED's type (`capabilities-and-effects.md` §A Handler Threads State). Without this, `s`
/// types `Any` inside an INLINE arm-body expression — a bare `(resume s s)` passed (`Any` agrees with
/// the seed vacuously), but `(+ s s)` defaulted `s:Any` to a generic Int64, missing the Qty-aware arith
/// arm → a spurious CDZ0201 next-state/seed mismatch on a well-typed Qty-stateful handler. Navigates
/// `binder(=arm's element 2) → arm → arms-list → handle-internal → INIT`, where INIT is the FIRST element
/// of `as_form(handle, HANDLE_INTERNAL)`'s tail (that accessor strips the head, so the `(handle-internal
/// INIT ARMS BODY)` seed is `.first()`/index 0 of the tail, not index 1 — index 1 is the arms-list), and
/// types the seed. Returns `None` for an UNDETERMINED seed (`Any`/`Var` — a recursive handler mid-solve),
/// preserving the prior `Any` fallthrough so no not-yet-solved state is falsely pinned.
pub fn handle_arm_state_ty(db: &mut Db, binder: StructId) -> Option<crate::ty::Ty> {
    // `binder` must be the arm's element-2 state binder: its DIRECT parent is the arm `(op (params…)
    // state body)`, and `binder == parts[2]`.
    let arm = db.parent_of(binder)?;
    if !crate::resolve::is_handle_arm(db, arm) {
        return None;
    }
    let Struct::List(parts) = db.ast.get(arm).clone() else {
        return None;
    };
    if parts.get(2).copied() != Some(binder) {
        return None;
    }
    // The handle: arm → arms-list → handle-internal `(handle-internal INIT ARMS BODY)`. `as_form` strips
    // the head, so its tail is `[INIT, ARMS, BODY]` — INIT is element 0 (element 1 is the arms-list, per
    // `is_handle_arm`'s use of the same accessor).
    let arms_list = db.parent_of(arm)?;
    let handle = db.parent_of(arms_list)?;
    let init = db
        .ast
        .as_form(handle, HANDLE_INTERNAL)
        .and_then(|t| t.first().copied())?;
    let seed_ty = crate::infer::type_of(db, init);
    if matches!(seed_ty, crate::ty::Ty::Any | crate::ty::Ty::Var(_)) {
        return None;
    }
    Some(seed_ty)
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
        let let_head = db.push_name("let");
        let name_atom = db.push_name(&name);
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
    let head = db.push_str("tuple");
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
        // A `(let (inits…) dispatch)` whose body is an `if`/`match` — finding #12 — OR a NESTED `let` chain
        // ending in a dispatch — finding #14 (`race k = let a=(A.next) in let b=(B.next) in (if … (race …) k)`,
        // a two-effect recursion). `thread_returning_tuple` descends this (threading the inits, recursing on the
        // body — which may be another `let`), so the pre-check must mirror it: the inits are on the unconditional
        // strict spine (no self-call may be gated behind a conditional THERE), and the body's own leaves must be
        // threadable (recursing through a nested `let`). Without this a nested `let a in let b in (if …)` fell to
        // the leaf case below and `selfcall_under_conditional` saw the tail self-call under the `if` → rejected,
        // forcing single-return (the trailing-observer miscompile — a later `(A.next)` reads PRE-recursion state).
        _ if db.ast.as_form(body, "let").map(|t| t.len()) == Some(2) && {
            let form = db.ast.as_form(body, "let").unwrap().to_vec();
            matches!(
                resolved_of(db, form[1]),
                Resolved::If { .. } | Resolved::Match { .. }
            ) || db.ast.as_form(form[1], "let").map(|t| t.len()) == Some(2)
                || db.ast.as_form(form[1], "do").is_some()
        } =>
        {
            let form = db.ast.as_form(body, "let").unwrap().to_vec();
            let (bindings_occ, body_occ) = (form[0], form[1]);
            let inits_ok = match db.ast.get(bindings_occ).clone() {
                Struct::List(pairs) => pairs.iter().all(|&pair| match db.ast.get(pair).clone() {
                    Struct::List(kv) if kv.len() == 2 => {
                        !selfcall_under_conditional(db, kv[1], callee_def)
                    }
                    _ => false,
                }),
                _ => false,
            };
            inits_ok && multivalue_leaves_threadable(db, body_occ, callee_def)
        }
        // A `(do stmt… tail)` — the leading stmts are for-effect (each threaded on the unconditional strict
        // spine); the tail carries the recursion. Mirrors `thread_returning_tuple`'s do arm (breaker #14 ra6).
        // The stmts must not gate a self-call behind a conditional, and the tail's leaves must be threadable.
        _ if db
            .ast
            .as_form(body, "do")
            .map(|t| t.len() >= 2)
            .unwrap_or(false) =>
        {
            let items = db.ast.as_form(body, "do").unwrap().to_vec();
            let (tail, stmts) = items.split_last().unwrap();
            let (tail, stmts) = (*tail, stmts.to_vec());
            stmts
                .iter()
                .all(|&st| !selfcall_under_conditional(db, st, callee_def))
                && multivalue_leaves_threadable(db, tail, callee_def)
        }
        _ => !selfcall_under_conditional(db, body, callee_def),
    }
}

/// Whether a RE-ENTRANT call (a self-call to `callee_def` OR a mutual-recursive PARTNER call) in `node`
/// sits UNDER a conditional — the group-aware analogue of `selfcall_under_conditional` for the mutual-SCC
/// multi-value fold. A partner call gated behind a branch has the same unhoistable-temp problem as a
/// gated self-call (the group's multi-value machinery binds a re-entrant call on the unconditional strict
/// spine; a branch-gated one is not covered by v1). Uses `contains_recursive_call` (self + mutual partner)
/// where `selfcall_under_conditional` uses `contains_self_call`.
fn reentrant_call_under_conditional(db: &mut Db, node: StructId, callee_def: usize) -> bool {
    match resolved_of(db, node) {
        Resolved::If { cond, then_, else_ } => {
            reentrant_call_under_conditional(db, cond, callee_def)
                || contains_recursive_call(db, then_, callee_def)
                || contains_recursive_call(db, else_, callee_def)
        }
        Resolved::Match { scrutinee, arms } => {
            reentrant_call_under_conditional(db, scrutinee, callee_def)
                || arms
                    .iter()
                    .any(|&(_, body)| contains_recursive_call(db, body, callee_def))
        }
        Resolved::And { lhs, rhs, .. } => {
            reentrant_call_under_conditional(db, lhs, callee_def)
                || contains_recursive_call(db, rhs, callee_def)
        }
        _ => match db.ast.get(node).clone() {
            Struct::List(children) => children
                .iter()
                .any(|&c| reentrant_call_under_conditional(db, c, callee_def)),
            Struct::Atom(_) => false,
        },
    }
}

/// The group-aware pre-check mirroring `multivalue_leaves_threadable` but for a mutual-recursive SCC member:
/// a leaf is threadable unless a RE-ENTRANT call (self OR mutual partner) is gated behind a conditional
/// (`reentrant_call_under_conditional`). Used by the group-entry mode decision to decline UP FRONT a group
/// whose leaves the multi-value tuple machinery cannot bind, so no partial group is reserved.
fn group_multivalue_leaves_threadable(db: &mut Db, body: StructId, callee_def: usize) -> bool {
    match resolved_of(db, body) {
        Resolved::If { cond, then_, else_ } => {
            !reentrant_call_under_conditional(db, cond, callee_def)
                && group_multivalue_leaves_threadable(db, then_, callee_def)
                && group_multivalue_leaves_threadable(db, else_, callee_def)
        }
        Resolved::Match { scrutinee, arms } => {
            !reentrant_call_under_conditional(db, scrutinee, callee_def)
                && arms.iter().all(|&(_, arm_body)| {
                    group_multivalue_leaves_threadable(db, arm_body, callee_def)
                })
        }
        // A `(let (inits…) dispatch)` whose body is an `if`/`match` — the group-SCC analogue of the #12 arm in
        // `multivalue_leaves_threadable`. `thread_returning_tuple` DOES descend this (threads each init with
        // `thread_bounded`, recurses on the body dispatch returning a tuple), so the pre-check must mirror it or
        // the group is declined up front and never threaded. The inits are on the unconditional strict spine
        // (no re-entrant call may be gated behind a conditional THERE — a demand-perform-demand init `(let a =
        // demand(child) in …)` is on the strict spine, fine), and the body dispatch's own leaves must be
        // group-threadable. Without this the whole `let` fell to the leaf case below and
        // `reentrant_call_under_conditional` saw the body's arm partner-call under the `match` → declined,
        // blocking the mutual demand/cache/compute spine (compiler-ml's lazy DB).
        _ if db.ast.as_form(body, "let").map(|t| t.len()) == Some(2) && {
            let form = db.ast.as_form(body, "let").unwrap().to_vec();
            matches!(
                resolved_of(db, form[1]),
                Resolved::If { .. } | Resolved::Match { .. }
            )
        } =>
        {
            let form = db.ast.as_form(body, "let").unwrap().to_vec();
            let (bindings_occ, body_occ) = (form[0], form[1]);
            let inits_ok = match db.ast.get(bindings_occ).clone() {
                Struct::List(pairs) => pairs.iter().all(|&pair| match db.ast.get(pair).clone() {
                    Struct::List(kv) if kv.len() == 2 => {
                        !reentrant_call_under_conditional(db, kv[1], callee_def)
                    }
                    _ => false,
                }),
                _ => false,
            };
            inits_ok && group_multivalue_leaves_threadable(db, body_occ, callee_def)
        }
        _ => !reentrant_call_under_conditional(db, body, callee_def),
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
            let if_head = db.push_name("if");
            let if_node = db.push_list(vec![if_head, rcond, rthen, relse]);
            Some(drain_and_wrap(db, ctx, mark, if_node))
        }
        // A `match`: thread the SCRUTINEE, then each arm BODY is its own tail — recurse under a fresh copy of
        // the post-scrutinee state. The PATTERN is a binder position (copied structurally). Rebuild `(match
        // rscrut (rpat body-tuple)…)`, wrapping any scrutinee-level self-call temps around it.
        Resolved::Match { scrutinee, arms } => {
            let mark = ctx.pending.borrow().len();
            let (rscrut, cur) = thread_bounded(db, scrutinee, states, ctx, 0)?;
            let match_head = db.push_name("match");
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
        // A `(let ((n init)…) dispatch)` whose BODY is itself a dispatch (`if`/`match`) holding the tail
        // self-call — finding #12 (`walk k = let d = E.next in (if … (walk (+ k 1))))`). The leaf arm below
        // would treat the whole `let` as one leaf and `selfcall_under_conditional` would see the self-call
        // under the body's `if` → decline, dropping the recursion to single-return (the trailing-observer
        // reads the PRE-call state, 705 vs 708). Instead THREAD the inits (each may perform — advancing
        // state), then recurse `thread_returning_tuple` on the body under the post-init state, and rebuild the
        // `let` around the body's tuple so the binders stay in scope. Only descend when the body is a
        // dispatch (the shape this arm handles); a plain-leaf `let` body still routes through the leaf arm
        // below (a self-call directly in a leaf `let` is bound by `thread_bounded`, unchanged). The inits are
        // threaded with `thread_bounded` exactly as the ordinary `let` arm does; a self-call IN an init is on
        // the unconditional strict spine and pushes a pending temp drained by the wrapping `drain_and_wrap`.
        _ if db.ast.as_form(body, "let").map(|t| t.len()) == Some(2) && {
            let form = db.ast.as_form(body, "let").unwrap().to_vec();
            matches!(
                resolved_of(db, form[1]),
                Resolved::If { .. } | Resolved::Match { .. }
            ) || db.ast.as_form(form[1], "let").map(|t| t.len()) == Some(2)
                || db.ast.as_form(form[1], "do").is_some()
                // A `(let ((d (E.op))) (f … recursive …))` whose BODY carries the tail re-entrant call —
                // the cross-def recursion-boundary shape (nr0: `(let ((d (S.depth))) (outer (- k 1) (+ acc
                // (inner d 0))))`). The leaf arm below would `thread_bounded` the WHOLE let and DROP the
                // let-init perform's binder (`d` rides into the self-call arg UNBOUND — a CDZ0101 the escape
                // validator catches). Route it here so the init is threaded (binding `d` to the resume value,
                // advancing the state) and the body is recursed as its own tail, rebuilding the `let` so `d`
                // stays in scope.
                || contains_recursive_call(db, form[1], callee_def)
        } =>
        {
            let form = db.ast.as_form(body, "let").unwrap().to_vec();
            let (bindings_occ, body_occ) = (form[0], form[1]);
            let Struct::List(pairs) = db.ast.get(bindings_occ).clone() else {
                return None;
            };
            let mark = ctx.pending.borrow().len();
            let mut cur = states;
            let mut rpairs = Vec::with_capacity(pairs.len());
            for pair in pairs {
                let Struct::List(kv) = db.ast.get(pair).clone() else {
                    return None;
                };
                if kv.len() != 2 {
                    return None;
                }
                let name_copy = copy_pure(db, kv[0]);
                let (rinit, next) = thread_bounded(db, kv[1], cur, ctx, 0)?;
                cur = next;
                rpairs.push(db.push_list(vec![name_copy, rinit]));
            }
            // The body dispatch is the let's tail — thread it returning a tuple under the post-init state.
            let rbody = thread_returning_tuple(db, body_occ, cur, ctx, callee_def)?;
            let let_head = db.push_name("let");
            let bindings = db.push_list(rpairs);
            let rlet = db.push_list(vec![let_head, bindings, rbody]);
            Some(drain_and_wrap(db, ctx, mark, rlet))
        }
        // A `(do stmt… tail)` whose tail is a dispatch (`if`/`match`) or a nested `let`/`do` chain — breaker
        // #14 ra6 (`(let ((a (E.next))) (do (E.next) (if … (race …) k)))`, a let-body-do with a PERFORMING
        // discarded head). The leaf arm below would treat the whole `do` as one leaf and `selfcall_under_
        // conditional` would see the tail self-call under the `if` → single-return, dropping the out-state the
        // discarded head advanced (the trailing observer reads pre-recursion state). Thread the leading stmts
        // (each may PERFORM — advancing state, its value discarded) with `thread_bounded`, recurse
        // `thread_returning_tuple` on the tail under the post-stmt state, and rebuild the `do`. Mirrors the
        // `let`-dispatch arm above (the do-analogue). Only when the tail is threadable-shaped; a plain-leaf do
        // tail still routes through the leaf arm.
        _ if db
            .ast
            .as_form(body, "do")
            .map(|t| t.len() >= 2)
            .unwrap_or(false) =>
        {
            let items = db.ast.as_form(body, "do").unwrap().to_vec();
            let (tail, stmts) = items.split_last().unwrap();
            let (tail, stmts) = (*tail, stmts.to_vec());
            let mark = ctx.pending.borrow().len();
            let mut cur = states;
            let mut rstmts = Vec::with_capacity(stmts.len());
            for st in stmts {
                // A leading stmt is evaluated for effect (value discarded); thread its state forward.
                let (rst, next) = thread_bounded(db, st, cur, ctx, 0)?;
                cur = next;
                rstmts.push(rst);
            }
            // The tail is the do's value — thread it returning a tuple under the post-stmt state.
            let rtail = thread_returning_tuple(db, tail, cur, ctx, callee_def)?;
            let do_head = db.push_name("do");
            let mut children = vec![do_head];
            children.extend(rstmts);
            children.push(rtail);
            let rdo = db.push_list(children);
            Some(drain_and_wrap(db, ctx, mark, rdo))
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
/// Whether `node` references a synthesized `#cv{…}` name — the condition-value binding the performing-
/// conditional hoist introduces (`(if C t e)` ≡ `(let ((#cv C)) (if #cv t e))`). Such a name is bound by a
/// `let` that wraps only its own conditional; a branch out-state carrying a `#cv` ref cannot be re-used in
/// the merged If-arm state position (it would resolve out of scope → CDZ0101). The If-arm branch-out-state
/// merge skips a slot whose branch out-state contains one. A cheap name-prefix scan (a `#cv`-prefixed name is
/// unbindable in source — CDZ0210 in binder position — so a user can't introduce a colliding one and the match
/// is unambiguous).
fn contains_cv_ref(db: &Db, node: StructId) -> bool {
    any_name(db, node, |nm| nm.starts_with("#cv"))
}

/// Whether `node`'s subtree mentions a NAME node equal to `name` (a purely syntactic scan). Used by the
/// `do`-peel's effectful-stmt guard: at the arm-return thread site a do-`def` binder resolves unreliably
/// (a body binder shows as `Poison(Unbound)`), so a resolution-based ref count can't be trusted — a stable
/// name-string match is the safe witness for "the next-state reads this effectful def's binder".
fn subtree_mentions_name(db: &Db, node: StructId, name: &str) -> bool {
    any_name(db, node, |nm| nm == name)
}

/// Whether any NAME atom in `node`'s subtree satisfies `pred` — a purely syntactic DFS (`as_name`/`get` only,
/// no resolution). The shared walker behind `contains_cv_ref` (`#cv`-prefix) and `subtree_mentions_name`
/// (exact-name match); collapse the two identical recursions (v-code-cleanliness dedup lead).
fn any_name(db: &Db, node: StructId, pred: impl Fn(&str) -> bool + Copy) -> bool {
    if let Some(nm) = db.ast.as_name(node) {
        return pred(nm);
    }
    match db.ast.get(node) {
        Struct::List(children) => children.iter().any(|&c| any_name(db, c, pred)),
        Struct::Atom(_) => false,
    }
}

/// Whether `prim` is an ACCUMULATING collection operation — one that takes a prior collection as its FIRST
/// operand and produces a new collection embedding it (`prelude-and-resolution.md`; the set v-rust-backend
/// root-caused as the O(k^N) shapes). Each re-substitutes the prior slot-state per dispatch, so a fold that
/// threads its result blows the Core up exponentially unless the state is per-dispatch let-bound. This is the
/// GENERIC axis — the compiler's own `Prim` intrinsics, NOT operator spelling (a user op coincidentally named
/// `push` resolves to no `Prim` and is excluded, per the no-hard-coded-capabilities rule). EXCLUDES fresh
/// constructors (`ListNew`/`MapNew`/`SetOf` — don't embed a prior), reads (`ListAt`/`MapLookup`/`*Len`),
/// removes (`MapRemove`/`SetRemove` — the shrink twin, not the shipping blow-up surface), and every scalar/
/// plain-compound op (`Add`/`Record`/`Tuple`/…): those keep the state O(1) and stay byte-identical.
fn is_accumulating_collection_prim(prim: crate::resolved::Prim) -> bool {
    use crate::resolved::Prim;
    matches!(
        prim,
        Prim::ListPush
            | Prim::ListPrepend
            | Prim::ListConcat
            | Prim::ListUpdate
            | Prim::MapInsert
            | Prim::SetInsert
            | Prim::SetUnion
            | Prim::SetIntersection
            | Prim::SetDifference
            | Prim::BytesConcat
    )
}

/// FINDING-24 guard: does a threaded slot-state GROW a collection off the prior state — i.e. does evaluating
/// it PRODUCE the result of an accumulating collection op (`(List.push pre t)`, `(Map.insert m k v)`, a
/// `Bytes.concat`), possibly THROUGH a `let`/`match`/`if`/name that forwards to one? Only such a state
/// re-embeds the prior slot per dispatch (each dispatch the body reads state k times, each read re-substitutes
/// the growing expr → O(k^N) Core, `core_of`'s memo can't collapse the StructId-distinct `deep_fresh_copy`
/// trees → rustc SIGSEGVs the async emit). Those states get the per-dispatch `#st` sibling let-bind (the fix);
/// EVERY OTHER shape — a bare name / prior `#st` ref, a tuple PROJECTION `(. t 1)`, a scalar counter `(+ s 1)`,
/// a plain record/tuple, a fresh-constant collection — keeps the state O(1) and stays byte-identical.
///
/// NARROWED (was: fire on ANY compound) to the accumulating-collection `Prim` set — matching a `Prim`, not a
/// spelling, is the generic axis (`is_accumulating_collection_prim`) and drops the benign compounds (counter
/// folds, projections, records, plain calls) that the broad guard over-fired on. The mk21 shape threads the
/// grown collection through a `let`-bound `match` (`(let ((m2 (match … (Map.insert m k v) …))) m2)`), so the
/// reach follows `Ref`/`Let`-body/`Match`-arms/`If`-branches to find the producing op — a top-level shape
/// match alone would miss it.
///
/// EXCLUDES a next-state that REACHES A PERFORM (`reaches_any_perform`): a host/foreign perform in the
/// next-state slot must be discharged IN PLACE by the enclosing fold (it is the effect the dispatch runs) —
/// hoisting it into an `#st` let-init reorders/elides it (breaker: `(host (ask) …)` next-state call trapped
/// "host-call ask.ask" + a final-dispatch state-call elided). Binding is for a PURE growing state only; a
/// perform-bearing state stays where it is regardless of size (correctness over the size win).
/// Correctness-safe either way for the pure case: a false "not growing" only leaves the exponential in place
/// (no miscompile); a false "growing" only adds a redundant kept binding (behavior-preserving, same value +
/// eval order). Keeps the common corpus byte-identical — the bind fires ONLY on a growing-collection state.
fn next_state_is_growing_compound(db: &mut Db, next_state: StructId) -> bool {
    // A perform-bearing state must NOT be hoisted into a let-init (it would reorder/elide the effect).
    // NOTE: a state carrying a `#seed` ref IS a valid F24 target (the growing states ARE seed-derived, e.g.
    // `(List.push #seed22 t)`); it binds correctly BECAUSE the #st let nests INNER to the #seed wrap and the
    // single reparent_under_handle_site resolves the whole tree in one pass (seed-graft contract, v-rb) — the
    // #st init must be registered BY REFERENCE to the original node (attached #seed), never deep_fresh_copy'd.
    if reaches_any_perform(db, next_state) {
        return false;
    }
    next_state_produces_accumulating_op(db, next_state, 0)
}

/// Recursion bound for the accumulating-op reach — a threaded state forwards through at most a few
/// `let`/`match`/`if`/name hops before its producing op; well above any real fold shape (mk21 is depth 3:
/// `let` → binder `Ref` → `match` arm). A deeper chain declines to fire (leaves the exponential in place —
/// the safe floor, never a miscompile).
const F24_REACH_LIMIT: u32 = 16;

/// Does evaluating `node` PRODUCE the result of an accumulating collection op (`is_accumulating_collection_prim`)?
/// Follows the value-producing spine only — an application HEAD that is an accumulating `Prim`, or a
/// `Ref`/`Let`-body/`Match`-arm/`If`-branch that forwards to one — so a growing op reached through the mk21
/// `let`-bound `match` fires while a benign compound (arith, record, projection, a plain call whose callee we
/// don't follow) does not. Bounded by `F24_REACH_LIMIT`.
/// Does an accumulating op's ELEMENT arg reference a name bound to a PRIOR DISPATCH RESULT — a body
/// let/do-binding the fold FRESHENED to `#{userletter}{id}` (`freshen_walk`, effects.rs ~7043/7076, which
/// renames every body-lifted binder to `#{name}{arena-id}` as it inlines the interposing let/do)? Such a
/// binding's value came from an EARLIER dispatch of this handler (mk2: `a = (Reg.touch n)`, referenced by
/// dispatch-2's op-arg `(+ #a64 1)`), so the growth is DATA-CHAINED on a prior dispatch's OUTPUT, not a
/// straight-line accumulator: it does not multiplicatively re-embed (mk2 baseline 4838 bytes, no blowup),
/// and firing the `#st` bind ORPHANS that resume-value binding when the drain re-scopes the body (CDZ0101
/// `#a…`).
///
/// SYNTACTIC signal (resolution is unavailable — at the arm-return thread site a body binder resolves to
/// `Poison(Unbound)`, indistinguishable from an outer param, so a resolve/reach test can't separate them):
/// a `#`-prefixed name that is NOT one of the KNOWN accumulation synthetics (`#st` slot bind, `#seed` handle
/// seed, `#cv` op-arg lift, `#kv` perform-result lift) is a `freshen_walk`-renamed BODY binding = a prior
/// dispatch result. A plain outer PARAM (`n`) stays a BARE name (never `#`-prefixed), so a real grower whose
/// op-arg reads `n` (bf1 `(+ 10 n)`) still fires — and its free-var aliasing is handled by the selective
/// copy. Every prelude/user op name (`+`, `UInt8`, `wrap`, `bin`) is bare too. Bounded by `F24_REACH_LIMIT`.
fn element_arg_reaches_prior_dispatch_result(db: &mut Db, node: StructId, depth: u32) -> bool {
    if depth > F24_REACH_LIMIT {
        return false; // too deep — do NOT over-exclude (a missed exclusion only leaves a benign case firing;
        // over-exclusion would disable the fix on a real accumulator — the worse failure)
    }
    if let Some(nm) = db.ast.as_name(node) {
        // Two `#`-prefixed shapes in an ELEMENT-arg position (elem/val/key — NOT the collection operand
        // args[0], which the caller skips) mean the growth is NOT a straight-line accumulator, so decline:
        //   (a) a `#seed` — a HANDLE SEED threaded as DATA. In a straight-line accumulator the only `#seed`
        //       is the collection operand (args[0], the accumulation itself — pfxM/f24-list `(Map.insert
        //       #seed30 …)`). A `#seed` in an ELEMENT arg is a CROSS-HANDLER seed woven into the added value
        //       (xh1: an inner `put` dispatched inside an outer `S` arm, `(Map.insert #st (+ #seed103 1)
        //       #seed103)` — `#seed103` is the OUTER S-handler's seed). The inner `#st` embeds the outer
        //       `#seed` whose binder is grafted only when the OUTER handler returns (AFTER the inner drain), so
        //       the bind orphans it (CDZ0101 `#seed`); and the bind does not even linearize the nested shape
        //       (still exponential) — trunk emits xh1 correctly (219KB, no rustc-SIGSEGV), so declining loses
        //       nothing. `#cv`/`#kv` (op-arg / perform-result lifts) are likewise not straight-line accumulation.
        //   (b) any OTHER `#`-prefixed name — a `freshen_walk`-renamed BODY binding (`#a64`, mk2's `a` = a prior
        //       dispatch's resume value), the data-chained-dispatch case.
        // The ONLY `#`-name that is NOT a decline signal here is `#st` (a PRIOR slot bind threaded forward as
        // the accumulation — but that too is normally the collection operand; in an element arg it would be a
        // slot read, still fine to keep). So: decline on any `#`-name EXCEPT `#st`.
        return nm.starts_with('#') && !nm.starts_with("#st");
    }
    match db.ast.get(node).clone() {
        Struct::Atom(_) => false,
        Struct::List(children) => children
            .iter()
            .any(|&c| element_arg_reaches_prior_dispatch_result(db, c, depth + 1)),
    }
}

fn next_state_produces_accumulating_op(db: &mut Db, node: StructId, depth: u32) -> bool {
    if depth > F24_REACH_LIMIT {
        return false;
    }
    match resolved_of(db, node) {
        // An accumulating op is applied as `(Map.insert m k v)` etc. Its HEAD is a member access
        // (`Map.insert` → `Resolved::Member`), whose intrinsic is read off the `(meta apply)` channel by
        // `meta_apply_of`; a direct `Prim` head (a desugared intrinsic) is read by `prim_of`. Consult both,
        // exactly as `lower.rs`'s application dispatch does (`meta_apply_of(head).or_else(prim_of(head))`).
        Resolved::Apply { head, args } => {
            let is_accum = crate::eval::meta_apply_of(db, head)
                .or_else(|| crate::eval::prim_of(db, head))
                .is_some_and(is_accumulating_collection_prim);
            // DATA-CHAINED-DISPATCH EXCLUSION (mk2): if an ELEMENT arg (any arg after the first — the
            // collection operand) reaches a name whose BINDING is a PRIOR DISPATCH RESULT (a resume-value
            // let-binding of THIS handler — its init reaches a discharged perform), the growth is data-chained
            // on a prior dispatch's OUTPUT, not a straight-line accumulator: (i) it does not multiplicatively
            // re-embed (mk2 baseline 4838 bytes, no blowup), and (ii) firing the `#st` bind orphans that
            // resume-value binding when the drain re-scopes the body (CDZ0101 `#a…`). EXEMPT: the collection
            // operand `args[0]` (it IS the accumulation — the slot/seed), a plain outer PARAM like main's `n`
            // (bf1's `(+ 10 n)` — `n` has no init/no perform, so a real bytes-frame grower still FIRES + is
            // fixed by the selective-copy), and any arm-inner pure let (`kk`=`Map.len`, `t`=`prev+v` — no
            // perform behind them). The signal is precise: a PERFORM behind an element-arg name = a prior
            // dispatch's resume value.
            is_accum
                && !args
                    .iter()
                    .skip(1)
                    .any(|&a| element_arg_reaches_prior_dispatch_result(db, a, 0))
        }
        // A bare name / prior `#st` ref → follow to its bound value (mk21's `m2` → the `match`).
        Resolved::Ref { value } => next_state_produces_accumulating_op(db, value, depth + 1),
        // A `let` forwards its VALUE from the body; the body is typically the binder name, resolved via `Ref`.
        Resolved::Let { body, .. } => next_state_produces_accumulating_op(db, body, depth + 1),
        // A `match`/`if` produces a grown collection if ANY reachable branch does — but ONLY when reached
        // DEEPER (depth > 0), i.e. inside a `let`-binding init whose RESULT is bound and threaded as a name
        // (mk21: `(let ((m2 (match … (Map.insert …) …))) m2)` — the match is bound to `m2` atomically). A
        // TOP-LEVEL conditional-valued next-state `(if c (List.push s v) s)` / `(match … (grow) (s))` is the
        // two-hole IF/MATCH-PEEL shape (`peel_resume_from_arm_body`): it already threads correctly, and the
        // refold REBUILDS its condition/scrutinee via push_list — so registering it BY REFERENCE for the `#st`
        // bind detaches a shared op-arg node (the `(> v 10)`, `v`↦body-free-var `n`) → a false CDZ0101 unbound
        // (breaker ts1, #2336). Declining to fire on a top-level conditional leaves the (small, non-blown-up)
        // peel shape byte-identical — the safe floor (a top-level `if` where BOTH branches grow would miss the
        // size win, but no such shape blows up in the corpus, and a false "not growing" is never a miscompile).
        Resolved::Match { arms, .. } if depth > 0 => arms
            .iter()
            .any(|&(_, body)| next_state_produces_accumulating_op(db, body, depth + 1)),
        Resolved::If { then_, else_, .. } if depth > 0 => {
            next_state_produces_accumulating_op(db, then_, depth + 1)
                || next_state_produces_accumulating_op(db, else_, depth + 1)
        }
        // SEE-THROUGH a TUPLE PROJECTION `(. t idx)` into the tuple's element `idx`. The multi-resume peel
        // may thread the next-state as a projection of a let-bound `(tuple <value> <state>)` — bound once
        // in scope so a value AND a threaded next-state share one slot (the tpwJ cross-scope collapse). A
        // GROWING state (`(Map.insert m k v)` / `(List.push s v)`) wrapped as that tuple's `idx` element
        // would otherwise be HIDDEN from this detection (a `Proj` matches none of the arms above → `false`),
        // so the `#st` per-dispatch bind would NOT fire and the FINDING-24 exponential blowup would RETURN
        // for a growing-state handler. Resolve the projected operand to its `(tuple …)` construction (a
        // direct tuple form, or a `Ref`/`Let` chain ending at one) and recurse into element `idx`, so a
        // growing element inside a threaded tuple is still detected and `#st` still fires. `None` if the
        // operand does not reach a tuple with that element (a non-tuple projection is not an accumulator).
        Resolved::Proj { operand, index } => tuple_elem_of(db, operand, index)
            .is_some_and(|elem| next_state_produces_accumulating_op(db, elem, depth + 1)),
        _ => false,
    }
}

/// The `index`-th element expression of the tuple `(tuple e0 e1 …)` the node at `operand` reaches — a DIRECT
/// tuple construction, or one reached through a `Ref` (a `let`-binder to a tuple init) or a `Let` body.
/// `None` if `operand` does not reach a tuple, or the tuple has no such element. Used by the tuple-projection
/// see-through arm of the growing-state detection so a `(. t idx)` next-state is inspected at its real
/// element rather than treated as opaque (which would hide a growing state from the `#st` bind — F24).
fn tuple_elem_of(db: &mut Db, operand: StructId, index: usize) -> Option<StructId> {
    // Follow a `Ref` (let-binder → its init) / `Let` (→ body) chain to the tuple construction, bounded.
    let mut cur = operand;
    for _ in 0..=F24_REACH_LIMIT {
        // A direct `(tuple e0 e1 …)` form — the head is `tuple`, the elements are the tail.
        if let Some(elems) = db.ast.as_form(cur, "tuple") {
            return elems.get(index).copied();
        }
        match resolved_of(db, cur) {
            Resolved::Ref { value } => cur = value,
            Resolved::Let { body, .. } => cur = body,
            _ => return None,
        }
    }
    None
}

/// Thread a conditional BRANCH / match ARM body, returning its rewrite AND its OUT-STATE (per slot) — the threaded
/// state each slot has after the branch runs. The `If`/`Match` arms use this to MERGE per-branch out-states
/// into a conditional-valued out-state so a SIBLING that reads state after the conditional (a recursive-call
/// operand, a `do`-continuation) sees the advance a branch perform made — fixing the recursive-branch-perform
/// self-recursive faces + the through-block fold (the If-arm previously returned the pre-branch post-condition
/// state, dropping branch advances). On a branch-local abort, the out-state is the incoming `states`
/// unchanged (the abort discards the continuation, so no advance is observable past it).
fn thread_branch_local_abort_with_out(
    db: &mut Db,
    branch: StructId,
    states: Vec<StructId>,
    ctx: &HandlerCtx,
    inline_depth: u32,
) -> Option<(StructId, Vec<StructId>)> {
    let before = ctx.abort_value.get();
    let states_in = states.clone();
    let (rbranch, out) = thread_bounded(db, branch, states, ctx, inline_depth)?;
    let after = ctx.abort_value.get();
    // A NEW abort fired while threading THIS branch → it is local to the branch: use the abort value as
    // the branch's rewrite and restore the cell to its prior state (so a sibling branch / the handle is
    // not collapsed). Its out-state is the incoming state (the abort abandons the continuation). If no new
    // abort fired, keep the ordinary threaded rewrite + its threaded out-state.
    if after != before
        && let Some(abort) = after
    {
        ctx.abort_value.set(before);
        // ABORT-FOLD do-SHAPE inside a BRANCH / match-ARM body (the branch/arm-body face of the abort-outer-
        // advance fix — this helper threads both `if` branches AND `match` arm BODIES; sibling faces landed
        // separately: direct-handle-body do-shape #2002, strict-operand #2010, match-SCRUTINEE #2017 — a
        // perform in a match SCRUTINEE is on the strict spine and threaded normally, NOT here). A FOREIGN
        // perform (an OUTER handler's effect) on the branch's strict spine BEFORE the abort has committed
        // its advance and must survive — but the branch-local collapse returns the BARE `abort` value,
        // discarding `rbranch` (which the do-arm already built as the sound `(do <foreign…> <abort-value>)`).
        // `(if c (do (A.tick) (B.bail 99)) …)` / `(match k (_ (do (A.tick) (B.bail 99))))` under B then reads
        // the outer `(A.get)` at the pre-advance state (109 vs 110). Use `rbranch` (a `do`-form reaching a
        // foreign perform) as the branch rewrite so the ENCLOSING fold discharges the prefix; its out-state
        // is still `states_in` (the abort abandons THIS branch's continuation, so no advance is observable
        // PAST the conditional — only the foreign prefix's own effect escapes, carried inside the `do`). Same
        // do-form gate + parented-foreign-reach as the reduce_handle do-shape fold; a non-do `rbranch`
        // (a bare `(B.bail 7)` / a `(+ 100 (B.bail 7))` collapsed to `7`) keeps the bare-abort value.
        if db.ast.as_form(rbranch, "do").is_some() && body_reaches_foreign_perform(db, branch, ctx)
        {
            Some((rbranch, states_in))
        } else {
            Some((abort, states_in))
        }
    } else {
        Some((rbranch, out))
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
            // OP-ARG LET-LIFT (cross-handler inline-arg-position completeness fix, breaker xh1/xh2). A
            // MULTI-USE param whose arg PERFORMS would duplicate the effect on substitution (the guard below).
            // For a FOREIGN perform arg — an op THIS handler does not discharge, e.g. `(B.put (A.get))` under
            // B where `A.get` homes to the enclosing A — the sound rewrite is to BIND it once to a fresh
            // `#cv` let and duplicate the pure ref, exactly the Site-5 hoist for performing conditions (verified
            // by xh2: the hand-let-bound spelling `(let ((x (A.get))) (B.put x))` FOLDS while the inline arg
            // declines). Lift each such arg BEFORE building the arm body: replace the arg with a `#cv` ref and
            // remember the binding, then wrap the whole arm result in the `let`s (the perform runs once, in
            // arg-evaluation order, then B's arm reads the pure `#cv` twice). Gated to FOREIGN performs (a
            // perform of THIS handler's OWN op as a multi-use arg still declines — it needs threading, not a
            // plain let-bind; a distinct, harder shape). A single-use or pure arg is untouched.
            let mut arg_lifts: Vec<(StructId, StructId)> = Vec::new(); // (#cv binder, arg)
            if arm.params.len() == rewritten_args.len() {
                // Collect indices of multi-use params whose arg performs; decide lift-vs-decline per arg.
                let mut to_lift: Vec<usize> = Vec::new();
                for (i, &p) in arm.params.iter().enumerate() {
                    let a = rewritten_args[i];
                    let refs = count_param_refs(db, arm.body, p);
                    if is_unit_param(db, p) || !arg_reaches_any_perform(db, a, ctx) || refs == 1 {
                        // A unit param, a PURE arg (nothing to preserve — a dropped pure arg stays elided,
                        // core-semantics.md #A Trap Occurs Only Where Its Computation Is Observed / the
                        // unused-parameter elision in 09-functions), or a SINGLE-use performing arg (naive
                        // substitution places it once → the perform runs exactly once, no lift needed).
                        continue;
                    }
                    // A performing arg used a number of times OTHER than once — `refs == 0` (the arm IGNORES
                    // its param, so substitution DROPS the arg) or `refs >= 2` (the arm reads it more than
                    // once, so substitution DUPLICATES it). Naive substitution would LOSE the perform (dropped)
                    // or RE-RUN it (duplicated). If the arg performs a FOREIGN op (none of it is THIS handler's
                    // discharged op that would need threading), LET-LIFT it so the perform runs EXACTLY ONCE
                    // for its effect: a DROPPED foreign perform still threads its state advance to the outer
                    // handler (an unread op-arg's foreign perform must not vanish — its declared operation is
                    // observable, capabilities-and-effects.md #A Handler Threads State; the same class as the
                    // do-discarded-foreign-perform preservation below), and a DUPLICATED one is bound once and
                    // read via the pure `#cv` ref (xh1/xh2). A perform of THIS handler's discharged op that
                    // would be DUPLICATED (`refs >= 2`) needs threading, not a plain let — decline (safe floor).
                    // (A DROPPED (`refs == 0`) discharged-op arg is left untouched here — out of scope; the
                    // enclosing fold's own threading covers the single-handler case, and a cross-level drop of
                    // a discharged-op arg is a separate, harder arc.)
                    // We already established `arg_reaches_any_perform` TRUE via the guard above, so test only
                    // the discharged-op half here — one traversal per candidate (github-liaison/Copilot #2120).
                    if !subtree_reaches_discharged_op(db, a, ctx) {
                        to_lift.push(i);
                    } else if refs >= 2 {
                        return None;
                    }
                }
                for i in to_lift {
                    let a = rewritten_args[i];
                    // `#cv{StructId}` is a globally-unique fresh binder. Collision-safety rests on TWO facts
                    // (github-liaison #2156 corrected the earlier "unspellable" claim): (1) the StructId is the
                    // arena index of the arg node `a` — a MONOTONIC, never-reused counter (`push_atom`/
                    // `push_list` = `StructId(structure.len())`), so two lift sites never share a `#cv{…}` name
                    // (the Site-5 performing-condition hoist keys on its own distinct `if`/`match` node id). (2)
                    // A user can never introduce a `#cv{N}` BINDER to capture/shadow a live lift site: a
                    // `#`-leading name is a refutable CONSTRUCTOR pattern, illegal in binding position (rejected
                    // CDZ0210). So `#cv…` is UNBINDABLE, not lexically unspellable — a backtick `` `#cv0` `` DOES
                    // lex as a REFERENCE, but reference position can't capture; only a binder could, and a
                    // `#cv…` binder is rejected. Pinned by `the_op_arg_lift_cv_binder_namespace_cannot_be_
                    // captured_by_a_user_binder`. No centralized gensym needed — arena-id monotonicity + binder-
                    // position rejection guarantee it (github-liaison/Copilot #2120/#2156 reviews).
                    let cv_name = format!("#cv{}", a.0);
                    let cv_binder = db.push_name(&cv_name);
                    let cv_ref = db.push_name(&cv_name);
                    arg_lifts.push((cv_binder, a));
                    rewritten_args[i] = cv_ref;
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
            // A FINDING-24 `#st{node}_{slot}` growing-state bind (set at the resume path below) is in scope
            // ONLY inside the RESUME continuation's `drain_and_wrap`. An ABORTIVE arm that READS the state
            // gets `cur[slot]` = that `#st` NAME substituted for its state binder, but the strict-op abort
            // collapse emits the abort value OUTSIDE that scope and drains the `#st` binds for `do`-form
            // bodies only — so a `+`-form abort's `#st` reference LEAKS unbound → a spurious CDZ0101 on a
            // well-formed program (breaker abx3/ab4). Decline HONESTLY (`reduce_handle` → None →
            // HANDLER_NOT_REDUCIBLE todo) rather than emit the leaked `#st`. NARROW: fires only when the
            // state IS a `#st` name (a prior resume grew it), this arm is abortive, AND its body references
            // the state — an abort-only handle (abx5: state is the seed, not `#st`) or a state-ignoring abort
            // arm (abx4) is untouched. The FULL fold (drain the `#st` into the strict-op abort value + the
            // outer-observation soundness) is a separate increment.
            if ctx.abortive.contains(&(decl, idx))
                && db
                    .ast
                    .as_name(cur[slot])
                    .is_some_and(|n| n.starts_with("#st"))
                && count_param_refs(db, arm.body, arm.state) > 0
            {
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
                // OP-ARG LET-LIFT on the ABORT value too (strict-fold #17 face-4 RESIDUAL, corpus-bugfix
                // 2026-08-11). When the aborting arm IGNORES its param, its unread arg — a FOREIGN perform
                // `(B.tick)` this handler does not discharge — was dropped by the `beta_reduce` above, so the
                // outer handler never threaded its state advance (a later `(B.tick)` read stale state:
                // `(A.bail (B.tick))` under B → 55003 not 55004). The RESUMPTIVE path already wraps its result
                // in `arg_lifts` (below); the abort path returned the bare arm value, skipping them. Wrap the
                // abort value in the SAME `#cv` lets so the foreign perform still runs ONCE for effect before
                // the abort collapses the continuation — the abort discards the CONTINUATION, not an
                // already-committed foreign advance (the do-shape abort-outer-advance rule, same spirit as the
                // `(do <foreign…> <abort-value>)` preservation in `reduce_handle`). A resumptive arg drop and an
                // abortive arg drop are the SAME face; both must thread the dropped foreign dispatch. Nothing
                // lifted (a pure/unread-pure arg, the common case) → `arm_body` unchanged, byte-identical.
                let aborted = if arg_lifts.is_empty() {
                    arm_body
                } else {
                    let mut wrapped = arm_body;
                    for &(binder, arg) in arg_lifts.iter().rev() {
                        let let_head = db.push_name("let");
                        let pair = db.push_list(vec![binder, arg]);
                        let bindings = db.push_list(vec![pair]);
                        wrapped = db.push_list(vec![let_head, bindings, wrapped]);
                    }
                    wrapped
                };
                let copied = copy_pure(db, aborted);
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
            // [tpwJ, breaker 2026-08-16] CROSS-SCOPE shared-`let` arm: when a `let` binder is referenced in
            // BOTH a resume value AND its next-state, the DISTRIBUTE peel below splits it into two separate
            // matches/ifs whose two copies land in different emit scopes — a multi-use binding kept in one,
            // copy-propagated away in the other → cross-scope "no local slot". Serve it via the COLLAPSED path
            // instead: rewrite the arm's tail resumes to `(tuple v s)` IN PLACE (structure intact, the binder
            // stays bound once in its own arm with its match-pattern binders live), bind that single
            // `(value, state)` tuple ONCE to a fresh `#st`-prefixed name (materialized by `drain_and_wrap`
            // OUTSIDE both, force-kept via the `should_keep_binding` `#st` carve-out), and PROJECT
            // value = `(. t 0)`, next_state = `(. t 1)`. Both projections read the ONE shared slot → no
            // cross-scope orphan. Falls back to the distribute peel if the collapse does not model the shape.
            // OPTION A (v-inference-confirmed): fire the COLLAPSE only when the combined is CLOSED at the
            // DRAIN level — i.e. every op-arg substituted into it is drain-safe (a literal / an outer param /
            // an already-`#st`-bound state ref), NOT a HANDLE-BODY or ARM-LOCAL `let` binding. The collapsed
            // tuple is materialized by `drain_and_wrap` OUTSIDE the arm, so a substituted op-arg that is a
            // body-bound name (which the fold LIFTS to a `#`-name bound near the perform site) would resolve
            // out of scope there → CDZ0101 (neu1's `#big70`). A body-bound-op-arg arm STAYS on the distribute
            // path (where it already folds correctly), so this narrowing closes the literal-op-arg family
            // (tpwJ + the tpw ladder) with ZERO regression and defers the body-bound-op-arg cross-scope class
            // as an honest follow-up. `Resolved::Ref` is exactly a `let`-binding reference; a `Param`/literal/
            // member/prim is not, so it is the precise drain-safe discriminator.
            // OPTION A (drain-closed): fire the COLLAPSE only when the built `combined` is HOISTABLE to the
            // drain level — it must reference no LIFTED `#`-name that is bound NEAR the perform site (a `#cv`
            // op-arg/condition lift, a `#big…`/freshen-walk body-let lift), since `drain_and_wrap` materializes
            // the combined OUTSIDE the arm and such a name would resolve out of scope there → CDZ0101 (neu1's
            // `#big70`). `#st`/`#seed` names ARE bound at the drain level (the per-dispatch state bind and the
            // handle-seed lift), so a threaded `(. #st… 1)` incoming state is fine — that is what keeps a
            // uniformly-literal-arg handler (tpwJ) ALL on the collapse path across dispatches. A mixed handler
            // whose combined reaches a body-lifted name (neu1) stays ALL on the distribute path (where it
            // already folds), so this narrowing closes the literal-op-arg family with ZERO regression and
            // defers the body-lifted class as a follow-up. Structural (not resolution-based): a lifted name's
            // resolution is unreliable pre-materialization, but its `#`-prefix is an exact, stable witness.
            // A-TIGHT GUARDS (v-inference-confirmed): the collapse's per-dispatch `#st`-bind + tuple-projection
            // is a STRAIGHT-LINE-thread transform, sound ONLY for a single-slot, non-recursive-driver handler
            // whose threaded state is not a growing collection. Fire it only there; the diverse remainder stays
            // on the proven distribute path (named follow-up classes: recursive-driver / growing-state /
            // cross-handler shared-let). (1) SINGLE-SLOT — a merged multi-slot ctx (a cross-handler shape,
            // xh1) threads several states; the single-name projection does not model that. (2) NOT a recursive
            // driver — `thread_returning_tuple` infers the recursive result type from the threaded shape, and a
            // tuple-projected next-state leaves it undetermined (rq3). (3) NOT a growing-collection next-state —
            // a `(. t 1)` projecting a `Map.insert`/`List.push` state would need the base's see-through arm
            // AND re-introduces the F24 surface here (lru1/rrb1); defer it.
            let mut arm_next_states: Vec<StructId> = Vec::new();
            let arm_state_grows = arm_resume_next_states(db, arm_body, &mut arm_next_states)
                .is_some()
                && arm_next_states
                    .iter()
                    .any(|&s| next_state_is_growing_compound(db, s));
            // GROWING-STATE was originally excluded from the collapse (F24 caution: a `(. #st_vs 1)` projecting
            // a `List.push`/`Map.insert` next-state re-introduces the F24 exponential + a see-through arm). But
            // for the mid-arm-FOREIGN-PERFORM shared-let shape (xhsGrow) the DISTRIBUTE fallback MISCOMPILES (the
            // shared-binder divergence), and the collapse's per-dispatch `#st_vs` bind IS the F24 fix — so
            // collapsing it is LINEAR (verified: a 6-dispatch xhsGrow compiles in ~0.4s, no blowup) AND correct
            // (differential-confirmed variant==inline-control at 2 and 6 dispatches). So ALLOW the collapse for a
            // growing-state arm that has a foreign-perform-with-args let-init — the correct fold, superseding the
            // safe-floor decline below (which still fires when the frozen combined is non-hoistable). A
            // growing-state arm WITHOUT such a perform (gws1, lru1/rrb1) stays excluded (distribute-correct).
            let collapse_ok = ctx.collapse_enabled.get()
                && ctx.slots.len() == 1
                && !ctx.in_recursive_specialize.get()
                && (!arm_state_grows || arm_has_let_init_reaching_arg_perform(db, arm_body));
            let mut collapsed: Option<StructId> = None;
            let is_candidate = {
                let mdo = ctx.multi_dispatch_ops.borrow().clone();
                arm_is_collapse_candidate(db, arm_body, (decl, idx), &mdo, &ctx.arms)
            };
            if collapse_ok
                && is_candidate
                && let Some(combined0) = peel_tuple_value_state(db, arm_body)
            {
                // FREEZE any mid-arm FOREIGN-perform argument into a kept `#st`-name computed against the
                // INCOMING state (xhs1). Bind-once of the shared binder is necessary but NOT sufficient: when
                // the OUTER handler later folds the embedded foreign perform, it RE-THREADS the perform's arg
                // against its own pass, so a state-dependent arg expression is re-derived (wrong incoming
                // state) — the resume copy stays I-frozen (right) while the arg copy diverges (wrong). Freezing
                // the arg to a force-kept `#st` slot at THIS (inner) fold gives the outer handler an opaque
                // frozen name, not a re-derivable expression. (v-inference's foreign-perform-arg analogue of
                // the `#st` state-bind.)
                let freeze_mark = ctx.pending.borrow().len();
                let combined = freeze_foreign_perform_args(db, combined0, ctx);
                if combined_hoistable_to_drain(db, combined) {
                    collapsed = Some(combined);
                } else {
                    // The freeze speculatively pushed drain-level `#fa` binds; the distribute fallback does not
                    // use the frozen combined, so roll them back (else an unreferenced `#fa` bind materializes).
                    ctx.pending.borrow_mut().truncate(freeze_mark);
                }
            }
            // SAFE-FLOOR (xhsGrow): a GROWING-STATE arm with a mid-arm foreign-perform-with-args let-init has
            // the collapse EXCLUDED by `arm_state_grows`, so it would fall to the distribute peel — which wraps
            // the foreign-perform let-init around BOTH resume slots and mis-threads the shared binder (the xhs1
            // divergence), a SILENT x3 miscompile (43078123 vs the correct 44071111). Decline until the collapse
            // extends to growing-state (the F24 surface). breaker-confirmed no over-fire: keyed on a
            // foreign-perform-with-args let-init + `arm_state_grows`, which no landed corpus case combines (gws1
            // grows but has NO foreign perform; xhs1-D perform but NON-growing → stay collapse-correct).
            if collapsed.is_none()
                && arm_state_grows
                && arm_has_let_init_reaching_arg_perform(db, arm_body)
            {
                return None;
            }
            let (value, next_state) = if let Some(combined) = collapsed {
                let name = format!("#st{}_vs", node.0);
                ctx.pending.borrow_mut().push((name.clone(), combined));
                let value = tuple_proj(db, &name, 0);
                let next_state = tuple_proj(db, &name, 1);
                (value, next_state)
            } else {
                peel_resume_from_arm_body(db, arm_body)?
            };
            // SAFE-DECLINE: a FOREIGN perform DIRECTLY in the threaded NEXT-STATE (as2/as1, breaker
            // 2026-08-05; both-perform gap closed per github-liaison/Copilot #2289). An arm `(resume t (+ t
            // (A.get)))` performing an OUTER effect `A.get` in its next-state expr is unsound as a fold: the
            // next-state threads forward as a state EXPRESSION, so the embedded `A.get` is either DROPPED (a
            // single perform discards the final slot state — as2: `(+ 10 0) + seed = 5`, should be 6) or
            // DUPLICATED across dispatches (multi-perform re-splices the state expr — as1: 63, fits no model).
            // Both are SILENT wrong values, both backends, O0-O3. Decline whenever the RAW resume next-state
            // performs a foreign op — INCLUDING the both-perform `(resume (A.get) (A.get))` where the VALUE
            // also performs one (Copilot #2289: the next-state foreign is dropped REGARDLESS of the value —
            // asb compiled to 56 vs the correct 57 — so a `&& !value-performs` clause let it slip past; that
            // clause is dropped). The proven-correct forms STAY folding because their next-state has NO direct
            // foreign perform: as3 `(resume (+ t (A.get)) t)` — foreign in the VALUE, next-state the bare `t`;
            // as7 `(let ((x (A.get))) (resume t (+ t x)))` — foreign is the let-INIT, resume next-state `(+ t
            // x)` is pure; the interposing `(do (A.tick) (resume v s))` — foreign is a do-STATEMENT, next-state
            // `s`. `arm_resume_next_states` reads the RAW resume child (descending do/let/match WITHOUT wrapping
            // the surrounding binder into it) — unlike `peel_resume_from_arm_body`, which WRAPS the `let`-init/
            // `do`-stmt into both slots and would make as7/interpose look unsound. Runs over the ORIGINAL
            // PARENTED `arm.body` (NOT the post-peel orphans, whose dead parent chain would POISON spec-name
            // resolution — `effect_op_of` resolves the decl-literal; resolving an orphan leaked a
            // `loop#eff3$s1` CDZ0101 into the recursive-fold surface). The structural (non-call-following)
            // `next_state_directly_performs_foreign` distinguishes as2's literal `(A.get)` from a recursive
            // fold that threads an outer effect through a self-call/specialized CALLEE body (never a direct
            // perform in the arm's next-state), so the recursive-fold suite stays folding. The correct FOLD
            // (run the next-state foreign once at dispatch, thread its pure result — the inline analogue of
            // as7's let-lift) is a deeper eval-order arc; decline is the safe floor.
            {
                let mut raw_next_states = Vec::new();
                if arm_resume_next_states(db, arm.body, &mut raw_next_states).is_some()
                    && raw_next_states
                        .into_iter()
                        .any(|s| next_state_directly_performs_foreign(db, s, ctx))
                {
                    return None;
                }
            }
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
            // FINDING-24 (exponential fold-lowering): when the perform advances a slot's threaded state to a
            // GROWING COMPOUND ((List.push pre t) / (Map.insert m k v) / (+ s 1)), the NEXT dispatch's init +
            // the body SUBSTITUTE that state expr, and each dispatch re-embeds the prior → O(k^N) Core
            // (`deep_fresh_copy` bakes N distinct fresh trees; `core_of` memo can't collapse StructId-distinct
            // copies; reproduced N=3..7 ~3x/dispatch, rustc SIGSEGVs the async emit). FIX (per-dispatch state
            // let-bind — the #seed-graft principle per dispatch): bind the grown state to a fresh
            // `#st{node}_{slot}` name (into `ctx.pending`, materialized once by `drain_and_wrap`, INNER to the
            // `#seed` wrap at reduce_handle:2736) and thread the NAME forward via `cur[slot]`, so the next
            // dispatch substitutes the SMALL name, not the growing expr → Core LINEAR. NOTE the #st bind must
            // be force-KEPT through emit (should_keep_binding #st-prefix carve-out, v-rust-backend's lower.rs
            // piece) — else copy-prop re-inlines the single-use binding and the blow-up returns; this fold-side
            // emission + that keep co-land as a co-gated pair.
            //
            // SEED-GRAFT CONTRACT (v-rb): the growing state may carry a `#seed` ref (a heap handle-seed
            // let-lift, bound OUTER by apply_seed_wrap; e.g. `(List.push #seed22 t)`). Register the state
            // BY REFERENCE to the ORIGINAL (pre-`deep_fresh_copy`) node so its `#seed` ref stays ATTACHED —
            // a `deep_fresh_copy` here would re-push the `#seed` ref DETACHED (orphan → CDZ0101 unbound
            // `#seed`). Do NOT resolve the #st init now; the single `reparent_under_handle_site` (2748)
            // resolves the whole wrapped tree (#st lets inner + #seed wrap outer) in one pass, so the #st
            // init's #seed ref binds up exactly like the body's own #seed refs. The threaded `#st` NAME is
            // single-use (one occurrence in the next dispatch), so it needs no fresh-copy sharing guard — the
            // deep_fresh_copy the pre-fix code did on next_state is only needed on the NON-bound path.
            let next_state =
                if ctx.bind_growing_state.get() && next_state_is_growing_compound(db, next_state) {
                    let sname = format!("#st{}_{slot}", node.0);
                    let sref = db.push_name(&sname);
                    // SELECTIVE copy: fresh-copy the init so an outer FREE-VAR the op-arg carries (bf1/bf2/bf3:
                    // `v`↦`(+ 10 n)`) is not ALIASED into two arena positions (→ CDZ0101 unbound `n`), while
                    // PRESERVING the `#seed` ref by-reference so a grafted handle-seed (incl a NESTED handler's,
                    // which piece-3's forget snapshot does not reach — xh1) stays attached and resolves as before.
                    // A full deep_fresh_copy fixes the free-var but re-orphans a nested `#seed`; the by-reference
                    // original fixes `#seed` but aliases the free-var — `deep_fresh_copy_keep_seed` does both.
                    let init = deep_fresh_copy_keep_seed(db, next_state);
                    ctx.pending.borrow_mut().push((sname, init));
                    sref
                } else {
                    deep_fresh_copy(db, next_state)
                };
            cur[slot] = next_state;
            // Wrap the perform's result VALUE in the op-arg `#cv` let-lifts (innermost binding = last lifted
            // arg, so bindings evaluate in arg order): `(let ((#cv (A.get))) <value with v↦#cv>)`. The foreign
            // perform now runs ONCE as the let-init (the enclosing fold discharges it), and B's arm reads the
            // pure `#cv` however many times. Nothing lifted → `value` unchanged (byte-identical common case).
            let value = if arg_lifts.is_empty() {
                value
            } else {
                let mut wrapped = value;
                for (binder, arg) in arg_lifts.into_iter().rev() {
                    let let_head = db.push_name("let");
                    let pair = db.push_list(vec![binder, arg]);
                    let bindings = db.push_list(vec![pair]);
                    wrapped = db.push_list(vec![let_head, bindings, wrapped]);
                }
                wrapped
            };
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
            // LET-LIFT over the continuation. A NON-FINAL do item that inlines a cross-fn effectful helper
            // whose body is a `(let ((x e)) lbody)` — the memoize combinator `store(k) = (let v = … in
            // (put; v))` — binds `x` LOCAL to that item. But `x` is referenced by the perform's OUT-STATE
            // (`put`'s next-state `Map.insert(s, k, x)`) + its substituted arg, which the `do` threads FORWARD
            // to the LATER items. Spliced there, `x` is OUTSIDE its `let` → a spurious CDZ0101 "unbound x"
            // (the sequenced-memoize leak; a FINAL-position such item is fine — nothing threads its state on).
            // Lift the `let` to wrap the whole continuation: `(do (let ((x e)) lbody) rest…)` ≡ `(let ((x e))
            // (do lbody rest…))`, so `x` scopes over `lbody` AND `rest` (whose out-state references it).
            // Sound: the init runs once, before the body + rest, in the same order — only `x`'s visibility
            // widens. Detect via an inline-PREVIEW (β-reduce the item's callee body with its args) so we see
            // the `let` the inline WILL produce; rewrite + re-thread the whole node (fixpoint lifts a second).
            for (i, &it) in items.iter().enumerate() {
                if i + 1 >= items.len() {
                    break; // the FINAL item's state is not threaded on — no escape, no lift needed
                }
                if let Some((binds, lbody)) = inlined_let_of_do_item(db, it, ctx) {
                    let do_head = db.push_name("do");
                    // `(do lbody rest…)` — the let body followed by the remaining items.
                    let mut cont = vec![do_head, lbody];
                    cont.extend_from_slice(&items[i + 1..]);
                    let cont_do = db.push_list(cont);
                    let let_head = db.push_name("let");
                    let lifted = db.push_list(vec![let_head, binds, cont_do]);
                    // Keep the items BEFORE `i` as the outer do prefix (they run first, in order); if `i==0`
                    // the lifted `let` IS the whole node.
                    let rewritten = if i == 0 {
                        lifted
                    } else {
                        let do_head2 = db.push_name("do");
                        let mut ch = vec![do_head2];
                        ch.extend_from_slice(&items[..i]);
                        ch.push(lifted);
                        db.push_list(ch)
                    };
                    return thread_bounded(db, rewritten, states, ctx, inline_depth);
                }
            }
            let mut cur = states;
            let mut last = None;
            // NON-FINAL items that, after rewriting, STILL reach a FOREIGN perform (an effect THIS handler
            // does not discharge) must be PRESERVED — dropping them loses a residual side effect. The
            // collapse-to-last shortcut is sound ONLY for an item that folded to a PURE expression (its
            // discharged effect went into the threaded state). But an INNER handler folding a body
            // `(do (Outer.bump) (Outer.get))` sees `Outer.bump` as FOREIGN: it does not fold it into any
            // slot, so the rewrite leaves the perform intact — and dropping the non-final one silently
            // erases its state advance the moment the OUTER handler folds it (the do-sequenced-outer-perform-
            // under-inner-handle miscompile: `(A.bump)` discarded under an inner `B` reads back stale). Keep
            // each such survivor and rebuild a `do` over them; the enclosing fold re-threads it (or, for a
            // host-delegated foreign op, it lowers as a host-call sequence). A pure-folded item is still
            // dropped — byte-identical to before for the single-handler-depth surface.
            let items_len = items.len();
            let mut kept: Vec<StructId> = Vec::new();
            let abort_before_do = ctx.abort_value.get();
            for (i, it) in items.into_iter().enumerate() {
                let (r, next) = thread_bounded(db, it, cur, ctx, inline_depth)?;
                cur = next;
                // A NON-FINAL item that ABORTS ends the sequence: everything AFTER it is DEAD (never runs at
                // runtime — the abort abandons the continuation). Its rewrite `r` is the abort value, which
                // becomes the do's value. Stop here — do NOT thread the dead suffix (threading it would run
                // its performs / push pending self-call temps for code that never executes, and letting the
                // loop continue would OVERWRITE `last` with the dead FINAL item's rewrite, dropping the abort
                // value → `(do (A.tick) (B.bail 99) (A.tick))` under B mis-yielded the trailing dead `(A.tick)`
                // as the do-value instead of 99, forcing the dead tick: 23/34 instead of 110). A non-final
                // abort at the LAST item falls through to the `i+1==items_len` arm normally.
                if i + 1 < items_len && ctx.abort_value.get() != abort_before_do {
                    last = Some(r);
                    break;
                }
                if i + 1 == items_len {
                    last = Some(r);
                } else if ctx.abort_value.get().is_none()
                    && body_reaches_foreign_perform(db, r, ctx)
                {
                    // KEEP a non-final residual-foreign-perform item — BUT only while no abort has fired.
                    // An ABORTIVE non-final item (`(do (Halt.stop n) …)`, `stop` never resumes) sets the
                    // abort cell and returns the abort VALUE as `r` — a `copy_pure` of the arm body, an
                    // ORPHAN (parent `None`) not yet reparented, carrying live names (the arm `(list v v v)`
                    // with `v`↦ the perform's runtime arg `n`). `body_reaches_foreign_perform` runs
                    // `resolved_of` over it, which RESOLVE-PINS `n` as unbound against the orphan scope —
                    // and that pin outlives `reduce_handle`'s later `reparent_under_handle_site`, surfacing a
                    // spurious CDZ0101 "unbound n" (the abortive-heap-list regression). Once an abort has
                    // fired the sequence is ABANDONED anyway — `reduce_handle` returns the abort value and
                    // discards this do-arm's result — so there is nothing to preserve. Skip the check.
                    kept.push(r);
                }
            }
            let last = last.unwrap();
            if kept.is_empty() {
                Some((last, cur))
            } else {
                kept.push(last);
                let do_head = db.push_name("do");
                let mut ch = vec![do_head];
                ch.extend(kept);
                Some((db.push_list(ch), cur))
            }
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
            // ACCUM-COPY REDIRECT (rn post-observer fix, increment 1). When `head` names a seed-wrapper this
            // merged context redirects to its accum COPY `f$acc` (which has extra accumulator params), the
            // wrapper's own call `(f 1)` supplied only the ORIGINAL args — so append the accumulator SEEDS
            // (read from the wrapper body `(f$acc orig… seed…)`) here, positioned right after the original
            // args and BEFORE captures/states, matching `f$acc`'s sig layout `[orig… seed… captures… states…]`.
            // A self-call INSIDE `f$acc` names `f$acc` directly (not the wrapper), so `accum_seed_redirect`
            // returns `None` for it and it passes its own accumulator arg — no double-seed. Empty (no-op) for
            // every non-redirected call.
            if let Some(head_def) = callee_def_index_of(db, head)
                && let Some((_acc, seeds)) = accum_seed_redirect(db, head_def, ctx.slots.len())
            {
                rargs.extend(seeds);
            }
            let spec = specialize_recursive(db, head, ctx)?;
            // CAPTURED enclosing-fn params come AFTER the original args and BEFORE the state args (the sig
            // layout). Each is passed as a fresh bare-name reference: inside `f#ctx` it resolves to the new
            // capture param; at the INITIAL call from the handle body it resolves to the enclosing fn's param
            // (`run-with`'s `tool`). They are CONSTANT across the recursion, so the same name is passed every
            // call — no threading. This one arm handles both the internal self-calls and the initial call.
            if let Some(captures) = db.effect_spec_captures.get(&spec).cloned() {
                for name in captures {
                    rargs.push(db.push_name(&name));
                }
            }
            // One trailing state arg per slot, in slot order — each a FRESH copy of the incoming state node.
            // The state node `cur[slot]` is ALSO returned as this call's out-state (single-return: `cur`
            // unchanged), so a LATER position that threads against that out-state — a mutual-partner call's
            // let-body performing the discharged op, whose perform arm β-reduces the state node into its
            // next-state — RE-PARENTS the shared node in the single-parent arena, orphaning THIS call's
            // trailing-arg occurrence and leaking the internal `f#eff{n}$s{k}` name in a CDZ0101 (the
            // mutual-cycle `compute#eff5$s0` leak). A fresh copy per call arg keeps the call's embedded state
            // distinct from the out-state that flows forward. `deep_fresh_copy` (not `copy_pure`) for the
            // same reason as the `if`/`match`/perform arms: a resolve-pinned `$s{k}` ref must be re-pushed
            // unpinned so it re-resolves against the specialized def's sig, not shared as a pinned node.
            for &s in cur.iter() {
                let fresh = deep_fresh_copy(db, s);
                rargs.push(fresh);
            }
            // Build the call `(<spec-name> args… state…)`. The specialized def is named, so a name atom
            // resolves to it (via `def_by_name`), and the ordinary recursive `Core::Call` + reachability
            // path emits it.
            let name_atom = db.push_name(&spec);
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
            // call in the other; and the effectful-helper-with-a-conditional-perform-in-a-self-call-arg case,
            // where the inlined helper's `if` threads the state into both branches). `deep_fresh_copy` (NOT
            // `copy_pure`) is required: `copy_pure` = `beta_reduce`, whose pinned-name fast path returns a
            // RESOLVE-PINNED state-ref (`f#ctx$s{k}`, pinned when the inlined helper body was resolved) AS-IS
            // — so both branches share the one pinned node, re-orphaning it. `deep_fresh_copy` re-pushes the
            // leaf fresh (unpinned), giving each branch a genuinely independent node that re-resolves against
            // the specialized def's sig (which declares `$s{k}`).
            let then_states: Vec<StructId> = cur.iter().map(|&s| deep_fresh_copy(db, s)).collect();
            let else_states: Vec<StructId> = cur.iter().map(|&s| deep_fresh_copy(db, s)).collect();
            let (rthen, then_out) =
                thread_branch_local_abort_with_out(db, then_, then_states, ctx, inline_depth)?;
            let (relse, else_out) =
                thread_branch_local_abort_with_out(db, else_, else_states, ctx, inline_depth)?;
            let if_head = db.push_name("if");
            let rif = db.push_list(vec![if_head, rcond, rthen, relse]);
            // MERGE the per-branch out-states. Only ONE branch runs at runtime, so the `if`'s out-state for
            // each slot is that branch's out-state, selected by the SAME condition: `(if cond then_out
            // else_out)`. This lets a SIBLING that reads state AFTER the conditional (a recursive-call operand,
            // a `do`-continuation) observe a branch perform's advance — the recursive-branch-perform fix
            // (self-recursive rw1/rw3/rw5 + the through-block fold; operator-prioritized). When NEITHER branch
            // advanced a slot (`then_out[i] == else_out[i] == cur[i]` — the common no-branch-perform case, e.g.
            // countdown), the merge collapses to `cur[i]` unchanged, so those working cases are byte-identical.
            // GATED on a PURE condition (`!subtree_performs(cond)`): only a pure `cond` is safely re-usable in
            // the state selector. A PERFORMING condition is lifted to `(let ((#cv COND)) (if #cv t e))` — its
            // threaded `rcond` is a `#cv` reference bound by a `let` that wraps ONLY the value-if, so re-using
            // it in the state position (outside that `let`) leaks the `#cv` name (CDZ0101). Those
            // performing-condition cases (the short-circuit `or`/`and` connectives desugared to an `if`) were
            // ALREADY correct with the post-condition `cur` out-state (the condition's own performs advanced
            // `cur`; the branches there are pure `true`/`false`/value literals), so skipping the merge for them
            // is both necessary (avoid the leak) and sound (no branch advance to propagate). The condition is
            // pure here, evaluated once as the value-if's head — re-using it in the state selector duplicates
            // no effect (the same soundness the `match` branch-dependent-next-state peel relies on).
            let cond_pure = !subtree_performs(db, cond, ctx) && !contains_cv_ref(db, rcond);
            let merged: Vec<StructId> = cur
                .iter()
                .zip(then_out.iter().zip(else_out.iter()))
                .map(|(&c, (&t, &e))| {
                    // SAFE to merge this slot only when: (1) the condition is PURE (a performing cond is
                    // lifted to `(let ((#cv COND)) …)`, so `rcond` = a `#cv` ref bound by a `let` wrapping only
                    // the value-if — re-using it in the state position leaks the name); AND (2) NEITHER branch
                    // out-state references a synthesized `#cv` (a branch whose sub-position performs is itself
                    // `#cv`-lifted INSIDE the branch, so its out-state carries a branch-local `#cv` that is
                    // out of scope in the merged state position — the leak the connective-desugar hit). A
                    // clean branch out-state (rw1's `(if true (St.get) 0)` → then_out is a plain resume
                    // next-state, no `#cv`) merges safely. When unsafe or when neither branch advanced the
                    // slot, keep `cur` (byte-identical to the pre-fix behavior — no regression).
                    //
                    // A BRANCH MUST ACTUALLY PERFORM to have advanced state (breaker en1 fix). The
                    // `(t != c || e != c)` test alone is a NODE-IDENTITY compare — but the branches were
                    // threaded over `deep_fresh_copy`(cur) (5140-5141), so `then_out`/`else_out` are FRESH
                    // node ids ALWAYS distinct from `c` even when the branch performed NOTHING and its
                    // out-state is semantically just `cur`. Without the perform gate, a pure branching
                    // conditional in the handle-body tail — the `(if (>= r 100) r 0)` LET-BODY of an inlined
                    // performing helper `(let ((r (+ x (St.bump)))) (if …))` — spuriously builds the state
                    // selector `(if cond r 0)` from its VALUE branches (`r`,`0`), which carry NO state advance.
                    // That value-`if` then rides forward as the slot's next-STATE (cur[slot] = the if), and a
                    // later dispatch substitutes it back as its resume value → the let-body `if` lands in the
                    // let's BINDINGS, the binder `r` referenced in the if-cond is structurally unreachable →
                    // false CDZ0101 "unbound r". Requiring an actual branch perform restores the invariant
                    // "state out of a pure conditional = cur unchanged": neither branch performs → no advance
                    // → keep `c` (en1 folds; the pure analog already did). A genuine branch perform (rw1/rw3:
                    // `(if c (St.get) 0)`) still merges — its out-state is a real threaded resume next-state.
                    let branch_advanced =
                        subtree_performs(db, then_, ctx) || subtree_performs(db, else_, ctx);
                    let mergeable = cond_pure
                        && branch_advanced
                        && (t != c || e != c)
                        && !contains_cv_ref(db, t)
                        && !contains_cv_ref(db, e);
                    if mergeable {
                        let ifh = db.push_name("if");
                        let condc = deep_fresh_copy(db, rcond);
                        let tc = deep_fresh_copy(db, t);
                        let ec = deep_fresh_copy(db, e);
                        db.push_list(vec![ifh, condc, tc, ec])
                    } else {
                        c
                    }
                })
                .collect();
            Some((rif, merged))
        }
        // A `(match scrutinee (pattern body)…)` — the analogue of `if` for the pattern engine. Thread the
        // SCRUTINEE (a perform there reads/threads state, `(match (Get.next) …)`), then rewrite each arm:
        // the PATTERN is a binder position (copied structurally, never threaded — like a `let` binder), the
        // BODY is threaded under the post-scrutinee state (only one arm runs, so each sees the same incoming
        // state, mirroring the `if` branches). An abortive perform in an arm BODY tail is branch-local — the
        // `match` IS the handle body's value, so per-arm the abort yields the arm value — captured by
        // `thread_branch_local_abort` (which restores the cell so a sibling arm / the handle is not
        // collapsed). The out-state MERGES the per-arm out-states into a match-valued out-state `(match
        // scrut (pat arm-out)…)` — the `Match` analogue of the `if` per-branch out-state merge below — so a
        // SIBLING reading state after the match (a recursive-call operand) observes an arm perform's advance
        // (the recursive-branch-perform self-recursive miscompile's match-arm face, #1993); when no arm
        // advanced a slot the merge collapses to the incoming state unchanged. Rebuild the same `(match
        // rscrut (pat rbody)…)` form so the pattern engine lowers it by the ordinary path.
        Resolved::Match { scrutinee, arms } => {
            let abort_before_scrut = ctx.abort_value.get();
            let (rscrut, cur) = thread_bounded(db, scrutinee, states, ctx, inline_depth)?;
            // SCRUTINEE ABORT collapses the whole match (the abort-outer-advance class, scrutinee face). The
            // scrutinee is evaluated BEFORE any arm; if it ABORTS — `(match (do (A.tick) (B.bail 99)) …)` under
            // B — no arm runs, so the match's value is the scrutinee's rewrite. `thread_bounded` already
            // produced the sound `(do (A.tick) 99)` (the do-arm kept the pre-abort foreign `A.tick`); return
            // it directly so the ENCLOSING fold discharges the foreign prefix (advancing the outer state)
            // rather than wrapping the aborted scrutinee in a dead `(match … arms)` whose bare-abort collapse
            // would drop `A.tick` (109 vs 110). Only when a NEW abort fired threading the scrutinee (cell
            // flipped) — a non-aborting scrutinee falls through to the ordinary per-arm threading below.
            if ctx.abort_value.get() != abort_before_scrut {
                return Some((rscrut, cur));
            }
            let match_head = db.push_name("match");
            let mut children = vec![match_head, rscrut];
            // Collect each arm's (pattern, out-state) so the arm out-states can be MERGED into a match-valued
            // out-state (the `Match` analogue of the `if` per-branch out-state merge — see that arm). An
            // arm-body perform advances the state, and a sibling reading state after the match (a recursive
            // call operand) must see that advance; without the merge the match returns the post-SCRUTINEE
            // state and the advance is dropped (the recursive-branch-perform self-recursive miscompile's
            // match-arm face). Each arm's out-state is captured alongside its rewrite.
            let mut arm_outs: Vec<Vec<StructId>> = Vec::with_capacity(arms.len());
            let mut arm_pats: Vec<StructId> = Vec::with_capacity(arms.len());
            for (pat, body) in arms.iter() {
                let (pat, body) = (*pat, *body);
                // The pattern binds names for the arm body (a binder position) — copy it structurally so it
                // is self-contained, exactly as a `let` binder name is copied (never substituted/threaded).
                let rpat = copy_pure(db, pat);
                // Each arm gets its OWN FRESH COPY of the incoming state-refs — the same single-parent-arena
                // reason as the `if` branches: an arm body EMBEDS the state (a perform substitutes it into a
                // resume value; a recursive/mutual call appends it as a trailing state arg), so sharing one
                // state-ref node across arms orphans whichever is parented second, leaking the internal
                // `f#ctx$s0` name (a mutual group dispatched by `match` with the perform in one arm and the
                // mutual call in another; and the conditional-perform-helper-in-a-self-call-arg case).
                // `deep_fresh_copy` (NOT `copy_pure`) for the same reason as the `if` arm above — `copy_pure`
                // shares a resolve-pinned `$s{k}` ref across arms; `deep_fresh_copy` gives each an unpinned
                // fresh node that re-resolves against the spec sig.
                let arm_states: Vec<StructId> =
                    cur.iter().map(|&s| deep_fresh_copy(db, s)).collect();
                let (rbody, arm_out) =
                    thread_branch_local_abort_with_out(db, body, arm_states, ctx, inline_depth)?;
                children.push(db.push_list(vec![rpat, rbody]));
                arm_pats.push(pat);
                arm_outs.push(arm_out);
            }
            let rmatch = db.push_list(children);
            // MERGE the per-arm out-states into a `(match scrut (pat arm-out)…)`-valued out-state per slot —
            // the `Match` analogue of the `if` merge. Only ONE arm runs, selected by the SAME (pure, already-
            // evaluated) scrutinee, so the slot's out-state is that arm's out-state under the matching pattern.
            // GATED identically to the `if` arm: a PURE scrutinee (a performing one is `#cv`-lifted, whose ref
            // can't be re-used in the state position) with `#cv`-free arm out-states; collapses to `cur[i]`
            // when NO arm advanced the slot (byte-identical to the pre-fix post-scrutinee behavior — working
            // cases unchanged, no regression). The scrutinee is re-used in the state selector, duplicating no
            // effect (pure) — the same soundness the `match` branch-dependent-next-state peel relies on.
            let scrut_pure = !subtree_performs(db, scrutinee, ctx) && !contains_cv_ref(db, rscrut);
            let merged: Vec<StructId> = (0..cur.len())
                .map(|i| {
                    let c = cur[i];
                    let advanced = arm_outs.iter().any(|o| o[i] != c);
                    let cv_clean = arm_outs.iter().all(|o| !contains_cv_ref(db, o[i]));
                    if scrut_pure && advanced && cv_clean {
                        let mh = db.push_name("match");
                        let sc = deep_fresh_copy(db, rscrut);
                        let mut ch = vec![mh, sc];
                        for (a, &pat) in arm_pats.iter().enumerate() {
                            let pc = copy_pure(db, pat);
                            let oc = deep_fresh_copy(db, arm_outs[a][i]);
                            ch.push(db.push_list(vec![pc, oc]));
                        }
                        db.push_list(ch)
                    } else {
                        c
                    }
                })
                .collect();
            Some((rmatch, merged))
        }
        // A short-circuit connective `(and lhs rhs)` / `(or lhs rhs)` whose rhs runs only conditionally on
        // `lhs`. Threading it as a strict two-operand form would evaluate rhs's perform even when `lhs`
        // short-circuits — an observable-effect change. Instead DESUGAR to the equivalent `if` (`(and lhs
        // rhs)` ≡ `(if lhs rhs false)`, `(or lhs rhs)` ≡ `(if lhs true rhs)`) and re-thread, so rhs is a
        // branch (threaded under the post-`lhs` state, run only on the taken path). `lhs` becomes the `if`
        // condition — evaluated exactly once either way. Only reached when a perform is inside (a pure
        // connective is copied wholesale by the pure-subtree arm below).
        Resolved::And { lhs, rhs, is_and } => {
            let if_head = db.push_name("if");
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
            let not_head = db.push_name("not");
            Some((db.push_list(vec![not_head, roperand]), cur))
        }
        // A tuple PROJECTION `(. operand index)` — STRICT one-operand. Thread the operand (a perform there
        // reads/threads state, `(. (tuple (Get.next) (Get.next)) 1)`), rebuild the same projection. The
        // index is a literal (copied structurally). `push_list` with the same `.`-head + index re-forms it.
        Resolved::Proj { operand, index } => {
            let (roperand, cur) = thread_bounded(db, operand, states, ctx, inline_depth)?;
            let dot = db.push_name(".");
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
            let dot = db.push_name(".");
            let key_atom = db.push_name(&key.name);
            Some((db.push_list(vec![dot, roperand, key_atom]), cur))
        }
        // An annotation `(: expr T)` — STRICT one-operand (the type is not runtime code). Thread `expr`,
        // rebuild `(: rexpr T)` with the type expression copied structurally.
        Resolved::Annot { expr, ty_expr } => {
            let (rexpr, cur) = thread_bounded(db, expr, states, ctx, inline_depth)?;
            let colon = db.push_name(":");
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
            let head = db.push_str(ctor);
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
                // A field is the canonical `(= label value)` ascription triple (Phase B) — label = child
                // 1, value = child 2, rebuilt WITH the `=` head — or a legacy `(label value)` pair.
                let (eq_head, label_id, value_id) =
                    if kv.len() == 3 && db.ast.as_name(kv[0]) == Some("=") {
                        (Some(kv[0]), kv[1], kv[2])
                    } else if kv.len() == 2 {
                        (None, kv[0], kv[1])
                    } else {
                        return None;
                    };
                // The label is copied structurally (a field name, not a value to thread); the VALUE is
                // threaded (it may perform). The `=` head (if any) is preserved so the rebuilt field keeps
                // the canonical triple shape.
                let label_copy = copy_pure(db, label_id);
                let (rvalue, next) = thread_bounded(db, value_id, cur, ctx, inline_depth)?;
                cur = next;
                let rebuilt = match eq_head {
                    Some(eq) => {
                        let eq_copy = copy_pure(db, eq);
                        db.push_list(vec![eq_copy, label_copy, rvalue])
                    }
                    None => db.push_list(vec![label_copy, rvalue]),
                };
                rfields.push(rebuilt);
            }
            let head = db.push_str("record");
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
            let head = db.push_str("map");
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
            // CAPTURED-VALUE INLINE (the closure-capture-reperform fix), gated on the let BODY being a
            // RETURNED LAMBDA — the ONLY shape that suffers capture orphaning. A returned lambda capturing a
            // binding — `(let ((base (Ctr.tick))) (fn (x) (+ x base)))` — has its body `copy_pure`d by the
            // `thread_bounded` no-perform arm, DETACHING `base` (it resolves to Poison-unbound in the copy),
            // so a substitution AFTER the copy has no resolvable `Ref` to match. We instead SUBSTITUTE each
            // pure threaded init into the ORIGINAL body BEFORE threading copies it (below), where `base`
            // still resolves to `Ref { value: kv[1] }` (the init occ, per inc-2b-1) — closing the lambda over
            // the FOLDED value (`(fn (x) (+ x 50))`), so when it escapes + is re-reduced by `lambda_of`/
            // `apply_lambda` (which re-derive from source, bypassing this threaded binding) it carries the
            // value, not the perform. Gated on the closure shape SYNTACTICALLY so a non-closure let body (a
            // recursive multi-value fold, an arithmetic tail, …) is byte-identical — the collect below (and
            // its `strongly_pure` probes, which touch shared caches) only runs for the closure case.
            // A body reached through NESTED `let`s ending in a lambda — `(let ((a (Ctr.tick))) (let ((b
            // …)) (fn …)))`, an outer capture referenced by a closure buried in an inner let — counts too,
            // so peel let-chains (`body_returns_lambda`). Without this the outer `let`'s body (an inner
            // `let`, not a lambda) failed the gate, and the outer capture `a` orphaned → CDZ0101.
            let body_is_closure = body_returns_lambda(db, body_occ);
            let mut cur = states;
            let mut rpairs = Vec::with_capacity(pairs.len());
            let mut capture_subst: HashMap<StructId, StructId> = HashMap::default();
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
                let init_abort_before = ctx.abort_value.get();
                let (rinit, next) = thread_bounded(db, kv[1], cur, ctx, inline_depth)?;
                cur = next;
                // LET-INIT ABORT COLLAPSE (breaker ax7/ah-x3). If threading THIS init fired an abort — the
                // init `(+ (A.tick) (B.bail 99))` / `(do (ask.ask) (Bail.bail 7))` aborts — the `let` never
                // binds the name and the body + remaining bindings are DEAD (the abort abandons the
                // continuation). `rinit` is already the sound rewrite the abort-lift produced: a `(do
                // <pre-abort-foreign-prefix> abort-value)` (or the bare abort value), carrying any committed
                // foreign advance / issued host call. Return it DIRECTLY as the let's value — do NOT push it
                // as a binding + continue to the body, which would (a) bind the name to the do-tail and run
                // `(+ x 1)` on it (a wrong value, ax7 → 109) and (b) leave the foreign prefix buried in a dead
                // binding the enclosing collapse discards (dropping A.tick's advance / never issuing the host
                // call, ah-x3). The abort cell stays SET so the enclosing fold collapses to this value. This
                // is the let-INIT analog of the strict-operand abort-lift (the body-abort case is handled by
                // the ABORT-VALUE RE-SCOPE below; this is the INIT-abort case it missed — the `abort_before`
                // snapshot there is taken AFTER this loop, so an init abort slipped through).
                if init_abort_before.is_none() && ctx.abort_value.get().is_some() {
                    // PRESERVE the EARLIER bindings' committed effects (breaker ax12). The bindings BEFORE
                    // the aborting one (`rpairs`, already threaded) ran on the strict spine before the abort,
                    // so an earlier binding whose init committed a FOREIGN advance / host call — `(let ((y
                    // (A.tick)) (x (+ 1 (B.bail 99)))) …)`, where `y=(A.tick)` advances the OUTER A-state
                    // before `x`'s abort — must survive. Returning the bare `rinit` (the abort value) drops
                    // them, losing `y`'s advance (109 vs 110). Re-wrap the abort value in a `let` over the
                    // earlier bindings so they run for-effect (and remain in scope, since the abort value may
                    // reference an earlier binder) before the abort value. When there are no earlier bindings
                    // (the aborting init is the FIRST) this is a bare `rinit` — the ax7 case, unchanged. The
                    // enclosing fold's do/bare-abort collapse then discharges the wrapping `let`'s foreign
                    // inits (advancing the outer state) before yielding the value.
                    // Collect the earlier bindings' INITS that committed a FOREIGN effect (a foreign
                    // perform / host call) — the pure ones commit nothing and are dropped. Sequence them as
                    // a for-effect `do` PREFIX before the abort value, so the enclosing fold discharges them
                    // (advancing the outer state / issuing the host call) before yielding the abort value. A
                    // `let`-WRAP would not work: an unused binding whose init performs gets dead-code
                    // dropped, losing the effect (ax12 stayed 109). Reuse each init's ALREADY-THREADED
                    // rewrite (`rpairs[i]` = `(name rinit_i)`, take the init `rinit_i`).
                    let mut foreign_prefix: Vec<StructId> = Vec::new();
                    for rp in &rpairs {
                        if let Struct::List(kv2) = db.ast.get(*rp).clone()
                            && kv2.len() == 2
                            && body_reaches_foreign_perform(db, kv2[1], ctx)
                        {
                            foreign_prefix.push(kv2[1]);
                        }
                    }
                    if foreign_prefix.is_empty() {
                        return Some((rinit, cur));
                    }
                    let do_head = db.push_name("do");
                    let mut ch = vec![do_head];
                    ch.extend(foreign_prefix);
                    ch.push(rinit);
                    return Some((db.push_list(ch), cur));
                }
                // A pure threaded init captured by a returned-lambda body → substitute it into the body
                // (see the block comment above). GATED to the closure shape + an effect-FREE init (an
                // effectful init would duplicate its effect on inline).
                if body_is_closure && strongly_pure(db, rinit, ctx) {
                    capture_subst.insert(kv[1], rinit);
                }
                // NESTED-LET INIT LET-LIFT (the effectful-let capture fix): if the threaded init is itself a
                // `(let ((x e)…) lbody)` — the shape an INLINED effectful helper produces, `let b = inner()`
                // where `inner`'s body is `(let a = S.get() in …)` — LIFT the inner bindings UP into THIS
                // `let`'s binding list (before this pair), and bind this name to the inner `lbody`. WHY: the
                // threaded out-state `cur` may REFERENCE an inner binder (`put(a)` threads its arg `a` as the
                // next state), and that out-state is spliced into the LATER bindings/body of THIS let; left
                // inside `b`'s init-`let`, `a` is out of scope there → a spanless CDZ0101 (the nested-eff-let
                // bug: `(let ((b (let ((a 10)) …))) (+ b a))` — the outer body's re-perform resolved to `a`).
                // Lifting makes `a` a sibling binding of `b`, in scope for the continuation. Sound: the inner
                // `let`'s inits already ran (in order) to produce `b`'s value; hoisting only WIDENS their
                // visibility, it does not reorder or duplicate them (a `let` binding list is sequential, same
                // as nesting). Only fires for a threaded init that IS a `let` (the inline/perform case); a
                // plain init is pushed as-is. Mirrors the `do`-arm let-lift (`inlined_let_of_do_item`).
                if let Some(inner) = db.ast.as_form(rinit, "let").map(|t| t.to_vec())
                    && inner.len() == 2
                    && let Struct::List(inner_pairs) = db.ast.get(inner[0]).clone()
                {
                    for ip in inner_pairs {
                        rpairs.push(ip);
                    }
                    rpairs.push(db.push_list(vec![name_copy, inner[1]]));
                } else {
                    rpairs.push(db.push_list(vec![name_copy, rinit]));
                }
            }
            // Close any captured pure binding into the body BEFORE threading detaches it (see the collect
            // comment above). A no-op when nothing captured (non-closure body → empty subst).
            let body_occ = if capture_subst.is_empty() {
                body_occ
            } else {
                crate::eval::beta_reduce(db, body_occ, &capture_subst)
            };
            // Snapshot the abort cell BEFORE threading the body, so we can tell if THIS body fires an abort.
            let abort_before = ctx.abort_value.get();
            let (rbody, cur) = thread_bounded(db, body_occ, cur, ctx, inline_depth)?;
            let let_head = db.push_name("let");
            let rbindings = db.push_list(rpairs);
            let wrapped = db.push_list(vec![let_head, rbindings, rbody]);
            // ABORT-VALUE RE-SCOPE. If threading the body FIRED AN ABORT (`(let ((v e)) (Bail.bail v))` — the
            // abortive arm `(bail (n) s n)` materializes `v` as the abort value), `reduce_handle` collapses
            // the whole handle to that abort value and DISCARDS this `let` wrapper — so a `v` in the abort
            // value orphans → spurious CDZ0101 (the abortive-perform-referencing-a-body-local-binding bug,
            // the abortive twin of the do-def-in-perform-arg fix). Re-wrap the abort value in THIS let's
            // bindings (and update the cell) so it carries the same scope + init evaluation the discarded
            // wrapper would have. Fired UNCONDITIONALLY whenever this body set the abort cell (`abort_before`
            // was None and it is now Some) — NOT gated on the value's free names: the rewrap preserves the
            // bindings' EVALUATION as well as their name scope, so re-wrapping even a closed abort value (a
            // bare-param abort like `(Bail.bail u)`, whose value binds no `let` name) is still correct — the
            // extra `let` is inert but harmless, and NOT special-casing it keeps this branch simple and
            // uniform. Sound: the bindings' inits ran (pure, in order) before the perform on this spine, so
            // scoping the abort value under them changes no evaluation order; it only restores what the
            // collapsed wrapper carried.
            if abort_before.is_none()
                && let Some(av) = ctx.abort_value.get()
            {
                let rewrapped = db.push_list(vec![let_head, rbindings, av]);
                ctx.abort_value.set(Some(rewrapped));
            }
            Some((wrapped, cur))
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
                // CALLER-OBSERVED OUT-STATE under the MERGE (recursive-nested-arm-resume fix). `reduce_handle`
                // runs `mark_caller_observed_outstate` for the OUTER ctx, but the merged body threads HERE
                // (not via reduce_handle), so a post-recursion sibling that observes the merged callee's
                // out-state — `(+ (loop 1) (A.get))`, where `(A.get)` reads the A-advance `loop` made per
                // iteration — is not marked, and the merged spec stays single-return (dropping the advance:
                // 20 vs 21). Mark it under the MERGED ctx so `specialize_recursive` emits multi-value and
                // threads the outer out-state to the observer. Additive (only upgrades a threadable callee).
                mark_caller_observed_outstate(db, inner_body, &merged);
                // Thread the inner body under the merged context, with the inner slot seeded by its init
                // (appended after the outer states). The merged vector = outer states ++ [inner init].
                let mut merged_states = states.clone();
                merged_states.push(inner_init);
                // DRAIN the merged body's pending MULTIVALUE temps (rn post-observer fix, increment 1 tail).
                // A caller-observed merged spec call in `inner_body` — `(+ (loop 1) (A.get))`, where the
                // post-loop `(A.get)` observes `loop`'s A-advance — emits a multi-value spec call let-bound to
                // `{spec}$t{k}`, pushed to `merged.pending`, with `(. t 0)` in its place. The single-handler
                // path drains these at reduce_handle's tail (line ~2528), but the MERGED body threads HERE via
                // `thread_bounded`, which never drained → `(. loop$acc#eff$t0 0)` referenced an unbound `$t0`
                // (CDZ0101). Mark the pending length, thread, then `drain_and_wrap` binds every temp this body
                // produced into wrapping `let`s around `rbody`. No-op when nothing pending (byte-identical to
                // before for a merged body with no multivalue call — the common case).
                let mark = merged.pending.borrow().len();
                let (rbody, out) =
                    thread_bounded(db, inner_body, merged_states, &merged, inline_depth)?;
                let rbody = drain_and_wrap(db, &merged, mark, rbody);
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
            let abort_before = ctx.abort_value.get();
            let mut cur = states;
            let (rhead, next0) = thread_or_copy(db, head, cur, ctx, inline_depth)?;
            cur = next0;
            let mut children = vec![rhead];
            // STRICT-OPERAND ABORT-LIFT (the operand face of the abort-outer-advance fix; do-shape landed
            // #2002). When a strict operand ABORTS — `(+ (A.tick) (B.bail 99))` under B — the operands
            // evaluated BEFORE it on the strict spine have already run; a FOREIGN one (an outer handler's
            // effect) committed its state advance and must survive the abort. But rebuilding `(+ (A.tick)
            // 99)` leaves the foreign perform inside a DEAD arithmetic wrapper that reduce_handle's bare-abort
            // collapse discards (the outer `(A.get)` then reads the pre-advance state → 109 vs 110). Lift the
            // pre-abort foreign operands into a for-effect `do` PREFIX around the abort value — `(do (A.tick)
            // 99)` — the SAME shape the do-arm produces, which the landed do-shape fold in reduce_handle then
            // preserves (the outer fold discharges the prefix, advancing the outer state, before the value).
            // Done in `thread` (not reduce_handle) because only here can we preserve UNCONDITIONALLY: sound for
            // BOTH the observed miscompile (→110) AND the correct-because-unobserved deep-nested case ("a
            // strict-operand abort in a DEEP-nested handler stack keeps its 99 when the advances are
            // UNOBSERVED") `(+ (A.a) (+ (B.b) (Bail.bail 99)))` (→ nested `(do (A.a) (do (B.b) 99))` = 99 still, the prefix runs
            // unobserved). `body_reaches_foreign_perform` reads the ORIGINAL operand `a` (parented — no orphan
            // resolve-pin poison, and equivalent to the rewrite: a foreign perform is never folded by this ctx,
            // a discharged one is not foreign). Only lifts when a foreign operand actually PRECEDED the abort
            // (`!kept_foreign.is_empty()`); a plain `(+ 5 (B.bail 99))` / `(+ (B.bail 7) (ask.ask))` keeps the
            // bare-abort collapse (nothing committed before the abort — 7, ask elided).
            let mut kept_foreign: Vec<StructId> = Vec::new();
            let mut abort_tail: Option<StructId> = None;
            for &a in args.iter() {
                let pre_abort = abort_before.is_none() && ctx.abort_value.get().is_none();
                let foreign = pre_abort && body_reaches_foreign_perform(db, a, ctx);
                let (ra, next) = thread_bounded(db, a, cur, ctx, inline_depth)?;
                cur = next;
                children.push(ra);
                if abort_before.is_none() && abort_tail.is_none() && ctx.abort_value.get().is_some()
                {
                    // THIS operand fired the abort (cell None→Some): `ra` is the abort value → the do-tail.
                    abort_tail = Some(ra);
                } else if abort_tail.is_none() && foreign {
                    // A pre-abort foreign operand: keep it for the for-effect prefix.
                    kept_foreign.push(ra);
                }
            }
            // NESTED-OPERAND face (breaker ax4): the aborting operand may ITSELF be an inner strict form
            // whose abort-lift already produced a `(do <foreign-prefix> abort-value)` tail — `(+ 999 (+
            // (A.tick) (B.bail 99)))` threads the inner `+` to `(do (A.tick) 99)`. This level's `kept_foreign`
            // is empty (999 pure), so rebuilding `(+ 999 (do (A.tick) 99))` buries the foreign prefix in a
            // DEAD arithmetic wrapper the bare-abort collapse discards → the advance is lost (109 vs 110,
            // silent cross-backend). The abort abandons the whole `+` anyway, so collapse to the tail
            // directly (dropping the dead pure siblings), preserving its do-prefix. NARROW: only when the
            // tail is ALREADY a `(do …)` (an inner abort-lift's output) — a bare-value tail keeps the
            // existing bare-abort collapse (which is correct + avoids perturbing the #seed-let scoping the
            // broad `reaches_foreign` gate broke — 9 regressions).
            let tail_is_lifted_do = abort_tail.is_some_and(|t| db.ast.as_form(t, "do").is_some());
            if let Some(tail) = abort_tail
                && (!kept_foreign.is_empty() || tail_is_lifted_do)
            {
                let do_head = db.push_name("do");
                let mut ch = vec![do_head];
                ch.extend(kept_foreign);
                ch.push(tail);
                return Some((db.push_list(ch), cur));
            }
            // OPERAND ABORT ABANDONS PURE SIBLINGS (breaker ax9/ah-x2). When an operand ABORTED and there
            // is no foreign prefix to preserve (`kept_foreign` empty) and the tail is a bare abort value
            // (not a lifted `do`), rebuilding `(op <pure siblings> <abort-value>)` — `(+ 999 99)` — SPLICES
            // the abort value into a dead arithmetic form. At the reduce_handle TOP LEVEL a downstream
            // bare-abort collapse reduces that away, but when this `(+ 999 (B.bail 99))` is a `do`-ITEM (or
            // any non-top position) nothing collapses it → the spliced `(+ 999 99)` = 1098 rides forward as
            // the value (ax9 → 1109). The abort abandons the whole operator application, so its value IS the
            // abort value: collapse to `tail` directly, dropping the pure siblings. Sound — the siblings are
            // pure (no `kept_foreign`), so dropping them discards no effect; the abort cell stays set so the
            // enclosing fold treats this as the abort. (A foreign sibling would have populated `kept_foreign`
            // → the do-prefix branch above; a lifted-do tail → that branch too. This is the bare-value,
            // pure-siblings-only residue.)
            if let Some(tail) = abort_tail {
                return Some((tail, cur));
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

/// Whether `node`'s value is a RETURNED LAMBDA, seen through `let`-CHAINS — a `(fn …)` directly, or the
/// body of a `(let (binds) <returns-lambda>)` (recursively). This is the shape that suffers CAPTURE
/// ORPHANING in the `let` thread arm: a closure buried at the end of one-or-more nested `let`s, capturing
/// a binding from an OUTER let, whose body is `copy_pure`d whole (detaching the capture). Gates the
/// captured-value inline (see the `let` arm). SYNTACTIC (no reduction) so it never perturbs a shared cache;
/// bounded by the arena's acyclic structure (a `let`-chain terminates). A NON-lambda tail (arithmetic, a
/// recursive multi-value fold) returns false, keeping every non-closure let-fold byte-identical.
fn body_returns_lambda(db: &mut Db, node: StructId) -> bool {
    match resolved_of(db, node) {
        Resolved::Lambda { .. } => true,
        _ => db
            .ast
            .as_form(node, "let")
            .map(<[_]>::to_vec)
            .filter(|tail| tail.len() == 2)
            .is_some_and(|tail| body_returns_lambda(db, tail[1])),
    }
}

/// finding #10 detector: whether `node` contains a `let` binding whose init is a CLOSURE OVER A PERFORMING
/// INNER-LET CAPTURE — `(let ((f (let ((a <perform>)…) (fn … a …)))) …)`. Such a closure re-derives its body
/// from source at each application (`apply_lambda`/`beta_reduce`), re-running the performing init and
/// discarding the once-evaluated draw — a silent per-application miscompile (breaker #10, ca1/ca1c). The
/// capture-once fold (thread the draw once, close over the result) is a later increment; this detects the
/// exact shape so `reduce_handle` declines it to the safe floor. NARROW: fires only when (1) the init is a
/// `(let (binds…) lambda-returning-body)` whose (2) at least one binding's init reaches a DISCHARGED or
/// FOREIGN perform AND (3) the returned lambda REFERENCES that binder. A closure whose captures are all pure
/// (d2fix), or a draw bound in a PLAIN let OUTSIDE the closure's init-let (d1, corpus-8688 direct handle-body
/// bindings), does not match — those bind the perform in a `let` the fold already threads once.
fn body_has_closure_over_performing_capture(db: &mut Db, node: StructId, ctx: &HandlerCtx) -> bool {
    // FACTORY-ARG entry (cc3): `init` is a call `(mk <performing-arg>…)` to a non-recursive helper that
    // RETURNS A LAMBDA closing over its param — `(def (mk (: m Int64)) (fn (x) (* x m)))`. The performing arg
    // (`(St.next)`) feeds the captured param, and the same `apply_lambda`/`beta_reduce` re-derivation re-runs
    // it per application (cc3: 116 not 80). Detect: a call whose callee body returns a lambda and whose an
    // argument reaches a discharged/foreign perform. Same fix locus, wider entry (finding #10, cc-batch).
    fn init_is_factory_over_performing_arg(db: &mut Db, init: StructId, ctx: &HandlerCtx) -> bool {
        let Resolved::Apply { head, args } = resolved_of(db, init) else {
            return false;
        };
        if is_perform(db, head, ctx).is_some() {
            return false; // a direct perform, not a factory call
        }
        let Some(callee) = crate::eval::lambda_body(db, head)
            .or_else(|| crate::eval::lambda_body_of_nullary(db, head))
        else {
            return false;
        };
        if crate::eval::is_recursive(db, callee) || !body_returns_lambda(db, callee) {
            return false;
        }
        // Some arg performs (discharged or foreign) → its once-drawn value is re-derived per application.
        args.iter().any(|&a| {
            subtree_reaches_discharged_op(db, a, ctx) || body_reaches_foreign_perform(db, a, ctx)
        })
    }
    // Is `init` a `(let (binds…) <returns-lambda>)` where a binding's init performs and the lambda refs it?
    fn init_is_performing_capture_closure(db: &mut Db, init: StructId, ctx: &HandlerCtx) -> bool {
        if init_is_factory_over_performing_arg(db, init, ctx) {
            return true;
        }
        let Some(inner) = db.ast.as_form(init, "let").map(|t| t.to_vec()) else {
            return false;
        };
        if inner.len() != 2 || !body_returns_lambda(db, inner[1]) {
            return false;
        }
        let Struct::List(pairs) = db.ast.get(inner[0]).clone() else {
            return false;
        };
        // Whether the returned-lambda body references a binder NAME (a by-name scan — the binder is a
        // synthesized `#a…`/user name, and resolution chains are unreliable here because the closure init is
        // not yet reparented, so `subtree_references_binder`'s resolve-chain match reads false; a name scan is
        // the robust check for this detector).
        fn refs_name(db: &Db, node: StructId, name: &str) -> bool {
            if db.ast.as_name(node) == Some(name) {
                return true;
            }
            match db.ast.get(node) {
                Struct::List(children) => children.iter().any(|&c| refs_name(db, c, name)),
                Struct::Atom(_) => false,
            }
        }
        pairs.iter().any(|&pair| match db.ast.get(pair).clone() {
            Struct::List(kv) if kv.len() == 2 => {
                let performs = subtree_reaches_discharged_op(db, kv[1], ctx)
                    || body_reaches_foreign_perform(db, kv[1], ctx);
                let refs = db
                    .ast
                    .as_name(kv[0])
                    .map(str::to_string)
                    .is_some_and(|nm| refs_name(db, inner[1], &nm));
                performs && refs
            }
            _ => false,
        })
    }
    // Scan every `let` binding init in the body for the shape.
    if let Some(form) = db.ast.as_form(node, "let").map(|t| t.to_vec())
        && form.len() == 2
        && let Struct::List(pairs) = db.ast.get(form[0]).clone()
    {
        for pair in pairs {
            if let Struct::List(kv) = db.ast.get(pair).clone()
                && kv.len() == 2
                && init_is_performing_capture_closure(db, kv[1], ctx)
            {
                return true;
            }
        }
    }
    // Recurse structurally so a nested occurrence (inside a branch, a deeper let body) is caught too.
    match db.ast.get(node).clone() {
        Struct::List(children) => children
            .iter()
            .any(|&c| body_has_closure_over_performing_capture(db, c, ctx)),
        Struct::Atom(_) => false,
    }
}

/// The capture-once REWRITE paired with `body_has_closure_over_performing_capture` (finding #10). Hoists
/// the performing init OUT of a let-bound closure's value-let so it wraps the binding:
/// `(let (… (f (let ((a <perform>)…) LAMBDA)) …) BODY)` becomes
/// `(let ((a <perform>)…) (let (… (f LAMBDA) …) BODY))`. After the hoist, `a` is a PLAIN let-init the
/// fold threads ONCE (the draw is discharged a single time, before the closure binding), and `f` is a
/// PURE-capture closure over the drawn RESULT — a shape the fold already folds, and multi-application
/// re-derives that pure closure without re-drawing (verified: the hoisted single- and multi-app forms
/// both fold, sharing the ONE draw). Sound: the init runs once, before the body, in the same order —
/// only the binder's visibility widens (identical in spirit to the do-item let-lift in `thread_bounded`).
/// Rewrites the FIRST matching binding it finds (structurally); the caller's `while` loop re-runs on the
/// result so a body with several such closures converges by fixpoint. Two faces are handled: FORM A — a
/// let-bound closure-value-let `(f (let ((a <perform>)…) LAMBDA))` (hoist the inner init out); FORM B — an
/// arg'd factory call `(f (mk <performing-arg>…))` (hoist each performing arg to a fresh `#cap`, then inline
/// the factory call to a pure closure over the drawn result). Returns the rewritten body, or `None` if no
/// matching binding is present. The caller `deep_fresh_copy`s the result so the rewritten tree has coherent
/// parent pointers (a reused subtree can share a load-time atom whose stale parent chain would otherwise
/// dead-end the scope walk → a false unbound); see the caller's hygiene comment.
fn hoist_performing_capture_closure(
    db: &mut Db,
    node: StructId,
    ctx: &HandlerCtx,
) -> Option<StructId> {
    // A by-name scan for whether the returned-lambda body references the closure-value-let's binder —
    // mirrors the detector's `refs_name` (resolution chains are unreliable pre-reparent, so a name scan
    // is the robust check), so the hoist targets exactly the binding the guard flagged.
    fn refs_name(db: &Db, node: StructId, name: &str) -> bool {
        if db.ast.as_name(node) == Some(name) {
            return true;
        }
        match db.ast.get(node) {
            Struct::List(children) => children.iter().any(|&c| refs_name(db, c, name)),
            Struct::Atom(_) => false,
        }
    }
    // Try to rewrite THIS node if it is a `(let (pairs) lbody)` with a matching binding.
    if let Some(form) = db.ast.as_form(node, "let").map(|t| t.to_vec())
        && form.len() == 2
        && let Struct::List(pairs) = db.ast.get(form[0]).clone()
    {
        for (i, &pair) in pairs.iter().enumerate() {
            let Struct::List(kv) = db.ast.get(pair).clone() else {
                continue;
            };
            if kv.len() != 2 {
                continue;
            }
            let (binder, value) = (kv[0], kv[1]);
            // FORM D (branch-selected capture, cpc1) — value is `(if C X Y)` where a branch is a
            // performing-capture creation-wrapper closure `(let ((a <perform>)…) LAMBDA)`. Hoisting the draw
            // unconditionally would be UNSOUND (the other branch does not draw), so instead DISTRIBUTE the
            // let-over-if into an if-over-lets: `(let (…(f (if C X Y))…) BODY)` → `(if C (let (…(f X)…) BODY)
            // (let (…(f Y)…) BODY))`. Each branch is then a plain `(let ((f <closure>)) BODY)` the next
            // fixpoint iteration's FORM A folds (a performing branch hoists its draw; a pure branch folds
            // directly), and the while-loop's `deep_fresh_copy` gives the duplicated BODY coherent parents.
            // Sound: exactly one branch runs, so BODY executes once and the draw fires only in the taken
            // branch. Gated on a branch actually being a performing creation-wrapper (else no distribution).
            if let Some(iff) = db.ast.as_form(value, "if").map(|t| t.to_vec())
                && iff.len() == 3
                && {
                    let branch_is_perf_wrapper = |db: &mut Db, b: StructId| {
                        db.ast.as_form(b, "let").map(|t| t.to_vec()).is_some_and(|inner| {
                            inner.len() == 2
                                && body_returns_lambda(db, inner[1])
                                && matches!(db.ast.get(inner[0]).clone(), Struct::List(ps) if ps.iter().any(|&ip| match db.ast.get(ip).clone() {
                                    Struct::List(kv2) if kv2.len() == 2 =>
                                        subtree_reaches_discharged_op(db, kv2[1], ctx) || body_reaches_foreign_perform(db, kv2[1], ctx),
                                    _ => false,
                                }))
                        })
                    };
                    branch_is_perf_wrapper(db, iff[1]) || branch_is_perf_wrapper(db, iff[2])
                }
            {
                let (cond, then_v, else_v) = (iff[0], iff[1], iff[2]);
                let mk_branch = |db: &mut Db, bval: StructId| {
                    let bpair = db.push_list(vec![binder, bval]);
                    let mut bp = pairs.clone();
                    bp[i] = bpair;
                    let bp_list = db.push_list(bp);
                    let lh = db.push_name("let");
                    db.push_list(vec![lh, bp_list, form[1]])
                };
                let then_let = mk_branch(db, then_v);
                let else_let = mk_branch(db, else_v);
                let if_head = db.push_name("if");
                return Some(db.push_list(vec![if_head, cond, then_let, else_let]));
            }
            // FORM B (arg'd FACTORY, cc3) — value is `(mk perf-arg…)`: a non-recursive factory whose body
            // returns a lambda, with ≥1 arg reaching a discharged/foreign perform. The performing arg's draw
            // re-runs per application. HOIST each performing arg to a fresh `#cap` wrapping the binding, then
            // INLINE the factory call `apply_lambda(mk, #cap…)` to a PURE closure over the drawn RESULT `#cap`
            // (the arg is now a VALUE, so copy-propagating it into the body is safe — no perform duplicated):
            // `(let (…(f (mk P))…) BODY)` → `(let ((#cap P)…) (let (…(f (mk-body[m:=#cap]))…) BODY))`. The
            // while-loop's `deep_fresh_copy` then gives the result fresh parents so the `#cap` refs resolve.
            if let Resolved::Apply { head, args } = resolved_of(db, value)
                && !args.is_empty()
                && is_perform(db, head, ctx).is_none()
                && let Some(callee) = crate::eval::lambda_body(db, head)
                    .or_else(|| crate::eval::lambda_body_of_nullary(db, head))
                && !crate::eval::is_recursive(db, callee)
                && body_returns_lambda(db, callee)
                && args.iter().any(|&a| {
                    subtree_reaches_discharged_op(db, a, ctx)
                        || body_reaches_foreign_perform(db, a, ctx)
                })
            {
                let mut cap_pairs: Vec<StructId> = Vec::new();
                let mut new_args: Vec<StructId> = Vec::new();
                for &arg in args.iter() {
                    if subtree_reaches_discharged_op(db, arg, ctx)
                        || body_reaches_foreign_perform(db, arg, ctx)
                    {
                        let cap = format!("#cap{}", arg.0);
                        let cap_binder = db.push_name(&cap);
                        let cap_ref = db.push_name(&cap);
                        cap_pairs.push(db.push_list(vec![cap_binder, arg]));
                        new_args.push(cap_ref);
                    } else {
                        new_args.push(arg);
                    }
                }
                if !cap_pairs.is_empty()
                    && let Ok(Some(closure)) = crate::eval::apply_lambda(db, head, &new_args)
                {
                    let new_pair = db.push_list(vec![binder, closure]);
                    let mut new_pairs = pairs.clone();
                    new_pairs[i] = new_pair;
                    let new_pairs_list = db.push_list(new_pairs);
                    let let_head_inner = db.push_name("let");
                    let inner_let = db.push_list(vec![let_head_inner, new_pairs_list, form[1]]);
                    let cap_pairs_list = db.push_list(cap_pairs);
                    let let_head_outer = db.push_name("let");
                    return Some(db.push_list(vec![let_head_outer, cap_pairs_list, inner_let]));
                }
            }
            // FORM A — the value is `(let (inner_pairs) LAMBDA)`: a closure-value-let returning a lambda, at
            // least one of whose inits reaches a discharged/foreign perform AND whose returned lambda
            // references that performing binder (the exact shape the detector fires on).
            let Some(inner) = db.ast.as_form(value, "let").map(|t| t.to_vec()) else {
                continue;
            };
            if inner.len() != 2 || !body_returns_lambda(db, inner[1]) {
                continue;
            }
            let Struct::List(inner_pairs) = db.ast.get(inner[0]).clone() else {
                continue;
            };
            let lambda = inner[1];
            let matches = inner_pairs.iter().any(|&ip| match db.ast.get(ip).clone() {
                Struct::List(ipkv) if ipkv.len() == 2 => {
                    let performs = subtree_reaches_discharged_op(db, ipkv[1], ctx)
                        || body_reaches_foreign_perform(db, ipkv[1], ctx);
                    let refs = db
                        .ast
                        .as_name(ipkv[0])
                        .map(str::to_string)
                        .is_some_and(|nm| refs_name(db, lambda, &nm));
                    performs && refs
                }
                _ => false,
            });
            if !matches {
                continue;
            }
            // Rebuild the current let with binding `i`'s value replaced by the returned LAMBDA, then wrap
            // it in the hoisted inner-pairs let: `(let (inner_pairs) (let (pairs[i→(binder LAMBDA)]) lbody))`.
            let new_pair = db.push_list(vec![binder, lambda]);
            let mut new_pairs = pairs.clone();
            new_pairs[i] = new_pair;
            let new_pairs_list = db.push_list(new_pairs);
            let let_head_inner = db.push_name("let");
            let inner_let = db.push_list(vec![let_head_inner, new_pairs_list, form[1]]);
            let let_head_outer = db.push_name("let");
            let hoisted = db.push_list(vec![let_head_outer, inner[0], inner_let]);
            return Some(hoisted);
        }
    }
    // Recurse structurally so a nested occurrence (inside a branch, a deeper let body) is rewritten too.
    if let Struct::List(children) = db.ast.get(node).clone() {
        for (j, &c) in children.iter().enumerate() {
            if let Some(rc) = hoist_performing_capture_closure(db, c, ctx) {
                let mut new_children = children.clone();
                new_children[j] = rc;
                return Some(db.push_list(new_children));
            }
        }
    }
    None
}

/// Rewrite a handle body `(do (def v e) rest…)` whose FIRST item is a do-local VALUE def into `(let ((v e))
/// (do rest…))` — recursively, so a chain of leading value defs all become nested `let`s scoping the rest.
/// This runs ONCE at the top of `reduce_handle` so every downstream fold (abortive one-hole, pure one-hole,
/// tail-resume thread) sees a properly-scoped body: those folds drop non-final `do` items and re-splice only
/// a surviving expression, which would orphan a `(def v e)` a later item references (e.g. a perform's arg) →
/// spurious CDZ0101 unbound. A `let` binding, by contrast, is rebuilt with its scope intact, so lifting to
/// `let` fixes the leak uniformly. Only a leading VALUE def (`(def NAME expr)`, sig a bare name) lifts — a
/// FUNCTION def (`(def (f p…) body)`, sig a list) resolves to a lambda and is left in place; a non-`(do …)`
/// body or a `do` whose first item is not a value def returns unchanged (byte-identical to before).
fn lift_do_local_value_defs(db: &mut Db, body: StructId) -> StructId {
    let Some(items) = db.ast.as_form(body, "do").map(<[_]>::to_vec) else {
        return body;
    };
    if items.len() < 2 {
        return body;
    }
    let first = items[0];
    let Some(tail) = db.ast.as_form(first, "def") else {
        return body;
    };
    // A VALUE def is exactly `(def NAME expr)`: two tail elements, the sig a bare name (a function def's sig
    // is a `(f p…)` list, which `as_name` rejects — leave it in place).
    if tail.len() != 2 || db.ast.as_name(tail[0]).is_none() {
        return body;
    }
    let name = tail[0];
    let value = tail[1];
    // `(do rest…)` — the remaining items; recurse so a further leading def lifts too.
    let do_head = db.push_name("do");
    let mut cont = vec![do_head];
    cont.extend_from_slice(&items[1..]);
    let cont_raw = db.push_list(cont);
    let cont_do = lift_do_local_value_defs(db, cont_raw);
    let pair = db.push_list(vec![name, value]);
    let binds = db.push_list(vec![pair]);
    let let_head = db.push_name("let");
    db.push_list(vec![let_head, binds, cont_do])
}

/// Capture-avoiding HYGIENE for the handler fold. Alpha-rename every LOCAL VALUE binder in `root` — a
/// `let`-binding pair name, and a `do`-local `(def NAME value)` name — to a FRESH `#`-prefixed name unique to
/// its binder node, rewriting the in-scope references that resolve to it. Run on the handle body AND each arm
/// body BEFORE the fold composes them (`splice_context`/`beta_reduce` splice the arm body, the continuation
/// `C = handle_body[perform := □]`, and resume VALUES across scope boundaries by STRUCTURALLY COPYING name
/// atoms that then RE-RESOLVE against the destination scope). Without freshening, a free name in the spliced
/// material is CAPTURED by a same-named local binder in the destination. F1 — the arm body `(do (def x 5)
/// (resume (+ x s) s))`: `C = (+ x [])` (its `x` the OUTER/global x=100) is spliced for the resume, landing
/// inside the arm's `(do (def x 5) …)` so `C`'s `x` binds to 5 (=10, not 105). F2 — the performer `(do (def
/// x 7) (+ x (E.get)))` and arm `(resume x s)`: the resume VALUE `x` (the global) is spliced at the perform
/// hole INSIDE `(do (def x 7) …)` so it binds to 7 (=14, not 107).
/// Freshening the local binders in both bodies makes such a collision impossible (a `#x…` binder shares its
/// name with nothing), so a spliced free name keeps its intended (outer) resolution. Silent-miscompile fix
/// (breaker, routed corpus-bugfix 2026-07-28; the effects twin of the eval-splice capture family — same
/// FRESH-NAMES-per-splice template as the metaprogramming quote/splice hygiene). A FUNCTION def `(def (f p…)
/// body)` (sig a list) is left untouched — it resolves to a lambda, not a value binder this fold splices
/// across. LAMBDA params / match-arm binders are NOT renamed here (the fold does not splice foreign material
/// under them in a capturing way — the arg substitution is by binder-node identity, immune to name); only the
/// `let`/`do`-def value binders that a `do`/`let` scope exposes to a spliced continuation need it.
fn freshen_local_binders(db: &mut Db, root: StructId) -> StructId {
    freshen_walk(db, root, &mut HashMap::default()).unwrap_or(root)
}

/// Recursive worker for [`freshen_local_binders`]. `renames` maps a binder-NODE occurrence (the one a
/// reference's `resolved_of` reaches) to its fresh name string, threaded down so in-scope references rewrite.
/// Returns `Some(new)` ONLY when a rename actually applied within this subtree, else `None` — the caller then
/// SHARES the original node untouched. Sharing is load-bearing: a free-name reference (an enclosing param /
/// global the handle body legitimately reads) keeps its RESOLVE-PINNED occurrence; rebuilding it with
/// `push_list`/`copy_pure` would re-resolve it against the not-yet-reparented tree → spurious CDZ0101 unbound.
/// So this pass touches ONLY the paths that carry a local binder or a reference to one, leaving every free
/// name shared and correctly resolved.
fn freshen_walk(
    db: &mut Db,
    node: StructId,
    renames: &mut HashMap<StructId, String>,
) -> Option<StructId> {
    // A NESTED `handle` / `handle-internal` is OPAQUE to this pass — SHARE it whole (return None). Its arm
    // bodies + body are the INNER handle's own concern, freshened when IT reduces (`reduce_handle` recurses).
    // Descending would rename the inner arm's `(def x …)` and force a REBUILD of this outer subtree, which
    // orphans a FN-LOCAL reference the outer body legitimately reads (the fn-local `(def x 1000)` sits OUTSIDE
    // this handle body; its `x` ref is a shared pinned atom whose resolution breaks once its parent is rebuilt
    // → spurious CDZ0101 unbound — the nested×arm-local×fn-local-body-x regression, corpus-bugfix 2026-07-28).
    // Sharing the whole nested handle keeps the outer body's fn-local refs intact; the inner handle freshens
    // its OWN binders in its own fold. (A rename registered by an ENCLOSING `let`/`do` scope does not reach
    // inside a shared nested handle — but the inner handle's body references to THIS scope's binders are rare
    // and, when present, are the inner fold's to resolve against the reparented outer result.)
    if matches!(resolved_of(db, node), Resolved::Handle { .. })
        || db.ast.head_name(node) == Some(HANDLE_INTERNAL)
    {
        // The arms + body ARE the inner handle's concern (freshened when IT reduces) — leave them opaque.
        // BUT the nested handle's SEED (its init) is evaluated in THIS (enclosing) scope, so a reference in
        // it to an enclosing binder an outer `let` just renamed MUST be rewritten here — else the inner fold
        // reduces with a stale seed reference (`init=cfg`) that no longer resolves after the outer let-binder
        // was freshened (`cfg`→`#cfg{n}`), baking a dangling name into the inner fold's `#seed` init →
        // spurious CDZ0101 unbound (the let-bound-seed × nested-handle orphan, sh2d family). Rewrite ONLY the
        // seed under the current renames; if it changed, rebuild the handle node with the fresh seed and the
        // ORIGINAL arms/body (still the inner fold's to freshen). The desugared `(handle-internal SEED arms
        // body)` carries the seed at CHILD INDEX 1 (index 0 = the `handle-internal` head). Only the internal
        // form reaches this fold stage; a raw `(handle E seed …)` (Resolved::Handle) is left fully opaque
        // (its seed index differs; not seen post-desugar). Nothing changed → share whole (return None).
        if db.ast.head_name(node) == Some(HANDLE_INTERNAL)
            && let Struct::List(children) = db.ast.get(node).clone()
            && children.len() >= 4
            && let Some(new_seed) = freshen_walk(db, children[1], renames)
        {
            let mut new_children = children.clone();
            new_children[1] = new_seed;
            return Some(db.push_list(new_children));
        }
        return None;
    }
    // A `let` — rename each binding-pair's binder to a fresh name, scoping the rename over the inits (later
    // bindings + the body see earlier binders) and the body. Rebuild only if anything changed.
    if let Some(tail) = db.ast.as_form(node, "let").map(<[_]>::to_vec)
        && tail.len() == 2
        && let Struct::List(pairs) = db.ast.get(tail[0]).clone()
    {
        let mut changed = false;
        let mut new_pairs = Vec::with_capacity(pairs.len());
        for pair in pairs {
            if let Struct::List(kv) = db.ast.get(pair).clone()
                && kv.len() == 2
                && let Some(name) = db.ast.as_name(kv[0]).map(str::to_string)
            {
                // Init is in the scope BEFORE this binder (freshen it under the current renames), then
                // register the fresh name so the body + later inits resolving to this binder rewrite. A
                // `let` reference resolves (`resolve_name`) to `Ref { value: <init occ> }` — the INIT
                // occurrence, NOT the binder-name occurrence — so key `renames` on the ORIGINAL init `kv[1]`.
                let new_init = freshen_walk(db, kv[1], renames).unwrap_or(kv[1]);
                let fresh = format!("#{name}{}", kv[0].0);
                renames.insert(kv[1], fresh.clone());
                let fresh_binder = db.push_name(&fresh);
                new_pairs.push(db.push_list(vec![fresh_binder, new_init]));
                changed = true; // a binder was always renamed
            } else if let Some(np) = freshen_walk(db, pair, renames) {
                new_pairs.push(np);
                changed = true;
            } else {
                new_pairs.push(pair);
            }
        }
        let new_body = freshen_walk(db, tail[1], renames);
        if !changed && new_body.is_none() {
            return None;
        }
        let let_head = db.push_name("let");
        let binds = db.push_list(new_pairs);
        return Some(db.push_list(vec![let_head, binds, new_body.unwrap_or(tail[1])]));
    }
    // A `do` — a `(def NAME value)` item binds NAME LOCAL to the rest of the `do`. Freshen each such def's
    // name, scoping over the following items; a non-def item is walked under the running renames.
    if let Some(items) = db.ast.as_form(node, "do").map(<[_]>::to_vec) {
        let mut changed = false;
        let mut new_items = vec![db.push_name("do")];
        for item in items {
            if let Some(dtail) = db.ast.as_form(item, "def").map(<[_]>::to_vec)
                && dtail.len() == 2
                && let Some(name) = db.ast.as_name(dtail[0]).map(str::to_string)
            {
                // A do-local `(def NAME V)` reference resolves (`do_def_binds`) to `Ref { value: V }` — the
                // VALUE occurrence — so key `renames` on the ORIGINAL value `dtail[1]`, not the name.
                let new_val = freshen_walk(db, dtail[1], renames).unwrap_or(dtail[1]);
                let fresh = format!("#{name}{}", dtail[0].0);
                renames.insert(dtail[1], fresh.clone());
                let fresh_binder = db.push_name(&fresh);
                let def_head = db.push_name("def");
                new_items.push(db.push_list(vec![def_head, fresh_binder, new_val]));
                changed = true;
            } else if let Some(ni) = freshen_walk(db, item, renames) {
                new_items.push(ni);
                changed = true;
            } else {
                new_items.push(item);
            }
        }
        return changed.then(|| db.push_list(new_items));
    }
    // A NAME reference that resolves to a renamed binder → its fresh name. `resolved_of` follows the scope
    // walk to the binder occurrence; if we renamed that occurrence, rewrite this reference.
    if db.ast.as_name(node).is_some()
        && !is_binder_occ_local(db, node)
        && let Resolved::Ref { value } = resolved_of(db, node)
        && let Some(fresh) = renames.get(&value).cloned()
    {
        return Some(db.push_name(&fresh));
    }
    // Otherwise: recurse into children, rebuilding ONLY if some child changed (else SHARE `node` so a free
    // name keeps its pinned resolution).
    match db.ast.get(node).clone() {
        Struct::List(children) => {
            let mut changed = false;
            let mut rebuilt = Vec::with_capacity(children.len());
            for c in children {
                match freshen_walk(db, c, renames) {
                    Some(nc) => {
                        rebuilt.push(nc);
                        changed = true;
                    }
                    None => rebuilt.push(c),
                }
            }
            changed.then(|| db.push_list(rebuilt))
        }
        Struct::Atom(_) => None,
    }
}

/// Whether `node` is itself a binder-position name in a `let`/`do`-def (handled by the freshen pass's binder
/// rewrite, so its reference branch must NOT also fire). A thin wrapper over the arena shape check.
fn is_binder_occ_local(db: &Db, node: StructId) -> bool {
    let Some(parent) = db.parent_of(node) else {
        return false;
    };
    // `(def NAME value)` — NAME is the binder (second child, first tail element).
    if let Some(dtail) = db.ast.as_form(parent, "def")
        && dtail.first() == Some(&node)
        && dtail.len() == 2
    {
        return true;
    }
    // A `let` binding pair `(NAME init)` whose parent pair's grandparent is a `let` bindings list.
    if let Struct::List(kv) = db.ast.get(parent)
        && kv.first() == Some(&node)
        && kv.len() == 2
        && let Some(grand) = db.parent_of(parent)
        && let Some(great) = db.parent_of(grand)
        && db
            .ast
            .as_form(great, "let")
            .is_some_and(|lt| lt.first() == Some(&grand))
    {
        return true;
    }
    false
}

/// A do-item's `(let (binds) lbody)` shape — either the item IS a `let`, or it is a cross-fn effectful
/// helper CALL that INLINES to a `let`-headed body. Returns `(binds-list-occ, lbody-occ)` (both from a
/// FRESH deep copy of the reduced body, so the lift re-parents them cleanly). `None` if the item neither is
/// nor inlines to a `let`. Used by the `do` thread arm's LET-LIFT (a non-final item's local `let` binding
/// escapes when the perform's out-state threads forward — see the lift's comment). Bounded: the inline
/// preview only β-reduces a single non-recursive discharged-effect callee (the same shape the inline arm
/// serves), never recurses.
fn inlined_let_of_do_item(
    db: &mut Db,
    item: StructId,
    ctx: &HandlerCtx,
) -> Option<(StructId, StructId)> {
    // The reduced body: the item itself if it is a bare `(let …)`, else the β-reduction of a cross-fn
    // effectful-helper call (a `let`-bearing helper like the memoize combinator). Only a helper the inline
    // arm would serve (`call_reaches_discharged_effect`) is previewed — a recursive callee (specialized, not
    // inlined) is excluded, matching the arm.
    let reduced = if db.ast.as_form(item, "let").is_some() {
        item
    } else if let Resolved::Apply { head, args } = resolved_of(db, item) {
        if !call_reaches_discharged_effect(db, head, ctx) {
            return None;
        }
        let r = match crate::eval::apply_lambda(db, head, &args).ok().flatten() {
            Some(r) => r,
            None => crate::eval::lambda_body_of_nullary(db, head)?,
        };
        deep_fresh_copy(db, r)
    } else {
        return None;
    };
    // The reduced body must be a two-child `(let bindings body)`. (A `let*` with multiple bindings is one
    // bindings-list child either way, so the shape check is uniform.)
    let tail = db.ast.as_form(reduced, "let").map(<[_]>::to_vec)?;
    if tail.len() != 2 {
        return None;
    }
    Some((tail[0], tail[1]))
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
    if rec && reaches {
        return true;
    }
    // ACCUM SUCCESSOR (part 1, recursive-nested-arm-resume fix). A linear non-tail recursion `f` that
    // `accum::introduce` rewrote into a seed wrapper (this `body`, now non-recursive — it calls `f$acc n 0`)
    // + a tail-recursive copy `f$acc`: `is_recursive(f.body)` reads false. Follow `transformed` (source→copy)
    // and test the COPY so the merge decision sees the accum-transformed recursive performer.
    if let Some(&acc) = db.transformed.get(&callee_def)
        && let Some(acc_body) = db.defs[acc].body
        && crate::eval::is_recursive(db, acc_body)
        && body_reaches_discharged(db, acc_body, ctx, 0)
    {
        return true;
    }
    false
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
        // A MODULE-member call `(. m walk)` resolves to a `Member`; follow it to the field lambda via
        // `member_value` (exactly as `lower::callee_def_index` does), so a module-EXPORTED recursive
        // performer is found by the handler-context reduction the same way a bare recursive def is. Without
        // this arm the merge/base-arm reduction returns `None` for a module callee, so its per-step perform
        // is never re-homed under the importer's handler → the recursive body lowers standalone → the
        // "no enclosing handler here" decline (the module × recursion × effect-context-mono composition gap).
        Resolved::Member { operand, key } => match crate::eval::member_value(db, operand, &key) {
            crate::eval::Member::Field(v) => callee_def_index_of(db, v),
            _ => None,
        },
        _ => None,
    }
}

/// Fold a case-of-known-ctor match that may sit under one or more leading `let` wrappers, KEEPING the
/// `let`s around the folded arm. `apply_lambda`'s eval-once path let-binds a resume-closure argument
/// (`(let ((kb …)) (match q …))`), so the unfolded recursive-callee body is a `let`-wrapped match rather
/// than a bare match — `eval::fold_ctor_match` only folds a bare match. Peel the leading `let`s, fold the
/// inner match, then re-wrap the same `let`s around the folded result. Returns `None` if there is no
/// case-of-known-ctor match under the lets (i.e. `fold_ctor_match` on the innermost body declines).
fn fold_ctor_match_through_lets(db: &mut Db, node: StructId) -> Option<StructId> {
    // A leading `(let (bindings…) body)` — recurse into `body`, then re-wrap. The `let` is the eval-once
    // binding `apply_lambda` synthesized for a resume-closure argument used across MULTIPLE callee arms; but
    // only the folded BASE arm survives, where the bound name is now used AT MOST ONCE. In that case INLINE
    // the binding (β-substitute it into the folded body) so the result is the bare directly-constructed form
    // the surrounding reduction expects (`(sched-step (PQCons …))`, not `(sched-step (let … (PQCons …)))`) —
    // and so a single-use resume-closure is not left behind a `let` the downstream pop-fold cannot see
    // through. A binding still used ≥2× in the folded arm is KEPT (re-wrapped) — inlining would duplicate it.
    if let Some(parts) = db.ast.as_form(node, "let").map(<[_]>::to_vec)
        && parts.len() == 2
    {
        let bindings = parts[0];
        let body = parts[1];
        let folded_body = fold_ctor_match_through_lets(db, body)?;
        // Try to inline each single-use binding into the folded body.
        if let Struct::List(pairs) = db.ast.get(bindings).clone() {
            let mut subst: HashMap<StructId, StructId> = HashMap::default();
            let mut kept: Vec<StructId> = Vec::new();
            for &pair in &pairs {
                if let Struct::List(kv) = db.ast.get(pair).clone()
                    && kv.len() == 2
                    && count_param_refs(db, folded_body, kv[0]) <= 1
                {
                    subst.insert(kv[0], kv[1]);
                } else {
                    kept.push(pair);
                }
            }
            if !subst.is_empty() {
                let inlined = crate::eval::beta_reduce(db, folded_body, &subst);
                if kept.is_empty() {
                    return Some(inlined);
                }
                let let_head = db.push_name("let");
                let kept_list = db.push_list(kept);
                return Some(db.push_list(vec![let_head, kept_list, inlined]));
            }
        }
        let let_head = db.push_name("let");
        return Some(db.push_list(vec![let_head, bindings, folded_body]));
    }
    // Otherwise fold the (bare) match directly.
    crate::eval::fold_ctor_match(db, node)
}

/// Whether `node` contains an APPLICATION whose head resolves to `def` (a call to that def) — a residual
/// self-call check for the deferred-resume fold's one-level recursion-unfold: after unfolding a recursive
/// callee once and folding its internal ctor-match, the base arm has no self-call (accept) while a
/// recursive arm still calls back into the callee (discard). Walks the arena structurally; an application's
/// HEAD is classified via `callee_def_index_of` (following Ref chains to the named def).
fn body_calls_def(db: &mut Db, node: StructId, def: usize) -> bool {
    if let Resolved::Apply { head, .. } = resolved_of(db, node)
        && callee_def_index_of(db, head) == Some(def)
    {
        return true;
    }
    match db.ast.get(node).clone() {
        Struct::List(children) => children.iter().any(|&c| body_calls_def(db, c, def)),
        Struct::Atom(_) => false,
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

/// Like [`callee_calls_other_recursive_def`] but TRANSITIVE through NON-RECURSIVE helper defs (finding #19
/// indirection face). `callee_calls_other_recursive_def` only sees a DIRECT call to another recursive def; the
/// indirection variant `outer → via → inner` (via a non-recursive pass-through `via`) hides the recursive
/// performer `inner` behind `via`, so the direct check misses it and the recursion-boundary decline guard
/// doesn't fire → single-return DROPS `inner`'s advance = a silent miscompile. Follow a call to a NON-recursive
/// def into its body (cycle-guarded via `visiting`) so the reachable recursive performer is found; a recursive
/// callee OTHER than `callee_def` is the hit (as the direct fn), and a recursive callee is NOT descended (it is
/// the performer). Used ONLY at the recursion-boundary decline guard, so the indirection reaches the SAME sound
/// DECLINE floor as the direct case (rather than a leaky multi-value attempt). The direct
/// `callee_calls_other_recursive_def` stays in use at the abortive guard (unchanged behavior there).
fn callee_transitively_calls_other_recursive_def(
    db: &mut Db,
    body: StructId,
    callee_def: usize,
    visiting: &mut Vec<usize>,
) -> bool {
    if let Resolved::Apply { head, .. } = resolved_of(db, body)
        && let Some(other) = callee_def_index_of(db, head)
        && other != callee_def
        && let Some(other_body) = db.defs[other].body
    {
        if crate::eval::is_recursive(db, other_body) {
            return true; // a directly-reachable OTHER recursive def — the hit
        }
        // A NON-recursive helper: follow it (cycle-guarded) to find a recursive performer it reaches.
        if !visiting.contains(&other) {
            visiting.push(other);
            let hit =
                callee_transitively_calls_other_recursive_def(db, other_body, callee_def, visiting);
            visiting.pop();
            if hit {
                return true;
            }
        }
    }
    match db.ast.get(body).clone() {
        Struct::List(children) => children
            .iter()
            .any(|&c| callee_transitively_calls_other_recursive_def(db, c, callee_def, visiting)),
        Struct::Atom(_) => false,
    }
}

/// Collect the def indices of every DIRECT callee in `body` that resolves to a known def — one edge per
/// application head. Used by `mutual_scc_of` to walk the recursive group. Deduped, order-insensitive.
fn direct_callee_defs(db: &mut Db, body: StructId, out: &mut Vec<usize>) {
    if let Resolved::Apply { head, .. } = resolved_of(db, body)
        && let Some(d) = callee_def_index_of(db, head)
        && !out.contains(&d)
    {
        out.push(d);
    }
    if let Struct::List(children) = db.ast.get(body).clone() {
        for c in children {
            direct_callee_defs(db, c, out);
        }
    }
}

/// The mutually-recursive SCC containing `callee_def`: the set of def indices reachable from `callee_def`
/// that ALSO reach back to it (a two-way path = same cycle), restricted to defs whose body reaches a
/// discharged op under `ctx`. This is the group the multi-value fold must reserve + thread together, so
/// each member's out-state threads across the cross-def calls. `callee_def` is always included. A pure
/// self-recursive def (no mutual partner) returns just `[callee_def]`. Bounded by the finite def table.
///
/// Membership test: def `d` is in the SCC iff `callee_def` reaches `d` (forward, via the call graph) AND
/// `d` reaches `callee_def` (so it cycles back — a genuine mutual partner, not a one-way callee). Both
/// directions use a bounded reachability walk over `direct_callee_defs`. Only defs that reach a discharged
/// op are kept (a pure helper in the cycle needs no state threading — it is inlined, not specialized).
fn mutual_scc_of(db: &mut Db, callee_def: usize, ctx: &HandlerCtx) -> Vec<usize> {
    // MEMO (v-compiler-perf advisory on #2877): SCC membership is program-STATIC + the discharged-op filter
    // is fixed by `ctx.key`, so the result is stable per `(callee_def, ctx.key)`. The group-entry path calls
    // this TWICE (the `group_entry` predicate + the member-registration loop), and sibling SCC members
    // re-derive the same set — the memo collapses all of that to ONE forward-BFS + per-def reaches-BFS.
    let memo_key = (callee_def, ctx.key.clone());
    if let Some(cached) = db.mutual_scc.get(&memo_key) {
        return cached.clone();
    }
    // Forward reachability from `start` over direct-callee edges (def-index graph), bounded by the def table.
    // `seen` is a HashSet — membership is O(1), so the walk is O(V+E) not O(V*E) (v-compiler-perf: was
    // `Vec::contains`). Small SCCs are unaffected either way; this keeps a wide call graph off the hot path.
    fn reaches(db: &mut Db, start: usize, target: usize) -> bool {
        let mut seen: std::collections::HashSet<usize> = std::collections::HashSet::new();
        let mut work: Vec<usize> = vec![start];
        while let Some(d) = work.pop() {
            if d == target {
                return true;
            }
            if !seen.insert(d) {
                continue;
            }
            if let Some(body) = db.defs[d].body {
                let mut callees = Vec::new();
                direct_callee_defs(db, body, &mut callees);
                for c in callees {
                    if c == target {
                        return true;
                    }
                    if !seen.contains(&c) {
                        work.push(c);
                    }
                }
            }
        }
        false
    }
    // The forward-reachable set from `callee_def` (candidate SCC members before the back-edge filter). The
    // ORDER is preserved (a `Vec` for a deterministic member sequence) + a `HashSet` mirrors it for O(1)
    // membership — reproducible output, no O(n) `contains`.
    let mut forward: Vec<usize> = Vec::new();
    let mut forward_set: std::collections::HashSet<usize> = std::collections::HashSet::new();
    let mut work: Vec<usize> = vec![callee_def];
    while let Some(d) = work.pop() {
        if !forward_set.insert(d) {
            continue;
        }
        forward.push(d);
        if let Some(body) = db.defs[d].body {
            let mut callees = Vec::new();
            direct_callee_defs(db, body, &mut callees);
            for c in callees {
                if !forward_set.contains(&c) {
                    work.push(c);
                }
            }
        }
    }
    // Keep the members that reach BACK to `callee_def` (same cycle) AND reach a discharged op under `ctx`.
    let mut scc: Vec<usize> = Vec::new();
    for &d in &forward {
        let in_cycle = d == callee_def || reaches(db, d, callee_def);
        if !in_cycle {
            continue;
        }
        let Some(body) = db.defs[d].body else {
            continue;
        };
        if body_reaches_discharged(db, body, ctx, 0) {
            scc.push(d);
        }
    }
    if !scc.contains(&callee_def) {
        scc.push(callee_def);
    }
    db.mutual_scc.insert(memo_key, scc.clone());
    scc
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
/// Whether a MUTUAL-PARTNER call (a re-entrant call to a DIFFERENT recursive def, not `callee_def` itself)
/// PRECEDES an out-state observation (a perform or another re-entrant call) on the strict spine — the
/// mutual analogue of `selfcall_precedes_perform_in_operands`. Single-return specialization threads the
/// mutual partner's call with the INCOMING state as its trailing arg and returns that incoming state
/// UNCHANGED as the out-state (the recursive-call thread arm's single-return path), so the partner's own
/// state ADVANCE is dropped: a later perform / call reading the out-state sees the pre-recursion state — a
/// SILENT wrong value (`(let ((child (typeof (- n 1)))) (+ child (St.get)))` where `typeof` is a mutual
/// partner that performs `put`s: the `(St.get)` reads the un-advanced state). The multi-value machinery
/// (`thread_returning_tuple` + out-state projection) solves this for a SELF-call but does NOT extend across
/// a mutual SCC — a partner's out-state is not projected. So this shape must DECLINE cleanly until the
/// group-aware multi-value fold lands, NOT specialize to a dropped-advance miscompile. Mirrors
/// `selfcall_precedes_perform_in_operands`'s spine positions (operands, `let` inits then body, `do` items,
/// `match` scrutinee then arms, `if` cond then branches) but keys the "seen re-entry" trigger on a MUTUAL
/// partner (`callee_calls_other_recursive_def`-style: a recursive def OTHER than `callee_def`), so a pure
/// SELF-recursive body — already handled by `selfcall_precedes_perform_in_operands` + the multi-value path —
/// is NOT re-declined here. A mutual partner that is the TAIL (nothing observes its out-state — the scalar
/// ping/pong `(match (St.put n) (_ (pong …)))`) does NOT fire: the partner call is last on its spine.
fn mutual_partner_precedes_observation(
    db: &mut Db,
    node: StructId,
    callee_def: usize,
    ctx: &HandlerCtx,
) -> bool {
    // A re-entrant position that OBSERVES the recursion's out-state: a perform (reads the current state) OR
    // another re-entrant call (threads against it). Mirrors the two disjuncts the self-call `let`-arm uses.
    fn observes_outstate(db: &mut Db, node: StructId, callee_def: usize, ctx: &HandlerCtx) -> bool {
        contains_any_perform(db, node, ctx) || contains_recursive_call(db, node, callee_def)
    }
    // A mutual-partner call: a call to a DIFFERENT def whose own body is recursive (so it cycles back). NOT
    // a self-call to `callee_def` (that path is `selfcall_precedes_perform_in_operands`).
    fn contains_mutual_partner_call(db: &mut Db, node: StructId, callee_def: usize) -> bool {
        if let Resolved::Apply { head, .. } = resolved_of(db, node)
            && let Some(other) = callee_def_index_of(db, head)
            && other != callee_def
            && let Some(other_body) = db.defs[other].body
            && crate::eval::is_recursive(db, other_body)
        {
            return true;
        }
        match db.ast.get(node).clone() {
            Struct::List(children) => children
                .iter()
                .any(|&c| contains_mutual_partner_call(db, c, callee_def)),
            Struct::Atom(_) => false,
        }
    }
    // Strict application operands, left-to-right.
    if let Resolved::Apply { args, .. } = resolved_of(db, node) {
        let mut seen_partner = false;
        for &a in args.iter() {
            if seen_partner && observes_outstate(db, a, callee_def, ctx) {
                return true;
            }
            if contains_mutual_partner_call(db, a, callee_def) {
                seen_partner = true;
            }
        }
    }
    // `let` inits then body.
    if let Some(form) = db.ast.as_form(node, "let").map(|t| t.to_vec())
        && form.len() == 2
        && let Struct::List(pairs) = db.ast.get(form[0]).clone()
    {
        let mut seen_partner = false;
        for pair in pairs {
            if let Struct::List(kv) = db.ast.get(pair).clone()
                && kv.len() == 2
            {
                if seen_partner && observes_outstate(db, kv[1], callee_def, ctx) {
                    return true;
                }
                if contains_mutual_partner_call(db, kv[1], callee_def) {
                    seen_partner = true;
                }
            }
        }
        if seen_partner && observes_outstate(db, form[1], callee_def, ctx) {
            return true;
        }
    }
    // `do` items, left-to-right.
    if let Some(items) = db.ast.as_form(node, "do").map(|t| t.to_vec()) {
        let mut seen_partner = false;
        for &it in items.iter() {
            if seen_partner && observes_outstate(db, it, callee_def, ctx) {
                return true;
            }
            if contains_mutual_partner_call(db, it, callee_def) {
                seen_partner = true;
            }
        }
    }
    // `match` scrutinee then arm bodies.
    if let Resolved::Match { scrutinee, arms } = resolved_of(db, node)
        && contains_mutual_partner_call(db, scrutinee, callee_def)
        && arms
            .iter()
            .any(|&(_, body)| observes_outstate(db, body, callee_def, ctx))
    {
        return true;
    }
    // `if` cond then branches.
    if let Resolved::If { cond, then_, else_ } = resolved_of(db, node)
        && contains_mutual_partner_call(db, cond, callee_def)
        && (observes_outstate(db, then_, callee_def, ctx)
            || observes_outstate(db, else_, callee_def, ctx))
    {
        return true;
    }
    // Recurse structurally.
    match db.ast.get(node).clone() {
        Struct::List(children) => children
            .iter()
            .any(|&c| mutual_partner_precedes_observation(db, c, callee_def, ctx)),
        Struct::Atom(_) => false,
    }
}

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

/// Whether `node` is (or contains) a strict APPLICATION / `do` / spine context in which a CONDITIONAL whose
/// BRANCH performs a discharged op is a NON-TAIL operand ALONGSIDE a re-entrant call (a self-call to
/// `callee_def` OR a mutual-recursive partner). This is the RESUMPTIVE recursive-branch-perform gap (v-effects
/// self-probe 2026-08-04, breaker-confirmed rw1-rw5): unlike `selfcall_precedes_perform_in_operands` (which
/// catches a self-call THEN a later perform reading the OUT-state), here a perform inside a conditional BRANCH
/// PRECEDES/coexists with the self-call, and the single-return specialization threads the branch perform
/// against the INCOMING state — but the advance is BRANCH-LOCAL (only the taken path advances) and the
/// recursion carries the incoming state forward, so the advance is DROPPED across the recursion (`(+ (if true
/// (St.get) 0) (walk (- n 1)))` seeded 1 → 3 not 6). The non-recursive branch-perform hoist (Site 1/2/4/5)
/// does NOT run inside the specialized body. Covers: runtime conditions (keyed on the BRANCH position, not a
/// foldable cond — rw3), MUTUAL recursion (a partner call via `contains_recursive_call` — rw4), and heap state
/// (`contains_any_perform` is state-shape-agnostic — rw5). Does NOT fire on the FOLDING shapes: a BARE tail
/// perform `(+ (St.get) (walk …))` (the perform is not under a conditional branch here — it is a direct
/// operand), a let-init-bound perform then `if` on the binding (`sum-down`: the perform is a let-init, not
/// under a branch), or a perform as the WHOLE tail of one branch with the self-call in a SIBLING branch
/// (`ev`/`od`: no shared strict context — the branches are mutually exclusive). The discriminator: the
/// branch-perform's enclosing conditional and a re-entrant call are BOTH operands of the SAME strict node.
fn branch_perform_coexists_with_reentrant_call(
    db: &mut Db,
    node: StructId,
    callee_def: usize,
    ctx: &HandlerCtx,
) -> bool {
    // A strict application `(op a0 … ak)`: if one operand is a conditional whose branch performs a discharged
    // op AND another operand is (contains) a re-entrant call, the branch-perform's advance is dropped while
    // the recursion threads the incoming state. (The head is a0; operands a1..). We check each operand for the
    // branch-perform shape and, separately, whether ANY operand carries a re-entrant call.
    if let Resolved::Apply { head, args } = resolved_of(db, node) {
        // The head + every argument, as a fresh iterator each call (no intermediate `Vec` alloc per Apply on
        // the effects AST walk — liaison/Copilot #1957). Factored into one closure so both `.any()` checks
        // share the EXACT same operand stream — a future edit to the stream changes both (liaison/Copilot
        // maintainability nit on #1961).
        let operands = || std::iter::once(head).chain(args.iter().copied());
        let has_reentrant = operands().any(|a| contains_recursive_call(db, a, callee_def));
        if has_reentrant && operands().any(|a| operand_is_branch_performing_conditional(db, a, ctx))
        {
            return true;
        }
    }
    // Recurse structurally — the shape may be nested (inside a branch, let-init, do-item, etc.).
    match db.ast.get(node).clone() {
        Struct::List(children) => children
            .iter()
            .any(|&c| branch_perform_coexists_with_reentrant_call(db, c, callee_def, ctx)),
        Struct::Atom(_) => false,
    }
}

/// Whether `node` is a CONDITIONAL (`if`/`match`) whose taken BRANCH (then/else, or an arm body) reaches a
/// discharged perform — i.e. a branch-performing conditional whose advance is branch-local. The condition /
/// scrutinee is NOT a branch (a perform there is on the strict spine, threaded normally), so it is excluded.
/// Peels PURE `let`/`do` block wrappers so `(let ((v (let ((b true)) (if b (St.get) 0)))) …)` as an operand
/// is recognized (the block-wrapped variant of the same gap).
fn operand_is_branch_performing_conditional(db: &mut Db, node: StructId, ctx: &HandlerCtx) -> bool {
    match resolved_of(db, node) {
        Resolved::If { then_, else_, .. } => {
            contains_any_perform(db, then_, ctx) || contains_any_perform(db, else_, ctx)
        }
        Resolved::Match { arms, .. } => arms
            .iter()
            .any(|&(_, body)| contains_any_perform(db, body, ctx)),
        // Peel a PURE block wrapper to its tail value (the block-wrapped branch-perform operand).
        Resolved::Let { body, .. } => operand_is_branch_performing_conditional(db, body, ctx),
        Resolved::Ref { value } => operand_is_branch_performing_conditional(db, value, ctx),
        _ => false,
    }
}

/// Collect the `db.defs` indices of every RECURSIVE-EFFECTFUL call in `node` this handler discharges — a
/// call `(f args…)` where `f` is a recursive def whose body reaches an op in `ctx.arms`. Used by
/// [`mark_caller_observed_outstate`] to know which callee's out-state a later spine item observes.
fn collect_rec_eff_call_defs(db: &mut Db, node: StructId, ctx: &HandlerCtx, out: &mut Vec<usize>) {
    if let Resolved::Apply { head, .. } = resolved_of(db, node)
        && recursive_call_reaches_discharged(db, &head, ctx)
        && let Some(cd0) = callee_def_index_of(db, head)
    {
        // ACCUM-COPY REDIRECT (rn post-observer fix, increment 1): record the def `specialize_recursive`
        // will actually specialize. A merged-ctx call to a seed-wrapper redirects to its accum COPY
        // `f$acc`; `force_multivalue` is keyed by the SPECIALIZED def's body, and the mode decision looks
        // it up under `f$acc`'s body — so record `f$acc`'s index here, not the (non-recursive) wrapper's,
        // else the observation misses and the caller-observed out-state is dropped (q4a folds 20 not 21).
        let cd = accum_seed_redirect(db, cd0, ctx.slots.len()).map_or(cd0, |(acc, _)| acc);
        if !out.contains(&cd) {
            out.push(cd);
        }
    }
    match db.ast.get(node).clone() {
        Struct::List(children) => {
            for c in children {
                collect_rec_eff_call_defs(db, c, ctx, out);
            }
        }
        Struct::Atom(_) => {}
    }
}

/// Whether `node` contains an ABORTIVE same-handler perform — a call to one of this handler's ops whose arm
/// is in `ctx.abortive` (a `(fin (u) s s)` with no `resume`). Used to detect the breaker-sr5 shape where a
/// same-effect ABORT arm observes a recursive callee's out-state (which the abort collapse currently reads
/// from the pre-recursion SEED slot, not the threaded out-state → silent miscompile).
fn contains_abortive_perform(db: &mut Db, node: StructId, ctx: &HandlerCtx) -> bool {
    if let Resolved::Apply { head, .. } = resolved_of(db, node)
        && let Some(id) = is_perform(db, head, ctx)
        && ctx.abortive.contains(&id)
    {
        return true;
    }
    match db.ast.get(node).clone() {
        Struct::List(children) => children
            .iter()
            .any(|&c| contains_abortive_perform(db, c, ctx)),
        Struct::Atom(_) => false,
    }
}

/// Whether the handle body has a strict spine on which a recursive-effectful callee (which advances THIS
/// handler's state, the same shape `mark_caller_observed_outstate` upgrades to multi-value) is followed by an
/// element containing an ABORTIVE same-handler perform that observes that out-state — breaker sr5. The
/// multi-value out-state threads correctly to a RESUMING observer (sr4), but the ABORT collapse materializes
/// its arm value against the pre-recursion seed slot, dropping the recursion-era advances → reads the seed
/// (0) not the advanced state (2). Declined by `reduce_handle` until the abort collapse can read the threaded
/// out-state. NARROW — the observing element must contain an abortive perform, so a resuming observer (sr4),
/// a plain non-abort same-op observer (sr1), and the observer-as-recursion-base-case (sr2) do NOT match.
fn body_recursive_advance_observed_by_abort(db: &mut Db, node: StructId, ctx: &HandlerCtx) -> bool {
    fn scan_seq(db: &mut Db, seq: &[StructId], ctx: &HandlerCtx) -> bool {
        let mut pending: Vec<usize> = Vec::new();
        for &el in seq {
            // A recursive-effectful callee is pending, and a LATER spine element aborts observing its
            // out-state — the sr5 miscompile shape.
            if !pending.is_empty() && contains_abortive_perform(db, el, ctx) {
                return true;
            }
            collect_rec_eff_call_defs(db, el, ctx, &mut pending);
        }
        false
    }
    if let Resolved::Apply { args, .. } = resolved_of(db, node)
        && scan_seq(db, &args, ctx)
    {
        return true;
    }
    if let Some(items) = db.ast.as_form(node, "do").map(|t| t.to_vec())
        && scan_seq(db, &items, ctx)
    {
        return true;
    }
    if let Some(form) = db.ast.as_form(node, "let").map(|t| t.to_vec())
        && form.len() == 2
        && let Struct::List(pairs) = db.ast.get(form[0]).clone()
    {
        let mut seq: Vec<StructId> = pairs
            .iter()
            .filter_map(|&pair| match db.ast.get(pair).clone() {
                Struct::List(kv) if kv.len() == 2 => Some(kv[1]),
                _ => None,
            })
            .collect();
        seq.push(form[1]);
        if scan_seq(db, &seq, ctx) {
            return true;
        }
    }
    if let Resolved::Match { scrutinee, arms } = resolved_of(db, node) {
        for &(_, arm_body) in &arms {
            if scan_seq(db, &[scrutinee, arm_body], ctx) {
                return true;
            }
        }
    }
    // Recurse structurally so a spine nested inside a branch/let/operand is scanned too.
    if let Struct::List(children) = db.ast.get(node).clone() {
        for c in children {
            if body_recursive_advance_observed_by_abort(db, c, ctx) {
                return true;
            }
        }
    }
    false
}

/// Scan the HANDLE BODY for a recursive-effectful call whose FINAL out-state is OBSERVED by a LATER item on
/// the same strict spine — `(do (run-ops …) (Prim.run 0))` where the trailing perform reads the state the
/// recursion advanced — and record each such callee in `db.force_multivalue` so `specialize_recursive`
/// upgrades it to the MULTI-VALUE calling convention (return `(value, out-state)` instead of a bare value).
/// The single-return convention returns the INCOMING state unchanged as a recursive call's out-state, so a
/// caller's continuation after the recursion sees the PRE-recursion state — the cross-fn-fold out-state
/// silent miscompile (task #15). This is the CALLER-side analogue of `selfcall_precedes_perform_in_operands`
/// (which flags the same shape INSIDE the recursive def). It marks only the observation; the mode decision
/// in `specialize_recursive` still requires the callee to be MULTI-VALUE-THREADABLE, so an unthreadable
/// callee stays single-return (unchanged) rather than declining — purely additive, no regression.
///
/// Spine positions (mirroring `selfcall_precedes_perform_in_operands`): `do` items, `let` inits then body,
/// an operator's operands, and a `match` scrutinee → each arm body. In each, once an element CONTAINS a
/// recursive-effectful call, any LATER element that reaches ANY perform observes its out-state. (A later
/// element that itself contains a recursive-effectful call reaches a perform too — `contains_any_perform`
/// over-reports a recursive callee — so the two-sibling-caller-calls shape is covered by the same rule; the
/// `thread` do/let/operand arms already thread `cur` between the two, so multi-value carries the advance.)
fn mark_caller_observed_outstate(db: &mut Db, node: StructId, ctx: &HandlerCtx) {
    // Scan an ordered SEQUENCE of spine elements: an earlier element's recursive-effectful callee whose
    // out-state a later element observes (reaches a perform) is recorded.
    fn scan_seq(db: &mut Db, seq: &[StructId], ctx: &HandlerCtx) {
        let mut pending: Vec<usize> = Vec::new();
        for &el in seq {
            if !pending.is_empty() && contains_any_perform(db, el, ctx) {
                let key = ctx.key.clone();
                for &cd in &pending {
                    if let Some(body) = db.defs[cd].body {
                        db.force_multivalue.insert((body, key.clone()));
                    }
                }
            }
            collect_rec_eff_call_defs(db, el, ctx, &mut pending);
        }
    }
    // An operator/user application: operands evaluate left-to-right (a strict spine).
    if let Resolved::Apply { args, .. } = resolved_of(db, node) {
        scan_seq(db, &args, ctx);
    }
    // A `do` sequences its items; a `let` its inits then its body. Both are strict spines.
    if let Some(items) = db.ast.as_form(node, "do").map(|t| t.to_vec()) {
        scan_seq(db, &items, ctx);
    }
    if let Some(form) = db.ast.as_form(node, "let").map(|t| t.to_vec())
        && form.len() == 2
        && let Struct::List(pairs) = db.ast.get(form[0]).clone()
    {
        let mut seq: Vec<StructId> = pairs
            .iter()
            .filter_map(|&pair| match db.ast.get(pair).clone() {
                Struct::List(kv) if kv.len() == 2 => Some(kv[1]),
                _ => None,
            })
            .collect();
        seq.push(form[1]); // the body follows the inits on the spine
        scan_seq(db, &seq, ctx);
    }
    // A `match` evaluates its scrutinee then the selected arm body — scrutinee before each arm.
    if let Resolved::Match { scrutinee, arms } = resolved_of(db, node) {
        for &(_, arm_body) in &arms {
            scan_seq(db, &[scrutinee, arm_body], ctx);
        }
    }
    // Recurse structurally so a spine nested inside a branch/let/operand is scanned too.
    if let Struct::List(children) = db.ast.get(node).clone() {
        for c in children {
            mark_caller_observed_outstate(db, c, ctx);
        }
    }
}

/// Collect the recursive-effectful callee def-indices TRANSITIVELY reachable from a self-call ARGUMENT `node`,
/// following NON-RECURSIVE helper bodies (finding #19 indirection face). A DIRECT recursive-effectful call is
/// recorded (via `collect_rec_eff_call_defs`); a call to a NON-recursive def is FOLLOWED into that def's body
/// (so `(via d)` where `via` calls the recursive `inner` reaches `inner`). Depth-general (any indirection chain
/// length, s19f) with a `visiting` cycle-guard. A recursive def's body is NOT descended (it is itself the
/// performer, already recorded by `collect_rec_eff_call_defs`). This makes the indirection variant reach the
/// same recursion-boundary marking as the direct case — and paired with the transitive decline guard, the
/// indirection reaches the sound DECLINE floor instead of the pre-fix silent wrong value.
fn collect_transitive_rec_eff_in_arg(
    db: &mut Db,
    node: StructId,
    ctx: &HandlerCtx,
    hits: &mut Vec<usize>,
    visiting: &mut Vec<usize>,
) {
    collect_rec_eff_call_defs(db, node, ctx, hits);
    if let Resolved::Apply { head, .. } = resolved_of(db, node)
        && let Some(cd) = callee_def_index_of(db, head)
        && let Some(cbody) = db.defs[cd].body
        && !crate::eval::is_recursive(db, cbody) // a recursive callee is recorded directly; don't descend
        && !visiting.contains(&cd)
    {
        visiting.push(cd);
        collect_transitive_rec_eff_in_arg(db, cbody, ctx, hits, visiting);
        visiting.pop();
    }
    if let Struct::List(children) = db.ast.get(node).clone() {
        for c in children {
            collect_transitive_rec_eff_in_arg(db, c, ctx, hits, visiting);
        }
    }
}

/// RECURSION-BOUNDARY caller-observed out-state (finding #19). `mark_caller_observed_outstate` above catches a
/// callee whose out-state a LATER SPINE ITEM in the SAME body observes — but it scans only the handle body, and
/// a callee's out-state can also be observed ACROSS A RECURSION: when a recursive-effectful def `D` calls
/// another recursive-effectful callee `C` in one of `D`'s OWN self-call's ARGUMENTS — `(def (outer k acc) …
/// (outer (- k 1) (+ acc (inner d 0))))` — `C`'s (`inner`'s) state advance must thread into `D`'s self-call so
/// the NEXT `D` iteration reads the advanced state (its own next perform / draw). Single-return drops `C`'s
/// advance every `D` iteration (the composed-recursion silent wrong value, finding #19). Neither
/// `mark_caller_observed_outstate` (scans the handle body, which is just `(outer 3 0)` — no spine) nor
/// `selfcall_precedes_perform_in_operands` (keys on `D`'s OWN self-calls, so `C` in the arg is invisible)
/// catches it. So scan each recursive-effectful def `D` reachable from the handle body: for every self-call of
/// `D`, mark EACH recursive-effectful callee found in that self-call's ARGUMENTS — and `D` itself — as
/// `force_multivalue`, so both take multi-value mode and thread `C`'s `(. t 1)` out-state into `D`'s recursion.
/// Purely additive (only UPGRADES a multi-value-threadable callee; the mode decision still gates on
/// `multivalue_leaves_threadable`, so a non-threadable one stays single-return / declines as before). The
/// pre-recursion-let non-tail shape (`(let ((x (inner …))) (if … (outer …)))`) is NOT flagged here — `x` is not
/// in the self-call's arguments — so its existing honest decline (breaker s19c) is preserved.
fn mark_recursion_boundary_observed_outstate(db: &mut Db, handle_body: StructId, ctx: &HandlerCtx) {
    // Collect the recursive-effectful defs reachable from the handle body (the same call-graph the fold walks).
    let mut roots: Vec<usize> = Vec::new();
    collect_rec_eff_call_defs(db, handle_body, ctx, &mut roots);
    // Transitively close over callees (s19e: the demand survives an indirection hop `outer → via-k → inner`).
    // A `HashSet` visited-set keeps the worklist O(V+E), not O(V²) from a `Vec::contains` membership test —
    // negligible on today's tiny rec-effectful subgraph, but the trivial scalable form (v-compiler-perf
    // advisory, mirroring the `08678f992` mutual_scc fix). `roots` stays a Vec for stable iteration order.
    let mut seen: crate::fxhash::FxHashSet<usize> = roots.iter().copied().collect();
    let mut i = 0;
    while i < roots.len() {
        let d = roots[i];
        i += 1;
        if let Some(body) = db.defs[d].body {
            let mut callees: Vec<usize> = Vec::new();
            collect_rec_eff_call_defs(db, body, ctx, &mut callees);
            for c in callees {
                if seen.insert(c) {
                    roots.push(c);
                }
            }
        }
    }
    // For each recursive-effectful def D, find its self-calls and mark any recursive-effectful callee in a
    // self-call's ARGUMENTS (+ D itself) multi-value.
    let key = ctx.key.clone();
    for &d in &roots {
        let Some(body) = db.defs[d].body else {
            continue;
        };
        // Walk D's body for self-calls `(D args…)`; for each, scan its args for recursive-effectful callees.
        fn scan_self_call_args(
            db: &mut Db,
            node: StructId,
            d: usize,
            ctx: &HandlerCtx,
            hits: &mut Vec<usize>,
        ) {
            if let Resolved::Apply { head, args } = resolved_of(db, node)
                && callee_def_index_of(db, head) == Some(d)
            {
                for a in args.iter().copied().collect::<Vec<_>>() {
                    // TRANSITIVE (finding #19 indirection face, s19e/f): a recursive-effectful callee whose
                    // out-state must thread can be reached through NON-RECURSIVE helper defs — `outer`'s
                    // self-call arg `(via d)` where `via` (non-recursive) calls the recursive `inner`. A direct
                    // `collect_rec_eff_call_defs` misses it (`via` is not itself recursive-effectful), so `outer`
                    // stayed unmarked → single-value → SILENT MISCOMPILE. Collect through non-rec helpers so the
                    // reachable performer is recorded; that marks `outer` caller-observed, and the transitive
                    // decline guard above then declines the indirection cleanly (the sound floor).
                    collect_transitive_rec_eff_in_arg(db, a, ctx, hits, &mut Vec::new());
                }
            }
            if let Struct::List(children) = db.ast.get(node).clone() {
                for c in children {
                    scan_self_call_args(db, c, d, ctx, hits);
                }
            }
        }
        let mut hits: Vec<usize> = Vec::new();
        scan_self_call_args(db, body, d, ctx, &mut hits);
        if !hits.is_empty() {
            // D's self-call carries a recursive-effectful callee's out-state → D must be multi-value, and each
            // such callee must be too (so its out-state is a runtime `(value, out-state)` D can thread).
            db.force_multivalue.insert((body, key.clone()));
            for c in hits {
                if let Some(cbody) = db.defs[c].body {
                    db.force_multivalue.insert((cbody, key.clone()));
                }
            }
        }
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

/// ACCUM-COPY REDIRECT detector (rn post-observer fix, increment 1). A linear NON-tail recursion `f` that
/// `accum::introduce` rewrote into a seed-wrapper `(def (f p…) (f$acc p… seed…))` + a tail-recursive copy
/// `f$acc` reads `is_recursive(f.body)=false` — so specializing `f` (the seed-wrapper) under a MERGED
/// context threads the NON-recursive wrapper (its body is just the seed call), which drops the advance (the
/// rn post-observer safe-decline). The ACTUAL recursion lives in `f$acc`. When `def_index` is such a
/// seed-wrapper — `slots>1` (merged), `db.transformed.get(&def_index)=Some(f$acc)`, body is a call
/// `(f$acc <orig-args…> <seed…>)` naming `f$acc` — return `Some((f$acc-index, seed-args))`: the caller
/// specializes `f$acc` and threads the SEEDS (the wrapper-body args past the original params) as extra
/// call-site args (before the trailing states, mirroring the captured-enclosing-param plumbing). Approach
/// (a), concierge-steered (2026-08-04): reading the seed from the wrapper body handles a CONSTANT or a
/// NON-constant seed uniformly — no latent hole. `None` (no redirect) for a non-wrapper / single-slot ctx
/// (the inside-out path already specializes `f$acc` directly). The returned seeds are DEEP-FRESH copies,
/// safe to splice at a call site. Idempotent (a pure read of `db.defs`/`db.transformed`/the AST).
fn accum_seed_redirect(
    db: &mut Db,
    def_index: usize,
    slots: usize,
) -> Option<(usize, Vec<StructId>)> {
    if slots <= 1 {
        return None;
    }
    let wrapper_body = db.defs[def_index].body?;
    if crate::eval::is_recursive(db, wrapper_body) {
        return None; // already the recursive copy (or a genuine recursion) — not a seed-wrapper
    }
    let &acc = db.transformed.get(&def_index)?;
    let wrapper_params = db.defs[def_index].params.len();
    let Struct::List(call_children) = db.ast.get(wrapper_body).clone() else {
        return None;
    };
    // The wrapper body must be a call `(f$acc <orig-args…> <seed…>)` naming the accum copy, with at least
    // the wrapper's own params as leading args (the rest are the accumulator seeds `f$acc` introduced).
    if call_children.is_empty()
        || db.ast.as_name(call_children[0]) != Some(db.defs[acc].name.as_str())
        || call_children.len() - 1 < wrapper_params
    {
        return None;
    }
    let seeds: Vec<StructId> = call_children[1 + wrapper_params..]
        .iter()
        .map(|&s| deep_fresh_copy(db, s))
        .collect();
    Some((acc, seeds))
}

fn specialize_recursive(db: &mut Db, head: StructId, ctx: &HandlerCtx) -> Option<String> {
    let call_head_def = callee_def_index_of(db, head)?;
    // ACCUM-COPY REDIRECT (rn post-observer fix, increment 1 — see `accum_seed_redirect`). A merged-ctx call
    // to a seed-wrapper `f` specializes the tail-recursive accum COPY `f$acc` instead (the wrapper's body is
    // just the non-recursive seed call, which drops the advance). The seed args the call site must add are
    // recomputed there via the same `accum_seed_redirect`; here we only need the redirected def index.
    let callee_def = accum_seed_redirect(db, call_head_def, ctx.slots.len())
        .map_or(call_head_def, |(acc, _seeds)| acc);
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
    // CALLER-OBSERVED OUT-STATE (task #15): the handle body has `(do (f …) (E.op))` where a LATER spine item
    // observes THIS callee's final out-state (recorded by `mark_caller_observed_outstate` before threading).
    // The single-return convention returns the incoming state unchanged, so the observer reads the
    // PRE-recursion state — a silent miscompile. Force multi-value mode so the advance threads to the
    // observing continuation. Only takes effect when the callee is multi-value-threadable (checked below);
    // an unthreadable callee stays single-return (no regression — the miscompile is pinned, not newly broken).
    let caller_observes_outstate = db.force_multivalue.contains(&(orig_body, ctx.key.clone()));
    // A caller-observed MUTUALLY-recursive callee cannot be threaded by the multi-value tuple machinery: it
    // threads a SELF-call's out-state, but a mutual-SCC callee's recursion goes through a SIBLING def (`ea`
    // calls `eb` calls `ea`), whose out-state the self-call arm does not project — forcing multi-value here
    // produces a body leaking the internal `$s0`/`$t0` state-param names (a confusing CDZ0101). Rather than
    // leak — or silently miscompile via single-return (the sibling's out-state is dropped, a DISTINCT
    // pre-existing miscompile the breaker filed separately) — DECLINE cleanly, an honest "not yet reducible"
    // todo. (Threading a mutual-SCC's out-state across the whole recursive group needs group-wide multi-value
    // specialization — a later increment.) Only the CALLER-OBSERVED mutual case declines; a bare mutual
    // recursion with no observed out-state still specializes single-return (unaffected).
    // TRANSITIVE reachability (finding #19 indirection face): a caller-observed callee whose recursion reaches
    // ANOTHER recursive performer — DIRECTLY or through a NON-RECURSIVE helper (`outer → via → inner`) — cannot
    // be soundly multi-value-threaded here (the sibling/indirect performer's out-state is not projected), so it
    // must reach the DECLINE floor. `callee_calls_other_recursive_def` was direct-only, so the indirection
    // variant slipped past this guard into a single-return that DROPPED the advance (silent miscompile 9 vs 7);
    // the transitive variant follows the pass-through so the indirection declines cleanly like the direct case.

    // GROUP-AWARE MULTI-VALUE (the mutual-performer SCC fold). This body is a member of a mutually-recursive
    // SCC being group-specialized in multi-value mode together — recorded in `group_multivalue_bodies` (either
    // because THIS is the entry call whose SCC we detect just below, or because an OUTER entry already
    // registered the whole group). A group member ALWAYS threads multi-value: each member returns `(value,
    // out-states…)`, and a cross-def call to a partner is let-bound + out-state-projected by the head-agnostic
    // recursive-call arm (which keys on `multivalue_specs`). This is what threads a mutual partner's state
    // advance to a later observer, replacing the single-return dropped-advance miscompile.
    let group_member = db
        .group_multivalue_bodies
        .contains(&(orig_body, ctx.key.clone()));
    // Detect the ENTRY of a mutual-performer SCC whose out-state a later spine item observes: the shape the
    // single-return floor declines (`mutual_partner_precedes_observation`). Collect the whole SCC and register
    // every member for group multi-value, so each member's specialization (reached via the recursive-call arm)
    // threads multi-value and the partners tie together. Only when the leaves are threadable across the group.
    // Detect a mutual-performer SCC that needs the group fold — fired at the FIRST-REACHED member (which may
    // NOT be the observing one: the handle body calls `typeof`, whose partner call to `compute` is a TAIL call
    // observing nothing, so `typeof` alone does not trip `mutual_partner_precedes_observation` — but its
    // partner `compute` DOES, `(let ((c (typeof …))) (+ c (St.get)))`). So scan the WHOLE SCC for ANY member
    // that observes a partner's out-state; if one exists (and every member's leaves are group-threadable),
    // register the entire group up front. Whichever member is specialized first becomes the registrar.
    let group_entry = !group_member && ctx.abortive.is_empty() && {
        let scc = mutual_scc_of(db, callee_def, ctx);
        // A genuine mutual SCC (more than just this def) with at least one out-state-observing member, all of
        // whose leaves the group multi-value machinery can bind.
        scc.len() > 1
            && scc.iter().any(|&m| {
                db.defs[m]
                    .body
                    .is_some_and(|mb| mutual_partner_precedes_observation(db, mb, m, ctx))
            })
            && scc.iter().all(|&m| {
                db.defs[m]
                    .body
                    .is_some_and(|mb| group_multivalue_leaves_threadable(db, mb, m))
            })
    };
    if group_entry {
        let scc = mutual_scc_of(db, callee_def, ctx);
        for &m in &scc {
            if let Some(mb) = db.defs[m].body {
                db.group_multivalue_bodies.insert((mb, ctx.key.clone()));
            }
        }
    }
    let multivalue = ((selfcall_precedes_perform_in_operands(db, orig_body, callee_def, ctx)
        || caller_observes_outstate)
        && ctx.abortive.is_empty()
        && multivalue_leaves_threadable(db, orig_body, callee_def))
        || group_member
        || group_entry;
    if selfcall_precedes_perform_in_operands(db, orig_body, callee_def, ctx) && !multivalue {
        return None; // an out-state-observing shape the multi-value path does not cover yet (abortive, or
        // a self-call gated behind a conditional inside a leaf) — decline BEFORE reserving the def.
    }
    // MUTUAL-PARTNER OUT-STATE OBSERVED (the group-fold soundness floor). A mutual-partner call PRECEDES an
    // out-state observation on the strict spine. If this is NOT a group multi-value member/entry (the group
    // path handles it above), fall back to the clean SINGLE-return decline: single-return would thread the
    // partner call with the incoming state and return it unchanged, dropping the partner's advance — a SILENT
    // wrong value. DECLINE cleanly (an honest "not yet reducible" todo) rather than miscompile. (A group
    // member/entry does NOT decline here — its whole SCC threads multi-value.)
    if !group_member
        && !group_entry
        && mutual_partner_precedes_observation(db, orig_body, callee_def, ctx)
    {
        return None;
    }
    // BRANCH-PERFORMING CONDITIONAL alongside a re-entrant call (v-effects self-probe 2026-08-04,
    // breaker-confirmed rw1-rw5, concierge-greenlit safe-decline). A discharged perform inside a conditional
    // BRANCH that is a strict operand ALONGSIDE a self-call / mutual-recursive call — `(+ (if c (St.get) 0)
    // (walk (- n 1)))` — has its advance dropped across the recursion: the single-return specialization
    // threads the branch perform against the INCOMING state, but the advance is branch-local and the recursion
    // carries the incoming state forward (seeded 1 → 3 not 6, all backends). The non-recursive branch-perform
    // hoist does NOT run inside the specialized body. DECLINE cleanly (safe floor) rather than fold the
    // dropped-advance wrong value; a full fold needs the branch-perform lifted before specialization (a later
    // increment). PRECISE (not a naive "perform under any conditional" scanner, which would decline every
    // recursive fn's base-case `if`): fires only when the branch-perform's conditional and a re-entrant call
    // are operands of the SAME strict node. Covers runtime-cond (rw3), mutual-SCC (rw4, via
    // `contains_recursive_call`), and heap-state (rw5, state-shape-agnostic). Does NOT over-decline the
    // FOLDING shapes: bare tail perform `(+ (St.get) (walk …))` (perform is a direct operand, not under a
    // branch), let-init-bound perform (`sum-down`), or perform-as-whole-branch with the self-call in a SIBLING
    // branch (`ev`/`od`: mutually exclusive, no shared strict context). Placed after the multivalue decision
    // so a shape that path linearizes is not pre-empted.
    if !multivalue && branch_perform_coexists_with_reentrant_call(db, orig_body, callee_def, ctx) {
        return None;
    }
    // CROSS-DEF RECURSION-BOUNDARY safe floor (finding #19), NARROWED. A caller-observed callee whose
    // recursion reaches ANOTHER recursive performer needs that callee's out-state threaded across the
    // recursion. This now FOLDS for the ONE-WAY nested case (`outer` calls `inner`, `inner` does NOT call
    // back — SCC of `outer` is just `{outer}`) under MULTI-VALUE mode: `thread_returning_tuple`'s
    // let-dispatch arm threads the cross-def out-state (nr0/nr1/nr10). It still DECLINES cleanly when the
    // fold would be unsound:
    //   * SINGLE-return (`!multivalue`, e.g. a self-call gated behind a conditional): single-return drops
    //     the cross-def advance every iteration — a silent miscompile (9 vs 7).
    //   * a MUTUAL SCC (the reached performer cycles BACK — `ea`↔`eb`, so `mutual_scc_of` size > 1):
    //     multi-value threads a SELF-call's out-state but not a mutual SIBLING's, so forcing it would leak
    //     the internal `$s0`/`$t0` names.
    // The `mutual_scc_of(callee_def).len() <= 1` test is what distinguishes the now-foldable ONE-WAY nested
    // case (`outer`'s SCC is just `{outer}`) from the still-declining MUTUAL case. (This replaced an
    // unconditional pre-group-detection decline; a caller-observed + transitive shape never reached the
    // group fold under that guard, so declining one here — group-registered or not — matches prior behavior.)
    if caller_observes_outstate
        && callee_transitively_calls_other_recursive_def(db, orig_body, callee_def, &mut Vec::new())
        && !(multivalue && mutual_scc_of(db, callee_def, ctx).len() <= 1)
    {
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

    // The specialized NAME — unique per (def, context). The `#` makes it unbindable in source (a `#`-prefixed
    // name is CDZ0210 in binder position, so no user binder can collide); the def-count suffix keeps distinct
    // specializations distinct.
    let base = db.defs[callee_def].name.clone();

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
    // Original param NAMES — a capture whose NAME collides with an original param must NOT be threaded as an
    // extra param: the spec fn already binds that name (the original param shadows the capture inside the body),
    // so appending it would (a) make the param list non-linear (CDZ0102 `id` bound more than once — breaker
    // two-effect-helpers, where two helpers driving the SAME DbState group each capture a cross-scope `id`
    // whose binder StructId differs from `type-of`'s own `id` param, dodging the by-StructId `own_binders`
    // filter) and (b) bind the wrong value (the body's `id` reads the original, not the appended capture).
    let mut orig_param_names: std::collections::HashSet<String> =
        orig_param_specs.iter().map(|(n, _)| n.clone()).collect();
    // ALSO seed with each arm's STATE-BINDER name (`db` in the DbState demand spine): a reference to the
    // handler state binder must resolve to the threaded state slot `f#eff$s0`, NOT be captured as an extra
    // enclosing param. The by-StructId `own_binders` filter above already excludes the state binder's OWN
    // occurrence — but with MULTIPLE arms all binding the SAME state name (`db` in every DbState arm) plus
    // the fold's arm copies, a `db` reference can resolve to a state-binder StructId that dodges that filter
    // (the same StructId-vs-name gap the `orig_param_names` seed closes for `id`). Wrongly captured, `db`
    // threads as a raw param and the arm's `require-ty db id` / `fill-ty db id t` read the STALE param instead
    // of the threaded slot — so a `set-ty` write is invisible to the next `get-ty` → re-demand/wrong value →
    // wasm unreachable, and ONLY when the wide-op arms (get-tcol/get-tree) coexist to expose the extra
    // state-binder occurrences (v-cml bug-3: arg-flow test passes standalone, fails in-suite; the 4-way
    // SPECBODY dump shows `require-ty db id` in the failing build vs `require-ty type-of#eff$s0 id` in the
    // passing one). Seeding the state name skips the wrong capture; the state-binder substitution binds the
    // reference to `$s0` as it does for the single-arm case. A NAME shared with a real enclosing capture is
    // out of scope here (the state binder is bound within the spec — its name never denotes an outer value).
    orig_param_names.extend(
        ctx.arms
            .values()
            .filter_map(|a| db.ast.as_name(a.state).map(str::to_string)),
    );

    // FULL FOLD — thread-the-let-local (2026-08-27, on v-inference's `handle_lift_escapes` signal). A handler
    // arm may capture a MAIN-LOCAL `let` binding (`(let ((m (* n 3))) (handle St 0 ((get (u) s (resume m s)))
    // (loop2 n)))`): `captured_enclosing_params` threads enclosing-fn PARAMS but NOT a main-local `let` binder
    // (`collect_captures` matches `Resolved::Param` only), so the arm's `m`, spliced into the lifted def, rides
    // in out of scope. An a-priori scan over-declines — pre-thread, a resume value / threaded-state / do-local
    // reference is indistinguishable from a genuine escape. Instead: thread with the captures known so far,
    // then let `handle_lift_escapes` name the PRECISE escaping occurrence on the ACTUAL threaded body (it walks
    // resolved value positions, so a state / resume / do-local ref resolves fine and is never flagged). Thread
    // THAT main-local as an extra capture param and re-run — the arm's `m` becomes a spec param, and both the
    // self-call and the initial call append the main-scope `m` (in scope at the caller: `(loop2 n)` sits inside
    // the `let`), so the recursion carries the constant capture exactly as it does an enclosing param (xas2).
    // The escaping occurrence resolves `Poison` in the lifted def, so its TYPE is read from `let_cap_types`
    // (built from the pre-thread arm bodies, where the reference is still bound). An escape that is NOT a typed
    // main-local candidate is unthreadable → clean decline (the honest floor stays for the residue).
    let let_cap_types = collect_local_capture_types(db, ctx, &orig_param_names);
    let mut forced_captures: Vec<(String, crate::ty::Ty)> = Vec::new();
    loop {
        let spec_name = format!("{base}#eff{}", db.defs.len());
        let mut captured_specs =
            captured_enclosing_params(db, ctx, &own_binders, &orig_param_names);
        // Append the main-local captures the validator confirmed on a prior iteration (empty on the first pass),
        // after the enclosing-param captures and before the trailing states — the layout every call site appends
        // args in. Skip any name an enclosing-param capture already covers (it resolves, so it never escapes).
        for (n, ty) in &forced_captures {
            if !captured_specs.iter().any(|(cn, _)| cn == n) {
                captured_specs.push((n.clone(), ty.clone()));
            }
        }
        // A capture with an undetermined type cannot annotate its extra param — decline the whole specialization
        // (mirrors the `orig_params` `Ty::Any` guard), so the shape stays a clean todo rather than emitting a
        // loosely-typed param.
        if captured_specs.iter().any(|(_, ty)| ty_has_any(ty)) {
            return None;
        }
        let capture_names: Vec<String> = captured_specs.iter().map(|(n, _)| n.clone()).collect();

        let spec_name_atom = db.push_name(&spec_name);
        let mut sig_children = vec![spec_name_atom];
        for (n, ty) in &orig_param_specs {
            let name_atom = db.push_name(n);
            let ty_expr = crate::eval::encode_typeval(db, ty);
            let colon = db.push_name(":");
            sig_children.push(db.push_list(vec![colon, name_atom, ty_expr]));
        }
        // The captured enclosing-fn params — annotated with each capture's solved type, AFTER the originals and
        // BEFORE the trailing states (the layout every call site appends args in: orig, captured, states).
        for (n, ty) in &captured_specs {
            let name_atom = db.push_name(n);
            let ty_expr = crate::eval::encode_typeval(db, ty);
            let colon = db.push_name(":");
            sig_children.push(db.push_list(vec![colon, name_atom, ty_expr]));
        }
        // The trailing state params — one per slot, named `{spec}$s{k}`, annotated with the slot's state type.
        let state_names: Vec<String> = (0..slot_tys.len())
            .map(|k| format!("{spec_name}$s{k}"))
            .collect();
        for (k, ty) in slot_tys.iter().enumerate() {
            let state_name = db.push_name(&state_names[k]);
            let state_type_expr = crate::eval::encode_typeval(db, ty);
            let colon = db.push_name(":");
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
        // CLONE the key at the insert (not move): `handle_lift_escapes` below may find the just-filled body
        // ESCAPES the spec scope and decline the fold, which must ROLL BACK this registration — else a later
        // identical call hits the still-present memo, returns `spec_name` WITHOUT re-validating, and emits the
        // poison def (the bogus CDZ0101 returns). Keeping `memo_key` lets the rollback remove it.
        db.effect_specializations
            .insert(memo_key.clone(), spec_index);
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
        let state_refs: Vec<StructId> = state_names.iter().map(|n| db.push_name(n)).collect();
        // PRE-SPEC-LIFT (recursive-nested-arm-resume fix, concierge-steered). When the recursive callee calls an
        // INNER op whose ARM resume-value performs an OUTER (merged-slot) op — `(step (u) t (resume (A.tick) t))`
        // — that outer perform is hidden until the inner op folds mid-thread, and re-threading the peeled resume
        // value produces a mis-scoped state-ref (the disproven arm-fold approach). INSTEAD, substitute the inner-
        // op call with its arm's peeled resume VALUE directly in `orig_body` BEFORE threading, so the outer
        // perform becomes a DIRECT-body perform threaded via the top-level perform arm — binding the spec's
        // `state_refs` identically to the working direct-body cases (14-eff:7706 `(+ (A.geta) (B.getb))`). Only
        // fires in the MERGED ctx (>1 slot) for an inner op whose arm reaches an outer perform AND threads its
        // own state trivially (next-state == state binder — no inner-op state advance to preserve); a non-trivial
        // inner-state advance is left to the ordinary fold (unchanged). Byte-identical when nothing matches.
        let orig_body = if ctx.slots.len() > 1 {
            lift_inner_op_arm_outer_perform(
                db,
                orig_body,
                ctx,
                caller_observes_outstate,
                &state_names,
            )
        } else {
            orig_body
        };
        // [tpwJ A-tight] Mark that we are threading a RECURSIVE-DRIVER body, so the perform arm's cross-scope
        // tuple COLLAPSE stays OFF here (it is unsound under recursive specialization — rq3). Restored after.
        let saved_recur = ctx.in_recursive_specialize.get();
        ctx.in_recursive_specialize.set(true);
        // MULTI-VALUE mode: the body's every tail leaf yields `("tuple" value out-states…)`, and each self-call
        // is let-bound (out-state projected + threaded). SINGLE-return mode: the ordinary `thread` (unchanged).
        let spec_body = if multivalue {
            // SAVE/RESTORE the multi-value scratch (`temp_ctr` + `pending`) around threading THIS body. In the
            // GROUP fold, threading one member's body recurses (via the recursive-call arm → `specialize_recursive`)
            // into a PARTNER member's OWN multi-value thread, which resets `temp_ctr`/`pending` — corrupting this
            // member's in-progress pending self/partner-call temps (the `$t0` leak: the partner's `clear()` wiped
            // the entry's pending temp before `thread_returning_tuple` drained it). Snapshot before, restore after,
            // so each member's multi-value scratch is independent. (For a non-group single self-recursive spec the
            // partner recursion is absent, so save/restore is inert — byte-identical to the prior reset.)
            let saved_ctr = ctx.temp_ctr.get();
            let saved_pending = std::mem::take(&mut *ctx.pending.borrow_mut());
            ctx.temp_ctr.set(0);
            let threaded = thread_returning_tuple(db, orig_body, state_refs, ctx, callee_def);
            ctx.temp_ctr.set(saved_ctr);
            *ctx.pending.borrow_mut() = saved_pending;
            threaded?
        } else {
            // SINGLE-RETURN specialization: the ordinary `thread` with NO `drain_and_wrap` after it. SUPPRESS the
            // FINDING-24 growing-state `#st` bind here — a `#st` pushed to `ctx.pending` on this path would never
            // materialize (nothing drains it) → an orphan `#st..` reference in the spec body (CDZ0101). This path
            // is immune to the exponential anyway: a recursive callee threads the state as a fn PARAMETER through
            // the self-call (one static site), not by re-substituting a growing expr per dispatch, so the bind
            // wins no size here. Restore the flag after (the shared `ctx` may thread other, drainable, bodies).
            let saved_bind = ctx.bind_growing_state.get();
            ctx.bind_growing_state.set(false);
            let threaded = thread(db, orig_body, state_refs, ctx);
            ctx.bind_growing_state.set(saved_bind);
            let (b, _out) = threaded?;
            b
        };
        ctx.in_recursive_specialize.set(saved_recur);

        // DEEP-FRESH-COPY the threaded body so NO original-body node is shared into the spec. Threading /
        // `beta_reduce` return an unchanged subtree AS-IS (a node with no substituted param is not rebuilt), so
        // the spec body can SHARE original param-reference nodes (e.g. the `n` in `(St.put n)` reached through a
        // perform-arm-threaded state expression). `core_of` MEMOIZES by StructId (`db.core`), so a shared
        // original node carries its cached `Core::Param{ORIGINAL binder}` — which has no slot in the spec
        // function (its slots are keyed by the spec sig's param nodes) → "parameter reference has no local slot"
        // at emit when a later inline copies + lowers it. Re-pushing every node fresh gives the whole body new
        // StructIds with no `db.core` memo; each ref re-resolves (lazily, post-parent) against the spec `(def sig
        // …)` form below → binds to the spec sig param → gets a slot. (The self-recursive / multi-value temps
        // `$s{k}`/`$t{k}` and any `#cv` bindings are name-resolved, so a fresh copy re-binds them identically.)
        let spec_body = deep_fresh_copy(db, spec_body);

        // Wrap in a REAL `(def (spec params… (: s T)) spec_body)` arena node so the parent index links
        // param → sig → def: `is_param_occurrence` walks that chain to classify each param, and `binder_in`
        // Case 4 resolves a body reference against the def signature. Without this the synthesized params
        // would not resolve. The `db.defs` entry's `body` points at `spec_body` (the def-form node is for
        // scope/param resolution, not the emitted body — emission reads `db.defs[i].body`).
        let def_head = db.push_name("def");
        let _def_form = db.push_list(vec![def_head, sig, spec_body]);

        db.fill_specialized_def(spec_index, spec_params, spec_body);

        // EMISSION-SITE SAFE FLOOR (specialize_recursive escape, v-effects pairing 2026-08-27). A recursive
        // performer whose handler arm captures a MAIN-LOCAL `let` binding escapes the lifted top-level def:
        // `collect_captures` threads enclosing-fn PARAMS as extra spec params but NOT a main-local `let` binder
        // (it matches `Resolved::Param` only), so the captured name (`ys`/`m`) rides into the lifted def where
        // it is out of scope and resolves UNBOUND. A-priori detection over-declines (an arm's threaded-state /
        // resume / do-local refs are indistinguishable pre-lift); the escape is precise ONLY on the actual
        // threaded body. Validate it; on an escape DECLINE the fold cleanly (an honest "not yet reducible" todo)
        // instead of emitting a def whose reference dangles as a bogus CDZ0101. The validation walk MEMOIZES
        // resolution of the fresh spec body against the fill-time context, so FORGET it afterward — `core_of`
        // must re-resolve against the fully-populated context (a mutual partner spec / multi-value drain filled
        // AFTER this call).
        let escape = handle_lift_escapes(db, spec_body);
        crate::resolve::forget_subtree(db, spec_body);
        if let Some(node) = escape {
            // ROLL BACK every registration this iteration made (v-effects attempt-4 finding): the memo entry, the
            // multi-value flag, and the capture list. Without this a later identical call hits the stale memo
            // and returns `spec_name` un-revalidated, re-emitting the poison def. The reserved (now
            // filled-but-unreferenced) def is harmless — the fold declined this shape, so nothing names it.
            db.effect_specializations.remove(&memo_key);
            db.multivalue_specs.remove(&spec_name);
            db.effect_spec_captures.remove(&spec_name);
            // FULL FOLD: if the escaping occurrence is a typed main-local `let` capture we have NOT already
            // threaded, add it and re-run — next pass binds it to a fresh spec param, so it no longer escapes.
            // A name already in `forced_captures` that STILL escapes is genuinely unthreadable (threading it did
            // not bind it — an exotic shape); and an escape that is not a typed main-local candidate cannot be
            // annotated. Either way DECLINE cleanly (the honest "not yet reducible" floor), so the residue stays
            // a todo rather than emitting a def whose reference dangles as a bogus CDZ0101.
            let name = db.ast.as_name(node).map(str::to_string)?;
            if forced_captures.iter().any(|(n, _)| n == &name) {
                return None;
            }
            match let_cap_types.get(&name) {
                Some(ty) if !ty_has_any(ty) && !orig_param_names.contains(&name) => {
                    forced_captures.push((name, ty.clone()));
                    continue;
                }
                _ => return None,
            }
        }
        return Some(spec_name);
    }
}

/// After `specialize_recursive` fills a lifted spec def, return the first VALUE-POSITION occurrence in its
/// threaded body that ESCAPES the spec scope: a PLAIN-SOURCE name resolving UNBOUND (`Resolved::Poison`).
/// The lift splices the handler arm's resume value into the lifted def; a value the arm captured from a
/// MAIN-LOCAL `let` binding (`(let ((ys …)) (handle … (loop2 n)))`) then rides in out of scope —
/// `collect_captures` threads enclosing-fn PARAMS but not main-local `let` binders (it matches
/// `Resolved::Param` only), so the spliced reference resolves `Poison`. The returned occurrence also names
/// WHICH main-local a later full-fold increment must thread as an extra spec param. `None` = emittable.
///
/// Walks the RESOLVED FORM, recursing ONLY value-carrying children, so a NON-value name — a member-access
/// KEY (`len` in `(List.len ys)`), a `let`/`fn`/`match` BINDER occurrence — is never resolved-as-a-value
/// and never falsely flagged (the raw-atom walk's 54-case over-decline: those keys/binders resolve `Poison`
/// too). The PLAIN-NAME gate additionally skips the fold's synthesized machinery (state params `{spec}$s{k}`,
/// multi-value temps `$t{k}`, growing-state `#st`/`#seed`/`#cv`, partner/self spec names `{base}#eff{n}` —
/// all bear a `#`/`$`). Globals (helper defs, prelude) resolve via the flat `def_by_name`/prelude and are
/// not `Poison`. CONSERVATIVE: an exotic variant whose value children are not enumerated here is simply not
/// recursed — a missed escape leaves that shape a todo (no regression), never an over-decline.
pub(crate) fn handle_lift_escapes(db: &mut Db, node: StructId) -> Option<StructId> {
    match resolved_of(db, node) {
        Resolved::Poison(_) => {
            if let Some(name) = db.ast.as_name(node)
                && !name.contains('#')
                && !name.contains('$')
            {
                return Some(node);
            }
            None
        }
        // Value-child recursion — keys/binders are structurally absent from these child lists.
        Resolved::Apply { head, args } => handle_lift_escapes(db, head)
            .or_else(|| args.iter().find_map(|&a| handle_lift_escapes(db, a))),
        Resolved::Let { bindings, body } => bindings
            .iter()
            .find_map(|&(_binder, init)| handle_lift_escapes(db, init))
            .or_else(|| handle_lift_escapes(db, body)),
        Resolved::If { cond, then_, else_ } => handle_lift_escapes(db, cond)
            .or_else(|| handle_lift_escapes(db, then_))
            .or_else(|| handle_lift_escapes(db, else_)),
        Resolved::And { lhs, rhs, .. } => {
            handle_lift_escapes(db, lhs).or_else(|| handle_lift_escapes(db, rhs))
        }
        Resolved::Not { operand } | Resolved::Try { operand } => handle_lift_escapes(db, operand),
        Resolved::Match { scrutinee, arms } => handle_lift_escapes(db, scrutinee).or_else(|| {
            arms.iter()
                .find_map(|&(_pat, body)| handle_lift_escapes(db, body))
        }),
        Resolved::Member { operand, .. } | Resolved::Proj { operand, .. } => {
            handle_lift_escapes(db, operand)
        }
        Resolved::Tuple { elems } | Resolved::List { elems } => {
            elems.iter().find_map(|&e| handle_lift_escapes(db, e))
        }
        Resolved::Record { fields } => fields
            .values()
            .copied()
            .collect::<Vec<_>>()
            .into_iter()
            .find_map(|v| handle_lift_escapes(db, v)),
        Resolved::Annot { expr, .. } | Resolved::ConstBlock { expr } => {
            handle_lift_escapes(db, expr)
        }
        // Leaves (literals, resolved Ref/Param, Prim, TypeVal) and any variant whose value children are not
        // enumerated: do not recurse. A Ref/Param means the name IS bound (not an escape); an unenumerated
        // exotic variant is conservatively skipped (a missed escape is a todo, never an over-decline).
        _ => None,
    }
}

/// Build a tuple projection `(. <name> index)` — a fresh bare-name reference to `name` projected at
/// `index`. Used by the multi-value self-call rewrite to read a let-bound self-call temp's value (`.0`)
/// and each slot's out-state (`.{slot+1}`).
fn tuple_proj(db: &mut Db, name: &str, index: u32) -> StructId {
    let dot = db.push_name(".");
    let name_atom = db.push_name(name);
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
///     NEXT-STATE is a SECOND match rebuilt around each branch's next-state `(match scrut (pat s)…)` — over
///     the SAME (pure) scrutinee. Keeping BOTH match-wrapped is load-bearing: a branch's next-state may
///     reference the branch's PATTERN BINDERS (the DB `put` arm's `Map.insert(s, k, v)` uses `k`,`v`), so it
///     CANNOT be hoisted out of the match — it must stay inside its branch. A branch-DEPENDENT next-state is
///     therefore ALLOWED (each branch keeps its own); the state threads forward as a match-VALUED expression,
///     sound because the scrutinee is the pure op arg (evaluating it in both matches duplicates no effect).
///     Every branch must itself peel to a resume. (v-compiler-ml's get/put memoized-DB shape: a `put` arm
///     `(match kv (| (k,v) => resume(unit, Map.insert(s,k,v))))` performed in a `;`-sequence with a `get`.)
///
/// PRE-SPEC-LIFT (recursive-nested-arm-resume fix): rewrite `node` (a recursive callee's body about to be
/// threaded under a MERGED ctx) so an inner-op call whose ARM resume-value performs an OUTER effect becomes
/// that outer perform DIRECTLY in the body spine. `(step 〈args〉)` where `step`'s arm is `(u) t (resume
/// 〈val performing A〉 t)` → replaced by the β-reduced `val` (params↦args, state↦a fresh state ref). This
/// makes the outer perform a direct-body perform (threaded via the top-level perform arm with the spec's
/// `state_refs` in scope), instead of surfacing mid-fold with a re-entrant mis-scoped state-ref.
///
/// NARROW (soundness): fires ONLY when the arm (1) peels to a bare `(resume val next)` (no do/match wrapper),
/// (2) has TRIVIAL state advance — `next` is exactly the state binder, so the inner op does not step its own
/// state (q4a's `(resume (A.tick) t)`; a real inner-state advance is left to the ordinary fold), (3) `val`
/// reaches a perform of a DIFFERENT discharged op (an outer merged slot), and (4) the op is 4-part (no `cont`
/// — an E5 continuation arm is out of scope). Anything else is returned unchanged (byte-identical). The
/// substituted `val` is `deep_fresh_copy`'d so its param/state refs re-resolve against the spec body freshly.
/// Whether `node` reaches a DIRECT perform of an op discharged by `ctx` that is NOT `own` — i.e. a sibling
/// (outer merged-slot) discharged op. Used by the pre-spec-lift to detect an arm resume-value that performs
/// a DIFFERENT effect than the arm's own op (the arm-hidden outer perform).
/// Whether the subtree at `node` contains a VALUE reference that resolves to the binder `binder` — a bare
/// name occurrence whose `resolved_of` chain reaches `binder` (an arm's state/param binder). Used by
/// `lift_inner_op_arm_outer_perform` to REFUSE lifting a resume value that reads the inner arm's state
/// binder (which the lift's `params↦args`-only substitution does not rebind, so lifting would orphan it).
/// A structural walk; the binder-position occurrence of `binder` itself is not a value reference, but the
/// lift only inspects a resume VALUE (never the binder slot), so any match here is a genuine read.
/// Whether THIS ONE node is a value reference reaching `binder` — a `Param { binder }` whose binder IS it,
/// or a `Ref` whose chain reaches it transitively (matching how `beta_reduce` substitutes: an op-arm param
/// `p` used as `(. p 0)` resolves to a `Ref` reaching `p`'s declaration occurrence, not a `Param`). The
/// SINGLE SOURCE OF TRUTH for "what counts as a reference to `binder`", shared by `subtree_references_binder`
/// (a short-circuit boolean walk) and `count_param_refs` (a counting walk) so the two can never diverge on
/// the ref-chain rule (github-liaison/Copilot #2102/#2128 review).
fn node_refs_binder(db: &mut Db, node: StructId, binder: StructId) -> bool {
    match resolved_of(db, node) {
        Resolved::Param { binder: b } => b == binder,
        Resolved::Ref { value } => {
            let mut target = value;
            // Bound the Ref-chain walk with a visited-set so a Ref CYCLE that does NOT pass through `binder`
            // (a→b→a, neither = binder) terminates instead of spinning forever — the resolver CAN represent
            // Ref cycles (`resolve::value_ref_cycle`), and this predicate is called from recursive subtree
            // walks, so an unguarded loop would HANG compilation rather than returning false (github-liaison
            // #2170 MED). A revisit means the chain entered a cycle downstream without reaching `binder`, so
            // `binder` is not referenced → false. Mirrors `value_ref_cycle`'s bounded walk. FxHashSet (not
            // std SipHash) — this is a hot subtree-walk path and the crate uses fxhash for internal keys.
            let mut seen = crate::fxhash::FxHashSet::default();
            loop {
                if target == binder {
                    break true;
                }
                if !seen.insert(target) {
                    break false;
                }
                match resolved_of(db, target) {
                    Resolved::Ref { value: next } => target = next,
                    _ => break false,
                }
            }
        }
        _ => false,
    }
}

fn subtree_references_binder(db: &mut Db, node: StructId, binder: StructId) -> bool {
    // Existence check: SHORT-CIRCUIT on the first hit (unlike `count_param_refs`, which counts the whole
    // tree). Reuses `node_refs_binder` — the same ref-chain predicate `count_param_refs` uses — so the two
    // stay single-source-of-truth AND this keeps its early-exit (github-liaison/Copilot #2128 review: the
    // earlier `count_param_refs(...) > 0` dedup lost the short-circuit, walking the entire subtree).
    if node_refs_binder(db, node, binder) {
        return true;
    }
    match db.ast.get(node).clone() {
        Struct::List(children) => children
            .iter()
            .any(|&c| subtree_references_binder(db, c, binder)),
        Struct::Atom(_) => false,
    }
}

fn performs_discharged_op_other_than(
    db: &mut Db,
    node: StructId,
    ctx: &HandlerCtx,
    own: (u32, u32),
) -> bool {
    if let Resolved::Apply { head, .. } = resolved_of(db, node)
        && let Some(id) = is_perform(db, head, ctx)
        && id != own
    {
        return true;
    }
    match db.ast.get(node).clone() {
        Struct::List(children) => children
            .iter()
            .any(|&c| performs_discharged_op_other_than(db, c, ctx, own)),
        Struct::Atom(_) => false,
    }
}

/// DEPTH-3+ GUARD for the pre-spec-lift (rn3 regression, breaker 2026-08-05). `val` is an inner-op arm's
/// resume value that performs a DIFFERENT discharged op (established by `performs_discharged_op_other_than`).
/// Return true iff the op `val` performs ITSELF has an arm whose resume value performs YET ANOTHER outer op
/// — a depth-3+ chain (`C.hop`→arm resumes `(B.step)`→B's arm resumes `(A.tick)`). The single-step lift
/// rewrites `C.hop`→`(B.step)` but does NOT chase `B.step`'s own arm-hidden `A.tick`, so folding it drops the
/// deepest advance (SILENT wrong value). Detecting this makes `lift_inner_op_arm_outer_perform` decline the
/// deeper chain (leave it un-lifted → clean decline) rather than mis-fold; a correct recursive lift is a
/// later increment. Only inspects a 4-part `(resume v next)` arm (the shape the lift itself handles); an arm
/// of a different shape is conservatively treated as "may perform outer" (decline — safe).
fn resume_val_op_arm_also_performs_outer(
    db: &mut Db,
    val: StructId,
    ctx: &HandlerCtx,
    own: (u32, u32),
) -> bool {
    // Find the discharged op `val` performs (the leftmost such op — its arm is what the lift would expose).
    fn find_performed_op(
        db: &mut Db,
        node: StructId,
        ctx: &HandlerCtx,
        own: (u32, u32),
    ) -> Option<(u32, u32)> {
        if let Resolved::Apply { head, .. } = resolved_of(db, node)
            && let Some(id) = is_perform(db, head, ctx)
            && id != own
        {
            return Some(id);
        }
        match db.ast.get(node).clone() {
            Struct::List(children) => children
                .iter()
                .find_map(|&c| find_performed_op(db, c, ctx, own)),
            Struct::Atom(_) => None,
        }
    }
    let Some(op_id) = find_performed_op(db, val, ctx, own) else {
        return false;
    };
    let Some(inner_arm) = ctx.arms.get(&op_id).cloned() else {
        return false;
    };
    // The op `val` performs (op_id) has an arm in THIS ctx. If that arm's resume value performs YET ANOTHER
    // effect op (ANY effect, not just one THIS ctx discharges — a depth-3 chain's third handler is NOT in
    // this 2-slot merge, so a ctx-scoped `is_perform` check would MISS it — use `effect_op_of`), other than
    // op_id itself, this is a deeper chain the single lift can't flatten → guard fires (decline).
    fn resume_reaches_another_effect_op(db: &mut Db, node: StructId, own_op: (u32, u32)) -> bool {
        if let Resolved::Apply { head, .. } = resolved_of(db, node)
            && let Some((d, i)) = crate::eval::effect_op_of(db, head)
            && (d.0, i) != own_op
        {
            return true;
        }
        match db.ast.get(node).clone() {
            Struct::List(children) => children
                .iter()
                .any(|&c| resume_reaches_another_effect_op(db, c, own_op)),
            Struct::Atom(_) => false,
        }
    }
    match tail_resume(db, inner_arm.body) {
        Some((inner_val, _)) => resume_reaches_another_effect_op(db, inner_val, op_id),
        // The deeper op's arm is NOT a bare `(resume v next)` — a shape this analysis cannot inspect (a
        // wrapped `do`/`match` body, or an abortive arm). Per the doc contract, treat an un-analyzable arm
        // CONSERVATIVELY as "may perform a further outer op" → true, so `lift_inner_op_arm_outer_perform`
        // DECLINES the chain (under the observer gate) rather than lifting a shape whose depth it can't verify
        // (github-liaison #2179 review: the old `None => false` said "safe to lift", the OPPOSITE of the
        // documented conservative-decline, letting a wrapped/abortive depth-3+ intermediate arm slip the lift).
        None => true,
    }
}

fn lift_inner_op_arm_outer_perform(
    db: &mut Db,
    node: StructId,
    ctx: &HandlerCtx,
    caller_observes_outstate: bool,
    state_names: &[String],
) -> StructId {
    // Is this node an inner-op call whose arm resume-value performs an outer op with trivial inner-state?
    if let Resolved::Apply { head, args } = resolved_of(db, node)
        && let Some((decl, idx)) = crate::eval::effect_op_of(db, head)
        && let Some(arm) = ctx.arms.get(&(decl.0, idx)).cloned()
        && arm.cont.is_none()
        && let Some((val, next)) = tail_resume(db, arm.body)
        // trivial inner-state: next-state is exactly the arm's state binder (no advance to preserve).
        && db.ast.as_name(next).is_some()
        && db.ast.as_name(next) == db.ast.as_name(arm.state)
        // the resume value performs a DIFFERENT discharged op (an outer merged slot).
        && performs_discharged_op_other_than(db, val, ctx, (decl.0, idx))
        // NOTE: `val` MAY read the inner arm's STATE binder — `(step (u) t (resume (A.tick (+ t b)) t))`,
        // the outer perform arg reads the inner state `t`. The substitution below RE-BINDS `arm.state` to the
        // inner slot's threaded state PARAM (`state_names[k]`), so the lifted value reads the spec's inner-slot
        // state rather than an ORPHANED `t` (its inner-handler binder is gone once lifted onto the body spine
        // — the #2077 orphan this used to decline to avoid). The `next == arm.state` guard above ensures the
        // inner op does not ADVANCE its state, so the incoming slot param is the right value to read. (The
        // former unconditional decline of a state-reading `val` was the pre-spec-lift floor; re-binding to the
        // slot param is the "later increment" that comment anticipated — nestop/xhsRec.)
        // …AND the resume value's own performed op does NOT ITSELF have an arm that performs a further outer
        // op — i.e. this is a DEPTH-2 chain, not depth-3+. rn3 (breaker): `loop` performs `C.hop`; C's arm
        // resumes `(B.step)`; B's arm resumes `(A.tick)`. Lifting `(C.hop)`→`(B.step)` in ONE step leaves
        // `(B.step)` whose OWN arm still hides the `A.tick` outer perform — the single lift does not chase
        // the second level, so the merged fold specializes against B alone and DROPS A's advance → SILENT 20
        // (a regression: this depth-3 shape was a safe decline under the #2077 floor). A correct depth-3 fold
        // must lift RECURSIVELY (a later increment); until then, DECLINE the deeper chain (leave it un-lifted
        // → `specialize_recursive` declines cleanly) rather than fold a wrong value. Depth-2 (val's op's arm
        // does NOT perform-outer, e.g. B.step's arm `(resume (A.tick) t)` where A.tick's arm is a plain
        // state step) is UNAFFECTED — it lifts and folds → 21.
        //
        // OBSERVER-GATED (rn3/rx4 vs rx6, breaker 2026-08-05). The depth-3+ decline above OVER-DECLINED the
        // NO-OBSERVER chain: `#2179` applied it unconditionally, so rx6 (bare `(loop 2)`, no post-recursion
        // observer of the out-state) regressed from fold-21 to a decline. The silent-20 miscompile the decline
        // exists to prevent ONLY arises under an OBSERVING caller (the accum-redirect path #2136 added, keyed
        // by `force_multivalue` = `caller_observes_outstate`): there the single-step lift drops the deepest
        // advance. WITHOUT an observer the deep chain still folds correctly (rx6 → 21 — the between-iteration
        // advance carries, the redirect never engages), so the lift must fire there as before. Gate the
        // depth-3+ decline on `caller_observes_outstate`: decline the deeper chain (rn3/rx4) only when observed;
        // let rx6 lift+fold when unobserved. (A correct recursive lift folding →21 at all depths regardless of
        // observation is a later increment.)
        && !(caller_observes_outstate
            && resume_val_op_arm_also_performs_outer(db, val, ctx, (decl.0, idx)))
    {
        // β-reduce the arm's resume value with params↦args (the op's args) so a param-referencing outer
        // perform arg resolves. (Unit-op arms bind nothing; a mismatch leaves it un-substituted, still sound
        // since the arm reached here means the shape matched.) Then deep-fresh so refs re-resolve in the body.
        let mut subst: HashMap<StructId, StructId> = HashMap::default();
        if arm.params.len() == args.len() {
            for (&p, &a) in arm.params.iter().zip(args.iter()) {
                if !is_unit_param(db, p) {
                    subst.insert(p, a);
                }
            }
        }
        // RE-BIND the inner arm's state binder to its slot's threaded state PARAM. A resume value reading the
        // inner state (`(A.tick (+ t b))`) would otherwise orphan `t` when lifted onto the outer body; mapping
        // `arm.state` to the inner slot's `state_names[k]` ref makes it read the spec's inner-slot state param
        // instead. The inner op's decl always owns a slot in the merged ctx, so the position is total.
        if subtree_references_binder(db, val, arm.state)
            && let Some(k) = ctx.slots.iter().position(|s| s.decl == decl.0)
        {
            let state_ref = db.push_name(&state_names[k]);
            subst.insert(arm.state, state_ref);
        }
        let reduced = if subst.is_empty() {
            val
        } else {
            crate::eval::beta_reduce(db, val, &subst)
        };
        return deep_fresh_copy(db, reduced);
    }
    // Recurse structurally, rebuilding with lifted children.
    match db.ast.get(node).clone() {
        Struct::List(children) => {
            let lifted: Vec<StructId> = children
                .iter()
                .map(|&c| {
                    lift_inner_op_arm_outer_perform(
                        db,
                        c,
                        ctx,
                        caller_observes_outstate,
                        state_names,
                    )
                })
                .collect();
            if lifted == children {
                node
            } else {
                db.push_list(lifted)
            }
        }
        Struct::Atom(_) => node,
    }
}

/// [tpwJ, breaker 2026-08-16] Rewrite every TAIL `(resume v s)` in an arm body to a `(tuple v s)`, leaving
/// the surrounding `do`/`let`/`match`/`if` structure INTACT — the COLLAPSED alternative to
/// [`peel_resume_from_arm_body`], which DISTRIBUTES the resume into two separate value/next-state expressions.
/// The distribute path reuses the original branch nodes across two separate matches/ifs; when a `let` binder
/// is referenced in BOTH the resume value AND its next-state, the two copies land in different emit scopes and
/// a multi-use binding is kept in one but copy-propagated away in the other → cross-scope "no local slot".
/// Collapsing keeps the binder bound ONCE in its own arm (its match-pattern binders `col`/`k` stay in scope)
/// and yields a single `(value, state)` tuple; the caller binds it once and PROJECTS `(. t 0)`/`(. t 1)`, so
/// the single kept slot is shared. Returns `None` on any shape the resume-peel does not model (same coverage
/// as `peel_resume_from_arm_body`), so the caller falls back to the distribute path.
/// [xhs1] Rewrite each FOREIGN-perform application in `node` — `(E.op arg…)` where `E.op` is an effect op
/// NOT discharged by `ctx` — so every argument is a FROZEN `#st`-prefixed name (force-kept, materialized at
/// THIS inner fold against the incoming state) rather than a live state-dependent subexpression: `(E.op arg)`
/// → `(let ((#stfa{id} arg)) (E.op #stfa{id}))`. The OUTER handler that later folds the embedded perform then
/// reads the frozen slot instead of RE-THREADING the arg against its own pass (which re-derives the incoming
/// state wrongly — the xhs1 residual). A trivial arg (a bare name / literal) is left as-is (nothing to freeze,
/// byte-identical). Recurses bottom-up.
fn freeze_foreign_perform_args(db: &mut Db, node: StructId, ctx: &HandlerCtx) -> StructId {
    // Recurse bottom-up, rebuilding ONLY if a child changed (preserve node sharing/resolution otherwise).
    let node = match db.ast.get(node).clone() {
        Struct::Atom(_) => return node,
        Struct::List(children) => {
            let rebuilt: Vec<StructId> = children
                .iter()
                .map(|&c| freeze_foreign_perform_args(db, c, ctx))
                .collect();
            if rebuilt == children {
                node
            } else {
                db.push_list(rebuilt)
            }
        }
    };
    if let Resolved::Apply { head, args } = resolved_of(db, node)
        && crate::eval::effect_op_of(db, head).is_some()
        && is_perform(db, head, ctx).is_none()
        && !args.is_empty()
        && let Struct::List(orig) = db.ast.get(node).clone()
    {
        // orig = [head-expr, arg0, …]. Freeze each STATE-DEPENDENT arg — a computed list OR a bare name that
        // resolves to a `let`-binding (`Ref`, e.g. the shared `c2`) — to a fresh `#fa`-name.
        // DRAIN-LEVEL FREEZE (correct-fold, co-designed with v-inference — supersedes the arm-local let). An
        // arm-local `(let ((#stfa arg)) (E.op #stfa))` buries the freeze binder INSIDE the combined; when the
        // OUTER handler folds the embedded foreign perform and threads `(+ acc #stfa)` FORWARD, it splices that
        // ref OUT of the arm-local binder scope → CDZ0101 unbound (xhs1 note, decline-by-accident). Instead push
        // each frozen arg to `ctx.pending` as a REAL drain-level bind — materialized by `drain_and_wrap` OUTSIDE
        // the arm, where the outer handler reads it in scope. Bind the drain-SAFE INIT, not the arm-local ref:
        // for a `let`-ref arg (the shared `c2`) that is its resolved `Ref { value }` (the binding's init, already
        // `col`→`#st`/`#seed`-substituted in the combined); for a computed-list arg the expr itself. The `#fa`
        // prefix is DISTINCT and NOT `#st`-matchable (so an arm-local binder can never again accidentally pass
        // `combined_hoistable_to_drain`); `#fa` is whitelisted there and re-resolved by the `#st`/`#fa` forget
        // filter (its init carries `#seed`/`#st` refs). Speculative: the caller rolls back these pending pushes
        // if the combined turns out non-hoistable (the distribute fallback does not use the frozen combined).
        let mut new_call: Vec<StructId> = vec![orig[0]];
        let mut any_frozen = false;
        for (i, &a) in orig.iter().enumerate().skip(1) {
            let init = match resolved_of(db, a) {
                Resolved::Ref { value } => Some(value),
                // A COMPUTED perform-arg (a compound of the shared binder, `(+ c2 1)` — xhsE) freezes the
                // EXPRESSION rather than a single `let`-ref's init. Freezing it RAW keeps its arm-local `let`
                // refs (`c2`), which then dangle at drain level (materialized OUTSIDE the arm) → CDZ0101. The
                // bare-ref case above is drain-safe for free because `Ref { value }` IS the binding's init
                // (already `col`→`#st`/`#seed`-substituted); do the same for the compound by INLINING each
                // arm-local `let`-ref inside it to that same drain-safe init, so the frozen expr references
                // only drain-level names (`#st`/`#seed`), op params, and enclosing params.
                _ if matches!(db.ast.get(a), Struct::List(_)) => {
                    Some(inline_arm_local_let_refs(db, a))
                }
                _ => None,
            };
            if let Some(init) = init {
                let nm = format!("#fa{}_{i}", node.0);
                ctx.pending.borrow_mut().push((nm.clone(), init));
                new_call.push(db.push_name(&nm));
                any_frozen = true;
            } else {
                new_call.push(a);
            }
        }
        if !any_frozen {
            return node;
        }
        return db.push_list(new_call);
    }
    // FREEZE an if-CONDITION / match-SCRUTINEE's state-dependent refs (xhsG / xhsGmatch). thread_bounded's
    // If-arm AND Match-arm merge the per-branch out-states into a `(if cond then_out else_out)` / `(match
    // scrut (pat arm-out)…)`-valued OUT-state that RE-USES the selector, threaded FORWARD as the next
    // dispatch's incoming state. A bare arm-local binder ref in the selector (`(> c2 5)`) then RE-CAPTURES the
    // next dispatch's binder there, selecting the wrong branch and mis-applying the performing branch's
    // advance. The perform ARG is frozen to `#fa`; the SELECTOR was not, so freeze its refs too.
    // GATED on a BRANCH/ARM that reaches an EFFECT-OP-WITH-ARGS perform (`reaches_perform_with_args`, the
    // same ctx-independent predicate the collapse candidate uses) — the xhsG/xhsGmatch shape (a mid-arm foreign
    // perform inside a branch/arm) which is what makes the merge forward a state-advancing selector. When NO
    // branch performs such an op, the merge collapses to the incoming state (no forwarded selector, no
    // re-capture) so the freeze is unnecessary — AND freezing then OVER-declines (pr-sync reject: quo1, a
    // shared-let collapse whose nested match/if selectors reference PATTERN-bound state names with no branch
    // perform, went pass→todo when its selectors were frozen to out-of-scope `#fac`). (NOT `subtree_performs`:
    // that is ctx-sensitive — at THIS inner fold the outer op is FOREIGN and it under-counted, missing xhsG.)
    if let Some(tail) = db.ast.as_form(node, "if").map(<[_]>::to_vec)
        && tail.len() == 3
        && (reaches_perform_with_args(db, tail[1]) || reaches_perform_with_args(db, tail[2]))
    {
        let frozen_cond = freeze_selector_refs(db, tail[0], ctx);
        if frozen_cond != tail[0] {
            let if_head = db.push_name("if");
            return db.push_list(vec![if_head, frozen_cond, tail[1], tail[2]]);
        }
    }
    if let Some(tail) = db.ast.as_form(node, "match").map(<[_]>::to_vec)
        && tail.len() >= 2
        && tail[1..]
            .iter()
            .any(|&arm| reaches_perform_with_args(db, arm))
    {
        let frozen_scrut = freeze_selector_refs(db, tail[0], ctx);
        if frozen_scrut != tail[0] {
            let match_head = db.push_name("match");
            let mut children = vec![match_head, frozen_scrut];
            children.extend_from_slice(&tail[1..]);
            return db.push_list(children);
        }
    }
    node
}

/// Replace every bare `let`-binding reference in `node` with that binding's INIT (its resolved `Ref { value }`),
/// leaving drain-safe names (`#`-prefixed: `#st`/`#seed`/`#fa`), op params, and enclosing params untouched (a
/// `Param` does not resolve to `Ref`, so it is never inlined). Used by the foreign-perform-arg freeze to make a
/// COMPUTED arg (`(+ c2 1)`) drain-safe before it is bound at drain level: substituting `c2` by its init
/// (already `col`→`#st`-substituted in the combined) yields an expression with no arm-local refs, exactly the
/// drain-safe form the bare-`Ref` path binds directly.
///
/// The substitution is ONE LEVEL — a matched ref becomes its `value` verbatim, WITHOUT recursing into that
/// value. This mirrors the bare-`Ref` freeze path (which binds `value` as-is because it is already drain-safe)
/// and, critically, avoids nontermination: a state binder's init resolves to a `Ref` whose value transitively
/// re-references the binder (the threaded-state fixpoint), so descending into substituted values stack-overflows
/// (xhsE). Recursion is over the ORIGINAL node's list STRUCTURE only, to reach each ref; it never re-enters a
/// value it just spliced in. Preserves node sharing when nothing changes.
fn inline_arm_local_let_refs(db: &mut Db, node: StructId) -> StructId {
    if let Some(name) = db.ast.as_name(node) {
        if !name.starts_with('#')
            && let Resolved::Ref { value } = resolved_of(db, node)
        {
            return value;
        }
        return node;
    }
    match db.ast.get(node).clone() {
        Struct::List(children) => {
            let rebuilt: Vec<StructId> = children
                .iter()
                .map(|&c| inline_arm_local_let_refs(db, c))
                .collect();
            if rebuilt == children {
                node
            } else {
                db.push_list(rebuilt)
            }
        }
        Struct::Atom(_) => node,
    }
}

/// Freeze each arm-local `let`-ref in an if-CONDITION or match-SCRUTINEE to a drain-bound `#fac` name (xhsG /
/// xhsGmatch): a bare binder ref re-captures the next dispatch's binding when the merged out-state selector
/// threads forward. Binds the ref's drain-safe INIT (`Ref { value }`) — pure refs only (a perform-reaching
/// ref is left as-is). `#fac` is `#fa`-prefixed so it inherits `combined_hoistable_to_drain`'s whitelist and
/// the `#st`/`#fa` forget filter. Non-ref nodes recurse structurally. Callers gate on a performing branch, so
/// this fires only where the out-state merge forwards the selector (avoids over-declining pure-branch matches).
fn freeze_selector_refs(db: &mut Db, node: StructId, ctx: &HandlerCtx) -> StructId {
    if let Resolved::Ref { value } = resolved_of(db, node)
        && !reaches_any_perform(db, value)
    {
        let nm = format!("#fac{}", node.0);
        ctx.pending.borrow_mut().push((nm.clone(), value));
        return db.push_name(&nm);
    }
    match db.ast.get(node).clone() {
        Struct::Atom(_) => node,
        Struct::List(children) => {
            let rebuilt: Vec<StructId> = children
                .iter()
                .map(|&c| freeze_selector_refs(db, c, ctx))
                .collect();
            if rebuilt == children {
                node
            } else {
                db.push_list(rebuilt)
            }
        }
    }
}

fn peel_tuple_value_state(db: &mut Db, arm_body: StructId) -> Option<StructId> {
    // Bare `(resume v s)` → `(tuple v s)`.
    if let Some((v, s)) = tail_resume(db, arm_body) {
        let tup_head = db.push_name("tuple");
        return Some(db.push_list(vec![tup_head, v, s]));
    }
    // `(do stmt… last)` — keep the leading statements, collapse the tail.
    if let Some(items) = db.ast.as_form(arm_body, "do").map(|t| t.to_vec()) {
        let (&last, stmts) = items.split_last()?;
        let collapsed = peel_tuple_value_state(db, last)?;
        let do_head = db.push_name("do");
        let mut children = vec![do_head];
        children.extend_from_slice(stmts);
        children.push(collapsed);
        return Some(db.push_list(children));
    }
    // `(let binds body)` — keep the binding, collapse the body (the binder stays in scope for both tuple slots).
    if let Some(tail) = db.ast.as_form(arm_body, "let").map(<[_]>::to_vec)
        && tail.len() == 2
    {
        let (bindings, body) = (tail[0], tail[1]);
        let collapsed = peel_tuple_value_state(db, body)?;
        let let_head = db.push_name("let");
        return Some(db.push_list(vec![let_head, bindings, collapsed]));
    }
    // A VISIBLE-ctor op-arg match — fold it (consume this dispatch's op arg), then collapse the arm body.
    if matches!(resolved_of(db, arm_body), Resolved::Match { .. })
        && let Some(folded) = crate::eval::fold_ctor_match(db, arm_body)
    {
        return peel_tuple_value_state(db, folded);
    }
    // `(match scrut (pat body)…)` — collapse each arm body, keep the ONE match (pattern binders stay in scope).
    if let Resolved::Match { scrutinee, arms } = resolved_of(db, arm_body) {
        if arms.is_empty() {
            return None;
        }
        let mhead = db.push_name("match");
        let mut children = vec![mhead, scrutinee];
        for (pat, body) in arms {
            let collapsed = peel_tuple_value_state(db, body)?;
            children.push(db.push_list(vec![pat, collapsed]));
        }
        return Some(db.push_list(children));
    }
    // `(if cond then else)` — collapse each branch, keep the ONE if.
    if let Resolved::If { cond, then_, else_ } = resolved_of(db, arm_body) {
        let ct = peel_tuple_value_state(db, then_)?;
        let ce = peel_tuple_value_state(db, else_)?;
        let if_head = db.push_name("if");
        return Some(db.push_list(vec![if_head, cond, ct, ce]));
    }
    None
}

/// [tpwJ] Whether the arm body binds a `let` whose binder is referenced in BOTH a resume VALUE and a resume
/// NEXT-STATE — the shape that makes the distribute-peel produce a cross-scope "no local slot" orphan (the
/// binding is kept in one of the two split scopes and copy-propagated away in the other). Such an arm is
/// served by the COLLAPSED `peel_tuple_value_state` path instead. Detected structurally by binder NAME (the
/// reused branch refs are memoized to a stale init, so a resolution-keyed test misses them).
/// [xhs1] Whether the arm has a `let`-binding whose INIT reaches a foreign perform WITH A NON-EMPTY ARGUMENT
/// LIST — a mid-arm arg-bearing perform bound to a name on the resume spine (`(let ((nv (O.note c2))) (resume
/// …))`, `(O.note 5)`). The DISTRIBUTE peel WRAPS such a let-init around BOTH the value and the next-state,
/// DUPLICATING the foreign perform (breaker xhsC: a constant-arg outer note RE-EXECUTES 3x across 2 dispatches)
/// and — when a binder feeds the perform arg — threading its two copies against different incoming state
/// (xhs1). The COLLAPSE binds the whole arm once (perform runs once, binder computed once) → correct.
/// ARG-BEARING ONLY (pr-sync reject of 07e85af7c): a NULLARY foreign perform let-init — `(let ((x (A.get)))
/// (resume t (+ t x)))`, the as7 fold-strict shape — is EXCLUDED. `thread_bounded`'s let-arm threads a nullary
/// perform's init exactly ONCE (no arg to re-derive), so DISTRIBUTE already folds as7 correctly (strict → 6);
/// widening the collapse to it wrongly heap-collapsed that fold-strict case. The xhs miscompile class is
/// precisely the ARG-bearing perform (an arg the peel re-threads / the perform the peel duplicates), so gate
/// on `reaches_perform_with_args`. Fires even at 1-use-per-slot (which the shared-let detector below excludes).
fn arm_has_let_init_reaching_arg_perform(db: &mut Db, node: StructId) -> bool {
    if let Some(tail) = db.ast.as_form(node, "let").map(<[_]>::to_vec)
        && tail.len() == 2
        && let Struct::List(pairs) = db.ast.get(tail[0]).clone()
        && pairs.iter().any(|&p| {
            matches!(db.ast.get(p).clone(), Struct::List(kv) if kv.len() == 2 && reaches_perform_with_args(db, kv[1]))
        })
    {
        return true;
    }
    if let Struct::List(children) = db.ast.get(node).clone() {
        return children
            .iter()
            .any(|&c| arm_has_let_init_reaching_arg_perform(db, c));
    }
    false
}

/// Whether `node`'s subtree contains an effect-operation APPLICATION WITH A NON-EMPTY argument list — the
/// arg-bearing foreign perform the drain-level freeze targets (`(O.note c2)`, `(O.note 5)`). A NULLARY perform
/// (`(A.get)`) is EXCLUDED: it has no arg to freeze/re-thread and DISTRIBUTE folds it strict (as3/as7). This is
/// the precise xhs collapse trigger — narrower than `reaches_any_perform` (which also reports nullary performs
/// and `resume`), so it does not sweep the fold-strict let-lifted-nullary-perform shape into the collapse.
fn reaches_perform_with_args(db: &mut Db, node: StructId) -> bool {
    fn walk(db: &mut Db, node: StructId, depth: u32) -> bool {
        if depth > 32 {
            return false; // too deep — do not over-widen the collapse (distribute is the safe default)
        }
        if let Resolved::Apply { head, args } = resolved_of(db, node)
            && !args.is_empty()
            && crate::eval::effect_op_of(db, head).is_some()
        {
            return true;
        }
        match db.ast.get(node).clone() {
            Struct::List(children) => children.iter().any(|&c| walk(db, c, depth + 1)),
            Struct::Atom(_) => false,
        }
    }
    walk(db, node, 0)
}

/// A collapse CANDIDATE: the cross-scope multi-use shared-let (tpwJ), a mid-arm-arg-bearing-foreign-perform
/// let-init (xhs1), OR a NULLARY-foreign-perform let-init whose binder is read by the threaded next-state
/// (pyfb3/pyfb1-let) — all corrupted by the distribute peel and correctly served by the tuple collapse.
/// `op` is THIS arm's operation identity and `multi_dispatch_ops` the ops drawn >=2 in the handle body: the
/// nullary-result branch fires ONLY when this arm's op is multi-dispatch (>=2 draws), because at a SINGLE
/// dispatch the distribute path folds the same shape strict with NO heap slot (as7) — collapsing it there
/// would re-land the 07e85af7c heap-collapse regression. `arms` is the handler's op→arm map (its keys are the
/// discharged ops) — used to classify the let-init perform as FOREIGN (an op this handler does not discharge).
fn arm_is_collapse_candidate(
    db: &mut Db,
    body: StructId,
    op: (u32, u32),
    multi_dispatch_ops: &std::collections::HashSet<(u32, u32)>,
    arms: &HashMap<(u32, u32), HandleArm>,
) -> bool {
    arm_has_let_shared_across_resume_slots(db, body)
        || arm_has_let_init_reaching_arg_perform(db, body)
        || (multi_dispatch_ops.contains(&op)
            && arm_has_nullary_foreign_perform_let_read_by_next_state(db, body, arms))
}

/// [pyfb3/pyfb1-let] Whether the arm has a tail-spine `(let ((k <NULLARY FOREIGN perform>)) (resume v s))`
/// whose binder `k` is read by the threaded NEXT-STATE `s`. Under >=2 dispatches the distribute peel wraps the
/// `let` around BOTH resume slots and the next-state's copy threads FORWARD, re-running the perform per
/// dispatch (triangular extra-perform, breaker pyfb3). The collapse binds the whole `let` ONCE (perform runs
/// once, `k` computed once) → correct. NULLARY (no args) distinguishes this from the arg-bearing xhs1 shape
/// (`arm_has_let_init_reaching_arg_perform`); FOREIGN (op not in `arms`) — a discharged-op let-init would need
/// threading, not a bind-once. Detected on the tail spine (follow `do`/`let`/`match`/`if` to the resume).
fn arm_has_nullary_foreign_perform_let_read_by_next_state(
    db: &mut Db,
    body: StructId,
    arms: &HashMap<(u32, u32), HandleArm>,
) -> bool {
    // A tail-spine `(let binds tail)`: check each binding for the nullary-foreign-perform shape whose binder is
    // read by the tail's resume next-state, then recurse into the tail (nested lets).
    if let Some(tail) = db.ast.as_form(body, "let").map(<[_]>::to_vec)
        && tail.len() == 2
        && let Struct::List(pairs) = db.ast.get(tail[0]).clone()
    {
        let tail_body = tail[1];
        let mut next_states: Vec<StructId> = Vec::new();
        arm_resume_next_states(db, tail_body, &mut next_states);
        for p in pairs {
            let Struct::List(kv) = db.ast.get(p).clone() else {
                continue;
            };
            if kv.len() != 2 {
                continue;
            }
            let (binder, init) = (kv[0], kv[1]);
            // init is a DIRECT effect-op application, NULLARY (no args), FOREIGN (not one of this handler's ops).
            let init_is_nullary_foreign_perform = match resolved_of(db, init) {
                Resolved::Apply { head, args } => {
                    args.is_empty()
                        && crate::eval::effect_op_of(db, head)
                            .is_some_and(|(d, i)| !arms.contains_key(&(d.0, i)))
                }
                _ => crate::eval::effect_op_of(db, init)
                    .is_some_and(|(d, i)| !arms.contains_key(&(d.0, i))),
            };
            if init_is_nullary_foreign_perform
                && let Some(name) = db.ast.as_name(binder).map(|n| n.to_string())
                && next_states
                    .iter()
                    .any(|&s| subtree_mentions_name(db, s, &name))
            {
                return true;
            }
        }
        return arm_has_nullary_foreign_perform_let_read_by_next_state(db, tail_body, arms);
    }
    // Follow the other tail-spine forms (do/match/if) to reach a `let`.
    match db.ast.get(body).clone() {
        Struct::List(children) => children
            .iter()
            .any(|&c| arm_has_nullary_foreign_perform_let_read_by_next_state(db, c, arms)),
        Struct::Atom(_) => false,
    }
}

/// The set of THIS handler's discharged ops drawn >= 2 times (statically) in the handle `body` — the
/// multi-dispatch ops. Per-op scoped (an op X drawn once and op Y drawn 3x must NOT both count as multi):
/// counts each `(decl,idx)` in `arms` separately. A recursive-driver single-static-draw is UNREACHABLE
/// (breaker: arm-perform-under-shared-def is front-end scope-rejected, no-home CDZ0401), so a static count is
/// a sound proxy for runtime dispatch count today; the collapse gate stays conservative (fires only at >=2).
fn ops_drawn_ge2(
    db: &mut Db,
    body: StructId,
    arms: &HashMap<(u32, u32), HandleArm>,
) -> std::collections::HashSet<(u32, u32)> {
    let mut out = std::collections::HashSet::default();
    let keys: Vec<(u32, u32)> = arms.keys().copied().collect();
    for op in keys {
        if count_op_performs(db, body, op) >= 2 {
            out.insert(op);
        }
    }
    out
}

/// Count performs of a SPECIFIC op `(decl,idx)` in `node` (stops at lambdas — a nested handler/closure is a
/// separate dispatch scope). Op-scoped sibling of `count_discharged_performs` (which counts ALL ctx ops).
fn count_op_performs(db: &mut Db, node: StructId, op: (u32, u32)) -> u32 {
    if let Resolved::Apply { head, .. } = resolved_of(db, node)
        && crate::eval::effect_op_of(db, head).is_some_and(|(d, i)| (d.0, i) == op)
    {
        return 1;
    }
    if matches!(resolved_of(db, node), Resolved::Lambda { .. }) {
        return 0;
    }
    match db.ast.get(node).clone() {
        Struct::List(children) => children.iter().map(|&c| count_op_performs(db, c, op)).sum(),
        Struct::Atom(_) => 0,
    }
}

fn arm_has_let_shared_across_resume_slots(db: &mut Db, arm_body: StructId) -> bool {
    // Collect let-binder names bound anywhere on the tail spine, then check each is referenced in BOTH the
    // resume value-slots and the resume next-state-slots — AND KEPT (referenced >=2x) in at least one slot.
    // The cross-scope orphan only arises when the DISTRIBUTE peel produces a MULTI-USE binding that
    // `should_keep_binding` KEEPS as a `Core::Let` in one of the two split scopes (a single-use-per-slot
    // binding is copy-propagated in both → no slot, no orphan — e.g. the closure-state `(let ((r (f v)))
    // (resume r (fn (x) (+ x r))))` arm, which the distribute path handles fine and must stay on it). Gating
    // the COLLAPSE on the kept (>=2) case keeps every single-use-per-slot arm on the proven distribute path.
    let mut binder_names: Vec<String> = Vec::new();
    collect_tail_let_binder_names(db, arm_body, &mut binder_names);
    binder_names.iter().any(|nm| {
        let mut value_occ = 0u32;
        let mut state_occ = 0u32;
        check_name_in_resume_slots(db, arm_body, nm, &mut value_occ, &mut state_occ);
        value_occ > 0 && state_occ > 0 && (value_occ >= 2 || state_occ >= 2)
    })
}

fn collect_tail_let_binder_names(db: &Db, node: StructId, out: &mut Vec<String>) {
    if let Some(tail) = db.ast.as_form(node, "let")
        && tail.len() == 2
        && let Struct::List(pairs) = db.ast.get(tail[0]).clone()
    {
        for p in pairs {
            if let Struct::List(kv) = db.ast.get(p).clone()
                && kv.len() == 2
                && let Some(nm) = db.ast.as_name(kv[0])
            {
                out.push(nm.to_string());
            }
        }
    }
    if let Struct::List(children) = db.ast.get(node).clone() {
        for c in children {
            collect_tail_let_binder_names(db, c, out);
        }
    }
}

fn check_name_in_resume_slots(
    db: &mut Db,
    node: StructId,
    name: &str,
    value_occ: &mut u32,
    state_occ: &mut u32,
) {
    if let Resolved::Resume { value, next_state } = resolved_of(db, node) {
        *value_occ += count_name_occ(db, value, name);
        *state_occ += count_name_occ(db, next_state, name);
    }
    if let Struct::List(children) = db.ast.get(node).clone() {
        for c in children {
            check_name_in_resume_slots(db, c, name, value_occ, state_occ);
        }
    }
}

/// [tpwJ Option A] Whether the collapsed `combined` tuple is HOISTABLE to the drain level — it references no
/// LIFTED `#`-name bound NEAR the perform site. `drain_and_wrap` materializes the combined OUTSIDE the arm, so
/// a `#cv` (op-arg / performing-condition lift) or a `#big…`/freshen-walk (body-`let` lift) reference would
/// resolve out of scope there → CDZ0101. `#st` (per-dispatch state bind) and `#seed` (handle-seed lift) ARE
/// bound at the drain level, so a threaded `(. #st… 1)` incoming state stays hoistable. Structural: a lifted
/// name's resolution is unreliable pre-materialization, but its prefix is an exact, stable witness.
fn combined_hoistable_to_drain(db: &Db, node: StructId) -> bool {
    match db.ast.get(node) {
        Struct::Atom(_) => match db.ast.as_name(node) {
            Some(n) if n.starts_with('#') => {
                n.starts_with("#st") || n.starts_with("#seed") || n.starts_with("#fa")
            }
            _ => true,
        },
        Struct::List(children) => children
            .clone()
            .iter()
            .all(|&c| combined_hoistable_to_drain(db, c)),
    }
}

/// Count occurrences of a bare NAME atom `name` in `node`'s subtree (structural — ignores resolution).
fn count_name_occ(db: &Db, node: StructId, name: &str) -> u32 {
    match db.ast.get(node) {
        Struct::Atom(_) => u32::from(db.ast.as_name(node) == Some(name)),
        Struct::List(children) => children
            .clone()
            .iter()
            .map(|&c| count_name_occ(db, c, name))
            .sum(),
    }
}

/// `None` if the arm body is not one of these (the honest "not yet reducible" decline).
fn peel_resume_from_arm_body(db: &mut Db, arm_body: StructId) -> Option<(StructId, StructId)> {
    // Bare `(resume v s)`.
    if let Some(vs) = tail_resume(db, arm_body) {
        return Some(vs);
    }
    // `(do stmt… (resume v s))` — peel the trailing resume, keeping the leading statements around BOTH the
    // value AND the next-state. A leading `(def d e)` binds `d` LOCAL to the `do`, and `d` may be referenced
    // by the VALUE, the NEXT-STATE, or BOTH — the accumulator arm `(do (def s2 (List.push s v)) (resume
    // (List.len s2) s2))` uses `s2` in both. Wrapping only the value (as this arm did before) returned the
    // next-state `s2` BARE → orphaned → spurious CDZ0101 "unbound s2" (the do-def-shared-across-both-resume-
    // slots false-reject, corpus-bugfix/breaker 2026-07-25 — the multi-use residue of the #21 do→let work;
    // the `let`-peel just below already wraps both, which is why the let-twin worked). Wrap EACH in its own
    // copy of the leading statements (binders copied so the two copies bind independently in the single-
    // parent arena). Sound: the leading stmts are pure do-local bindings on the arm's tail spine (an
    // effectful stmt would be an earlier hole the thread path handles, not reached here), so materializing
    // them around both the value and the next-state is the same evaluation, only widening each binder's
    // visibility. The value/next-state COPY is what breaks the shared-node aliasing (a `resume(d, d)` peels
    // to the same node twice — distinct wrappers must not share it). Mirrors the `let`/`match` peels below.
    if let Some(items) = db.ast.as_form(arm_body, "do").map(|t| t.to_vec()) {
        let (&last, stmts) = items.split_last()?;
        let (v, s) = peel_resume_from_arm_body(db, last)?;
        let stmts = stmts.to_vec();
        // A leading stmt that reaches a PERFORM (a mid-arm FOREIGN op — an outer handler's or a host op —
        // that this fold does not discharge) is EFFECTFUL and must run EXACTLY ONCE per dispatch. It belongs
        // around the VALUE only (the perform's result, sequenced into the continuation the resume returns
        // into); wrapping it around the NEXT-STATE too would RE-RUN the effect, and since the next-state
        // threads forward the duplicate re-accumulates every prior dispatch → the foreign perform fires
        // TRIANGULARLY (breaker pyfb1: N draws → N(N+1)/2 performs, correct N). A PURE stmt (a `(def d e)`
        // with a pure init, e.g. the accumulator `(def s2 (List.push s v))`) is value-idempotent, so it stays
        // wrapped around BOTH — a binder it introduces may be referenced by the value OR the next-state.
        let stmt_reaches_perform: Vec<bool> = stmts
            .iter()
            .map(|&st| reaches_any_perform(db, st))
            .collect();
        let any_effectful = stmt_reaches_perform.iter().any(|&b| b);
        if any_effectful {
            // SAFE FLOOR: an effectful `(def d <perform>)` whose binder `d` is read by the next-state cannot
            // be dropped from the next-state (orphans `d`) nor duplicated (re-runs the effect) — that is the
            // bind-once-and-share shape (needs the `#st`/freeze path). Decline (reject-not-miscompile) rather
            // than mis-fold. A bare effectful stmt (pyfb1's `(B.beat)`, no binder) is unaffected.
            for (i, &st) in stmts.iter().enumerate() {
                if stmt_reaches_perform[i]
                    && let Some(dtail) = db.ast.as_form(st, "def").map(<[_]>::to_vec)
                    && let Some(&binder) = dtail.first()
                    && let Some(name) = db.ast.as_name(binder).map(|n| n.to_string())
                    && subtree_mentions_name(db, s, &name)
                {
                    return None;
                }
            }
        }
        let wrap = |db: &mut Db, inner: StructId, keep: &[bool]| -> StructId {
            let do_head = db.push_name("do");
            let mut children = vec![do_head];
            for (i, &st) in stmts.iter().enumerate() {
                if keep[i] {
                    children.push(copy_pure(db, st));
                }
            }
            children.push(inner);
            if children.len() == 2 {
                // No leading stmts kept — drop the degenerate `(do inner)` wrapper, return `inner` bare.
                return children[1];
            }
            db.push_list(children)
        };
        let all_true: Vec<bool> = stmts.iter().map(|_| true).collect();
        // VALUE: all leading stmts (effects run once here). NEXT-STATE: pure stmts only (effectful ones
        // already ran in the value position — do NOT re-run them per threaded dispatch).
        let vw = wrap(db, v, &all_true);
        let pure_only: Vec<bool> = stmt_reaches_perform.iter().map(|&b| !b).collect();
        let sw = wrap(db, s, &pure_only);
        return Some((vw, sw));
    }
    // `(let ((x e)…) (resume v s))` — a resume in the TAIL of a `let` body. The resume IS the tail (the
    // let's value is its body's value), so peel it and keep the `let` around BOTH the value and the
    // next-state: the binders `x…` may be referenced by `v` OR `s` (the DES `sleep` arm
    // `(let ((wake (at s d))) (resume unit wake))` — the next-state `wake` is a let-binding), so neither
    // can be hoisted out of the `let` — each keeps its own copy of the `let` wrapper. The binder names are
    // copied so the two copies bind independently (single-parent arena). Sound: the let-inits run once
    // before the resume, and re-materializing them around the value + next-state is the same evaluation
    // (they are pure bindings on the arm's tail spine — an effectful init would be an earlier hole the
    // thread path handles, not reached here). This is the `let`-wrapped analogue of the `do` peel above.
    if let Some(tail) = db.ast.as_form(arm_body, "let").map(<[_]>::to_vec)
        && tail.len() == 2
    {
        let (bindings, body) = (tail[0], tail[1]);
        let (v, s) = peel_resume_from_arm_body(db, body)?;
        let wrap = |db: &mut Db, inner: StructId| -> StructId {
            let let_head = db.push_name("let");
            let binds_copy = copy_pure(db, bindings);
            db.push_list(vec![let_head, binds_copy, inner])
        };
        let vw = wrap(db, v);
        let sw = wrap(db, s);
        return Some((vw, sw));
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
    // A match whose scrutinee is a VISIBLE CONSTRUCTOR — the op-arg destructure `(match c ((Cmd.Go k) …))`
    // where `c` was β-substituted to the perform's arg `(Cmd.Go 7)`. This match is CONSUMED at THIS dispatch
    // (the op arg is not threaded state), so FOLD it: select the matching arm and substitute its payload
    // binder with the ctor's payload (`fold_ctor_match`), then recurse to peel the arm's own body. Without
    // this, the match-peel below rebuilds the NEXT-STATE as `(match c ((Cmd.Go k) <inner-next-state>))`,
    // RE-WRAPPING the op-arg match — so the op-arg payload `k` is threaded through STATE and a LATER dispatch
    // conflates its own `k` with the state-threaded one (breaker #13 cmmin5: `(+ (M.step (Cmd.Go 15)) (M.step
    // (Cmd.Go 7)))` computed 45 not 37 — dispatch-2's `k`=7 read dispatch-1's k1=15). Folding the op-arg match
    // consumes it here so only the INNER (state-scrutinee) match threads. A match over the STATE binder (or any
    // non-visible-ctor scrutinee) is untouched — it falls to the general match-peel below and threads correctly.
    if matches!(resolved_of(db, arm_body), Resolved::Match { .. })
        && let Some(folded) = crate::eval::fold_ctor_match(db, arm_body)
    {
        return peel_resume_from_arm_body(db, folded);
    }
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
    // `(if cond (resume v0 s0) (resume v1 s1))` — the arm RESUMES PER BRANCH, selecting the value/next-state
    // by a condition over the op arg / handler state (`get-ty(nid) s => (if (= nid 0) (resume (Some t) s)
    // (resume (None) s))`). This is the `if` analogue of the `match` peel above: peel each branch to its
    // `(value, next_state)`, then rebuild TWO `if`s over the SAME condition — the VALUE `(if cond v0 v1)`
    // (the perform's result) and the NEXT-STATE `(if cond s0 s1)` (threaded forward). Keeping both `if`-
    // wrapped is load-bearing for the same reason as the `match` peel: a branch's next-state may differ per
    // branch, so it must stay under its own condition. The condition is re-used in both, sound on the same
    // invariant the `match` scrutinee reuse relies on (it is over the pure op arg / state, so evaluating it
    // in both `if`s duplicates no effect); each branch must itself peel to a resume.
    if let Resolved::If { cond, then_, else_ } = resolved_of(db, arm_body) {
        let (vt, st) = peel_resume_from_arm_body(db, then_)?;
        let (ve, se) = peel_resume_from_arm_body(db, else_)?;
        let vhead = db.push_name("if");
        let shead = db.push_name("if");
        let cond_v = copy_pure(db, cond);
        let cond_s = copy_pure(db, cond);
        let value = db.push_list(vec![vhead, cond_v, vt, ve]);
        let state = db.push_list(vec![shead, cond_s, st, se]);
        return Some((value, state));
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
    // A `(do e0 … en)` SEQUENCE runs each item unconditionally, in order — a strict spine whose value is the
    // last item. It is a pure one-hole context iff exactly one item is the hole and the rest are strongly
    // pure (the `let`-inits-then-body discipline). `do` is a raw AST form (collapses to its last item's
    // `Ref` under `resolved_of`, hiding earlier items), so match it structurally first. `pure_hole_seq`
    // returns `Impure` for a SECOND hole — so a do-sequenced MULTI-perform body (`(do (St.get) (+ 1 (St.
    // get)))`) yields `Impure` here, falling through to the thread / match-shaped-resume-peel path EXACTLY
    // as before (that path, not this one, folds a multi-perform do-body — this arm must not steal it). A
    // do-local `(type …)`/`(effect …)` declaration runs nothing — skip it.
    if let Some(items) = db.ast.as_form(node, "do").map(<[_]>::to_vec) {
        let positions: Vec<StructId> = items
            .into_iter()
            .filter(|&it| !matches!(db.ast.head_name(it), Some("type") | Some("effect")))
            .collect();
        return pure_hole_seq(db, positions.into_iter(), ctx);
    }
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
        Resolved::Annot { expr, .. } | Resolved::ConstBlock { expr } => pure_hole(db, expr, ctx),
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
/// A `do`-aware leading-strict-hole finder, for the E5 non-tail CONTINUATION-FOLD blocks (the escaping-k
/// re-performing reify AND the two-hole general-one-shot refold) ONLY. Identical to [`leading_strict_hole`]
/// except it ALSO treats a `(do e0 … en)` body as a strict spine (each item unconditional, in order; the
/// leading hole is the first performing item). `do` is scoped here rather than added to the global
/// [`leading_strict_hole`] because the THREAD / tail paths also call that function and ALREADY fold
/// `do`-sequenced multi-perform bodies via the match-shaped-resume-peel / threading path — a global
/// `do`-arm STEALS those (regressed `a_state_destructuring_arm_…` and `a_sequenced_memoize_helper_…` when
/// tried globally). The two continuation-fold blocks are the only consumers that need to see through a `do`
/// (the DES `(do (Sim.sleep w) (inst-ns (Sim.now)))` body shape — a deferred-resume-thunk whose continuation
/// re-performs under a `do`), so the `do`-awareness lives here and the global finder stays byte-identical
/// for every other path. Both blocks gate against the thread path first (`!is_tail_resumptive_arm`), so a
/// `do`-wrapped interpose/forward arm is not stolen.
fn do_aware_leading_hole(db: &mut Db, node: StructId, ctx: &HandlerCtx) -> Option<StructId> {
    if let Some(items) = db.ast.as_form(node, "do").map(<[_]>::to_vec) {
        let positions: Vec<StructId> = items
            .into_iter()
            .filter(|&it| !matches!(db.ast.head_name(it), Some("type") | Some("effect")))
            .collect();
        // Reuse the strict-spine sequencer: at most one item is the hole, the rest strongly pure.
        return leading_strict_hole_seq(db, positions.into_iter(), ctx);
    }
    leading_strict_hole(db, node, ctx)
}

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
        Resolved::Annot { expr, .. } | Resolved::ConstBlock { expr } => {
            leading_strict_hole(db, expr, ctx)
        }
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
/// FLATTEN a pure LET-WRAPPED resume body `(let ((x e)…) body)` (body bears a `resume`) into `body` with
/// each pure init `e` spliced in place of its binder refs — returning the let-free body, or `None` if `node`
/// is not such a let / any init reaches a perform (leave an effectful init a let). `arm_binders` are the
/// arm's state + op-params: their uses in `body` are RESOLVE-PINNED (`pin_refs_to_binders`) BEFORE the
/// flatten `beta_reduce`, so beta_reduce SHARES those occurrences (capture-share path) instead of copying
/// them fresh. Without the pin, a bare arm-state sibling `s` in the resume value (`(+ (Bytes.len b) s)`) is
/// copied fresh by copy_structural, then re-resolves against the now-let-less tree where the arm form is no
/// longer its ancestor → dangling `unbound s` (adv-20). The pin keeps its `Ref{arm.state}` so the caller's
/// subsequent `{arm.state → init}` subst substitutes it. The pin runs FIRST, while `body` is still parented
/// under the arm (so `resolved_of` reaches arm.state), before any `push_list` copy. (v-inference Option A.)
fn flatten_pure_let_wrapped_resume(
    db: &mut Db,
    node: StructId,
    arm_binders: &[StructId],
) -> Option<StructId> {
    let tail = db.ast.as_form(node, "let").map(<[_]>::to_vec)?;
    if tail.len() != 2 || count_resumes(db, tail[1]) < 1 {
        return None;
    }
    let (bindings, lbody) = (tail[0], tail[1]);
    let mut subst: HashMap<StructId, StructId> = HashMap::default();
    if let Struct::List(pairs) = db.ast.get(bindings).clone() {
        for pair in pairs {
            if let Struct::List(kv) = db.ast.get(pair).clone()
                && kv.len() == 2
            {
                if reaches_any_perform(db, kv[1]) {
                    return None;
                }
                // adv-20 residual: if the init is ITSELF a nested pure `let` reading the arm state (`(let ((c
                // (+ s 1))) …)`), recursively flatten it FIRST so its inner let-init `s` does not survive the
                // splice unsubstituted (the flatten handles only one `let` level per pass, so a nested one
                // would otherwise reach emit slotless). A non-`let` init is returned unchanged, so the common
                // single-level case is byte-identical to before.
                let flat_init = flatten_nested_pure_let(db, kv[1], arm_binders);
                subst.insert(kv[1], flat_init);
            }
        }
    }
    // Pin arm-state/param uses FIRST (while `lbody` is still parented under the arm) so the flatten copy
    // shares them rather than orphaning a bare sibling `s`.
    pin_refs_to_binders(db, lbody, arm_binders);
    Some(crate::eval::beta_reduce(db, lbody, &subst))
}

/// Recursively flatten a NESTED pure `let` used as an initializer — `(let ((c e)…) body)` → `body` with each
/// pure init `e` spliced for its binder refs, at every depth — returning the let-free node, or `node`
/// unchanged if it is not a `let` (the common case) or any init reaches a perform (leave it intact —
/// duplicating an effectful init would re-perform). Pins the arm-state/param uses in the whole `let` FIRST so
/// the flatten copy SHARES them (keeps `Ref{arm.state}`) rather than orphaning them, exactly as the top-level
/// flatten does for `lbody`; the caller's `{arm.state → init}` subst then substitutes the shared occurrences.
/// (adv-20 nested-let residual.)
fn flatten_nested_pure_let(db: &mut Db, node: StructId, arm_binders: &[StructId]) -> StructId {
    let Some(tail) = db.ast.as_form(node, "let").map(<[_]>::to_vec) else {
        return node;
    };
    if tail.len() != 2 {
        return node;
    }
    let (bindings, body) = (tail[0], tail[1]);
    let mut subst: HashMap<StructId, StructId> = HashMap::default();
    if let Struct::List(pairs) = db.ast.get(bindings).clone() {
        for pair in pairs {
            if let Struct::List(kv) = db.ast.get(pair).clone()
                && kv.len() == 2
            {
                if reaches_any_perform(db, kv[1]) {
                    return node;
                }
                let flat = flatten_nested_pure_let(db, kv[1], arm_binders);
                subst.insert(kv[1], flat);
            }
        }
    }
    pin_refs_to_binders(db, node, arm_binders);
    crate::eval::beta_reduce(db, body, &subst)
}

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
/// Whether the arm body `node` reaches a DISPATCHING nested handle in a TOLL / continuation-context position
/// — a nested handle WHOSE BODY PERFORMS, sitting anywhere OUTSIDE a `resume`'s own argument subtrees. Skips
/// a `resume` node whole: its ANSWER value folds correctly (spliced into the continuation and reduced,
/// pyre6) and its NEXT-STATE is guarded separately (`reaches_nested_handle`, pyre3). A DISPATCHING nested
/// handle in the post-resume toll `(+ (resume …) (handle E 40 … (+ (E.tick) 2)))` is not reduced to its
/// value — its own dispatch leaks into the outer fold and silently miscompiles (breaker pyth1) — so the
/// two-hole block declines when this is true (reject-not-miscompile). GATED TO DISPATCHING: a NON-dispatching
/// nested handle in the toll (pure body, `(handle E 40 … (: 7))` = 7, breaker pyth2) folds CORRECTLY and must
/// NOT be declined; the discriminator is whether the handle's BODY reaches a perform. A deeper dispatching
/// handle nested inside a non-dispatching one is still caught (descent continues past a non-dispatching one).
fn arm_toll_reaches_nested_handle(db: &mut Db, node: StructId) -> bool {
    // A `resume`'s args are handled elsewhere — skip the whole subtree (do not descend).
    if matches!(resolved_of(db, node), Resolved::Resume { .. }) {
        return false;
    }
    // A nested handle whose BODY dispatches (performs) is the miscompiling case — flag it. Extract the body
    // from `Resolved::Handle` (raw) or, post-desugar, the last child of a `(handle-internal seed arms… body)`.
    let handle_body = match resolved_of(db, node) {
        Resolved::Handle { body, .. } => Some(body),
        _ if db.ast.head_name(node) == Some(HANDLE_INTERNAL) => match db.ast.get(node).clone() {
            Struct::List(children) => children.last().copied(),
            Struct::Atom(_) => None,
        },
        _ => None,
    };
    if let Some(body) = handle_body
        && reaches_any_perform(db, body)
    {
        return true;
    }
    match db.ast.get(node).clone() {
        Struct::List(children) => children
            .iter()
            .any(|&c| arm_toll_reaches_nested_handle(db, c)),
        Struct::Atom(_) => false,
    }
}

/// Whether `node` reaches a NESTED HANDLE — a `Resolved::Handle` or the desugared `handle-internal` head —
/// anywhere in its subtree. Guards the two-hole refold's next-state threading: a next-state that is (or
/// contains) a handle expression must not be threaded raw as a recursive-`reduce_handle` seed (breaker pyre3
/// silent miscompile — the handle is threaded unevaluated instead of reduced to its value). A plain value /
/// pure-arithmetic next-state has no nested handle and threads correctly.
fn reaches_nested_handle(db: &mut Db, node: StructId) -> bool {
    if matches!(resolved_of(db, node), Resolved::Handle { .. })
        || db.ast.head_name(node) == Some(HANDLE_INTERNAL)
    {
        return true;
    }
    match db.ast.get(node).clone() {
        Struct::List(children) => children.iter().any(|&c| reaches_nested_handle(db, c)),
        Struct::Atom(_) => false,
    }
}

fn rewrite_resume_to_refolded_context(
    db: &mut Db,
    node: StructId,
    handle_body: StructId,
    perform: StructId,
    arms: &[HandleArm],
) -> Option<StructId> {
    // A `(let ((x e)…) resume-bearing-body)` — a LET-WRAPPED resume (v-cad's PRNG: `roll(k,s) => let s2 =
    // next(s) in resume(s2%k, s2)`). `resolved_of` peels the let SEMANTICALLY and would hand back a `Resume`
    // whose value/next-state reference the let binder (`s2`) DANGLING — the enclosing let is dropped, so the
    // recursive re-seed `reduce_handle(next_state=s2, …)` sees `s2` unbound → decline. Match the let
    // STRUCTURALLY *before* the Resume check and INLINE the bindings (`x := e`) into the body, so the resume's
    // value/next-state become closed expressions the recursive refold can re-seed.
    if let Some(tail) = db.ast.as_form(node, "let").map(<[_]>::to_vec)
        && tail.len() == 2
        && count_resumes(db, tail[1]) >= 1
    {
        return refold_let_by_binder_inline(db, node, handle_body, perform, arms, |db, init| {
            // SOUNDNESS: a let binder may be referenced MORE THAN ONCE in the body (v-cad's PRNG uses
            // `s2` in both the resume value AND the next-state), so inlining DUPLICATES the init. That
            // is sound only if the init is PURE — an effectful init (`(let ((v (Amb.flip))) …)`)
            // duplicated would re-perform. Decline (leave the let for a later increment) if any init
            // reaches a perform. (A perform-bearing init is properly a two-hole SEQUENCE — its own
            // leading hole — not this let-refold's job.)
            if reaches_any_perform(db, init) {
                return None;
            }
            // A `let` reference resolves to `Ref { value: init }` — the INITIALIZER occurrence, not the
            // binder-name occurrence — so `beta_reduce` matches `subst` keyed by the initializer. Map
            // `initializer := initializer` so each body reference to the let binder splices the (pure,
            // closed) initializer expression in place.
            Some(init)
        });
    }
    // `(let ((x <init reaching a resume>)…) body)` — the resume sits in an INITIALIZER, binding `x` to the
    // CONTINUATION RESULT, with the let BODY as that resume's post-continuation context `C[x]` (breaker's
    // pyr3 post-resume let-binder gap: `(let ((r (resume s next))) (if (> r 35) then else))`). The generic
    // recursion below WOULD rewrite the init's resume to its refolded continuation, but the let BODY's
    // references to `x` resolve to the ORIGINAL initializer occurrence (the resume node), so after the
    // rebuild they DANGLE at that bare resume and re-lower as "resume outside a lowered handler arm". Rewrite
    // each resume-bearing init to its refolded continuation — PURE, every effect discharged — and INLINE it
    // into the body's binder references (sound to duplicate a pure value), so no binder ref is left pointing
    // at a bare resume. A sibling init reaching a FOREIGN perform is a two-hole SEQUENCE (its own leading
    // hole), not this refold's job — decline. Guarded to the resume-NOT-in-body case (the branch above owns
    // the resume-in-body shape) so the two are mutually exclusive.
    if let Some(tail) = db.ast.as_form(node, "let").map(<[_]>::to_vec)
        && tail.len() == 2
        && count_resumes(db, tail[1]) == 0
        && count_resumes(db, tail[0]) >= 1
    {
        return refold_let_by_binder_inline(db, node, handle_body, perform, arms, |db, init| {
            if count_resumes(db, init) >= 1 {
                // The resume-bearing init: rewrite it to its refolded (pure) continuation, then map the
                // binder's references to that. Declines cleanly (`None`) if the recursive refold cannot be
                // served.
                rewrite_resume_to_refolded_context(db, init, handle_body, perform, arms)
            } else if reaches_any_perform(db, init) {
                None
            } else {
                // A pure sibling init — inline as-is (`initializer := initializer`, as the branch above).
                Some(init)
            }
        });
    }
    // `(match <scrutinee reaching a resume> (pat body)…)` — the resume is the match SCRUTINEE and an arm
    // BINDER captures the resume RESULT (breaker's pyr7: `(match (resume s next) ((0) …) ((r) (+ (* r 2)
    // s)))`). Same binder-refs dangling class as the let-init branch, one site over: a match-arm binder
    // reference resolves to the SCRUTINEE occurrence (the bare resume node), so the generic recursion below
    // rewrites the scrutinee's resume but leaves the arm-binder refs pointing at the original resume ->
    // re-lower as "resume outside a lowered handler arm". Rewrite the scrutinee's resume to its refolded
    // (PURE -- every effect discharged) continuation, then beta_reduce the whole match with {scrutinee :=
    // refolded} so BOTH the scrutinee position AND every arm-binder reference splice the pure value (sound
    // to duplicate). Guarded: the scrutinee reaches a resume and NO arm body does (an arm-body resume is the
    // peel/tail path's shape, served elsewhere -- keeps this mutually exclusive with peel_resume_from_arm_body).
    if let Resolved::Match {
        scrutinee,
        arms: match_arms,
    } = resolved_of(db, node)
        && count_resumes(db, scrutinee) >= 1
        && match_arms.iter().all(|&(_, b)| count_resumes(db, b) == 0)
    {
        // Rewrite the scrutinee's resume to its refolded (PURE — every effect discharged) continuation, then
        // REBUILD the match with `substitute_nodes` replacing the scrutinee node by identity with the
        // refolded value. `substitute_nodes` rebuilds the arms FRESH (pattern + body copied together, so each
        // arm's pattern↔body binder link is structurally preserved) and the copied binder-ref nodes carry no
        // cached resolution; forget+re-resolve the rebuilt subtree so an arm binder — which resolves through
        // its PATTERN to the scrutinee — RE-BINDS to the new pure scrutinee instead of the original resume
        // node (which the generic recursion would leave it pointing at, re-lowering as "resume outside a
        // lowered handler arm"; a separate pattern/body copy would instead orphan the ref → CDZ0101).
        let new_scrut =
            rewrite_resume_to_refolded_context(db, scrutinee, handle_body, perform, arms)?;
        let mut sub: HashMap<StructId, StructId> = HashMap::default();
        sub.insert(scrutinee, new_scrut);
        let rebuilt = substitute_nodes(db, node, &sub);
        reparent_under_handle_site(db, rebuilt, handle_body);
        crate::resolve::forget_subtree(db, rebuilt);
        crate::resolve::resolve_subtree(db, rebuilt);
        return rewrite_resume_to_refolded_context(db, rebuilt, handle_body, perform, arms);
    }
    if let Resolved::Resume { value, next_state } = resolved_of(db, node) {
        // SILENT-MISCOMPILE GUARD (breaker pyre3). The next-state threads forward as the SEED of the recursive
        // `reduce_handle` below. If it is itself a NESTED HANDLE expression `(resume v (handle E … body))`, it
        // is threaded RAW — not first reduced to its value — so a later dispatch reading the state observes the
        // unevaluated handle in the seed slot and mis-evaluates it: a closed pure handle = 42 in next-state
        // produced 415210 not the correct 47210 (uniform wasm+rust), whereas the referentially-equal pure
        // ARITHMETIC `(* 6 7)` = 42 threads correctly. A pure sub-expression must be replaceable by its value;
        // the fold isn't reducing a next-state-position handle, so it silently miscompiles. DECLINE (a clean
        // todo, matching the let-hoisted sibling `(let ((ns (handle …))) (resume v ns))` which already
        // declines) rather than thread a handle expression as a state seed. Narrow: only a next-state that
        // REACHES a nested handle trips it — a plain value / pure-arithmetic next-state folds unchanged.
        if reaches_nested_handle(db, next_state) {
            return None;
        }
        // Build `C[value]` (the continuation with the hole filled by the resume value), then re-reduce it
        // under the same handler seeded with the resume's next-state — so a further discharged perform in
        // `C` is handled by the recursive fold.
        let filled = splice_context(db, handle_body, perform, value);
        // RE-ANCHOR the spliced continuation under the ORIGINAL handle body's site before the recursive fold
        // (breaker pm-family, false-CDZ0101 fix). `splice_context` rebuilds `C[value]` as a DETACHED tree
        // (`push_list`, parent = None). If `C` references a FREE enclosing binder — the handle BODY reads a
        // caller param / outer `let` (`(+ n (St.price 1) …)` where `n` is `main`'s param) — that leaf is a
        // resolve occurrence that resolves by a scope WALK up `parent_of`. Detached, its walk dead-ends before
        // reaching `(def (main (: n)))` → a spurious CDZ0101 "unbound n" on a VALID program (loud reject). This
        // shape reaches the detaching refold only via ≥2 performs: a single perform leaves `C` in place, parented.
        // The recursive `reduce_handle` below CANNOT fix it — its own `reparent_under_handle_site` reads
        // `parent_of(filled)` = None and returns early (a "top-level handle body, no enclosing scope"), so the
        // free var stays orphaned through every refold level. Anchor `filled` under the SAME parent the
        // original `handle_body` sits under (its live lexical chain) so `C`'s free names resolve exactly as
        // they did before the splice. `handle_body` is the still-parented original; a top-level body (parent
        // None) leaves `filled` as-is (nothing to anchor — the pre-existing behavior). Done HERE (pre-recursion)
        // because the detachment is introduced HERE. Route through `reparent_under_handle_site` (NOT a raw
        // `db.reparent`) so a `handle_body` sitting in a 2-element PAIR body position — a `match` arm `(pattern
        // body)` or a `let` binding `(name init)`, when the handle is distributed into such an arm — rebuilds a
        // fresh `(pattern filled)` pair: resolve's binder helpers (`match_arm_binds`, resolve.rs:1880/1941/2005)
        // require the scope-walk `from` to be EXACTLY the pair's recorded body child, so parenting `filled`
        // directly under the pair (leaving `pb[1]` = the old `handle_body`) would leave a pattern/let binder
        // referenced inside `filled` unresolvable — re-introducing this very false-CDZ0101 class for that
        // sub-case (github-liaison/Copilot #2305 review). `reparent_under_handle_site` handles the pair rebuild
        // AND the plain (non-pair, list/handle-node) parent identically to the raw reparent, so it is strictly
        // safer; a parentless `handle_body` is a no-op (leaves `filled` as-is), matching the prior guard.
        reparent_under_handle_site(db, filled, handle_body);
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

/// Shared scaffolding for the two `(let …)` refold branches of [`rewrite_resume_to_refolded_context`]: match
/// `node` as a two-element `let` form, walk each `(name init)` binding pair, ask `classify_init` for the
/// substitution target of that init (or `None` to decline the whole fold), then `beta_reduce` the body under
/// the collected `initializer := target` map and recurse. A `let` reference resolves to the INITIALIZER
/// occurrence, so `beta_reduce`'s `subst` is keyed by `init` (never the binder name). The two call sites
/// differ ONLY in their guard (resume-in-body vs resume-only-in-an-init) and `classify_init` — the guards are
/// mutually exclusive and stay at the call sites (merging them would newly admit a resume-in-BOTH shape
/// neither branch handles). Returns the refolded body, or `None` if `classify_init` declines any init.
fn refold_let_by_binder_inline(
    db: &mut Db,
    node: StructId,
    handle_body: StructId,
    perform: StructId,
    arms: &[HandleArm],
    mut classify_init: impl FnMut(&mut Db, StructId) -> Option<StructId>,
) -> Option<StructId> {
    let tail = db.ast.as_form(node, "let").map(<[_]>::to_vec)?;
    let (bindings, lbody) = (tail[0], tail[1]);
    let mut subst: HashMap<StructId, StructId> = HashMap::default();
    if let Struct::List(pairs) = db.ast.get(bindings).clone() {
        for pair in pairs {
            if let Struct::List(kv) = db.ast.get(pair).clone()
                && kv.len() == 2
            {
                subst.insert(kv[1], classify_init(db, kv[1])?);
            }
        }
    }
    let inlined = crate::eval::beta_reduce(db, lbody, &subst);
    rewrite_resume_to_refolded_context(db, inlined, handle_body, perform, arms)
}

/// Copy the one-hole context `handle_body` (the pure delimited continuation), replacing the sole hole
/// occurrence `perform` with (a fresh copy of) `filler` — i.e. build `C[filler]`. The hole `perform` is a
/// UNIQUE occurrence in the arena (`pure_hole` verified exactly one discharged perform reaches on a pure
/// spine), so a by-identity match locates it. Everything else is copied structurally so the result is
/// self-contained and re-parents its free names against the splice site.
/// Rebuild `node`, replacing any subtree whose identity is a key of `sub` with the mapped replacement
/// (verbatim — the replacement is spliced as-is, NOT recursed into). A node not in `sub` is copied
/// structurally (children rebuilt), so the result is a fresh tree with the mapped nodes swapped in. The
/// multi-node analogue of [`splice_context`]. Used by the E5 step-2 ctl→resume rewrite to swap each
/// `(k v)` application for its `(resume v state)` node in one pass.
fn substitute_nodes(db: &mut Db, node: StructId, sub: &HashMap<StructId, StructId>) -> StructId {
    if let Some(&repl) = sub.get(&node) {
        return repl;
    }
    match db.ast.get(node).clone() {
        Struct::List(children) => {
            let rebuilt: Vec<StructId> = children
                .iter()
                .map(|&c| substitute_nodes(db, c, sub))
                .collect();
            db.push_list(rebuilt)
        }
        Struct::Atom(_) => copy_pure(db, node),
    }
}

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

/// Whether the arm body CONDITIONALLY resumes — its top-level shape is an `if`/`match` in which SOME branch
/// (transitively, through nested `if`/`match`/`do`/`let` tails) resumes and SOME branch returns a bare
/// NON-resuming value (a per-arm conditional abort, `(if cond ABORT-VALUE (resume …))`). Such an arm is
/// neither cleanly abortive (it HAS a resume) nor uniformly tail-resumptive (a branch aborts), so the E5
/// pure-one-hole / two-hole reify folds mis-handle it: they rewrite only the resuming branch and leave the
/// aborting branch a bare value, mis-splicing the continuation and orphaning a synthesized copy of a
/// seed/param free name → a relocated CDZ0101 at lowering (check passes, emit diverges; corpus-bugfix/breaker
/// 2026-07-28). Both reify blocks DECLINE when this holds, so the shape reports the honest "not yet reducible"
/// todo (the conditional-abort/continuation machinery is a later increment) rather than a mis-fold. Only the
/// TOP-LEVEL conditional shapes are inspected (an `if`/`match` whose branches split resume-vs-abort); a bare
/// resume, a `do`/`let`-tail resume, or a fully-resuming match are NOT flagged (every branch resumes).
fn arm_partially_resumes(db: &mut Db, node: StructId) -> bool {
    // Only meaningful when the arm resumes at all AND cannot be uniformly peeled to a resume — a uniformly
    // peelable arm (`peel_resume_from_arm_body` Some) resumes in every branch, so it is NOT partial.
    if !arm_has_resume(db, node) {
        return false;
    }
    // Descend the top-level conditional/sequencing shape; a branch that neither peels to a resume nor is
    // itself a conditional-with-a-resume is a bare non-resuming value → the arm is partial.
    fn branch_resumes(db: &mut Db, n: StructId) -> bool {
        peel_resume_from_arm_body(db, n).is_some() || arm_has_resume(db, n)
    }
    match resolved_of(db, node) {
        Resolved::If { then_, else_, .. } => {
            // Partial iff the two branches DISAGREE on whether they resume.
            branch_resumes(db, then_) != branch_resumes(db, else_)
                || arm_partially_resumes(db, then_)
                || arm_partially_resumes(db, else_)
        }
        Resolved::Match { arms, .. } => {
            let flags: Vec<bool> = arms.iter().map(|&(_, b)| branch_resumes(db, b)).collect();
            flags.iter().any(|&f| f) && flags.iter().any(|&f| !f)
                || arms.iter().any(|&(_, b)| arm_partially_resumes(db, b))
        }
        _ => false,
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
    orig_param_names: &std::collections::HashSet<String>,
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
    // Pre-seed `seen` with the original param names so a same-named enclosing capture is skipped (the original
    // param shadows it in the spec body — appending it would duplicate the name → CDZ0102).
    seen.extend(orig_param_names.iter().cloned());
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

/// A NAME → solved-type table for every name occurring in the handler arm bodies that resolves to a
/// DETERMINED type in the ORIGINAL (pre-thread) context. The full-fold loop in `specialize_recursive`
/// uses it to supply the type of a main-local `let` capture the validator flags: the flagged occurrence
/// resolves `Poison` in the LIFTED def (out of scope there), so its type can only be read here, from the
/// arm body where the reference is still bound to its enclosing `let`. Names matching an original param
/// are excluded (the spec already binds that name — it can never be a threaded capture). Over-inclusion
/// is harmless: only names the validator actually flags as escaping are ever consulted, and an own binder
/// / global / prelude name is never flagged (it resolves, not `Poison`). Walks arm bodies in the SAME
/// stable `(decl, op-index)` order `captured_enclosing_params` uses, for determinism.
fn collect_local_capture_types(
    db: &mut Db,
    ctx: &HandlerCtx,
    orig_param_names: &std::collections::HashSet<String>,
) -> std::collections::HashMap<String, crate::ty::Ty> {
    let mut out: std::collections::HashMap<String, crate::ty::Ty> =
        std::collections::HashMap::new();
    let mut arm_bodies: Vec<((u32, u32), StructId)> =
        ctx.arms.iter().map(|(&k, a)| (k, a.body)).collect();
    arm_bodies.sort_by_key(|&(k, _)| k);
    fn walk(
        db: &mut Db,
        node: StructId,
        orig_param_names: &std::collections::HashSet<String>,
        out: &mut std::collections::HashMap<String, crate::ty::Ty>,
    ) {
        if let Some(name) = db.ast.as_name(node).map(str::to_string)
            && !name.contains('#')
            && !name.contains('$')
            && !orig_param_names.contains(&name)
            && !out.contains_key(&name)
            && !matches!(resolved_of(db, node), Resolved::Poison(_))
        {
            let ty = crate::infer::type_of(db, node);
            if !ty_has_any(&ty) {
                out.insert(name, ty);
            }
        }
        if let Struct::List(children) = db.ast.get(node).clone() {
            for c in children {
                walk(db, c, orig_param_names, out);
            }
        }
    }
    for (_, body) in arm_bodies {
        walk(db, body, orig_param_names, &mut out);
    }
    out
}

fn count_param_refs(db: &mut Db, node: StructId, binder: StructId) -> u32 {
    // Per-node "is this a reference to `binder`" uses the shared `node_refs_binder` predicate (a `Param`
    // binder-match or a `Ref` chain reaching it) — the same rule `subtree_references_binder` uses, so the
    // two never diverge. This walk COUNTS every reference (no short-circuit — callers need 0/1/many).
    let here = u32::from(node_refs_binder(db, node, binder));
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
    // An `effect_op_of` head is a perform regardless of which handler owns it, so this ANY-perform detector
    // never consults `ctx` (unlike its sibling `body_reaches_foreign_perform`, which needs it to distinguish
    // a FOREIGN op). `ctx` stays in the signature for a uniform call shape with the sibling detector; the
    // ctx-free core is `reaches_any_perform` so callers without a `HandlerCtx` can reuse the same walk.
    let _ = ctx;
    reaches_any_perform(db, node)
}

/// Whether `node` reaches a perform of one of THIS ctx's DISCHARGED ops (in `ctx.arms`), following non-
/// recursive callee bodies (bounded). Narrower than `arg_reaches_any_perform` (which counts foreign performs
/// too) — used by the op-arg let-lift to exclude an arg that performs the handler's own op.
fn subtree_reaches_discharged_op(db: &mut Db, node: StructId, ctx: &HandlerCtx) -> bool {
    fn walk(db: &mut Db, node: StructId, ctx: &HandlerCtx, depth: u32) -> bool {
        // Depth bound 16, DELIBERATELY STRICTER than `reaches_any_perform`'s 32 (github-liaison/Copilot #2120
        // review asked why they differ). The two walks over-report in OPPOSITE directions, so the bound tunes
        // each toward its own safe side: this walk's `true` means "may perform a DISCHARGED op" → the op-arg
        // let-lift DECLINES (does not lift), which is always safe (a missed lift is a clean todo, never a
        // miscompile), so a lower bound = decline-sooner = safe. `reaches_any_perform`'s `true` means "may
        // perform ANY effect" → it gates the DUPLICATION guard (decline if a multi-use arg performs), also
        // decline-on-true, but it must be permissive enough to actually SEE a foreign perform worth lifting,
        // so its bound is higher. A shared const would force one walk off its safe side; the mismatch is
        // intentional (each errs toward decline within its own role). Both bounds far exceed any real arg
        // nesting, so neither triggers in practice.
        if depth > 16 {
            return true; // too deep — assume it may (safe over-report → decline, not mis-lift)
        }
        if let Resolved::Apply { head, args } = resolved_of(db, node) {
            if is_perform(db, head, ctx).is_some() {
                return true;
            }
            if crate::eval::effect_op_of(db, head).is_none()
                && !is_pure_operator_head(db, head)
                && let Some(callee) = crate::eval::lambda_body(db, head)
                    .or_else(|| crate::eval::lambda_body_of_nullary(db, head))
                && !crate::eval::is_recursive(db, callee)
                && walk(db, callee, ctx, depth + 1)
            {
                return true;
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

/// Whether the subtree at `node` transitively reaches ANY perform (this handler's discharged op, a foreign
/// op, or a bare `resume`), following NON-RECURSIVE calls into their bodies (bounded depth). CONSERVATIVE:
/// a recursive/unresolvable call, or a chain deeper than the bound, reports `true`. No `HandlerCtx` needed —
/// an effect-op head performs regardless of the handler in scope.
fn reaches_any_perform(db: &mut Db, node: StructId) -> bool {
    fn walk(db: &mut Db, node: StructId, depth: u32) -> bool {
        if depth > 32 {
            return true; // too deep — assume it may perform (safe over-report)
        }
        // A `let` — descend into its INIT VALUES + body EXPLICITLY (breaker tk3d). A `let`'s raw bindings
        // sublist `((n init)…)` structurally looks like an APPLICATION `((n init))`, so the generic
        // child-walk below would hand the bindings list to the `Apply` arm — whose `args.iter()` are the
        // WRONG operands (it never reaches the `init` value), so a perform in a let-INIT (`(let ((out
        // (Sink.flush))) …)`) is MISSED. That under-report made `mark_caller_observed_outstate` fail to see a
        // let-bound perform observing a cross-fn helper's out-state → the helper was not upgraded to
        // multi-value → its slot advance was dropped, reading the seed (tk3d: len 0 not 3, a 3-backend silent
        // miscompile). Route the `let` (and `do`, same raw-form sublist hazard is absent but the explicit
        // walk is uniform + cheap) through its real init/body positions so a perform anywhere in them is seen.
        if let Some(tail) = db.ast.as_form(node, "let").map(<[_]>::to_vec)
            && tail.len() == 2
            && let Struct::List(pairs) = db.ast.get(tail[0]).clone()
        {
            for pair in pairs {
                if let Struct::List(kv) = db.ast.get(pair).clone()
                    && kv.len() == 2
                    && walk(db, kv[1], depth + 1)
                {
                    return true;
                }
            }
            return walk(db, tail[1], depth + 1);
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

/// Whether `node` performs a FOREIGN, IN-PROGRAM-ROUTED op (an effect op NOT in this handler's arms — an
/// outer handler's effect) DIRECTLY, as a literal `(Outer.op …)` Apply somewhere in the expression tree —
/// WITHOUT following user calls into their bodies. The NARROW twin of `body_reaches_foreign_perform` (which
/// over-reports through recursive/unresolvable heads, sweeping the whole recursive-fold surface). Used to
/// gate the as2/as1 safe-decline: an inner arm's NEXT-STATE `(+ t (A.get))` literally performs the OUTER
/// effect in the threaded-state position (dropped/duplicated → silent wrong). A recursive fold that threads
/// an outer effect does so through a self-call/specialized CALLEE body, never a direct `(A.get)` in the
/// arm's own next-state — so NOT following calls is exactly the discriminator that spares those folds.
///
/// EXCLUDES a HOST-DELEGATED perform (one enclosed by a `(host (E…) …)` router — `perform_host_target`
/// resolves it): a host call sequences through cdz-run's RESPONSE QUEUE, not the state-expression thread
/// that drops/duplicates a handler-routed perform, so a host op in the next-state slot is strict-correct and
/// never miscompiled (breaker as6 `(resume t (+ t (ask.ask)))` under `(host (ask) …)` = 155). Declining it
/// would be an over-decline of a working program — so the gate fires ONLY on an IN-PROGRAM foreign effect
/// (the miscompiling class), matching the actual defect boundary breaker's as-class radius established.
fn next_state_directly_performs_foreign(db: &mut Db, node: StructId, ctx: &HandlerCtx) -> bool {
    if let Resolved::Apply { head, .. } = resolved_of(db, node)
        && let Some((decl, idx)) = crate::eval::effect_op_of(db, head)
        && !ctx.arms.contains_key(&(decl.0, idx))
        && perform_host_target(db, node, head).is_none()
    {
        return true;
    }
    match db.ast.get(node).clone() {
        Struct::List(children) => children
            .iter()
            .any(|&c| next_state_directly_performs_foreign(db, c, ctx)),
        Struct::Atom(_) => false,
    }
}

/// Collect the RAW next-state child of every tail `resume` in a (parented) arm body, descending through
/// `do`/`let`/`match` wrappers WITHOUT lifting them into the child. This differs deliberately from
/// `peel_resume_from_arm_body`, which WRAPS each returned child in the surrounding `let`/`do` for scoping —
/// wrapping folds a shared `let`-init / `do`-statement foreign perform (as7 `(let ((x (A.get))) …)`, the
/// interposing `(do (A.tick) …)`) INTO the next-state, making it look unsound when it is not (that perform
/// runs ONCE on the value spine). The RAW resume child is the exact expression that threads forward as this
/// slot's next state, so testing IT for a direct foreign perform is the precise unsoundness check: a foreign
/// op literally in `(resume v NS)`'s `NS` is dropped/duplicated (as2/as1, and the both-perform `(resume
/// (A.get) (A.get))` github-liaison/Copilot #2289 flagged), whereas a foreign in the VALUE slot (as3) or in a
/// shared `let`-init/`do`-stmt (as7/interpose) is served. Returns `None` if `arm_body` is not tail-resumptive
/// (no resume to inspect — the caller then leaves the fold to its other arms). A `match`-shaped arm
/// contributes every branch's next-state (any branch performing a foreign in the state position is unsound).
fn arm_resume_next_states(db: &mut Db, arm_body: StructId, out: &mut Vec<StructId>) -> Option<()> {
    if let Some((_v, s)) = tail_resume(db, arm_body) {
        out.push(s);
        return Some(());
    }
    if let Some(items) = db.ast.as_form(arm_body, "do").map(|t| t.to_vec()) {
        let last = *items.last()?;
        return arm_resume_next_states(db, last, out);
    }
    if let Some(tail) = db.ast.as_form(arm_body, "let").map(<[_]>::to_vec)
        && tail.len() == 2
    {
        return arm_resume_next_states(db, tail[1], out);
    }
    if let Resolved::Match { arms, .. } = resolved_of(db, arm_body) {
        if arms.is_empty() {
            return None;
        }
        for (_pat, body) in arms {
            arm_resume_next_states(db, body, out)?;
        }
        return Some(());
    }
    None
}

/// Whether `param` is the unit placeholder `()` (a nullary operation's single "parameter", which binds
/// nothing). `()` resolves to `Resolved::Unit`.
fn is_unit_param(db: &mut Db, param: StructId) -> bool {
    matches!(resolved_of(db, param), Resolved::Unit)
}

/// The first EFFECT-OP PERFORM reached inside a match-arm GUARD condition, if any — the detection side of
/// the "guards must be side-effect-free" rule (operator directive, PR #2543; CDZ0407 EffectInGuard, emitted
/// by `infer`'s guarded-arm check). Returns the offending PERFORM node (an `(E.op …)` application whose head
/// resolves to an effect op) so the caller can anchor the diagnostic THERE, not at the whole guard. `None`
/// when the guard cond is pure.
///
/// CONTEXT-FREE, unlike [`subtree_performs`]: a guard must be pure regardless of which handler encloses it
/// (a performing guard is a re-evaluation hazard — the pattern engine may evaluate a guard speculatively or
/// repeatedly, breaker finding #9 — so ANY effect op is forbidden, not only ops a specific handler
/// discharges). So this keys on `eval::effect_op_of` (the op-IDENTITY channel) directly rather than a
/// `HandlerCtx` discharged-op set. A `resume` is not reachable in a guard (guards are not handler arms), so
/// only an effect-op application is detected. Does NOT descend into a LAMBDA body — a closure VALUE built in
/// a guard performs nothing when constructed (its body's effects fire only when applied, an `Apply` node the
/// walk sees on its own); this mirrors `subtree_performs`'s lambda-value treatment so a guard that merely
/// builds a performing closure it never applies is not flagged. Structural walk over the resolved form; the
/// FIRST perform found (pre-order) is returned.
pub(crate) fn effect_op_in_guard_cond(db: &mut Db, cond: StructId) -> Option<StructId> {
    if let Resolved::Apply { head, .. } = resolved_of(db, cond)
        && crate::eval::effect_op_of(db, head).is_some()
    {
        return Some(cond);
    }
    // A lambda VALUE performs nothing when constructed — do not descend its body (same rule as
    // `subtree_performs`); the application that fires its effects is a separate `Apply` node.
    if matches!(resolved_of(db, cond), Resolved::Lambda { .. }) {
        return None;
    }
    match db.ast.get(cond).clone() {
        Struct::List(children) => children
            .iter()
            .find_map(|&c| effect_op_in_guard_cond(db, c)),
        Struct::Atom(_) => None,
    }
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
    #[cfg(test)]
    crate::db::SUBTREE_PERFORMS_UNCACHED_CALLS.with(|c| c.set(c.get() + 1));
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

/// Whether the REDUCED handle body `node` (the `Some` result of [`reduce_handle`], already grafted under the
/// handle site) still carries a discharged-op perform that LEAKED past the fold — a perform lexically inside
/// a LIVE lambda whose lowering ACTUALLY surfaces the misleading `NO_HOME_STANDALONE_DECLINE`. The fold could
/// not route it because the closure ESCAPED its reach (stored in a collection, extracted via `List.at` +
/// `match ((Some f) …)`, applied through the slot; `subtree_performs` treats a lambda VALUE as pure since a
/// closure body performs only when APPLIED, and the fold cannot trace the application back through the slot).
/// That lambda lifts to a STANDALONE function lowered with NO handle frame — losing the perform's lexical
/// ancestry — so its perform reaches `lower`'s standalone arm and would report "performed with no enclosing
/// handler here" even though this handle LEXICALLY encloses it. Called at the LOWERING entry to turn that
/// into the honest `HANDLER_NOT_REDUCIBLE_DECLINE` todo (breaker's diagnostic-quality finding, routed by
/// corpus-bugfix 2026-07-28).
///
/// The LEAF DISCRIMINATOR lowers the candidate perform (`core_of`) and fires ONLY when its core is exactly the
/// `NO_HOME_STANDALONE_DECLINE` poison — so a perform that lowers to something else (a partial application of
/// the enclosing closure declines EARLIER, at the application site, with the more-specific "partial
/// application of a runtime closure" message; a host-delegated perform lowers to a `HostCall`) is left with
/// its own, better outcome. DECLINE-ONLY: it can only replace one honest reject's MESSAGE with another,
/// never admit a program. `arms` supplies the discharged op-identity set; an empty set (a malformed handle)
/// never leaks by this measure (its own reject path handles it).
pub fn reduced_body_leaks_escaped_perform(db: &mut Db, node: StructId, arms: &[HandleArm]) -> bool {
    let discharged: Vec<(u32, u32)> = arms
        .iter()
        .filter_map(|a| crate::eval::effect_op_of(db, a.op).map(|(d, i)| (d.0, i)))
        .collect();
    if discharged.is_empty() {
        return false;
    }
    // YIELD to a more-specific co-occurring decline. When the reduced body PARTIALLY applies a runtime
    // closure (a curried closure called under its arity), lowering reports the precise "partial application
    // of a runtime closure" decline at that application site — a BETTER message than the generic
    // not-yet-reducible this guard would emit. A performing closure that is BOTH escaped AND partially
    // applied (the `Box.C`-wrapped 2-arg closure applied to one arg) should surface THAT message, so don't
    // fire here; the partial-application path owns the shape. (An escaped closure that is FULLY applied — the
    // collection case — has no such sharper decline, so the guard fires and fixes the misleading no-home.)
    if reduced_body_partially_applies_a_closure(db, node) {
        return false;
    }
    leaks_no_home_perform_in_lambda(db, node, &discharged, false)
}

/// Whether the reduced handle body applies a runtime closure UNDER its arity (a curried closure called with
/// fewer args than its arrow-peel count) — the shape `lower` declines as "partial application of a runtime
/// closure". A syntactic scan for an application `(f args…)` whose head resolves to a value of curried arrow
/// type with arity > the arg count. Used to let that more-specific decline win over the escaped-closure
/// no-home remap (see [`reduced_body_leaks_escaped_perform`]).
fn reduced_body_partially_applies_a_closure(db: &mut Db, node: StructId) -> bool {
    if let Resolved::Apply { head, args } = resolved_of(db, node) {
        let mut ty = crate::infer::type_of(db, head);
        let mut arity = 0usize;
        while let crate::ty::Ty::Fn(_, r) = ty {
            arity += 1;
            ty = *r;
        }
        if arity > 0 && args.len() < arity {
            return true;
        }
    }
    match db.ast.get(node).clone() {
        Struct::List(children) => children
            .iter()
            .any(|&c| reduced_body_partially_applies_a_closure(db, c)),
        Struct::Atom(_) => false,
    }
}

/// Recursive worker for [`reduced_body_leaks_escaped_perform`]. `in_lambda` flips true once we descend under
/// a lambda; a discharged-op perform found there fires ONLY if its own `core_of` is the misleading
/// `NO_HOME_STANDALONE_DECLINE` (the actual outcome, not a structural guess). Stops at a NESTED handle: a
/// perform inside a handle discharging its op is RE-HOMED there (the escaping-k self-reinstall emits a
/// reified continuation `(fn (#kv) (handle-internal … (+ #kv (E.op))))` — a correct fold, not a leak); an
/// ungranted one is caught when that inner handle is itself reduced. Skips a DEAD `let` binding's init
/// (unreferenced downstream → lowering drops it, so its vestigial lambda never lifts — the directly-applied
/// `(let ((f (fn … (E.op)))) (f 3))` folds to `(let ((f …)) (* 3 2))`, `f` now dead).
fn leaks_no_home_perform_in_lambda(
    db: &mut Db,
    node: StructId,
    discharged: &[(u32, u32)],
    in_lambda: bool,
) -> bool {
    // A nested handle re-homes the ops it discharges — do not descend (see the doc note).
    if matches!(resolved_of(db, node), Resolved::Handle { .. })
        || db.ast.head_name(node) == Some(HANDLE_INTERNAL)
    {
        return false;
    }
    if in_lambda
        && let Resolved::Apply { head, .. } = resolved_of(db, node)
        && let Some((decl, idx)) = crate::eval::effect_op_of(db, head)
        && discharged.contains(&(decl.0, idx))
        && matches!(
            crate::lower::core_of(db, node),
            crate::core::Core::Poison(ref r) if r.message == crate::diag::NO_HOME_STANDALONE_DECLINE
        )
    {
        return true;
    }
    // A DEAD let-binding's init is dropped by lowering — skip it (see the doc note).
    if let Resolved::Let { bindings, body } = resolved_of(db, node) {
        for (i, &(name_occ, init)) in bindings.iter().enumerate() {
            let referenced = binder_name_referenced(db, name_occ, body)
                || bindings
                    .iter()
                    .skip(i + 1)
                    .any(|&(_, later_init)| binder_name_referenced(db, name_occ, later_init));
            if referenced && leaks_no_home_perform_in_lambda(db, init, discharged, in_lambda) {
                return true;
            }
        }
        return leaks_no_home_perform_in_lambda(db, body, discharged, in_lambda);
    }
    let next_in_lambda = in_lambda || matches!(resolved_of(db, node), Resolved::Lambda { .. });
    match db.ast.get(node).clone() {
        Struct::List(children) => children
            .iter()
            .any(|&c| leaks_no_home_perform_in_lambda(db, c, discharged, next_in_lambda)),
        Struct::Atom(_) => false,
    }
}

/// Whether the name bound by `name_occ` (a `let`-binder name atom) appears as a reference anywhere in
/// `scope` — a syntactic name-match walk (sufficient for the dead-binding skip: a shadowing inner binder only
/// ADDS occurrences, so a false "referenced" is conservative — it keeps the leak scan from skipping a live
/// init, never makes it fire spuriously).
fn binder_name_referenced(db: &mut Db, name_occ: StructId, scope: StructId) -> bool {
    let Some(name) = db.ast.as_name(name_occ).map(|s| s.to_string()) else {
        return false;
    };
    fn walk(db: &Db, node: StructId, name: &str) -> bool {
        if db.ast.as_name(node) == Some(name) {
            return true;
        }
        match db.ast.get(node).clone() {
            Struct::List(children) => children.iter().any(|&c| walk(db, c, name)),
            Struct::Atom(_) => false,
        }
    }
    walk(db, scope, &name)
}

/// The number of SYNTACTIC discharged-perform occurrences in `node` — a `(op …)` application whose head is
/// one of this handler's discharged operations (`is_perform`). Does NOT follow calls into effectful callees
/// (unlike `subtree_performs`, which is a reachability predicate) and does NOT descend into lambda bodies (a
/// perform inside a closure fires only when applied). Used by the escaping-k re-performing reify (FACE-1 B2)
/// to gate on the number of holes in the continuation: the self-re-installing reify folds cleanly only when
/// the continuation carries EXACTLY ONE remaining perform after the leading hole (so a single natural re-
/// entry bottoms out at the pure-one-hole fold). A body with more discharged performs would need repeated
/// re-installs the current single-level reify does not drive to completion — decline cleanly instead.
fn count_discharged_performs(db: &mut Db, node: StructId, ctx: &HandlerCtx) -> u32 {
    if let Resolved::Apply { head, .. } = resolved_of(db, node)
        && is_perform(db, head, ctx).is_some()
    {
        return 1;
    }
    if matches!(resolved_of(db, node), Resolved::Lambda { .. }) {
        return 0;
    }
    match db.ast.get(node).clone() {
        Struct::List(children) => children
            .iter()
            .map(|&c| count_discharged_performs(db, c, ctx))
            .sum(),
        Struct::Atom(_) => 0,
    }
}

/// Copy a perform-FREE subtree so it is self-contained in the rewritten body (a fresh occurrence
/// re-resolving against the rewritten scope). A constant leaf is shared; a name atom is copied fresh; a
/// list is copied with its children copied. (This is `beta_reduce` with an empty substitution — reused
/// so the copy discipline is identical.)
fn copy_pure(db: &mut Db, node: StructId) -> StructId {
    crate::eval::beta_reduce(db, node, &HashMap::default())
}

/// Whether `node` is a SELF-CONTAINED constant — a bare int/bool/float/string leaf whose value resolves
/// position-independently, so the `thread` fold may safely SHARE + `deep_fresh_copy` it at N state-splice
/// sites (each copy re-resolves to the same constant). A NAME atom (or any list that may contain one) is
/// NOT shareable: it resolves by a scope walk, and a `deep_fresh_copy` at a splice site re-pushes it UNPINNED
/// so a leaf pinned to a LIVE enclosing binder (a caller runtime-arg seed) re-resolves against the folded
/// orphan → unbound (the let-wrapped-handle-seed CDZ0101). Used to gate the seed let-lift in `reduce_handle`:
/// a constant seed threads as-is (byte-identical, the common case); a non-constant seed is let-bound once.
fn seed_is_shareable_constant(db: &Db, node: StructId) -> bool {
    match db.ast.get(node) {
        Struct::Atom(lid) => !matches!(db.ast.leaf(*lid), Leaf::Name(_)),
        Struct::List(_) => false,
    }
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

/// FINDING-24 `#st` init copy: a [`deep_fresh_copy`] that PRESERVES a `#seed` ref BY REFERENCE (returns the
/// original atom) while re-pushing every OTHER node fresh. The `#st` per-dispatch state bind needs BOTH
/// invariants at once, which neither a full fresh-copy nor a by-reference registration alone gives:
///   - An OUTER FREE-VAR the growing op-arg carries (`(Bytes.concat fr (bin (u8 (UInt8.wrap v))))`, `v` ↦
///     `(+ 10 n)` — main's param `n`) must be FRESH-copied: a by-reference registration ALIASES the shared
///     `n` occurrence (single-parent arena → the other occurrence orphans, CDZ0101 unbound `n`; bf1/bf2/bf3).
///   - The handle-seed `#seed` ref (`(Map.insert #seed k v)`, bound OUTER by `apply_seed_wrap`) must stay
///     ATTACHED to its ORIGINAL atom: fresh-copying it re-pushes it UNPINNED, and piece-3's `forget_subtree`
///     only clears the memo for the CURRENT handle's drained inits — a NESTED handler's inner `#seed`
///     (xh1: an inner `put`-handler seed reached across a handler boundary) is NOT in that snapshot, so a
///     fresh `#seed` re-resolves UNBOUND (CDZ0101 `#seed`). Keeping the original `#seed` atom preserves its
///     grafted parent chain, so it resolves exactly as the by-reference path did.
///
/// A `#seed` ref occurs ONCE in a next-state (one handle-seed splice), so sharing that single atom into the
/// fresh parent is arena-safe (one position, one parent). Every non-`#seed` leaf — including the free-var —
/// is genuinely fresh, so nothing else is aliased.
fn deep_fresh_copy_keep_seed(db: &mut Db, node: StructId) -> StructId {
    match db.ast.get(node).clone() {
        Struct::Atom(lid) => {
            // A `#seed`-named atom is returned BY REFERENCE (keeps its grafted attachment); any other leaf
            // (a free-var name, a literal) is re-pushed fresh so it cannot alias its origin occurrence.
            if let Leaf::Name(nm) = db.ast.leaf(lid)
                && nm.starts_with("#seed")
            {
                return node;
            }
            let leaf = db.ast.leaf(lid).clone();
            db.push_atom(leaf)
        }
        Struct::List(children) => {
            let copied: Vec<StructId> = children
                .iter()
                .map(|&c| deep_fresh_copy_keep_seed(db, c))
                .collect();
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
