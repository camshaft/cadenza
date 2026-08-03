# PR #1085 review comments — guide/src/content/pillarBridge.test.ts (v-guide-editor)

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1085
(PR: "cand: v-guide-editor — pillarBridge test").

## 1. `files.get(...)!` non-null assertion crash-risk (amazon-q + Copilot agree, pillarBridge.test.ts:73/74) — test-robustness
> [amazon-q] Non-null assertion used without validation. If the slug doesn't exist in the files map,
> this will throw at runtime. Add validation before the non-null assertion or use optional chaining
> with a fallback.
> [Copilot] `files.get(...)!` can throw a generic runtime error if the registry mapping ever fails
> for the closer slug (even if `files.size` is still large). Use an explicit `assert.ok` like the
> first test so failures point to the missing mapping with a clear message.

Both reviewers flag the same spot: replace the `!` non-null assertion with an `assert.ok(closerFile,
"…")` so a missing mapping yields a clear failure message instead of a generic throw.

## 2. Redundant early-return hides a more specific failure (Copilot, pillarBridge.test.ts:45) — test-robustness
> The test returns early when `platformChapters` is empty, but the later guard test asserts
> `platformChapters.length >= 1`. This makes the early return redundant and can hide the more
> specific failure message from this test. Prefer asserting the platform pillar exists here instead
> of returning.

Both points make the pillarBridge tests fail loudly-and-specifically rather than silently pass or
throw generically.
