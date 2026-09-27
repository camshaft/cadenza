//! Pre-resolution desugar of a leading `try` value-def in a `do` block to an equivalent `let`.
//!
//! A two-form `(do (def x (try e)) body)` is SEMANTICALLY a `(let ((x (try e))) body)` — a do-local
//! value declaration is scoped to the forms that follow it, so a leading value-def whose only successor
//! is the body IS a `let` binding whose body is that form. Both spellings route through the same BRICK-3b
//! runtime-`?` `lower_let` short-circuit and build the SAME `Core::MatchSum` boundary.
//!
//! WHY REWRITE IT — the do-def form is FRAGILE under INLINING. When the enclosing function is β-copied at
//! a call site, a do-local reference resolves via `resolve::do_local_binds`' scope-ascent, and under a
//! NESTED inline that ascent can land on the ORIGINAL do-block rather than the copy — so the copied
//! `try`'s BRICK-3b `core_override` (a `SumPayload` on the copied init) is missed, the copied MatchSum's
//! success-arm body re-lowers the reference through the generic `Resolved::Try` path, and the boundary
//! re-declines `CDZ0900`. That decline is a SAFE FLOOR over a latent miscompile: naively un-declining it
//! (returning the payload read) drops the short-circuit and reads a disconnected scrutinee on the failure
//! leg (`23-try-operator.sexp` tdd1: `main`'s `n=5` leg returned `0` instead of `-1`). A `let` binding's
//! reference, by contrast, resolves ROBUSTLY through the β-copy (its binder is recognized structurally by
//! `resolve::binder_in` Cases 1/2), so the `let` twin compiles AND runs correctly whether inlined or not.
//!
//! Rewriting the `try` do-def to a `let` makes the `try` shape INLINING-INVARIANT (v-core-opt's endorsed
//! direction — order/inline-dependence IS the fragility), producing the identical MatchSum whether inlined
//! or not. TRY-SPECIFIC: only a `(def x (try …))` value-def with exactly a body successor is rewritten, so
//! a non-`try` do-def keeps the do-fold's copy-propagation path unchanged and the blast radius is minimal.
//!
//! Runs at LOAD, BEFORE the parent index and resolution (alongside `reify_quotes` / `desugar_eval`), so the
//! rewritten `let` resolves like hand-written source.

use crate::ast::{Arenas, Leaf, LeafId, Struct, StructId};
use crate::prelude::{push_atom, push_list};

/// The binding/control heads a hoist of an expression-position `?` MUST NOT cross (BRICK 3 slice 1,
/// `DESIGN-try-operator-rcdzc.md` §7.1). Hoisting a `(try e)` out past one of these would change scoping
/// (`let`/`do`/`fn`/`def`/`module`) or conditional evaluation (`if`/`match`/`handle`/`loop`) — so the
/// downward search STOPS here, leaving such a `?` to `lower_let` (binding-tail) or a later slice
/// (control-flow, which must distribute the `?` into the arm's continuation). Also stops at the special
/// forms whose interior is not an ordinary evaluated sub-expression (`quote`/`eval`/`effect`/`type`).
fn is_boundary_or_control_head(h: &str) -> bool {
    matches!(
        h,
        "let"
            | "do"
            | "match"
            | "if"
            | "handle"
            | "handle-abort"
            | "fn"
            | "def"
            | "quote"
            | "eval"
            | "effect"
            | "module"
            | "type"
            | "loop"
    )
}

/// Whether `node` is a bare ATOM — a literal or a name reference. Such a node has NO observable effect and
/// cannot trap, so REORDERING its evaluation (which hoisting a later `?` before it does) is unobservable.
/// A `List` (an application) is conservatively treated as impure (it may call / perform / trap).
fn is_pure_atom(ast: &Arenas, node: StructId) -> bool {
    matches!(ast.get(node), Struct::Atom(_))
}

/// Find a single hoistable expression-position `(try e)` reachable from `node`, or `None`. Descends only
/// through ASCRIPTIONS and pure APPLICATIONS/CONSTRUCTORS (a name head not in the stop-set), and only past
/// an argument once every argument evaluated BEFORE it is a pure atom — so the `?` it returns can be lifted
/// to a boundary `let` WITHOUT reordering any observable effect or trap (BRICK 3 slice 1 gate, §7.1). Stops
/// at binding/control heads (`is_boundary_or_control_head`) so a binding-tail `?` (handled by `lower_let`)
/// and a control-flow `?` (a later slice) are left untouched, and at a non-name head (an applied lambda).
fn find_hoistable_try(ast: &Arenas, node: StructId) -> Option<StructId> {
    let Struct::List(kids) = ast.get(node) else {
        return None;
    };
    if kids.is_empty() {
        return None;
    }
    let kids: Vec<StructId> = kids.clone();
    let hname = ast.as_name(kids[0]);
    // `(try e)` — the node to hoist (exactly the arity-1 operator form).
    if hname == Some("try") && kids.len() == 2 {
        return Some(node);
    }
    // A binding/control head, or a non-name head (an applied lambda / compound-ctor form not yet handled):
    // do not cross it.
    match hname {
        Some(h) if !is_boundary_or_control_head(h) => {}
        _ => return None,
    }
    // A pure application / constructor / ascription: descend into the FIRST argument (left-to-right, so the
    // scrutinee-first evaluation order is preserved) that contains a hoistable `?`, provided every earlier
    // argument is a pure atom.
    for i in 1..kids.len() {
        if let Some(tn) = find_hoistable_try(ast, kids[i]) {
            return if kids[1..i].iter().all(|&a| is_pure_atom(ast, a)) {
                Some(tn)
            } else {
                // An earlier argument is a non-atom (may call/perform/trap) — hoisting the `?` before it
                // would reorder that effect. Decline the hoist (safe floor); a later slice with real
                // effect/trap analysis can relax this.
                None
            };
        }
    }
    None
}

/// Hoist a single EXPRESSION-position `?` in each fallible-boundary function body to a boundary `let`
/// (BRICK 3 slice 1, `DESIGN-try-operator-rcdzc.md` §7.1): `C[(try e)]` => `(let ((x (try e))) C[x])`.
/// The resulting binding-tail `let` rides the proven `lower_let` runtime-`?` `Core::MatchSum` short-circuit
/// (the same path tdd1 / stored-closure use) — no `Core::Block`/`Break` and no new emit, and it is
/// INLINE-SAFE because the `match` is local (an emit-time boundary-block wrap is NOT, since inlining moves
/// the `?` into a caller with a different result type). Runs at LOAD, AFTER `desugar_try_do_defs` (so a
/// binding-tail do-def is already a `let` the search stops at) and before resolution.
pub fn desugar_try_expr_position(ast: &mut Arenas) {
    // FAST BAIL: no `(try …)` form anywhere (the common case) → no interned `try` name leaf.
    if !ast
        .leaves
        .iter()
        .any(|l| matches!(l, Leaf::Name(n) if n.as_ref() == "try"))
    {
        return;
    }
    let original_len = ast.structure.len() as u32;
    // Each fallible-boundary body is a `def`/`fn` node's LAST child (`(def target body)` / `(fn params
    // body)`, both arity-3). Plan `(body_node, try_node)` for each body with a hoistable expression-position
    // `?`; only ORIGINAL nodes are boundary bodies (the rewrite APPENDS the `let` scaffolding).
    let mut plans: Vec<(StructId, StructId)> = Vec::new();
    for i in 0..original_len {
        let id = StructId(i);
        let Struct::List(kids) = ast.get(id) else {
            continue;
        };
        if kids.len() != 3 {
            continue;
        }
        let is_def_or_fn = matches!(ast.as_name(kids[0]), Some("def") | Some("fn"));
        if !is_def_or_fn {
            continue;
        }
        let body = kids[2];
        if let Some(tn) = find_hoistable_try(ast, body) {
            plans.push((body, tn));
        }
    }
    for (body, tn) in plans {
        // `tn` = `(try e)`; capture its head + operand before overwriting it in place.
        let Struct::List(tn_kids) = ast.get(tn) else {
            continue;
        };
        if tn_kids.len() != 2 {
            continue;
        }
        let try_head = tn_kids[0];
        let operand = tn_kids[1];
        // A fresh binder name, unique per `?` node (its original StructId index).
        let xname: std::sync::Arc<str> = format!("__try_hoist_{}", tn.0).into();
        // The let-INIT `(try e)` — a fresh node reusing the original head + operand (so the operand subtree
        // is shared, not re-copied).
        let try_init = push_list(ast, vec![try_head, operand]);
        // The let-binding NAME atom.
        let binder = push_atom(ast, Leaf::Name(xname.clone()));
        // Overwrite `tn` IN PLACE to a NAME reference to the binder — its parent (the enclosing application)
        // already points at `tn`, so this repoints the continuation's use to `x` with no parent surgery.
        let ref_lid = LeafId(ast.leaves.len() as u32);
        ast.leaves.push(Leaf::Name(xname));
        ast.structure[tn.0 as usize] = Struct::Atom(ref_lid);
        // Wrap the boundary body: move its current struct to a fresh `b_inner` node (the continuation `C[x]`,
        // now carrying the `x` reference), then overwrite the body node IN PLACE to `(let ((x (try e)))
        // b_inner)` — preserving the body's StructId/span as the `let`, mirroring `desugar_try_do_defs`.
        let b_struct = ast.structure[body.0 as usize].clone();
        let b_inner = StructId(ast.structure.len() as u32);
        ast.structure.push(b_struct);
        let let_head = push_atom(ast, Leaf::Name("let".into()));
        let pair = push_list(ast, vec![binder, try_init]);
        let bindings = push_list(ast, vec![pair]);
        ast.structure[body.0 as usize] = Struct::List(vec![let_head, bindings, b_inner]);
    }
}

/// Rewrite every two-form `(do (def x (try e)) body)` in `ast` to `(let ((x (try e))) body)` in place.
pub fn desugar_try_do_defs(ast: &mut Arenas) {
    // FAST BAIL for a program with no `(try …)` form (the overwhelming common case). A `(def x (try e))`
    // needs a `try` head, which is a `Leaf::Name("try")` in the interned leaf pool; if none exists, no
    // `try` form exists anywhere and the whole scan is dead work. A single O(leaves) prescan (leaves are
    // interned once, far fewer than a per-node structural probe) over-approximates: it falls through
    // spuriously only for a program that MENTIONS the identifier `try` without a `(try …)` form (e.g. a
    // user def named `try`), which then runs the exact shape scan below — same result, just not skipped.
    // Sibling of the `quote::reify_quotes` / `eval_ast::desugar_eval` fast-bails.
    if !ast
        .leaves
        .iter()
        .any(|l| matches!(l, Leaf::Name(n) if n.as_ref() == "try"))
    {
        return;
    }
    // Only ORIGINAL nodes can be a source do-block; the rewrite APPENDS the `let` scaffolding (ids >= this
    // bound) and overwrites the do node in place, so the scan must not consider its own output.
    let original_len = ast.structure.len() as u32;
    // Plan `(do_node, def_form, x_name, try_node, body)` for each two-form `(do (def x (try e)) body)`.
    let mut plans: Vec<(StructId, StructId, StructId, StructId, StructId)> = Vec::new();
    for i in 0..original_len {
        let id = StructId(i);
        // A `(do …)` block with EXACTLY two tail forms — a value-def and a body. A do-block with other
        // statements (an effect before/after the def) is NOT a simple `let` (the extra forms are sequenced
        // for effect), so it is left to the ordinary do-fold; only the clean two-form idiom is rewritten.
        let Some(tail) = ast.as_form(id, "do") else {
            continue;
        };
        let [def_form, body] = tail else {
            continue;
        };
        let (def_form, body) = (*def_form, *body);
        // The first form must be a bare-name value-def `(def x V)` whose value `V` is a `(try …)` form.
        let Some(def_tail) = ast.as_form(def_form, "def") else {
            continue;
        };
        let [x_name, v] = def_tail else {
            continue;
        };
        let (x_name, v) = (*x_name, *v);
        // `x` is a bare NAME atom (a value-def) — not a `(f p…)` function signature (a List head).
        if ast.as_name(x_name).is_none() {
            continue;
        }
        // `V` is a `(try e)` form (the whole point — a non-`try` value-def is left alone).
        if ast.as_form(v, "try").is_none() {
            continue;
        }
        plans.push((id, def_form, x_name, v, body));
    }
    for (do_node, def_form, x_name, try_node, body) in plans {
        // Build `(let ((x (try e))) body)`, REUSING the original `x_name` / `try_node` / `body` occurrences
        // so their subtrees (and the `try`'s operand) are shared, not re-copied.
        let pair = push_list(ast, vec![x_name, try_node]); // the binding pair `(x (try e))`
        let bindings = push_list(ast, vec![pair]); //            the bindings list `((x (try e)))`
        let let_head = push_atom(ast, Leaf::Name("let".into()));
        // Overwrite the `(do …)` node IN PLACE, preserving its `StructId`/span as the `let`'s node.
        ast.structure[do_node.0 as usize] = Struct::List(vec![let_head, bindings, body]);
        // Blank the now-dead `(def x (try e))` wrapper: `x_name` and `try_node` were re-parented into the
        // binding pair above, and the parent index (built later) records the LAST parent per child — leaving
        // the def form would out-rank the pair as their recorded parent and orphan the binder. Emptying it
        // leaves the binding pair the sole parent of `x_name`/`try_node` (mirrors the dead-wrapper blanking
        // in `reify_quotes` / `desugar_eval`).
        ast.structure[def_form.0 as usize] = Struct::List(Vec::new());
    }
}
