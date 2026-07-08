# Comparing two same-shape nominal sum types answers false instead of rejecting

*2026-07-08*

**What happened.** Adversarial probing of the nominal-type boundary found that comparing values of two
DIFFERENT user-declared sum types that share a variant name answers `false` rather than rejecting the
comparison. `(type A (Mk Int64))` and `(type B (Mk Int64))` are distinct sum types; `(= (A.Mk 1) (B.Mk
1))` runs to `false`. The seed's own render carries the type tag — `(A.Mk 1)` vs `(B.Mk 1)` — so it knows
they are distinct nominal types, yet the comparison compares them structurally (on the shared variant set
`{Mk}` and the payload) and answers `false`. The analogous nominal-RECORD comparison is correctly caught:
`(= (Point (x 0) (y 0)) (Vector (x 0) (y 0)))` declines "comparison across a nominal boundary" (the
corpus pins it CDZ0202). When the two sum types instead have DIFFERENT variant names (`Foo` vs `Bar`),
the comparison declines "different shapes" — so only the same-variant-name case slips through to a wrong
`false`.

**Why it is a break.** type-system.md #Nominal Is An Orthogonal Modifier Over Any Structural Type makes
nominal available over "record, tuple, or SUM", and requires two nominal types to be "distinct whenever
their fully-qualified names differ, even when their underlying structures and their declared local names
are identical." #Nominal Types Are Not Comparable Across Their Boundary then makes a comparison of two
different nominal types a type error (CDZ0202). So `A` and `B` — distinct fully-qualified names, identical
structure and shared local variant name `Mk` — are distinct nominal types, and `(= (A.Mk 1) (B.Mk 1))`
MUST be rejected CDZ0202, exactly as the Point/Vector record case is. Answering `false` is the untagged
structural comparison the nominal boundary forbids — a wrong VALUE, not merely a missing rejection: the
spec says the comparison must be caught, and `false` answers it.

**Root cause (likely) — the nominal-tag identity is tracked for nominal records in comparison but not for
user-declared sums.** The equality path recognizes the prelude nominal records `Point`/`Vector` and
declines across their boundary, but for a user `(type …)` sum it falls back to a purely structural
comparison keyed on the variant set and payload: two sums with the same variant name `{Mk}` are treated
as the same shape, compared structurally, and (being structurally identical or not) answered `true`/
`false` — the tag `A` vs `B` is dropped. The fix is to carry the sum's nominal identity (its declaring
type's fully-qualified name) into the comparison and reject CDZ0202 when the two operands' nominal
identities differ, exactly as the record path does — before the structural variant-set comparison.

**The lesson (the recurring family).** The nominal-boundary rejection is proven on one structural kind
(nominal records — and symbols, a nominal over String) but not carried to the sibling (nominal sums),
even though the spec declares nominal "orthogonal over record, tuple, or sum." This is the same "a check
proven on one form is not carried to its sibling" shape as the annotation-descent (tuple/list/sum vs
record), the call/perform argument-type, and the bool/sum-vs-int exhaustiveness findings — here the
siblings are the structural kinds a nominal tag can sit over. The tell: the identical nominal-boundary
comparison rejects for two nominal records but answers `false` for two nominal sums. And because a
value-driven structural fallback answers rather than declines, the failure is a wrong value, not a safe
decline.

**Corpus case added.** `spec/semantics/05-compound-types.sexp` §"comparing two same-shape nominal sum
types is a type error, not false" — `(do (type A (Mk Int64)) (type B (Mk Int64)) (= (A.Mk 1) (B.Mk 1)))`
MUST reject CDZ0202, the sum sibling of the existing nominal-record boundary cases. Gated `(needs
sum-type-declaration)`, which the seed realizes, so the behavior gate runs and catches it (expected reject
CDZ0202, observed a running component answering `false`). A generation that does not yet track nominal
tags on a sum declines rather than answering `false`.
