# three-op-tuple-state — three ops of one effect each update a distinct tuple field

## pymo3 — adda(field a), addb(field b), rd(read both), interleaved over (a,b) state
Seed (n%3, 0). adda: ans a, thread (a+1,b). addb: ans b, thread (a,b+10). rd: ans a*100+b, no change.
Body 10000*adda + 1000*addb + 100*adda + 10*addb + rd (eval adda,addb,adda,addb,rd).
Model: n=10 seed(1,0): adda1->(2,0),addb0->(2,10),adda2->(3,10),addb10->(3,20),rd320 => 10620.
       n=0  seed(0,0): adda0->(1,0),addb0->(1,10),adda1->(2,10),addb10->(2,20),rd220 => 420.

## Verdict: PASS-WITNESS (compiles + correct)
Verified 10620 / 420 on wasm + rust + rust-async. Extends pymo1 (two-op scalar shared
state) to three ops over a STRUCTURED (tuple) state with per-field independent advance —
the fold threads a shared compound state across distinct ops correctly.
