# A computed-key map literal mis-dispatches its lookup result in a match

*2026-07-08*

**What happened.** A map built by a `(map …)` literal with a RUN-TIME-computed key is mis-represented:
`Map.lookup` on it returns the correct `(Some v)` when rendered directly, but MATCHING that result
dispatches to the `None` arm — a wrong-arm miscompile. `(let ((j (+ 2 3))) (match (Map.lookup (map (j 1))
5) ((Some v) v) ((None _) -1)))` yields **-1**, though key 5 (= `(+ 2 3)`) is present with value 1, so the
`(Some v)` arm should bind v=1 and yield 1. The lookup alone renders `(Some 1)` (the value is right); only
the match reads the wrong variant. The same map with a CONST key (`(let ((j 5)) (map (j 1)))`) matches
correctly (→1), and a `Map.insert`-built map (even with a computed key) matches correctly (→1). Only the
computed-key `(map …)` LITERAL is broken.

**Why it is a break.** core-semantics.md #Matching Is Exhaustive Or Rejected: a match evaluates the branch
of the first pattern that matches the scrutinee. The scrutinee is `(Some 1)` (proven by rendering the
lookup directly), so the `(Some v)` arm matches and the result is 1. Taking the `None` arm is a wrong
value — the Option's variant tag is misread.

**Root cause (the under-realized computed-key map literal).** A `(map …)` literal with all-constant
entries is const-folded to a map value; a literal with a computed key must be built as a runtime heap map
(the persistent-map handle). The literal path with a runtime key builds a map that does NOT behave as a
proper runtime map: `Map.lookup` on it returns an Option whose variant tag the match dispatch misreads
(it takes None for a Some). A `Map.insert`-built runtime map does not have this problem — its lookup
Option matches correctly — so the defect is specifically the computed-key `(map …)` literal's
construction, which produces a mis-tagged map (or a map whose lookup emits a mis-tagged Option). The fix
is to build a computed-key `(map …)` literal as the same runtime persistent map a `Map.insert` chain
produces, so its lookup results carry the correct variant tag.

**Relationship to the const/runtime map-equality miscompile.** This is the same root as "a map with a
computed key equals the same map with a constant key" (the c71 equality miscompile): both are the
computed-key `(map …)` literal producing a map that doesn't behave like a proper runtime map. Equality
compares it wrong; a lookup's Option matches it wrong. `Map.insert`-built maps are correct in both; only
the computed-key literal is broken. Fixing the literal's construction (build the real runtime map) should
resolve both symptoms.

**The lesson (const↔runtime must be invisible to behavior).** A value's construction path (const-folded
vs runtime-built) must not change how it behaves under any operation — the same discipline as
const-fold↔runtime arithmetic agreement. The computed-key map literal violates it: the map behaves
correctly when built const or via `Map.insert`, but the literal-with-runtime-key path produces a
mis-represented map whose lookup mis-dispatches. The tell: swap the key from a constant to `(+ 2 3)` (same
value 5) and a correct match (→1) becomes a wrong one (→-1).

**Corpus case added.** `spec/semantics/05-compound-types.sexp` §"matching a lookup from a computed-key map
literal selects the present-value arm" — `(let ((j (+ 2 3))) (match (Map.lookup (map (j 1)) 5) ((Some v)
v) ((None _) -1)))` MUST yield 1. Gated `(needs maps)`, realized; the behavior gate catches it (expected 1,
observed -1). A generation whose computed-key map literal is not yet a proper runtime map declines rather
than mis-dispatching its lookup result.
