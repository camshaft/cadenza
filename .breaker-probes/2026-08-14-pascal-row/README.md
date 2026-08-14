# pas1 — Pascal's triangle row through effect state (2026-08-14, tick 1487)

`next` rebuilds the row via a recursive pairwise-sum (pairs, let-free, seeded
acc (list 1), appends the trailing 1) answering the row total — successive
answers are powers of two (2,4,8,16), a strong self-check. `coef` reads the
binomial coefficient at a SEED-KEYED index ((n%3)+1: k=2 for n=10, k=1 for
n=0) mid-descent and after; an out-of-range read answers -1. Arm dual-uses
the rebuilt row via match binder (fence-safe).

Seeds diverge at both coef reads (row-4: 1 vs 4... wait, C(4,2)=6 vs C(4,1)=4
— rows: n=10 reads 1 then 6? No: reads at row-2 (1,2,1): k=2→1 / k=1→2; at
row-4 (1,4,6,4,1): k=2→6 / k=1→4). Packed: 2040108160599 / 2040208160399.

First draft read coef(n%5) — seed-invariant for {10,0}; re-keyed (n%3)+1.

PASS ×3 wasm. **Pool (with qrm1, lru1 — trio ready).**
