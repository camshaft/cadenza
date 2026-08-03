# PR #1806 review comments — cdz-kernel/src/{executor,authz,wasm_host,reducer}.rs (v-agent-harness) — OPEN

https://github.com/camshaft/cadenza/pull/1806 (trait-rename T2 kernel-side).

## 5x "beat T2/T3" migration-beat references in durable doc comments (Copilot, executor.rs:118, authz.rs:79, wasm_host.rs:721+1058, reducer.rs:154) — doc/durability
> These doc/migration comments reference internal rollout beats ("beat T2" / "beat T2/T3"), which are
> likely to become stale. Reword to describe the stable behavior (this impl defines the un-suffixed
> method) rather than the migration sequence.
The recurring durability pattern (same as #1554/#1622/#1664/#1687/#1717) — migration-beat tags in durable
code comments across 5 sites (executor:118, authz:79, wasm_host:721+1058, reducer:154). Reword each to the
stable statement (e.g. "defines the un-suffixed trait method; the legacy _async name forwards during the
rename window") without the "beat T2/T3" sequencing. LOW/doc, batch the 5 into one reword. Fix-forward.
