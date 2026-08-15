# tie1 — two-pointer merge of implicit streams (2026-08-15, tick 1531)

(ai, bi) index state over IMPLICIT arithmetic streams (a_k = 3k + seed-offset,
b_k = 4k+1): `take` binds both computed heads ONCE through match binders
(rps2 idiom), answers the smaller advancing that index; a TIE answers 50+value
advancing the a-side. Seed offset (n%4)+1: n=10 (offset 3) buries its tie
mid-stream (row 5: 59) while n=0 (offset 1) OPENS with one (row 1: 51).

Note: the first draft with the heads recomputed per-branch (3 uses each)
DECLINED ×3 — consistent with the tightened frontier; the match-binder
hoist saved it exactly as it did for rps2. The declining draft is not banked
separately (same shape family as known fences).

Complements the landed mrg1 (explicit lists, 2 handlers) with the computed-
stream face. PASS ×3. **Pool (7th trio seed).**
