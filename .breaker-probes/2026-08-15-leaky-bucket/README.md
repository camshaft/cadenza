# lky1 — leaky-bucket meter (2026-08-15, tick 1507)

SCALAR level state: `arrive` fills toward the seed-shaped capacity (n+8)
answering only the OVERFLOW spill — the level clamps at the brim; `drain`
leaks 5 clamped at empty answering the new level. The small bucket (cap 8)
spills twice (5 then 4... rows 0,5,3,4,3,0) where the large one (cap 18)
never spills (0,0,8,0,12,7) — opposite zero/nonzero patterns per row.

Complements odf1 (token bucket = credit-side) with the queue-side twin
(spill on overflow vs reject on insufficient credit). F24-safe: 2-branch
arms over a scalar, capacity recomputed in-arm but branches are cheap.

PASS ×3 wasm. **Pool.**
