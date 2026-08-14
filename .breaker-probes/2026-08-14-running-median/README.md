# med — running median + the callee-let-list-liveness decline (2026-08-14, tick 1480)

med1 (running median over sorted-insert list) declined ×3 — investigation
produced an 11-probe shrink ladder isolating a NEW minimal decline shape,
SIMPLER than lstM and unrelated to mixed ops:

**A def with a List parameter that uses the list AGAIN after an internal
`let`, called from a handler arm, declines at ≥2 dispatches (single op).**

| probe | callee shape | verdict |
|-------|--------------|---------|
| medD | scalar-arg def w/ internal let | PASS |
| medI | List-arg def, let binds element, list DEAD after let | PASS |
| medJ | List-arg def, let binds List.len, list DEAD after let | PASS |
| medF | List-arg def, list LIVE across let, ONE dispatch | PASS |
| medG/medK | list LIVE across let, TWO dispatches | DECLINE |
| medH | same, arm itself let-free | DECLINE |
| medE | same at three dispatches | DECLINE ×3 backends |
| medB/medC | median via recursion or inlined len (no callee let) | PASS |

medK is the minimal witness (7-line callee `lastof` = let m = len; getat at
m-1). All declines uniform wasm/rust/rust-async. NOT a miscompile — a clean
frontier decline. Note: every PASSING pool probe's helper defs (lastv, dropl,
rev, maxs) are let-free — retroactively explains why the pool never hit this.
med1 held back from corpus until this flips; medK on the flip-watch.
