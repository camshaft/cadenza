# hiv1 — beehive with swarm pressure (2026-08-17, tick 1673)

Attack: TWO divisions with different roles in one protocol — the skeleton-crew
forage banks `(/ y 2)` (argument halving, shown in the answer AND added to
honey), and the swarm hatch halves the COLONY `(/ (+ bees k) 2)` (the halved
compound in both the answer and the rebuild). The forage tier test reads
`bees` — a field the OTHER op halves — so cross-op tier flips ride the swarm
timing. 3-tier forage (full/half/empty) x 2-branch hatch.

Differential: colony 4 vs 2: n=10 swarms on hatch #1 (703 → 3 bees) so
forage #2 stays FULL-tier (141); n=0 builds to 5, swarms on hatch #2 —
forage #2 runs half-tier... rows [32,50,111,703] — n=0's forage #1 is
half-tier (32 = 3-banked... (/ 6 2)=3 →32). Tiers flip between runs; read
1451 vs 1131.

Hand model: n=10 → 617031410501451; n=0 → 320501117031131 (mixed base).

Pass ×3 wasm + rust + rust-async on trunk a94789c60 (B2 unit-test coverage
for the O2 gate — the tier question is getting test scaffolding).
