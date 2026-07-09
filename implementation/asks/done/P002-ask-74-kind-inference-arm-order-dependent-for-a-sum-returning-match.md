## 74. ✅ RETIRED — NOT a real blocker (2026-07-08). The decline was an artifact of RETURNING a bare `Hir` as the program result, which the actual pipeline never does.

**Resolution: retired after rigorous bisection. The rewrite's real pipeline has NO ask-74 problem.**

**What I originally saw.** `(resolve-program <runtime module Ast>)` declined "cannot infer runtime compound
result shape," and I hypothesized an arm-order / sum-result-kind inference gap (the sum sibling of the fixed
ask-14).

**Why that was wrong (verified 2026-07-08).** Bisected it properly and the hypothesis did not hold:
- Returning a runtime user-sum as the program result COMPILES for a simple producer
  (`(def (mkh n) (if (< n 1) (Hir.HError 0) (Hir.HInt n)))` returned as `main` → `(Hir.HInt 5)`).
- The resolve cluster — even with the EXACT real `resolve-program`/`find-main-body(-go)`/`resolve-main-body`
  bodies, `Ast.List` arm first, run on a hand-built module `Ast` — COMPILES standalone.
- Adding `lower` as a consumer COMPILES.
- **The decline appears ONLY when `main` RETURNS `resolve-program`'s `Hir` result directly** (a bare
  compound sum as the program's runtime result, in the full-module context). CONSUMING that `Hir` — matching
  it into a scalar, or (the real case) passing it through `lower` — COMPILES.

**The decisive check.** The actual pipeline never returns a bare `Hir`; it consumes resolve's result through
`lower → eval-mir → select → serialize → wrap-component` (all the way to bytes). That FULL chain, fed a
hand-built module `Ast` (bypassing only `decode`/ask-73), **compiles to the BYTE-IDENTICAL scalar component**
— verified for `(main) 42/7/0/300` → 89/89/89/90 bytes, byte-identical to native, runs to the right value.

**Conclusion.** There is no return-kind gap in the rewrite's resolve/lower path. The residual "cannot infer
runtime compound result shape" only bites a program that *returns a runtime-built compound sum as its
top-level result via this particular call shape* — which the compiler does not do, and which is anyway the
same known family as the existing corpus todo "a function returning None or a nested Some infers its compound
result shape" (not a new gap). The false minimal corpus case I had added was already removed.

**So the rewrite's front rung has exactly ONE real blocker: ask-73** (the tail-recursive tuple return in
`decode`, upstream of resolve). Once ask-73 lands, `decode → resolve → … → component` is complete
(everything downstream of `decode` is verified byte-identical). Retiring this ask; moving to `done/`.
