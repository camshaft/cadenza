# A tuple annotated with the wrong arity is not rejected

*2026-07-08*

**What happened.** Adversarial probing of the annotation checker found that a tuple annotated with a
type of the wrong ARITY is accepted. `(: (tuple 1 2) (Tuple Int64 Int64 Int64))` — a two-element tuple
annotated as a three-element tuple type — runs to `(tuple 1 2)` instead of rejecting. Both directions
slip through: `(: (tuple 1 2) (Tuple Int64))` (too few) and `(: (tuple 1 2 3) (Tuple Int64 Int64))` (too
many) are also accepted. The element-TYPE check does fire — `(: (tuple 1 2) (Tuple Int64 Bool))` is
rejected "annotation's parameter type contradicts the value" — so only the arity comparison is missing.

**Why it is a break.** type-system.md #A Tuple Is Reshaped Positionally … states a tuple is "a fixed-size
positional value whose length is part of its type", and #The Structural Types Are Record, Tuple, And Sum
gives a tuple's shape as "its element types in order". So a two-element tuple has type `(Tuple Int64
Int64)`, which cannot unify with a three-element `(Tuple Int64 Int64 Int64)` any more than a wrong
element type can — a provable contradiction that #Annotations Constrain, Never Contradict requires be
rejected (CDZ0203). Accepting it and returning `(tuple 1 2)` under a declared three-element type is the
silent annotation-replaces-inference the section forbids.

**Root cause (likely) — the annotation-descent walks shared element positions but never compares the
tuple arities.** The annotation-contradicts check (`matches_annotation` / `annotation_payload_param`)
recurses into a tuple annotation's element types and compares each positionally against the value's
elements (so a wrong element type is caught), but it iterates over the shared/available positions without
first checking that the annotation's element count equals the value's element count. So a longer or
shorter annotation type matches on the overlapping prefix and the length difference is ignored. The fix
is to compare the tuple's arity against the annotation's arity before (or alongside) the positional
element-type walk, rejecting CDZ0203 on a mismatch — exactly as the structural-comparison rule already
requires tuples be "comparable only when their lengths are identical" (type-system.md #Structural Values
Are Comparable Only When Their Shapes Match).

**The lesson (the recurring family).** The annotation check for a compound covers one aspect (element
types) but not the sibling aspect (arity / length) of the same shape. This is a within-form instance of
the "a check proven on one aspect is not carried to its sibling" family — the same shape as the
annotation-descent landing for a sum's payload, a list's element, and a record's field but not (until
now) a record's field-set or a tuple's arity. A structural type's shape is BOTH its constituent types AND
their count/set; a checker that verifies the types positionally must also verify the count, or a
wrong-length annotation passes on the overlapping prefix. The tell: the identical annotation is rejected
for a wrong element type but accepted for a wrong element count.

**Corpus case added.** `spec/semantics/07-type-system.sexp` §"a tuple annotated with the wrong arity is
rejected" — `(: (tuple 1 2) (Tuple Int64 Int64 Int64))` MUST reject CDZ0203, the arity companion of the
existing wrong-element-type annotation-descent cases (list element, record field, sum payload). Native
seed; the behavior gate catches it (expected reject CDZ0203, observed a running component). A generation
that does not yet check tuple arity declines rather than accepting.
