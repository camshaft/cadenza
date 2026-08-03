# PR #1236 review comment — guide/src/content/arc.test.ts (v-guide-editor)

Mirrored from https://github.com/camshaft/cadenza/pull/1236 (PR: "cand: v-guide-editor — 2fe007e61").
Follow-up to my #1166 regex-backtracking filing — the regex is now bounded, but the comment lagged.

## Doc comment stale after the bounded-regex change (Copilot, arc.test.ts:157) — doc
> The doc comment still claims this uses the "same regex as links/pillarBridge tests", but this file
> now uses a bounded `[\s\S]{0,500}?` variant. Also, the justification hard-codes current max-gap
> measurements (e.g. "platform-safety is 316"), which will go stale as copy changes. Consider
> updating the comment to describe the bounded-regex intent without asserting it matches other tests
> or relying on exact current measurements.

Follow-through on #1166 (bounding the regex to avoid catastrophic backtracking): the code fix landed,
but the comment now (a) claims parity with the links/pillarBridge tests that this file no longer
matches, and (b) bakes in a specific gap measurement ("platform-safety is 316") that will drift as
copy changes. Reword to describe the bounded-regex intent (cap the slug->import gap) without the
cross-test-parity claim or the hard-coded number.
