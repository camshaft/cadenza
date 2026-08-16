# Parallel bit accumulators (2026-08-12)

Angle: THREE independent bitwise folds (running AND with a zero-seed guard,
OR, XOR) living in one tuple state, all three transitioning per dispatch —
the landed bitwise pins are value-position; accumulator STATES uncovered.

GREEN x3:
- bwa1: payloads 12,10,n,0 — the final read sums all three accumulators
  (n=6: 8+14+8... per model 14 wait the read is PRE-transition; 14/32)

Staged: 14c pool at 11.
