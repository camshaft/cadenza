# PR #2245 review — rcdzc/src/tests.rs (v-effects) — OPEN — comment-correctness [VERIFIED, LOW] (fold of my #2243)

https://github.com/camshaft/cadenza/pull/2245 (tighten the ms-family pin test-precision — the fold-forward
for my #2243 review). Copilot 1 inline on the recursion-reasoning comment.

## the comment says `(loop 1)` returns `1`, but with multi-shot `flip` resuming 2 and 3, `(loop 1)` = `2+3 = 5` — the arithmetic reasoning is wrong (Copilot, tests.rs:68179) — comment-correctness [VERIFIED, LOW]
> The updated explanation incorrectly states that `(loop 1)` returns `1`. With `loop` defined as
> `(* (Amb.flip) (loop (- n 1)))` and a multi-shot `flip` that resumes with 2 and 3, `(loop 1)` evaluates
> to `2+3 = 5` (and `loop 2` still totals 25). Please adjust the comment so the reasoning matches.

VERIFIED the arithmetic. `loop = (if (= n 0) 1 (* (Amb.flip) (loop (- n 1))))` (diff:29), handler `flip =
(+ (resume 2 s) (resume 3 s))` (multi-shot: continuation runs with 2, then 3, summed). So:
- `(loop 0)` = 1.
- `(loop 1)` = `(* (Amb.flip) (loop 0))` = `(* flip 1)` → the continuation `(* □ 1)` runs once resumed with
  2 (=2) and once with 3 (=3), handler sums = **2+3 = 5**. NOT 1.
So the comment (diff:26-27) "`(loop 1)` returns 1, so `(* flip1 (* flip2 1))` = `(* flip1 flip2)` = the
identical 4-path cross-product" is WRONG on the premise: `(loop 1)` is 5, not 1. LOW/comment-correctness
(the pin's ASSERTED value + the run_returns/uncoded-Err structure from my #2243 review are fine — this is
only the explanatory comment's arithmetic). Fix per Copilot: state `(loop 1) = 5` (2+3). NOTE on the
final total: Copilot says "loop 2 still totals 25" — plausible, but whether `(loop 2)` under NESTED
multi-shot resumption equals exactly 25 is subtle effects semantics (the outer flip's continuation now
wraps a value that ITSELF summed two resumes); v-effects should confirm the final number the comment
should state, rather than me asserting it. The pin's `Ok == "25"` assert (from my #2243 fold) already
encodes the expected result, so if the comment's corrected reasoning doesn't reach 25, that's worth a
double-check of the assert too. v-effects owns rcdzc effects. PR OPEN → foldable. (Comment-only on the fix
for my #2243 — the reasoning just needs to match the program's actual multi-shot arithmetic.)
