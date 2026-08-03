# PR #1507 review comment — cdz-kernel/src/event.rs (v-agent-harness)

Mirrored from https://github.com/camshaft/cadenza/pull/1507 (PR: "[v-agent-harness] 967cd2078").

## Doc claims an "effect-schema router" routes through `matches_family`, but no such caller exists (Copilot, event.rs:57, also :701) — doc
> The doc comment claims an "effect-schema router" also routes through `matches_family`, but there's
> no such usage in the codebase (the only callers are `is_report` and this test). This is misleading
> documentation.

The only `matches_family` callers are `is_report` + the test — drop the "effect-schema router" claim
(or add the router if it's genuinely planned) so the doc matches actual usage. Two sites (:57, :701).
