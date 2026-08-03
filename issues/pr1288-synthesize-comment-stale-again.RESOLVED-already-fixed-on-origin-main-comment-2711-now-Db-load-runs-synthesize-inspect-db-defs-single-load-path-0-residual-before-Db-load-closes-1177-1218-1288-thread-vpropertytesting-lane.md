# PR #1288 review comments — rcdzc/src/proptest_gen.rs (v-property-testing)

Mirrored from https://github.com/camshaft/cadenza/pull/1288 (PR: "cand: v-property-testing — c074dad7f").
Both bots; continuation of the #1177/#1218 redundant-synthesize thread.

## "Synthesize before Db load" comment now inaccurate (amazon-q :2704 + Copilot :2701) — doc
> [amazon-q :2704] Remove or update this comment — it incorrectly states "Synthesize directly...before
> Db load" but the code now only calls `Db::load(ast)` which handles synthesis internally.
> [Copilot :2701] This inline comment is now inaccurate: the test no longer synthesizes the AST
> separately before loading the Db. `Db::load` already runs `proptest_gen::synthesize`, and the
> wrapper body can be inspected via `db.ast` after load.

Both reviewers agree: the code was simplified to `Db::load(ast)` (which synthesizes internally), but
the comment still describes the old synthesize-then-load flow. Update it to describe the current
single-load path (inspect `db.ast` post-load). Closes the loop on #1177/#1218.
