# List.update state transitions (2026-08-11)

Angle: List.update appears in 14-effects only in landed RRB pins (value
position); as the ARM's state transition at a DRAWN index, and the ring-buffer
cursor idiom, were uncovered.

GREEN x3:
- lu1: update at the op's index argument; poke answers the OLD cell; later
  reads see the write and its untouched neighbor — 209930/409899
- lu2: RING BUFFER — (list, cursor) tuple state, cursor rotates mod 3, the
  fourth put overwrites slot 0 — 118/18

Pin candidates: 253 pool.
