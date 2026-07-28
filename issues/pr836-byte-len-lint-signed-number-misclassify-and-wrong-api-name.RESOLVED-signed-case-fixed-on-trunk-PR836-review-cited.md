# PR#836 review comments — the new byte-len-scalar-walk corpus lint: misclassifies signed numbers + recommends a non-existent API

Mirrored from GitHub PR review comments (Copilot), ids `3636927236`, `3636927318`.
PR: https://github.com/camshaft/cadenza/pull/836 (batch-staging; fixes belong on trunk)
Location: `xtask/src/main.rs:4498` + `:4650` — the `byte-len-bounds-scalar-walk` corpus lint (commit
`74ad333c5`, "Tier-2 WARN-level corpus lint … concierge ruling C part 2").

CONTEXT: this lint is the durable guard that came out of my PR#832 recurrence suggestion — the concierge
ruled C (both lint + convention). Copilot is now reviewing the lint itself; both findings are real.

## Comments (verbatim)

- (id 3636927236, main.rs:4498) "`call_first_ident_args` is documented as skipping numeric literals,
  but the current check only excludes tokens whose *first char is an ASCII digit*. This will
  misclassify signed numbers like `-1`/`+1` (and other non-digit numeric prefixes) as identifiers,
  producing misleading lint warnings."
- (id 3636927318, main.rs:4650) "The warning text suggests using `String.length`, but this codebase
  uses `String.scalar-len` for codepoint/scalar counts. As written, the warning recommends a
  non-existent API name."

## Liaison verification (CONFIRMED on trunk)

1. main.rs:4494-4498: the token filter is
   `!tok.is_empty() && !tok.starts_with('"') && !tok.chars().next().unwrap().is_ascii_digit()`. It only
   rejects a leading ASCII digit — a signed literal `-1`/`+1` starts with `-`/`+`, passes the filter,
   and is collected as an "identifier" → a spurious/misleading lint warning on a numeric arg. The doc
   comment (line ~4490) says "skip … a numeric literal", so code ≠ doc. Fix: also exclude a leading
   `-`/`+` followed by a digit (or a full numeric-literal check).
2. main.rs:4650: the WARN text says "Prefer a codepoint-length bound (e.g. `String.length`) …" but
   there is NO `String.length` in the prelude — the codepoint-count API is `String.scalar-len` (the
   very API this whole lint steers people toward). Recommending a non-existent name in the guidance is
   both wrong and ironic. Fix: `String.scalar-len`.

Both are real defects in the freshly-landed lint (a false-positive source + wrong remediation advice).
Owner: v-fleet-tooling (owns `xtask`; the lint is `74ad333c5`). Routed as a note. Minor but the lint is
new + operator-visible, so worth fixing before it emits confusing warnings.
