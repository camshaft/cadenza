# result-state — a (Result Int64 Int64) sum value in the HANDLER STATE slot, threaded

## pyrs1 — Result-typed state, Err recovers to Ok mid-thread
Answer Ok v->v*10 / Err e->-e; next Ok v->Ok(v+1) / Err e->Ok e; seed Err(7) if n%3=0 else Ok(n%3).
Model: n=10 seed Ok(1): 10,20,30 -> 12030. n=0 seed Err(7): d1=-7 (recovers Ok 7), d2=70, d3=80 -> 80.
PASS-WITNESS: verified 12030/80 x3 (wasm+rust+rust-async). Companion to pyos1 (Option-state) and
pyres1 (Result-ANSWER) — this is the Result value in the STATE slot with an arm-crossing recovery.
