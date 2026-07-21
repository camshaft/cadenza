# SMITH FINDING — backends DISAGREE on value: [value] wasm=(tuple false (tuple (tuple false (tuple (tuple "i" -340282350000000000000000000000000000000.0 "i") (tuple 146.69 1089 "i") (tuple 146.69 1089 "i")) (tuple)) 127 127) 127) rust=(tuple false (tuple (tuple false (tuple (tuple "i" -340282349999999991754788743781432688640.0 "i") (tuple 146.69 1089 "i") (tuple 146.69 1089 "i")) (tuple)) 127 127) 127)

_Filed by `cdz-smith` (the compiler fuzzer). This is an auto-generated finding: a
generated program made the compiler PANIC, HANG, or emit INVALID wasm — never valid
behavior, since the compiler reports every legitimate "no" as a diagnostic. Triage, fix,
then rename this file `.RESOLVED.md` (or `.REJECTED.md` with a rationale) so it is not
re-triaged._

- **Category:** differential
- **Compiler commit:** `767a86c62`
- **Hits:** 1
- **Signature:** `differential-value-wasm-tuple-false-tuple-tuple-false-tuple-tuple-i-i-tuple-i-tu`

## Reproducer

`differential-value-wasm-tuple-false-tuple-tuple-false-tuple-tuple-i-i-tuple-i-tu.smith.sexp` (also inline):

```scheme
(do (def (main) (tuple false (tuple (tuple false (tuple (tuple "i" -3.4028235e38 "i")  ) ) 127 127) 127)) (export main))
```

Reproduce in-process:

```
cargo run -p cdz-smith -- verify differential-value-wasm-tuple-false-tuple-tuple-false-tuple-tuple-i-i-tuple-i-tu.smith.sexp
```

## Backend value disagreement (miscompile)

Both emit backends produced a VALID artifact, but they DISAGREE on the program's
result — the wasm component (run via `cdz-run`) and the Rust backend (run via
`cdz run-rust`) computed different values, or one ran to a value where the other
trapped. The backends share the front-end and diverge below the emit seam, so this
is a lowering bug on one side. The crash/invalid-wasm oracles are blind to it.

- **Disagreement:** [value] wasm=(tuple false (tuple (tuple false (tuple (tuple "i" -340282350000000000000000000000000000000.0 "i") (tuple 146.69 1089 "i") (tuple 146.69 1089 "i")) (tuple)) 127 127) 127) rust=(tuple false (tuple (tuple false (tuple (tuple "i" -340282349999999991754788743781432688640.0 "i") (tuple 146.69 1089 "i") (tuple 146.69 1089 "i")) (tuple)) 127 127) 127)

; ===== fuzzer DUP (2026-07-21, trunk 767a86c62) =====
; DUPLICATE of the Float64-render divergence routed to v-rust-backend. Sole differing leaf is
; -3.4028235e38: wasm -340282350000000000000000000000000000000.0 (shortest round-tripping) vs rust
; -340282349999999991754788743781432688640.0 (full binary expansion). This is the NEGATIVE variant;
; all other leaves (false/127/"i"/146.69/1089) agree. Same known dedup weakness (signature keys on
; the full render string incl. tuple nesting). Marked DUP; NOT re-routed.
