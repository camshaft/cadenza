# tax1 — progressive tax brackets (2026-08-15, tick 1548)

Scalar audit total + bandtax callee splitting income into three bands via
chained MATCH BINDERS OVER IF-EXPRESSIONS (lo/mid/hi clamps), truncating
10/20/30% rates. Seed-shaped bracket edge (40 vs 20): the wider bracket
taxes every income less (3,11,29,0 → 43 vs 4,14,37,0 → 55); the sub-bracket
income (9) taxes to zero on both.

## Fence scope refinement
kgt0 established binder-over-IF DECLINES — but that was in the handler ARM.
tax1's binder-over-if chains live in a CALLEE DEF (bandtax) and PASS ×3.
So the binder-scrutinee fence (if/perform scrutinees) is ARM-scOPED — the
same shape in a plain def compiles. Sharpens the workaround docs: move the
binder-over-if into a helper def, OR expand to a lattice in the arm.

PASS ×3. **Pool (with mnc1; +1 fills the 9th trio).**
