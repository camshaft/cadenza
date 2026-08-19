# closure-out-of-handle — handled body returns a closure capturing a draw, applied after teardown
## pyco1 — (fn (x) (+ x d)) with d = a drawn value, applied (f 100) OUTSIDE the handle. Model 101/100. PASS x3.
The captured draw survives handler teardown and is usable outside the effect region. Promotable.
