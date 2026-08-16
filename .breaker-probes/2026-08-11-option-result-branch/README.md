# Option verdicts across dispatch (2026-08-11)

Angle: state-dependent Some/None verdicts from the arm (the verdict FLIPS per
dispatch as state compares to the key), and an Option built by one dispatch
STORED into the tuple state for a later drain.

GREEN x3:
- op1: the arm's Option verdict flips per dispatch; both variants cross; pure
  helper unwraps — 10097/-303
- op2: dispatch-built Option stored INTO the tuple state, drained later —
  Some and None both persist through the thread — 10197/-703

FENCE (banked): `try` whose operand PERFORMS, inside a cross-fn helper,
declines ("not yet reducible") — the arm-side try pins use pure operands;
the performing-operand try is a fold frontier (tp1).

Pin candidates: staged pool.

## v-effects (2026-08-11): op1 REDUNDANT (state-dependent Some/None resume-value verdict already pinned at 14c:4 op2 — arm resumes (if (> k s) (Some..) (None)) matched at two sites). SKIPPED op1.
op2 (THIS probe) CLAIMED-HELD — DISTINCT: an Option built by one dispatch STORED INTO the tuple handler state, drained by a LATER dispatch (Some/None persist through the thread). green x3 + opt-sweep 0-div, 10197/-703 traced. HELD behind sl1 (behind queued MR tk1 ac36ad9c6). tp1 performing-operand-try fence stays banked.

## SENT by v-effects (2026-08-11)
op2 pinned to 14b (MR fa55aad09, +3 baseline lines). CLAIMED-HELD -> SENT.
