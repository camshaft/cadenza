# hnc1 — Hanoi smallest-disk cycle (2026-08-15, tick 1569)

(peg, moves) state: the small disk moves on every ODD move, cycling pegs in
the seed-picked direction (+1 vs +2 mod 3), answering peg*10+1; even moves
answer the small disk's resting peg (the classic Hanoi observation that the
smallest disk moves every other turn in a fixed rotation). count reads.

The two directions trace MIRROR peg sequences (11,·,21,·,1 vs 21,·,11,·,1)
that RE-CONVERGE at the mid-cycle zero rows (peg 0 at moves 5-6 both) then
diverge again — mirror symmetry as the seed differential, with the (% n 3)
compound recomputed per odd-branch (cheap, in-envelope).

PASS ×3. **Pool (with rrl1; +1 fills the 10th trio... pool recount: 9 trios
+ rrl1/hnc1).**
