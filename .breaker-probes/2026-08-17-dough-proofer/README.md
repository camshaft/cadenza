# dgh1 — dough proofer with knock-backs (2026-08-17, tick 1704)

Attack: NESTED division in a shared compound — the risen volume
`(+ vol (/ (* t glu) 2))` is itself divided by 3 in the knock-back
(`(/ (+ vol (/ (* t glu) 2)) 3)` — a div-of-sum-of-div), with the outer
division in the answer's mod AND the rebuild. The knock-back also STRENGTHENS
the multiplier (glu+1) — the deflate-and-strengthen pair mutating both the
compound's inputs for the next round (mil1's dragged-classifier family, two
fields at once).

Envelope: 4-dispatch draft scratch-declined (nested-div compound x4); 3
passes.

Differential: gluten 3 vs 2: n=10 knocks back on proof #2 (714 — deflate to
4, strengthen to 4) then rises 6 (60); n=0 rises clean (37, 41) to the brink
and knocks on proof #3 (714). Reads 1041 vs 431 (vol 10 vs 4, glu 4 vs 3).

Hand model: n=10 → 487140601041; n=0 → 370417140431 (mixed base).

Pass ×3 wasm + rust + rust-async on trunk 86ae0a4bc.
