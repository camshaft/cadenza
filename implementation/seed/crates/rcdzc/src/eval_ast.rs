//! EVAL DESUGAR — turn `(eval AST-VALUE)` into the SOURCE FORM that `AST-VALUE` denotes, then let the
//! ordinary pipeline resolve/type/lower it (`metaprogramming.md` §Eval Is Optional For Macros And
//! Interactive Use). `eval` executes an AST value AS code; because compile-time evaluation is ONE TIER
//! (`metaprogramming.md` §Compile-Time Evaluation Is One Tier — macro expansion, generic reduction,
//! folding are the SAME mechanism), evaluating a COMPILE-TIME-VISIBLE AST is exactly macro expansion:
//! reconstruct the syntax the AST denotes and splice it in, so `(eval (quote (+ 1 2)))` becomes `(+ 1 2)`
//! and folds to `3` through the ordinary path — no separate interpreter.
//!
//! This is the INVERSE of [`crate::quote::reify_quotes`]. That pass rewrites `(quote FORM)` into the
//! `Ast` construction that BUILDS `FORM`'s value; this pass reads such a construction back into `FORM`.
//! Running AFTER `reify_quotes` means `(eval (quote (+ 1 2)))` has already become `(eval (Ast.List (list
//! (Ast.Name "+") (Ast.Int 1) (Ast.Int 2))))`, so ONE reconstruction handles both the quoted spelling
//! and a hand-written `(Ast.* …)` argument — they are the same nodes by then.
//!
//! ```text
//! (Ast.Int N)               ->  N                        -- the integer literal
//! (Ast.Name "foo")          ->  foo                      -- the bare name
//! (Ast.List (list a b c))   ->  (<recon a> <recon b> …)  -- the compound form
//! ```
//!
//! Only a fully-reconstructable, COMPILE-TIME-VISIBLE argument is rewritten (the `Ast` sum's `Int`/`Name`/
//! `List` variants over `(list …)` literals — the same three the reifier produces). A `eval` of a RUNTIME
//! or non-constant AST value is LEFT UNTOUCHED for `resolve` to decline (the compiler does not execute a
//! dynamically-constructed AST — `metaprogramming.md`: "the compiler constructs and analyzes AST but does
//! not execute dynamically-constructed AST"). A malformed reconstruction (an empty `Ast.List` — a compound
//! with no operator) is rewritten to `(trap "malformed AST")`, the runtime halt the corpus records.
//!
//! ## Ordering / in-place rewrite
//!
//! Modelled on [`crate::quote::reify_quotes`] and [`crate::effects::desugar_handles`]: a scan collects
//! the rewrites, then they are applied by overwriting each `(eval …)` node's structure entry with its
//! reconstruction. Runs during `Db::load` AFTER `reify_quotes` (so a quoted argument is already an
//! `Ast.*` tree) and BEFORE the parent index — so the spliced-in source resolves like hand-written code.

use crate::ast::{Arenas, CompoundCtor, Leaf, Struct, StructId};
use crate::prelude::{push_atom, push_list};

/// A pending eval rewrite: overwrite the `(eval …)` node's structure entry with `replacement`, then blank
/// the dead reified-argument wrapper nodes (`arg`'s subtree minus what the reconstruction still references).
struct EvalPlan {
    eval: StructId,
    arg: StructId,
    replacement: StructId,
}

/// Every node id reachable from `root` through the structure child lists (inclusive). Used to diff the
/// dead reified-argument subtree against the live reconstruction so the dead wrapper nodes can be blanked.
pub(crate) fn reachable(ast: &Arenas, root: StructId) -> std::collections::HashSet<u32> {
    let mut seen = std::collections::HashSet::new();
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        if !seen.insert(n.0) {
            continue;
        }
        if let Struct::List(children) = ast.get(n) {
            stack.extend(children.iter().copied());
        }
    }
    seen
}

/// Desugar every `(eval AST)` whose argument is a compile-time-visible `Ast` construction into the source
/// form that AST denotes (see the module docs). A non-reconstructable argument is left for `resolve`.
///
/// The compiler does NOT require `eval` to compile programs, and it does NOT execute a dynamically-built
/// AST: only a COMPILE-TIME-VISIBLE `Ast` construction is reconstructed to source; a runtime/non-constant
/// AST argument is left for `resolve` to decline. And because `eval` folds the reconstructed source
/// through the ordinary compile-time path (the SAME tier as generic reduction, monomorphization, and
/// constant folding), there is one place the meaning of compile-time computation lives.
//= spec/capabilities/metaprogramming.md#eval-is-optional-for-macros-and-interactive-use
//# The compiler MUST NOT require `eval` to compile programs — the compiler constructs and analyzes AST but does not execute dynamically-constructed AST.
//= spec/capabilities/metaprogramming.md#compile-time-evaluation-is-one-tier
//# Macro expansion, generic reduction, monomorphization, and constant folding MUST be the same compile-time evaluation mechanism rather than separate subsystems, so that there is one place the meaning of compile-time computation lives and the four cannot drift apart.
// So a macro is not a distinct construct: `(eval AST)` desugars to the source the AST denotes and runs
// through the ONE ordinary compile-time path, i.e. an ordinary compile-time function applied to a
// program's AST data — not a separate macro interpreter.
//= spec/capabilities/metaprogramming.md#compile-time-evaluation-is-one-tier
//# A macro MUST be an ordinary compile-time function over the abstract syntax tree, so that a macro is not a distinct construct but an application of the one compile-time tier to a program's data.
pub fn desugar_eval(ast: &mut Arenas) {
    // Only ORIGINAL nodes can be a source `(eval …)`; reconstruction APPENDS, so bound the scan.
    let original_len = ast.structure.len() as u32;
    // FAST BAIL for a program with no `(eval …)` (the overwhelming common case). This pass runs at
    // EVERY load, scanning every node with an `as_form(id,"eval")` probe; an `(eval ARG)` node is a
    // `List` headed by the NAME `eval`, so its head is a `Leaf::Name("eval")` in the leaf pool. If no
    // such name leaf exists, no `(eval …)` form exists anywhere and the whole scan is dead. A single
    // O(leaves) prescan (leaves interned once, far fewer than a per-node structural probe) is the cheap
    // over-approximation: it may fall through spuriously only for a program that MENTIONS the identifier
    // `eval` without an eval form (a user def named `eval`), which then runs the exact scan below —
    // same result, just not skipped. (Sibling of the `quote::reify_quotes` quote-free fast-bail.)
    if !ast
        .leaves
        .iter()
        .any(|l| matches!(l, Leaf::Name(n) if n.as_ref() == "eval"))
    {
        return;
    }
    #[cfg(test)]
    crate::db::DESUGAR_EVAL_SCAN_NODES.with(|c| c.set(c.get() + original_len as u64));
    let mut plans: Vec<EvalPlan> = Vec::new();
    for i in 0..original_len {
        let id = StructId(i);
        // Match `(eval ARG)` — head name `eval`, exactly one argument. Clone the slice's ids so the
        // borrow ends before the reconstruction's `&mut ast`.
        let Some(arg) = ast.as_form(id, "eval").and_then(|tail| match tail {
            [only] => Some(*only),
            _ => None,
        }) else {
            continue;
        };
        if let Some(replacement) = reconstruct(ast, arg) {
            // eval PROVENANCE: a node LIVE-REUSED from the eval argument (`id < original_len`) is the
            // caller's spliced operand; a FRESH reconstructed node (`>= original_len`) is template.
            rename_captured_binders(ast, replacement, &|id| id.0 < original_len);
            plans.push(EvalPlan {
                eval: id,
                arg,
                replacement,
            });
        }
    }
    for EvalPlan {
        eval,
        arg,
        replacement,
    } in plans
    {
        // The nodes the LIVE reconstruction references — the copy about to be written into `eval` shares
        // them, so they must stay intact. Computed BEFORE the overwrite (the reconstruction root's subtree),
        // and it includes any REUSED live splice operand (a let-bound name `,x`, a param) `reconstruct`
        // carried over from the eval argument.
        let live = reachable(ast, replacement);
        // The dead reified-ARGUMENT wrapper subtree: every `(Ast.List …)`/`(Ast.Int …)`/`(list …)`/`(. Ast
        // …)` node the eval's argument built (by `reify_quotes` or a hand-written `Ast.*`). Once the eval
        // node is overwritten with the reconstructed source, this whole tree is unreachable EXCEPT for the
        // live splice operands the reconstruction reused. Blank each dead wrapper: `parent_index` records
        // the LAST (highest-id) parent per child, and a reified `(Ast.Int x)` wrapper (higher id than the
        // eval node) would otherwise remain `x`'s recorded parent — an orphan whose own parent is `None`,
        // so a scope walk from `x` dead-ends and a lexically-scoped `,x` resolves as a spurious "unbound
        // name". Blanking the dead wrappers leaves the reconstruction (at the eval position) the sole parent
        // of each shared splice operand, so it resolves against the eval's enclosing `let`/`def`.
        for dead in reachable(ast, arg) {
            if !live.contains(&dead) && dead != eval.0 {
                ast.structure[dead as usize] = Struct::List(Vec::new());
            }
        }
        // Overwrite the `(eval …)` node with a COPY of the reconstruction root's structure, so the eval's
        // own `StructId` (and span) is preserved as the spliced-in form's node.
        let entry = ast.get(replacement).clone();
        ast.structure[eval.0 as usize] = entry;
        // BLANK the reconstruction root when it is a FRESH appended node (a `push_*`-built compound) — it is
        // now a duplicate of the copy written into `eval`, and (higher-id) would otherwise out-rank the eval
        // copy as the shared children's parent (the same orphan hazard, mirrored from `reify_quotes`). A
        // reconstruction returning an ORIGINAL node directly (id < original_len — the whole eval arg was a
        // single `(Ast.Int <payload>)`, so `reconstruct` returns `payload` live) is the live spliced value,
        // still reachable through the eval copy; the `live`-set diff above already kept it, so don't blank.
        if replacement.0 >= original_len {
            ast.structure[replacement.0 as usize] = Struct::List(Vec::new());
        }
    }
}

/// Hygiene pass over a reconstructed source subtree `root`: rename any TEMPLATE-introduced binder that
/// would CAPTURE a variable spliced from an active unquote. The `original_len` boundary is the provenance
/// signal — a node with id `< original_len` is LIVE-REUSED from the eval argument (an unquote operand
/// carrying its enclosing-scope name), while a node with id `>= original_len` is FRESH, built by the
/// reconstruction (a template literal / binder). So: (1) collect the spellings of live-reused Name nodes
/// (the enclosing-scope names the splice must preserve); (2) find every binder form (`let`/`fn`/`match`)
/// in `root` and, for each binder whose spelling is a spliced name, alpha-rename it — rewriting the FRESH
/// occurrences of that spelling within the binder form's subtree to a fresh unique name. The live-reused
/// spliced occurrences (id < original_len) are left untouched, so they keep their spelling and resolve in
/// the enclosing scope rather than being captured by the template binder.
fn rename_captured_binders(ast: &mut Arenas, root: StructId, is_caller: &dyn Fn(StructId) -> bool) {
    // (1) The CALLER-ORIGIN (spliced) name spellings reachable in the reconstruction — the caller's
    // variables that a template binder must not shadow. `is_caller` is the PROVENANCE predicate: for
    // `eval` a node is caller-origin iff it is LIVE-REUSED from the eval argument (`id < original_len`);
    // for a MACRO expansion iff it was reconstructed UNDER an `ast-lift` (the caller's active-unquote
    // syntax). Both reduce to "this node is the caller's, not the template's".
    let mut spliced: std::collections::HashSet<String> = std::collections::HashSet::new();
    for n in reachable(ast, root) {
        if is_caller(StructId(n))
            && let Some(name) = ast.as_name(StructId(n))
        {
            spliced.insert(name.to_string());
        }
    }
    if spliced.is_empty() {
        return;
    }
    // (2) Walk every compound in the reconstruction; at a binder form, rename any TEMPLATE binder
    // colliding with a caller name. A stack walk over the reconstructed structure.
    let mut stack = vec![root];
    let mut seen = std::collections::HashSet::new();
    while let Some(node) = stack.pop() {
        if !seen.insert(node.0) {
            continue;
        }
        let Struct::List(items) = ast.get(node) else {
            continue;
        };
        let items = items.clone();
        for &c in &items {
            stack.push(c);
        }
        // Collect this form's binder names (by kind) that collide with a caller name.
        for binder in binder_names_of(ast, node) {
            // A caller-origin binder (a `let`/`fn`/`def` the CALLER spliced in) is the caller's own — do
            // NOT rename it; only a TEMPLATE-introduced binder is alpha-renamed for hygiene.
            if is_caller(binder) {
                continue;
            }
            let Some(spelling) = ast.as_name(binder).map(str::to_string) else {
                continue;
            };
            if !spliced.contains(&spelling) {
                continue;
            }
            // Alpha-rename: a fresh, collision-proof name (contains a space, so the reader/round-trip
            // never produces it and no source name collides). Rewrite the binder node + every
            // TEMPLATE-origin Name node of this spelling within `node`'s subtree; the caller-origin
            // occurrences keep the original spelling and resolve in the caller's scope.
            let fresh = format!("{spelling} $capture{}", binder.0);
            for m in reachable(ast, node) {
                if !is_caller(StructId(m)) && ast.as_name(StructId(m)) == Some(spelling.as_str()) {
                    // Overwrite the leaf this atom points at with the fresh name.
                    if let Struct::Atom(lid) = ast.get(StructId(m)) {
                        let lid = *lid;
                        ast.leaves[lid.0 as usize] = Leaf::Name(fresh.clone().into());
                    }
                }
            }
        }
    }
}

/// The binder NAME nodes a binding form `node` introduces, or empty if `node` is not a binder form.
/// Mirrors the binder shapes `eval.rs`/`resolve.rs` recognize:
///  - `(let ((n init)…) body)` — each binding pair's first element `n`;
///  - `(fn (p…) body)` / `(fn ((: p T)…) body)` — each param `p` (bare or the name inside `(: p T)`);
///  - `(match scrut (pat body)…)` — each arm's PATTERN when it is a bare-name binder.
fn binder_names_of(ast: &Arenas, node: StructId) -> Vec<StructId> {
    let Struct::List(items) = ast.get(node) else {
        return Vec::new();
    };
    let head = items.first().and_then(|&h| ast.as_name(h));
    let mut binders = Vec::new();
    match head {
        Some("let") => {
            // items = [let, bindings-list, body…]; bindings-list = [(n init)…].
            if let Some(&bindings) = items.get(1)
                && let Struct::List(pairs) = ast.get(bindings)
            {
                for &pair in pairs {
                    if let Struct::List(p) = ast.get(pair)
                        && let Some(&name) = p.first()
                    {
                        binders.push(binder_of_param(ast, name));
                    }
                }
            }
        }
        Some("fn") => {
            // items = [fn, param-list, body…]; param-list = [p… | (: p T)…].
            if let Some(&params) = items.get(1)
                && let Struct::List(ps) = ast.get(params)
            {
                for &p in ps {
                    binders.push(binder_of_param(ast, p));
                }
            }
        }
        Some("def") => {
            // items = [def, sig, body…]. The introduced binder is the def's NAME: `sig` is either a bare
            // NAME (a value def `(def x V)`) or a signature LIST `(name param…)` (a function def
            // `(def (f p…) body)`) whose FIRST child is the name. (A do-local `def` in a macro expansion
            // binds that name for the following forms — `resolve::do_local_binds` — so a template `(def x
            // …)` must not capture a caller `x`.) The function-def PARAMS are a deeper case left to the
            // `fn`/nested handling; here we alpha-rename the def name itself.
            if let Some(&sig) = items.get(1) {
                match ast.get(sig) {
                    Struct::Atom(_) => binders.push(sig),
                    Struct::List(s) => {
                        if let Some(&name) = s.first() {
                            binders.push(name);
                        }
                    }
                }
            }
        }
        Some("match") => {
            // items = [match, scrut, (pat body)…]; a pattern binds its bare-name sub-nodes — a BARE pattern
            // `x`, but also every binder NESTED in a COMPOUND pattern: a variant payload `(Some x)`, a
            // `(tuple x y)`, a `(list h .. rest)`, a `(map (k v) .. rest)`, and nesting thereof. Recurse so a
            // compound-pattern binder is renamed too (else it captures a spliced var — the gap v-inference
            // caught: `(match (Some 1) ((Some x) ,x))` mis-captured `,x`).
            for &arm in items.iter().skip(2) {
                if let Struct::List(a) = ast.get(arm)
                    && let Some(&pat) = a.first()
                {
                    collect_pattern_binders(ast, pat, &mut binders);
                }
            }
        }
        _ => {}
    }
    binders
}

/// Collect every bare-name BINDER node in a match PATTERN into `out`, recursing into compound patterns.
/// A pattern is a binder-bearing shape: a bare name `x` (a binder — unless `_`); a compound `(head sub…)`
/// where `head` is a constructor / a `tuple`/`list`/`map`/`record` alias — its HEAD is not a binder (it is
/// the ctor/alias name or a `.`-qualified ctor), but each SUB-pattern is; a `..` rest marker's following
/// name is a rest binder. A literal (Int/Float/Str/Bool) or `_` binds nothing. Mirrors the pattern-binder
/// enumeration `resolve.rs`/`lower.rs` do (list/map/variant pattern binders), scoped to name collection.
fn collect_pattern_binders(ast: &Arenas, pat: StructId, out: &mut Vec<StructId>) {
    match ast.get(pat) {
        Struct::Atom(_) => {
            // A bare name is a binder (`_` binds nothing; a literal atom is not a Name so `as_name` is None).
            if let Some(name) = ast.as_name(pat)
                && name != "_"
            {
                out.push(pat);
            }
        }
        // A record-pattern FIELD `(= field sub-pattern)` (path B): the field NAME binds nothing, only
        // the sub-pattern (child 2). Without this the generic recursion below would collect the field
        // name as a spurious binder (the `=` head is skipped, then both `field` and the sub-pattern walk).
        Struct::List(items) if items.len() == 3 && ast.as_name(items[0]) == Some("=") => {
            collect_pattern_binders(ast, items[2], out);
        }
        Struct::List(items) => {
            // A compound pattern `(head sub…)`: the head is the ctor / `tuple`|`list`|`map`|`record` alias /
            // the `.`-qualified ctor form `(. T Ctor)` — NOT a binder. Every following element is a
            // sub-pattern (recurse); a `..` rest marker is skipped (its neighbor name is an ordinary binder
            // reached by the recursion). Skip element 0 (the head). A `(record (= f p) …)` field is handled
            // by the arm above; a legacy `(record (f p))` pair recurses here (head `f` skipped, `p` walked).
            let items = items.clone();
            for &sub in items.iter().skip(1) {
                // Skip the flat `..` rest marker itself (a bare `..` name); its binder neighbor recurses
                // normally. For the wrapped `(.. operand)` node, the rest binder is the operand INSIDE it —
                // recurse the operand then skip the node.
                if ast.as_name(sub) == Some("..") {
                    continue;
                }
                if let Some(args) = ast.as_form(sub, "..") {
                    for &a in args {
                        collect_pattern_binders(ast, a, out);
                    }
                    continue;
                }
                collect_pattern_binders(ast, sub, out);
            }
        }
    }
}

/// A param slot's binder name: the bare name, or the `p` inside an annotated `(: p T)` binder.
fn binder_of_param(ast: &Arenas, slot: StructId) -> StructId {
    if let Some(tail) = ast.as_form(slot, ":")
        && let Some(&name) = tail.first()
    {
        return name;
    }
    slot
}

/// Reconstruct the source form an `Ast` construction `node` denotes — the inverse of the reifier's map.
/// Returns the root of the fresh reconstructed tree, or `None` if `node` is not a fully compile-time-
/// visible `Ast.*` construction (then the `(eval …)` is left for `resolve` to decline).
///
/// An empty `Ast.List` (a compound with no operator — malformed AST) reconstructs to `(trap "malformed
/// AST")`: eval of a malformed AST is a runtime halt (`metaprogramming.md` §Eval Is Optional: "eval on
/// malformed AST traps"), not a value.
/// Reconstruct an `Ast.*` value back to the SOURCE it denotes — the `eval` path (a bare name / computed
/// active-unquote operand passes through to fold in the enclosing scope; a spliced `Ast` VALUE stays an
/// `Ast`, so an `Ast` in a numeric position is the deliberate CDZ0201 reject).
pub(crate) fn reconstruct(ast: &mut Arenas, node: StructId) -> Option<StructId> {
    let mut caller = std::collections::HashSet::new();
    reconstruct_inner(ast, node, false, &mut caller)
}

/// Reconstruct for MACRO EXPANSION — like [`reconstruct`] but SEES THROUGH a spliced reflected `Ast`
/// construction at an `ast-lift` operand (a macro's reified `quote`-parameter argument: `,x` where `x` was
/// bound to `(Ast.Int 5)` reconstructs to `5`, its denotation, not the constructor call). A macro's `,x`
/// splices SYNTAX (code), so its expansion is real code; distinct from `eval`, where a spliced `Ast` value
/// is data and must not be seen through (DESIGN-macro-system.md §4).
pub(crate) fn reconstruct_macro(ast: &mut Arenas, node: StructId) -> Option<StructId> {
    // Track CALLER-ORIGIN nodes: a caller's quote-argument reaches the expansion through an active
    // unquote (the `ast-lift` boundary), so `reconstruct_inner` records every node it reconstructs UNDER
    // an `ast-lift` into `caller`. Everything else in `root` is MACRO-TEMPLATE syntax.
    let mut caller = std::collections::HashSet::new();
    let root = reconstruct_inner(ast, node, true, &mut caller)?;
    // HYGIENE (preserve-by-default, DESIGN-macro-system.md / metaprogramming.md §Macros Are Hygienic): a
    // binder INTRODUCED by the macro template must not CAPTURE a caller-spliced identifier of the same
    // name. Alpha-rename any template binder whose spelling collides with a caller-origin name; the
    // caller's occurrences (in `caller`) keep their spelling and resolve in the caller's scope.
    rename_captured_binders(ast, root, &|id| caller.contains(&id.0));
    Some(root)
}

fn reconstruct_inner(
    ast: &mut Arenas,
    node: StructId,
    see_through_lift: bool,
    caller: &mut std::collections::HashSet<u32>,
) -> Option<StructId> {
    // `(Ast.Int payload)` -> the payload AS SOURCE. `Ast.Int` arises two ways, both reconstructing to the
    // payload node itself: (a) a reified INTEGER LITERAL `(Ast.Int 42)` — payload is the literal `42`, whose
    // source is `42`; (b) an ACTIVE-UNQUOTE lift `(Ast.Int <e>)` where `reify_active` wrapped the unquote's
    // LIVE operand `<e>` (a name `,x`, a computed `,(+ 1 2)`) — its source is `<e>` itself. So unwrapping the
    // `Ast.Int` back to `payload` reconstructs both: a splice of a compile-time-known VARIABLE or expression
    // (the core macro idiom — `(eval `(+ ,x 4))` with `x`=3 → `(+ x 4)` → folds to 7) reconstructs like a
    // literal splice, rather than leaving the `(eval …)` un-desugared (its head `eval` then a misleading
    // "unbound name `eval`"). The payload node is REUSED live: for (b) it is the evaluated code, which must
    // resolve against the `(eval …)`'s enclosing scope, so it is spliced in unchanged (not copied). The
    // reconstructed source folds through the ordinary compile-time path; a payload that is NOT compile-time-
    // known then declines/errors there as ordinary code would, not here.
    if let Some(payload) = ast_ctor_arg(ast, node, "Int") {
        // MACRO path (see_through_lift): return a FRESH copy of a bare int literal, NOT the payload node
        // itself. The payload is shared with this (soon-dead) `Ast.Int` wrapper, and
        // `collect_bigint_ctor_arg_literals` — recomputed / consulted over the post-expansion arena —
        // marks a bare `Ast.Int` arg to ground `BigInt`; a SHARED spliced literal would inherit that mark
        // and wrongly ground `BigInt` (the body-literal BigInt-vs-Int64 bug). A fresh copy is a distinct
        // node the ctor-arg scan never sees, so it grounds `Int64` by ordinary inference at the call site.
        // (eval keeps the reuse — it splices at load, before the scan, and needs live-reuse for scope.)
        if see_through_lift {
            let stripped = strip_bigint_grounding(ast, payload);
            if let Struct::Atom(l) = ast.get(stripped) {
                let leaf = ast.leaf(*l).clone();
                if matches!(leaf, Leaf::Int { .. }) {
                    return Some(push_atom(ast, leaf));
                }
            }
            return Some(stripped);
        }
        // STRIP the reifier's `(: <lit> BigInt)` grounding wrapper (`quote::ast_bigint_payload`): an
        // `Ast.Int` VALUE stores its integer as `BigInt` (non-lossy storage), but the source an eval
        // RECONSTRUCTS must ground by ordinary width inference — an `(eval (quote (+ 1 2)))` yields an
        // `Int64` `3`, not a `BigInt`, exactly as the un-quoted `(+ 1 2)` would. So the reconstructed
        // literal is the BARE inner node, not the annotated one; BigInt is a property of the stored AST,
        // not one the reconstructed source carries out. A payload with no wrapper (a live active-unquote
        // operand) is returned unchanged.
        return Some(strip_bigint_grounding(ast, payload));
    }
    // `(Ast.Float payload)` -> the payload AS SOURCE (the float-literal node). A reified float literal
    // `(Ast.Float 1.5)` unwraps back to the `1.5` literal, which evaluates to itself.
    if let Some(payload) = ast_ctor_arg(ast, node, "Float") {
        return Some(payload);
    }
    // `((intrinsic "ast-lift") e)` -> the operand `e` AS SOURCE. `ast-lift` wraps a RUNTIME active-unquote
    // operand (a name / a computed expression — `quote::reify_active`), so reconstructing the source the
    // AST denotes unwraps it back to `e` (the evaluated code), reused live exactly as `Ast.Int`'s payload.
    // So `(eval (quasiquote (+ (unquote x) 4)))` — whose `,x` now reifies to `(ast-lift x)` rather than a
    // literal-dispatched `(Ast.Int x)` — reconstructs to `(+ x 4)` and folds in the eval's enclosing scope.
    if let Some(payload) = ast_lift_arg(ast, node) {
        // eval (see_through_lift = false): pass the operand through unchanged — a bare name / computed
        // expr folds in scope, and a spliced `Ast` VALUE stays an `Ast` (an Ast in a numeric position is
        // the deliberate CDZ0201 reject). MACRO (true): the operand is a reflected quote-param argument
        // (syntax), so reconstruct it to its DENOTATION (`(Ast.Int 5)` → `5`); a non-Ast operand
        // (`unwrap_or`) still passes through.
        if see_through_lift {
            let r = reconstruct_inner(ast, payload, true, caller).unwrap_or(payload);
            // `r` and its whole subtree are CALLER-ORIGIN — reconstructed from an active-unquote operand
            // (the caller's spliced syntax). Record them so hygiene never renames a caller identifier.
            caller.extend(reachable(ast, r));
            return Some(r);
        }
        return Some(payload);
    }
    // `(Ast.Bool payload)` -> the payload AS SOURCE (the `true`/`false` literal node). Like `Ast.Int`,
    // the payload node is reused live: a reified boolean literal `(Ast.Bool true)` unwraps back to `true`.
    if let Some(payload) = ast_ctor_arg(ast, node, "Bool") {
        return Some(payload);
    }
    // `(Ast.Str payload)` -> the payload AS SOURCE (the string-literal node). A reified string literal
    // `(Ast.Str "hi")` unwraps back to the `"hi"` literal, which evaluates to itself.
    if let Some(payload) = ast_ctor_arg(ast, node, "Str") {
        return Some(payload);
    }
    // `(Ast.Bytes payload)` -> the payload AS SOURCE (the `b"…"` byte-literal node). Like `Ast.Str`, a
    // reified byte-string literal unwraps back to the `b"…"` literal, which evaluates to itself. So
    // `(eval (quote b"hi"))` reconstructs the byte literal and folds to that Bytes value.
    if let Some(payload) = ast_ctor_arg(ast, node, "Bytes") {
        return Some(payload);
    }
    // `(Ast.Name payload)` -> the bare name the String payload spells. `Ast.Name` carries the identifier
    // as a String (the reifier turned a `Leaf::Name` into a `Leaf::Str`); reconstruction turns it back.
    if let Some(payload) = ast_ctor_arg(ast, node, "Name") {
        let name = ast.as_str(payload)?.to_string();
        return Some(push_atom(ast, Leaf::Name(name.into())));
    }
    // `(Ast.List (list e…))` -> the compound form `(<recon e>…)`. An empty list: for EVAL it is malformed
    // (an empty compound has no operator to evaluate — `eval` on malformed AST traps). For a MACRO
    // expansion (`see_through_lift`) an empty `()` is a VALID template element — e.g. a nullary handler
    // arm's empty parameter list `(op () state body)`, or an empty binding list — so reconstruct it as the
    // empty list `()`, not a trap (the malformed-trap wrongly turned a macro's empty `()` into `(trap …)`,
    // mangling the arm → CDZ0201; breaker gap#5). A genuinely empty-compound EXPRESSION a macro emits is
    // still caught downstream by resolve, without breaking a valid empty list.
    if let Some(payload) = ast_ctor_arg(ast, node, "List") {
        let elems = list_elems(ast, payload)?;
        if elems.is_empty() {
            if see_through_lift {
                return Some(push_list(ast, Vec::new()));
            }
            return Some(trap_form(ast, "malformed AST"));
        }
        let mut children = Vec::with_capacity(elems.len());
        for e in elems {
            children.push(reconstruct_inner(ast, e, see_through_lift, caller)?);
        }
        return Some(push_list(ast, children));
    }
    // The DEDICATED native collection-ctor variants (the inverse of `quote::reify_inner`'s `Leaf::Ctor`
    // branch): `Ast.<X>Ctor (list <child…>)` -> the native `#<x>(<recon child>…)` literal (a `Leaf::Ctor`
    // head + reconstructed children, NO name head). List/Tuple/Set carry bare element ASTs; Record/Map
    // carry `Ast.FieldPair` children rebuilt to `(= k v)` (record) / `(k v)` (map) entries. So `(eval
    // (quote #list(1 2 3)))` rebuilds the native `#list(1 2 3)` and folds to the runtime list.
    for (variant, ctor) in [
        ("ListCtor", CompoundCtor::List),
        ("TupleCtor", CompoundCtor::Tuple),
        ("SetCtor", CompoundCtor::Set),
    ] {
        if let Some(payload) = ast_ctor_arg(ast, node, variant) {
            let elems = list_elems(ast, payload)?;
            let mut children = vec![push_atom(ast, Leaf::Ctor(ctor))];
            for e in elems {
                children.push(reconstruct_inner(ast, e, see_through_lift, caller)?);
            }
            return Some(push_list(ast, children));
        }
    }
    // Record/Map: children are `Ast.FieldPair` — rebuild each to the canonical native `(= k v)` entry (a
    // `Leaf::FieldPair` head — the M3 spelling BOTH `#record(…)` and `#map(…)` use, per the corpus's
    // `#map((= 1 2))` / `#record((= a 1))`), then head with the native ctor leaf.
    for (variant, ctor) in [
        ("RecordCtor", CompoundCtor::Record),
        ("MapCtor", CompoundCtor::Map),
    ] {
        if let Some(payload) = ast_ctor_arg(ast, node, variant) {
            let elems = list_elems(ast, payload)?;
            let mut children = vec![push_atom(ast, Leaf::Ctor(ctor))];
            for fp in elems {
                // A genuine `Ast.FieldPair` rebuilds to a `(= k v)` entry. A NON-FieldPair child is a
                // reflected `(.. rest)` REST MARKER (reified as an `Ast.List [Ast.Name "..", <binder>]`) —
                // reconstruct it directly so the native `#map`/`#record` pattern stays OPEN (the map-rest
                // face of the #6855 fix; without this a quoted map/record-rest pattern closed and its match
                // fell through to the catch-all).
                if ast_ctor_arg(ast, fp, "FieldPair").is_some() {
                    let (k, v) = reconstruct_field_pair(ast, fp, see_through_lift, caller)?;
                    let eq = push_atom(ast, Leaf::FieldPair);
                    let entry = push_list(ast, vec![eq, k, v]);
                    children.push(entry);
                } else {
                    children.push(reconstruct_inner(ast, fp, see_through_lift, caller)?);
                }
            }
            return Some(push_list(ast, children));
        }
    }
    // `Ast.Member (tuple <obj> <key>)` -> the member access `(. <recon obj> <recon key>)`.
    if let Some(payload) = ast_ctor_arg(ast, node, "Member") {
        let (obj, key) = tuple2_of(ast, payload)?;
        let obj = reconstruct_inner(ast, obj, see_through_lift, caller)?;
        let key = reconstruct_inner(ast, key, see_through_lift, caller)?;
        let dot = push_atom(ast, Leaf::Name(".".into()));
        return Some(push_list(ast, vec![dot, obj, key]));
    }
    None
}

/// Reconstruct an `Ast.FieldPair (tuple <key-ast> <value-ast>)` to its `(reconstructed key, reconstructed
/// value)` source pair. `None` if `node` is not a well-formed `Ast.FieldPair` over a 2-tuple.
fn reconstruct_field_pair(
    ast: &mut Arenas,
    node: StructId,
    see_through_lift: bool,
    caller: &mut std::collections::HashSet<u32>,
) -> Option<(StructId, StructId)> {
    let payload = ast_ctor_arg(ast, node, "FieldPair")?;
    let (k, v) = tuple2_of(ast, payload)?;
    Some((
        reconstruct_inner(ast, k, see_through_lift, caller)?,
        reconstruct_inner(ast, v, see_through_lift, caller)?,
    ))
}

/// The two element occurrences of a `(tuple a b)` 2-tuple value form — the `(Tuple Ast Ast)` payload shape
/// of `Ast.FieldPair`/`Ast.Member`. Accepts the native `#tuple(a b)` ctor-leaf head and the `tuple` name
/// alias. `None` if `payload` is not a 2-element tuple form.
fn tuple2_of(ast: &Arenas, payload: StructId) -> Option<(StructId, StructId)> {
    let elems = ast
        .compound_form_of(payload, CompoundCtor::Tuple)
        .filter(|e| e.len() == 2)?;
    Some((elems[0], elems[1]))
}

/// If `node` is the compiler-internal lift `((intrinsic "ast-lift") e)` — a 2-element list whose head is
/// `(intrinsic ast-lift)` — its operand `e`. The shape `crate::quote::ast_lift` builds around a runtime
/// active-unquote operand. `None` otherwise.
fn ast_lift_arg(ast: &Arenas, node: StructId) -> Option<StructId> {
    let Struct::List(items) = ast.get(node) else {
        return None;
    };
    if items.len() != 2 {
        return None;
    }
    // The head is `(intrinsic ast-lift)` — a 2-element list `[intrinsic, ast-lift]`.
    let Struct::List(head) = ast.get(items[0]) else {
        return None;
    };
    if head.len() == 2
        && ast.as_name(head[0]) == Some("intrinsic")
        && ast.as_name(head[1]) == Some("ast-lift")
    {
        Some(items[1])
    } else {
        None
    }
}

/// If `node` is the constructor application `(Ast.<variant> payload)` — a list whose head is the
/// projection `(. Ast <variant>)` and which carries exactly one argument — that argument. The shape
/// `crate::quote::ast_ctor` builds and the reader produces for a hand-written `(Ast.<variant> x)`.
/// Strip a `(: <inner> BigInt)` type-annotation wrapper the reifier adds to ground an `Ast.Int`'s
/// literal payload to `BigInt` (`quote::ast_bigint_payload`), returning `<inner>`. A node that is not
/// that exact 3-element `(: _ BigInt)` shape is returned unchanged (a live active-unquote operand — a
/// name / computed expression — carries no wrapper). Used by `reconstruct` so an eval-reconstructed
/// integer literal grounds by ordinary width inference (`Int64`), not the stored `BigInt`.
fn strip_bigint_grounding(ast: &Arenas, node: StructId) -> StructId {
    if let Struct::List(items) = ast.get(node)
        && items.len() == 3
        && ast.as_name(items[0]) == Some(":")
        && ast.as_name(items[2]) == Some("BigInt")
    {
        return items[1];
    }
    node
}

fn ast_ctor_arg(ast: &Arenas, node: StructId, variant: &str) -> Option<StructId> {
    let Struct::List(items) = ast.get(node) else {
        return None;
    };
    if items.len() != 2 {
        return None;
    }
    // The head is `(. Ast <variant>)` — a 3-element list `[., Ast, <variant>]`.
    let Struct::List(head) = ast.get(items[0]) else {
        return None;
    };
    if head.len() == 3
        && ast.as_name(head[0]) == Some(".")
        && ast.as_name(head[1]) == Some("Ast")
        && ast.as_name(head[2]) == Some(variant)
    {
        Some(items[1])
    } else {
        None
    }
}

/// The element occurrences of a `(list e…)` value-constructor form — the reader-shaped list literal the
/// reifier wraps `Ast.List`'s payload in. Accepts ALL THREE head spellings of the list ctor: the native
/// ctor-LEAF-KIND head (the M2 `[…]` literal the reader now emits — e.g. from a `Ast.List([…])` printed by
/// the M2 printer and re-read), the `list` NAME alias, and the unshadowable `"list"` STRING-ctor head (all
/// denote the same list literal). Before M2 only name/string were reached — a reified `Ast.List` payload
/// re-read from ML rendered as a native list literal, so `eval` of it declined CDZ0101 (nothing to
/// reconstruct); `compound_form_of` closes that by recognizing the native leaf head too. `None` if `payload`
/// is not a list form.
fn list_elems(ast: &Arenas, payload: StructId) -> Option<Vec<StructId>> {
    ast.compound_form_of(payload, CompoundCtor::List)
        .map(<[StructId]>::to_vec)
}

/// Build `(trap "MSG")` — the diverging halt an eval of a malformed AST reconstructs to. `trap` is the
/// prelude diverging primitive `∀a. String → a`, so it validates in the `(eval …)` result position.
fn trap_form(ast: &mut Arenas, msg: &str) -> StructId {
    let trap = push_atom(ast, Leaf::Name("trap".into()));
    let message = push_atom(ast, Leaf::Str(msg.into()));
    push_list(ast, vec![trap, message])
}
