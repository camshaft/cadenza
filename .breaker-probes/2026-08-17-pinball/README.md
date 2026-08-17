# pnb1 — pinball with bonus ladder (2026-08-17, tick 1687)

Attack: a BOUNDARY-CROSSING detector comparing div results of PRE and POST
values — `(> (/ (+ score (* v mult)) 50) (/ score 50))` — the same compound
`(+ score (* v mult))` in the test, both crossing answers, the plain answer's
mod-100, and every rebuild (x5); the crossing branch nests a cap test on mult
(3-leaf bumper). The nudge's tilt branch halves and resets two fields (dark
at 3 dispatches — the tilts-threshold pin lives in the 800-row).

Envelope: 4-dispatch draft scratch-declined (3-leaf + heavy compound at 4 —
tighter than lom1's 3-leaf pass at 4 because the compound here is x5 vs
lom1's x2; repetition and leaves interact). 3-dispatch passes.

Differential: starting mult 2 vs 1: n=10's FIRST bumper crosses 50 (40→...
20*2=40, no... 402 = plain 40 at mult 2; crossing on bumper #2: 40+30=70 →
730) vs n=0 never crossing (201, 801, 351 — score 35 at read). Reads 7031
vs 3511.

Sed-derive slip AGAIN (3rd): trim recomputed outputs wrong on first pass —
model re-run caught before gating. The rule held.

Hand model: n=10 → 40280173007031; n=0 → 20180135103511 (mixed base:
rows base-1000 + read base-100000).

Pass ×3 wasm + rust + rust-async on trunk cde130bab.
