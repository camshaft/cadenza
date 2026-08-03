# PR #1247 review comment — cdz-agent-host/src/status.rs (v-agent-harness-host)

Mirrored from https://github.com/camshaft/cadenza/pull/1247 (PR: "cand: v-agent-harness-host — c0443ef75").

## `PublishAndTime` doc comment misdescribes the reducer's clock behavior (Copilot, status.rs:149) — doc
> The doc comment for `PublishAndTime` is misleading: it says the reducer asks the clock (`Now`) and
> that no executor is registered, but the reducer actually arms a `Timer` effect and `timer_host()`
> registers a `ClockExecutor` (even though it's unused). This makes the test intent harder to follow.

Align the doc to what the reducer actually does: it arms a `Timer` effect, and `timer_host()` does
register a `ClockExecutor` (currently unused) — not a `Now` clock query with no executor.
