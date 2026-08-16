# chr1 — chairlift with two-seat clamp (2026-08-16, tick 1647)

Attack: an inline MIN as an if-branch pair where the two branches encode the
clamp ASYMMETRICALLY — the full branch uses the constant (200 tag, w-2), the
partial/empty branch uses the variable (w*100, queue zeroed, l+w) — so a
"simplify both branches to min(w,2)" rewrite must keep four distinct slots
straight. The empty chair still counts (c+1 in both branches, the only
symmetric field).

Differential: seed sizes the first group (3 vs 2): n=10 splits it across a
partial chair (row 3 = 102: one rider, chair 3); n=0 rides out on chair 1 and
sends chair 2 up EMPTY (row 3 = 2: zero take, chair-count echo). Rows 1-3 and
the tally (403 vs 303) diverge.

Hand model: n=10 → 33211102011103403; n=0 → 22201002011103303 (base-1000).

Pass ×3 wasm + rust + rust-async on trunk d3086251e.
