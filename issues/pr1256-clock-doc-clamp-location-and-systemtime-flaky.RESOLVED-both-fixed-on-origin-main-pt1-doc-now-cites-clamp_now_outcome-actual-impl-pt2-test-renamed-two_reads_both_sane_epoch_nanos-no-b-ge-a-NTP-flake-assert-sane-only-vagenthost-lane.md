# PR #1256 review comments — cdz-agent-host/src/clock.rs (v-agent-harness-host)

Mirrored from https://github.com/camshaft/cadenza/pull/1256 (PR: "cand: v-agent-harness-host — 1e70e31f6").
Both intersect the #1253 clamp_now monotonicity work.

## 1. Doc claims the KERNEL monotonic-clamps `Now`, but no such Now-clamp is visible in cdz-kernel (Copilot, clock.rs:15, also :51) — doc/correctness
> The module-level docs claim the *kernel* performs a monotonic clamp (`max(raw, last_now+1ns)`) and
> that the `Now` sequence is therefore strictly increasing, but there doesn't appear to be any
> `Now`-specific clamp logic in `cdz-kernel` (no references to `last_now`, `clamp`, or decoding `Now`
> payloads). This makes the docs misleading; either cite/link the actual implementation location or
> describe only the on-the-wire payload encoding here.

⚠ Worth reconciling with #1253: `clamp_now_outcome` (kernel.rs:963) IS the monotonic clamp, so this
may just be a stale/mislocated doc reference (point it at `clamp_now_outcome`) — OR the clamp genuinely
isn't wired for the `Now` path this doc describes, which would be a real gap. Confirm the `Now` clamp
is actually applied where this doc claims; if it is, cite `clamp_now_outcome`; if the doc here should
only cover wire encoding, trim the strictly-increasing claim to where it's enforced.

## 2. `SystemTime` isn't monotonic → `b >= a` assert can be CI-flaky (Copilot, clock.rs:134) — test-flakiness
> `SystemTime` is not guaranteed to be monotonic (NTP / operator adjustments can move it backwards),
> so asserting `b >= a` can make this test flaky in CI. If monotonicity is a requirement, it should
> be asserted where the clamping/recording logic actually lives; otherwise, change this unit test to
> only assert both readings are sane epoch nanos values.

The unit test asserts `b >= a` on two raw `SystemTime` reads, but wall-clock can jump backward (NTP),
so this is a latent CI flake. Assert monotonicity at the clamp layer (`clamp_now_outcome`, per #1253)
instead; here just assert both reads are sane epoch-nanos.
