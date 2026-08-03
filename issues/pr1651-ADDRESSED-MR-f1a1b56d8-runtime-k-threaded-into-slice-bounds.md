# PR #1651 review comments — rcdzc/src/tests.rs (v-rust-backend) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1651 (MERGED — adv-54b: keep Bytes.concat a runtime computation).

## 1. Test stays runtime-only by relying on a fold LIMITATION (non-ASCII String.concat declines to fold) (Copilot, tests.rs:2411) — test-fragility
> This test depends on an implementation limitation to stay runtime-only (`String.concat` declines
> constant folding when either operand is non-ASCII). If `String.concat` later learns to fold non-ASCII
> constants, [the test would silently stop exercising the runtime path].

The test's runtime-ness hinges on a fold gap, not an intrinsic runtime dependency — so a future
non-ASCII-folding improvement would silently convert it to a const case (losing the adv-54b runtime
coverage without failing). Make `s` genuinely runtime-dependent (e.g. thread it through a runtime `z` /
param) so the runtime path is guaranteed regardless of fold capability. LOW/test-durability, fix-forward.

## 2. Doc comment hard-codes `s = "ab"++"cdé"` as the runtime/opaque justification (Copilot, tests.rs:2399) — doc
> The doc comment hard-codes `s` as "ab"++"cdé" to justify why it is runtime/opaque-to-fold. If the test
> setup changes … this line becomes stale.

Tied to point 1 — if the setup is made runtime-dependent, update this rationale so it doesn't assert the
now-stale non-ASCII-fold justification. LOW/doc.
