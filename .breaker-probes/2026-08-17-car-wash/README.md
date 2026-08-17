# cwx1 — car wash with wax upsell (2026-08-17, tick 1682)

Attack: a CROSS-FIELD ORDERING GATE — the wax admits only when `washed >
waxed` (two counters in strict-order relation, incremented by DIFFERENT ops),
so the wax outcome depends on the interleaving history, not any single field.
Both refusal branches resume st untouched with field-echoing answers (900 +
dregs / 800 + waxed). The wash's decrement-by-3 vs the seed float is the
scheduler.

Differential: soap 7 vs 4: n=10 washes twice then fails wash #3 (901 at soap
1); n=0 fails wash #2 immediately — so the SECOND wax call finds washed(2) >
waxed(1) on n=10 (served, 27) but washed(1) == waxed(1) on n=0 (REFUSED,
801). Same call, opposite gate outcomes, purely from counter history.

Hand model: n=10 → 41011015901027122; n=0 → 11901015901801111 (base-1000).

Pass ×3 wasm + rust + rust-async on trunk 6789dc56e.
