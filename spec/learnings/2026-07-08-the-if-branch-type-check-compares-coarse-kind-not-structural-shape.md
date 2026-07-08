# The if-branch type check compares coarse kind, not structural shape

*2026-07-08*

**What happened.** Adversarial probing of conditional branch-type agreement found two wrong-value
miscompiles: `(if true (tuple 1 2) (tuple 3 4 5))` runs to `(tuple 1 2)` (and `(if false …)` to
`(tuple 3 4 5)`), and `(if true (tuple 1 2) (tuple 1 true))` runs to `(tuple 1 2)`. In each the two
branches are *different types* — a two-tuple vs a three-tuple (arity is part of a tuple's type), and
`(Tuple Int64 Int64)` vs `(Tuple Int64 Bool)` (element type differs) — so the conditional has no single
type and must be rejected. Every coarser mismatch is correctly caught: Int-vs-Bool, Int-vs-Float,
tuple-vs-scalar, tuple-vs-list branches all reject CDZ0201. Only two branches of the *same kind* but
different structure slip through.

**Why it is a break.** core-semantics.md #Conditionals Evaluate One Branch: "Every branch of a
conditional MUST be type-checked whether or not it is evaluated." The corpus (02-binding-and-control.sexp
§"a conditional's branches must have the same type") pins Int/Bool, Int/Float, and compound/scalar
mismatches as CDZ0201. A tuple's arity and element types are part of its type (type-system.md), so two
tuple branches that differ in either are as ill-typed as an Int/Bool pair — the whole `if` cannot have
one type. Returning whichever branch the constant condition selects runs a program that must be rejected.

**Root cause — the branch comparison is at kind granularity.** In the seed
(`codegen.rs::check_type_rejections`, the `"if"` arm), the check is
`if let (Some(ta), Some(tb)) = (static_type(then), static_type(else)) { if ta != tb { reject } }`.
`static_type` returns a coarse `StaticType` enum (Bool / Int / Float / Tuple / List / Record / Sum …) —
a *kind*, not a structural type. So `(tuple 1 2)` and `(tuple 3 4 5)` both yield `StaticType::Tuple`,
`ta == tb` holds, and the mismatch passes. The compiler already has the right tool: `shapes_incompatible`
does a full recursive structural comparison (arity, element types, sum variant tags) and is used by the
list-element-homogeneity check. The if-branch check just never adopted it — it stayed at the coarse-kind
comparison that predates structural checking.

**The lesson (the same family, at a coarser resolution).** This run's recurring defect — *a check that
covers only part of its obligation* — here takes the form of a check run at the wrong *granularity*: kind
where the type demands structure. It is subtle because `ta != tb` looks like a real type comparison; the
gap is that `StaticType` is lossy, collapsing every tuple to one value. Two sibling checks in the same
file already went structural (list homogeneity via `shapes_incompatible`, pattern shape via
`check_pattern_shape`); the if-branch check is the one that didn't, so the same class of mismatch that is
caught in a list `(list (tuple 1 2) (tuple 3 4 5))` is missed across `if` branches. When one check in a
compiler is upgraded from kind-equality to structural-equality, its siblings comparing the same kinds of
values need the same upgrade — a coarse `==` on a lossy type descriptor is not a type check.

**Corpus cases added.** `spec/semantics/02-binding-and-control.sexp` §"a conditional with two tuple
branches of different arity is a type error" (`(if true (tuple 1 2) (tuple 3 4 5))`) and §"a conditional
with two tuple branches of different element type is a type error" (`(if true (tuple 1 2) (tuple 1
true))`), both MUST reject CDZ0201, as the compound-vs-compound companions of the existing scalar and
compound/scalar branch cases. Native seed; the behavior gate catches both (expected reject CDZ0201,
observed a running component). Fix: compare the two branches with `shapes_incompatible` when both
const-fold to compounds, alongside the existing coarse-kind check.
