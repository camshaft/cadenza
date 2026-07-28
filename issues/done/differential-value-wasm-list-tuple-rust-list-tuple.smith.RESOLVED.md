# SMITH FINDING — backends DISAGREE on value: [value] wasm=(list (tuple -340282350000000000000000000000000000000.0)) rust=(list (tuple -340282349999999991754788743781432688640.0))

_Filed by `cdz-smith` (the compiler fuzzer). This is an auto-generated finding: a
generated program made the compiler PANIC, HANG, or emit INVALID wasm — never valid
behavior, since the compiler reports every legitimate "no" as a diagnostic. Triage, fix,
then rename this file `.RESOLVED.md` (or `.REJECTED.md` with a rationale) so it is not
re-triaged._

- **Category:** differential
- **Compiler commit:** `a2157b996`
- **Hits:** 1
- **Signature:** `differential-value-wasm-list-tuple-rust-list-tuple`

## Reproducer

`differential-value-wasm-list-tuple-rust-list-tuple.smith.sexp` (also inline):

```scheme
(do (def (main) (list (tuple -3.4028235e38))) (export main))
```

Reproduce in-process:

```
cargo run -p cdz-smith -- verify differential-value-wasm-list-tuple-rust-list-tuple.smith.sexp
```

## Backend value disagreement (miscompile)

Both emit backends produced a VALID artifact, but they DISAGREE on the program's
result — the wasm component (run via `cdz-run`) and the Rust backend (run via
`cdz run-rust`) computed different values, or one ran to a value where the other
trapped. The backends share the front-end and diverge below the emit seam, so this
is a lowering bug on one side. The crash/invalid-wasm oracles are blind to it.

- **Disagreement:** [value] wasm=(list (tuple -340282350000000000000000000000000000000.0)) rust=(list (tuple -340282349999999991754788743781432688640.0))

; ===== fuzzer DUP (2026-07-21, trunk a2157b996) =====
; DUPLICATE of the Float64-render divergence routed to v-rust-backend. Minimal: (list (tuple -3.4028235e38)).
; Sole differing leaf -3.4028235e38: wasm shortest-round-tripping vs rust full binary expansion. Same
; known dedup weakness. Marked DUP; NOT re-routed. NOTE: this is the ~4th recurrence — the diff keeps
; re-hitting +/-3.4028235e38 in new wrappers. Root fix = v-rust-backend render (stops recurrence).
