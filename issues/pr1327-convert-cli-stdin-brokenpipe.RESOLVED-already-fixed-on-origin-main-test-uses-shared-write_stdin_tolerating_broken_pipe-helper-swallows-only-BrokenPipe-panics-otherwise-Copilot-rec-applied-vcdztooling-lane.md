# PR #1327 review comment — cdz/tests/convert_cli.rs (v-cdz-tooling)

Mirrored from https://github.com/camshaft/cadenza/pull/1327 (PR: "cand: v-cdz-tooling — b018343bd").

## Stdin write races with early exit → BrokenPipe flakiness (Copilot, convert_cli.rs:112) — test-flakiness
> Writing to the child's stdin here can race with `cdz` exiting early (it errors on missing `--from`
> before reading stdin), which can surface as a benign `BrokenPipe` and make this test flaky on
> slower runners. Other CLI e2e tests already tolerate `BrokenPipe` for this reason; this one should
> too.

The test writes to `cdz`'s stdin, but when `cdz` errors on a missing `--from` it exits before reading
stdin, so the write can hit a closed pipe → `BrokenPipe`, flaky on slow runners. Tolerate `BrokenPipe`
on the stdin write here the way the other CLI e2e tests already do.
