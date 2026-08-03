# PR #1326 review comments — implementation/compiler-ml/src/emit-rec-db.cdz (v-compiler-ml)

Mirrored from https://github.com/camshaft/cadenza/pull/1326 (PR: "cand: v-compiler-ml — af11c1078").
amazon-q claims VERIFIED against the diff (lines 7, 19).

## `List.at(...) => None` arm recurses instead of failing, in two index-loops (amazon-q, emit-rec-db.cdz:44 + :56) — defensiveness
> When `List.at` returns `None` unexpectedly, the function continues recursing and eventually returns
> `true` at the base case. This allows validation to pass when an out-of-bounds access occurs...
> If `i < List.len(...)`, `List.at(...)` should always succeed; a `None` indicates a defect that
> should cause validation to fail, not silently succeed.
> [suggests `| Option.None(_) => false`]

Verified: both `can-call-args` (:44/diff:7) and the new `all-defs-emittable` (:56/diff:19) use
`Option.None(_) => <recurse>`. HONEST SEVERITY: the loops guard with `if i >= len then true else
match List.at(...)`, so in the `else` branch `i < len` and `List.at` cannot return `None` — the arm
is effectively unreachable, so this is NOT a live "validation passes on OOB" bug today. BUT: (a) the
new `all-defs-emittable` gates whether `emit-recursive-module` emits vs declines — a validation
predicate where "None → true (emittable)" is the WRONG default if the invariant ever weakens; (b)
`| None => false` is the safer, intent-matching arm (an unexpected None IS a defect). Low-risk change,
worth taking for a gating predicate. (Note the pre-existing `can-call-args` had the same style, so
this is consistent-but-fragile, not a new regression.)
