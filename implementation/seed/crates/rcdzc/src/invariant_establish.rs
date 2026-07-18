//! DATA-TYPE INVARIANT ESTABLISH — Part 1: synthesize a TYPED predicate-check def per `@invariant` type.
//!
//! The DATA-level verification-family member (design §10). An `@invariant(PRED)` on a type `T` states a
//! property every value of `T` maintains, with the value bound to `it`. ESTABLISH is the obligation that
//! every constructor of `T` produces a value satisfying `PRED`.
//!
//! ## What THIS pass does (Part 1)
//!
//! For each type declared with an `@invariant` — `(@ (invariant PRED) (type T …))` — synthesize a checker:
//!
//! ```text
//! (def (__invariant_check_T (: it T)) <CHECK-BODY>)
//! ```
//!
//! appended to the top-level item list. Because it flows through the ordinary resolve/infer/lower like any
//! hand-written def (the `proptest_gen` synthesis discipline), the predicate is now RESOLVED + TYPE-CHECKED
//! with `it : T` in scope — which it never was as the stripped recorded predicate. It is also the CALLEE the
//! construct-site establish check (Part 2) will invoke.
//!
//! ## `it` binds the WHOLE value; a SINGLE-PAYLOAD NEWTYPE AUTO-UNWRAPS (ruling 2026-07-18)
//!
//! `it` is the whole value of `T`. For a SINGLE-VARIANT, SINGLE-PAYLOAD newtype `(type T (V U))`, a bare
//! scalar predicate `(>= it 0)` would hit the nominal boundary (`it : T` is not comparable to an `Int64`
//! literal — CDZ0202). So the checker AUTO-UNWRAPS: the CHECK-BODY is
//!
//! ```text
//! (match it (((. T V) __inv_u) PRED[it := __inv_u]))
//! ```
//!
//! — destructure the sole variant, binding its payload, and rewrite every `it` in PRED to that payload. This
//! realizes the design's "Percent-as-newtype-over-Int64: T IS the scalar, `(>= it 0)` typechecks" intent
//! faithfully within the nominal-newtype grammar — the author writes the bare form (or an accessor
//! `(> (len it) 0)`, which likewise needs the underlying `(List …)`), the checker transparently unwraps `it`
//! to the payload, so the nominal boundary never bites (and v-property-testing's generator keeps reading the
//! bare `it` shape from the raw AST, unaffected by this transform).
//!
//! **A predicate that ALREADY destructures `it` itself is NOT rewritten.** If PRED contains a `(match it …)`
//! (the author unwraps the whole value directly — `(match it (((. T V) v) …))`), rewriting `it` → the payload
//! would corrupt it (the inner match would try to destructure the payload, not the value). So the auto-unwrap
//! only applies when PRED does NOT self-destructure `it`; a self-destructuring PRED uses `it` (the whole
//! value) directly. Both forms are thus valid: bare/accessor (auto-unwrapped) and self-destructure (as-is).
//!
//! A type that is NOT a single-payload newtype (a multi-variant sum, a nullary/multi-payload variant, or a
//! record) does NOT auto-unwrap — the predicate uses `it` directly via an accessor / its own match. (Only the
//! single-payload newtype has the transparent-scalar reading the bare form needs.)
//!
//! The check-body's PRED is a COPY of the recorded node (the `@invariant` wrapper's PRED stays intact for
//! `strip_annotations` → `db.invariants`). Runs at load BEFORE `strip_annotations` (wrapper still present) +
//! `scan_top_level` (the synthesized def scans like hand-written source); all appended nodes are ordinary AST.

use crate::ast::{Arenas, Leaf, Struct, StructId};
use crate::prelude::{push_atom, push_list};

fn name(ast: &mut Arenas, n: &str) -> StructId {
    push_atom(ast, Leaf::Name(n.to_string()))
}

/// The fresh payload-binder name the auto-unwrap introduces — chosen not to collide with a user name
/// (a `__`-prefixed name the reader would not write; and never `it`, which stays in scope for accessor forms).
const UNWRAP_BINDER: &str = "__inv_u";

/// Deep-copy a predicate subtree, rewriting every bare-name occurrence of `from` to `to` (and re-pushing
/// other names as fresh occurrences so they resolve in the def's scope). Non-name leaves are shared.
/// `from`/`to` empty ⇒ a plain copy (no rewrite).
fn copy_rewrite(ast: &mut Arenas, node: StructId, from: &str, to: &str) -> StructId {
    match ast.get(node).clone() {
        Struct::Atom(lid) => {
            let leaf = ast.leaf(lid).clone();
            if let Leaf::Name(n) = &leaf {
                let n = if !from.is_empty() && n == from {
                    to.to_string()
                } else {
                    n.clone()
                };
                return push_atom(ast, Leaf::Name(n));
            }
            node
        }
        Struct::List(children) => {
            let copied: Vec<StructId> = children
                .iter()
                .map(|&c| copy_rewrite(ast, c, from, to))
                .collect();
            push_list(ast, copied)
        }
    }
}

/// True if `node` (a predicate subtree) contains a `(match it …)` whose scrutinee is the bare name `it` —
/// i.e. the author already destructures the whole value. When so, auto-unwrap must NOT rewrite `it` → the
/// payload (that would corrupt the author's match). A conservative syntactic scan.
fn self_destructures_it(ast: &Arenas, node: StructId) -> bool {
    if let Some(mtail) = ast.as_form(node, "match")
        && let Some(&scrut) = mtail.first()
        && ast.as_name(scrut) == Some("it")
    {
        return true;
    }
    if let Struct::List(children) = ast.get(node) {
        return children.iter().any(|&c| self_destructures_it(ast, c));
    }
    false
}

/// If `type_decl_tail` (the children of a `(type …)` AFTER the head, i.e. `[NAME, variant…]`) describes a
/// SINGLE-VARIANT, SINGLE-PAYLOAD newtype `(type T (V U))`, return `(V-name)` — the sole variant's ctor name.
/// Returns `None` for a nullary variant, a multi-payload variant, a multi-variant sum, or a record.
fn sole_newtype_variant_name(ast: &Arenas, type_decl_tail: &[StructId]) -> Option<String> {
    // `[NAME, variant]` — exactly one variant after the type name.
    let [_name, variant] = type_decl_tail else {
        return None;
    };
    // The variant is `(V U)` — a list whose head is the ctor name and with exactly ONE payload element.
    let Struct::List(items) = ast.get(*variant) else {
        return None;
    };
    if items.len() != 2 {
        return None; // nullary (`(V)`) or multi-payload (`(V A B)`) — not a single-payload newtype
    }
    ast.as_name(items[0]).map(str::to_string)
}

/// Synthesize a `__invariant_check_<T>` def per `@invariant`-annotated type + append to the top-level items.
/// A no-op for a program with no `@invariant`. See the module docs.
pub(crate) fn synthesize(ast: &mut Arenas) {
    let root = ast.root;
    let prefix_len = if ast.as_form(root, "do").is_some() {
        1
    } else if ast.as_form(root, "module").is_some() {
        2
    } else {
        return;
    };
    let Struct::List(root_children) = ast.get(root).clone() else {
        return;
    };
    if root_children.len() <= prefix_len {
        return;
    }

    // Scan ORIGINAL nodes for `(@ (invariant PRED) (type T …))`. Collect (type-name, sole-newtype-variant?,
    // PRED). Bounded to the pre-pass length so the appended checker defs are not re-scanned.
    let original_len = ast.structure.len() as u32;
    let mut plans: Vec<(String, Option<String>, StructId)> = Vec::new();
    for i in 0..original_len {
        let id = StructId(i);
        let Some(tail) = ast.as_form(id, "@") else {
            continue;
        };
        let (Some(&name_occ), Some(&inner)) = (tail.first(), tail.get(1)) else {
            continue;
        };
        let Some(&pred) = ast.as_form(name_occ, "invariant").and_then(|t| match t {
            [only] => Some(only),
            _ => None,
        }) else {
            continue;
        };
        let Some(type_tail) = ast.as_form(inner, "type").map(<[_]>::to_vec) else {
            continue;
        };
        let Some(type_name) = type_tail
            .first()
            .and_then(|&n| ast.as_name(n))
            .map(str::to_string)
        else {
            continue;
        };
        let sole_variant = sole_newtype_variant_name(ast, &type_tail);
        plans.push((type_name, sole_variant, pred));
    }
    if plans.is_empty() {
        return;
    }

    // Build one checker def per plan.
    let mut new_defs: Vec<StructId> = Vec::with_capacity(plans.len());
    for (type_name, sole_variant, pred) in plans {
        // CHECK-BODY: for a single-payload newtype whose predicate does NOT self-destructure `it`,
        // AUTO-UNWRAP — `(match it (((. T V) __inv_u) PRED[it:=__inv_u]))`. If PRED already destructures `it`
        // (or the type is not a single-payload newtype), use the predicate over `it` directly.
        let auto_unwrap = sole_variant
            .as_ref()
            .is_some_and(|_| !self_destructures_it(ast, pred));
        let check_body = match sole_variant {
            Some(variant) if auto_unwrap => {
                let body_pred = copy_rewrite(ast, pred, "it", UNWRAP_BINDER);
                // pattern `(. T V)` — the qualified ctor; binds the payload to `__inv_u`.
                let ctor_pat = {
                    let dot = name(ast, ".");
                    let ty = name(ast, &type_name);
                    let v = name(ast, &variant);
                    push_list(ast, vec![dot, ty, v])
                };
                let payload = name(ast, UNWRAP_BINDER);
                let full_pat = push_list(ast, vec![ctor_pat, payload]);
                let arm = push_list(ast, vec![full_pat, body_pred]);
                let match_head = name(ast, "match");
                let it_scrut = name(ast, "it");
                push_list(ast, vec![match_head, it_scrut, arm])
            }
            // No auto-unwrap: `None` (not a single-payload newtype) OR a self-destructuring PRED (the author
            // already unwraps `it`). Use the predicate over `it` directly (a plain copy).
            _ => copy_rewrite(ast, pred, "", ""),
        };
        let sig = {
            let fn_name = name(ast, &format!("__invariant_check_{type_name}"));
            let param = {
                let colon = name(ast, ":");
                let it = name(ast, "it");
                let ty = name(ast, &type_name);
                push_list(ast, vec![colon, it, ty])
            };
            push_list(ast, vec![fn_name, param])
        };
        let def_head = name(ast, "def");
        new_defs.push(push_list(ast, vec![def_head, sig, check_body]));
    }

    // Rebuild the root, appending the checker defs after the existing items (ids stay stable — append-only).
    let mut children: Vec<StructId> = root_children;
    children.extend(new_defs);
    let new_root = push_list(ast, children);
    ast.root = new_root;
}
