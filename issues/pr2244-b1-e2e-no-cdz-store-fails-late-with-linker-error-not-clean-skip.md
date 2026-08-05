# PR #2244 review — cdz-kernel/tests/reducer_cadenza_b1_e2e.rs (v-agent-harness) — OPEN — test-robustness [VERIFIED, LOW-MED]

https://github.com/camshaft/cadenza/pull/2244 (b1 e2e attaches the component store so the transitive nfc
compose resolves — the §23 / b1-e2e work my #2210 store findings + #2242 manifest-key underpin). Copilot 1
inline.

## the test attaches ComponentStore only when `CDZ_STORE` is set, but the runtime's transitive bare `cadenza:nfc/normalize` import then fails LATE with an opaque linker/compose error if `CDZ_STORE` is unset → skip (or fail-early with a targeted message) so the failure mode is clear + deterministic (Copilot, reducer_cadenza_b1_e2e.rs:76) — test-robustness [VERIFIED, LOW-MED]
> The test only attaches a ComponentStore when `CDZ_STORE` is set, but the value-heap runtime's
> transitive bare import (e.g. `cadenza:nfc/normalize`) will still fail later with a linker/compose error
> if `CDZ_STORE` is missing. Consider skipping the test (or failing early with a targeted message) when
> `CDZ_STORE` is unset so the failure mode is clear and deterministic.

VERIFIED in the #2244 diff: the store is attached via `if let Ok(store_dir) = std::env::var("CDZ_STORE") {
reducer = reducer.with_component_store(ComponentStore::open(&store_dir, …)) }` (diff:17-19). The test's OWN
comment (diff:12-16) describes the failure: "Without CDZ_STORE, the transitive compose can't find
cadenza:nfc/normalize → 'imports cadenza:nfc/normalize, not found in linker'. Only when CDZ_STORE is wired
(the nix path)…". So when `CDZ_STORE` is UNSET, the test proceeds WITHOUT the store, and the b1 run later
hits the runtime's bare `nfc/normalize` import → an opaque linker/compose error deep in the run, NOT a
clean skip. LOW-MED/test-robustness (a developer running `cargo test` locally without `CDZ_STORE` gets a
confusing mid-run linker failure instead of "skipped: needs CDZ_STORE"). Fix per Copilot: at the top, if
`CDZ_STORE` is unset, SKIP with a message (like the other env-gated e2es — CDZ_REDUCER_COMPONENT /
CEDAR_POLICY_COMPONENT pattern) OR fail-early with a targeted "b1 e2e requires CDZ_STORE for the transitive
nfc compose" message — so the failure mode is deterministic + self-explaining. (Consistency: the codebase's
other e2es env-gate + skip-when-unset; this one should match.) v-agent-harness owns cdz-kernel/tests. PR
OPEN → foldable. (This is the b1 e2e that my #2210 SHA-256 store fix + #2242 transitive-compose unblock —
worth the env-gating being clean since it's the first real reducer end-to-end.)
