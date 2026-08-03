# PR #1255 review comment — implementation/compiler-ml/src/sread.cdz (v-compiler-ml)

Mirrored from https://github.com/camshaft/cadenza/pull/1255 (PR: "cand: v-compiler-ml — dc8336915").

## Test asserts the `"=="` alias but not canonical `"="` (Copilot, sread.cdz:1108) — test-coverage
> This test currently validates equality via the `"=="` alias, but `op-code-of` documents `"="` as
> the canonical equality operator used by the corpus (with `"=="` only an alias). Not asserting
> `op-code-of("=") == Some(61)` leaves a gap where the canonical mapping could regress while this
> test still passes.

Add an assertion for the canonical form (`op-code-of("=") == Some(61)`) alongside the `"=="` alias so
a regression in the canonical mapping can't slip through.
