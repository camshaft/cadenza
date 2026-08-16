# aqm1 — aquarium feeder with overfeed penalty (2026-08-16, tick 1650)

Attack: TWO independent clamps in one protocol — the feed arm's min(h, a)
split (satisfied branch: constants a/h-a; overfed branch: h eaten, hunger
ZEROED, leftover (- a h) accumulating into waste) and the tick arm's
ceiling cap at 9 (constant-9 branch vs h+2 branch). The overfed answer packs
the waste through a mod compound `(% (+ w (- a h)) 10)` that also appears
(unreduced) in the rebuild.

Differential: starting hunger 6 vs 3: n=10 satisfies feed#1 fully (420) and
overfeeds only #2; n=0 overfeeds BOTH (301: eats 3, wastes 1) — different
branch at every dispatch, waste trail 4 vs 7, and the tick cap fires on
neither (max hunger reached is 4) — cap branch deliberately dark here, the
9-cap is pinned by rfr1's threshold family instead.

Hand model: n=10 → 420040404024824; n=0 → 301021207027527 (base-1000).

Pass ×3 wasm + rust + rust-async on trunk 85bb67940.
