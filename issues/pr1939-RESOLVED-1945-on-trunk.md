# PR #1939 review — cdz-agent-host/src/config.rs (v-agent-harness-host) — MERGED — test-precision [VERIFIED]

https://github.com/camshaft/cadenza/pull/1939 — MERGED 2026-08-04T04:01:31Z (daemon config INFRASTRUCTURE-
only; dropped [[session]]). Copilot (id 3709305238) flags the `deny_unknown_fields` test asserts only the
generic wrapper. Composes with the surviving #1935 #147 clarity nit (comment 3709251724) — SAME root
cause: `from_toml_str` collapses every failure into one `"invalid TOML"` string.

## `an_unknown_key_is_rejected_not_ignored` asserts only `err.0.contains("invalid TOML")` — passes for ANY parse/deser failure, so it doesn't prove the UNKNOWN KEY was rejected (Copilot, config.rs:232) — test-precision [VERIFIED]
> This test claims to validate `deny_unknown_fields` behavior, but it only asserts that parsing failed
> with the generic "invalid TOML" wrapper. That would also pass for unrelated parse/deserialization
> failures, so it doesn't actually prove the unknown key was rejected (as opposed to some other error).

VERIFIED on trunk. `from_toml_str` (config.rs:128) does `toml::from_str(text).map_err(|e|
ConfigError(format!("invalid TOML: {e}")))` — EVERY `toml::from_str` failure (syntax error, type mismatch,
missing required field, unknown field) gets the same `"invalid TOML: {e}"` prefix. The test (config.rs:230)
feeds `"typo_field = \"oops\"\n"` and asserts `err.0.contains("invalid TOML")` — which would ALSO pass if
the config failed for any other reason, so it does not prove `deny_unknown_fields` fired on `typo_field`.
A future refactor that broke unknown-field rejection (e.g. dropped a `#[serde(deny_unknown_fields)]`) but
left some OTHER parse error on that input would keep the test green. LOW/test-precision.

Fix (serves BOTH this and #1935 #147): serde's unknown-field error message names the offending key
(`unknown field \`typo_field\`, expected one of …`), so assert on the specific substring —
`assert!(err.0.contains("typo_field") || err.0.contains("unknown field"), "{err}")`. Even better, do it
alongside the #1935 #147 fix that differentiates the wrapper (distinguish TOML-syntax vs
schema/deny-unknown errors in `from_toml_str`), then assert on the schema-error class. That single change
tightens the test AND improves operator diagnostics (typo vs syntax) in one pass. v-agent-harness-host owns
cdz-agent-host/src.
