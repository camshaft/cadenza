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
//! An ACTIVE unquote keeps its operand `e` LIVE (reused, not reified) and wraps it `(Ast.Int e)`, so `e`
//! resolves/types/lowers as ordinary code: an unbound name in it is the ordinary CDZ0101 (NOT swallowed
//! into inert AST), and its Int64 value lifts to an `(Ast.Int …)` node structurally identical to a const
//! fold's — so `` `(f ,x) `` (x=1) equals `(quote (f 1))`. ⚠ THE LIFT IS Int-ONLY this increment: the
//! active operand is wrapped `(Ast.Int e)` unconditionally (every corpus active-unquote is Int-valued),
//! so a non-Int active unquote gets `Ast.Int`'s payload type-error (a decline-equivalent, never a
//! miscompile); a type-directed lift (Ast-identity, other payload types) is a later increment.
//!
//! An ACTIVE `(unquote-splicing e)` (splice list elements into the parent) BAILS (`None`) — the whole
//! quasiquote is left for `resolve` (so the splice-non-list CDZ0201 check + the decline still fire); a
//! real list-flattening splice is a later increment.
//!
//! ## Scope of this increment
//!
//! The built-in `Ast` sum currently has three variants — `Int`/`Name`/`List` — so only a form built
//! from integers, names, and lists is reifiable. A quote whose body mentions any OTHER leaf (a string,
//! float, bool, char, symbol, bytes literal — no `Ast` variant carries it yet) is LEFT UNTOUCHED here:
//! it flows to `resolve::resolve_quote`, which DECLINES (a Todo, never a miscompile). Likewise an
//! arity-≠1 `(quote …)` is left for `resolve_quote` to reject CDZ0201. This pass only ever rewrites a
//! quote/quasiquote it can reify COMPLETELY — partial reification is never emitted.
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
/// `(. Ast …)` projections and `(list …)` forms resolve like hand-written source.
pub fn reify_quotes(ast: &mut Arenas) {
    // Snapshot the pre-existing node count: only ORIGINAL nodes can be a source quote, and reification
    // APPENDS (ids >= this bound), so the scan must not consider its own output. Descending id order so
    // an outer quote is reified from its body's original structure before its inner quotes are reached
    // (see the ordering note in the module docs).
    let original_len = ast.structure.len() as u32;
    let mut plans: Vec<QuotePlan> = Vec::new();
    for i in (0..original_len).rev() {
        let id = StructId(i);
        // A well-formed one-operand `(quote FORM)` reifies INERTLY (everything structural — a quote body
        // never evaluates); a `(quasiquote TEMPLATE)` reifies ACTIVELY (depth 1 — an `,e` at depth 1
        // evaluates). Any other arity is left for `resolve_quote`/`resolve_quasiquote` to reject CDZ0201;
        // a non-quote node is skipped. An INNER quote/quasiquote the enclosing reification already
        // consumed structurally is orphaned by the time the descending scan reaches it, so re-planning it
        // is harmless dead work (module docs §Ordering).
        let reified = if let Some([form]) = ast
            .as_form(id, "quote")
            .map(<[StructId]>::to_vec)
            .as_deref()
        {
            // Reify INERTLY. `None` = a leaf with no `Ast` variant yet, OR a STRAY unquote (`,x`/`,@x` not
            // under a quasiquote — a syntax error). Leave the quote for resolve: a missing-variant body
            // DECLINES (Todo), a stray unquote gets CDZ0003 (`resolve::resolve_unquote`). Never partial.
            reify(ast, *form, false)
        } else if let Some([tmpl]) = ast
            .as_form(id, "quasiquote")
            .map(<[StructId]>::to_vec)
            .as_deref()
        {
            // Reify ACTIVELY at depth 1. `None` = an un-reifiable leaf, an ACTIVE splice (`,@` — deferred),
            // or an arity fault — leave the quasiquote for `resolve_quasiquote` (a decline / the CDZ0201
            // splice-non-list check). Never partial.
            reify_active(ast, *tmpl, 1)
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
}

/// Build the `Ast` value that denotes the syntax at `node`, returning the root of the fresh
/// construction tree — or `None` if `node` (or any descendant) is un-reifiable. Purely structural:
/// recurses by SHAPE. `under_qq` is whether `node` sits under a `(quasiquote …)` template WITHIN the
/// quote body; it starts false at the quote body and turns true at the first `quasiquote` head.
///
/// Two reasons a reification bails (`None`):
///  - a leaf the `Ast` sum cannot yet carry (a Str/Float/Bool/Char/Sym/Bytes literal — only `Int`/`Name`/`List`);
///  - a STRAY `unquote`/`unquote-splicing` (`under_qq` still false): an escape outside any quasiquote is
///    a syntax error (`metaprogramming.md` §Quasiquote Constructs AST With Selective Evaluation), and a
///    plain `(quote …)` body is inert data — NOT a template — so `(quote (g ,x))` must reject CDZ0003,
///    not be reified. Bailing leaves the quote un-rewritten for `resolve::resolve_unquote` to code. An
///    escape UNDER a quasiquote (`under_qq` true — `(quote `(+ ,x))`) is ordinary inert structure and
///    reifies as an `Ast.List` mentioning the name (never evaluated), the corpus's inert-nesting case.
fn reify(ast: &mut Arenas, node: StructId, under_qq: bool) -> Option<StructId> {
    match ast.get(node) {
        Struct::Atom(l) => match ast.leaf(*l).clone() {
            // An integer literal -> `(Ast.Int N)`. The payload REUSES the literal's exact value/radix by
            // cloning the leaf, so the reified constant reads back identically.
            leaf @ Leaf::Int { .. } => {
                let payload = push_atom(ast, leaf);
                Some(ast_ctor(ast, "Int", payload))
            }
            // A bare name -> `(Ast.Name "foo")`. The name TEXT becomes a String payload (`Ast.Name`
            // carries the identifier as a String — `type-system.md`), so the identifier is captured as
            // data, not resolved as a reference.
            Leaf::Name(name) => {
                let payload = push_atom(ast, Leaf::Str(name));
                Some(ast_ctor(ast, "Name", payload))
            }
            // Any other leaf kind (Str/Float/Bool/Char/Sym/Bytes/…) has no `Ast` variant yet: not
            // reifiable — bail so the whole quote declines rather than miscompiling.
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
///  - `(unquote e)` at depth 1 → ACTIVE: reuse `e` LIVE, wrap `(Ast.Int e)` — `e` is evaluated as
///    ordinary code (unbound name → CDZ0101; its Int64 value lifts to an `Ast.Int` node). Int-only lift
///    this increment (a non-Int active unquote gets `Ast.Int`'s payload type-error).
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
            // A NON-INTEGER LITERAL operand (`,2.0`, `,"s"`, `,true`) cannot lift: the only value-carrying
            // `Ast` variant this increment builds is `Ast.Int` (Int64 payload). Wrapping such a literal
            // `(Ast.Int 2.0)` would leak the INTERNAL reification mechanism as a misleading "variant
            // constructor's payload has declared type Int64, but Float64 was applied" — worse, its
            // coercion fix would silently REWRITE the author's `2.0`→`2`, corrupting their value. BAIL
            // instead (as `reify` does for a bare non-Int leaf), so the quasiquote declines honestly with
            // "quasiquote produces an AST value (not yet built)" — the type-directed lift of a non-Int
            // active operand is a later increment (module docs §Quasiquote). A NON-literal operand (a
            // NAME `,n` or a call `,(f x)`) stays LIVE and wraps in `Ast.Int` as before — its type is
            // unknown at reify time (pre-typecheck), and an Int-valued one is the corpus case.
            //
            // 🔑 A `Leaf::Name` is NOT a literal — it is a runtime REFERENCE (a let-bound var `,n` or a
            // param), which the comment above says stays live. Only a non-int VALUE literal
            // (Float/String/Bool/Char) bails; a bare int literal `,42` lifts. So exclude `Leaf::Name` from
            // the bail (else `(let ((n 42)) `(op-const ,n))` and a `,param` regress to a spurious decline).
            if let Struct::Atom(l) = ast.get(items[1])
                && !matches!(ast.leaf(*l), Leaf::Int { .. } | Leaf::Name(_))
            {
                return None;
            }
            // Reuse the operand node LIVE (it is evaluated code, not reified) and wrap it `(Ast.Int e)`:
            // the Int64 value lifts to an `Ast.Int` node identical to a const fold's.
            Some(ast_ctor(ast, "Int", items[1]))
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
        // Any other compound: structural `(Ast.List (list <child…>))`, recursing at the SAME depth so an
        // active unquote nested anywhere inside still fires.
        _ => {
            let mut reified_children = Vec::with_capacity(items.len());
            for child in items {
                reified_children.push(reify_active(ast, child, depth)?);
            }
            Some(wrap_ast_list(ast, reified_children))
        }
    }
}

/// The inert reification of an escape/nesting head node `(HEAD inner)` → `(Ast.List (list (Ast.Name
/// "HEAD") <inner>))` where `<inner>` is the ALREADY-reified operand. So a quoted-but-not-active
/// `,`/`,@`/`` ` `` renders as the two-element list the reader produced (head name + operand), matching
/// the corpus nested-quasiquote value form.
fn reify_escape_list(ast: &mut Arenas, head: &str, inner: StructId) -> StructId {
    let head_payload = push_atom(ast, Leaf::Str(head.to_string()));
    let head_name = ast_ctor(ast, "Name", head_payload);
    wrap_ast_list(ast, vec![head_name, inner])
}

/// Wrap already-reified child `Ast` nodes in `(Ast.List (list <child…>))` — the shared tail of every
/// compound reification (plain-quote list, active-quasiquote list, escape-head list).
fn wrap_ast_list(ast: &mut Arenas, children: Vec<StructId>) -> StructId {
    let list_head = push_atom(ast, Leaf::Name("list".to_string()));
    let mut list_form = Vec::with_capacity(children.len() + 1);
    list_form.push(list_head);
    list_form.extend(children);
    let list_val = push_list(ast, list_form);
    ast_ctor(ast, "List", list_val)
}

/// Build the constructor application `(Ast.<variant> payload)` — i.e. the list `[(. Ast <variant>),
/// payload]`, where the head is the member-access projection `(. Ast <variant>)` the reader produces
/// for the dotted name `Ast.<variant>`. So the emitted node is byte-for-byte the shape a hand-written
/// `(Ast.Int 42)` reads to, and resolves/types/lowers identically.
fn ast_ctor(ast: &mut Arenas, variant: &str, payload: StructId) -> StructId {
    let dot = push_atom(ast, Leaf::Name(".".to_string()));
    let ast_name = push_atom(ast, Leaf::Name("Ast".to_string()));
    let variant_name = push_atom(ast, Leaf::Name(variant.to_string()));
    let proj = push_list(ast, vec![dot, ast_name, variant_name]);
    push_list(ast, vec![proj, payload])
}
