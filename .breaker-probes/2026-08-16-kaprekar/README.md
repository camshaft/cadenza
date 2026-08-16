# ksc — Kaprekar walk: THIRD compile hang, simplest yet (2026-08-16, tick 1584)

Kaprekar step for 3-digit numbers via the 99*(max_digit − min_digit)
identity. kap is a def whose body multiplies the difference of two nested-if
callees (dmax/dmin, each 2-level, each taking the same three digit-extraction
compounds (/ v 100), (% (/ v 10) 10), (% v 10)).

| variant | kap calls in arm | verdict |
|---------|------------------|---------|
| binder (match (kap v) (v2 …)) | 1 via binder | DECLINE (binder-over-call w/ 2 consumers… decline not hang) |
| inline ×2 (answer + state) | 2 | **HANG** |
| single call, state-only | 1 | **HANG** |

THE SIMPLEST HANG WITNESS YET: ONE call site, a 2-branch arm, 2-tuple state,
8 dispatches. The explosive object is the CALLEE ITSELF: kap's body feeds
three shared digit-compounds into two nested-if defs inside an arithmetic
frame — the partial-eval walk apparently re-expands the digit compounds
through the dmax/dmin branches multiplicatively. cmb1 (5-way in-arm
recompute) and pom5 (state-field equality cascade) needed arm-level
structure; ksc1 hangs from a def-body shape alone, reachable by ONE ordinary
call.

Also: the binder form DECLINING (not hanging) while the direct call HANGS
inverts the usual severity — the workaround shape is WORSE-behaved here.
