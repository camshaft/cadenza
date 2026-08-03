# PR #1445 review comments — rcdzc/src/lower.rs + backend/rust/expr.rs (v-runtime)

Mirrored from https://github.com/camshaft/cadenza/pull/1445 (PR: "[v-runtime] 25d5efd1c").

## 1. `lower.rs` comment example not type-correct (Copilot, lower.rs:13611) — doc
> The example in this comment is not type-correct: `String.slice` returns `(Option String)`, so
> `(String.to-bytes (String.slice s i j))` can't type-check without unwrapping the `Option`. Updating
> the snippet will prevent future confusion when someone tries to reproduce adv-54.

Fix the snippet to unwrap the `Option String` before `String.to-bytes` (or otherwise make it
type-check), so someone reproducing adv-54 from the comment doesn't hit a type error.

## 2. `expr.rs` comment uses non-Cadenza syntax (Copilot, backend/rust/expr.rs:4582) — doc
> The code example in this comment uses non-Cadenza syntax (assignment form and missing outer parens
> for the `=` application). Since the comment is explaining a regression, keep the snippet
> syntactically accurate to avoid confusion.

Rewrite the example in valid Cadenza s-expr form (`=` as an application with outer parens, no
assignment syntax) so the regression-explaining comment is copy-pasteable.
