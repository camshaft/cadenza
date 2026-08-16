# tns ladder — race to four with absorbing dead rally (2026-08-16, tick 1605)

Attack: an ABSORBING post-win state (the dead rally answers 99 WITHOUT touching
the score — `(resume (: 99 Int64) st)` reuses the matched state unchanged) in
front of a 3-branch arm whose live branches rebuild the pair and stamp a
century flag via `(/ (+ p 1) 4)` on the winner's fourth point. The product test
`(* (- 4 pa) (- 4 pb))` = 0 detects either player's win in one expression.

Differential: seeds hand OPPOSITE players the sweep (stroke+seed mod 3), so the
same row shapes swap tally columns and the dead-rally sentinel lands at the
same position with mirrored closing scores (040 vs 004).

## Envelope datapoint
- tns1 (6 rallies × 3-branch arm): F24 instruction-budget clean decline —
  confirms the 3-branch × 6-dispatch corner of the envelope map.
- tns2 (5 rallies): PASSES ×3 wasm + rust + rust-async (rust gates green under
  a 60→100 load storm, backgrounded + verdict-grep).

Model hand-verified (python, banked in transcript):
- n=10: rows [1,2,3,104,99] score 4 → 1002003104099004
- n=0:  rows [10,20,30,140,99] score 40 → 10020030140099040

Trunk f9aceecd6.
