# tkb1 — ticket booth with group comp (2026-08-17, tick 1698)

Attack: an ASYMMETRIC DEDUCT/CREDIT split — the group branch deducts strips
in FULL (k) while crediting sold with k−1 and comps with 1 (three fields
moved by three DIFFERENT amounts derived from one argument); the small-party
branch moves two fields by the same k. The affordability guard is checked
BEFORE the size split (guard-then-classify vs cwx1's classify-then-guard).

Differential: opening till 10 vs 6: the mid-run trio is SERVED on n=10 (30)
but refused on n=0 (901 — the group of 5 drained the till to 1); after the
common restock the final group lands identically (31) but the reads split
(302 vs 172: strips 3 vs 1, sold 10 vs 7... n=10 read 302 = strips 3, sold
0? — trust the model: 302 / 172).

Hand model: n=10 → 410300600310302; n=0 → 419010500310172 (mixed base).

Pass ×3 wasm + rust + rust-async on trunk c6fa89785.
