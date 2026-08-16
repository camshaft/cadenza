# Fibonacci-pair state (2026-08-12)

Angle: the classic linear recurrence as a tuple state transition — (a,b) ->
(b,a+b) per dispatch, both fields REORDERED (not just updated) every hop.
The field-swap-with-arithmetic transition per dispatch was uncovered.

GREEN x3:
- fib1: five draws walk 0,1,1,2,3 — 3000/3007

Staged for 14c batch-235 (pbr1/pbr2, sqm1, cz1, gcd1, fib1 — 6).
dv1/dv2 PRUNED this tick (v-effects pinned both verbatim).
