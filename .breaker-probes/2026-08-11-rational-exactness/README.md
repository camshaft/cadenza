# Rational exactness across dispatch (2026-08-11)

Angle: rq1-3 pin Rational states with accessor round-trips; the EXACTNESS
verdicts (reduce landing exactly on a target across dispatches) were
uncovered — a float-contaminated state thread would miss unity.

GREEN x3:
- rx1: thirds accumulate; (* s 3) == 1 verdict true only at the seed — 1
- rx2: cross-denominator (1/2 + 1/3 + 1/6) lands EXACTLY at unity on the
  third dispatch (0,0,1 -> 100); the reduce/normalize happens per hop

Vocab: Rational.of takes TWO args (num denom); built-ins reject partial
application ("must be applied to exactly its arguments").

Pin candidates: 245 pool.
