# fnc1 — funicular counterbalance (2026-08-17, tick 1675)

Attack: a DERIVED-TWIN answer — the mid-slope run answers BOTH cars' positions
from ONE field (`(+ posA 2)` and `(- 4 (+ posA 2))` — the mirror is derived,
never stored), while the arrival branch resets TWO fields and bumps a third
(unload + swap-to-base + trip count in one rebuild). The climb compound
`(+ posA 2)` appears 4x (test, both mirror slots, rebuild).

Differential: starting mid-slope (2) vs base (0): n=10's first board is
REFUSED (902) but its early arrival (710, zero passengers) opens the base
for board #2 — so the runs take opposite branch sequences at EVERY dispatch
([902,710,22,22] read 122 vs [33,22,902,713] read 100).

Hand model: n=10 → 902710022022122; n=0 → 33022902713100 (base-1000; two
earlier drafts had converging reads — fixed by the arrival-swap rebuild +
the *2 seed spread).

Pass ×3 wasm + rust + rust-async on trunk 0db236a9d.
