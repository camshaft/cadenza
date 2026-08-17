# tun1 — orchestra tuning round (2026-08-17, tick 1694)

Attack: SIGNED integer halving toward a target — the retune rebuild shifts
`(+ pitch (/ (- 42 (+ pitch off)) 2))` where the gap can be NEGATIVE (signed
division truncation toward zero is the pin: a -3 gap halves to -1, not -2),
and the answer takes |gap| via an if-negate. The gap compound
`(- 42 (+ pitch off))` appears x6 (both band tests, the abs pair, the
rebuild). Band test = two comparisons as nested-if AND with a NEGATIVE
right bound (>= -1).

Differential: stand pitch 41 vs 40: n=10's first section plays IN TUNE (411)
and the drift stays small; n=0 retunes at once (821) and every later gap
echoes the half-steps differently (reads 4122 vs 4113 — tuned counts 2 vs 1).

Hand model: n=10 → 4118218324214122; n=0 → 8218228334214113 (mixed base).

Pass ×3 wasm + rust + rust-async on trunk e4b91e88b.
