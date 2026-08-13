# 2026-08-13 priority-pop protocol (tick 1416)

- `pq1.sexp` — SELECTION-order protocol: pushes append unsorted (priority,value)
  pairs; popmin runs TWO recursive walks per dispatch — find-min (strict < so
  the FIRST of equal priorities wins, deterministic) then drop-at (index-filtered
  rebuild into a fresh annotated empty). Seed = the middle push's priority:
  n=2 pops 20 then 10; n=9 pops 10 then 30 (the seeded element left behind).
  vs mns1/cst1 (positional stacks): removal position here is DATA-dependent,
  and the two walks share the dispatch frame. PASS ×3 (1232010/1231030).
