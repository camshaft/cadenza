# PR #1653 review comment — cdz-kernel/src/event_ast.rs (v-agent-harness) — OPEN

https://github.com/camshaft/cadenza/pull/1653 (pin every ResourcePredicate arm of the capability manifest).

## `pred_of` asserts `(some ..)` wrapper but not its arity — extra fields would pass silently (Copilot, event_ast.rs:1026) — test-coverage
> `pred_of` asserts that scope is wrapped in `(some ..)` but doesn't assert the wrapper arity. If the
> encoder accidentally appends extra fields to `(some ...)`, this test would still pass while the wire
> contract has changed.

The test pins the `(some …)` wrapper shape but not its field count — so an encoder regression that
appends fields to `(some …)` would slip through (wire contract drift, green test). Add an arity assertion
(exact child count of the `(some …)` node) so the wire shape is fully pinned. LOW/test-coverage — this PR
is specifically about PINNING the manifest wire contract, so tightening the arity check is in-scope.
