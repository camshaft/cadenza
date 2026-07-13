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
    push_atom(ast, Leaf::Name(name.to_string()))
}

/// Whether a parameter occurrence is a destructuring PATTERN that this transform moves into a `let` — a
/// COMPOUND (list) parameter that is NOT an annotation `(: name T)`. That is a tuple pattern (the one
/// shape Increment A destructures), OR a constructor / record / list pattern (which the `let`-validation
/// path then REJECTS as refutable / DECLINES as not-yet-supported — either way an honest coded outcome,
/// not the confusing CDZ0101-unbound a non-rewritten compound param would give its binders).
///
/// A bare-name parameter (`a`), an annotated binder (`(: a T)`), and the empty list are NOT destructured:
/// a bare name is the trivial binder, an annotation is the parameter's type constraint (its inner name is
/// the real binder), and `()` is the unit value.
fn is_destructuring_param(ast: &Arenas, id: StructId) -> bool {
    match ast.get(id) {
        Struct::List(children) if !children.is_empty() => {
            // An annotation `(: name T)` is a typed binder, not a destructure — leave it alone.
            ast.as_form(id, ":").is_none()
        }
        _ => false, // a bare name atom / `()` — not a destructuring pattern
    }
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
        if is_destructuring_param(ast, param) {
            let fresh = format!("p${k}");
            let sig_name = push_name(ast, &fresh); // the parameter slot (a whole-value binder)
            let init_ref = push_name(ast, &fresh); // the `let` initializer (reads that parameter)
            new_params.push(sig_name);
            bindings.push((param, init_ref));
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
