# option-state — an (Option Int64) sum value in the HANDLER STATE slot, threaded

## pyos1 — Option-typed state threaded across 3 dispatches
```
(tick () s (resume (match s ((Some v) (* v 10)) ((None) -1))
                   (match s ((Some v) (Some (+ v 1))) ((None) (Some 0)))))
```
Seed None (n%3=0) else Some(n%3). Deep handler, body 1000*d1 + 100*d2 + d3.

## Verdict: PASS-WITNESS (compiles + correct)
Model: n=10 seed Some(1): d1=10,d2=20,d3=30 -> 12030. n=0 seed None: d1=-1,d2=0,d3=10 -> -990.
Verified 12030 / -990 on wasm + rust + rust-async (fresh worktree-local cdz).

## Contrast with pyu8w1 (the narrow-int decline)
pyu8w1 showed a UInt8 (narrow-int) handler state threaded across >=2 dispatches DECLINES
(width-alias #16-23 class). pyos1 shows a SUM-TYPE (Option) state slot threads FINE across
3 dispatches — so the fold's state-slot gap is SPECIFIC to narrow-int width, NOT to
non-scalar/boxed state layouts in general. Pins that boundary.
