# hmg1 — Hamming-distance tracker with XOR-folding reference (2026-08-15, tick 1544)

(reference, last) 2-tuple: `cmp v` answers popcount(v XOR ref) via the
let-free recursive bits callee, remembering v; `lock` folds the last value
INTO the reference (ref ← last XOR ref) answering the old reference. The
seed reference (n+5) propagates through both locks — the distance rows
diverge after the first fold (1,2,·,1,2 vs 1,2,·,3,2) and both lock answers
carry the seed (15→3 vs 5→9 chains).

First draft's lock (ref ← last) erased the seed after one fold, collapsing
rows — the XOR-fold keeps the seed alive through the whole stream (a
persistence-of-seed design pattern worth reusing).

Branch-free arms, 6 dispatches. PASS ×3. **Pool.**
