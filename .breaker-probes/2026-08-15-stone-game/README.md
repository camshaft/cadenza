# stg1 — stone-taking game (2026-08-15, tick 1535)

(pile, turn) state: `take k` clamps at the pile answering pile*10 + whose
turn it was; landing the pile at zero answers 100+id; turn alternates. The
smaller pile (11) drains three moves early, so its tail is repeated
drained-pile "wins" for ALTERNATING players (101,100,101) while the larger
(21) is still counting down and lands its single win on the last move.

Interesting decline note: the first draft's arm had a redundant 3-branch
nest whose two outer branches were IDENTICAL expressions — it DECLINED ×3;
collapsing to the equivalent 2-branch form compiles. Identical-branch
duplication seems to trip the same frontier as real branches (the duplicate
is not collapsed before the fold cost check). Not banked separately — noted
here; the shape is degenerate (no natural program writes identical branches).

PASS ×3. **Pool — fills tie1/prk1/stg1 (seventh trio ready).**
