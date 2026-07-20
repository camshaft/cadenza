# SMITH FINDING — backends DISAGREE on value: [value] wasm=(tuple 10.41 (tuple 10.41 (tuple 10.41 (tuple 10.41 (tuple true 340282350000000000000000000000000000000.0))))) rust=(tuple 10.41 (tuple 10.41 (tuple 10.41 (tuple 10.41 (tuple true 340282349999999991754788743781432688640.0)))))

_Filed by `cdz-smith` (the compiler fuzzer). This is an auto-generated finding: a
generated program made the compiler PANIC, HANG, or emit INVALID wasm — never valid
behavior, since the compiler reports every legitimate "no" as a diagnostic. Triage, fix,
then rename this file `.RESOLVED.md` (or `.REJECTED.md` with a rationale) so it is not
re-triaged._

- **Category:** differential
- **Compiler commit:** `af0a646f7`
- **Hits:** 1
- **Signature:** `differential-value-wasm-tuple-tuple-tuple-tuple-tuple-true-rust-tuple-tuple-tupl`

## Reproducer

`differential-value-wasm-tuple-tuple-tuple-tuple-tuple-true-rust-tuple-tuple-tupl.smith.sexp` (also inline):

```scheme
(do (def (main) (tuple 10.41 (tuple 10.41 (tuple 10.41 (tuple 10.41 (tuple true 3.4028235e38)))))) (export main))
```

Reproduce in-process:

```
cargo run -p cdz-smith -- verify differential-value-wasm-tuple-tuple-tuple-tuple-tuple-true-rust-tuple-tuple-tupl.smith.sexp
```

## Backend value disagreement (miscompile)

Both emit backends produced a VALID artifact, but they DISAGREE on the program's
result — the wasm component (run via `cdz-run`) and the Rust backend (run via
`cdz run-rust`) computed different values, or one ran to a value where the other
trapped. The backends share the front-end and diverge below the emit seam, so this
is a lowering bug on one side. The crash/invalid-wasm oracles are blind to it.

- **Disagreement:** [value] wasm=(tuple 10.41 (tuple 10.41 (tuple 10.41 (tuple 10.41 (tuple true 340282350000000000000000000000000000000.0))))) rust=(tuple 10.41 (tuple 10.41 (tuple 10.41 (tuple 10.41 (tuple true 340282349999999991754788743781432688640.0)))))

; ===== PM triage (corpus-bugfix, 2026-07-20, trunk af0a646f7) — SPEC-SETTLED, routed v-rust-backend =====
; VERIFIED reproducing: wasm renders 3.4028235e38 as 340282350000000000000000000000000000000.0 (shortest
; round-tripping); rust run-rust as 340282349999999991754788743781432688640.0 (full binary expansion). Both
; parse to the SAME f64. CANONICAL = shortest round-tripping (SPEC: 12-metaprogramming:907/1100/1121/1155
; "shortest round-tripping decimal ... top of the exponent range"; reference-compiler.md:698/735 round-trip).
; So WASM is correct; the RUST value-renderer is the bug (emit shortest form via Grisu/Ryu, not exact
; expansion). NO concierge ask (spec settles it). Routed to v-rust-backend. Pin a top-of-exponent render case
; once fixed. Not a fix agent (their run-rust printer lane).

## Cleaner minimal repro (fuzzer, 2026-07-20) — use for the corpus pin
The fuzzer re-surfaced this (harness under-deduped on the render-string wrapper) and filed a CLEANER minimal
form, now marked .DUP: `(do (def (main) (tuple 3.4028235e38)) (export main))` — wasm=(tuple 3402823500...0.0)
vs rust=(tuple 3402823499999999917...640.0). Same spec-settled root (shortest-round-tripping canonical, rust
emits full binary expansion). corpus-bugfix: when v-rust-backend lands the render fix, PIN this minimal
top-of-exponent form (a single-float or 1-tuple, both backends -> shortest form) rather than the deep-nested
original. No re-route (fuzzer confirmed no action needed; original already routed to v-rust-backend).
