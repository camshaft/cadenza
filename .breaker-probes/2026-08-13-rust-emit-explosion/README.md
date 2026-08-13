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

## Tick 1367 — cross-backend + cross-state boundary (REVISES the hypothesis)
Emit-size series at k dispatches (same driver, arm varies):
| state/arm | backend | k=4 | k=5 | k=6 | k=7 | growth |
|---|---|---|---|---|---|---|
| Map computed-key | rust-async | 46K | 172K | 680K | 2.7M | ~4.0x |
| Map computed-key | wasm | 11K | 42K | 165K | 687K | ~4.0x |
| List computed-idx | rust-async | 11K | 24K | 66K | 190K | ~2.8x |
| scalar (+ s v) | rust-async | — | — | — | 3.7K | flat |
The explosion is NOT rust-specific: wasm grows at the SAME ~4x rate for the Map
arm — the continuation duplication lives in the SHARED specialized fold, with
per-backend constant factors (wasm ~4x smaller constants; rustc just hits its
limits first via the single-line 2.7MB source). Scalar arms are flat — the
duplication requires a HEAP-collection state arm. Sources: f24-{list,scalar}*.sexp.

## Tick 1369 — CALL-SITE not dispatch-count
`f24-loop.sexp` — the SAME 7 dispatches driven by ONE recursive call site
(walk k): rust-async emit 5.2KB, wasm 1.3KB. vs 2.7MB / 687KB for 7 straight-line
let-bound call sites. So the 4x-per-site growth is per STATIC CALL SITE — the
specialized fold re-specializes (and duplicates the rest of the continuation)
at each syntactic S.add site; a loop-driven program with one site is immune.
Workaround exists (loop-ize); still a real cliff for straight-line effect code.
