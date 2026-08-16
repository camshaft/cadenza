# stm1 — postage meter with weight bands (2026-08-16, tick 1633)

Attack: a CEILING-DIVISION band compound `(+ 5 (* (/ (+ (- g 20) 9) 10) 2))`
appearing 4x in the heavy branch (affordability test, reject answer, taken
answer, taken rebuild) — the (+9)/10 started-unit idiom's first outing in an
arm; nested-if band split (light/heavy) each with its own refuse/take pair
(4 leaves).

Differential: seed sizes the ink tank (20 vs 35): n=0 rejects the 80g parcel
(917 row, audit 421 = 4 ink, 2 franked, 1 rejected); n=10 franks all three
(172 row, audit 230 = 2 ink... 35-5-11-17=2, 3 franked, 0 rejected). Rejection
row itself is seed-flipped (two earlier drafts had both seeds rejecting —
re-keyed the tank increment twice).

Ops note: hand-written deep nesting produced a paren slip (depth trace showed
line-22 collapse); REWROTE programmatically with an f-string template + a
balance assert before writing. For 4x-repeated compounds, generate — don't
hand-indent.

Hand model: n=10 → 50011901720230; n=0 → 55011409170421 (base-10000).

Pass ×3 wasm + rust + rust-async on trunk 931c11dd3.
