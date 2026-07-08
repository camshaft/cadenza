# Nested-sum exhaustiveness is not checked

*2026-07-08*

**What happened.** Adversarial probing of match exhaustiveness found that the check does not compose into
nested patterns. `(match (Some (Some 5)) ((Some (Some x)) x) ((None _) -1))` arms the outer `Some` (with
an inner `Some`) and the outer `None`, but leaves `(Some (None _))` — a value of the scrutinee type
`Option (Option Int64)` — uncovered, with no wildcard. It runs to `5` instead of being rejected
non-exhaustive. The same gap appears for user sums (`(match (Some (T.A unit)) ((Some (T.A _)) 1) ((None
_) 0))` misses `(Some (T.B _))`) and tuples-of-sums (`(match (tuple (Some 1) (Some 2)) ((tuple (Some a)
(Some b)) …))` misses the `None` components). The flat case IS checked (`(match (Some 5) ((Some x) x))`
→ "does not cover every variant"), and the nested case with the CONSTANT scrutinee being the uncovered
value IS caught (`(match (Some (None unit)) ((Some (Some x)) x) ((None _) -1))` → "does not cover the
scrutinee"). So exhaustiveness is checked at the top level and value-driven at the nested level, but not
type-driven at the nested level.

**Why it is a break.** core-semantics.md #Matching Is Exhaustive Or Rejected: "A match whose patterns do
not cover every value of the scrutinee's type MUST be a compile-time error." #Patterns Compose: a
constructor pattern's binder MAY itself be a constructor pattern, matched recursively "to any depth". So
a value of type `Option (Option Int64)` ranges over `(Some (Some _))`, `(Some (None _))`, and `(None _)`;
a match omitting `(Some (None _))` with no wildcard leaves a value of the type uncovered — non-exhaustive,
CDZ0210, exactly as a flat match missing a top-level variant is. The composed arm set does not cover the
type: the outer `Some` is covered, but its payload's own variant set (`Some | None`) is not.

**Root cause (likely) — the exhaustiveness check covers the top-level variant set but does not recurse
into nested constructor positions, and on the static path it checks against the constant scrutinee's
nested shape rather than the type.** The flat check (top-level variant set present, or a wildcard) fires;
but when an arm's payload is itself a constructor pattern, the check does not verify that the payload
position's variant set is covered across the arm set. And the constant-scrutinee shortcut (the c32
family) reappears at the nested level: `(Some (Some 5))` hits `(Some (Some x))`, so the static path
returns that arm without asking whether `(Some (None _))` is covered. The fix is to make exhaustiveness a
recursive property over the composed pattern — at each constructor position, the union of the arms'
sub-patterns must cover that position's variant set (or a wildcard/binder must be present) — checked
against the TYPE, not the constant scrutinee's shape.

**The lesson (the recurring family).** Exhaustiveness is ARM-SET-vs-TYPE (the c32 lesson) AND it composes
recursively (the #Patterns Compose lesson) — and the check honored neither at the nested level: it stayed
top-level and value-driven. A property proven at the top level (flat variant-set coverage) and a property
proven for the pattern structure (patterns compose) must both hold together in the nested case. The tell:
the identical missing-variant is rejected flat (`(match (Some 5) ((Some x) x))`) but accepted nested
(`(match (Some (Some 5)) ((Some (Some x)) x) ((None _) -1))`), and rejected when the constant scrutinee IS
the uncovered case but accepted when it hits a covered arm.

**Corpus case added.** `spec/semantics/02-binding-and-control.sexp` §"a nested sum match missing an inner
variant is non-exhaustive" — `(match (Some (Some 5)) ((Some (Some x)) x) ((None _) -1))` MUST reject
CDZ0210, the nested companion of the flat sum-missing-a-variant and constant-scrutinee cases above it.
Native seed; the behavior gate catches it (expected reject CDZ0210, observed a running component). A
generation that does not yet check nested exhaustiveness declines rather than emitting.
