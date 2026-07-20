# SMITH FINDING — backends DISAGREE on value: [value] wasm=(tuple 340282350000000000000000000000000000000.0) rust=(tuple 340282349999999991754788743781432688640.0)

_Filed by `cdz-smith` (the compiler fuzzer). This is an auto-generated finding: a
generated program made the compiler PANIC, HANG, or emit INVALID wasm — never valid
behavior, since the compiler reports every legitimate "no" as a diagnostic. Triage, fix,
then rename this file `.RESOLVED.md` (or `.REJECTED.md` with a rationale) so it is not
re-triaged._

- **Category:** differential
- **Compiler commit:** `274f3dfdf`
- **Hits:** 1
- **Signature:** `differential-value-wasm-tuple-rust-tuple`

## Reproducer

`differential-value-wasm-tuple-rust-tuple.smith.sexp` (also inline):

```scheme
(do (def (main) (tuple 3.4028235e38)) (export main))
```

Reproduce in-process:

```
cargo run -p cdz-smith -- verify differential-value-wasm-tuple-rust-tuple.smith.sexp
```

## Backend value disagreement (miscompile)

Both emit backends produced a VALID artifact, but they DISAGREE on the program's
result — the wasm component (run via `cdz-run`) and the Rust backend (run via
`cdz run-rust`) computed different values, or one ran to a value where the other
trapped. The backends share the front-end and diverge below the emit seam, so this
is a lowering bug on one side. The crash/invalid-wasm oracles are blind to it.

- **Disagreement:** [value] wasm=(tuple 340282350000000000000000000000000000000.0) rust=(tuple 340282349999999991754788743781432688640.0)

; ===== fuzzer DUP (2026-07-20, trunk 274f3dfdf) =====
; DUPLICATE of differential-value-wasm-tuple-tuple-tuple-tuple-tuple-true-rust-tuple-tuple-tupl
; (already PM-triaged spec-settled + routed v-rust-backend). SAME root cause: Float64 3.4028235e38
; renders shortest-round-tripping on wasm (correct) vs full binary expansion on rust run-rust (the
; bug). The harness dedup keys on the render string incl. the tuple-nesting wrapper, so this
; minimal 1-tuple form filed as a separate bucket. This is a CLEANER minimal repro — the fix agent
; may prefer it. Not re-routed as a new issue (noise); marked DUP so it isn't independently triaged.
