# PR #1613 review comments — implementation/seed/crates/cdz-agent-host/tests/capability_manifest_e2e.rs (v-agent-harness-host)

Mirrored from https://github.com/camshaft/cadenza/pull/1613 (PR: "cdz-agent-host: e2e for
capability-manifest projection over REAL executors + a live Cedar policy (host-discovery I2/I3)").
Two LOW nits.

## 1. Module doc uses first-person "my Clock/Http/Model" vs neutral sibling tests (Copilot, capability_manifest_e2e.rs:7) — doc/style
> The module doc comment uses first-person phrasing ("my Clock/Http/Model") and heavy emphasis. Other
> docs/tests in this crate use neutral, third-person wording (e.g. `cedar_authz_e2e`). Keeping docs
> author-agnostic reads better.

VERIFIED: the `//!` module doc reads "the REAL host wiring — my Clock/Http/Model executors …". Sibling
test `cedar_authz_e2e` uses neutral third-person. Reword "my Clock/Http/Model" → "the Clock/Http/Model"
(or "the host's"). LOW/style.

## 2. `policy_component_bytes()` duplicated verbatim from cedar_authz_e2e.rs (Copilot, capability_manifest_e2e.rs:35) — dedup
> `policy_component_bytes()` appears to be duplicated verbatim from `tests/cedar_authz_e2e.rs`.
> Consider factoring it into a shared `tests` helper (e.g. `tests/util.rs` or `tests/common/mod.rs`) to
> keep the skip/read behavior in one place.

VERIFIED byte-identical: both `capability_manifest_e2e.rs:37` and `cedar_authz_e2e.rs:30` define the
same `fn policy_component_bytes() -> Option<Vec<u8>>` (read `CEDAR_POLICY_COMPONENT`, skip-if-unset,
panic-on-unreadable). Factor into `tests/common/mod.rs` (the idiomatic shared-helper spot for Rust
integration tests) so the skip/read contract lives in one place. LOW. (Note: these are integration
tests in `tests/`; the operator's prefer-unit-tests directive is about NEW coverage — the dedup here is
fine either way, just centralize the existing helper.)
