# A None arm and a nested-Some arm do not unify at the compound-result boundary

*2026-07-08*

**What happened.** Adversarial stress-probing of the just-realized c64 tuple-of-recursive-results
lowering surfaced an adjacent gap in payload-kind arm-unification. A function whose branches return
`(None unit)` and `(Some (Some n))` — an `Option (Option Int64)` — is declined "cannot infer runtime
compound result shape" when the value is returned as the program result. The single-level analogue works:
a function returning `(None unit)` vs `(Some n)` yields `(Some 5)` (its arms' kinds unify at the result
boundary). And a producer whose BOTH arms are `(Some (Some …))` works, yielding `(Some (Some 5))`
(consistent kind). Only a `None` arm paired with a NESTED-`Some` arm is not unified. The value is
unambiguous — consuming the same producer with a nested match `((Some (Some x)) …)` reconstructs `(Some
(Some 5))` — so the program is valid; the compound-result-shape inference just does not recover the
nested-vs-nullary arm kind.

**Why it is a break (an honest decline of a valid program).** `Option (Option Int64)` is a well-formed
type; `(if (< n 0) (None unit) (Some (Some n)))` is a well-typed expression of it whose value is `(Some
(Some 5))` for n=5 (proven by the consuming-match reconstruction). Returning it across the run boundary
must produce that value. Declining "cannot infer runtime compound result shape" is decline-don't-
miscompile-safe (no wrong value), but it rejects a valid program that a self-hosted compiler naturally
writes (a pass returning `Option (Option _)` — e.g. an optional lookup that itself yields an optional).

**Root cause (likely) — the branch arm-kind unification that recovers a payload kind handles a None arm
against a single-level Some arm but not against a nested Some arm.** The seed recovers a sum value's
payload kind by unifying the kinds its match/if arms produce (the machinery in
`[[sum-match-payload-kind-recovered-by-arm-unification]]` and
`[[recursive-bool-return-branch-order-inference]]`). For `None` (payload kind Unit) vs `(Some n)`
(payload kind Int64 scalar) the unification succeeds — the result is a single-level Option whose payload
kind is recovered. For `None` vs `(Some (Some n))` the `Some` arm's payload is itself a compound
(`Option Int64`, a heap sum), and the unification against the `None` arm's Unit payload does not resolve
the nested compound kind, so the compound RESULT shape cannot be inferred. The fix is to unify a None arm
with a nested-Some arm by taking the nested arm's (compound) payload kind as the result's `Some` payload
kind — the nested-payload extension of the single-level unification that already works.

**The lesson (immediately stress-probe a newly-realized capability at its boundary).** The c60→c64
lowering (tuple of recursive/sibling-call results matched with constructor patterns) was realized over
the previous two cycles; probing its boundary immediately surfaced this adjacent arm-unification gap one
level of nesting deeper. A fix's boundary is exactly where the next sibling gap sits — here, the
arm-kind unification recovers a single-level payload but not a nested one. The tell: `None`/`(Some n)`
returns fine, `(Some (Some …))`/`(Some (Some …))` returns fine, but `None`/`(Some (Some n))` — the
mixed nullary-vs-nested pair — declines.

**Corpus case added.** `spec/semantics/05-compound-types.sexp` §"a function returning None or a nested
Some infers its compound result shape" — `(cl 5)` for `(def (cl n) (if (< n 0) (None unit) (Some (Some
n))))` expects `(Some (Some 5))`. Gated `(needs fallible-access)`, which the seed realizes, so it runs;
the seed currently DECLINES ("cannot infer runtime compound result shape"), so the case classifies `todo`
(gate stays GREEN). It will PASS when the None-vs-nested-Some arm-kind unification lands. A generation
that does not yet unify a None arm with a nested-Some arm at the result boundary declines rather than
miscompiling.
