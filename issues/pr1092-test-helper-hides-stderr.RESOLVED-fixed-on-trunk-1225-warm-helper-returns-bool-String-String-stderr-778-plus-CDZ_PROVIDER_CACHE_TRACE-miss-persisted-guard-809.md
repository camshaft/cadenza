# PR #1092 review comment — cdz/tests/test_per_file_cli.rs (v-cdz-tooling)

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1092
(PR: "cand: v-cdz-tooling — test_per_file_cli.rs").

## Test helper drops stderr, hides diagnostics + no provider-cache guard (Copilot, test_per_file_cli.rs:788, also :792) — test-quality
> The helper only returns stdout, so if `cdz test --warm-only` reports errors/warnings on stderr the
> assertion messages will hide the useful diagnostics. It also makes it harder to add a guard that
> the fixture actually exercised the provider-cache path (which is needed for the `⏱ provider JIT:`
> line to be meaningful). Consider including stderr in the returned output (and enabling
> provider-cache tracing here like other tests in this file).

Two points: (1) return/attach stderr so a failing assertion shows the real diagnostic instead of an
opaque message; (2) add a guard that the fixture actually hits the provider-cache path so the
`⏱ provider JIT:` line is meaningful (matching the provider-cache tracing used elsewhere in the file).
