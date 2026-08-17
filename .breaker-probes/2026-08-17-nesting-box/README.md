# bnb1 — nesting box season (2026-08-17, tick 1715)

Attack: CEILING division via the (+1)/2 idiom with the CO-PART derived by
subtraction — hatched = (eggs+1)/2, remaining = eggs - hatched — the ceil
compound appears x3 (answer, both rebuild fields via the embedded subtract;
cdr1's conservation family at ceil instead of 2/3). Fledge keeps a CONSTANT
remainder (all-but-one: fl += ch-1, ch = 1 — the tkb1 three-amounts family
where one amount is `1`).

Differential: clutch 3 vs 1: n=10 ceil-halves 3→2+1 (21), lays, halves 4→2+2
(22), fledges 3→... rows [21,43,22,31] read 213; n=0 halves 1→1+0 (10 — the
ceil edge where half of one is one), lays, halves 3→2+1 (21), fledges (21).
Reads 213 vs 112.

Hand model: n=10 → 210430220310213; n=0 → 100330210210112 (mixed base).

Ops: first id nst1 TAKEN (arm next-state trap, batch ~230s) — renamed bnb1.

Pass ×3 wasm + rust + rust-async on trunk 141665bdd.
