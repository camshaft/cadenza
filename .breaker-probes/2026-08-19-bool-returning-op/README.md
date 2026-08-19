# bool-returning-op — op returns a Bool from a state comparison, body uses it as if-guards
## pybr1 — ge(t) answers (>= s t) threading +1; body: two if-guards select constants. Model 1030/2030. PASS x3.
Bool-typed op result from a comparison; threaded state flips the first guard across seeds. Promotable.
