# Generic sum with rope payload as state (2026-08-11)

Angle: user generic sums (gs family) are pinned with SCALAR payloads; a ROPE
payload inside the generic wrapper threading as handler state was uncovered.
Also adjacent to #18 (rope through state thread) — the generic wrapper does
NOT trigger the invalid-wasm class (no computed-index rope-view read).

GREEN x3:
- gr1: (Box a) = Full a | Hole; Hole->Full transition, payload byte-len across
  dispatch — 19/19
- gr2: the rope grows INSIDE Full each recursive dispatch (per-hop unwrap/
  rebuild), Hole seeds, drain reads payload length — 5/-1

Pin candidates: 239 pool.
