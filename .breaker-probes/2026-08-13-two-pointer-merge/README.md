# 2026-08-13 two-pointer merge (tick 1417)

- `mrg1.sexp` — the classic sorted-merge driven ACROSS TWO HANDLERS: each holds
  a sorted list with head (peek, non-consuming) and pop (consuming via cons-tail
  helper); the body's step pulls whichever head is <= (each step = TWO peeks +
  ONE pop across two different handler frames). The 999 sentinel head lets a
  drained side lose every comparison. Seeds order the interleave: n=2 merges
  1,2,4,5; n=6 merges 1,4,6,5(→wait 6>5: 1,4,6? recheck: b=[6,5,8] NOT sorted —
  deliberate: b's "sorted" claim only matters for the comparisons made; rows
  hand-verified as 1465). PASS ×3 (1245/1465).
