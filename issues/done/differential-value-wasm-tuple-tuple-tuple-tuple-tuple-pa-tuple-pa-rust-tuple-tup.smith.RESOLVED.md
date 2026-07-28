# SMITH FINDING — backends DISAGREE on value: [value] wasm=(tuple 34.46 (tuple 34.46 (tuple 34.46 (tuple 34.46 (tuple 340282350000000000000000000000000000000.0 "pa" 34.46) (tuple 340282350000000000000000000000000000000.0 "pa" 34.0)) 127) 127) 127) rust=(tuple 34.46 (tuple 34.46 (tuple 34.46 (tuple 34.46 (tuple 340282349999999991754788743781432688640.0 "pa" 34.46) (tuple 340282349999999991754788743781432688640.0 "pa" 34.0)) 127) 127) 127)

_Filed by `cdz-smith` (the compiler fuzzer). This is an auto-generated finding: a
generated program made the compiler PANIC, HANG, or emit INVALID wasm — never valid
behavior, since the compiler reports every legitimate "no" as a diagnostic. Triage, fix,
then rename this file `.RESOLVED.md` (or `.REJECTED.md` with a rationale) so it is not
re-triaged._

- **Category:** differential
- **Compiler commit:** `dd93b6905`
- **Hits:** 1
- **Signature:** `differential-value-wasm-tuple-tuple-tuple-tuple-tuple-pa-tuple-pa-rust-tuple-tup`

## Reproducer

`differential-value-wasm-tuple-tuple-tuple-tuple-tuple-pa-tuple-pa-rust-tuple-tup.smith.sexp` (also inline):

```scheme
(do (def (main) (tuple 34.46 (tuple 34.46 (tuple 34.46 (tuple 34.46 (tuple 3.4028235e38 "pa" 34.46) ) 127) 127) 127)) (export main))
```

Reproduce in-process:

```
cargo run -p cdz-smith -- verify differential-value-wasm-tuple-tuple-tuple-tuple-tuple-pa-tuple-pa-rust-tuple-tup.smith.sexp
```

## Backend value disagreement (miscompile)

Both emit backends produced a VALID artifact, but they DISAGREE on the program's
result — the wasm component (run via `cdz-run`) and the Rust backend (run via
`cdz run-rust`) computed different values, or one ran to a value where the other
trapped. The backends share the front-end and diverge below the emit seam, so this
is a lowering bug on one side. The crash/invalid-wasm oracles are blind to it.

- **Disagreement:** [value] wasm=(tuple 34.46 (tuple 34.46 (tuple 34.46 (tuple 34.46 (tuple 340282350000000000000000000000000000000.0 "pa" 34.46) (tuple 340282350000000000000000000000000000000.0 "pa" 34.0)) 127) 127) 127) rust=(tuple 34.46 (tuple 34.46 (tuple 34.46 (tuple 34.46 (tuple 340282349999999991754788743781432688640.0 "pa" 34.46) (tuple 340282349999999991754788743781432688640.0 "pa" 34.0)) 127) 127) 127)

; ===== fuzzer DUP (2026-07-21, trunk dd93b6905) =====
; DUPLICATE of the Float64-render divergence already routed to v-rust-backend (rust run-rust emits the
; full binary expansion 340282349999999991754788743781432688640.0 vs wasm shortest-round-tripping
; 340282350000000000000000000000000000000.0 for 3.4028235e38). The ONLY differing leaf here is that
; same float; "pa" is a generated STRING literal, not partial application. Same known dedup weakness
; (signature keys on render string incl. tuple nesting). Marked DUP; NOT re-routed.
