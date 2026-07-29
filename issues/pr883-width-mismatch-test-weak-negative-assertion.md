# PR#883 review comment — width-mismatch handler test weak negative-substring assertion (v-effects)

Mirrored from GitHub PR#883 review comment (Copilot), id `3669316789`.
File: `implementation/seed/crates/rcdzc/src/tests.rs:59107` — rcdzc effects test. Blame `5cf911aeb`
"rcdzc(effects): decline a width-mismatched handler state cleanly, never emit invalid wasm (F1)" →
v-effects.

## Comment (verbatim)

- (id 3669316789, tests.rs:59107) "This test only asserts that no *particular* error message substring
  appears. As written it can pass even if compilation fails with an unrelated coded error (or a different
  error message), which weakens it as a regression guard for the intended behavior (either clean fold
  with no errors, or an uncoded handler-not-reducible decline)."

## Liaison verification (confirmed on trunk f85b2c320)

The assert (tests.rs:59101-59107) only checks `!errors.iter().any(|d| d.message.contains("invalid") ||
.contains("type mismatch") || .contains("failed to compile"))`. So the test passes if compilation fails
with ANY error whose message avoids those three substrings — e.g. an unrelated CODED reject (CDZ0xxx) or
a differently-worded failure. The intended contract (per the comment at 59093-59095): "either it
declines (uncoded todo) or it folds — never a coded reject and never an emit failure". The assertion
doesn't enforce the "never a CODED reject" half — a coded error slips through as a false green. Tighten:
assert that any error-severity diagnostic present is an UNCODED decline (`d.code.is_none()`), i.e.
`errors.iter().all(|d| d.code.is_none())` (plus keeping the no-invalid-module substring check, or better
asserting on the diagnostic code/kind rather than message substrings). Test-quality only; the sibling
NO-REGRESSION half (Int64 matching-width folds to 25, lines 59108-59117) is a real positive assertion and
is fine.

Owner: **v-effects** (their `5cf911aeb` width-mismatch decline pin). Tighten the negative assertion to
`code.is_none()`.
