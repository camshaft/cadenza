# PR #2155 review — cdz-agent-host (v-agent-harness-host) — OPEN — doc-accuracy/security-posture [VERIFIED, LOW] (batched, 4-5 sites)

https://github.com/camshaft/cadenza/pull/2155 (prometheus PULL metrics-export backend — hand-rolled scrape
server, loopback-default, metrics-export-prometheus feature). Copilot 4 inline — ALL THE SAME finding
across 4 files → batched. Network-bind backend (the #2112 IPv6-bind lineage), so posture accuracy matters.

## the docs across 4-5 sites say the scrape endpoint "binds LOOPBACK by default", but there is NO automatic loopback default in code: `bind: String` has no `#[serde(default)]`, config validation REJECTS an empty bind, and a non-loopback bind is only WARNED (not defaulted/rejected) → the docs overstate a safe-by-default posture that's actually configure-required + warn-only (Copilot, config.rs:169 · daemon.rs:421 · Cargo.toml:77 · export.rs:398 & :482) — doc-accuracy/security-posture [VERIFIED, LOW]
> [config.rs:169] "Bound to LOOPBACK by default", but the `bind: String` field has no serde default and
> config validation rejects an empty bind … docs should describe loopback as the recommended/default
> *configuration*, not an automatic default.
> [daemon.rs:421] the bind address comes directly from config and there is no automatic loopback default
> in code.
> [Cargo.toml:77] the implementation binds whatever address is configured (and requires a non-empty bind).
> [export.rs:398 & :482] `run_prometheus_scrape_server` always binds the explicit `bind_addr` it is given
> (and config validation requires a non-empty bind).

VERIFIED in the #2155 diff — the four doc sites all say "binds/bound LOOPBACK by default" (Cargo.toml:16,
daemon.rs:182, config.rs:228, export.rs:282), but the CODE contradicts "default":
- `bind: String` (diff:235) has NO `#[serde(default)]` — contrast `prefix` immediately after it (diff:237)
  which DOES. So `bind` is a required field; there's no defaulting to `127.0.0.1:PORT`.
- validation REJECTS an empty bind: `MetricsTarget::Prometheus { bind, .. } if bind.trim().is_empty() =>`
  (diff:256) → error. So the operator MUST explicitly set `bind`.
- a non-loopback bind is only WARNED, not prevented: `is_loopback_bind` (diff:317) gates a `tracing::warn`
  in `run_prometheus_scrape_server` (diff:341-345) — "bound to a NON-loopback address with no auth …".
So the ACTUAL posture is: `bind` is required (no default), loopback is the RECOMMENDED value, and a
non-loopback bind is warn-only (still binds). "binds LOOPBACK by default" is wrong on two counts — there's
no default at all, and non-loopback isn't blocked. LOW (doc-accuracy with a security-posture edge: a reader
could believe the endpoint is safe-by-default when it's configure-required + warn-only; the warn is good,
but the "default" wording undersells that an operator can bind 0.0.0.0 and only get a log line). Fix per
Copilot (all sites): reword to "loopback is the RECOMMENDED/typical configuration for this no-auth endpoint;
a non-loopback bind is warned at boot" — describe the posture as guidance + a warn, not an automatic
default. v-agent-harness-host owns cdz-agent-host. PR OPEN → all foldable pre-merge. (The bind SECURITY
itself — warn-on-non-loopback — is sound and matches the concierge posture ruling; only the "by default"
wording overstates. No code bug.)
