# Spec gap: what a repeated binder in one pattern means is unspecified

*2026-07-05*

**What happened.** Probing pattern matching, a `/loop` run reached a pattern that binds the same name
twice — `(tuple x x)` against a two-tuple, or `(Some x)` in an arm whose scope already binds `x`. The
run could not record an oracle. `core-semantics.md` §Pattern Matching says only that "A name a pattern
binds MUST be in scope only in the branch guarded by that pattern"; it does not say whether a pattern
may bind the same name more than once, and if so what that means. Three readings are each defensible and
observably distinct: (a) a repeated binder is a compile-time error (the linear-pattern discipline of
most ML-family languages); (b) the second binding shadows the first, so `(tuple x x)` binds `x` to the
second element; (c) the repeat is a non-linear equality constraint, so `(tuple x x)` matches only a
tuple whose two elements are equal (the Prolog/Erlang reading). This was already flagged in the seed's
notes as "a spec-ambiguous design Q" when the tuple-pattern-arity fix landed.

**Why.** The pattern-matching requirements pin binder *scope* but not binder *linearity*. The three
readings give different results for the same program — `(match (tuple 1 2) ((tuple x x) x) (_ 0))` is a
rejection under (a), 2 under (b), and 0 (falls through) under (c) — so there is no single recorded
behavior the corpus can carry until the specification chooses. It is an under-specification of the
pattern language, orthogonal to the arity/kind/exhaustiveness rules the corpus already witnesses.

**The requirement it drove.** *Deferred to a clarity pass* (this entry is the hand-off, per the
operator's request to document gaps for a clarity agent rather than resolve them inline). The resolution
should add one RFC-2119 sentence to `core-semantics.md` §Pattern Matching fixing binder linearity — the
conventional and simplest-to-implement choice is (a), a repeated binder in one pattern is a compile-time
error (a dedicated diagnostic, sibling to the arity/kind mismatches), which keeps patterns linear and
avoids the hidden-equality-constraint surprise of (c). Once fixed, it is witnessed by a case in
`spec/semantics/05-compound-types.sexp` (or `02-binding-and-control.sexp`): `(match (tuple 1 2) ((tuple
x x) x) (_ 0))` → the chosen outcome. Until then the corpus records nothing for a duplicate binder.
