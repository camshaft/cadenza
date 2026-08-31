//! QUOTE REIFICATION — turn `(quote FORM)` into the `Ast` sum VALUE that denotes `FORM`'s structure,
//! WITHOUT evaluating `FORM` (`metaprogramming.md` §Quote Produces An AST Value).
//!
//! A quote evaluates to an ordinary `Ast` value (`type-system.md` §The Abstract Syntax Tree Type Is An
//! Ordinary Sum Type). So rather than teach resolve/infer/lower a bespoke "quote" node, this desugar
//! REWRITES each quote into the constructor application that BUILDS that value — the same AST the reader
//! produces for the hand-written constructor form. Everything downstream (resolution, inference,
//! lowering, structural equality) then treats a quote result and a hand-built `Ast.*` value identically,
//! which is exactly the corpus invariant: `(= (quote 42) (Ast.Int 42))` is `true`, and `(quote (+ 1 2))`
//! IS `(Ast.List (list (Ast.Name "+") (Ast.Int 1) (Ast.Int 2)))`.
//!
//! The reification is purely STRUCTURAL — it maps each syntax node to a constructor by its SHAPE, never
//! by its spelling:
//!
//! ```text
//! integer literal  N        ->  (Ast.Int N)
//! bare name        foo      ->  (Ast.Name "foo")
//! compound         (a b c)  ->  (Ast.List (list <reify a> <reify b> <reify c>))
//! ```
//!
//! Because it is structural, a quote is INERT (`metaprogramming.md` §Quote Produces An AST Value: "a
//! quote body is inert data"): a nested quasiquote/unquote inside a plain quote is reified as ordinary
//! `Ast.Name`/`Ast.List` structure — its `,x` becomes the NAME `x` (`(Ast.Name "x")`), NOT x's value.
//! So `(quote `(+ ,x))` and `(quote `(+ ,y))` denote DIFFERENT trees (they mention `x` vs `y`), and a
//! plain quote never evaluates anything in its body.
//!
//! ## Quasiquote — selective evaluation
//!
//! `(quasiquote TEMPLATE)` reifies like `quote`, EXCEPT at an ACTIVE `(unquote e)` hole, where `e` is
//! EVALUATED and its value INSERTED into the built AST (`metaprogramming.md` §Quasiquote Constructs AST
//! With Selective Evaluation). Quasiquote NESTS, so the "active" positions are tracked by a DEPTH counter
//! (the classic Bawden algorithm): the quasiquote body starts at depth 1, each nested `quasiquote` bumps
//! it, each `unquote` drops it; an `unquote` reached at depth 1 is active (evaluate), any deeper one is
//! inert structure.
//!
//! ```text
//! (unquote e)          depth 1  ->  (Ast.Int e)   -- ACTIVE: e stays LIVE, evaluated + lifted
//! (unquote e)          depth>1  ->  (Ast.List (list (Ast.Name "unquote")  <reify e @ depth-1>))
//! (quasiquote t)       any      ->  (Ast.List (list (Ast.Name "quasiquote") <reify t @ depth+1>))
//! ```
//!
//! An ACTIVE unquote keeps its operand `e` LIVE (reused, not reified) and lifts its VALUE into the
//! matching `Ast` leaf, so `e` resolves/types/lowers as ordinary code: an unbound name in it is the
//! ordinary CDZ0101 (NOT swallowed into inert AST), and its value lifts to a node structurally identical
//! to a const fold's — so `` `(f ,x) `` (x=1) equals `(quote (f 1))`. The lift is TYPE-DIRECTED: a value
//! LITERAL dispatches by its structural kind HERE (Int → `Ast.Int`, Float → `Ast.Float`, Bool →
//! `Ast.Bool`, String → `Ast.Str`); a RUNTIME operand (a name / a computed expression, unknown type at
//! reify time) is wrapped in the compiler-internal `(ast-lift e)`, which `lower` resolves by `e`'s
//! INFERRED type — IDENTITY when `e` is already an `Ast` (splice a sub-tree), else the matching leaf. A
//! literal the `Ast` sum has no value variant for BAILS (declines honestly, never a miscompile) — today
//! only a reader error-marker leaf (`BadChar`/`BadEscape`), since the `Ast` sum covers every real leaf.
//!
//! An ACTIVE `(unquote-splicing e)` (depth 1) SPLICES e's list elements into the parent: `reify_active`
//! builds the parent's element list by CONCATENATING runs of ordinary reified elements with
//! `(ast-splice-lift e)` segments (each lifts e's elements — scalars into their matching `Ast` leaf, an
//! `Ast` element by identity), so `` `(f ,@xs g) `` with xs=`(a b)` flattens to `(Ast.List (Ast.Name
//! "f") (Ast.Name "a") (Ast.Name "b") (Ast.Name "g"))`. A splice operand that is not a list is the
//! CDZ0201 non-list type error, and a non-liftable element (a nested list) declines
//! (reject-don't-miscompile) — the splice-lift map for those is a later increment.
//!
//! ## Quote PATTERNS — the dual direction
//!
//! A `` ` ``/`,` template in PATTERN position (a match arm's pattern slot) DESTRUCTURES an `Ast`
//! scrutinee (`metaprogramming.md` §A Quasiquote In Pattern Position Destructures An AST). It desugars to
//! the EXACTLY-EQUIVALENT `Ast.*` constructor PATTERN — `` `(+ ,a ,b) `` IS `(Ast.List (list (Ast.Name
//! "+") a b))` as a pattern — so it reuses the decision-tree matcher's existing `Ast.*` sum-pattern
//! handling, adding a surface not a mechanism. The reification is the same STRUCTURAL shape as
//! construction, but an unquote is a BINDER, not an evaluation:
//!
//! ```text
//! integer  N        ->  (Ast.Int N)              -- matches by equality
//! bare name foo     ->  (Ast.Name "foo")         -- literal head/subterm, matches by equality
//! (unquote P)       ->  P                         -- the sub-PATTERN (bare name binds; a nested ctor further-matches)
//! compound (a b c)  ->  (Ast.List (list <pat a> <pat b> <pat c>))   -- fixed arity element-by-element
//! final (unquote-splicing name) -> a `.. name` rest binder in the list pattern (binds remaining elements)
//! ```
//!
//! A NON-FINAL `,@` is ill-formed → CDZ0221 (a rest binds the tail, meaningful only last). Because the
//! desugar targets the constructor pattern, exhaustiveness is the ORDINARY rule (a quote pattern never
//! covers every `Ast` → an all-quote-pattern match needs a catch-all or is CDZ0210) and equality/encoding
//! are the constructor form's. A pattern-position quote/quasiquote MUST take this path, NOT the
//! construction reify above (which would evaluate an active unquote's binder as code → a spurious
//! CDZ0101); [`reify_quotes`] routes by whether the node sits in a match-arm pattern slot.
//!
//! ## Scope of this increment
//!
//! The built-in `Ast` sum carries a variant for EVERY syntax leaf — `Int`/`Float`/`Bool`/`Str`/`Name`/
//! `List`/`Bytes`/`Char`/`Symbol` — so quote/reflection is TOTAL: a form built from any of them is
//! reifiable (operator directive — reflection must never decline on a well-formed leaf). The ONLY leaf a
//! quote still bails on is a reader ERROR MARKER (`BadChar`/`BadEscape`), which arises solely from
//! malformed source (which does not compile); it flows to `resolve::resolve_quote`, which DECLINES (a
//! Todo, never a miscompile). Likewise an arity-≠1 `(quote …)` is left for `resolve_quote` to reject
//! CDZ0201. This pass only ever rewrites a quote/quasiquote it can reify COMPLETELY — partial
//! reification is never emitted.
//!
//! ## Ordering / in-place rewrite
//!
//! Modelled on [`crate::effects::desugar_handles`]: a scan collects the rewrites, then they are applied.
//! The reader builds children BEFORE parents, so a nested (inner) quote/quasiquote always has a SMALLER
//! `StructId` than the one enclosing it. Processing in DESCENDING id order therefore reifies an OUTER
//! quote — reading its body's still-ORIGINAL structure (its descendants have smaller ids, not yet
//! rewritten) — before the pass reaches an inner quote's id. By then the inner node is ORPHANED (the
//! outer's reified tree is all fresh nodes), so rewriting it is harmless dead work. The scan handles a
//! TOP-LEVEL `quasiquote` too; an INNER `quasiquote` the enclosing reification already consumed
//! structurally is orphaned, so re-planning it (at a fresh depth) is likewise harmless — its rewrite
//! lands on a node nothing reachable references. Reification READS existing nodes + APPENDS fresh ones;
//! the ONE exception is an ACTIVE unquote, which REUSES its operand node live (never rewritten — it is
//! the evaluated code, kept reachable so it resolves against the quasiquote's enclosing scope). A live
//! quote/quasiquote INSIDE an active unquote operand is real code and is correctly planned by the scan
//! (it is reachable, not orphaned). So no live node is ever mutated out from under a pending rewrite.

use crate::ast::{Arenas, Leaf, Struct, StructId};
use crate::prelude::{push_atom, push_list};

/// A pending quote rewrite: overwrite `quote`'s structure entry with the reified `Ast` construction
/// rooted at `reified`.
struct QuotePlan {
    quote: StructId,
    reified: StructId,
}

/// Desugar every reifiable `(quote FORM)` into the `Ast` constructor application that builds `FORM`'s
/// value (see the module docs). Runs during `Db::load`, before the parent index — so the emitted
/// `(. Ast …)` projections and `(list …)` forms resolve like hand-written source. Returns the
/// quote-PATTERN nodes with a NON-FINAL `,@` splice (ill-formed), which `collect_faults` reports CDZ0221.
///
/// So `(quote <expr>)` evaluates to an `Ast` sum VALUE representing `<expr>`'s structure without
/// evaluating `<expr>` (the reification builds the constructor application, never runs the quoted form),
/// and that `Ast` is an ordinary sum type deconstructible by pattern matching like any other sum.
//= spec/capabilities/metaprogramming.md#quote-produces-an-ast-value
//# The expression `(quote <expr>)` MUST evaluate to an AST sum type value representing the structure of `<expr>`, without evaluating `<expr>` itself.
//= spec/capabilities/metaprogramming.md#quote-produces-an-ast-value
//# The AST MUST be a sum type with variants for each syntactic form, deconstructible by pattern matching like any other sum type.
// Quasiquote is the same reification with SELECTIVE evaluation: a depth counter (Bawden) tracks active
// positions — the body starts at depth 1, each nested `quasiquote` bumps it, each `unquote` drops it; an
// `unquote` reached at depth 1 is ACTIVE (its operand stays live, evaluated + inserted into the built
// AST), a deeper one is reified inertly — so `` `(+ ,,x)`` evaluates the inner `,` and quasiquote nests.
// (An active `,@` unquote-SPLICING is a later increment — it bails here — so §…-selective-evaluation's
// splice sentence stays uncited.)
//= spec/capabilities/metaprogramming.md#quasiquote-constructs-ast-with-selective-evaluation
//# The expression `` `<template>`` (quasiquote) MUST produce an AST value like `quote`, but with selective evaluation at marked positions.
//= spec/capabilities/metaprogramming.md#quasiquote-constructs-ast-with-selective-evaluation
//# Any subexpression `,<expr>` (unquote) within a quasiquote template MUST evaluate `<expr>` normally and insert its result into the AST being constructed at that position.
//= spec/capabilities/metaprogramming.md#quasiquote-constructs-ast-with-selective-evaluation
//# Quasiquote MUST nest, so that ``` ``(+ ,,x)``` evaluates the inner `,` to produce `` `(+ ,<x-value>)``.
pub fn reify_quotes(ast: &mut Arenas) -> Vec<StructId> {
    // Snapshot the pre-existing node count: only ORIGINAL nodes can be a source quote, and reification
    // APPENDS (ids >= this bound), so the scan must not consider its own output. Descending id order so
    // an outer quote is reified from its body's original structure before its inner quotes are reached
    // (see the ordering note in the module docs).
    let original_len = ast.structure.len() as u32;
    // FAST BAIL for a quote-FREE program (the overwhelming common case). `reify_quotes` runs at EVERY
    // load, but everything below — the parent/child-index build + pattern downward-walk
    // (`pattern_position_nodes`), the binder scan (`binder_position_nodes`), and the reverse per-node
    // plan loop with its two allocating `as_form(id,"quote"/"quasiquote").map(to_vec)` probes — is pure
    // dead work when the program contains no `quote`/`quasiquote` FORM at all. A `(quote …)`/
    // `(quasiquote …)` node is a `List` headed by the NAME `quote`/`quasiquote`, so its head is a
    // `Leaf::Name("quote")`/`Leaf::Name("quasiquote")` in the leaf pool; if NO such name leaf exists,
    // there is no quote head anywhere and the whole pass is a no-op. A single O(leaves) scan (leaves are
    // interned once, far fewer than a structural walk's per-node string compares) is the cheap
    // over-approximation: it may fire spuriously only for a program that MENTIONS the identifier
    // `quote`/`quasiquote` without a quote form (e.g. a user def named `quote`), in which case we fall
    // through to the exact shape-driven logic below — same result, just not skipped.
    let has_quote_name = ast
        .leaves
        .iter()
        .any(|l| matches!(l, Leaf::Name(n) if n.as_ref() == "quote" || n.as_ref() == "quasiquote"));
    if !has_quote_name {
        return Vec::new();
    }
    // The set of PATTERN-POSITION nodes — a match-arm pattern slot and everything under it. A
    // quote/quasiquote here DESTRUCTURES (desugars to an `Ast.*` PATTERN via `reify_pattern`), it does
    // not construct. Built from a local parent map because this pass runs before `Db`'s `parent_index`.
    let pattern_nodes = pattern_position_nodes(ast, original_len);
    // The set of BINDER-POSITION nodes — a `def` SIGNATURE list `(NAME param…)` and a `fn` PARAMS list.
    // A user may DEFINE a function named `quote`/`quasiquote` (`def quote(x) = x + 2`), whose signature
    // is spelled `(quote x)` — a `(quote …)`-headed list that is NOT a quote EXPRESSION but a binding
    // form. `quote`/`quasiquote` are grammar heads recognized STRUCTURALLY in EXPRESSION position only
    // (exactly as `if`/`match`/`bin` are — those are freely definable because they never dispatch on a
    // signature); a signature/params list is never resolved as an expression. But reification is a
    // pre-pass over EVERY `(quote …)` node by shape, so without this exclusion it rewrote the signature
    // `(quote x)` into `(Ast.Name "x")`, erasing the parameter binder — the body's `x` then resolved
    // CDZ0101 "unbound". Leave a binder-position node untouched so the def scans as an ordinary function
    // named `quote` and its parameters bind. (Only the signature list ITSELF is excluded, not the body:
    // a genuine `(quote …)` in the def BODY still reifies.)
    let binder_nodes = binder_position_nodes(ast, original_len);
    // Quote-pattern nodes carrying a NON-FINAL `,@` splice — ill-formed (a rest binds the tail, meaningful
    // only last). Collected while scanning + reported CDZ0221 by `collect_faults` (the node is left
    // un-reified since `reify_pattern` bails on it).
    let mut nonfinal_splice: Vec<StructId> = Vec::new();
    let mut plans: Vec<QuotePlan> = Vec::new();
    #[cfg(test)]
    crate::db::REIFY_QUOTES_POSITION_SCAN_NODES.with(|c| c.set(c.get() + original_len as u64));
    for i in (0..original_len).rev() {
        let id = StructId(i);
        // A `(quote x)`/`(quasiquote x)` node that IS a def signature or fn params list is a BINDING
        // form, not a quote expression — skip it entirely so the parameter binder survives (see
        // `binder_position_nodes`). A user function named `quote` resolves as an ordinary def.
        if binder_nodes.contains(id.0 as usize) {
            continue;
        }
        // A `(quote FORM)`/`(quasiquote TEMPLATE)` in PATTERN position desugars to the equivalent `Ast.*`
        // PATTERN (`reify_pattern` — an unquote is a BINDER). Elsewhere it CONSTRUCTS: a `quote` reifies
        // INERTLY (structural — a quote body never evaluates), a `quasiquote` reifies ACTIVELY (depth 1 —
        // an `,e` at depth 1 evaluates). Any other arity is left for `resolve_quote`/`resolve_quasiquote`
        // to reject CDZ0201; a non-quote node is skipped. An INNER quote/quasiquote the enclosing
        // reification already consumed is orphaned by the time the descending scan reaches it, so
        // re-planning it is harmless dead work (module docs §Ordering).
        let in_pattern = pattern_nodes.contains(id.0 as usize);
        let reified = if let Some([form]) = ast
            .as_form(id, "quote")
            .map(<[StructId]>::to_vec)
            .as_deref()
        {
            if in_pattern {
                // A `(quote FORM)` pattern is inert structure as a pattern — same desugar as a quasiquote
                // pattern with no active holes (a literal by equality, a compound as a fixed-arity list).
                reify_pattern(ast, *form, false)
            } else {
                // Reify INERTLY. `None` = a leaf with no `Ast` variant yet, OR a STRAY unquote (`,x`/`,@x`
                // not under a quasiquote — a syntax error). Leave the quote for resolve: a missing-variant
                // body DECLINES (Todo), a stray unquote gets CDZ0003 (`resolve::resolve_unquote`).
                reify(ast, *form, false)
            }
        } else if let Some([tmpl]) = ast
            .as_form(id, "quasiquote")
            .map(<[StructId]>::to_vec)
            .as_deref()
        {
            if in_pattern {
                // A NON-FINAL `,@` in this pattern template is ill-formed — record the quasiquote node
                // for `collect_faults` to reject CDZ0221 (a rest binds the tail, meaningful only last).
                // Checked before reify so the node is caught even though `reify_pattern` bails on it.
                if template_has_nonfinal_pattern_splice(ast, *tmpl) {
                    nonfinal_splice.push(id);
                }
                // Desugar to the `Ast.*` PATTERN. `None` = an un-reifiable leaf, a non-final `,@`
                // (recorded above → CDZ0221), or an arity fault — leave for resolve.
                reify_pattern(ast, *tmpl, true)
            } else {
                // Reify ACTIVELY at depth 1. `None` = an un-reifiable leaf, an ACTIVE splice (`,@` —
                // deferred), or an arity fault — leave the quasiquote for `resolve_quasiquote`.
                reify_active(ast, *tmpl, 1)
            }
        } else {
            continue;
        };
        if let Some(reified) = reified {
            plans.push(QuotePlan { quote: id, reified });
        }
    }
    for plan in plans {
        // Overwrite the quote node with a COPY of the reified root's structure, so the quote's own
        // `StructId` (and its span) is preserved as the result value's node.
        let root = ast.get(plan.reified).clone();
        ast.structure[plan.quote.0 as usize] = root;
        // BLANK the now-duplicate original root (`plan.reified` is a higher-id APPENDED node that lists
        // the SAME children as the copy just written into `plan.quote`). `parent_index` records the
        // LAST (highest-id) parent per child, so leaving `plan.reified` intact would make the shared
        // children's parent the ORPHAN root (its own parent is `None`) — a scope-walk dead end. Harmless
        // for a plain quote (all-fresh children, never walked up from), but an ACTIVE unquote REUSES its
        // live operand: that operand must resolve through `plan.quote`'s ancestors (the enclosing
        // `let`/`def`), so its parent must be the copy, not the orphan. Emptying the orphan (it is
        // unreachable — nothing references `plan.reified`) drops its claim, leaving `plan.quote` the sole
        // parent of the shared subtree. (`plan.reified` >= original_len and `plan.quote` < original_len,
        // so this never clobbers another plan's quote node.)
        ast.structure[plan.reified.0 as usize] = Struct::List(Vec::new());
    }
    nonfinal_splice
}

/// Whether a quote-PATTERN template contains a NON-FINAL `,@` splice — an `(unquote-splicing …)` that is
/// not the last element of its enclosing list (`` `(f ,@init ,last) `` — `,@init` before `,last`). A
/// `,@` binds the tail, so it is meaningful only LAST; a non-final one is ill-formed (CDZ0221). Walks the
/// template structurally (an `unquote`'s own operand is a sub-pattern, not evaluated here).
fn template_has_nonfinal_pattern_splice(ast: &Arenas, node: StructId) -> bool {
    let Struct::List(items) = ast.get(node) else {
        return false;
    };
    let items = items.clone();
    let last = items.len().saturating_sub(1);
    for (i, &child) in items.iter().enumerate() {
        // A `(unquote-splicing …)` child that is not in the final position is the fault.
        if ast.head_name(child) == Some("unquote-splicing") && i != last {
            return true;
        }
        // Recurse into a nested list child (a deeper template level).
        if matches!(ast.get(child), Struct::List(_))
            && template_has_nonfinal_pattern_splice(ast, child)
        {
            return true;
        }
    }
    false
}

/// The set of original node ids that sit in PATTERN position — a match arm's pattern slot and every
/// descendant of it. A quote/quasiquote in this set DESTRUCTURES (desugars to an `Ast.*` PATTERN), the
/// rest CONSTRUCT. Returns a bitset over `0..original_len`.
///
/// A match is `(match scrutinee (pattern body)…)`: the arm is a 2-element list `(pattern body)` whose
/// parent's head is `match` and which is NOT the scrutinee (arm index ≥ 1 among `match`'s children —
/// child 0 is `match`, child 1 the scrutinee, children ≥ 2 the arms). The arm's FIRST child is the
/// pattern slot; every node reachable under it is pattern position. Runs before `Db`'s `parent_index`,
/// so it builds its own parent map over the ORIGINAL nodes.
fn pattern_position_nodes(ast: &Arenas, original_len: u32) -> BitSet {
    // Local parent + child-index map over the original nodes (mirrors `db::parent_index`).
    let mut parent = vec![u32::MAX; original_len as usize];
    let mut child_ix = vec![0u32; original_len as usize];
    for i in 0..original_len as usize {
        if let Struct::List(children) = &ast.structure[i] {
            for (pos, &c) in children.iter().enumerate() {
                if (c.0) < original_len {
                    parent[c.0 as usize] = i as u32;
                    child_ix[c.0 as usize] = pos as u32;
                }
            }
        }
    }
    // The pattern-SLOT roots: each match arm's first child. An arm is a 2-list at child-index ≥ 2 of a
    // `(match …)` node (index 0 = `match`, 1 = scrutinee), so its pattern is that arm's child 0.
    let mut roots: Vec<StructId> = Vec::new();
    for i in 0..original_len as usize {
        if ast.head_name(StructId(i as u32)) != Some("match") {
            continue;
        }
        let Struct::List(children) = &ast.structure[i] else {
            continue;
        };
        // children: [match, scrutinee, arm, arm, …] — arms are index ≥ 2.
        for &arm in children.iter().skip(2) {
            if let Struct::List(pb) = &ast.structure[arm.0 as usize]
                && pb.len() == 2
            {
                roots.push(pb[0]); // the pattern slot
            }
        }
    }
    // Mark each pattern-slot root and everything under it (a downward walk over the child lists).
    let mut in_pattern = BitSet::new(original_len as usize);
    let mut stack = roots;
    while let Some(n) = stack.pop() {
        if n.0 >= original_len || in_pattern.contains(n.0 as usize) {
            continue;
        }
        in_pattern.insert(n.0 as usize);
        if let Struct::List(children) = &ast.structure[n.0 as usize] {
            stack.extend(children.iter().copied());
        }
    }
    in_pattern
}

/// The set of original node ids that are a BINDER-position list — a `def` SIGNATURE `(NAME param…)` or
/// a `fn` PARAMS list `(param…)`. A `(quote …)`/`(quasiquote …)`-headed list here is NOT a quote
/// expression: it is the signature of a user function named `quote`/`quasiquote` (`def quote(x) = …` →
/// signature `(quote x)`) or a params list whose first parameter is so named. These heads are grammar
/// forms the resolver dispatches STRUCTURALLY in EXPRESSION position only — exactly as `if`/`match`/`bin`
/// (all freely definable, because a signature is never resolved as an expression). Reification is a
/// shape-driven PRE-PASS, though, so without this exclusion it rewrote the signature into `(Ast.Name …)`
/// and erased the parameter binders. Only the signature/params LIST ITSELF is marked (not its
/// descendants): a param is a bare name or a `(: name T)` annotation, never a quote to reify, and a
/// genuine `(quote …)` in the def BODY must still reify. Returns a bitset over `0..original_len`.
fn binder_position_nodes(ast: &Arenas, original_len: u32) -> BitSet {
    let mut binder = BitSet::new(original_len as usize);
    for i in 0..original_len as usize {
        let id = StructId(i as u32);
        // A def's signature is its first tail element; a fn's params list likewise. Marking by the
        // enclosing `def`/`fn` (rather than by the list's own head) is precise: it flags a list ONLY
        // when it genuinely occupies the binder slot, never a same-shaped list elsewhere.
        let binder_list = ast
            .as_form(id, "def")
            .or_else(|| ast.as_form(id, "fn"))
            .and_then(|tail| tail.first().copied());
        if let Some(list) = binder_list
            && list.0 < original_len
            && matches!(ast.get(list), Struct::List(_))
        {
            binder.insert(list.0 as usize);
        }
    }
    binder
}

/// A tiny fixed-size bitset over `0..n` (a `Vec<u64>` of words) — enough for the pattern-position mark.
struct BitSet {
    words: Vec<u64>,
}

impl BitSet {
    fn new(n: usize) -> BitSet {
        BitSet {
            words: vec![0u64; n.div_ceil(64)],
        }
    }
    fn insert(&mut self, i: usize) {
        self.words[i / 64] |= 1u64 << (i % 64);
    }
    fn contains(&self, i: usize) -> bool {
        self.words
            .get(i / 64)
            .is_some_and(|w| w & (1u64 << (i % 64)) != 0)
    }
}

/// Build the `Ast` value that denotes the syntax at `node`, returning the root of the fresh
/// construction tree — or `None` if `node` (or any descendant) is un-reifiable. Purely structural:
/// recurses by SHAPE. `under_qq` is whether `node` sits under a `(quasiquote …)` template WITHIN the
/// quote body; it starts false at the quote body and turns true at the first `quasiquote` head.
///
/// Two reasons a reification bails (`None`):
///  - a leaf the `Ast` sum cannot carry (a Char/Sym/Bytes literal — the realized set is `Int`/`Float`/`Bool`/`Str`/`Name`/`List`);
///  - a STRAY `unquote`/`unquote-splicing` (`under_qq` still false): an escape outside any quasiquote is
///    a syntax error (`metaprogramming.md` §Quasiquote Constructs AST With Selective Evaluation), and a
///    plain `(quote …)` body is inert data — NOT a template — so `(quote (g ,x))` must reject CDZ0003,
///    not be reified. Bailing leaves the quote un-rewritten for `resolve::resolve_unquote` to code. An
///    escape UNDER a quasiquote (`under_qq` true — `(quote `(+ ,x))`) is ordinary inert structure and
///    reifies as an `Ast.List` mentioning the name (never evaluated), the corpus's inert-nesting case.
fn reify(ast: &mut Arenas, node: StructId, under_qq: bool) -> Option<StructId> {
    reify_inner(ast, node, under_qq, true)
}

/// Reify `node` PURELY STRUCTURALLY — every atom to its `Ast.*` leaf variant, every list to `Ast.List` —
/// with NO quasiquote/unquote escape interpretation (`unquote`/`quasiquote` heads are ordinary names,
/// captured as data like any other). This is the faithful AST-reflection reifier used by import
/// reflection (`link.rs`, the `__ast__` binding): it reflects a module's canonical AST as data, so a
/// module that happens to contain the word `unquote`/`quasiquote` is reflected verbatim, not
/// mis-interpreted as a template. Returns `None` only when a descendant leaf has no `Ast` variant (a
/// Char/Sym literal — the realized `Ast` set is `Int`/`Float`/`Bool`/`Str`/`Name`/`List`/`Bytes`), so the
/// reflection declines rather than miscompiling. Mutates `ast` in place (appends the construction tree),
/// exactly as `reify` does; the appended tree is self-contained (fresh nodes, cloned leaf values), so it
/// can be produced against the merged link arena and referenced by a synthesized def.
pub(crate) fn reflect_document(ast: &mut Arenas, node: StructId) -> Option<StructId> {
    // `under_qq = true` disables the stray-escape bail (there is no quote/quasiquote context here — we are
    // reflecting arbitrary module syntax, not a quote body), so escapes reflect as ordinary structure.
    // `ground_ints = true` — value position, so an int literal grounds to the `Ast.Int` `BigInt` payload.
    reify_inner(ast, node, true, true)
}

/// Does `ast` contain a `(. Ast module)` form anywhere — the `Ast.module` self-reflection intrinsic? A
/// cheap structural scan used to GATE the pre-resolve source snapshot: the snapshot (a per-file arena
/// clone the `Prim::ReflectModule` fill reflects from) is captured ONLY for a file that actually
/// self-reflects, so the overwhelmingly common program that never mentions `Ast.module` pays NO clone.
/// Precise (not a bare `module` name-leaf scan — `module` is also the module-declaration keyword): it
/// matches the exact `(. Ast module)` member form, mirroring how the fill occurrence resolves.
pub(crate) fn contains_ast_module(ast: &Arenas) -> bool {
    (0..ast.structure.len() as u32).any(|i| {
        let id = StructId(i);
        matches!(ast.as_form(id, "."), Some(tail)
            if tail.len() == 2
                && ast.as_name(tail[0]) == Some("Ast")
                && ast.as_name(tail[1]) == Some("module"))
    })
}

/// Does `ast` contain a `(. Type ast)` / `(. Type ast-generic)` member form — the type→AST reflection
/// intrinsics (`Type.ast` / `Type.ast-generic`, `DESIGN-type-to-ast-reflection.md`)? Same cheap structural
/// scan + snapshot-gate role as [`contains_ast_module`]: the type→AST fold reflects the type declaration's
/// PRE-RESOLVE source (the live arena is rewritten before lowering), so a file that uses it must capture a
/// source snapshot. Precise — matches the exact member forms, not a bare `ast` name-leaf scan.
pub(crate) fn contains_type_ast(ast: &Arenas) -> bool {
    (0..ast.structure.len() as u32).any(|i| {
        let id = StructId(i);
        matches!(ast.as_form(id, "."), Some(tail)
            if tail.len() == 2
                && ast.as_name(tail[0]) == Some("Type")
                && matches!(ast.as_name(tail[1]), Some("ast" | "ast-generic")))
    })
}

/// The shared body of `reify`, parameterized by whether an integer-literal payload is GROUNDED to
/// `BigInt`. In VALUE position (`ground_ints` true — the reifier building an `Ast` value) a bare int
/// literal is wrapped `(: N BigInt)` so it grounds to `Ast.Int`'s `BigInt` payload. In PATTERN position
/// (`ground_ints` false — `reify_pattern`'s leaf delegate) the payload is left BARE: a `(: N BigInt)`
/// ascription is not a pattern, and the nested literal-pattern probe (`lower.rs`) tests a bare literal
/// against the `BigInt` sub-value directly (`Probe::Int` already carries an arbitrary-precision value).
fn reify_inner(
    ast: &mut Arenas,
    node: StructId,
    under_qq: bool,
    ground_ints: bool,
) -> Option<StructId> {
    match ast.get(node) {
        Struct::Atom(l) => match ast.leaf(*l).clone() {
            // An integer literal -> `(Ast.Int N)`. The payload REUSES the literal's exact value/radix by
            // cloning the leaf, so the reified constant reads back identically.
            leaf @ Leaf::Int { .. } => {
                let payload = push_atom(ast, leaf);
                let payload = if ground_ints {
                    ast_bigint_payload(ast, payload)
                } else {
                    payload
                };
                Some(ast_ctor(ast, "Int", payload))
            }
            // A float literal -> `(Ast.Float d)`. `Ast.Float` carries a `Float64` payload (a float is a
            // syntactic form — `type-system.md`). The payload REUSES the literal's `Decimal` leaf, so the
            // reified constant reads back to the exact same double (its canonical bits are stable).
            leaf @ Leaf::Float(_) => {
                let payload = push_atom(ast, leaf);
                Some(ast_ctor(ast, "Float", payload))
            }
            // A boolean literal -> `(Ast.Bool b)`. `Ast.Bool` carries a `Bool` payload (`type-system.md`
            // §The Abstract Syntax Tree Is An Ordinary Sum Type — "a boolean" is a syntactic form), so the
            // reified `true`/`false` reads back identically. The payload REUSES the literal's leaf.
            leaf @ Leaf::Bool(_) => {
                let payload = push_atom(ast, leaf);
                Some(ast_ctor(ast, "Bool", payload))
            }
            // A string LITERAL -> `(Ast.Str "…")`. `Ast.Str` carries a `String` payload (a string is a
            // syntactic form — `type-system.md`), DISTINCT from `Ast.Name` (an identifier reference): the
            // reified node carries the string's TEXT as a String literal, so `(quote "hi")` reads back as
            // the string value, not a name. The payload REUSES the literal's leaf.
            leaf @ Leaf::Str(_) => {
                let payload = push_atom(ast, leaf);
                Some(ast_ctor(ast, "Str", payload))
            }
            // A bare name -> `(Ast.Name "foo")`. The name TEXT becomes a String payload (`Ast.Name`
            // carries the identifier as a String — `type-system.md`), so the identifier is captured as
            // data, not resolved as a reference.
            Leaf::Name(name) => {
                let payload = push_atom(ast, Leaf::Str(name));
                Some(ast_ctor(ast, "Name", payload))
            }
            // A byte-sequence LITERAL (`b"…"`) -> `(Ast.Bytes b"…")`. `Ast.Bytes` carries a `Bytes` payload
            // (a raw blob is a syntactic form — operator seq 113), so `(quote b"hi")` reifies to a single
            // bytes node whose payload REUSES the literal's leaf; it rides the AST codec as one
            // length-prefixed raw-bytes leaf (`KIND_BYTES`), not a node-per-byte list.
            leaf @ Leaf::Bytes(_) => {
                let payload = push_atom(ast, leaf);
                Some(ast_ctor(ast, "Bytes", payload))
            }
            // A CHAR literal (`#\a`) -> `(Ast.Char #\a)`. `Ast.Char` carries a `Char` payload (a char is a
            // syntactic form — `type-system.md`), so the reified node captures the exact scalar; the
            // payload REUSES the literal's leaf. This (with `Symbol` below) makes reflection/quote TOTAL
            // over syntax leaves — a char literal reflects instead of declining (operator directive).
            leaf @ Leaf::Char(_) => {
                let payload = push_atom(ast, leaf);
                Some(ast_ctor(ast, "Char", payload))
            }
            // A SYMBOL literal (`#"x"`) -> `(Ast.Symbol #"x")`. `Ast.Symbol` carries a `Symbol` payload
            // (DISTINCT from `Ast.Name`'s String and `Ast.Str`'s String — a symbol is the nominal
            // member-key form), so `(quote #"x")` reads back as the symbol value. The payload REUSES the
            // literal's leaf.
            leaf @ Leaf::Sym(_) => {
                let payload = push_atom(ast, leaf);
                Some(ast_ctor(ast, "Symbol", payload))
            }
            // A TYPE-SUFFIXED numeric literal (`5N`/`0.5R`, a `Leaf::Suffixed`). The reader represents such a
            // literal as the annotation `(: <this Suffixed leaf> BigInt|Rational)` — the leaf carries the
            // numeric value AND its suffix (so the printer re-emits `5N`, not `(: 5 BigInt)`). It has no
            // dedicated `Ast` variant, so reify its BODY as the matching numeric leaf (an integer body ->
            // `Ast.Int`, a float body -> `Ast.Float`). The suffix marker is redundant with the enclosing
            // `(: … BigInt)` annotation the reader already produced, so `(quote 5N)` reifies to the SAME
            // `Ast` value as `(quote (: 5 BigInt))` and `(quote 0.5R)` as `(quote (: 0.5 Rational))` — the
            // metaprogramming face of the reader's suffix-is-an-annotation rule. Mirrors the body extraction
            // in `suffixed::normalize`, so the quoted + non-quoted spellings agree. Without this arm the
            // whole quote declined ("quote produces an AST value, not supported") on the un-reifiable leaf.
            Leaf::Suffixed { value, .. } => {
                let body_leaf = match value {
                    crate::ast::SuffixBody::Int { value, radix } => Leaf::Int { value, radix },
                    crate::ast::SuffixBody::Float(d) => Leaf::Float(d),
                };
                if let Leaf::Int { .. } = body_leaf {
                    let payload = push_atom(ast, body_leaf);
                    let payload = if ground_ints {
                        ast_bigint_payload(ast, payload)
                    } else {
                        payload
                    };
                    Some(ast_ctor(ast, "Int", payload))
                } else {
                    let payload = push_atom(ast, body_leaf);
                    Some(ast_ctor(ast, "Float", payload))
                }
            }
            // A reader ERROR-RECOVERY marker leaf (`BadChar`/`BadEscape`) has no `Ast` variant — it only
            // arises from MALFORMED source (which does not compile), so leaving it un-reifiable is not a
            // real reflection gap: bail (decline) rather than miscompile.
            _ => None,
        },
        Struct::List(items) => {
            let items = items.clone();
            // A STRAY escape outside a quasiquote → bail (CDZ0003, see the doc comment). Checked before
            // reifying children so `(quote (g ,x))`'s `,x` is caught. A quasiquote HEAD opens a template
            // context for its descendants (`under_qq` becomes true); the escape's own children stay inert.
            let head = items.first().and_then(|&h| ast.as_name(h));
            let head_is_escape = matches!(head, Some("unquote") | Some("unquote-splicing"));
            if head_is_escape && !under_qq {
                return None;
            }
            let child_under_qq = under_qq || head == Some("quasiquote");
            // A NATIVE MEMBER access `(. obj key)` (a `Leaf::Member` head) → the dedicated `Ast.Member
            // (tuple <reify obj> <reify key>)` (spec: a reflected member access is an `Ast.Member`, not a
            // name-headed node). Without this the `Leaf::Member` head hit `reify_inner`'s `_ => None` and a
            // `(quote (String.byte-len …))` / `(quote (List.at …))` — whose head is `(. String byte-len)` /
            // `(. List at)` — was left un-reified → `eval` declined CDZ0101.
            if let Some((obj, key)) = ast.member_parts(node) {
                let robj = reify(ast, obj, child_under_qq)?;
                let rkey = reify(ast, key, child_under_qq)?;
                let th = push_atom(ast, Leaf::Name("tuple".into()));
                let tup = push_list(ast, vec![th, robj, rkey]);
                return Some(ast_ctor(ast, "Member", tup));
            }
            // A NATIVE RATIONAL literal `3/2` (a `(RationalTag <num> <den>)` node headed by `Leaf::Rational`)
            // → the dedicated `Ast.Rational (tuple <reify num> <reify den>)` (spec: a reflected rational
            // literal is its own first-class variant, not a name-headed node). The num/den children are
            // ordinary `Leaf::Int` value leaves, so each reifies to an `Ast.Int`. Without this the
            // `Leaf::Rational` head hit `reify_inner`'s leaf-match `_ => None` and the whole `(quote 3/2)`
            // declined ("quote produces an AST value, not supported").
            if let Some((num, den)) = ast.rational_parts(node) {
                let rnum = reify(ast, num, child_under_qq)?;
                let rden = reify(ast, den, child_under_qq)?;
                let th = push_atom(ast, Leaf::Name("tuple".into()));
                let tup = push_list(ast, vec![th, rnum, rden]);
                return Some(ast_ctor(ast, "Rational", tup));
            }
            // A NATIVE COLLECTION-CTOR head (`#list`/`#tuple`/`#record`/`#map`/`#set`, a `Leaf::Ctor`) —
            // reflect to the DEDICATED first-class `Ast.<X>Ctor` variant carrying the reified CHILDREN with
            // NO head (spec: metaprogramming.md §"Quoting a collection construction … MUST produce that
            // collection's own first-class AST variant … rather than a name-headed generic node"). A
            // List/Tuple/Set carries the bare element ASTs; a Record/Map carries `Ast.FieldPair` children
            // (each `(= k v)` entry → a `(Tuple <reify k> <reify v>)`). This is DISTINCT from the generic
            // `Ast.List` below (a NAME-headed node — a form/application `(if …)`/`(f a)`, or the name-alias
            // `(list 1 2 3)`), so the two spellings reflect distinctly (they ARE different syntax). Without
            // this the ctor-leaf head hit `reify_inner`'s `_ => None` and the whole `(quote #list …)` was
            // left un-reified → `eval` declined CDZ0101 on a compile-time-visible form (the eval-fold reds).
            if let Some(&h) = items.first()
                && let Some((variant, fieldpair_children)) = native_ctor_variant(ast, h)
            {
                let mut children = Vec::with_capacity(items.len() - 1);
                for &child in &items[1..] {
                    // A `(.. rest)` REST MARKER inside a quoted record/map PATTERN (an OPEN pattern) is not a
                    // `(= k v)` entry — reify it via the generic path (→ `Ast.List [Ast.Name "..", …]`), NOT
                    // `reify_field_pair` (whose `field_kv` 2-list fallback would mis-read `(.. r)` as a
                    // `(= .. r)` pair, CLOSING the pattern → the quoted map/record-rest match fell through to
                    // the catch-all: the #6855 map-rest wrong-value fold). Reconstruct restores the marker.
                    let reified = if fieldpair_children && ast.as_form(child, "..").is_none() {
                        reify_field_pair(ast, child, child_under_qq)?
                    } else {
                        reify(ast, child, child_under_qq)?
                    };
                    children.push(reified);
                }
                let payload = list_form(ast, children);
                return Some(ast_ctor(ast, variant, payload));
            }
            // A compound `(a b c …)` -> `(Ast.List (list <reify a> <reify b> …))`. Reify every child
            // (bailing if any is un-reifiable), then wrap in a `list` constructor, then in `Ast.List`.
            let mut reified_children = Vec::with_capacity(items.len());
            for child in items {
                reified_children.push(reify(ast, child, child_under_qq)?);
            }
            Some(wrap_ast_list(ast, reified_children))
        }
    }
}

/// Reify a QUASIQUOTE template at nesting `depth` (the quasiquote body is depth 1; a nested `quasiquote`
/// bumps it, an `unquote`/`unquote-splicing` head drops it — the Bawden depth algorithm). Returns the
/// root of the fresh `Ast` construction tree, or `None` (bail — leave for resolve) on an un-reifiable
/// leaf, an ACTIVE splice, or an arity fault. Selective evaluation happens at an ACTIVE unquote (see the
/// module docs §Quasiquote):
///  - `(unquote e)` at depth 1 → ACTIVE: reuse `e` LIVE, wrap in the `Ast.*` ctor matching its VALUE —
///    `e` is evaluated as ordinary code (unbound name → CDZ0101). A value LITERAL dispatches by kind (Int
///    → `Ast.Int`, Float → `Ast.Float`, Bool → `Ast.Bool`, String → `Ast.Str`; a Char/Sym/Bytes literal
///    bails — no value variant). A runtime operand (a NAME or a call) is wrapped in the `ast-lift`
///    intrinsic and resolved by its INFERRED type at `lower` (identity when already an `Ast`, else the
///    matching leaf ctor).
///  - `(unquote-splicing e)` at depth 1 → ACTIVE splice: BAIL (deferred — leave for resolve).
///  - `(unquote e)` at depth>1 → inert `(Ast.List (list (Ast.Name "unquote") <reify e @ depth-1>))`.
///  - `(quasiquote t)` at any depth → inert `(Ast.List (list (Ast.Name "quasiquote") <reify t @ depth+1>))`.
///  - any other node → same STRUCTURAL reification as a plain quote, recursing at the SAME depth.
fn reify_active(ast: &mut Arenas, node: StructId, depth: u32) -> Option<StructId> {
    let Struct::List(items) = ast.get(node) else {
        // A leaf (int/name/…) is depth-independent structure — reify it exactly as a plain quote does.
        // `under_qq=true` so a bare stray unquote can't arise (a leaf is never an escape head anyway).
        return reify(ast, node, true);
    };
    let items = items.clone();
    let head = items
        .first()
        .and_then(|&h| ast.as_name(h))
        .map(str::to_string);
    match head.as_deref() {
        // An unquote at depth 1 is ACTIVE. `unquote` evaluates + embeds; `unquote-splicing` splices a
        // list's elements (deferred → bail). Arity ≠ 1 → bail for `resolve_unquote`'s CDZ0201.
        Some("unquote") if depth == 1 => {
            if items.len() != 2 {
                return None;
            }
            // Embed the LIVE operand as the `Ast` leaf its VALUE denotes. The operand stays live (it is
            // evaluated code, not reified) so an unbound name in it is still the ordinary CDZ0101.
            //
            // A VALUE LITERAL's kind is known structurally HERE, so dispatch directly to the matching leaf
            // ctor (Int → `Ast.Int`, Float → `Ast.Float`, Bool → `Ast.Bool`, String → `Ast.Str`) — no
            // runtime type needed. A literal the `Ast` sum has no value variant for (a Char/Sym/Bytes)
            // BAILS (declines honestly).
            //
            // KEYSTONE: A RUNTIME operand — a `Leaf::Name` (a let-bound var `,n` / a param) or a non-leaf computed
            // expression (`,(f x)`) — has an unknown type at reify time (this runs pre-typecheck). Wrap it
            // in the compiler-internal `(ast-lift e)` intrinsic, which `lower` resolves by the operand's
            // INFERRED type: IDENTITY when `e` is already an `Ast` (splice a sub-tree), else wrap in the
            // matching `Ast.Int`/`Bool`/`Str` leaf. This replaces the old unconditional `(Ast.Int e)` wrap,
            // which type-errored a non-Int runtime operand against `Ast.Int`'s Int64 payload
            // (`[[unquote-computed-ast-needs-inferred-type-lift]]`).
            match ast.get(items[1]) {
                Struct::Atom(l) => match ast.leaf(*l) {
                    Leaf::Int { .. } => {
                        // Ground the literal payload to `BigInt` (the `Ast.Int` payload type) before
                        // wrapping — bind the inner result first (a direct
                        // `ast_ctor(ast, "Int", ast_bigint_payload(ast, items[1]))` double-mut-borrows
                        // `ast`, E0499).
                        let payload = ast_bigint_payload(ast, items[1]);
                        Some(ast_ctor(ast, "Int", payload))
                    }
                    Leaf::Float(_) => Some(ast_ctor(ast, "Float", items[1])),
                    Leaf::Bool(_) => Some(ast_ctor(ast, "Bool", items[1])),
                    Leaf::Str(_) => Some(ast_ctor(ast, "Str", items[1])),
                    // A NAME is a runtime reference — lift by inferred type at lower.
                    Leaf::Name(_) => Some(ast_lift(ast, items[1])),
                    // A literal with no value-carrying `Ast` variant yet (Char/Sym/Bytes) — bail.
                    _ => None,
                },
                // A non-leaf operand (a computed expression) — a runtime value, lift by inferred type.
                Struct::List(_) => Some(ast_lift(ast, items[1])),
            }
        }
        Some("unquote-splicing") if depth == 1 => None,
        // A nested unquote (depth>1) is INERT structure at depth-1; its head + operand reify structurally.
        Some(h @ ("unquote" | "unquote-splicing")) => {
            if items.len() != 2 {
                return None;
            }
            let inner = reify_active(ast, items[1], depth - 1)?;
            Some(reify_escape_list(ast, h, inner))
        }
        // A nested quasiquote is INERT structure at depth+1.
        Some("quasiquote") => {
            if items.len() != 2 {
                return None;
            }
            let inner = reify_active(ast, items[1], depth + 1)?;
            Some(reify_escape_list(ast, "quasiquote", inner))
        }
        // Any other compound: `(Ast.List <element-list>)`, recursing at the SAME depth so an active
        // unquote nested anywhere inside still fires. A child that is an ACTIVE `(unquote-splicing e)`
        // (depth 1) SPLICES e's elements into the parent (`metaprogramming.md`: ,@ evaluates its operand
        // to a LIST and splices its elements), so the element list is built by CONCATENATING runs of
        // ordinary reified elements with `(ast-splice-lift e)` segments (each lifts e's Int64 elements to
        // `Ast.Int` nodes). With NO splice child, this reduces to the plain `(list <reified…>)` form.
        _ => {
            // A NATIVE MEMBER access `(. obj key)` inside a quasiquote → `Ast.Member (tuple …)`, recursing
            // via `reify_active` so a nested active unquote in `obj`/`key` still fires (the quasiquote twin
            // of the plain-quote member branch in `reify_inner`). Covers a quasiquoted `(String.concat …)` /
            // `(Bytes.concat …)` whose head is a `(. Module op)` member.
            if let Some((obj, key)) = ast.member_parts(node) {
                let robj = reify_active(ast, obj, depth)?;
                let rkey = reify_active(ast, key, depth)?;
                let th = push_atom(ast, Leaf::Name("tuple".into()));
                let tup = push_list(ast, vec![th, robj, rkey]);
                return Some(ast_ctor(ast, "Member", tup));
            }
            // A NATIVE RATIONAL literal `3/2` inside a quasiquote → `Ast.Rational (tuple …)`, recursing via
            // `reify_active` (the quasiquote twin of the plain-quote rational branch in `reify_inner`).
            if let Some((num, den)) = ast.rational_parts(node) {
                let rnum = reify_active(ast, num, depth)?;
                let rden = reify_active(ast, den, depth)?;
                let th = push_atom(ast, Leaf::Name("tuple".into()));
                let tup = push_list(ast, vec![th, rnum, rden]);
                return Some(ast_ctor(ast, "Rational", tup));
            }
            // A NATIVE COLLECTION-CTOR head inside a quasiquote — reflect to the dedicated `Ast.<X>Ctor`
            // (bare children, no head), exactly as the plain-quote path (`reify_inner`), but recursing via
            // `reify_active` so a nested active unquote inside the collection still fires. Record/Map carry
            // `Ast.FieldPair` children (their `(= k v)`/`(k v)` entries); List/Tuple/Set carry bare elements.
            // Covers a quasiquoted `#map`/`#record` value (e.g. inside a quasiquoted `(match #map(…) …)`), the
            // pattern-side gap that left the map-pattern eval-fold case declining CDZ0101.
            if let Some(&h) = items.first()
                && let Some((variant, fieldpair_children)) = native_ctor_variant(ast, h)
            {
                let mut children = Vec::with_capacity(items.len() - 1);
                for &child in &items[1..] {
                    // A `(.. rest)` REST MARKER inside a quasiquoted record/map PATTERN (an OPEN pattern) is
                    // not a `(= k v)` entry — reify it via the generic active path, NOT `field_kv` (whose
                    // 2-list fallback mis-reads `(.. r)` as a `(= .. r)` pair, CLOSING the pattern → the
                    // map/record-rest match falls through: the quasiquote twin of the #6855 map-rest fold).
                    let reified = if fieldpair_children && ast.as_form(child, "..").is_none() {
                        let (k, v) = field_kv(ast, child)?;
                        let rk = reify_active(ast, k, depth)?;
                        let rv = reify_active(ast, v, depth)?;
                        let th = push_atom(ast, Leaf::Name("tuple".into()));
                        let tup = push_list(ast, vec![th, rk, rv]);
                        ast_ctor(ast, "FieldPair", tup)
                    } else {
                        reify_active(ast, child, depth)?
                    };
                    children.push(reified);
                }
                let payload = list_form(ast, children);
                return Some(ast_ctor(ast, variant, payload));
            }
            let has_active_splice = items.iter().any(|&c| {
                depth == 1
                    && ast.head_name(c) == Some("unquote-splicing")
                    && matches!(ast.get(c), Struct::List(l) if l.len() == 2)
            });
            if !has_active_splice {
                let mut reified_children = Vec::with_capacity(items.len());
                for child in items {
                    reified_children.push(reify_active(ast, child, depth)?);
                }
                return Some(wrap_ast_list(ast, reified_children));
            }
            // Build the element list as a concat of segments: a run of ordinary reified elements is a
            // `(list e…)`; an active `(unquote-splicing e)` is `(ast-splice-lift e)` (e stays LIVE). The
            // segments concat left-to-right (`List.concat`), then wrap in `(Ast.List …)`.
            let mut segments: Vec<StructId> = Vec::new();
            let mut run: Vec<StructId> = Vec::new();
            for child in &items {
                if depth == 1
                    && let Some([spliced]) = ast
                        .as_form(*child, "unquote-splicing")
                        .map(<[StructId]>::to_vec)
                        .as_deref()
                {
                    // Flush the pending ordinary run as one `(list …)` segment, then the lift segment.
                    if !run.is_empty() {
                        segments.push(list_form(ast, std::mem::take(&mut run)));
                    }
                    segments.push(ast_splice_lift(ast, *spliced));
                } else {
                    run.push(reify_active(ast, *child, depth)?);
                }
            }
            if !run.is_empty() {
                segments.push(list_form(ast, run));
            }
            // Concat the segments left-to-right into ONE element list, then `(Ast.List <it>)`. At least one
            // segment exists (a splice child guaranteed it).
            let mut elem_list = segments[0];
            for &seg in &segments[1..] {
                elem_list = list_concat(ast, elem_list, seg);
            }
            Some(ast_ctor(ast, "List", elem_list))
        }
    }
}

/// Reify a quote/quasiquote TEMPLATE in PATTERN position into the equivalent `Ast.*` constructor PATTERN
/// (module docs §Quote PATTERNS). Same STRUCTURAL shape as construction, but an unquote is a BINDER
/// (its operand is a sub-PATTERN reused live), not an evaluation. `under_qq` is whether we're inside a
/// quasiquote template (a plain `(quote …)` pattern has no active `,`, so `under_qq=false`). Returns the
/// pattern root, or `None` (bail — leave for resolve) on an un-reifiable leaf, an arity fault, or a
/// non-final `,@` (ill-formed — `resolve`/lowering reject it CDZ0221). Selective:
///  - integer `N` → `(Ast.Int N)`, bare name `foo` → `(Ast.Name "foo")` — literal, matches by equality.
///  - `(unquote P)` → the sub-pattern `P` reused LIVE (a bare name binds; a nested `(Ast.Int n)` matches).
///  - `(quasiquote t)` under a template → nest one level: reify `t` structurally (its `,` are deeper).
///  - `(unquote-splicing name)` as the FINAL element of a compound → a `.. name` REST binder in the list
///    pattern (binds the remaining elements); anywhere else → bail (a non-final splice is CDZ0221).
///  - compound `(a b c)` → `(Ast.List (list <pat a> <pat b> <pat c>))` — a fixed-arity list pattern.
///
/// Because this desugars a quasiquote pattern to the EQUIVALENT `Ast.*` sum-constructor pattern, matching
/// through it is indistinguishable (by structural equality or encoding) from matching through the
/// constructors directly; a literal subterm matches by equality, a `,<P>` unquote matches its sub-tree
/// against `P` (binding when `P` is a name), and a final `,@name` splice binds the remaining list
/// elements; exhaustiveness is the ordinary match rule (an all-quote-pattern match needs a catch-all or
/// is CDZ0210); and it layers over the untyped `Ast` substrate so it destructures arbitrary tree shape.
//= spec/capabilities/metaprogramming.md#a-quasiquote-in-pattern-position-destructures-an-ast
//# A quasiquote template `` `<template>`` appearing in pattern position MUST destructure an abstract-syntax-tree scrutinee, matching the template's structure against the tree.
//= spec/capabilities/metaprogramming.md#a-quasiquote-in-pattern-position-destructures-an-ast
//# A quasiquote pattern MUST be equivalent to the pattern formed from the corresponding abstract-syntax-tree sum constructors, so that a value matched through a quasiquote pattern cannot be distinguished by structural equality or by the encoding from the same value matched through the constructors.
//= spec/capabilities/metaprogramming.md#a-quasiquote-in-pattern-position-destructures-an-ast
//# A literal subterm within a quasiquote pattern MUST match the abstract-syntax-tree node it denotes by equality, and a `,<pattern>` (unquote) subterm MUST match the sub-tree at its position against `<pattern>`, binding the sub-tree when `<pattern>` is a name.
//= spec/capabilities/metaprogramming.md#a-quasiquote-in-pattern-position-destructures-an-ast
//# A `,@<name>` (unquote-splicing) subterm within a quasiquote pattern MUST bind the remaining elements of its enclosing list as a list, and MUST appear only as the final element of its template.
//= spec/capabilities/metaprogramming.md#a-quasiquote-in-pattern-position-destructures-an-ast
//# A match over an abstract-syntax-tree scrutinee whose arms are quasiquote patterns MUST be subject to the exhaustiveness rule exactly as any other match, so that a quasiquote pattern is not a special case (core-semantics.md §"Matching Is Exhaustive Or Rejected").
//= spec/capabilities/metaprogramming.md#a-quasiquote-in-pattern-position-destructures-an-ast
//# A quasiquote pattern MUST layer over the untyped abstract-syntax-tree analysis substrate, so that it may destructure arbitrary tree structure — the dual of the construction quote, which carries the type of the expression it builds (§"A Typed Quote Carries The Type Of The Expression It Builds").
fn reify_pattern(ast: &mut Arenas, node: StructId, under_qq: bool) -> Option<StructId> {
    let Struct::List(items) = ast.get(node) else {
        // A leaf: an integer/name literal reifies exactly as construction (matches by equality). A leaf
        // is never an escape head, so `under_qq` is irrelevant — reuse the structural `reify`. Pass
        // `ground_ints: false`: a literal in PATTERN position stays a bare `(Ast.Int N)` (no `(: N BigInt)`
        // ascription, which is not a pattern) — the nested literal-pattern probe tests it against the
        // `BigInt` sub-value directly.
        return reify_inner(ast, node, under_qq, false);
    };
    let items = items.clone();
    let head = items
        .first()
        .and_then(|&h| ast.as_name(h))
        .map(str::to_string);
    match head.as_deref() {
        // An `(unquote P)` binds/further-matches: its operand P is the sub-PATTERN, reused LIVE (a bare
        // name resolves as a match-arm binder, a nested ctor pattern further-matches). Only meaningful in
        // a quasiquote template; a `,` in a plain-quote pattern is inert structure (falls to the default).
        Some("unquote") if under_qq => {
            if items.len() != 2 {
                return None;
            }
            Some(items[1])
        }
        // A splice at top of a compound (not as the final element) is handled in the compound arm below;
        // a bare `(unquote-splicing …)` reached HERE (not inside a parent list) has no enclosing list to
        // bind the rest of → bail for resolve (CDZ0221 / decline).
        Some("unquote-splicing") if under_qq => None,
        // A nested `(quasiquote t)` inside a pattern template: reify `t` structurally one level in. (A
        // pattern that literally contains a nested quasiquote is exotic; treat it as inert structure.)
        Some("quasiquote") => {
            if items.len() != 2 {
                return None;
            }
            let inner = reify_pattern(ast, items[1], true)?;
            Some(reify_escape_list(ast, "quasiquote", inner))
        }
        // A compound `(a b c …)` → `(Ast.List (list <pat…>))`, a fixed-arity list pattern. A FINAL
        // `(unquote-splicing name)` element becomes a `.. name` REST binder (binds the remaining
        // elements); a non-final splice bails (CDZ0221 — a rest is meaningful only last).
        _ => {
            // A native MEMBER access `(. obj key)` PATTERN → `Ast.Member (tuple <pat obj> <pat key>)` — the
            // pattern-direction twin of the value-side member reflection (`reify_inner`/`reify_active`).
            if let Some((obj, key)) = ast.member_parts(node) {
                let robj = reify_pattern(ast, obj, under_qq)?;
                let rkey = reify_pattern(ast, key, under_qq)?;
                let th = push_atom(ast, Leaf::Name("tuple".into()));
                let tup = push_list(ast, vec![th, robj, rkey]);
                return Some(ast_ctor(ast, "Member", tup));
            }
            // A native COLLECTION-CTOR PATTERN (`#list`/`#tuple`/`#record`/`#map`/`#set`, a `Leaf::Ctor`
            // head) → the dedicated `Ast.<X>Ctor` (bare children, no head; record/map children are
            // `Ast.FieldPair`), reifying each child in PATTERN mode (`reify_pattern` — an `(unquote P)`
            // element/value is a BINDER, a literal matches by equality). The pattern-direction twin of the
            // value-side `Leaf::Ctor` reflection: without it a quasiquoted match arm whose pattern is a
            // native `#map((= 1 x))`/`#list(…)` left the ctor-leaf head un-reified → eval declined CDZ0101
            // (the 12th eval-fold red, the pattern-side gap `reify_inner`/`reify_active` didn't cover).
            // Fall through to the generic list-pattern path when a `,@`-splice rest is present (its `.. rest`
            // handling below) or the head is not a native ctor.
            let has_active_splice = under_qq
                && items
                    .iter()
                    .any(|&c| ast.as_form(c, "unquote-splicing").is_some());
            if !has_active_splice
                && let Some(&h) = items.first()
                && let Some((variant, fieldpair_children)) = native_ctor_variant(ast, h)
            {
                let mut children = Vec::with_capacity(items.len() - 1);
                for &child in &items[1..] {
                    let reified = if fieldpair_children && ast.as_form(child, "..").is_none() {
                        // A record/map entry `(= k v)` / `(k v)` → `Ast.FieldPair (tuple <pat k> <pat v>)`.
                        let (k, v) = field_kv(ast, child)?;
                        let rk = reify_pattern(ast, k, under_qq)?;
                        let rv = reify_pattern(ast, v, under_qq)?;
                        let th = push_atom(ast, Leaf::Name("tuple".into()));
                        let tup = push_list(ast, vec![th, rk, rv]);
                        ast_ctor(ast, "FieldPair", tup)
                    } else {
                        // A bare element (list/tuple/set) OR a `(.. rest)` REST MARKER inside a record/map
                        // pattern (an OPEN pattern binding the residual entries). Reify via the generic
                        // pattern path — for the rest marker this preserves the `..`-headed form as an
                        // `Ast.List [Ast.Name "..", <binder>]` bare (non-`FieldPair`) child, which
                        // reconstruct restores as the rest marker so the reflected pattern stays OPEN. Do
                        // NOT run `field_kv` on a `(.. r)` marker: its 2-element-list fallback would mis-read
                        // it as a `(= .. r)` field pair, CLOSING the pattern → a quoted `#map((= k v) (.. r))`
                        // match then found no field `..` and fell through to the catch-all (a wrong-value
                        // fold — the #6855 map-rest miscompile).
                        reify_pattern(ast, child, under_qq)?
                    };
                    children.push(reified);
                }
                let payload = list_form(ast, children);
                return Some(ast_ctor(ast, variant, payload));
            }
            let mut pat_children: Vec<StructId> = Vec::with_capacity(items.len());
            for (i, &child) in items.iter().enumerate() {
                let is_last = i + 1 == items.len();
                if under_qq
                    && let Some([spliced]) = ast
                        .as_form(child, "unquote-splicing")
                        .map(<[StructId]>::to_vec)
                        .as_deref()
                {
                    if !is_last {
                        return None; // a non-final `,@` — ill-formed (CDZ0221), bail for resolve
                    }
                    // FINAL `,@name` → a `.. name` rest binder: append the `..` marker then the bare
                    // binder (reused live), so the list pattern is `(list <pat…> .. name)`.
                    let dotdot = push_atom(ast, Leaf::Name("..".into()));
                    pat_children.push(dotdot);
                    pat_children.push(*spliced);
                } else {
                    pat_children.push(reify_pattern(ast, child, under_qq)?);
                }
            }
            Some(wrap_ast_list(ast, pat_children))
        }
    }
}

/// The inert reification of an escape/nesting head node `(HEAD inner)` → `(Ast.List (list (Ast.Name
/// "HEAD") <inner>))` where `<inner>` is the ALREADY-reified operand. So a quoted-but-not-active
/// `,`/`,@`/`` ` `` renders as the two-element list the reader produced (head name + operand), matching
/// the corpus nested-quasiquote value form.
fn reify_escape_list(ast: &mut Arenas, head: &str, inner: StructId) -> StructId {
    let head_payload = push_atom(ast, Leaf::Str(head.into()));
    let head_name = ast_ctor(ast, "Name", head_payload);
    wrap_ast_list(ast, vec![head_name, inner])
}

/// Wrap already-reified child `Ast` nodes in `(Ast.List (list <child…>))` — the shared tail of every
/// compound reification (plain-quote list, active-quasiquote list, escape-head list).
fn wrap_ast_list(ast: &mut Arenas, children: Vec<StructId>) -> StructId {
    let list_val = list_form(ast, children);
    ast_ctor(ast, "List", list_val)
}

/// If `head` is a NATIVE collection-ctor leaf (`#list`/`#tuple`/`#record`/`#map`/`#set`, a `Leaf::Ctor`),
/// the matching dedicated `Ast.<X>Ctor` variant name + whether its children are `Ast.FieldPair`
/// (record/map carry `(= k v)`/`(k v)` entries; list/tuple/set carry bare element ASTs). `None` for a
/// non-ctor head (a name-headed form/application, reflected as the generic `Ast.List`). Shared by the
/// plain-quote (`reify_inner`) and quasiquote (`reify_active`) reflection paths.
fn native_ctor_variant(ast: &Arenas, head: StructId) -> Option<(&'static str, bool)> {
    let Struct::Atom(l) = ast.get(head) else {
        return None;
    };
    match ast.leaf(*l) {
        Leaf::Ctor(crate::ast::CompoundCtor::List) => Some(("ListCtor", false)),
        Leaf::Ctor(crate::ast::CompoundCtor::Tuple) => Some(("TupleCtor", false)),
        Leaf::Ctor(crate::ast::CompoundCtor::Set) => Some(("SetCtor", false)),
        Leaf::Ctor(crate::ast::CompoundCtor::Record) => Some(("RecordCtor", true)),
        Leaf::Ctor(crate::ast::CompoundCtor::Map) => Some(("MapCtor", true)),
        _ => None,
    }
}

/// Reify one entry of a native `#record`/`#map` literal to an `Ast.FieldPair` — a `(Tuple <reify key>
/// <reify value>)` payload (spec: a reflected record/map is a `RecordCtor`/`MapCtor` of `FieldPair`
/// values). The entry is a record `(= k v)` (name-headed or the native `Leaf::FieldPair` leaf) or a map
/// `(k v)` 2-element pair. `None` if it is not a well-formed key/value entry (then the enclosing ctor bails).
fn reify_field_pair(ast: &mut Arenas, entry: StructId, under_qq: bool) -> Option<StructId> {
    // A well-formed `(= k v)` entry → `Ast.FieldPair (tuple <reify k> <reify v>)`.
    if let Some((k, v)) = field_kv(ast, entry) {
        let rk = reify(ast, k, under_qq)?;
        let rv = reify(ast, v, under_qq)?;
        let tuple_head = push_atom(ast, Leaf::Name("tuple".into()));
        let tup = push_list(ast, vec![tuple_head, rk, rv]);
        return Some(ast_ctor(ast, "FieldPair", tup));
    }
    // A MALFORMED record/map entry — a child that is NOT a `(= k v)` field pair (a surplus/too-few element
    // in a `#record`/`#map` that the compiler rejects AS CODE). `quote` reifies SYNTAX, not semantics: the
    // malformed literal still PARSES to a well-defined structure, so reify the entry GENERICALLY as data
    // (its own `Ast.*` node — e.g. a bare surplus `2` → `Ast.Int`) rather than bailing the whole quote.
    // The reflected `Ast.<X>Ctor` then carries a non-`FieldPair` child, which round-trips deterministically
    // through the codec like any other child (the ctor payload is a generic `(List Ast)`, not typed to
    // `FieldPair`). A WELL-FORMED collection is unaffected — every child hits the field-pair arm above.
    reify(ast, entry, under_qq)
}

/// The `(key, value)` of a record/map field entry: a native `Leaf::FieldPair` leaf `(= k v)`, a
/// name-headed `(= k v)` triple, or a map `(k v)` 2-element pair. `None` otherwise.
fn field_kv(ast: &Arenas, entry: StructId) -> Option<(StructId, StructId)> {
    if let Some(kv) = ast.field_pair_parts(entry) {
        return Some(kv);
    }
    if let Some(kv) = ast.field_pair(entry) {
        return Some(kv);
    }
    match ast.get(entry) {
        Struct::List(items) if items.len() == 2 => Some((items[0], items[1])),
        _ => None,
    }
}

/// Build a `(list <child…>)` value-constructor form — the reader-shaped list literal the `list` prelude
/// alias resolves to (`ListNew`). Shared by `wrap_ast_list` and the splice element-list assembly.
fn list_form(ast: &mut Arenas, children: Vec<StructId>) -> StructId {
    let list_head = push_atom(ast, Leaf::Name("list".into()));
    let mut form = Vec::with_capacity(children.len() + 1);
    form.push(list_head);
    form.extend(children);
    push_list(ast, form)
}

/// Build `((intrinsic "ast-splice-lift") operand)` — the compiler-internal lift `(List Int64) → (List
/// Ast)` applied to an active `,@` splice's LIVE operand (it stays live — evaluated code). At lowering a
/// constant operand list folds to a `(List Ast)` of `Ast.Int` nodes (`lower::lower_ast_splice_lift`).
fn ast_splice_lift(ast: &mut Arenas, operand: StructId) -> StructId {
    let intr = push_atom(ast, Leaf::Name("intrinsic".into()));
    let who = push_atom(ast, Leaf::Name("ast-splice-lift".into()));
    let prim = push_list(ast, vec![intr, who]);
    push_list(ast, vec![prim, operand])
}

/// Build `((intrinsic "ast-lift") operand)` — the compiler-internal lift `∀a. a → Ast` applied to a
/// RUNTIME active-unquote operand (a name / a computed expression) that stays LIVE (evaluated code). At
/// lowering (`lower::lower_ast_lift`) it resolves by the operand's INFERRED type: identity when already
/// `Ast`, else wrap in the matching `Ast.Int`/`Bool`/`Str` leaf. The runtime-operand companion of the
/// literal-operand `ast_ctor` dispatch (whose kind is known structurally at reify time).
fn ast_lift(ast: &mut Arenas, operand: StructId) -> StructId {
    let intr = push_atom(ast, Leaf::Name("intrinsic".into()));
    let who = push_atom(ast, Leaf::Name("ast-lift".into()));
    let prim = push_list(ast, vec![intr, who]);
    push_list(ast, vec![prim, operand])
}

/// Build `((. List concat) a b)` — the `List.concat` member-access application that joins two element
/// lists. A constant `a`/`b` folds to one merged `Core::ListNew` at lowering (`Prim::ListConcat`).
fn list_concat(ast: &mut Arenas, a: StructId, b: StructId) -> StructId {
    let dot = push_atom(ast, Leaf::Name(".".into()));
    let list_mod = push_atom(ast, Leaf::Name("List".into()));
    let concat = push_atom(ast, Leaf::Name("concat".into()));
    let proj = push_list(ast, vec![dot, list_mod, concat]);
    push_list(ast, vec![proj, a, b])
}

/// Build the constructor application `(Ast.<variant> payload)` — i.e. the list `[(. Ast <variant>),
/// payload]`, where the head is the member-access projection `(. Ast <variant>)` the reader produces
/// for the dotted name `Ast.<variant>`. So the emitted node is byte-for-byte the shape a hand-written
/// `(Ast.Int 42)` reads to, and resolves/types/lowers identically.
fn ast_ctor(ast: &mut Arenas, variant: &str, payload: StructId) -> StructId {
    let dot = push_atom(ast, Leaf::Name(".".into()));
    let ast_name = push_atom(ast, Leaf::Name("Ast".into()));
    let variant_name = push_atom(ast, Leaf::Name(variant.into()));
    let proj = push_list(ast, vec![dot, ast_name, variant_name]);
    push_list(ast, vec![proj, payload])
}

/// Ground an integer-literal payload to `BigInt` — wrap it in the type-annotation node `(: <lit> BigInt)`.
/// `Ast.Int`'s payload is `BigInt` (a quoted AST stores integers non-lossily; `numeric-model.md` — a
/// literal grounds to `BigInt` losslessly), but a BARE int literal defaults to a deferred `Int64` and
/// `BigInt` is MONOMORPHIC (`unify.rs` — no silent `Int64`→`BigInt` promotion), so a reified
/// `(Ast.Int 42)` would decline against the `BigInt` payload. Annotating the payload `(: 42 BigInt)`
/// grounds the literal to `BigInt` exactly as an explicit annotation does in source — lossless, and
/// in-spec (explicit context overrides the default width; not a promotion). The eval/splice boundary
/// STRIPS this wrapper (`eval_ast::reconstruct`) so a reconstructed literal grounds by ordinary
/// inference — BigInt is a STORAGE property of the AST value, not a property the source carries out.
fn ast_bigint_payload(ast: &mut Arenas, payload: StructId) -> StructId {
    let colon = push_atom(ast, Leaf::Name(":".into()));
    let bigint = push_atom(ast, Leaf::Name("BigInt".into()));
    push_list(ast, vec![colon, payload, bigint])
}
