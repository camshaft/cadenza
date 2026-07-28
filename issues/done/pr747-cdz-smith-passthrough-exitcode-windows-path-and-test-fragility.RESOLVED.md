# PR#747 review comments — `cdz smith` passthrough: exit-code truncation, Windows PATH, fragile test

Mirrored from GitHub PR review comments (Copilot), ids `3623988472`, `3623988501`, `3623988551`.
PR: https://github.com/camshaft/cadenza/pull/747 (merged; fix still belongs on trunk)
Locations:
- `implementation/seed/crates/cdz/src/main.rs:3699` (`run_smith` PATH fallback)
- `implementation/seed/crates/cdz/src/main.rs:3708` (exit-code propagation)
- `implementation/seed/crates/cdz/tests/smith_cli.rs:82` (passthrough test)

All landed in `c2c4c5f88` ("cdz: mount `cdz smith`/`cdz fuzz` as a passthrough to the standalone
cdz-smith fuzzer") — v-cdz-tooling's unified `cdz` CLI.

## Comments (verbatim)

- (id 3623988472, main.rs:3699) "`run_smith` falls back to `PathBuf::from(\"cdz-smith\")` for `$PATH`
  lookup. On Windows this may fail to resolve an installed `cdz-smith.exe` depending on how `Command`
  performs extension resolution. Mirror the `locate_cdz_smith` Windows handling for the PATH fallback
  to avoid spurious NotFound errors on Windows."
- (id 3623988501, main.rs:3708) "`ExitCode::from(status.code().unwrap_or(1) as u8)` truncates the
  child's exit status (and also treats negative/out-of-range codes as arbitrary u8 values). This can
  misreport failures, especially if `cdz-smith` returns a code outside 0..=255. Convert with
  `u8::try_from` and fall back to `ExitCode::FAILURE` when the code is unavailable or out of range."
- (id 3623988551, smith_cli.rs:82) "This test assumes the forwarded flags are accepted by `cdz-smith`
  (it currently parses its own args and will exit non-zero on unknown ones like
  `--iters`/`--not-a-cdz-flag`). If `cdz-smith` happens to be present beside `cdz`, the test will fail
  even though passthrough is working. Prefer a passthrough assertion that doesn't depend on
  `cdz-smith`'s accepted flags — e.g. forward a leading-hyphen value as the first arg and accept
  either the clean not-found error or `cdz-smith`'s own 'unknown subcommand' error."

## Liaison verification (CONFIRMED on trunk)

- main.rs:3708 — `ExitCode::from(status.code().unwrap_or(1) as u8)` — the `as u8` truncates a code
  outside 0..=255 (e.g. a child exit 256 → 0 = false SUCCESS report). Real (minor) fidelity bug.
  Fix: `u8::try_from(code).map(ExitCode::from).unwrap_or(ExitCode::FAILURE)`.
- main.rs:3699 — `locate_cdz_smith().unwrap_or_else(|| PathBuf::from("cdz-smith"))` — the co-built
  path is handled, but the bare-name PATH fallback may not resolve `.exe` on Windows. Plausible;
  low priority (fleet runs Linux) but a correctness point for a Windows install.
- smith_cli.rs:82 — the passthrough test's robustness depends on cdz-smith's accepted flags; if a
  cdz-smith is co-present the assertion can spuriously fail. Test-hardening point.

All three are v-cdz-tooling's `cdz smith`/`fuzz` passthrough. Routed as a note. Exit-code truncation
is the most substantive (a real if narrow mis-report); the other two are robustness/portability.

## RESOLUTION (v-cdz-tooling, verified on trunk 9caca2b2d)
All 3 findings FIXED — landed in 356a00083 ("cdz: fix cdz smith passthrough exit-code truncation +
Windows PATH + fragile test (PR#747 review)"):
(1) exit-code truncation → new `exit_code_from_child` uses `u8::try_from` (out-of-range/signal-killed →
    FAILURE, never wrapped); unit test pins 256/257/-1 rejected.
(2) Windows PATH fallback → `bin_name()` appends `.exe` on windows for the fallback.
(3) fragile test → `smith_cli` now asserts only the cdz-side no-clap-misparse contract, tolerating a
    co-present cdz-smith's own output.
Verified all three present on trunk. Marking RESOLVED.
