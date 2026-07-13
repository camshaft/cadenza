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
    /// A stable identity STRING for this handler context — the discharged ops + their arm occurrences,
    /// in sorted order — used as the specialization memo key (`db.effect_specializations`). A RESOLVED
    /// identity (occurrences), NOT `format!("{:?}", body)` — the old compiler's stringly-typed-syntax
    /// footgun (`DESIGN-effects-rcdzc.md` §4.3). Empty until built by `HandlerCtx::new`.
    key: String,
    /// The STATE-BINDER occurrence for the SINGLE-STATE fast path (E3 countdown/range-sum): the arm's
    /// `state` binder, if every arm in this context shares one. A specialized recursive fn threads this
    /// as its trailing state parameter. `None` when the context is not single-state (multiple distinct
    /// state binders across arms — the two-nested case, a later increment).
    single_state: Option<StructId>,
    /// The state's TYPE (the type of the handle's `init` seed) — used to ANNOTATE the specialized
    /// recursive fn's trailing state parameter so it types (a synthesized param has no source
    /// annotation). `None` if the init type is undetermined (then a specialization declines).
    state_ty: Option<crate::ty::Ty>,
}

impl HandlerCtx {
    /// Build a handler context from its operation→arm map, computing the specialization key and the
    /// single-state binder. The key is the discharged ops (`decl:idx`) plus each arm's occurrence, sorted
    /// — a stable RESOLVED identity. Single-state holds when there is exactly one arm (so exactly one
    /// state binder); a multi-arm context is not single-state here (its state threading is the same binder
    /// per arm today, but a recursive specialization over multiple arms is a later increment).
    fn new(arms: HashMap<(u32, u32), HandleArm>, state_ty: Option<crate::ty::Ty>) -> HandlerCtx {
        let mut parts: Vec<String> = arms
            .iter()
            .map(|((d, i), arm)| format!("{d}:{i}@{}", arm.op.0))
            .collect();
        parts.sort();
        let key = parts.join(",");
        // Single-state: all arms discharge ONE effect (one `decl`), so the handler threads ONE logical
        // state — regardless of arm count. Each arm binds its OWN `state` occurrence, but they name the
        // same threaded value, and a recursive fn under such a handler specializes with ONE trailing
        // state param (each perform substitutes its arm's own state binder). Covers countdown/range-sum
        // (1 arm) AND a 2-arm single-effect scalar handler. A context spanning TWO effects (two distinct
        // decls — the two-nested case) needs a state STACK, a later increment, so it stays `None` here.
        // The representative binder (used only as a presence gate downstream) is the first arm's.
        let one_effect = {
            let mut decls = arms.keys().map(|&(d, _)| d);
            let first = decls.next();
            first.is_some() && decls.all(|d| Some(d) == first)
        };
        let single_state = if one_effect {
            arms.values().next().map(|a| a.state)
        } else {
            None
        };
        HandlerCtx {
            arms,
            key,
            single_state,
            state_ty,
        }
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
                    out.push(
                        crate::diag::Reject::coded(
                            crate::diag::Code::LatentAuthority,
                            "this entrypoint delegates an effect to the host that its body never \
                             reaches (latent authority); the manifest must be exactly the effects \
                             that escape",
                        )
                        .at(occ),
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
/// occurrence is `decl` — following NON-RECURSIVE calls into their callee bodies (the perform may be
/// cross-function, as the delegation-reaches-a-recursive-callee case shows; a recursive callee is not
/// followed here — that reachability is E3, and missing it only UNDER-reports latent authority, the
/// safe direction). Used by the CDZ0404 latent-authority check.
fn body_reaches_effect(db: &mut Db, node: StructId, decl: u32, depth: u32) -> bool {
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
        && !crate::eval::is_recursive(db, callee)
        && body_reaches_effect(db, callee, decl, depth + 1)
    {
        return true;
    }
    match db.ast.get(node).clone() {
        Struct::List(children) => children
            .iter()
            .any(|&c| body_reaches_effect(db, c, decl, depth)),
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
    // The state's type — from the `init` seed — annotates a specialized recursive fn's state param. A
    // deferred/undetermined init type (e.g. a bare literal not yet grounded) is grounded to a definite
    // integer here via `type_of`; if still undetermined a recursive specialization declines.
    let state_ty = {
        let t = crate::infer::type_of(db, init);
        if matches!(t, crate::ty::Ty::Any) {
            None
        } else {
            Some(t)
        }
    };
    let ctx = HandlerCtx::new(map, state_ty);
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
    thread_bounded(db, node, state, ctx, 0)
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
    state: StructId,
    ctx: &HandlerCtx,
    inline_depth: u32,
) -> Option<(StructId, StructId)> {
    if inline_depth > THREAD_INLINE_LIMIT {
        return None; // an unbounded inline chain — decline (a recursive callee the spec path missed)
    }
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
                let (ra, next) = thread_bounded(db, a, cur, ctx, inline_depth)?;
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
            if ctx.single_state.is_some() && recursive_call_reaches_discharged(db, &head, ctx) =>
        {
            // Thread state through the args first (they evaluate before the call), then the call takes the
            // current threaded state as its trailing argument.
            let mut cur = state;
            let mut rargs = Vec::with_capacity(args.len() + 1);
            for &a in args.iter() {
                let (ra, next) = thread_bounded(db, a, cur, ctx, inline_depth)?;
                rargs.push(ra);
                cur = next;
            }
            let spec = specialize_recursive(db, head, ctx)?;
            rargs.push(cur); // the state argument, in the state as it stands at the call
            // Build the call `(<spec-name> args… state)`. The specialized def is named, so a name atom
            // resolves to it (via `def_by_name`), and the ordinary recursive `Core::Call` + reachability
            // path emits it.
            let name_atom = db.push_atom(Leaf::Name(spec));
            let mut call = vec![name_atom];
            call.extend(rargs);
            // The call's VALUE is the specialized fn's result; the state after it is not observed (the
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
            let mut cur = state;
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
            let (rcond, cur) = thread_bounded(db, cond, state, ctx, inline_depth)?;
            let (rthen, _) = thread_bounded(db, then_, cur, ctx, inline_depth)?;
            let (relse, _) = thread_bounded(db, else_, cur, ctx, inline_depth)?;
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
            let mut cur = state;
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
        // A NESTED `handle` in the handled body — handlers COMPOSE inside-out. Reduce the inner handle
        // recursively (`reduce_handle`), which discharges ITS OWN effect and rewrites its performs to
        // plain code; the result may still perform the OUTER effect (an effect the inner handler does not
        // discharge), which we then thread under the OUTER context. So `(handle_B … (handle_A … body))`
        // folds `A` away first, leaving `B` performs for `B`'s fold. The inner reduction is
        // self-contained (it threads its OWN init/state); its result is a value in the OUTER context, so
        // the outer state passes THROUGH unchanged at this node (the inner handle's own state is not the
        // outer's). This is what makes two nested intra-program handlers work.
        Resolved::Handle {
            init: inner_init,
            arms: inner_arms,
            body: inner_body,
        } => {
            let reduced = reduce_handle(db, inner_init, &inner_arms, inner_body)?;
            // Thread the reduced result (which may still perform the outer effect) under the outer ctx.
            thread_bounded(db, reduced, state, ctx, inline_depth)
        }
        // An ordinary application / arithmetic / comparison / connective / `not` over sub-expressions:
        // thread state through the operands in left-to-right order, rebuilding the same head. This
        // covers `(+ (E.op) 1)`, `(List.push s (E.op))`, etc. The head itself is not a perform (that
        // arm above caught it), so it is copied as-is.
        Resolved::Apply { head, args } => {
            let mut cur = state;
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
    let _state_binder = ctx.single_state?;
    let state_ty = ctx.state_ty.clone()?;
    // The state type must be FULLY DETERMINED to annotate the trailing state param. An UNDETERMINED
    // component (an `Any` — most commonly an empty-list seed `(list)`, whose element type is `Ty::Any`
    // until an operation pins it) would bake a wrong/loose annotation (`(: s (List Any))`) that mistypes
    // the threaded body. Decline cleanly rather than emit it — a non-empty seed (`(list 0)`, whose element
    // is `Int64`) specializes; an empty-list seed needs the state's type inferred from the arms'
    // resume-next-states (`(List.push s v)` reveals `List Int64`), a later increment.
    if ty_has_any(&state_ty) {
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

    // Build the specialized def as a REAL AST form `(def (spec (: n Tn)… (: s Ts)) <body>)`, so its
    // parameters resolve (via `is_param_occurrence`, which walks to a `def` form) and each types by its
    // annotation. Every param — original AND the trailing state — is an ANNOTATED binder `(: name T)`
    // (the state param LAST, since the self-call appends state last).
    let spec_name_atom = db.push_atom(Leaf::Name(spec_name.clone()));
    let mut sig_children = vec![spec_name_atom];
    for (n, ty) in &orig_param_specs {
        let name_atom = db.push_atom(Leaf::Name(n.clone()));
        let ty_expr = crate::eval::encode_typeval(db, ty);
        let colon = db.push_atom(Leaf::Name(":".to_string()));
        sig_children.push(db.push_list(vec![colon, name_atom, ty_expr]));
    }
    let state_name = db.push_atom(Leaf::Name(format!("{spec_name}$s")));
    let state_type_expr = crate::eval::encode_typeval(db, &state_ty);
    let colon = db.push_atom(Leaf::Name(":".to_string()));
    let state_param = db.push_list(vec![colon, state_name, state_type_expr]);
    sig_children.push(state_param);
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

    // Thread `orig_body` under `ctx`, with the incoming state = a REFERENCE to the state param `s`. The
    // performs' resume values reference the arm's state binder, which `thread`'s perform arm substitutes
    // with this state expression; the recursive self-call re-enters and (via the memo) rewrites to
    // `(spec_name <threaded-state>)`. The state name atom must re-resolve to the param, so we pass a
    // FRESH occurrence of the name (a bare `s` reference), not the binder occurrence.
    let state_ref = db.push_atom(Leaf::Name(format!("{spec_name}$s")));
    let (spec_body, _out) = thread(db, orig_body, state_ref, ctx)?;

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
    state: StructId,
    ctx: &HandlerCtx,
    inline_depth: u32,
) -> Option<(StructId, StructId)> {
    if subtree_performs(db, node, ctx) {
        thread_bounded(db, node, state, ctx, inline_depth)
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
