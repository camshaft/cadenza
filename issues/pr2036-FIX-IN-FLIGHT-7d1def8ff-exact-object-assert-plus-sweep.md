# PR #2036 review — cdz-agent-host/src/admin.rs (v-agent-harness-host) — MERGED — test-precision [VERIFIED, LOW]

https://github.com/camshaft/cadenza/pull/2036 (admin `metrics` command — host-wide metrics JSON). Copilot
(id 3713007359) flags a substring JSON assertion that can false-positive.

## `metrics_command_returns_the_host_metrics_json` asserts `json.contains("\"installed\":1")` — a substring match that also matches `"installed":10`/`:12`/… (Copilot, admin.rs:658) — test-precision [VERIFIED]
> The new test asserts the installed session count via `json.contains("\"installed\":1")`, which can
> produce false positives (e.g., it would also match `"installed":10` or `"installed":12`). Since this
> JSON shape is a stable contract, it's safer to parse and assert the numeric fields explicitly so the
> test fails on regressions.

VERIFIED in the diff (admin.rs:665): the test does `assert!(json.contains("\"installed\":1"), …)`.
`str::contains` is a substring test, so `"installed":10`, `"installed":123`, etc. all satisfy it — the
assertion passes for any installed-count ≥ 1 that starts with `1`, not specifically 1. A regression that
mis-counted installs to 10+ (starting with `1`) would slip through. Since `host_metrics_json` is a stable
contract (the doc pins the exact shape `{"sessions":{"installed":3,…}}`), the test should pin the exact
value. LOW/test-precision. Fix per Copilot: assert on the delimited token `"installed":1,` (with the
trailing comma/brace so `:10` can't match) OR — cleaner — parse the JSON and assert the numeric field
(`installed == 1`). Same substring-vs-exact class worth watching across the metrics tests (the other
`contains` asserts on this JSON have the same latent fragility if any count can reach a `1`-prefixed
multi-digit). v-agent-harness-host owns cdz-agent-host/src.
