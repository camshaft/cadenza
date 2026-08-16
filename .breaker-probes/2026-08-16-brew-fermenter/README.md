# brw1 — brew fermenter with stuck rescue (2026-08-16, tick 1657)

Attack: the inline-min compound `(if (< y (- g 10)) y (- g 10))` appears FIVE
times across the day arm — in the stuck test, both branches' answers, and
both branches' rebuilds — the heaviest single-compound repetition banked at a
passing envelope (2 branches, 4 dispatches; snk1's x4 held at 6 dispatches,
this x5 at 4). The stuck branch mutates all 3 fields; the healthy branch
mutates 1.

Differential: weak yeast (2) sticks on DAY ONE (758 alarm, rescue to 6);
strong (5) ferments clean to the floor... n=10 rows [555,505,60,446] read
4460 (no rescue) vs n=0 [758,526,70,457] read 4571 (one rescue) — every row
differs.

Hand model: n=10 → 5550505006004464460; n=0 → 7580526007004574571
(base-10000).

Pass ×3 wasm + rust + rust-async on trunk bc7437703.
