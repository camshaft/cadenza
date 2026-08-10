//! TAGGED-TEMPLATE EXPANSION — turn a `(tagged-template <tag> (chunks c…) (holes h…))` node (the reader's
//! canonical form for `tag"…{expr}…"`) into the ordinary function application `(<tag> (list c…) (list h…))`
//! (`metaprogramming.md` §A Tagged Template Is A Binding-Dispatched Compile-Time Macro Over Literal Chunks
//! And Holes). The `tag` is dispatched BY BINDING — it resolves in the ordinary name environment to a
//! compile-time function `List String -> List Ast -> Ast` — and applying it to the chunk list and the hole
//! list is an ORDINARY call the existing one-tier compile-time evaluator β-reduces (`eval::apply_lambda`),
//! splicing the returned `Ast` in place and folding it through the same path that reduces generics and
//! `(eval (quote …))`. So this pass adds NO evaluator: it only rewrites the node to the application whose
//! meaning is exactly the tag function's result, expanded to a fixpoint (the rewritten call, and anything
//! its result contains, is reduced by the ordinary fold) and type-checked as hand-written code.
//!
//! ```text
//! (tagged-template jsx (chunks "a" "b") (holes x))  ->  (jsx (list "a" "b") (list x))
//! (tagged-template id  (chunks "hi")    (holes))    ->  (id  (list "hi")    (list))
//! ```
//!
//! The invariant `chunks.len() == holes.len() + 1` is the reader's; this pass does not re-check it. A node
//! that is not the expected 4-child `(tagged-template <tag-name> (chunks …) (holes …))` shape is left
//! UNTOUCHED for `resolve` to reject (it reaches resolve as an ordinary form → an unbound-name error on the
//! reserved head). A tag that does not resolve, or resolves to a non-function / wrong-arity / wrong-type
//! binding, is the ordinary application error at the (rewritten) call site — dispatch is by binding.
//!
//! ## Ordering / in-place rewrite
//!
//! Modelled on [`crate::quote::reify_quotes`] and [`crate::eval_ast::desugar_eval`]: a scan collects the
//! rewrites, then each `(tagged-template …)` node's structure entry is overwritten with the application.
//! Runs during `Db::load` (after quote reification / eval reconstruction, before the parent index) so the
//! emitted `(<tag> (list …) (list …))` resolves like hand-written source and the tag's own binding is found
//! by the ordinary scope walk. The chunk/hole child nodes are REUSED live (they are ordinary values /
//! expressions — a chunk is a `Str` leaf, a hole an ordinary expression that must resolve against the
//! template's enclosing scope), so they are spliced into the `(list …)` forms unchanged.

use crate::ast::{Arenas, Leaf, Struct, StructId};
use crate::prelude::{push_atom, push_list};

/// A pending rewrite: overwrite the `(tagged-template …)` node `tt` with the application `replacement`.
struct Plan {
    tt: StructId,
    replacement: StructId,
}

/// Rewrite every well-formed `(tagged-template <tag> (chunks c…) (holes h…))` into `(<tag> (list c…)
/// (list h…))` — the binding-dispatched application the one-tier evaluator reduces (see the module docs).
//= spec/capabilities/metaprogramming.md#a-tagged-template-is-a-binding-dispatched-compile-time-macro-over-literal-chunks-and-holes
//# The tag of a tagged template MUST be dispatched by binding, not by spelling, so that a program adds an embedded domain-specific syntax by defining or importing a function rather than by extending the reader.
//= spec/capabilities/metaprogramming.md#a-tagged-template-is-a-binding-dispatched-compile-time-macro-over-literal-chunks-and-holes
//# The compiler MUST resolve the tag name to a binding and require it to be a compile-time function from a list of the chunk strings and a list of the hole expressions to an abstract syntax tree.
//= spec/capabilities/metaprogramming.md#a-tagged-template-is-a-binding-dispatched-compile-time-macro-over-literal-chunks-and-holes
//# The compiler MUST evaluate that tag function on the one-tier compile-time evaluation mechanism, applied to the chunks and holes.
//= spec/capabilities/metaprogramming.md#a-tagged-template-is-a-binding-dispatched-compile-time-macro-over-literal-chunks-and-holes
//# The compiler MUST splice the tag function's resulting abstract syntax tree in the tagged template's position, expanding to a fixpoint before type checking, so that a tagged template is meaning-equivalent to the hand-written program its tag function produces and is type-checked as ordinary code.
pub fn expand(ast: &mut Arenas) {
    // Only ORIGINAL nodes can be a source `(tagged-template …)`; the rewrite APPENDS, so bound the scan.
    let original_len = ast.structure.len() as u32;
    // FAST BAIL for a program with no `(tagged-template …)` (the overwhelming common case). This pass
    // runs at EVERY load, scanning every node via `rewrite_of` (an `as_name(items[0]) == "tagged-
    // template"` probe); the reader only ever emits a `tagged-template`-headed list for the ML surface
    // `tag"…{expr}…"`, so its head is a `Leaf::Name("tagged-template")` in the leaf pool. If no such
    // name leaf exists, no tagged-template form exists anywhere and the scan is dead. A single O(leaves)
    // prescan is the cheap over-approximation (spurious fall-through only for a program that mentions the
    // identifier — which the reader never produces, so effectively never). Sibling of the
    // `quote::reify_quotes` / `desugar_eval` fast-bails.
    if !ast
        .leaves
        .iter()
        .any(|l| matches!(l, Leaf::Name(n) if n.as_ref() == "tagged-template"))
    {
        return;
    }
    #[cfg(test)]
    crate::db::TAGGED_TEMPLATE_SCAN_NODES.with(|c| c.set(c.get() + original_len as u64));
    let mut plans: Vec<Plan> = Vec::new();
    for i in 0..original_len {
        let id = StructId(i);
        if let Some(replacement) = rewrite_of(ast, id) {
            plans.push(Plan {
                tt: id,
                replacement,
            });
        }
    }
    for Plan { tt, replacement } in plans {
        // Overwrite the `(tagged-template …)` node with a COPY of the application's structure, so the
        // node's own `StructId` (and span) is preserved as the call's node. Then blank the now-duplicate
        // appended root (it lists the same children as the copy; leaving it intact would out-rank the copy
        // as the shared children's parent — the orphan hazard `reify_quotes`/`desugar_eval` also guard).
        let entry = ast.get(replacement).clone();
        ast.structure[tt.0 as usize] = entry;
        ast.structure[replacement.0 as usize] = Struct::List(Vec::new());
    }
}

/// If `node` is a well-formed `(tagged-template <tag> (chunks c…) (holes h…))`, build and return the
/// application `(<tag> (list c…) (list h…))` (a fresh appended node); else `None` (left for resolve).
fn rewrite_of(ast: &mut Arenas, node: StructId) -> Option<StructId> {
    // Clone the child ids so the immutable borrow ends before we append.
    let items = match ast.get(node) {
        Struct::List(items) => items.clone(),
        _ => return None,
    };
    // Shape: [tagged-template, <tag-name>, (chunks …), (holes …)] — exactly 4 children.
    if items.len() != 4 || ast.as_name(items[0]) != Some("tagged-template") {
        return None;
    }
    let tag = items[1];
    // The tag must be a NAME (the binding to resolve); a non-name head is malformed → leave for resolve.
    let tag_name = ast.as_name(tag).map(str::to_string)?;
    // `(chunks c…)` / `(holes h…)` — take their element tails. A wrong shape leaves the node untouched.
    let chunks = ast.as_form(items[2], "chunks")?.to_vec();
    let holes = ast.as_form(items[3], "holes")?.to_vec();

    // Build `(list c…)` and `(list h…)` reusing the chunk/hole nodes live (they are ordinary values /
    // expressions that must resolve against the template's enclosing scope).
    let chunk_list = list_form(ast, chunks);
    let hole_list = list_form(ast, holes);
    // The application head is a FRESH name occurrence of the tag (a new binder-free reference resolving by
    // the ordinary scope walk), so the original `tag` node is untouched.
    let tag_ref = push_atom(ast, Leaf::Name(tag_name.into()));
    Some(push_list(ast, vec![tag_ref, chunk_list, hole_list]))
}

/// Build a `(list e…)` value-constructor form — the reader-shaped list literal the `list` prelude alias
/// resolves to (`ListNew`). Mirrors `crate::quote::list_form`.
fn list_form(ast: &mut Arenas, children: Vec<StructId>) -> StructId {
    let list_head = push_atom(ast, Leaf::Name("list".into()));
    let mut form = Vec::with_capacity(children.len() + 1);
    form.push(list_head);
    form.extend(children);
    push_list(ast, form)
}
