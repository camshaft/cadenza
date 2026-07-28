# PR#850 review comments — effects do-local-def work: stale test doc-block + overclaiming rewrite comment

Mirrored from GitHub PR review comments (Copilot), ids `3647424130`, `3647424162`.
PR: https://github.com/camshaft/cadenza/pull/850 (merged; fixes belong on trunk)
Locations: `implementation/seed/crates/rcdzc/src/tests.rs:10014`, `implementation/seed/crates/rcdzc/src/effects.rs:1948`.
Both from the do-local-value-def-into-perform work (`e49c698a1`).

## Comments (verbatim)

- (id 3647424130, tests.rs:10014) "The doc comment immediately above this test currently includes a
  large preceding section about state-destructuring/multi-perform threading
  ('peel_resume_from_arm_body', fold-to-18, etc.). As a result, rustdoc will associate that unrelated
  documentation with this do-local-def regression test, which is misleading. Consider removing that
  stale section (it's duplicated by the later dedicated state-destructuring test) and keep a doc
  comment that describes only this test."
- (id 3647424162, effects.rs:1948) "This comment claims the lifted `(def v e)` is a 'pure value def'
  that 'sequences no effect', and also says it rewrites 'each non-final value def'. Both statements are
  stronger than what the code guarantees/implements (the rewrite applies to a leading chain of value
  defs, and `e` may itself perform). Rewording this comment would avoid documenting an incorrect
  semantic assumption."

## Liaison verification (CONFIRMED plausible on trunk; both doc-accuracy)

1. tests.rs:10014 — a large doc block about a DIFFERENT scenario (state-destructuring / multi-perform,
   `peel_resume_from_arm_body`, fold-to-18) sits immediately above the do-local-def regression test, so
   rustdoc attaches it to the wrong test (same doc-attachment class as several earlier findings). The
   state-destructuring content is duplicated by its own dedicated test later. Fix: trim the block to
   describe only THIS test.
2. effects.rs:1948 — the comment overclaims: "pure value def … sequences no effect" and "each non-final
   value def". The code rewrites a LEADING CHAIN of value defs, and the bound `e` MAY itself perform, so
   neither "pure/no-effect" nor "each" is accurate. Fix: reword to "a leading chain of value defs
   (whose RHS may itself perform)".

Both doc-only, no behavior change. Owner: v-effects (`rcdzc/src/effects.rs` + its test; `e49c698a1`).
Routed as one bundled note.
