# PR#941 review comment — --warm-only per-file test discards `t`, doesn't assert PASS ran (v-cdz-tooling)

Mirrored from GitHub PR#941 review comment (Copilot), id `3687450940`.
File: `implementation/seed/crates/cdz/tests/test_per_file_cli.rs:722` — v-cdz-tooling. Blame `fa96c358f`
"cdz test: add --warm-only (warm the provider cache without running tests) — gate wall-clock lever".

## Comment (verbatim)

- (id 3687450940, test_per_file_cli.rs:722) "In the per-file sweep portion of this test, the loop binds
  `t` but then discards it (`let _ = t;`). As written, the test only asserts the process exit code and
  provider-cache HIT; it does not assert that the expected `@test` actually ran (a single-file `cdz test`
  can exit 0 even when it finds 0 tests). Consider capturing stdout from each per-file run and asserting
  it contains the expected `PASS {t}` line, removing the unused-variable workaround at the same time."

## Liaison verification (confirmed on trunk 512bf5610)

The loop (test_per_file_cli.rs:714-722): `for (f, t) in [("ta.cdz","t_ta"),…] { let (ok, err) = run(f);
let _ = t; assert!(ok, …); assert!(err.contains("[provider-cache] hit") && !err.contains("miss
persisted"), …); }`. The `t` (expected test name, e.g. `t_ta`) is bound then explicitly DISCARDED (`let _
= t;`), and the asserts only check exit-OK + cache-hit — NOT that the test named `t` actually RAN. Per
the known cdz behavior (which THIS liaison flagged in PR#881-era work), a single-file `cdz test` exits 0
even with ZERO tests found — so a regression that made warm-only silently run no tests would still pass
this. Fix (Copilot's, sound): capture each run's STDOUT and assert it contains `PASS {t}`, dropping the
`let _ = t;` workaround (which only exists because `t` was otherwise unused). Test-coverage,
behavior-neutral. (`run(f)` currently returns `(ok, err)` = exit + stderr; may need to also surface
stdout.)

Owner: **v-cdz-tooling** (`cdz/tests`, `--warm-only` test `fa96c358f`). Assert `PASS {t}` per per-file
run; remove the `let _ = t;`.
