# PR#707 review comment — db.rs doc references a non-existent test name

Mirrored from GitHub PR review comment (Copilot), id `3617634655`.
PR: https://github.com/camshaft/cadenza/pull/707 (merged; fix still belongs on trunk)
Location: `implementation/seed/crates/rcdzc/src/db.rs:228`

## Comment (verbatim)

> The doc comment references a non-existent test name
> (`resolved_of_clone_count_is_bounded_on_a_match_heavy_program`). The actual regression
> test added in this PR is `a_wide_match_resolves_in_a_bounded_number_of_clones`, so the
> pointer is currently misleading.

## Liaison verification (CONFIRMED)

- `db.rs:228` doc for `RESOLVED_OF_CALLS` ends: "See `resolved_of_clone_count_is_bounded_on_a_match_heavy_program`."
- No such test exists. The real regression test is `a_wide_match_resolves_in_a_bounded_number_of_clones`
  at `implementation/seed/crates/rcdzc/src/tests.rs:60037`.

Fix: update the doc pointer on db.rs:228 to name `a_wide_match_resolves_in_a_bounded_number_of_clones`.
Doc-only, no behavior change. This instrumentation is part of the `resolved_of`→`resolved_ref`
borrow-family clone-counter cleanup → routed to v-memory-safety.

## RESOLVED (corpus-bugfix, 2026-07-21, trunk 1294a22b4)
STALE — already fixed on trunk. db.rs:228 now reads "See `a_wide_match_resolves_in_a_bounded_number_of_clones`"
(the correct test name); the non-existent `resolved_of_clone_count_is_bounded_on_a_match_heavy_program` is
gone (grep: real-test 1 hit, stale-name 0 hits). The doc pointer was corrected between the PR#707 filing and
now (bundled with the RESOLVED_OF_CALLS/db.rs work). No route needed. Marked RESOLVED.
