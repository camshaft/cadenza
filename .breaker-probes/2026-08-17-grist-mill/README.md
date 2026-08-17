# mil1 — grist mill riding the wind (2026-08-17, tick 1699)

Attack: a 3-band classifier where each band moves a DIFFERENT field with a
DIFFERENT transform of the argument (full g / half g/2 / spoil-all g) and
the fast band ALSO drags the classifying field (rpm -= 2 — the classifier
input mutated by the classified branch, saw1's pre/post family applied to a
band selector). The spoil answer's mod-100 reads the post-accumulation value
`(% (+ sp g) 100)` matching the rebuild.

Differential: starting wheel 6 vs 2: n=10 grinds ideal (41), gusts past the
band (93), SPOILS (906 — drag to 7), then ideal again (51); n=0 grinds
coarse (22), gusts INTO the band (53), ideal twice (61, 51). No two rows
agree except the final ideal; reads 976 vs 1350.

Hand model: n=10 → 410939060510976; n=0 → 220530610511350 (mixed base).

Pass ×3 wasm + rust + rust-async on trunk c6fa89785.
