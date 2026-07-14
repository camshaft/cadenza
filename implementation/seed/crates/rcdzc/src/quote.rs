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
//! ## Scope of this increment
//!
//! The built-in `Ast` sum currently has three variants — `Int`/`Name`/`List` — so only a form built
//! from integers, names, and lists is reifiable. A quote whose body mentions any OTHER leaf (a string,
//! float, bool, char, symbol, bytes literal — no `Ast` variant carries it yet) is LEFT UNTOUCHED here:
//! it flows to `resolve::resolve_quote`, which DECLINES (a Todo, never a miscompile). Likewise an
//! arity-≠1 `(quote …)` is left for `resolve_quote` to reject CDZ0201. This pass only ever rewrites a
//! quote it can reify COMPLETELY — partial reification is never emitted.
//!
//! ## Ordering / in-place rewrite
//!
//! Modelled on [`crate::effects::desugar_handles`]: a scan collects the rewrites, then they are applied.
//! The reader builds children BEFORE parents, so a nested (inner) quote always has a SMALLER `StructId`
//! than the quote enclosing it. Processing quotes in DESCENDING id order therefore reifies an OUTER
//! quote — reading its body's still-ORIGINAL structure (its descendants have smaller ids, not yet
//! rewritten) — before the pass reaches the inner quote's id. By then the inner quote node is ORPHANED
//! (the outer's reified tree is all fresh nodes that only READ the inner's leaf values, never reference
//! its ids), so rewriting it is harmless. Reification only ever READS existing nodes and APPENDS fresh
//! ones, so no live node is ever mutated out from under a pending rewrite.

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
        // A well-formed one-operand `(quote FORM)`. Any other arity is left for `resolve_quote` to
        // reject CDZ0201; a non-quote node is skipped.
        let tail = ast.as_form(id, "quote").map(<[StructId]>::to_vec);
        let Some([form]) = tail.as_deref() else {
            continue;
        };
        let form = *form;
        // Reify the body. `None` = either it mentions a leaf with no `Ast` variant yet, OR it contains a
        // STRAY unquote (a `,x`/`,@x` not under a quasiquote — a syntax error, `metaprogramming.md`
        // §Quasiquote Constructs AST With Selective Evaluation). In both cases leave the quote for
        // resolve: a missing-variant body DECLINES (a Todo), a stray unquote gets CDZ0003 (via
        // `resolve::resolve_unquote`). Never emit a partial reification.
        if let Some(reified) = reify(ast, form, false) {
            plans.push(QuotePlan { quote: id, reified });
        }
    }
    for plan in plans {
        // Overwrite the quote node with a COPY of the reified root's structure, so the quote's own
        // `StructId` (and its span) is preserved as the result value's node.
        let root = ast.get(plan.reified).clone();
        ast.structure[plan.quote.0 as usize] = root;
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
            let list_head = push_atom(ast, Leaf::Name("list".to_string()));
            let mut list_form = Vec::with_capacity(reified_children.len() + 1);
            list_form.push(list_head);
            list_form.extend(reified_children);
            let list_val = push_list(ast, list_form);
            Some(ast_ctor(ast, "List", list_val))
        }
    }
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
