# Float specials as op ARGUMENTS (2026-08-11)

Angle: fx5-7 pin specials through the STATE thread; the ARGUMENT direction
(NaN/inf crossing the dispatch boundary as op args) was unpinned.

GREEN x3:
- fa1: NaN + inf born at runtime in the BODY (a*a overflow, inf-inf), cross
  as op args; the arm's (= x x) and (= x Float64.nan) tests — 311/111

Semantics learned (the probe first pinned IEEE expectations, compiler said 311):
- Cadenza `=` is CANONICAL equality: (= NaN NaN) is TRUE (not IEEE false),
  and (= x Float64.nan) IDENTIFIES NaN. Consistent with the fx6 doc note
  ("canonical equality distinguishes"). The arm sees NaN self-equal AND
  nan-canonical -> 3 (not the IEEE 2).
- Float64 literals with no finite value ((/ 1.0 0.0) at const-fold) are
  rejected at compile time: "no value form yet" — specials must be BORN at
  runtime from a parameter.

Pin candidate: 236 pool.
