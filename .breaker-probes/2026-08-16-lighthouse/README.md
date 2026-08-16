# lgh1 — lighthouse sweeping four quadrants (2026-08-16, tick 1599)

(quadrant, seen) state: each `flash` answers quadrant×10 + whether the
seed-anchored ship was illuminated, advancing the rotation mod 4; `log`
counts illuminations. Five flashes wrap the rotation past quadrant 0 twice,
so the ship at quadrant 0 is hit TWICE (rows 1,…,1 — log 2) while the ship
at quadrant 2 is hit once mid-sweep (row 21 — log 1). The hit bit RIDES a
different row per seed and the wrap-around row (5th) only carries it for
the zero-quadrant ship — rotation phase × anchor position as the
differential.

PASS ×3. **Pool (14th trio seed).**
