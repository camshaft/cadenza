# PR#785 review comment — subtree-calls-self ignores arg3/arg4; a self-call in a 3rd/4th arg is missed (self-recursion declines)

Mirrored from GitHub PR review comment (Copilot), id `3630537842`.
PR: https://github.com/camshaft/cadenza/pull/785 (batch-staging; fix belongs on trunk)
Location: `implementation/compiler-ml/src/parse-db.cdz:183` (`subtree-calls-self`, NApp arm)

## Comment (verbatim)

> `subtree-calls-self` only scans the NApp's first arg plus `arg2-of`, but ignores `arg3-of`/`arg4-of`.
> For 3–4 arg calls, a direct self-call nested in the 3rd/4th argument will be missed, causing
> `call-is-self-recursive` to return false and the def to be excluded from the def-env (so
> self-recursion may incorrectly decline at eval).

## Liaison verification (CONFIRMED on trunk — functional gap, + a sibling with the same hole)

`subtree-calls-self` (parse-db.cdz:172) NApp arm (lines ~176-183): after `subtree-calls-self-arg`
(arg1) it only chains `arg2-of(tree, id)`. There is NO `arg3-of` / `arg4-of` scan. Both helpers exist
(`arg3-of` at parse-db.cdz:262, `arg4-of` at :283), so 3-4 arg calls ARE representable. So a direct
self-call nested in the 3rd/4th arg of a call is not found → `call-is-self-recursive` (parse-db.cdz)
returns false → the def is excluded from the def-env → at eval a self-recursive def with such a call
DECLINES instead of running (e.g. `fac` written with a helper taking the recursive call in arg 3).

BROADER: the sibling walker `subtree-calls-name` (the TRANSITIVE one, used for the lower's
recursion cycle-guard, parse-db.cdz ~121) has the SAME hole — its NApp arm also only chains `arg2-of`
(line ~132), ignoring arg3/arg4. Copilot flagged only `subtree-calls-self`, but v-compiler-ml should
audit `subtree-calls-name` too: a missed transitive call there could either mis-classify recursion for
the cycle-guard (inline-hang risk) or under-detect. Same root cause: both walkers cap at arg2.

Fix: extend BOTH walkers' NApp arms to also scan `arg3-of` and `arg4-of` (chaining the fuel/`fl`
through each, as arg2 does). Add a compiler-ml conformance/@test: a directly-self-recursive def whose
self-call sits in a 3rd (and 4th) argument position must RUN (not decline). Owner: v-compiler-ml
(`compiler-ml/*`; Slice B1b-p3 self-recursion, commit `cf932f8b3`). Routed as a note flagged
FUNCTIONAL-GAP.
