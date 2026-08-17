# saw1 — sawmill with dulling blade (2026-08-17, tick 1678)

Attack: the yield compound `(- ln (/ blade 2))` reads the PRE-cut wear while
the rebuild stores POST-cut wear `(+ blade 2)` and the answer's tag reads the
post value `(% (+ blade 2) 10)` — pre and post values of the same field in
one branch (a stale-read-vs-fresh-read ordering pin). Jam resets the field
the yield divides by. 2-branch arm, 4 dispatches, yield compound x2 (answer
+ planks accumulation).

Differential: pre-dulled blade (3 vs 0) reaches the jam threshold after two
cuts (n=10: jam mid-run at cut #3, 901) vs after three (n=0: jam on the LAST
cut) — every yield shifts by the wear offset and the reads differ (921 vs
1201: planks 9 vs 12, blade 2 vs 0 post-jam).

Hand model: n=10 → 450279010320921; n=0 → 520340469011201 (mixed base:
rows base-1000 + read base-10000).

Pass ×3 wasm + rust + rust-async on trunk 0db236a9d.
