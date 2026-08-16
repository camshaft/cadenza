# irg ladder — irrigation controller, zone-scaled costs (2026-08-16, tick 1611)

Attack: the zone-scaled cost `(* amt (+ zone 1))` appears 3x in the taken
branch (condition, answer, rebuild) and the zone-advance `(% (+ zone 1) 3)`
appears in BOTH branches' answers and rebuilds (4x) — two distinct shared
compounds per arm, one branch-asymmetric (cost) and one branch-symmetric
(advance). Skip still advances the rotation (a branch that mutates 2 of 3
fields vs 2 different fields).

## Fence datapoint (scratch-locals budget, F24 class)
- irg1 (5 dispatches): DECLINES scratch-locals budget.
- irg2 (4 dispatches): STILL DECLINES — the 2-shared-compound arm is heavier
  than snk1's single-compound arm (which passed at 5 dispatches with 1 compound
  x4). Compound COUNT, not just repetition count, drives the fence.
- irg3 (3 dispatches): PASSES x3 all backends.
Envelope refinement: 2-branch x 3-tuple with TWO shared compounds fences at
3-4 dispatches; with ONE shared compound at 5-6 (snk1 passed 5, tns1 declined 6
with 3 branches). The (b) fix acceptance battery should include irg2.

Model slips caught pre-send: irg2's 4-dispatch pin initially only diverged in
the report row (weak pin — redesigned request set); irg3 derived from irg2 via
sed initially had STALE outputs (caught by hand-model re-run before gating).

irg3 hand model: n=10 rows [31,42,60] report 100 → 31042060100;
n=0 rows [31,42,900] report 310 → 31042900310.

Pass x3 wasm + rust + rust-async on trunk 68122fd42. irg1/irg2 held for the
(b) joint pass-pin.
