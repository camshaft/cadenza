# bwl1 — spare-chain scorer (2026-08-15, tick 1522)

(prev2, prev1, score) 3-tuple threading a TWO-ROLL history: each roll adds
its pins plus a DOUBLE when the previous two summed to 10 (the bowling spare
rule); a -1 sentinel guards the opening frame. total reads the score.

Only n=10's opening pair (7+3) hits ten, so its third roll doubles (5→10)
and the scores drift from row 3 (7,10,20,26,30 vs 3,6,11,17,21). First
model draft had NO spare firing on either seed (7+4=11 / 3+4=7) — re-rolled
the second pin to 3 per the weak-pin rule.

3-branch arm, 6 dispatches, cheap recomputes — envelope-safe. PASS ×3.
**Pool — fills knt1/lck1/bwl1 (fifth trio ready).**
