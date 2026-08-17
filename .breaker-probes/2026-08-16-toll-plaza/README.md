# tol1 — toll plaza with exact-change lane (2026-08-16, tick 1664)

Attack: a 4-leaf arm (exact / change-covered / delayed / underpay) where the
change compound `(- amt 5)` appears in the affordability TEST and the taken
ANSWER but NOT the rebuild (the till keeps the flat toll `(+ till 5)` — the
answer/rebuild use DIFFERENT compounds from the same inputs, inverse of the
usual shared-compound shape). Delayed resumes with only dl bumped; underpay
resumes st untouched.

Differential: float 3 vs 0: n=10 serves every car ([21,101,91,61] read 2310);
n=0 delays ALL THREE overpayers ([901,101,902,903] read 513 — the exact-lane
five is its only successful transaction). Every row differs.

Hand model: n=10 → 211010910612310; n=0 → 9011019029030513 (mixed base;
first-draft read rows overflowed base-1000, repacked at 10000; second draft
also widened the change amounts so the tills diverge past row 1).

Pass ×3 wasm + rust + rust-async on trunk 7b8cc9162.

## Context note (same tick): v-core-opt CONFIRMED my O2 refutation — their
cmb1-flips claim was stale (pre-P1/P3 instrumentation); on the current gate
ALL 127 cmb1 shares are P3-excluded → empty plan → hang stands. cmb1-FULL is
now a P3-refinement question with v-rb (Arith-feeding-match is likely a P3
false-positive vs the Sum-constructor case P3 was built for).
