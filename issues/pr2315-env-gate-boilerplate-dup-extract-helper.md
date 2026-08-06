# PR #2315 review — cdz-agent-host/src/host.rs (v-agent-harness-host) — OPEN — maintainability/DRY [VERIFIED, LOW]

https://github.com/camshaft/cadenza/pull/2315 (full-agent-loop E2E — real effect-emitting reducer_b2 runs an
Http turn through the host; branch cand/v-agent-harness-host-785ce68459dc). Copilot 1 inline (id 3724833212,
host.rs:1054).

## the env-gated skip + both-or-fail-loud boilerplate (`REDUCER_CADENZA_B2_COMPONENT` + `CDZ_STORE`) is DUPLICATED with the genesis async E2E above → the two E2Es can drift (mismatched skip semantics / error messages); extract a `require_env_pair_or_skip` helper (Copilot, host.rs:1054) — DRY [VERIFIED, LOW]
> The env-gated skip + both-or-fail-loud boilerplate is duplicated with
> `real_genesis_reducer_folds_setup_events_through_the_host_async_path` above. This duplication makes it easy
> for the two E2Es to drift ... Consider extracting a small helper in the tests module (e.g.
> `fn require_env_pair_or_skip(test_name, a, b) -> Option<(String, String)>`) and reusing it here and in the
> genesis E2E.

VERIFIED in the #2315 diff: the new test opens with the same env-gate contract as the genesis async E2E —
`let reducer_path = std::env::var("REDUCER_CADENZA_B2_COMPONENT").ok(); let store_dir =
std::env::var("CDZ_STORE").ok();` then a both-or-fail-loud skip (the doc even says "same contract as the
genesis + b1/b2 kernel e2es"). So the skip semantics + error wording are copy-duplicated across at least two
host E2Es (and echo the kernel e2es' pattern). LOW / maintainability — behavior is correct; the risk is
future drift when the env contract changes (one test updated, the other not). Fix per Copilot: extract a
tests-module helper `require_env_pair_or_skip(test_name, a, b) -> Option<(String, String)>` and reuse it in
both host E2Es, single-sourcing the skip contract.

Notes for v-ah-host: (a) same-crate TEST-only change — does NOT touch the strict cdz-kernel seam. (b) aligns
with the operator's prefer-crate-unit-tests lean (a shared test helper is fine). (c) entirely v-ah-host's
call — a cheap DRY win, not a correctness issue. v-agent-harness-host owns cdz-agent-host. PR OPEN → foldable
pre-merge.
