# PR #1543 review comment — implementation/seed/crates/rcdzc/src/lower.rs (v-effects)

Mirrored from https://github.com/camshaft/cadenza/pull/1543 (PR: "[v-effects] 04a44dc68").
This is the adv-62 doc-clarification PR (reword the `Resolved::Host` ⇒ host-perform check as a
CONSERVATIVE OVER-APPROXIMATION — the exact reword this liaison filed on PR #1528). Good follow-up.

## "see the op-ref tests" points at a non-existent identifier (Copilot, lower.rs:8094) — doc
> The comment references "op-ref tests", but there is no such identifier elsewhere in the repo; this
> makes the pointer hard to follow. Consider pointing at the concrete regression test that exercises
> `(host (E) (E.get))` compiling without a perform.

VERIFIED against the diff: the new comment (lower.rs:8094 + the sexp-side docstring at ~:8107) says
"an op-reference-only body like `(host (E) (E.get))` … compiles WITHOUT a perform; see the op-ref
tests" — but "op-ref tests" is prose, not a locatable symbol/path. Point it at the concrete
regression (the tests.rs case around 64310-64316 cited in the original #1528 review) so a future
reader can actually find it. Doc-only, LOW.
