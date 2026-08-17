# lnd ladder — laundromat washer→dryer pipeline (2026-08-17, tick 1670)

Attack: TWO sequential conditional moves fused into one 4-leaf arm — the
advance must FIRST empty the dryer into done, THEN hand the washer's load to
the dryer, with all four (d>0)x(w>0) combinations as distinct leaves whose
rebuilds move different field PAIRS (a pipeline stage in one dispatch). The
refused load resumes st untouched showing the occupant.

## Envelope
- lnd1 (5 dispatches x 4-leaf advance): instruction-budget clean decline —
  consistent with cnv1 (4-branch @ 5 declines).
- lnd2 (4 dispatches): PASSES x3 all backends. Differential: the seed leaves
  a leftover load in the washer → n=10's first load REFUSED (902), pipeline
  phase-shifted (dryer gets the leftover; read 204 = 2 done, washer 0,
  dryer 4) vs n=0's clean flow (read 304).

Hand model: n=10 → 902020041042204; n=0 → 31030041043304 (base-1000).

Ops note: a sed-derive of the trimmed body broke BOTH the paren balance AND
the (call...) clauses (auto-repair stripped a structural closer → "no primary
result clause"). REGENERATED from the f-string template with balance assert
— the stm1 generate-don't-derive rule now applies to op-count trims too.

Pass ×3 wasm + rust + rust-async on trunk eae898166. lnd1-shape (5-disp)
held conceptually for (b); not banked (superset of cnv1's datapoint).
