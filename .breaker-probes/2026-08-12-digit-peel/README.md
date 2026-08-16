# Digit-peel negative face (2026-08-12)

Angle: the digit-peel state machine EXISTS as dg1 (14c, positive seeds only)
— caught by the free-id pre-check mid-flow (a landed pin appeared between my
pre-check and bank: the check ran pre-fetch. RULE: run the free-id check
AFTER the fetch/sync, not before). The uncovered residue: NEGATIVE seeds,
where truncated / and dividend-sign % must agree per peel.

GREEN x3:
- dgn1: -251 -> digits -1,-5,-2 -> -152; -8 -> -8,0,0 -> -800

Staged: 14c pool at 8 (…, rle1, dgn1). cp2 pruned this tick (v-effects
pinned oracle binary-search).
