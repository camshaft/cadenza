# mtr1 — metronome with seed-keyed time signature (2026-08-16, tick 1636)

Attack: the WRAP THRESHOLD itself is seed-derived — `(+ 3 (% n 3))` appears in
the branch CONDITION and again inside the taken answer (the downbeat row is
tagged with the signature), so the same compound gates control flow AND
flows into the value. Off-beat rebuild advances one field; downbeat rebuild
resets it while bumping two others (asymmetric 1-vs-3 field touch).

Differential: signatures 4 vs 3 land the downbeats at ticks {4} vs {3} within
five ticks — every row after the first two differs, and the report packs a
different live beat (1 vs 2) with equal accents... n=0 gets TWO bars started
(bar 1 at tick 3) vs n=10's one: rows [10,20,193,...] vs [10,20,30,194,...].

Hand model: n=10 → 10020030194110111; n=0 → 10020193110120112 (base-1000).

Pass ×3 wasm + rust + rust-async on trunk 4c75635d9. (First theme candidate
— tide triangle-wave — SKIPPED: overlaps my own staged tid1; coverage-check
against the STAGED POOL, not just landed corpus.)
