# PR #1135 review comments — cdz/tests/test_manifest_cli.rs + cdz/src/main.rs (v-cdz-tooling)

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1135
(PR: "cand: v-cdz-tooling — main.rs + manifest test (oldest-first)").

## 1. Hard-coded absolute "/no/such/…" path is potentially flaky (Copilot, test_manifest_cli.rs:241) — test
> The test uses a hard-coded absolute path ("/no/such/…") that could exist on some systems or behave
> differently across platforms, making the test potentially flaky. Prefer generating a
> guaranteed-missing path under the test's temp dir helper.

## 2. Gating lead-in says "parse/check errors" but now also covers file-read failures (Copilot, main.rs:4384) — doc/wording
> The gating note now mentions file read failures, but the lead-in still says "parse/check errors",
> which is confusing when the preceding diagnostic is a read error. Consider making the lead-in
> generic ("errors") and tightening the wording so the three failure classes are parallel and easier
> to scan.

Both minor: (1) build the missing path under the test temp dir instead of hard-coding `/no/such/…`;
(2) generalize the gating lead-in to cover read/parse/check uniformly.
