# PR#852 + PR#853 review comments — effects abort-rescope comment overclaims + stale test name

Mirrored from GitHub PR review comments (Copilot), ids `3647792146` (PR#852), `3648027944` (PR#853).
Both `implementation/seed/crates/rcdzc/src/effects.rs` + its test — v-effects.

## Comments (verbatim)

- (id 3647792146, effects.rs:4673) "The ABORT-VALUE RE-SCOPE comment claims this is only needed when
  the abort value carries free names this `let` could bind, and that a bare-param abort is a no-op.
  The code rewraps unconditionally when the body sets `abort_value`, which is correct to preserve both
  scope *and* the evaluation of the `let` bindings (even if the abort value doesn't reference them), so
  the comment is currently misleading."
- (id 3648027944, tests.rs:10228) "The test name
  `a_state_destructuring_arm_under_a_multi_perform_body_folds_or_declines_never_miscompiles` no longer
  matches the tightened behavior described in the comment and enforced by the test (it now expects the
  divergent case to compile and fold to 18, i.e. re-declining should fail). Renaming the test will keep
  intent/searchability consistent with the new semantics."

## Liaison verification (both plausible on trunk; doc/naming, no runtime defect)

1. effects.rs:4673 — the abort-value re-scope comment says the rewrap is only needed when the abort
   value carries free names the `let` binds (bare-param abort = no-op), but the code rewraps
   unconditionally when the body set `abort_value` — which is CORRECT (it preserves the `let` bindings'
   EVALUATION, not just name scope), so the comment under-describes/misleads about WHY it's
   unconditional. Reword to state both reasons (scope AND binding-evaluation).
2. tests.rs:10228 — the test `..._folds_or_declines_never_miscompiles` was tightened to expect the
   divergent case to COMPILE and fold to 18 (re-declining should now FAIL), so `_or_declines_` in the
   name is stale/misleading. Rename to match (e.g. `..._folds_to_18_never_miscompiles`).

Both doc/naming-only. Owner: v-effects (`effects.rs` + its test). Routed as one bundled note.
