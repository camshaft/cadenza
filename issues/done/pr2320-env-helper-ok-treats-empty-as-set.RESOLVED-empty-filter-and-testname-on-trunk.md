# PR #2320 review — cdz-agent-host/src/host.rs (v-agent-harness-host) — OPEN — robustness [VERIFIED, LOW-MED]

https://github.com/camshaft/cadenza/pull/2320 (extract shared env-gate helper for the real-reducer E2Es —
the fix-forward for MY #2315 DRY finding; branch cand/v-agent-harness-host-6d6c08ddbe96). Copilot 1 inline
(id 3724950654, host.rs:931). NOTE: this is a follow-on finding ON the helper my #2315 relay prompted — a
one-layer-deeper residual on the fix.

## `require_reducer_and_store_or_skip` uses `std::env::var(...).ok()`, treating an empty-but-present var as SET (`CDZ_STORE=""` → `ComponentStore::open("")` resolves to CWD, masking a misconfigured CI env); also fails on non-UTF8 paths; panic msgs lack `test_name` (Copilot, host.rs:931) — robustness [VERIFIED, LOW-MED]
> `require_reducer_and_store_or_skip` uses `std::env::var(...).ok()`, which treats an empty-but-present env
> var as set (e.g. `CDZ_STORE=""`). For `CDZ_STORE`, this becomes `ComponentStore::open("")` later, which
> resolves relative to the current working directory and can mask a misconfigured CI environment. It also
> fails on non-UTF8 paths. Consider switching to `var_os`, rejecting empty values explicitly, and including
> `test_name` in the panic messages.

VERIFIED in the #2320 diff: the extracted helper does `let reducer_path = std::env::var(reducer_env).ok();
let store_dir = std::env::var("CDZ_STORE").ok();` (diff:40-41) — the same `.ok()` the old inline sites used,
now single-sourced. `.ok()` maps a present-but-empty var to `Some("")` (only Err→None), so `CDZ_STORE=""`
counts as SET and flows to `ComponentStore::open("")` → CWD-relative resolution, silently masking a
misconfigured CI env instead of skipping/failing loud. Plus `var` (not `var_os`) errors on non-UTF8 paths,
and the skip/panic messages don't name the test. LOW-MED / robustness — a hardening of the very helper the
extraction created (good place to fix it once, for all E2Es). Fix per Copilot: `var_os` + explicit
reject-empty (treat `Some("")` as unset → skip) + thread `test_name` into the messages.

Note for v-ah-host: this is exactly the "extract-then-harden" win — since #2315's helper single-sources the
skip contract, fixing it here fixes all E2Es at once (the drift risk my #2315 flagged, now with correct
empty/UTF8 semantics). Same-crate test-only, no cdz-kernel seam. v-agent-harness-host owns cdz-agent-host. PR
OPEN → foldable pre-merge; entirely their call.
