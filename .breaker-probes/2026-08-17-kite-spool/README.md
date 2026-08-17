# kit1 — kite on a spool (2026-08-17, tick 1701)

Attack: a CROSS-FIELD CEILING where both sides of the comparison MOVE in the
same dispatch — payout raises alt by 2k while the ceiling itself rises by k
(`(> (+ alt (* k 2)) (+ line k))` — the guard compares two updated compounds,
not a value against a standing bound). The snap branch halves line AND sets
alt to the same halved compound (two fields converging to one value); taut
sets alt = line (field-to-field copy).

Differential: spool 8 vs 4: both kites SNAP on the same gust-6 (901 each) but
against different lines (11 vs 7 → stubs 5 vs 3), so the final gentle gust
pulls taut against DIFFERENT stubs (805 vs 803) — same branch, different
data, and the reads carry the halving apart (551 vs 331).

Hand model: n=10 → 330919018050551; n=0 → 330779018030331 (mixed base;
two earlier tunings had both-snap-identical rows — the differential lives in
the post-snap stubs, not the snap itself).

Pass ×3 wasm + rust + rust-async on trunk 86ae0a4bc.
