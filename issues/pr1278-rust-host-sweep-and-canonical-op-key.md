# PR #1278 review comments — xtask/src/main.rs (v-rust-backend) — Rust host-call sweep

Mirrored from https://github.com/camshaft/cadenza/pull/1278 (PR: "cand: v-rust-backend — b3fad7331").

## 1. `sweep_one_case` still unconditionally declines host cases for Rust after H1 shims landed (Copilot, main.rs:2907) — correctness/coverage
> `sweep_one_case` still treats Rust as having no host-boundary path and unconditionally declines
> host cases. With H1 host-call shims now implemented for the sync Rust backend, this sweep should
> include host-delegating cases for `GateTarget::Rust` (only `RustAsync` should skip them).

If H1 host-call shims are live for the sync Rust backend, the sweep is now skipping cases it could
actually run — include host-delegating cases for `GateTarget::Rust` and keep the skip only for
`RustAsync`. Confirm H1 sync-Rust host support is really landed before flipping.

## 2. `build_rust_host_shims` prints the raw response-key casing, won't match canonical op keys (Copilot, main.rs:1539) — correctness
> `build_rust_host_shims` prints `host-call\t{op}` using the *recorded host-response key* casing.
> `(host-calls …)` verification compares observed calls against the canonical op keys emitted by
> runners (kebab-normalized effect), so a response key like `Param.width` would produce an observed
> call that can't match a canonical `param.width` fixture. Store/print the canonicalized op key
> instead.

This is the substantive one: the observed-call key is printed in the response-key casing
(`Param.width`) but `(host-calls …)` verification expects the canonical kebab-normalized form
(`param.width`), so host-call assertions on the Rust backend would never match. Canonicalize the op
key before printing/storing it.
