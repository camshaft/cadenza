# PR#918 review comment — grouping regression test can pass via the fallback path, not the composed path it guards (v-cdz-tooling)

Mirrored from GitHub PR#918 review comment (Copilot), id `3684213303`.
File: `implementation/seed/crates/cdz/tests/test_per_file_cli.rs:628` — v-cdz-tooling. Blame `48fba368c`
"cdz test: store only a group's TARGET consumers, not imported-member consumers (PR#914)" — the
regression test for the PR#914-A grouping fix I routed.

## Comment (verbatim)

- (id 3684213303, test_per_file_cli.rs:628) "This test can pass even if the composed/grouped precompile
  path is not exercised (e.g., if grouping declines and `cdz test` falls back to per-file `EmitTests`).
  Since the regression is in the grouping path, make the test assert that a composed provider was
  actually emitted/persisted (using the existing `CDZ_PROVIDER_CACHE_TRACE` hook) so it fails if it
  accidentally runs only the fallback path."

## Liaison verification (confirmed on trunk 5dfc74b9e)

The test (test_per_file_cli.rs:620-628) asserts `out.status.success() && stdout.contains("PASS t_mid") &&
stdout.contains("PASS t_hi")` + no wrong-group-link error strings. But `cdz test` is BEST-EFFORT: if the
grouped/composed precompile DECLINES for any reason, it silently falls back to per-file `EmitTests`, under
which both tests still PASS (t_mid/t_hi run standalone) — so the assertion holds WITHOUT ever exercising
the grouping path the PR#914 regression lives in. A future regression that breaks grouping (making it
decline) would leave this test GREEN via fallback — a false guard. Copilot's fix is sound: assert the
COMPOSED path actually ran — e.g. via the existing `CDZ_PROVIDER_CACHE_TRACE` hook (assert a composed
provider was emitted/persisted), so the test fails if it silently degrades to fallback. Test-coverage;
behavior-neutral.

Owner: **v-cdz-tooling** (`cdz/tests`, their PR#914 grouping-fix regression test `48fba368c`). Assert the
composed provider was emitted (CDZ_PROVIDER_CACHE_TRACE) so the test can't pass via fallback.
