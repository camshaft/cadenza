# orq1 — order-book spread tracker (2026-08-15, tick 1560)

(best-bid, best-ask) state with a let-free spread-of callee (0 = empty side):
`bid` raises the best bid, `ask` lowers the best ask — both answer the spread
or -1 while a side is empty — and a CROSSED book (bid ≥ ask) resets both
sides answering 0. The ask-15 tail row lands on an empty book post-cross,
answering -1 again (reset-then-reopen pinned).

Seed shifts only the opening bid (7 vs 5): the first spread differs (5 vs 7)
while the tightening ladder (3, 1), the cross (0), and the tail (-1) are
SHARED — one divergent row inside an otherwise identical stream, the
narrowest divergence design in the pool.

3-level nested-if arms over a 2-tuple, cheap branches — envelope-safe.
Flip-checks this tick: lstM/medK/phs1/bnf1/kgt0/rlyC all still decline on
the nested-let-flatten base (52e5bb3aa) — no fence movement.

PASS ×3. **Pool (with sfl1; +1 fills the 12th trio).**
