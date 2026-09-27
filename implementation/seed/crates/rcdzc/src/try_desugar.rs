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

use crate::ast::{Arenas, CompoundCtor, Leaf, LeafId, Struct, StructId};
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

/// Find a single hoistable expression-position `(try e)` reachable from `node`, and the impure operands
/// evaluated BEFORE it (in evaluation order) that must be let-bound to preserve that order. Returns
/// `(try_node, impure_prefix)`, or `None`. Descends through ASCRIPTIONS and APPLICATIONS/CONSTRUCTORS (a
/// name head not in the stop-set), into the FIRST argument (left-to-right, so `match`'s scrutinee-first
/// order is preserved) that contains a `?`. Every EARLIER argument is evaluated before the `?`, so a
/// PURE-ATOM one (literal / bare name — no effect, no trap) is inlined unchanged, and a NON-ATOM one (may
/// call / perform / trap) is collected into `impure_prefix` to be bound to its own `let` ahead of the `?`
/// (`hoist_try_at`) — preserving evaluation order and any trap/effect timing relative to the short-circuit
/// (BRICK 3 slices 1/2b, `DESIGN-try-operator-rcdzc.md` §7.1). Stops at binding/control heads
/// (`is_boundary_or_control_head`) — a binding-tail `?` is `lower_let`'s, a control-flow arm is descended by
/// `collect_tail_positions`, not here — and at a non-name head (an applied lambda / compound-ctor form).
fn find_hoistable_try(ast: &Arenas, node: StructId) -> Option<(StructId, Vec<StructId>)> {
    let Struct::List(kids) = ast.get(node) else {
        return None;
    };
    if kids.is_empty() {
        return None;
    }
    let kids: Vec<StructId> = kids.clone();
    let hname = ast.as_name(kids[0]);
    // `(try e)` — the node to hoist (exactly the arity-1 operator form); no prefix at this level.
    if hname == Some("try") && kids.len() == 2 {
        return Some((node, Vec::new()));
    }
    // Descendable into its arguments/elements: a non-control NAME head (an application / constructor /
    // ascription), OR a FLAT compound-ctor head — `#list`/`#tuple`/`#set`, whose head is a `Leaf::Ctor`
    // (not a name), a pure container whose elements evaluate left-to-right exactly like operator operands.
    // A `record`/`map` (paired `(= k v)` entries) or a non-name applied-lambda head is NOT descended.
    let descendable = match hname {
        Some(h) => !is_boundary_or_control_head(h),
        None => matches!(
            ast.compound_ctor_leaf(node),
            Some(CompoundCtor::List | CompoundCtor::Tuple | CompoundCtor::Set)
        ),
    };
    if !descendable {
        return None;
    }
    // Descend into the FIRST argument containing a `?`. Every earlier argument (positions `1..i`) is
    // evaluated before it; collect the NON-ATOM ones as the impure prefix to bind (OUTER level first), then
    // append the inner prefix from the recursive descent (inner level after).
    for i in 1..kids.len() {
        if let Some((tn, inner_prefix)) = find_hoistable_try(ast, kids[i]) {
            let mut prefix: Vec<StructId> = kids[1..i]
                .iter()
                .copied()
                .filter(|&a| !is_pure_atom(ast, a))
                .collect();
            prefix.extend(inner_prefix);
            return Some((tn, prefix));
        }
    }
    None
}

/// Collect the hoist candidates of `node` as `(target, search_root)` pairs — where `search_root` is
/// searched for a hoistable `?` and `target` is the node WRAPPED in the boundary `let` (BRICK 3 slices
/// 2a/2c, `DESIGN-try-operator-rcdzc.md` §7.1). Descends the tail slots of control/binding forms — an
/// ascription's expr, an `if`'s two arms, a `match`'s arm bodies, a `let`'s body, a `do`'s LAST form — each
/// yielding a tail LEAF as `(leaf, leaf)` (target == search_root). PLUS: for a `(do (def name V) …)` whose
/// FIRST form is a VALUE-def, `V` is a search_root with `target = the do` — a `?` in the def value
/// short-circuits the WHOLE do (its value becomes the boundary's), sound because the def is FIRST so no
/// preceding statement's effect is reordered. A `?` in a NON-tail slot (an `if` condition, a `match`
/// scrutinee, a `do` non-last/non-first-def statement, a `let` init) is NOT collected — it is `lower_let`'s
/// (a binding-tail init) or a later slice's, a clean decline.
fn collect_tail_positions(ast: &Arenas, node: StructId, out: &mut Vec<(StructId, StructId)>) {
    let Struct::List(kids) = ast.get(node) else {
        out.push((node, node));
        return;
    };
    if kids.is_empty() {
        out.push((node, node));
        return;
    }
    let kids: Vec<StructId> = kids.clone();
    match ast.as_name(kids[0]) {
        // `(: expr T)` — expr is the tail (T is a type, not evaluated).
        Some(":") if kids.len() == 3 => collect_tail_positions(ast, kids[1], out),
        // `(if cond then else)` — both arms are tail; the condition is not.
        Some("if") if kids.len() == 4 => {
            collect_tail_positions(ast, kids[2], out);
            collect_tail_positions(ast, kids[3], out);
        }
        // `(match scrut (pat body)…)` — each 2-form arm's BODY is tail; the scrutinee is not. A non-2-form
        // arm (guard / multi-form) is left untouched (conservative — stays a clean decline).
        Some("match") if kids.len() >= 3 => {
            for &arm in &kids[2..] {
                if let Struct::List(ak) = ast.get(arm)
                    && ak.len() == 2
                {
                    let body = ak[1];
                    collect_tail_positions(ast, body, out);
                }
            }
        }
        // `(let bindings body)` — the body is tail; a binding INIT `?` is `lower_let`'s (binding-tail), not
        // collected here.
        Some("let") if kids.len() == 3 => collect_tail_positions(ast, kids[2], out),
        // `(do stmt… last)` — the LAST form is tail. ADDITIONALLY, if the FIRST form is a VALUE-def
        // `(def name V)`, `V` is a hoist search-root whose target is the WHOLE do: a `?` in the def value
        // short-circuits the entire do to the boundary value. Sound ONLY because the def is FIRST (nothing is
        // evaluated before it), so hoisting a `?` out of `V` reorders no preceding statement's effect.
        Some("do") if kids.len() >= 2 => {
            if let Some(dk) = ast.as_form(kids[1], "def")
                && dk.len() == 2
                && ast.as_name(dk[0]).is_some()
            {
                out.push((node, dk[1]));
            }
            collect_tail_positions(ast, kids[kids.len() - 1], out);
        }
        // Any other node — a tail LEAF expression to search for a hoistable `?`.
        _ => out.push((node, node)),
    }
}

/// Let-bind `node` in place: move its current content to a fresh init node and overwrite `node` to a fresh
/// NAME reference to `binder_name`, returning `(binder_atom, init_node)` for the enclosing `let`. Its parent
/// already points at `node`, so this repoints the continuation's use to the binder with no parent surgery.
fn bind_node_in_place(ast: &mut Arenas, node: StructId) -> (StructId, StructId) {
    let xname: std::sync::Arc<str> = format!("__try_hoist_{}", node.0).into();
    // The let-INIT — a fresh node holding `node`'s current content (its child ids shared, not re-copied).
    let init_struct = ast.structure[node.0 as usize].clone();
    let init = StructId(ast.structure.len() as u32);
    ast.structure.push(init_struct);
    // The binder NAME atom, and the in-place NAME reference overwriting `node`.
    let binder = push_atom(ast, Leaf::Name(xname.clone()));
    let ref_lid = LeafId(ast.leaves.len() as u32);
    ast.leaves.push(Leaf::Name(xname));
    ast.structure[node.0 as usize] = Struct::Atom(ref_lid);
    (binder, init)
}

/// Rewrite `target` (a tail-position node holding `C[(try e)]`) in place to bind the impure prefix operands
/// (evaluated before the `?`) and then the `?` itself, as NESTED `let`s wrapping the continuation:
/// `(let ((t0 p0)) … (let ((x (try e))) C[t0…/x]))` (BRICK 3 slices 1/2b, `DESIGN-try-operator-rcdzc.md`
/// §7.1). `prefix` is the impure operands in EVALUATION order (outermost `let` first); `tn` is the `(try e)`
/// node (innermost). Preserves `target`'s StructId/span. Each resulting binding-tail `let` rides the
/// inline-safe `lower_let` runtime-`?` `Core::MatchSum` short-circuit (the impure-prefix `let`s bind ordinary
/// values; only the innermost `let` binds a `?`).
fn hoist_try_at(ast: &mut Arenas, target: StructId, tn: StructId, prefix: &[StructId]) {
    let Struct::List(tn_kids) = ast.get(tn) else {
        return;
    };
    if tn_kids.len() != 2 {
        return;
    }
    // Bind each impure prefix operand (eval order) and the `?` last — overwriting each in place to a name
    // reference within `target`'s subtree. `pairs` is `(binder, init)` in eval order.
    let mut pairs: Vec<(StructId, StructId)> = Vec::new();
    for &p in prefix {
        pairs.push(bind_node_in_place(ast, p));
    }
    pairs.push(bind_node_in_place(ast, tn));
    // The continuation `C[…]` — `target`'s content moved to a FRESH node (carrying the name references), so
    // `target`'s original children are re-parented under this node, not left as twins of `target`.
    let mut body = StructId(ast.structure.len() as u32);
    let target_struct = ast.structure[target.0 as usize].clone();
    ast.structure.push(target_struct);
    // Wrap in nested `let`s INNERMOST-first (the `?` binds closest to the continuation, the first-evaluated
    // prefix operand outermost) so evaluation order is preserved. The OUTERMOST `let` is written IN PLACE into
    // `target` (preserving its StructId/span) — never via a clone of a separate node, which would leave
    // `target` and that node as twin parents of the same children and break scope resolution.
    let n = pairs.len();
    for (idx, (binder, init)) in pairs.into_iter().rev().enumerate() {
        let let_head = push_atom(ast, Leaf::Name("let".into()));
        let pair = push_list(ast, vec![binder, init]);
        let bindings = push_list(ast, vec![pair]);
        if idx + 1 == n {
            ast.structure[target.0 as usize] = Struct::List(vec![let_head, bindings, body]);
        } else {
            body = push_list(ast, vec![let_head, bindings, body]);
        }
    }
}

/// Collect every node id that lives INSIDE a `(quote …)` form (the quote's argument subtrees, transitively).
/// A `?` under `quote` is DATA, not control flow: `quote E` reifies `E` VERBATIM as the user wrote it, so a
/// `(try e)` inside a quote must survive to `reify_quotes` unchanged — hoisting it would make `quote (try e)`
/// reify the desugared `let` scaffolding instead of the source, and (because the hoist binder is named from a
/// node id) two textually-identical `(quote E)` occurrences would desugar to DIFFERENT names, breaking the
/// binary-AST round-trip identity the quote-corpus pass checks. The scan below skips these ids.
fn collect_quoted_nodes(ast: &Arenas) -> std::collections::HashSet<u32> {
    let mut quoted = std::collections::HashSet::new();
    let len = ast.structure.len() as u32;
    for i in 0..len {
        let id = StructId(i);
        if ast.as_name(id).is_some() {
            continue;
        }
        let Struct::List(kids) = ast.get(id) else {
            continue;
        };
        if kids.first().and_then(|&h| ast.as_name(h)) != Some("quote") {
            continue;
        }
        // Mark the whole quoted subtree (every form after the `quote` head) as off-limits to the hoist.
        let mut stack: Vec<StructId> = kids[1..].to_vec();
        while let Some(n) = stack.pop() {
            if !quoted.insert(n.0) {
                continue;
            }
            if let Struct::List(ks) = ast.get(n) {
                stack.extend(ks.iter().copied());
            }
        }
    }
    quoted
}

/// Hoist a single EXPRESSION-position `?` in each fallible-boundary function body to a boundary `let`
/// (BRICK 3 slices 1/2a, `DESIGN-try-operator-rcdzc.md` §7.1): `C[(try e)]` => `(let ((x (try e))) C[x])`.
/// The resulting binding-tail `let` rides the proven `lower_let` runtime-`?` `Core::MatchSum` short-circuit
/// (the same path tdd1 / stored-closure use) — no `Core::Block`/`Break` and no new emit, and it is
/// INLINE-SAFE because the `match` is local (an emit-time boundary-block wrap is NOT, since inlining moves
/// the `?` into a caller with a different result type). Runs at LOAD, AFTER `desugar_try_do_defs` (so a
/// binding-tail do-def is already a `let` the search stops at) and before resolution. A `def`/`fn` node that
/// lives INSIDE a `quote` is EXCLUDED (`collect_quoted_nodes`) — a quoted `?` is source data, not evaluated
/// control flow, and must reify verbatim.
pub fn desugar_try_expr_position(ast: &mut Arenas) {
    // FAST BAIL: no `(try …)` form anywhere (the common case) → no interned `try` name leaf.
    if !ast
        .leaves
        .iter()
        .any(|l| matches!(l, Leaf::Name(n) if n.as_ref() == "try"))
    {
        return;
    }
    // Nodes inside a `(quote …)` — the hoist must NOT descend into these (they reify verbatim). Computed once
    // on the source structure; the fixpoint below only APPENDS `let` scaffolding (never inside a quote), so
    // the set stays valid across passes (new node ids are absent from it, hence scanned normally).
    let quoted = collect_quoted_nodes(ast);
    // FIXPOINT: hoist ONE `?` per boundary body per pass, then repeat until a pass finds none. Each hoist
    // converts an expression-position `?` to a `let`-INIT (which the search stops at), so the count of
    // searchable `?`s strictly DECREASES — the loop terminates, and it handles MULTIPLE `?`s in one body
    // (e.g. several `#list` elements: each pass hoists the next, its predecessors already bound to names).
    // Bounded by the structure size as a defensive backstop against any non-decreasing pass.
    let mut guard = ast.structure.len() + 1;
    loop {
        // Each fallible-boundary body is a `def`/`fn` node's LAST child (`(def target body)` / `(fn params
        // body)`, both arity-3). For each body, collect its TAIL positions and plan the FIRST hoistable
        // expression-position `?`. Re-scan the (grown) structure each pass; the appended `let` scaffolding
        // is never a `def`/`fn`, and a hoisted `?` is now a `let`-init the search skips.
        let len = ast.structure.len() as u32;
        let mut plans: Vec<(StructId, StructId, Vec<StructId>)> = Vec::new();
        for i in 0..len {
            // A `def`/`fn` inside a `(quote …)` is quoted data — never a hoist boundary.
            if quoted.contains(&i) {
                continue;
            }
            let id = StructId(i);
            let Struct::List(kids) = ast.get(id) else {
                continue;
            };
            if kids.len() != 3 {
                continue;
            }
            // A boundary body is a FUNCTION def `(def (sig …) body)` (its `kids[1]` is the signature LIST) or
            // a `fn` lambda `(fn params body)`. A VALUE def `(def name value)` (`kids[1]` a NAME atom) is NOT
            // a boundary — its value's type is the binding's, not a function result — and crucially a
            // do-LOCAL value-def's `?` belongs to the enclosing function's boundary (handled by the do-def
            // value candidate in `collect_tail_positions`, which wraps the whole `do`), NOT to the value
            // position. Treating a value-def as a boundary would hoist the `?` into the value and miscompile.
            let is_boundary = match ast.as_name(kids[0]) {
                Some("def") => matches!(ast.get(kids[1]), Struct::List(_)),
                Some("fn") => true,
                _ => false,
            };
            if !is_boundary {
                continue;
            }
            let body = kids[2];
            let mut candidates: Vec<(StructId, StructId)> = Vec::new();
            collect_tail_positions(ast, body, &mut candidates);
            for (target, search_root) in candidates {
                if let Some((tn, prefix)) = find_hoistable_try(ast, search_root) {
                    plans.push((target, tn, prefix));
                    break; // one hoist per body per pass
                }
            }
        }
        if plans.is_empty() {
            break;
        }
        for (target, tn, prefix) in plans {
            hoist_try_at(ast, target, tn, &prefix);
        }
        guard -= 1;
        if guard == 0 {
            break;
        }
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
