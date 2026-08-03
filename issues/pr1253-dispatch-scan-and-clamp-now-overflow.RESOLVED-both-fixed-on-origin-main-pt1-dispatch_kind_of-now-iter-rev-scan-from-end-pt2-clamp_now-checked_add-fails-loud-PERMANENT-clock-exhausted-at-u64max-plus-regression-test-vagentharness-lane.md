# PR #1253 review comments — cdz-kernel/src/kernel.rs (v-agent-harness)

Mirrored from https://github.com/camshaft/cadenza/pull/1253 (PR: "cand: v-agent-harness — 65365ac29").

## 1. `dispatch_kind_of` forward-scans the log per EffectResult during replay (Copilot, kernel.rs:694) — efficiency
> `dispatch_kind_of` linearly scans the log from the beginning. During replay this is called for
> every `EffectResult`, so this repeated forward scan can become a noticeable hot path as logs grow.
> Scanning from the end is equivalent (there is at most one `Dispatched` per id) and will usually
> find the match much sooner.

Replay cost: forward-scanning the whole log for every `EffectResult` is O(log²) over a replay. Since
there's at most one `Dispatched` per id, scan from the END — same result, usually finds it near the
matching result event.

## 2. `clamp_now_outcome` saturating_add(1) silently breaks monotonicity at u64::MAX (Copilot, kernel.rs:963) — correctness
> `clamp_now_outcome` uses `saturating_add(1)` to compute the monotonic floor. If `last_now ==
> u64::MAX`, the saturating add returns `u64::MAX`, so the clamp can no longer guarantee a
> strictly-increasing value even though the doc comment states it does. It's better to detect
> overflow explicitly and surface it as an error so the monotonicity invariant isn't silently
> violated.

At `last_now == u64::MAX`, `saturating_add(1)` == `u64::MAX`, so the "strictly increasing" guarantee
the doc states is silently violated (returns a non-increasing value). Detect the overflow and surface
it as an error rather than letting the invariant break silently. (Edge case, but it's a documented
invariant — better a loud error than a silent monotonicity break in a durable-clock path.)
