//! DESTRUCTURING PARAMETERS — a `def` parameter that is an irrefutable PATTERN (`(def (f (tuple a b))
//! …)`) rather than a bare name, lowered by rewriting the def to a fresh whole-value parameter plus a
//! destructuring `let` over the body.
//!
//! The insight (DESIGN-binding-patterns-rcdzc.md §5.2): a binding position that holds an irrefutable
//! pattern is a single-arm match, and the `let` case ALREADY realizes exactly that — a `let` binder may
//! be a tuple pattern (`resolve::last_binder_named` routes an element reference to a `SumPayload` reading
//! the element out of the bound value; `lower::check_binding_pattern` validates it). So a destructuring
//! PARAMETER is reduced to the destructuring `let`:
//!
//! ```text
//! (def (f (tuple a b)) BODY)   →   (def (f p$0) (let (((tuple a b) p$0)) BODY))
//! ```
//!
//! `f` keeps ARITY 1 — the parameter is a fresh whole-pair name `p$0`, and the pattern moves into a `let`
//! that destructures it. A body reference to `a`/`b` ascends into the synthesized `let` (nearest binder)
//! and resolves to the `SumPayload` element read, exactly as if the program had written the `let` by
//! hand; `p$0` binds the whole argument the caller passes. This mirrors `accum::introduce`'s load-time
//! synthesis — fresh AST resolves through the ordinary scope walk, so no re-resolution scope corruption.
//!
//! Multiple pattern parameters nest their `let`s (outermost = the first parameter), so each destructures
//! its own fresh whole-value name. A bare-name / annotated `(: a T)` / wildcard parameter is left
//! untouched. A refutable / ill-shaped / non-linear pattern is NOT rejected here — the synthesized `let`
//! carries it to `check_binding_pattern`, which faults it with the binding-position code (CDZ0210 /
//! CDZ0201 / CDZ0102), so an ill-formed destructuring parameter declines-or-rejects rather than
//! miscompiling, through the one validation path the `let` case already owns.

use crate::ast::{Arenas, Leaf, Struct, StructId};
use crate::db::Def;
use crate::prelude::{push_atom, push_list};

/// Append a bare `Name` atom occurrence (a synthesized reference/binder).
fn push_name(ast: &mut Arenas, name: &str) -> StructId {
    push_atom(ast, Leaf::Name(name.into()))
}

/// The inner PATTERN of a parameter that this transform destructures, and (if it is an annotated
/// `(: <pattern> T)` parameter) the annotation type — else `None` for a bare name / `(: name T)` /
/// `()`. A destructuring parameter is a COMPOUND (list) pattern: a tuple pattern (the shape Increment A
/// destructures), or a constructor / record / list pattern (which the `let`-validation path then REJECTS
/// as refutable / DECLINES as not-yet-supported — an honest coded outcome, not the confusing
/// CDZ0101-unbound a non-rewritten compound param would give its binders).
///
/// An ANNOTATION `(: <inner> T)` is peeled: if its inner is itself a destructuring pattern
/// (`(: (tuple a b) T)`), the parameter destructures the inner pattern AND carries the annotation `T` —
/// returned as `Some((inner, Some(T)))`, so the rewrite keeps `T` on the fresh whole-value binder. A
/// `(: name T)` (inner is a bare name — a typed binder) is NOT destructured (`None`): its inner name IS
/// the real binder. A bare name / `()` is not a pattern (`None`).
fn destructure_pattern(ast: &Arenas, id: StructId) -> Option<(StructId, Option<StructId>)> {
    // A REST parameter `(.. binder)` (varargs, `DESIGN-variable-arity-functions.md` §2) is NOT a
    // destructuring pattern — it is a plain binder that receives the GATHERED trailing arguments as a
    // list (handled at application in `apply_lambda`). Leave it untouched (like a bare / `(: name T)`
    // param) so it is not rewritten into a `let` with a `(.. …)` binding the `let`-validation rejects.
    if ast.as_form(id, "..").is_some() {
        return None;
    }
    // An annotation `(: <inner> T)`: peel to the inner pattern. Destructure iff the inner is a compound
    // pattern (a list); a `(: name T)` typed binder is left alone.
    if let Some(tail) = ast.as_form(id, ":")
        && tail.len() == 2
    {
        let inner = tail[0];
        let ty = tail[1];
        return match ast.get(inner) {
            Struct::List(children) if !children.is_empty() => Some((inner, Some(ty))),
            _ => None, // `(: name T)` — a typed binder, not a destructure
        };
    }
    // A bare compound `(tuple a b)` (no annotation) is a destructuring pattern with no type.
    match ast.get(id) {
        Struct::List(children) if !children.is_empty() => Some((id, None)),
        _ => None, // a bare name atom / `()` — not a destructuring pattern
    }
}

/// Whether a parameter occurrence is a destructuring PATTERN this transform moves into a `let`.
fn is_destructuring_param(ast: &Arenas, id: StructId) -> bool {
    destructure_pattern(ast, id).is_some()
}

/// Run destructuring-parameter lowering over the module's `defs`, mutating `ast` (appending the
/// synthesized fresh-name parameters + `let` wrappers) and `defs` (rewriting a matched def's params +
/// body). Called at load AFTER `accum::introduce` and BEFORE the parent index / `def_by_name` are built,
/// so the rewritten def resolves like any hand-written one. A def with no pattern parameter is untouched.
pub(crate) fn lower(ast: &mut Arenas, defs: &mut [Def]) {
    // Collect the rewrites first (an immutable scan), then apply — the synthesis reads `defs` for a fresh
    // name, so a stable pre-scan avoids observing half-applied rewrites.
    let mut plans: Vec<usize> = Vec::new();
    for (i, d) in defs.iter().enumerate() {
        if d.body.is_some() && d.params.iter().any(|&p| is_destructuring_param(ast, p)) {
            plans.push(i);
        }
    }
    for def_ix in plans {
        apply(ast, defs, def_ix);
    }
    // A destructuring parameter on a `fn`/LAMBDA (`(fn ((tuple a b)) BODY)`) needs the SAME desugar, but a
    // lambda is an EXPRESSION node inside a def body, NOT in `defs`, so the loop above never reaches it —
    // its inner pattern names fell through the scope walk to a spurious CDZ0101 (breaker adv-51, a
    // core-semantics.md:135-137 violation: a `fn` parameter MUST accept an irrefutable pattern, exactly as
    // a `def` parameter and a `let` binder do). Walk each def's (now-rewritten) body subtree and rewrite
    // every `fn` node with a destructuring parameter IN PLACE to `(fn (p$k) (let (((pat) p$k)) BODY))`,
    // the lambda twin of the def-param rewrite. Runs at load before the parent index / resolve, so the
    // synthesized `let` resolves through the ordinary scope walk. Collect nodes first, then apply, for the
    // same stable-scan reason.
    let mut lambda_nodes: Vec<StructId> = Vec::new();
    for d in defs.iter() {
        if let Some(body) = d.body {
            collect_destructuring_lambdas(ast, body, &mut lambda_nodes);
        }
    }
    for fn_node in lambda_nodes {
        apply_lambda(ast, fn_node);
    }
}

/// Walk the subtree at `node`, appending every `(fn <params> body)` occurrence whose parameter list holds
/// at least one destructuring pattern. Recurses into all children (nested lambdas included). A `fn` node
/// that itself contains a destructuring lambda in its body is collected AND recursed, so an inner lambda
/// is rewritten too — `apply_lambda` only touches the outer `fn`'s own params, leaving inner nodes for
/// their own entry (the `StructId`s are stable; `apply_lambda` mutates a node's children in place, never
/// re-parents a descendant).
fn collect_destructuring_lambdas(ast: &Arenas, node: StructId, out: &mut Vec<StructId>) {
    if let Some(tail) = ast.as_form(node, "fn")
        && let Some(&params_occ) = tail.first()
        && let Struct::List(params) = ast.get(params_occ)
        && params.iter().any(|&p| is_destructuring_param(ast, p))
    {
        out.push(node);
    }
    // Recurse into children. `collect_destructuring_lambdas` borrows `ast` IMMUTABLY (no `&mut`), so the
    // `ast.get(node)` borrow and the recursive re-borrow coexist — iterate the child slice in place with
    // NO per-call Vec clone (the earlier `children.clone()` was an unnecessary copy, not a borrow need).
    if let Struct::List(children) = ast.get(node) {
        for &child in children {
            collect_destructuring_lambdas(ast, child, out);
        }
    }
}

/// Rewrite one `(fn <params> body)` node with at least one destructuring parameter IN PLACE: give each
/// pattern parameter a fresh whole-value name in the params list, and wrap the body in a destructuring
/// `let` per pattern parameter (innermost-first so the first parameter's binding is the outermost `let`).
/// The lambda twin of [`apply`]; a `fn` node's children are `[<params-list> <body>]` (no NAME, unlike a
/// def sig), so it edits children `[0]` (the params list) and `[1]` (the wrapped body) directly.
fn apply_lambda(ast: &mut Arenas, fn_node: StructId) {
    let Some(tail) = ast.as_form(fn_node, "fn") else {
        return;
    };
    let (Some(&params_occ), Some(&orig_body)) = (tail.first(), tail.get(1)) else {
        return;
    };
    let Struct::List(orig_params) = ast.get(params_occ) else {
        return;
    };
    let orig_params = orig_params.clone();

    let mut new_params: Vec<StructId> = Vec::with_capacity(orig_params.len());
    let mut bindings: Vec<(StructId, StructId)> = Vec::new(); // (pattern-occ, fresh-name-init-occ)
    for (k, &param) in orig_params.iter().enumerate() {
        if let Some((pattern, annot_ty)) = destructure_pattern(ast, param) {
            let fresh = format!("p${k}");
            let init_ref = push_name(ast, &fresh); // the `let` initializer (reads that parameter)
            // Keep a `(: <pattern> T)` annotation on the fresh whole-value binder as `(: p$k T)`, so the
            // argument is still checked against `T`; else a bare `p$k` name. Mirrors `apply`.
            let sig_name = if let Some(ty) = annot_ty {
                let name = push_name(ast, &fresh);
                let colon = push_name(ast, ":");
                push_list(ast, vec![colon, name, ty])
            } else {
                push_name(ast, &fresh)
            };
            new_params.push(sig_name);
            bindings.push((pattern, init_ref));
        } else {
            new_params.push(param);
        }
    }

    // Wrap the body in one `let` per pattern parameter, INNERMOST first (first param → outermost `let`).
    let mut body = orig_body;
    for (pattern, init_ref) in bindings.into_iter().rev() {
        let pair = push_list(ast, vec![pattern, init_ref]);
        let bindings_list = push_list(ast, vec![pair]);
        let let_head = push_name(ast, "let");
        body = push_list(ast, vec![let_head, bindings_list, body]);
    }

    let new_params_list = push_list(ast, new_params);
    // Swap the `fn` node's children in place: `(fn <new-params> <wrapped-body>)`. The head (child 0) stays.
    if let Struct::List(children) = &mut ast.structure[fn_node.0 as usize]
        && children.len() == 3
    {
        children[1] = new_params_list;
        children[2] = body;
    }
}

/// Rewrite one def with at least one tuple-pattern parameter: give each such parameter a fresh
/// whole-value name in the signature, and wrap the body in a destructuring `let` per pattern parameter.
fn apply(ast: &mut Arenas, defs: &mut [Def], def_ix: usize) {
    let sig_occ = defs[def_ix].sig_occ;
    let orig_params = defs[def_ix].params.clone();
    let Some(orig_body) = defs[def_ix].body else {
        return;
    };

    // Build the new parameter list occurrences (the signature children after the def NAME) and the
    // `(pattern fresh-name)` binding pairs, in parameter order. A bare/annotated parameter keeps its
    // occurrence; a tuple-pattern parameter becomes a fresh `p$k` name, and its pattern joins the
    // bindings that wrap the body.
    let mut new_params: Vec<StructId> = Vec::with_capacity(orig_params.len());
    let mut bindings: Vec<(StructId, StructId)> = Vec::new(); // (pattern-occ, fresh-name-init-occ)
    for (k, &param) in orig_params.iter().enumerate() {
        if let Some((pattern, annot_ty)) = destructure_pattern(ast, param) {
            let fresh = format!("p${k}");
            let init_ref = push_name(ast, &fresh); // the `let` initializer (reads that parameter)
            // The parameter slot is a fresh whole-value binder. If the original parameter carried a type
            // annotation `(: <pattern> T)`, keep it on the fresh binder as `(: p$k T)` — so the argument
            // is still checked against `T` (the annotation is not lost). Otherwise a bare `p$k` name.
            let sig_name = if let Some(ty) = annot_ty {
                let name = push_name(ast, &fresh);
                let colon = push_name(ast, ":");
                push_list(ast, vec![colon, name, ty])
            } else {
                push_name(ast, &fresh)
            };
            new_params.push(sig_name);
            // The destructuring `let` binds the INNER pattern (the tuple, un-annotated) to the fresh name.
            bindings.push((pattern, init_ref));
        } else {
            new_params.push(param);
        }
    }

    // Wrap the body in one `let` per pattern parameter, INNERMOST first so the first parameter's binding
    // is the OUTERMOST `let` (its binders are in scope for the whole body, including the later `let`s).
    //   (let ((<pat_k> p$k)) body)   nested outward over k = n-1 … 0.
    let mut body = orig_body;
    for (pattern, init_ref) in bindings.into_iter().rev() {
        let pair = push_list(ast, vec![pattern, init_ref]);
        let bindings_list = push_list(ast, vec![pair]);
        let let_head = push_name(ast, "let");
        body = push_list(ast, vec![let_head, bindings_list, body]);
    }

    // Rebuild the signature `(NAME new_param…)` in the arena and swap it + the wrapped body into the def
    // FORM, so a body reference resolves by ascending to the rewritten `(def sig body)` (resolve Case 4
    // reads the form's signature + body children).
    let name_occ = match ast.get(sig_occ) {
        Struct::List(children) => children.first().copied(),
        _ => None,
    };
    let Some(name_occ) = name_occ else {
        return;
    };
    let mut sig_children = vec![name_occ];
    sig_children.extend(new_params.iter().copied());
    let new_sig = push_list(ast, sig_children);

    // Swap the def form's signature (child 0) and body (child 1) in place. The def FORM is the parent of
    // `sig_occ`; the transform runs before the parent index, so locate it by scanning the top items (as
    // `accum` does). `(def <old-sig> <old-body>)` becomes `(def <new-sig> <wrapped-body>)`.
    if let Some(def_form) = find_def_form(ast, sig_occ)
        && let Struct::List(children) = &mut ast.structure[def_form.0 as usize]
        && children.len() == 3
    {
        children[1] = new_sig;
        children[2] = body;
    }

    // Update the `Def` record to match the rewritten form.
    defs[def_ix].sig_occ = new_sig;
    defs[def_ix].params = new_params;
    defs[def_ix].body = Some(body);
}

/// The top-level `(def sig …)` FORM occurrence whose signature is `sig_occ`. Scans the module's items
/// (the transform runs before the parent index). `None` if the def is not directly under
/// `(module …)`/`(do …)` (then the rewrite is skipped, leaving it untouched). Mirrors `accum`'s helper.
fn find_def_form(ast: &Arenas, sig_occ: StructId) -> Option<StructId> {
    let items = match ast.get(ast.root) {
        Struct::List(top) => match ast.as_name(*top.first()?) {
            Some("module") => top.get(2..).unwrap_or(&[]).to_vec(),
            Some("do") => top[1..].to_vec(),
            _ => vec![ast.root],
        },
        _ => return None,
    };
    items
        .into_iter()
        .find(|&item| ast.as_form(item, "def").and_then(|t| t.first().copied()) == Some(sig_occ))
}
