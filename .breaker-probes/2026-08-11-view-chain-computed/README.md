# Chained view reads with computed bounds (2026-08-11) — post-#18 hardening

Angle: #18's fix floated the index-operand scratch floor for SINGLE String.at/
Bytes.at reads; CHAINED views (slice-then-at, slice-of-slice) with computed
bounds over the effect-grown rope stack more scratch operands in one emit —
the natural next place for a floor miss.

GREEN x3 (fresh binary in-chain):
- vc1: computed slice of the rope, then computed at INTO the slice — 1/-1
  (the -1 face also pins the None short-circuit through the chain)
- vc2: slice OF a slice, both bounds computed — double view depth — 2/3

No counterexample — the floor float covers chained operands. Pin candidates.
