# PR #1284 review comments — cdz/src/lsp.rs (v-lsp) — the REAL inlines (verified against diff)

Mirrored from https://github.com/camshaft/cadenza/pull/1284 (PR: "cand: v-lsp — 31abdbdc2", the
multibyte-hover test strengthen). NOTE: an earlier amazon-q "get_range_from_diag underflow" review on
this PR was a HALLUCINATION (that fn isn't in the diff) — dropped. These two Copilot inlines are the
real ones and check out against the actual diff.

## 1. Unnecessary `.clone()` of `on_name` for the well-formedness assert (Copilot, lsp.rs:4062) — simplification
> This clones `on_name` just to call `assert_hover_is_well_formed`. You can avoid the clone (and
> better exercise the helper on the original `Option<Hover>`) by keeping the `Option` until after the
> well-formedness assertion.

## 2. Column description off-by-one in the test comment (Copilot, lsp.rs:4071) — doc
> The column description here is slightly off: in `"def café = 42"`, UTF-16 column 8 is on the space
> immediately after `é` (before the `=`), not on the `=` itself. Tightening this avoids confusion
> when maintaining the test.

Both minor: drop the clone (keep the `Option` through the assert), and fix the comment to say col 8
lands on the space after `é`, not the `=`.
