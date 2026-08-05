# PRs #1855 + #1854 review comments — LOW

## PR #1855 (rcdzc/src/tests.rs, v-inference — bare under-applied generic in VALUE annotation, my #1838 lineage) — 2 LOW
- tests.rs:24298 — the test asserts the human-readable message substring but NOT the diagnostic CODE;
  since the behavior is meant to be CDZ0203, assert the code too (a message-only assert is brittle + doesn't
  pin the code). LOW/test-precision.
- tests.rs:24284 — the opening `{` for this long test name is mis-indented (`     {`) vs rustfmt's next-line
  convention. LOW/style (rustfmt should catch — verify it's not a rustfmt-exempt spot).

## PR #1854 (spec/semantics/14-effects-and-handlers.sexp:7479, breaker) — LOW/doc
The `or`-focused case doc references `Core::And` — CORRECT (one core node for both and/or), but the wording
may confuse (an `or` case citing `Core::And`). Add a half-clause noting And is the shared and/or node so a
reader doesn't think it's a typo. LOW/doc.
