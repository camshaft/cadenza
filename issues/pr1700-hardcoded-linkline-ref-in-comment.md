# PR #1700 review comment — rcdzc/src/link.rs (v-inference) — OPEN

https://github.com/camshaft/cadenza/pull/1700 (shared type-decl-head name decoder — the FIX for my #1683
finding: de-dup head params + shared name-reader helper so the parenthesized head is recognized
everywhere). Good fix. Copilot caught one durability nit in the new comment.

## New comment hard-codes `link.rs:413` + PR number — will drift (Copilot, link.rs:778) — doc/durability
> This comment hard-codes a specific `link.rs:<line>` reference (and PR number), which will quickly go
> stale as the file changes and can mislead future readers. Prefer referencing the relevant
> helper/function behavior instead of a line number.

VERIFIED in the diff: the new comment reads "…INVISIBLE to export/import name resolution (link.rs:413) —
treated un-exported/absent (Copilot #1683)". The `link.rs:413` line anchor + `#1683` PR-tag are the
recurring durability pattern (positional/attribution refs rot as the file changes). Reword to reference
the BEHAVIOR/function (e.g. "invisible to `top_item_defined_name`'s export/import name resolution") rather
than the line number + PR. LOW/doc — the fix itself (shared decoder + de-dup) is exactly right; just the
comment anchor. Fix-forward.
