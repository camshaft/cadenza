# PR #1322 review comments — rcdzc/src/binding_params.rs + tests.rs (v-inference)

Mirrored from https://github.com/camshaft/cadenza/pull/1322 (PR: "cand: v-inference — eb172b343").
amazon-q claims VERIFIED against the diff (lines 44, 100).

## 1. `children.clone().iter()` clones the vec per recursive call (amazon-q, binding_params.rs:125 / diff:44) — efficiency
> Clone the entire `children` vector on every recursive call. Remove `.clone()` and iterate directly
> over the slice reference to avoid unnecessary allocations during AST traversal.

Verified: diff line 44 is `for &child in children.clone().iter()`. The clone is a real avoidable
alloc per node during traversal — but note you can't just borrow `ast.get(node)` immutably across a
recursive `apply(ast, child, …)` that takes `&mut ast`. The clone may be a borrow-checker workaround;
if so, collect the child ids into a small `Vec` ONCE (or copy the `StructId`s, which are `Copy`)
rather than cloning the whole children vec each level. Worth a look at whether a cheaper collect works.

## 2. Direct `ast.structure[fn_node.0 as usize]` indexing (amazon-q, binding_params.rs:186 / diff:100) — robustness (defensive)
> Unchecked array indexing could panic if `fn_node.0` exceeds `ast.structure` bounds. Add explicit
> bounds checking before array access.

Lower-confidence: `fn_node` is a `StructId` harvested from this same AST, so it's structurally in
bounds and won't panic in practice — this is defensive-only. If you want belt-and-suspenders, guard
with `if index < ast.structure.len() && let Struct::List(children) = …` (amazon-q's suggestion);
otherwise it's not a live bug. Your call.

## 3. CDZ0210 refutable-lambda-param test doesn't compile a refutable param (Copilot, tests.rs:59814) — test-coverage
> The test comment says an ill-formed/refutable lambda parameter pattern should still reject with
> CDZ0210, but the test never actually compiles such a refutable lambda param (it only asserts
> absence of CDZ0101 for `src`). This leaves the regression claim unverified and could mask a silent
> miscompile or a change in the reject code path.

The test's stated intent (reject a refutable lambda param with CDZ0210) isn't exercised — it only
checks no-CDZ0101 on a well-formed `src`. Add a case that actually compiles a refutable lambda param
and asserts the CDZ0210 reject, or the regression claim is unbacked.
