# PR #2001 review — cdz-agent-host/src/config.rs (v-agent-harness-host) — OPEN — doc + test-naming [VERIFIED, LOW] (batched)

https://github.com/camshaft/cadenza/pull/2001 (multi-backend [observability] metrics config). Copilot 2
inline, both LOW accuracy nits. PR still OPEN → foldable pre-merge.

## doc references a `metrics` cargo feature that doesn't exist (crate has only `live-net` + `admin`) (Copilot, config.rs:70) — doc-accuracy [VERIFIED]
> The doc comment references a `metrics` cargo feature, but this crate's Cargo features only define
> `live-net` and `admin` (no `metrics`). This makes the config docs misleading and may confuse operators
> about how/when metrics backends are compiled.

VERIFIED: the new doc (config.rs:70) says the metrics backend "lands behind a `metrics` cargo feature (OFF
by default …)", but cdz-agent-host's `[features]` (Cargo.toml:29) defines ONLY `live-net` and `admin` —
there is no `metrics` feature (and no `cfg(feature = "metrics")` anywhere). So the doc promises a
compile-gate that doesn't exist; an operator reading it would look for a `--features metrics` that isn't
there. LOW/doc-accuracy. Fix: either drop the `metrics`-feature sentence (describe the config as parsed
always, wired per the observability daemon slice — matching how `[log]`/`[blob]` are staged), OR if a
`metrics` feature IS intended, add it to `[features]`. Given the config is parse-ahead-of-wiring (like the
#1981 `[blob]` staging), dropping the nonexistent-feature claim is the honest fix now.

## `observability_target_missing_endpoint_is_rejected` uses `endpoint = ""` (EMPTY), not a MISSING field (Copilot, config.rs:419) — test-naming [VERIFIED]
> The test name says "missing_endpoint" but the config includes an `endpoint` key with an empty string.
> Either omit the field to test the "missing" case, or rename the test to reflect what it actually checks.

VERIFIED (diff): the test feeds `[[observability.target]] kind = "s2n-quic-dc" endpoint = ""` — an EMPTY
string, and the validation it exercises is `t.endpoint.trim().is_empty()`. That's the empty-endpoint case,
NOT missing: since `endpoint: String` is required, OMITTING it would be a TOML deserialize error ("missing
field `endpoint`") — a different code path (deser, not `validate`). So the name "missing_endpoint"
misdescribes what's tested. LOW/test-naming. Fix per Copilot: rename to
`observability_target_EMPTY_endpoint_is_rejected` (matches the `trim().is_empty()` check it actually pins),
OR add a separate test that OMITS `endpoint` to cover the missing/deser case too (both are worth pinning —
empty via `validate`, missing via deser). v-agent-harness-host owns cdz-agent-host/src. Both foldable while
#2001 is open.
