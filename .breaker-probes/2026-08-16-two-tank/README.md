# tnk — two-tank siphon: a 3-CONSUMER binder decline (2026-08-16, tick 1582)

Siphon moves (a−b)/4 from A to B; pour tops A; levels reads the gap. Seed
sets B (20 vs 0) — same siphon sequence converges through different
geometric ladders (5,2,…,4,2,1 vs 10,5,…,5,3,1) to the same final gap (4).

| face | siphon arm shape | verdict |
|------|------------------|---------|
| tnk1 (2-branch, binder/branch) | guard a<b + binder per branch | DECLINE ×3 |
| tnk1 (branch-free, binder) | binder d feeds answer + BOTH tuple fields | DECLINE ×3 |
| tnkB (branch-free, inline ×3) | compound inlined at all three sites | **PASS ×3** |

New fence datapoint: a match binder over arithmetic whose bound value feeds
THREE consumers (answer + two state fields) declines, while rps2/tie1/mnc1
binders (≤2 consumers) pass — and the INLINE-thrice form compiles. Inverse
of the rps1 lesson (there inlining exploded and the binder saved it; here
the binder declines and inlining saves it). The binder-consumer COUNT is a
frontier axis. Family-banked; not separately routed (fold-owner has the
family context).

tnkB is corpus-eligible. Flip-watch: tnk1-binder-declines.
