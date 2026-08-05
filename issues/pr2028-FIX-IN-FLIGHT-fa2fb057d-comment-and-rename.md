# PR #2028 review — cdz-agent-host/src/factory.rs (v-agent-harness-host) — OPEN — test-comment accuracy [VERIFIED, LOW]

https://github.com/camshaft/cadenza/pull/2028 (wire MeteredExecutor into LiveExecutorSet — the impl side of
my #2019 doc-vs-impl finding). Copilot (id 3712593431) flags a misleading test comment.

## test comment says "one Ok each" but the second scripted executor returns `TimedOut` (Copilot, factory.rs:619) — test-comment accuracy [VERIFIED]
> The test comment says "one Ok each" but the second scripted executor returns `TimedOut`, not `Ok`, so
> the comment is misleading about what the test is asserting.

VERIFIED in the #2028 diff: the test comment reads "Two families here, one Ok each → the shared snapshot
sees both", but the second scripted executor's `outcomes` is `vec![EffectOutcome::TimedOut]` (not `Ok`).
So the test actually exercises an `Ok` + a `TimedOut` both aggregating into the shared host-wide
`Arc<EffectMetrics>` snapshot — which is arguably a BETTER test (it proves the metered wrap tallies a
non-Ok outcome too), but the comment misdescribes it as "one Ok each". The variable name `ok_b` is
likewise a slight misnomer for a TimedOut-returning executor. LOW/test-comment accuracy. Fix: reword the
comment to "one Ok + one TimedOut → the shared snapshot sees both outcomes" (and optionally rename `ok_b`
→ `timedout_b` for clarity). No behavior change — the test asserts the right thing; only the comment lies.
v-agent-harness-host owns cdz-agent-host/src. PR OPEN → foldable.
