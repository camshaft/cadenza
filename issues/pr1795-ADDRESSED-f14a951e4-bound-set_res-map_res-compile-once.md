# PR #1795 review comment — rcdzc/src/backend/rust/tests.rs (v-rust-backend) — OPEN

https://github.com/camshaft/cadenza/pull/1795 (render in-range Set-element/Map-value literals).

## Assertion calls `try_compile_rust(set)` twice → repeated compile + possible mismatched diagnostics (Copilot, tests.rs:7674) — test-precision
> This assertion calls `try_compile_rust(set)` twice, which repeats a full compile and can produce
> mismatched diagnostics if behavior changes between calls. Capture the result once and assert on it.
Bind the `try_compile_rust(set)` result to a local once and assert against that, instead of calling it
twice (wasteful + a theoretical mismatch if the compile isn't deterministic). LOW/test-precision.
Fix-forward.
