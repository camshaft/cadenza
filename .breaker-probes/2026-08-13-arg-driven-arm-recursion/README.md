# 2026-08-13 arg-driven pure recursion in the arm (tick 1400)

- `cla1.sexp` — the arm runs a PURE recursion whose depth is driven by the
  crossed ARGUMENT (collatz length, budget k=64): n=27 spins 111 frames inside
  ONE dispatch (64 hits the budget), n=3 just 7. vs cz1 (the collatz WALK
  performs per step — dispatch-count data-dependent): here the recursion is
  wholly INSIDE one dispatch frame — arm-frame depth, not dispatch count.
  State accumulates lengths; total exposes the sum. PASS ×3 (7080015/64080072).
