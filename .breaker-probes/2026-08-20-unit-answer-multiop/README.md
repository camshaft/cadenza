# unit-answer-multiop — a Unit-answering advance op beside a value-answering reader

## pymu1 — bump (-> Unit Unit) answers unit + advances (+ s 3); peek (-> Int64) reads s*100
Interleaved peek,bump,peek,bump,peek via let/do. Body 1000*x + y + peek3.
Model: n=10 s0=1: x=peek 100, bump s->4, y=peek 400, bump s->7, peek3 700 => 101100.
       n=0  s0=0: x=0, bump s->3, y=300, bump s->6, peek3 600 => 900.

## Verdict: PASS-WITNESS (compiles + correct)
Verified 101100 / 900 on wasm + rust + rust-async. Adds a UNIT resume-answer (bump, which
only advances state) alongside a value-answer (peek) in one shared-state tail fold — the
Unit answer carries no data yet the state thread must still advance. Complements pymo1/pymo3
(all-value-answer multi-op) with a mixed Unit/value answer set.
