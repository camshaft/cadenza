# Gondola reflecting cable (2026-08-17)

- `gnd1.sexp` — signed-direction position walk with reflection at BOTH termini
  (negate dir, count trip), the new position let-bound ONCE and consumed by the
  terminus test, both resume answers, and both next-state tuples; direction bit
  derived arithmetically as (/ (+ dir 1) 2) rather than branched. Nested-if
  terminus test (= np 4) OR (= np 0). Seeds n%3 place the start at 1 vs 0 so
  the reflection points shift across runs. PASS x3 (wasm/rust/rust-async) at
  8deb431dd.

Envelope note: the pre-let draft — identical machine but with (+ pos dir)
repeated x6 through the arm (condition, both answers, both next-states) —
scratch-locals-DECLINED on wasm. One let hoisting the shared compound brings
it under the fence: repetition of a 2-leaf compound x6 in a 2-way branch
overflows where the let-bound form (1 compound + refs) passes. Consistent
with the pnb1/lom1 repetition-x-leaves law; new face: let-hoisting as the
boundary crossing itself.
