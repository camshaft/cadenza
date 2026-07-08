# Int64 match exhaustiveness is value-driven on a constant scrutinee

*2026-07-08*

**What happened.** Adversarial probing of match exhaustiveness found that a wildcard-free Int64 match
on a COMPILE-TIME CONSTANT scrutinee is accepted when the constant hits a present arm, rather than
rejected non-exhaustive. `(match 5 (5 1))` runs to `1` — the scrutinee folds to `5`, the sole arm names
`5`, and the compiler returns the arm the constant matched without checking that a finite set of literal
arms cannot cover Int64. The same match on a DYNAMIC scrutinee is correctly rejected: `(match x (5 1))`
for a parameter `x` declines "match does not cover the scrutinee" (CDZ0210). So exhaustiveness for Int64
is value/scrutinee-driven on the static path and arm-set-vs-type on the dynamic path — an asymmetry.

The Bool and Sum siblings do NOT have this asymmetry: `(match true (true 1))` and `(match (Some 5)
((Some x) x))` both reject CDZ0210 even though the constant scrutinee hits the present arm — the corpus
pins each as "…on a constant scrutinee is non-exhaustive even when the constant hits the sole arm." Only
Int64 still takes the value-driven shortcut.

**Why it is a break.** core-semantics.md #Matching Is Exhaustive Or Rejected: "A match whose patterns do
not cover every value of the scrutinee's TYPE MUST be a compile-time error." An Int64's type has 2^64
values; no finite set of literal arms covers it, so a wildcard-free Int64 match is non-exhaustive exactly
as a Bool match missing an arm or a sum match missing a variant is. Exhaustiveness is a compile-time
property of the arm set against the type, not of which value the scrutinee happens to hold — the constant
`5` hitting the arm `5` does not excuse every other Int64 being uncovered.

**Root cause (likely) — the static-scrutinee compile path skips the arm-set-vs-type check for Int64.**
This is the exact residue of the bug the bool path already fixed (recorded in
`[[bool-match-exhaustiveness-static-scrutinee]]`): `gen_match`'s static/const-scrutinee branch checks
sum exhaustiveness (`sum_match_exhaustive`) and bool exhaustiveness (`match_scrutinee_is_bool` +
`bool_match_exhaustive`), then returns the first arm the constant matches. For an Int64 constant
scrutinee there is no parallel guard: it returns the matched arm's value without asking whether the arm
set covers the Int64 type, so a wildcard-free int match is accepted whenever the constant hits an arm.
The fix is the Int64 parallel of the bool guard: in the static-scrutinee branch, an Int64 (or any
scalar with an infinite value set) match with only literal arms and no wildcard/catch-all is
non-exhaustive → reject CDZ0210, regardless of which arm the constant matched.

**The lesson (the recurring family).** Exhaustiveness is ARM-SET-vs-TYPE, never scrutinee-value-driven —
and a fix proven on one scrutinee kind (bool, then sum) must carry to the third (int). The bool path
learned "a constant scrutinee that happens to hit a present arm does NOT excuse a missing arm"; the sum
path learned it; the int path was never given the parallel guard. This is the same "a check proven on
one form is not carried to its sibling" shape as the collection-growth-operator, if/match
unselected-alternative, call/perform argument-type, and record-field-annotation findings — here the
siblings are the three scrutinee kinds a static match path dispatches on. The tell: the identical
wildcard-free single-literal-arm match rejects on a dynamic scrutinee but runs on a constant one, and
rejects for bool/sum constants but runs for an int constant.

**Corpus case added.** `spec/semantics/02-binding-and-control.sexp` §"an int match on a constant
scrutinee is non-exhaustive even when the constant hits the sole arm" — `(match 5 (5 1))` MUST reject
CDZ0210, the Int64 companion of the existing constant-scrutinee present-arm bool and sum cases. Native
seed; the behavior gate catches it (expected reject CDZ0210, observed a running component). A generation
that does not yet check int-literal exhaustiveness on a constant scrutinee declines rather than accepting.
