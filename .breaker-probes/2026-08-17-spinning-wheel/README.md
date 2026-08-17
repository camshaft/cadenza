# spn1 — spinning wheel with div/mod split (2026-08-17, tick 1702)

Attack: the div/mod PAIR over one field with the results SPLIT across slots —
quotient `(/ tw 2)` goes to the answer AND the yarn accumulation; remainder
`(% tw 2)` goes to the answer's tag AND becomes the threaded twist (kln1's
divmod-fusion target but with the pair DISTRIBUTED across all four slots
rather than co-located in one answer). Tangle zeroes the divided field.

Differential: twist 5 vs 2: n=10 opens with a clean draft (21 — quotient 2,
remainder 1 kept) and ends tangling once (901); n=0 opens tangling (901) and
ends tangling twice (902) — reads 501 vs 202 (yarn 5 vs 2, tangles 1 vs 2).

Hand model: n=10 → 210650309010501; n=0 → 9010550219020202 (mixed base;
first seed spread 1-or-3 made both wheels tangle identically — re-keyed to
2-or-5).

Pass ×3 wasm + rust + rust-async on trunk 86ae0a4bc.
