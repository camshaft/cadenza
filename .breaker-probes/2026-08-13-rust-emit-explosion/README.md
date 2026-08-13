# 2026-08-13 rust-emit explosion — FINDING #24 (tick 1365)

A 7-dispatch Map-state handler (computed-key lookup + insert per arm, ~20 source
lines) emits 2.7MB of rust — ONE 2.7MB line — for BOTH -t rust and -t rust-async.
Measured emit-size series (rust-async, same program at k dispatches):
  k=4: 46KB · k=5: 172KB · k=6: 680KB · k=7: 2.7MB   (~4x per dispatch)
rustc handles the sync 2.7MB emit; SIGSEGVs on the async emit REPRODUCIBLY
(3 attempts at loads ~30/~28/~10 — NOT a load artifact as first logged).
wasm emit unaffected (pfxM passes wasm; rust passes when rustc survives).

Hypothesis: the per-dispatch specialized fold duplicates the continuation per
dispatch in the rust emit path — each S.add's resume body inlines the remaining
chain, giving 4^k growth. Compile-time size bug, not a miscompile.

- `pfxM-s.sexp` — standalone 7-dispatch source (compile with -t rust-async to repro)
- `pfxM-{4,5,6}.sexp` — the size-series sources
- `pfxM-7-gatecase.sexp` — the gate case (wasm+rust green; rust-async blocked on rustc)
Filed → concierge (v-rust-backend op-paused; fold-owner routing needed).
