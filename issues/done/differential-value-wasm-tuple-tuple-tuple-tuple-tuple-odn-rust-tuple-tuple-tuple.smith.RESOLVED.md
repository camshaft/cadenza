# SMITH FINDING — backends DISAGREE on value: [value] wasm=(tuple 10.39 (tuple 10.39 (tuple 10.39 (tuple 10.39 (tuple "odn" 13.51 340282350000000000000000000000000000000.0) 151.96) 127) 127) 127) rust=(tuple 10.39 (tuple 10.39 (tuple 10.39 (tuple 10.39 (tuple "odn" 13.51 340282349999999991754788743781432688640.0) 151.96) 127) 127) 127)

_Filed by `cdz-smith` (the compiler fuzzer). This is an auto-generated finding: a
generated program made the compiler PANIC, HANG, or emit INVALID wasm — never valid
behavior, since the compiler reports every legitimate "no" as a diagnostic. Triage, fix,
then rename this file `.RESOLVED.md` (or `.REJECTED.md` with a rationale) so it is not
re-triaged._

- **Category:** differential
- **Compiler commit:** `866aab024`
- **Hits:** 1
- **Signature:** `differential-value-wasm-tuple-tuple-tuple-tuple-tuple-odn-rust-tuple-tuple-tuple`

## Reproducer

`differential-value-wasm-tuple-tuple-tuple-tuple-tuple-odn-rust-tuple-tuple-tuple.smith.sexp` (also inline):

```scheme
(do (def (main) (tuple 10.39 (tuple 10.39 (tuple 10.39 (tuple 10.39 (tuple "odn" 13.51 3.4028235e38) ) 127) 127) 127)) (export main))
```

Reproduce in-process:

```
cargo run -p cdz-smith -- verify differential-value-wasm-tuple-tuple-tuple-tuple-tuple-odn-rust-tuple-tuple-tuple.smith.sexp
```

## Backend value disagreement (miscompile)

Both emit backends produced a VALID artifact, but they DISAGREE on the program's
result — the wasm component (run via `cdz-run`) and the Rust backend (run via
`cdz run-rust`) computed different values, or one ran to a value where the other
trapped. The backends share the front-end and diverge below the emit seam, so this
is a lowering bug on one side. The crash/invalid-wasm oracles are blind to it.

- **Disagreement:** [value] wasm=(tuple 10.39 (tuple 10.39 (tuple 10.39 (tuple 10.39 (tuple "odn" 13.51 340282350000000000000000000000000000000.0) 151.96) 127) 127) 127) rust=(tuple 10.39 (tuple 10.39 (tuple 10.39 (tuple 10.39 (tuple "odn" 13.51 340282349999999991754788743781432688640.0) 151.96) 127) 127) 127)

; ===== fuzzer DUP (2026-07-21, trunk 866aab024) =====
; DUPLICATE of the compound-element Float64-render divergence routed to v-runtime (float_leaf shortest
; {:e} vs full expansion). Sole differing leaf = tuple-element 3.4028235e38. Marked DUP; NOT re-routed.
