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

use crate::ast::{Arenas, CompoundCtor, IntValue, Leaf, Radix, Struct, StructId};
use crate::db::{Db, Def, EffectDecl, OpDecl};
use crate::fxhash::FxHashMap as HashMap;
use crate::prelude::{meta_field, push_atom, push_list};
use crate::resolve::resolved_of;
use crate::resolved::{HandleArm, Resolved};

mod thread;
pub(crate) use thread::*;

mod reduce;
pub use reduce::reduce_handle;
pub(crate) use reduce::*;

/// Inject the prelude `(effect Eval (op in-caller (-> Ast Ast)))` as a top-level module member BEFORE
/// `scan_top_level`, so `Eval` / `in-caller` resolve in every module. `Eval` is the COMPILE-TIME
/// metaprogramming effect (co-designed with v-metaprogramming, operator-confirmed): a macro carries
/// `{Eval}` in its written effect row and `(in-caller ast)` evaluates an `Ast` in the DIRECT caller's
/// lexical env, returning the resulting `Ast`. It is a DECL ONLY — no ambient handler — because the macro
/// EXPANDER discharges every `in-caller` op at EXPANSION (pre-infer) against the captured call-site env and
/// ERASES `{Eval}` from the row before infer's effect-row solve, so infer never sees an unhandled `Eval`
/// (the decl merely makes `{Eval}` rows + `(in-caller …)` RESOLVE at parse/resolve time, pre-expansion).
/// Mirrors `wit_world::inject_world_import_effects`'s inject-before-`scan_top_level` pattern; `Ast` is a
/// built-in prelude sum, resolvable here. SKIPS if the module already declares its own `Eval` (a user decl,
/// or a re-inject guard) so it never duplicates.
pub(crate) fn inject_prelude_eval_effect(ast: &mut Arenas) {
    let root = ast.root;
    // Only inject into a top-level-ITEMS CONTAINER — a `(do …)` or `(module …)`. A program whose root is a
    // BARE single form (a lone `(def sig body)`, or a bare expression) is NOT an item list: appending the
    // Eval member to it would corrupt the form — e.g. `(def main body)` → `(def main body (effect Eval …))`,
    // read as a def with TWO bodies → a spurious CDZ0201 "more than one body" on a valid single-def program
    // (#7823 regression, breaks the cdz lsp diagnostic-count tests). Such a program has no macro/`{Eval}` use
    // anyway, so skipping the inject there loses nothing. `scan_top_level` still handles the bare-form root.
    if ast.as_form(root, "do").is_none() && ast.as_form(root, "module").is_none() {
        return;
    }
    let items = match ast.get(root) {
        Struct::List(items) => items.clone(),
        Struct::Atom(_) => return,
    };
    // Skip if an `(effect Eval …)` is already declared at top level (user shadow / re-inject guard).
    let already = items.iter().any(|&it| {
        ast.as_form(it, "effect")
            .and_then(|t| t.first().copied())
            .and_then(|n| ast.as_name(n))
            .is_some_and(|n| n == "Eval")
    });
    if already {
        return;
    }
    // Build `(effect Eval (op in-caller (-> Ast Ast)))` (sequential lets — `push_*` each take `&mut ast`).
    let arrow_head = push_atom(ast, Leaf::Name("->".into()));
    let arrow_dom = push_atom(ast, Leaf::Name("Ast".into()));
    let arrow_cod = push_atom(ast, Leaf::Name("Ast".into()));
    let arrow = push_list(ast, vec![arrow_head, arrow_dom, arrow_cod]);
    let op_head = push_atom(ast, Leaf::Name("op".into()));
    let op_name = push_atom(ast, Leaf::Name("in-caller".into()));
    let op = push_list(ast, vec![op_head, op_name, arrow]);
    let effect_head = push_atom(ast, Leaf::Name("effect".into()));
    let effect_name = push_atom(ast, Leaf::Name("Eval".into()));
    let decl = push_list(ast, vec![effect_head, effect_name, op]);
    // Add the Eval decl as a top-level SIBLING member. The top-level item set is what `top_items` reads:
    // a `(module …)` / `(do …)` root is an item CONTAINER (its members are the tail), so we append into it
    // (mirror `append_module_member`: after a `(module name …)` header if present, else at the end). But a
    // program whose root is a SINGLE bare top-level form — `(def …)`, `(type …)`, a bare expression — is
    // NOT a container: `top_items` returns `vec![root]`, the lone item. Appending the decl to *that* form's
    // own children would grow the form itself (e.g. `(def answer 42 (effect Eval …))` → a false CDZ0201
    // "this definition has more than one body"), which broke every single-top-form program (LSP
    // diagnostics-as-you-type, snippets). So wrap the lone form and the Eval decl in a synthetic top-level
    // `(do …)`, which `top_items` expands to the two sibling items.
    let root_is_item_container =
        ast.as_form(root, "module").is_some() || ast.as_form(root, "do").is_some();
    if root_is_item_container {
        let mut new_items = items;
        let insert_at = if ast.as_form(root, "module").is_some() && new_items.len() >= 2 {
            2
        } else {
            new_items.len()
        };
        new_items.insert(insert_at, decl);
        ast.structure[root.0 as usize] = Struct::List(new_items);
    } else {
        let do_head = push_atom(ast, Leaf::Name("do".into()));
        let wrapped = push_list(ast, vec![do_head, root, decl]);
        ast.root = wrapped;
    }
}

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
    // The record head is the NATIVE ctor-LEAF `Leaf::Ctor(Record)` (unshadowable, recognized by kind — the
    // NAME `record` is a shadowable alias); it resolves structurally via `compound_ctor_leaf` (the M3
    // reader-flip removed the legacy `"record"` string-head dual-read).
    let head = push_atom(ast, Leaf::Ctor(CompoundCtor::Record));
    let mut children = vec![head];

    // `(meta t)` — the effect type-value, so a later pass can recover the effect's identity.
    let eff_ty = effect_typeval(ast, decl);
    children.push(meta_field(ast, "t", eff_ty));

    // One field per operation, its value the operation-value record. The operation's INDEX in
    // declaration order is its stable operation index.
    for (index, op) in decl.ops.iter().enumerate() {
        let value = op_value(ast, decl, op, index as u32);
        let k = push_atom(ast, Leaf::Name(op.name.clone().into()));
        children.push({
            let eq = push_atom(ast, Leaf::Name("=".into()));
            push_list(ast, vec![eq, k, value])
        });
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
    let head = push_atom(ast, Leaf::Ctor(CompoundCtor::Record));
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
pub(crate) struct HandlerCtx {
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
    /// EFFECT NON-LOCAL EXIT (v-effects, CASE 1). Set TRUE by `reduce_handle` ONLY when it has DECIDED to
    /// lower this handle's unliftable abort(s) as `Core::HandleAbort` (a non-local RETURN from the function)
    /// rather than decline — i.e. exactly when (a) the handle is the WHOLE enclosing-function body (so
    /// handle-result == function-result, from the lowering caller) AND (b) the ONLY unsoundness is a SAME-FN
    /// abort the tail-resumptive fold cannot lift (a cross-fn abort still declines). When TRUE, `thread`'s
    /// abortive-arm handler emits `Core::HandleAbort` at the abort position instead of the whole-handle
    /// COLLAPSE. Left FALSE for the cases that already FOLD (an unconditional collapse / a hoisted conditional
    /// abort) so their existing lowering (with its seed-wrap / drain / foreign-advance handling) is UNTOUCHED —
    /// this flag flips ONLY the formerly-declining same-fn subset. Further GATED at the handler with
    /// `!in_recursive_specialize` (an abort inside a specialized recursive callee stays declined — a bare
    /// return there cannot abandon the caller's continuation; the deferred tagged-return "later vertical").
    abort_as_handleabort: std::cell::Cell<bool>,
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
            abort_as_handleabort: std::cell::Cell::new(false),
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
    // MEMO (seq-203 EXPONENTIAL fix): dedup the inter-procedural transitive-follow re-analysis. Without it, a
    // nested immediately-applied-lambda chain re-analyzes each shared sub-callee body via BOTH the follow
    // (`sub_extra`) AND `walk(head)`'s re-walk of the same lambda → 2^N compile-time hang (v-compiler-perf:
    // N=24 = 14s, N~40 = hang). Keyed on `(callee_body, arity, depth)`: identical inputs yield an identical
    // result (a re-analysis is redundant), and DEPTH is in the key so a body reached at a different
    // inter-procedural depth — where the `depth < 32` follow-gate may differ — is a DISTINCT entry, never
    // reusing a result computed under a different gate state (sound). Mirrors the sibling `check_no_home`'s
    // `followed` (callee_body, handled) dedup. The `handled` set is NOT in the key: this fn re-seeds it empty.
    memo: &mut crate::fxhash::FxHashMap<(StructId, usize, u32), Vec<Vec<u32>>>,
) -> Vec<Vec<u32>> {
    if let Some(cached) = memo.get(&(callee_body, arity, depth)) {
        return cached.clone();
    }
    // Test-only compile-cost counter: a memo MISS = one full body re-analysis. Pins the seq-203 #5755 memo
    // (a future un-memoization flips this from POLYNOMIAL back to 2^N — see the regression guard). Surfaced
    // via `CompileOutput::param_apply_extra_handled_calls`.
    #[cfg(test)]
    {
        db.param_apply_extra_handled_calls += 1;
    }
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
        memo: &mut crate::fxhash::FxHashMap<(StructId, usize, u32), Vec<Vec<u32>>>,
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
                        param_apply_extra_handled(db, head, sub_body, args.len(), depth + 1, memo);
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
                walk(db, head, params, handled, out, depth, memo);
                for &a in args.iter() {
                    walk(db, a, params, handled, out, depth, memo);
                }
            }
            Resolved::Handle { init, arms, body } => {
                walk(db, init, params, handled, out, depth, memo);
                for arm in arms.iter() {
                    walk(db, arm.body, params, handled, out, depth, memo);
                }
                let added: Vec<u32> = arms
                    .iter()
                    .filter_map(|a| crate::eval::effect_op_of(db, a.op).map(|(d, _)| d.0))
                    .collect();
                let before = handled.len();
                handled.extend(&added);
                walk(db, body, params, handled, out, depth, memo);
                handled.truncate(before);
            }
            Resolved::Host { effects, body } => {
                let added: Vec<u32> = effects
                    .iter()
                    .filter_map(|&e| host_effect_decl(db, e))
                    .collect();
                let before = handled.len();
                handled.extend(&added);
                walk(db, body, params, handled, out, depth, memo);
                handled.truncate(before);
            }
            _ => {
                if let Struct::List(children) = db.ast.get(node).clone() {
                    for c in children {
                        walk(db, c, params, handled, out, depth, memo);
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
    walk(
        db,
        callee_body,
        &params,
        &mut handled,
        &mut out,
        depth,
        memo,
    );
    memo.insert((callee_body, arity, depth), out.clone());
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
                    // Fresh memo per external entry (seq-203): shared through this call's transitive-follow
                    // recursion to kill the 2^N re-analysis of a nested applied-lambda chain.
                    let mut memo = crate::fxhash::FxHashMap::default();
                    param_apply_extra_handled(db, head, callee_body, args.len(), depth, &mut memo)
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

/// Mark every node id in `root`'s subtree as lying within a HANDLER-ARM-TAIL / `#st`-threaded region — call
/// on the form `reduce_handle` produced (the reduced/threaded handle). Recorded in `db.handler_region_nodes`
/// for CASE2's strict-heap-ctor decompose in `lower_let` to consult via [`node_in_handler_region`]: a dead
/// list/set/map ctor whose `let` is in such a region must NOT be `Core::Seq`-decomposed, because the handler
/// tail / `#st`-drop / per-dispatch-reclaim lowering recognizes sequencing via `do` FORMS and NOT `Core::Seq`,
/// so the wrapper perturbs it (olc1/cst1/sga1). Whole-subtree marking is intentionally over-broad but SAFE:
/// it only causes the (rare) pure-trap dead ctor inside a reduced handler to skip strict-eval — a known-gap,
/// never a miscompile. The set-insert doubles as the visited-guard (idempotent; terminates on a shared node).
pub fn mark_handler_region(db: &mut Db, root: StructId) {
    if !db.handler_region_nodes.insert(root) {
        return;
    }
    if let Struct::List(children) = db.ast.get(root).clone() {
        for c in children {
            mark_handler_region(db, c);
        }
    }
}

/// Whether `id` lies within a handler-arm-tail / `#st`-threaded region marked by [`mark_handler_region`].
/// CASE2's `lower_let` strict-heap-ctor decompose calls this on the dead-ctor's enclosing `let`-form id and
/// SKIPS the decompose + `Core::Seq` wrap when true (conformance-neutral skip; see [`mark_handler_region`]).
pub fn node_in_handler_region(db: &Db, id: StructId) -> bool {
    db.handler_region_nodes.contains(&id)
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
    // Ground the seed's free type params from the arms' resume next-states before returning. An
    // `(Option.None)` seed types as `(Option _)` (`None` fixes no payload), but the resumes thread
    // `(Option.Some …)` — the SAME state — so unifying grounds the payload to `(Option (List Int64))`.
    // Otherwise the state binder keeps the ungrounded `(Option _)` and a live `SumPayload` read of the
    // unsolved payload emits at the wrong width (func-12: an i32 heap handle read as i64).
    let seed_ty = crate::infer::ground_handler_state_ty(db, seed_ty, arms_list);
    // Memoize the grounded state type onto the INIT node. The handler fold MATERIALIZES the threaded
    // state as a `LocalRef` to this init `(Option.None)` node, and at the `SumPayload` width emit the
    // backend reads `type_of(that LocalRef)` = `type_of(init)` — the RAW ungrounded `(Option _)` — so the
    // payload read picks `get-int` (i64) for what is an i32 heap handle → invalid wasm (func-12). The init
    // IS the handler's initial state, so typing it as the (fully-ground) state type is correct, and the
    // memo makes every materialized occurrence read the grounded type at emit. `type_of` never memoizes a
    // free-var type, so only stamp a fully-ground result.
    if !crate::infer::ty_has_free_var(db, &seed_ty) {
        db.types.fill(init, seed_ty.clone());
    }
    if matches!(seed_ty, crate::ty::Ty::Any | crate::ty::Ty::Var(_)) {
        return None;
    }
    Some(seed_ty)
}

/// If `id` is a handler SEED (`init`) node whose computed type `t` still has a free var, ground it against
/// the handler's resume next-states — otherwise return `t` unchanged. Called from `infer::type_of` right
/// after `compute`, this grounds the state type AT TYPE-CHECK (the FIRST demand of the seed's type),
/// BEFORE lowering reads it: `type_of` does not memoize a free-var type but DOES memoize the resulting
/// fully-ground one, so the grounded state type becomes the cached answer every later read sees — including
/// the type match-lowering reads for the seed scrutinee (`(match s …)` where `s` resolves to a `Ref` to
/// this init). Without this the nested-SUM payload of an unannotated `(None)` seed stays `_` at the inner
/// match dispatch (`(Option (Option Int64))` → "sum match dispatches on a non-sum sub-value"): the
/// binder-keyed [`handle_arm_state_ty`] fill runs only once the state BINDER is typed, which for a
/// nested-match shape happens AFTER lowering has already read the raw seed type. Navigates by AST parent
/// (`id → (handle-internal INIT ARMS BODY) → ARMS`), valid at type-check (the lowering-time parent is a
/// restructured node, so this must run at type-check, which it does). Takes the pre-computed `t` so it
/// never re-enters `type_of(id)`; the grounding's own re-entrancy is bounded by `GROUNDING_ARMS`.
pub fn ground_seed_if_handle_init(db: &mut Db, id: StructId, t: crate::ty::Ty) -> crate::ty::Ty {
    if !crate::infer::ty_has_free_var(db, &t) {
        return t;
    }
    let Some(handle) = db.parent_of(id) else {
        return t;
    };
    // Scope the `as_form` borrow (a `&[StructId]` into `db.ast`) so it ends before the `&mut db` grounding
    // call below. `id` must be the SEED (element 0) of `(handle-internal INIT ARMS BODY)`; ARMS is element 1.
    let arms_list = {
        let Some(tail) = db.ast.as_form(handle, HANDLE_INTERNAL) else {
            return t;
        };
        if tail.first().copied() != Some(id) {
            return t;
        }
        match tail.get(1) {
            Some(&a) => a,
            None => return t,
        }
    };
    crate::infer::ground_handler_state_ty(db, t, arms_list)
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
    let head = db.push_atom(Leaf::Ctor(CompoundCtor::Tuple));
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

/// Whether `body` contains an ABORTIVE perform (of a `ctx`-abortive op) at a NON-TAIL position — the shape
/// single-return specialization mis-folds as tail-RESUMPTIVE (it returns the arm value as the perform's
/// result, threading it forward instead of abandoning the computation). Mirrors [`self_calls_tail`]'s tail
/// propagation: an `if`/`match` branch and a `let` body inherit the position's tail-ness; an operator
/// operand, a call/self-call ARGUMENT, and an `if`/`let` condition/init are NON-tail. An abort in the
/// function's actual TAIL is sound here (its arm value IS the return); an abort anywhere else is NOT — most
/// notably the ACCUMULATOR ARGUMENT that `accum::introduce` moves a non-tail associative recursion's abort
/// into (`(+ (loop …) (if c (E.bail) k))` → a tail self-call whose accum arg carries the abort), which
/// `self_calls_tail` misses because THE SELF-CALL is tail even though the abort is not. Declining this shape
/// is a safe CDZ0900 floor until the non-local-exit calling convention (the tagged-return vertical) lands.
fn abortive_perform_off_tail(db: &mut Db, node: StructId, ctx: &HandlerCtx, tail: bool) -> bool {
    if let Resolved::Apply { head, args } = resolved_of(db, node)
        && let Some(op) = is_perform(db, head, ctx)
    {
        if ctx.abortive.contains(&op) && !tail {
            return true;
        }
        // A tail-position (or resumptive) perform is sound here; still descend its ARGS (non-tail).
        return args
            .iter()
            .any(|&a| abortive_perform_off_tail(db, a, ctx, false));
    }
    if let Resolved::If { cond, then_, else_ } = resolved_of(db, node) {
        return abortive_perform_off_tail(db, cond, ctx, false)
            || abortive_perform_off_tail(db, then_, ctx, tail)
            || abortive_perform_off_tail(db, else_, ctx, tail);
    }
    if let Some(form) = db.ast.as_form(node, "let").map(|t| t.to_vec())
        && form.len() == 2
    {
        if let Struct::List(pairs) = db.ast.get(form[0]).clone() {
            for pair in pairs {
                if let Struct::List(kv) = db.ast.get(pair).clone()
                    && kv.len() == 2
                    && abortive_perform_off_tail(db, kv[1], ctx, false)
                {
                    return true;
                }
            }
        }
        return abortive_perform_off_tail(db, form[1], ctx, tail);
    }
    match db.ast.get(node).clone() {
        Struct::List(children) => children
            .iter()
            .any(|&c| abortive_perform_off_tail(db, c, ctx, false)),
        Struct::Atom(_) => false,
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
    // TAGGED-ABORT (non-local-exit CC, v-effects). When the ONLY obstacle to specializing this abortive
    // context is a NON-TAIL SELF-recursive call — walk `(+ 1 (walk (- n 1)))` with a base abort — the
    // tagged-return convention (specialize `walk#eff` to return `#tuple(tag value)`; every non-tail self-call
    // short-circuits its pending frame on the abort tag) folds it soundly to the abort value (99, not 102).
    // ELIGIBLE only for the shape `thread_returning_tagged` models in v1: a SINGLE-slot, SELF-recursive (not
    // mutual) callee whose abort is at a TAIL position (an off-tail / accum-rewritten abort stays declined by
    // the 4511 guard below). When eligible, DON'T decline here — route to `thread_returning_tagged` at the
    // threading fork. Otherwise the safe-floor decline stands (no miscompile ever ships).
    let calls_mutual = callee_calls_other_recursive_def(db, orig_body, callee_def);
    let all_tail = recursive_self_calls_all_tail(db, orig_body, callee_def);
    let off_tail = abortive_perform_off_tail(db, orig_body, ctx, true);
    // ACCUM-OFF-TAIL (13175): accumulator-introduction rewrote a NON-tail associative abortive recursion
    // `(+ (loop (- k 1)) (if c (E.bail) k))` into a TAIL self-call whose ACCUMULATOR ARG carries the abort —
    // so ALL self-calls are tail (`all_tail`) yet the abort is OFF-tail (`off_tail`). Route this shape to the
    // tagged-return CC (`thread_returning_tagged` distributes the abort out of the arg and short-circuits the
    // self-call on the abort tag); the threader DECLINES any sub-shape it cannot model, so routing never
    // miscompiles. NARROW: gated on `!ctx.abortive.is_empty()` (below), so a RESUMPTIVE accumulator recursion
    // (rw1/rw3/rwmatch — no abortive arm → empty `ctx.abortive`) is UNTOUCHED.
    let accum_off_tail = all_tail && off_tail && !ctx.abortive.is_empty();
    let tagged_abort = !ctx.abortive.is_empty()
        && ctx.slots.len() == 1
        // An OFF-tail abort normally vetoes tagged mode (the safe floor) — EXCEPT the accum-off-tail shape,
        // which the tagged threader now models (distributes the abort out of the accumulator arg).
        && (!off_tail || accum_off_tail)
        // A CALLER-OBSERVED out-state (`force_multivalue`, the sr5 family) already folds via the MULTI-VALUE
        // path (decided at ~4650, AFTER this point). Exclude it here so tagged mode — computed earlier —
        // does NOT preempt the multi-value tuple threading that case relies on. (Same `orig_body`/`ctx.key`
        // the multivalue decision reads.)
        && !db.force_multivalue.contains(&(orig_body, ctx.key.clone()))
        // The TRIGGER — one of:
        //   * a NON-tail SELF-call — walk `(+ 1 (walk …))` (#7613);
        //   * FORCED by a pending-in-handle-body caller (`db.force_tagged_abort`, adv-52 #7640) — a tail-recursive
        //     abortive callee whose abort must abandon a pending op at the OUTER call site;
        //   * a MUTUAL SCC — `ev`↔`od` where the cross-def call is non-tail (`(+ 1 (od …))`); the tagged threader
        //     treats a call to ANY SCC member as a recursive tag-check-short-circuit, so a partner's abort
        //     propagates its tag up the pending frames.
        //   * the ACCUM-OFF-TAIL shape (13175) — a tail self-call whose accumulator arg carries the abort.
        // `thread_returning_tagged` handles all four; it DECLINES (→ safe floor) any sub-shape v1 does not
        // model, so relaxing the gate never miscompiles.
        // A tail-recursive callee that ALREADY folds via the ordinary tail path (annotated-walk-and-bail) is NOT
        // rerouted: it is neither non-tail-self, forced, mutual, nor accum-off-tail, so no trigger fires.
        && (!all_tail
            || db.force_tagged_abort.contains(&callee_def)
            || calls_mutual
            || accum_off_tail);
    if !ctx.abortive.is_empty()
        && !recursive_self_calls_all_tail(db, orig_body, callee_def)
        && !tagged_abort
    {
        return None;
    }
    // ABORTIVE PERFORM AT A NON-TAIL POSITION IN THE (possibly ACCUM-REWRITTEN) CALLEE BODY. The self-call
    // tail-check above is INSUFFICIENT when accumulator-introduction has rewritten a non-tail associative
    // recursion `(+ (loop (- k 1)) (if c (E.bail) k))` into a TAIL self-call whose ACCUMULATOR ARGUMENT
    // carries the abort — the self-call is now tail (so the check above passes) but the abort rides a
    // non-tail position, and single-return specialization folds it as tail-RESUMPTIVE (returns the arm value
    // as the perform's result, flowing back through the accumulator → breaker's `103` instead of the abort
    // value or a decline). Reuse the handle-body guard's non-tail-abort detector on the callee body: an
    // abortive perform the fold cannot lift to a capturable position DECLINES cleanly (CDZ0900, a safe floor)
    // — the sound lowering needs the non-local-exit calling convention (the tagged-return vertical).
    if !tagged_abort
        && !ctx.abortive.is_empty()
        && abortive_perform_off_tail(db, orig_body, ctx, true)
    {
        return None;
    }
    // ABORTIVE + MUTUAL RECURSION: decline. `recursive_self_calls_all_tail` above checks only THIS def's
    // OWN self-calls, so a MUTUALLY-recursive callee (`ev` calls `od` calls `ev`) whose partner has a
    // NON-tail call to it passes that check yet still has pending frames an abort must abandon — the same
    // miscompile (`(def (ev n) (if (= n 0) (Bail 99) (+ 1 (od …)))) (def (od n) (+ 1 (ev …)))` → 103, not
    // 99). Verifying cross-def tail-ness over the whole recursive group is the non-local-exit vertical;
    // until then, an abortive context over a MUTUALLY-recursive callee (one that calls ANOTHER recursive
    // def) declines cleanly — UNLESS `tagged_abort` fired for it (the non-local-exit tagged-return CC now
    // threads the whole SCC: `thread_returning_tagged` treats a call to any `mutual_scc_of` member as a
    // recursive tag-check-short-circuit, so a partner's abort propagates its tag up the pending frames). A
    // mutual shape the tagged threader cannot model returns `None` there → this safe-floor decline still
    // stands via the fold's overall `?`. (A self-recursive callee is handled by the tail check above.)
    if !tagged_abort
        && !ctx.abortive.is_empty()
        && callee_calls_other_recursive_def(db, orig_body, callee_def)
    {
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
        // A CALLER-observed SCC (the handle body's trailing draw reads the SCC's final out-state — mutrec) is
        // group-foldable ONLY when it is PURE-mutual — no member SELF-recurses. A member that ALSO self-calls
        // (frb3: `outer2` self-recurses AND mutual-calls `inner2`) has its `caller_observes_outstate` set by
        // the recursion-BOUNDARY marking (finding #19, keyed on a self-call's arg reaching a partner), NOT a
        // handle-body observer; the group fold does not compose that self-recursion out-state with the mutual
        // threading and silently mis-values (frb3 → 2 not 3). So gate the caller-observed branch on
        // no-self-call; a within-body partner-precedes-observation SCC is unaffected (its own branch).
        let caller_observed_pure_mutual = caller_observes_outstate
            && scc.iter().all(|&m| {
                db.defs[m]
                    .body
                    .is_some_and(|mb| !contains_self_call(db, mb, m))
            });
        // A genuine mutual SCC (more than just this def) with at least one out-state-observing member, all of
        // whose leaves the group multi-value machinery can bind.
        scc.len() > 1
            && (caller_observed_pure_mutual
                || scc.iter().any(|&m| {
                    db.defs[m]
                        .body
                        .is_some_and(|mb| mutual_partner_precedes_observation(db, mb, m, ctx))
                }))
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
    // sr5: a CALLER-OBSERVED callee whose out-state an ABORTIVE observer reads (`(do (def _g (grow k)) (Acc.fin))`,
    // `fin` aborts reading state) MUST still thread multi-value — the abort collapse binds its arm-state from the
    // threaded `cur[slot]`, so a single-return callee (which leaves `cur` unadvanced) makes the abort read the
    // PRE-recursion seed (breaker sr5, was a guarded decline). The `ctx.abortive.is_empty()` floor over-excluded
    // this: `selfcall_precedes` genuinely needs abortive-empty (its advance isn't threaded), but a caller-observed
    // out-state IS threaded to the abort via `cur`. So allow the abortive context for the caller-observed case.
    let multivalue = ((selfcall_precedes_perform_in_operands(db, orig_body, callee_def, ctx)
        && ctx.abortive.is_empty()
        || caller_observes_outstate)
        && multivalue_leaves_threadable(db, orig_body, callee_def))
        || group_member
        || group_entry;
    if selfcall_precedes_perform_in_operands(db, orig_body, callee_def, ctx) && !multivalue {
        return None; // an out-state-observing shape the multi-value path does not cover yet (abortive, or
        // a self-call gated behind a conditional inside a leaf) — decline BEFORE reserving the def.
    }
    // sr5 SAFE-DECLINE (the non-threadable subset): a caller-observed callee under an ABORTIVE observer whose
    // leaves the multi-value machinery CANNOT bind stays single-return, which would leave `cur` unadvanced and
    // make the abort read the pre-recursion seed (the sr5 miscompile). Fold it only when threadable (above);
    // decline cleanly otherwise. This replaces the pre-reduction `body_recursive_advance_observed_by_abort`
    // guard's role for the shape the threading path cannot serve.
    if caller_observes_outstate && !ctx.abortive.is_empty() && !multivalue {
        return None;
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
    if !group_entry
        && !group_member
        && caller_observes_outstate
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
        // TAGGED-ABORT CC: register BEFORE threading so the handle-body call rewrite (thread.rs recursive-call
        // arm) knows THIS spec returns a `#tuple(tag value)` and collapses its call to `(. r 1)`.
        if tagged_abort {
            db.tagged_abort_specs.insert(spec_name.clone());
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
        let spec_body = if tagged_abort {
            // TAGGED-ABORT MODE (non-local-exit CC): thread the body so every tail yields `#tuple(tag value)`
            // and each non-tail self-call short-circuits its pending frame on the abort tag. Declines (→ the
            // 4499 safe floor already bypassed above cannot re-fire; a None here propagates as the whole
            // fold's decline) for any sub-shape v1 does not model.
            {
                // The recursive GROUP (self-recursive → `[callee_def]`; mutual SCC → all members). The tagged
                // threader treats a call to ANY member as a recursive call (tag-check-short-circuit), so a
                // mutual partner's abort propagates its tag up through the pending frames the same as a self-call.
                let scc = mutual_scc_of(db, callee_def, ctx);
                let _ = accum_off_tail; // the accum shape is handled inside thread_returning_tagged's self-call arm
                thread_returning_tagged(db, orig_body, state_refs, ctx, callee_def, &scc)?
            }
        } else if multivalue {
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
            db.tagged_abort_specs.remove(&spec_name);
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
        Resolved::Tuple { elems } | Resolved::List { elems } | Resolved::Set { elems } => {
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

pub(crate) fn subtree_references_binder(db: &mut Db, node: StructId, binder: StructId) -> bool {
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
        Resolved::Tuple { elems } | Resolved::List { elems } | Resolved::Set { elems } => {
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
        Resolved::Tuple { elems } | Resolved::List { elems } | Resolved::Set { elems } => {
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
    // rebuild they DANGLE at that bare resume and re-lower as the bare-resume not-reducible decline. Rewrite
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
    // re-lower as the bare-resume not-reducible decline. Rewrite the scrutinee's resume to its refolded
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
        return reduce_handle(db, next_state, arms, filled, false);
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
pub(crate) fn arm_has_resume(db: &mut Db, node: StructId) -> bool {
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
/// CDZ0408 detection (the boundary-crossing MULTI-SHOT subset of the tail-resumptive-fold decline). After
/// [`reduce_handle`] returns `None`, the emit path (`lower/compute.rs`) asks this whether the decline is the
/// specific "multi-shot resumption crosses an effect boundary" invariant (→ [`Code::MultiShotCrossesEffectBoundary`],
/// CDZ0408) versus the generic cross-function / non-tail not-yet decline (→ CDZ0900). TRUE iff some arm is
/// MULTI-SHOT (`count_resumes > 1`) AND the handle body reaches a perform this handler does NOT discharge (a
/// host call or an outer handler's op — [`body_reaches_foreign_perform`], which treats any op not in this
/// handler's arms as foreign). This is exactly the case [`reduce::reduce_handle`]'s refold guard skips
/// (`count_resumes == 1 || !body_reaches_foreign_perform`, reduce.rs) — re-running the continuation per
/// resume would DOUBLE the boundary effect (§4.4). The `count_resumes > 1` gate EXCLUDES the genuinely-not-yet
/// one-shot cross-function / non-tail forms, so they correctly keep CDZ0900. Builds a minimal `HandlerCtx`
/// (op→arm map keyed `(decl.0, idx)`, mirroring `reduce_handle`) — only the arm-op keyset is consulted.
pub(crate) fn handler_declines_multishot_boundary(
    db: &mut Db,
    arms: &[HandleArm],
    body: StructId,
) -> bool {
    // THIS handler (the single-handle host-boundary shape): a multi-shot arm here whose body reaches a
    // perform foreign to THIS handler.
    if one_handle_multishot_reaches_foreign(db, arms, body) {
        return true;
    }
    // NESTED handles in the body (the outer-handler-op shape): a one-shot OUTER handler wrapping an INNER
    // multi-shot handler whose continuation reaches the OUTER handler's op — from the INNER handler's arm
    // set that outer op is FOREIGN, so re-running the continuation per resume would double the outer effect.
    // reduce_handle declines the whole (outer) handle here, so the inner handle is never lowered on its own;
    // walk the body for any nested `Resolved::Handle` and check its own multi-shot-boundary condition.
    body_has_nested_multishot_boundary(db, body, 0)
}

/// The single-handle multi-shot-boundary check: some arm of THIS handler is multi-shot (`count_resumes > 1`)
/// AND `body` reaches a perform NOT discharged by this handler (a host call or an op of another effect —
/// [`body_reaches_foreign_perform`]). Builds a minimal `HandlerCtx` (op→arm map keyed `(decl.0, idx)`,
/// mirroring `reduce_handle`; only the arm-op keyset is consulted).
fn one_handle_multishot_reaches_foreign(db: &mut Db, arms: &[HandleArm], body: StructId) -> bool {
    if !arms.iter().any(|arm| count_resumes(db, arm.body) > 1) {
        return false;
    }
    let mut map = HashMap::default();
    for arm in arms {
        match crate::eval::effect_op_of(db, arm.op) {
            Some((decl, idx)) => {
                map.insert((decl.0, idx), arm.clone());
            }
            None => return false, // malformed / non-effect-op arm — not this subset (CDZ0403/CDZ0101 elsewhere)
        }
    }
    let ctx = HandlerCtx::new(db, map, Vec::new());
    body_reaches_foreign_perform(db, body, &ctx)
}

/// Walk `node`'s subtree for a nested `Resolved::Handle` whose OWN arms+body meet the single-handle
/// multi-shot-boundary condition (an inner multi-shot handler whose continuation reaches an op foreign to
/// IT — e.g. an outer handler's op). Bounded depth (a backstop; handler nesting is shallow).
fn body_has_nested_multishot_boundary(db: &mut Db, node: StructId, depth: u32) -> bool {
    if depth > 16 {
        return false;
    }
    if let Resolved::Handle { arms, body, .. } = resolved_of(db, node) {
        if one_handle_multishot_reaches_foreign(db, &arms, body) {
            return true;
        }
        if body_has_nested_multishot_boundary(db, body, depth + 1) {
            return true;
        }
    }
    match db.ast.get(node).clone() {
        Struct::List(children) => children
            .iter()
            .any(|&c| body_has_nested_multishot_boundary(db, c, depth + 1)),
        Struct::Atom(_) => false,
    }
}

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

/// Whether `node` DIRECTLY performs a FOREIGN op — one NOT in `discharged` (this handle's ops) and not
/// host-delegated. The `discharged`-set companion of [`next_state_directly_performs_foreign`] (which keys on
/// a built `HandlerCtx`), for use at arm-setup where the ctx is not yet built. PURE (no arena mutation).
fn expr_performs_foreign(
    db: &mut Db,
    node: StructId,
    discharged: &std::collections::HashSet<(u32, u32)>,
) -> bool {
    if let Resolved::Apply { head, .. } = resolved_of(db, node)
        && let Some((decl, idx)) = crate::eval::effect_op_of(db, head)
        && !discharged.contains(&(decl.0, idx))
        && perform_host_target(db, node, head).is_none()
    {
        return true;
    }
    match db.ast.get(node).clone() {
        Struct::List(children) => children
            .iter()
            .any(|&c| expr_performs_foreign(db, c, discharged)),
        Struct::Atom(_) => false,
    }
}

/// Rebuild `node`, replacing every DIRECT foreign perform (an `(Outer.op …)` of an op NOT in `discharged`,
/// not host-delegated) with a fresh ORDINARY `_cdz_ns{k}` NAME reference, recording each `(name,
/// original-perform-node)` in `binds` in left-to-right (evaluation) order. The pure companion of
/// [`hoist_next_state_foreign_perform`]: it turns `(+ t (A.get))` into `(+ t _cdz_ns0)` + binding `(_cdz_ns0
/// (A.get))`. The binder is an ORDINARY `_cdz_`-prefixed name (like `synth_binding_name`), NOT a `#`-prefixed
/// one: a `#`-name trips the fold's growing-state / `#cv`/`#st`/`#seed` heuristics and mis-threads the lifted
/// perform across dispatches (an early `#ns` attempt miscompiled as1 → 89; the ordinary name folds → 61,
/// matching the hand-written `let`-lift). The `_cdz_ns` prefix is unbindable-by-collision (no source uses it)
/// yet ordinary to the fold. Byte-identical (no new nodes) when the expression contains no foreign perform.
fn hoist_foreign_in_expr(
    db: &mut Db,
    node: StructId,
    discharged: &std::collections::HashSet<(u32, u32)>,
    binds: &mut Vec<(String, StructId)>,
) -> StructId {
    if let Resolved::Apply { head, .. } = resolved_of(db, node)
        && let Some((decl, idx)) = crate::eval::effect_op_of(db, head)
        && !discharged.contains(&(decl.0, idx))
        && perform_host_target(db, node, head).is_none()
    {
        let name = format!("_cdz_ns{}", binds.len());
        binds.push((name.clone(), node));
        return db.push_name(&name);
    }
    match db.ast.get(node).clone() {
        Struct::List(children) => {
            let rebuilt: Vec<StructId> = children
                .iter()
                .map(|&c| hoist_foreign_in_expr(db, c, discharged, binds))
                .collect();
            db.push_list(rebuilt)
        }
        Struct::Atom(_) => node,
    }
}

/// NEXT-STATE FOREIGN-PERFORM HOIST (as2/as1/asb fold, v-effects; operator directive: drive effects to 100%).
/// An inner handler arm whose tail resume NEXT-STATE performs an OUTER effect directly — `(resume t (+ t
/// (A.get)))` — is unsound to thread as-is: the next-state threads forward as a state EXPRESSION, so the
/// embedded `(A.get)` is DROPPED (as2 → 5 not 6) or DUPLICATED (as1), which is why
/// [`next_state_directly_performs_foreign`] DECLINES it. But that perform is a DISPATCH-TIME evaluation:
/// lifting it (and any perform in the resume VALUE, which must sequence FIRST) to `let`-inits before the
/// resume — `(let ((_cdz_ns0 (A.get))) (resume t (+ t _cdz_ns0)))` — runs it once per dispatch and threads
/// its PURE result, the proven-value-equivalent form (as2→6, as1→61, asb→57, all hand-verified). This
/// normalizes the arm UP FRONT (at [`reduce_handle`]'s arm setup), so the ordinary resumptive fold serves it.
///
/// TRIGGERS only when the NEXT-STATE performs a foreign op — so a foreign in the resume VALUE alone (as3,
/// served natively → 56) is UNTOUCHED (returns `None`, byte-identical). When it fires, it hoists the VALUE's
/// performs FIRST then the next-state's, preserving value-before-next-state evaluation order (asb → 57, not
/// the 67 a next-state-first order gives). Handles a BARE tail `(resume v ns)` arm body (as2/as1/asb); a
/// resume wrapped in `do`/`let`/`match` returns `None` and keeps declining (incremental-safe, no regression).
fn hoist_next_state_foreign_perform(
    db: &mut Db,
    arm_body: StructId,
    discharged: &std::collections::HashSet<(u32, u32)>,
) -> Option<StructId> {
    let (value, next_state) = tail_resume(db, arm_body)?;
    // TRIGGER: only intervene when the NEXT-STATE performs a foreign op. A pure next-state (as3, or an
    // ordinary state-threading arm) needs no hoist — leave it byte-identical.
    if !expr_performs_foreign(db, next_state, discharged) {
        return None;
    }
    // Hoist the VALUE's foreign performs FIRST (they sequence before the next-state's), then the next-state's,
    // into ONE binding list — so `_cdz_ns0…` are in value-then-next-state evaluation order.
    let mut binds: Vec<(String, StructId)> = Vec::new();
    let value2 = hoist_foreign_in_expr(db, value, discharged, &mut binds);
    let next_state2 = hoist_foreign_in_expr(db, next_state, discharged, &mut binds);
    let resume_head = db.push_name("resume");
    let new_resume = db.push_list(vec![resume_head, value2, next_state2]);
    let binding_nodes: Vec<StructId> = binds
        .into_iter()
        .map(|(name, node)| {
            let n = db.push_name(&name);
            db.push_list(vec![n, node])
        })
        .collect();
    let let_head = db.push_name("let");
    let bindings = db.push_list(binding_nodes);
    Some(db.push_list(vec![let_head, bindings, new_resume]))
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

    /// The compile-time metaprogramming effect `(effect Eval (op in-caller (-> Ast Ast)))` is INJECTED
    /// into every module before `scan_top_level`, so `Eval`/`in-caller` resolve everywhere (a macro's
    /// `{Eval}` row + `(in-caller …)` op). Here a trivial module with NO effect decl still gets it
    /// registered in `db.effect_decls`. DECL-only — no ambient handler — per the erasure model (the
    /// expander discharges `in-caller` at expansion + erases `{Eval}` before infer). See
    /// `inject_prelude_eval_effect`.
    #[test]
    fn prelude_eval_effect_is_injected_and_registered() {
        let ast = parse("(do (def (main) 1) (export main))");
        let db = crate::db::Db::load(ast);
        let eval = db
            .effect_decls
            .iter()
            .find(|e| e.name == "Eval")
            .expect("prelude Eval effect is injected + registered in every module");
        assert!(
            eval.ops.iter().any(|o| o.name == "in-caller"),
            "Eval carries the single in-caller op"
        );
    }

    /// The injection SKIPS a module that declares its OWN `Eval` effect (no duplicate) — the user's decl
    /// wins, so there is exactly one `Eval` in `effect_decls`.
    #[test]
    fn prelude_eval_injection_skips_a_user_declared_eval() {
        let ast = parse("(do (effect Eval (op ask (-> Unit Int64))) (def (main) 1) (export main))");
        let db = crate::db::Db::load(ast);
        let evals: Vec<_> = db
            .effect_decls
            .iter()
            .filter(|e| e.name == "Eval")
            .collect();
        assert_eq!(evals.len(), 1, "no duplicate Eval — the user decl wins");
        assert!(
            evals[0].ops.iter().any(|o| o.name == "ask"),
            "the surviving Eval is the USER's (op ask), not the injected in-caller"
        );
    }

    /// The injection only fires into a `(do …)`/`(module …)` top-level-ITEMS container — NOT a program
    /// whose ROOT is a BARE single form. Appending the Eval member to a bare `(def sig body)` root would
    /// corrupt it into a two-body def (`(def sig body (effect Eval …))`) → a spurious CDZ0201 "more than
    /// one body" on a valid single-def program (the #7859 regression: it broke the cdz lsp diagnostic-count
    /// tests + GHA). So a bare-def root gets NO injected Eval; the def is left intact (one body). Direct
    /// rcdzc-level guard for that invariant (the cdz lsp tests catch it only cross-crate). See
    /// `inject_prelude_eval_effect`.
    #[test]
    fn prelude_eval_injection_skips_a_bare_single_form_root() {
        let ast = parse("(def (main) 1)");
        let db = crate::db::Db::load(ast);
        assert!(
            db.effect_decls.iter().all(|e| e.name != "Eval"),
            "a bare single-def root is not a container — Eval must NOT be injected (else the def is \
             corrupted into a two-body def, #7859)"
        );
        // The single def survives intact with exactly one body — no spurious extra body from an append.
        let main = db
            .defs
            .iter()
            .find(|d| d.name == "main")
            .expect("the bare def is scanned as `main`");
        assert!(
            main.body.is_some(),
            "the bare def keeps its single body (not split/corrupted by an injected member)"
        );
    }

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
