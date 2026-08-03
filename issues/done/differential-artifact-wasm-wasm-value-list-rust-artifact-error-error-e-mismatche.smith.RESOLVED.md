# SMITH FINDING — backends DISAGREE on value: [artifact] wasm=wasm value (list 2147483647 -41) rust=artifact-error error[E0308]: mismatched types

_Filed by `cdz-smith` (the compiler fuzzer). This is an auto-generated finding: a
generated program made the compiler PANIC, HANG, or emit INVALID wasm — never valid
behavior, since the compiler reports every legitimate "no" as a diagnostic. Triage, fix,
then rename this file `.RESOLVED.md` (or `.REJECTED.md` with a rationale) so it is not
re-triaged._

- **Category:** differential
- **Compiler commit:** `7e38799c0`
- **Hits:** 1
- **Signature:** `differential-artifact-wasm-wasm-value-list-rust-artifact-error-error-e-mismatche`

## Reproducer

`differential-artifact-wasm-wasm-value-list-rust-artifact-error-error-e-mismatche.smith.sexp` (also inline):

```scheme
(do (def (main) (list (: 2147483647 UInt64) (match "km" (_ (match "km" (_ (match "km" (_ (match false (_ -41)))))))))) (export main))
```

Reproduce in-process:

```
cargo run -p cdz-smith -- verify differential-artifact-wasm-wasm-value-list-rust-artifact-error-error-e-mismatche.smith.sexp
```

## Backend value disagreement (miscompile)

Both emit backends produced a VALID artifact, but they DISAGREE on the program's
result — the wasm component (run via `cdz-run`) and the Rust backend (run via
`cdz run-rust`) computed different values, or one ran to a value where the other
trapped. The backends share the front-end and diverge below the emit seam, so this
is a lowering bug on one side. The crash/invalid-wasm oracles are blind to it.

- **Disagreement:** [artifact] wasm=wasm value (list 2147483647 -41) rust=artifact-error error[E0308]: mismatched types
