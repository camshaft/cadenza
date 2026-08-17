# rnk1 — ice rink with wear feedback (2026-08-17, tick 1680)

Attack: a FEEDBACK multiplier — the skate arm's wear rate reads the field it
degrades (quality < 5 doubles the wear: `(* k 2)` vs `k`), with a floor
branch pair under each rate (4 leaves); the zamboni resurfaces +6 CAPPED at
10 (both cap branches answer identically, diverging only in the rebuild —
the rcy1 same-answer twin at a 2-leaf scale) and clears skaters while
echoing the pre-clear headcount.

Differential: starting ice 9 vs 4: n=10 wears slow (rate 1) and resurfaces
INTO the cap (12→10); n=0 fires the fast-wear feedback immediately (4<5:
3*2=6 → floor... 4-6 → 0? no: 4≥... q=4 <5 → rate 2, 4-6 → floored 3? model:
q=4-3*2=-2→0? rows show 3 — 4-3*... n=0 row1 = 3: (4 - 3*... rate check
BEFORE wear: q=4 <5 → double: 4-6 <0 → floor 0 branch answers sk%10=3.
Read 241 vs 641.

4-dispatch draft scratch-declined (4-leaf skate + 2-leaf zamboni; the sil
arm-sum law); 3-dispatch passes.

Hand model: n=10 → 63713064641; n=0 → 3713024241 (base-1000).

Pass ×3 wasm + rust + rust-async on trunk 6789dc56e.
