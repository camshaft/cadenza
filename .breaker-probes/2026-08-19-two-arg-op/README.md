# two-arg-op — a two-argument op, arm uses both args + state in the resume
## py2a1 — combine(a,b): answer = a*s+b, both args + state, two dispatches. Model 811/509. PASS x3.
Confirms a two-argument op's arm receives both args (a,b) and the captured state, all in scope
at the resume. Promotable pass-witness.
