# lap1 — stopwatch with lap splits (2026-08-14, tick 1493)

3-op handler over (clock, last-mark, best-split): `tick` advances by a
seed-shaped stride ((n%3)+2, recomputed in-arm — NOTE this is the dst1
face-1 shape ((% n) in arm) at 9 dispatches and it PASSES; the dst
code-too-large face must need more than seed-recompute alone, likely the
3-way nested-if around it); `lap` answers the split since the last mark,
threading a -1 sentinel-seeded minimum; `bst` reads the best.

Middle lap of three is the unique best on both seeds (3/2 after strides).
9 dispatches. Whole trace scales by stride (3 vs 2) — every row differs.

PASS ×3 wasm. **Pool — completes hrn1/rrb1/lap1 (trio ready).**
