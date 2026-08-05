# PR #2077 review — rcdzc/src/effects.rs (v-effects) — MERGED — correctness [VERIFIED-PLAUSIBLE, MED] (HIGH-class fix)

https://github.com/camshaft/cadenza/pull/2077 (FIX recursive-performer-of-nested-op-whose-resume-performs-
outer — a self-probed HIGH miscompile). Copilot (id 3714910008) flags the lift can orphan the arm's state
binder.

## `lift_inner_op_arm_outer_perform` lifts an arm resume VALUE that may still reference `arm.state`; after lifting, that binder is out of scope → unbound/mis-resolved reference (Copilot, effects.rs:6753) — correctness [VERIFIED-PLAUSIBLE]
> `lift_inner_op_arm_outer_perform` can lift an arm resume VALUE that still references the arm's state
> binder (`arm.state`). After lifting, that binder is no longer in scope (the perform is gone), so any
> such reference becomes unbound or resolves incorrectly. Either substitute the state binder to the slot's
> state-ref, or conservatively skip lifting when `val` references `arm.state`.

VERIFIED the mechanism. The function (effects.rs:6742) matches an inner-op call whose arm resume-value
performs an OUTER discharged op with trivial inner-state (guard: `next == arm.state`), then β-reduces `val`
with `params↦args` (the op's args) + `deep_fresh_copy`s it, and RETURNS it as the lifted node — replacing
the inner-op call. The β-subst covers the arm PARAMS but NOT `arm.state`. The `next == arm.state` guard only
ensures the NEXT-state is trivial ("no advance to preserve") — it does NOT ensure `val` itself is free of
`arm.state` references. So a resume value that READS the threaded inner state (`(resume (+ (Inner.op)
inner_state) …)`-ish, where `val` mentions `arm.state`) gets lifted with a now-orphaned `arm.state` ref →
CDZ0101-class unbound-name / mis-resolve — the same state-binder-scope failure mode as this arc's #1933
(orphaned state-ref) and the deep-fresh-copy discipline the sibling arms use. MED (correctness in a
freshly-landed HIGH-class fix; reachability depends on whether a lifted resume-value that reads arm.state
occurs in practice — the trivial-next-state guard makes a state-READING val plausible even when the
next-state is trivial).

Fix per Copilot: either (a) substitute `arm.state` → the slot's state-ref in the β-subst (so a reference
resolves to the threaded state at the lift site), or (b) conservatively SKIP lifting when `val` references
`arm.state` (`subtree_references(db, val, arm.state)`) — falling back to the decline path. (b) is the safe
minimum; (a) is more complete if a state-reading resume-value is a shape you want to lift. v-effects should
confirm with a witness: an inner-op arm whose resume-value reads `arm.state` AND performs an outer op with
trivial next-state → check the lift doesn't emit an unbound state ref. v-effects owns effects.rs.
