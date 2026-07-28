# SMITH FINDING — backends DISAGREE on value: [artifact] wasm=wasm value "e" rust=artifact-error error[E0433]: cannot find module or crate `cdz_num` in this scope

_Filed by `cdz-smith` (the compiler fuzzer). This is an auto-generated finding: a
generated program made the compiler PANIC, HANG, or emit INVALID wasm — never valid
behavior, since the compiler reports every legitimate "no" as a diagnostic. Triage, fix,
then rename this file `.RESOLVED.md` (or `.REJECTED.md` with a rationale) so it is not
re-triaged._

- **Category:** differential
- **Compiler commit:** `4c9e6867f`
- **Hits:** 1
- **Signature:** `differential-artifact-wasm-wasm-value-e-rust-artifact-error-error-e-cannot-find`

## Reproducer

`differential-artifact-wasm-wasm-value-e-rust-artifact-error-error-e-cannot-find.smith.sexp` (also inline):

```scheme
(do (def (main) "e") (export main))
```

Reproduce in-process:

```
cargo run -p cdz-smith -- verify differential-artifact-wasm-wasm-value-e-rust-artifact-error-error-e-cannot-find.smith.sexp
```

## Backend value disagreement (miscompile)

Both emit backends produced a VALID artifact, but they DISAGREE on the program's
result — the wasm component (run via `cdz-run`) and the Rust backend (run via
`cdz run-rust`) computed different values, or one ran to a value where the other
trapped. The backends share the front-end and diverge below the emit seam, so this
is a lowering bug on one side. The crash/invalid-wasm oracles are blind to it.

- **Disagreement:** [artifact] wasm=wasm value "e" rust=artifact-error error[E0433]: cannot find module or crate `cdz_num` in this scope

; ===== fuzzer REJECTED — infra false-positive (2026-07-22, trunk 4c9e6867f) =====
; NOT a backend divergence. All 12 artifact-mismatch buckets this sweep were rust-side
; error[E0433]: cannot find module or crate cdz_num — the cdz run-rust link failed because the
; libcdz_num/libcdz_rt rlibs beside the cdz binary were STALE (a trunk change bumped cdz-num; the
; leaner per-tick '--bin cdz'-only rebuild left the rlibs stale). After 'cargo build --release'
; refreshed the rlibs, cdz run-rust works again ((tuple) => value (tuple)) and the sweep is clean.
; This is the RUST-side analog of the stale-store trap. Rejected, not routed.
