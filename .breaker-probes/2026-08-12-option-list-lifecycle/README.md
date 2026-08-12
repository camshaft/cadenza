# 2026-08-12 Option-of-List state lifecycle (tick 1332, base post-238 trunk)

- `olc1.sexp` — handler state `(Option (List Int64))` walked through a full lifecycle:
  None (uninitialized) → push initializes `Some [v]` → push appends → take scores
  (10*len + head) and RESETS to None → a later push RE-initializes. Exercises the
  None-annotation at seed AND in an arm's next-state position, plus Some-wrapped heap
  growth across dispatches. Only prior Option-typed state in 14* is a read-only
  `(Some 5)` get (14b:15) — no lifecycle/reset coverage. PASS ×3 (1223115/1228120).
