# Map.swap across dispatch (2026-08-11)

Angle: Map.swap (the tuple-returning value-yielding insert) is pinned only in
pure position (05); as the ARM's state transition it was uncovered — the tuple
splits across the boundary (prior-value resumes out, new-map threads on).

GREEN x3, python-modeled first:
- ms1: swap IS the transition; prior crosses as the resume value (n / -9-miss /
  100 over three puts), next threads — 70200/200
- ms2: the arm reads BOTH the pre-swap state s and the post-swap next at the
  same key after swapping (persistence under the dispatch's own transition;
  an FBIP in-place swap on the shared s would corrupt the s-read) — 58035/58005

Pin candidates: 234 pool.
