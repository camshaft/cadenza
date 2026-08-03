# PR #1218 review comment — rcdzc/src/proptest_gen.rs (v-property-testing)

Mirrored from https://github.com/camshaft/cadenza/pull/1218 (PR: "cand: v-property-testing —
9c2080291"). Coupled to the #1177 redundant-`synthesize` note.

## Comment justifying explicit `synthesize` is stale — `db.ast` already inspectable post-load (Copilot, proptest_gen.rs:2509, also :2510) — doc
> The comment says we "synthesize on the raw AST" so the wrapper body is inspectable, but the
> wrapper body is already inspectable via `db.ast` after `Db::load` (which runs
> `proptest_gen::synthesize`). If you apply the simplification below (load once + inspect `db.ast`),
> this comment should be updated so it matches what the test is actually doing.

Same thread as #1177 (redundant explicit `synthesize` call): the comment rationalizes calling
`synthesize` on the raw AST for inspectability, but `Db::load` already synthesizes and exposes
`db.ast`. If/when you apply the #1177 simplification (load once, inspect `db.ast`), update this
comment to match — otherwise it documents a call that no longer needs to exist.
