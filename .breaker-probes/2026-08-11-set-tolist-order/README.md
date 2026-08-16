# Set.to-list order across dispatch (2026-08-11)

Angle: the Bytes-order arc pinned Set.to-list's Bytes DECLINE; the scalar
ORDER GUARANTEE (ascending) through the state thread — insertion order
scrambled by dispatches — was unpinned in effects position.

GREEN x3 (order is ascending, uniform across backends):
- st1: inserts 3,1,n across dispatches; positional drain reads 1,3,7 / 1,2,3
  — 137/123
- st2: NEGATIVES + zero (-3,0,n) — signed ascending holds (offset fold keeps
  digits positive) — 475059/424750

Pin candidates: 254 pool.
