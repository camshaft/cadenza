# string-state-grow — a heap String handler state that GROWS per dispatch

## pystr2 — String state, answer=scalar-len, next=concat(s,"x")
Seed "a"/"ab"/"abc" by n%3. Body 100*d1 + d2.
Model: n=10 seed "ab": len 2, then len("abx")=3 -> 203. n=0 seed "a": 1, then 2 -> 102.
PASS-WITNESS: verified 203/102 x3 (wasm+rust+rust-async). A heap String value threaded and
REBUILT (concat) across the resume seam, distinct from pyls1 (list-state) and pystr1 (earlier
string thread) — here the state monotonically grows and the answer reads its live length.
