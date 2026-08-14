# hld1 — inventory hold/settle/release protocol (2026-08-14, tick 1469)

3-op handler over (on-hand, held): `hold` reserves against AVAILABLE
(on-hand minus already-held), answering the running held total or the negated
available on reject; `settle` deducts held from on-hand; `release` drops holds
without deducting. Seeds reject DIFFERENT holds: n=10 accepts 4+9 (avail 18)
and rejects the final 6 after settling down to 5 → 4130503049505; n=0 rejects
the 9 immediately (avail 4) → 3960403039604.

First draft had on-hand = n, collapsing n=0 to all-zero rows — reseeded to
n+8 per the weak-pin rule.

PASS ×3 wasm. THREE distinct ops performed × 7 dispatches × scalar-tuple state,
branch-only arms — cliff-consistent (no List-valued dual-use let). **Pool.**
