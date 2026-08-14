# 2026-08-14 compensating transaction log (tick 1459)

- `cmt1.sexp` — every do applies its delta AND pushes the INVERSE onto the undo
  stack; compensate pops the LAST inverse and applies it — strict LIFO unwind
  back to the seed (the final compensate lands exactly on n, proving the
  inverses compose to identity). Completes the undo-family triangle: und1 =
  single-slot last-delta, rpl1 = replay-forward kept log, cmt1 = pop-backward
  inverse stack. The drained compensate answers -99. PASS ×3
  (151815251510/50805150500).
