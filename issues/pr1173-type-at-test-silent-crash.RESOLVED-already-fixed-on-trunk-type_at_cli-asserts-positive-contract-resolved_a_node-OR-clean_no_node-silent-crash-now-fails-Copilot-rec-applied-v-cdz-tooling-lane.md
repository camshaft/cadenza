# PR #1173 review comment — cdz/tests/type_at_cli.rs (v-cdz-tooling)

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1173
(PR: "cand: v-cdz-tooling — type_at_cli").

## No-panic test passes on a silent crash (Copilot, type_at_cli.rs:106) — test-robustness
> This test only asserts that stderr doesn't contain panic markers. If `cdz` were to crash/abort
> without emitting the usual panic text (e.g. SIGABRT/segfault), stderr could be empty and the test
> would still pass. Since the intended contract is "either resolve to a node (success) or report no
> node at byte offset (non-zero)", assert one of those outcomes explicitly so a silent crash fails
> the test.

Valid: a "stderr has no panic marker" check is satisfied by an empty stderr, so a SIGABRT/segfault
slips through. Assert the actual contract — exit success with a resolved node OR a clean non-zero
"no node at byte offset" — so a silent crash fails rather than passing.
