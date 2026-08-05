# PR #2253 review — cdz-kernel/src/wasm_host.rs (v-agent-harness) — OPEN — 1 API-parity (MED) + 1 error-UX (LOW-MED) + 1 doc (LOW) + 1 test-coverage (LOW-MED) [VERIFIED]

https://github.com/camshaft/cadenza/pull/2253 (AsyncComponentReducer composes §23 deps per-fold — the
async twin of the sync ComponentReducer path I reviewed in #2203/#2242). Copilot 4 inline — the async twin
lags the sync API/error/coverage in several ways.

## `AsyncComponentReducer` doesn't retain/expose the declared dep set (no `deps: Vec<ComponentDep>` field, no `deps()`/`resolve_deps()`), unlike sync `ComponentReducer` → async callers can't run the §23 discover→resolve-from-CAS→attach flow without re-parsing component metadata (Copilot, wasm_host.rs:1952) — API-parity [VERIFIED, MED]
> `from_component_bytes` detects whether the component declares §23 deps, but `AsyncComponentReducer`
> doesn't retain or expose that declared dep set (unlike `ComponentReducer::deps()`/`resolve_deps()`) …
> Consider mirroring the sync API by storing `deps: Vec<ComponentDep>` … and adding `deps()` +
> `resolve_deps()` helpers so async and sync reducers can be wired uniformly.
VERIFIED against source. Sync `ComponentReducer` stores `deps: Vec<ComponentDep>` (wasm_host.rs:217) +
exposes `deps()` (:1482) + `resolve_deps()` (:1500, does the CAS lookup). The async twin (#2253 diff:33-53)
COMPUTES `let deps = declared_deps(...)` (diff:82) only to decide `instance_pre = None`, then stores
`resolved_deps` + `component_store` — but adds NO `deps: Vec<ComponentDep>` field and NO
`deps()`/`resolve_deps()` accessors. So an async caller can't discover the declared deps to resolve them
from the CAS + attach — the very §23 flow the PR enables — without re-parsing the component elsewhere. MED
API-parity (the async path is meant to MIRROR sync but omits the dep-discovery half of the API). Fix per
Copilot: store `deps: Vec<ComponentDep>` on `AsyncComponentReducer` + add `deps()` + `resolve_deps()`
(async) mirroring the sync API, so both wire uniformly.

## the dep-compose branch (`instance_pre == None`) with `resolved_deps` never attached fails with a generic wasmtime/linker "missing imports" error → emit an early actionable "resolve+attach via with_resolved_deps" error (Copilot, wasm_host.rs:2056) — error-UX [VERIFIED, LOW-MED]
> … if `resolved_deps` was never attached, instantiation will fail with a generic wasmtime/linker error
> about missing imports. Consider emitting an early, actionable error telling the caller to resolve and
> attach deps via `with_resolved_deps` (and `with_component_store` when needed) before folding.
VERIFIED-plausible: same failure-mode class as my #2203 c4 ("declares no resolved dep" wording) + #2244
(clean-skip). A dep-bearing async reducer folded without `with_resolved_deps` hits a deep generic linker
error instead of a targeted message. LOW-MED. Fix: early-check `resolved_deps.is_empty() && !deps.is_empty()`
→ actionable error naming `with_resolved_deps`/`with_component_store`.

## the struct doc says `instance_pre.is_some()` iff dependency-free, but `with_resolved_deps` can force `instance_pre = None` even for dep-free components (Copilot, wasm_host.rs:1906) — doc [VERIFIED, LOW]
VERIFIED: doc "So `instance_pre.is_some()` iff dependency-free" (diff:43), but c1 notes `with_resolved_deps`
can null `instance_pre` even for a dep-free component (to force deps composing in the fold's store). So the
iff is not strict. LOW/doc. Fix: reword to "`instance_pre.is_some()` when the fast path applies (dep-free
AND no forced per-fold compose)".

## no E2E coverage of the async dep-bearing path (linker clone + compose_dep_into_linker + instantiate_async) (Copilot, async_component_reducer_e2e.rs:108) — test-coverage [VERIFIED, LOW-MED]
VERIFIED: the PR adds the async dep-compose branch but the e2e only documents it — no test exercises the
dep-bearing path. Same coverage-gap shape as my #2244 (b1 e2e) + #2184 (genesis happy-path). LOW-MED. Fix:
a focused test with a small synthetic WAT reducer + stub dep through the dep-compose + `instantiate_async`
branch, so the new path is regression-guarded.

The API-parity (c3) is the one that matters — the async twin should mirror sync's dep-discovery API.
v-agent-harness owns cdz-kernel/src. PR OPEN → all foldable pre-merge.
