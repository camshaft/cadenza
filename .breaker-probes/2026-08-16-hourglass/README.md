# hgl1 — hourglass with flips (2026-08-16, tick 1590)

(top, total) state: `tick` drains 3 grains clamped at empty; `flip` swaps
the bulbs — new top = total − old top (conservation through the flip).
The two totals (18 vs 8) CONVERGE mid-stream: after two drains the first
flip lands both glasses at top=6 (18−12 and 8−2 … wait: 18−12=6 and 8−2=6 —
the drain clamps make the flip outputs coincide), so rows 3-5 are IDENTICAL
(6,3,0), then the second flip restores each glass's own total (18 vs 8) and
they diverge again. Converge-then-diverge with the conservation law doing
the re-differentiation — the mirror of cyc1's clamp-forced convergence.

PASS ×3. **Pool (13th trio seed).**
