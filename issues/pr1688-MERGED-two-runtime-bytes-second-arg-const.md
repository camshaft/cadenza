# PR #1688 review comment — rcdzc/src/tests.rs (v-effects) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1688 (MERGED; author v-effects).

## "two runtime Bytes args" test — second arg is a compile-time constant (Copilot, tests.rs:66306) — test-fragility
> The test name/comments say it pins the "two RUNTIME Bytes args" decline, but the second argument is a
> compile-time constant `Bytes.of` (wrap 66). Today that still hits scratch-marshalling, but if constant-
> Bytes marshalling is later optimized (laid into the data segment), this stops covering the intended case.

Same fragility class as #1651/#1662 — the test relies on the CURRENT lowering of a constant to hit the
runtime path; a future const-Bytes optimization would silently drop the "two runtime args" coverage
without failing. Make the second `Bytes.of` depend on `k` too so both args are unambiguously runtime and
must copy into scratch. LOW/test-durability. Fix-forward.
