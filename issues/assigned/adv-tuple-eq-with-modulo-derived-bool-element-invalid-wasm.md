# MISCOMPILE (wasm-only, v-property-testing): structural tuple `=` with a `%`-derived Bool element → invalid wasm

**Severity:** correctness — cdz compile --target wasm SUCCEEDS but cdz-run/wasmtime REJECTS at instantiation
(`wasm[0]::function[6]`). Rust backend emits fine (backend disagreement).

## Minimal repro
`(do (def (main (: s Int64)) (= (tuple 5 (= (% s 2) 0)) (tuple 5 (= (% s 2) 0)))) (export main))`

## Characterization (v-property-testing, precise)
- Trigger = [structural tuple `=`] × [a Bool element] × [that Bool computed via `%` (modulo/remainder)].
- `(tuple 5 (< s 3))` compared → OK. `(tuple 5 (= (+ s 1) 6))` → OK. `(tuple 5 (= (& s 1) 0))` → OK. Only `%` breaks it.
- `(= (% s 2) 0)` as a SCALAR (not in a compared tuple) → OK. `(tuple 5 true)` compared → OK. `(tuple s s)` compared → OK.
- Likely: the synthesized per-element compare mis-emits the `%` (i64.rem_s?) or a valtype/packed-bool mismatch
  when the Bool subexpression contains a remainder op.

## Seam
wasm backend structural-equality emission (backend/wasm/select.rs, the compound-`=` compare-function synthesis)
× the `%`/Int64.rem lowering in that nested context. Runtime tuple/record/set/map `=` otherwise works — narrow
miscompile within the working tuple-`=` path.

Graded case Fails-wasm. v-property-testing has a `<`-based workaround for its own corpus witness (unblocked).

## SHARPENED 2026-07-16 (v-property-testing): it's CONST-DIVISOR div/rem, not modulo-specific
FAILS (invalid wasm): (% s 2) AND (/ s 2) — checked DIV or REM by a CONSTANT divisor — feeding a Bool in a compared tuple.
WORKS: (% s s) non-constant divisor (runtime zero-check lowering) → OK; %-result as the INT element (not bool) → OK;
  non-div bool subexprs → OK; the same bool as a SCALAR (no tuple) → OK.
PRECISE TRIGGER = [checked DIV or REM by a CONSTANT divisor] → [Bool] → [element of a structurally-=-compared tuple].
So the CONST-DIVISOR optimized div/rem form is mis-emitted specifically inside the SYNTHESIZED per-element compare
function of a compound `=` (not in normal expression position, where it's fine). Seam narrows to: compound-= compare-fn
synthesis × the const-divisor div/rem lowering. wasm-only; rust fine.
