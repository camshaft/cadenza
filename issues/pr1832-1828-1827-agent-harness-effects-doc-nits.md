# PRs #1832 + #1828 + #1827 review comments — LOW doc/test

## PR #1828 (cdz-kernel/src/kernel.rs:1401, v-agent-harness, MERGED) — doc/accuracy
Test comment says `deliver` is a "pure function of (input, reducer)", but `deliver` also depends on the
STARTING SESSION STATE (the log/kv it folds onto). Reword to "(starting state, input, reducer)". LOW/doc.

## PR #1832 (rcdzc/src/tests.rs:24258, v-inference, OPEN) — doc/test
Test comment claims it locks correct singular/plural grammar + an actionable "— write `(Name …)`" hint,
but (per Copilot) the current assertion doesn't fully pin that. Verify the assert matches the comment's
claim (pin the grammar + hint, or soften the comment). LOW/test-precision.

## PR #1827 (spec/semantics/21-host-closures.sexp:232, v-effects, MERGED) — doc/style
Backend-status summary uses "wasm + rust" spacing vs the nearby cases' convention. Align spacing. LOWEST.
