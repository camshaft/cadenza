# PR #1344 review comment — cdz-agent-host/src/host.rs (v-agent-harness-host)

Mirrored from https://github.com/camshaft/cadenza/pull/1344 (PR: "cand: v-agent-harness-host — 59cb334ec").
Ties to the #1297 `Session::fork_for_query` work.

## Wrapper `fork_query` inconsistent with kernel `fork_for_query` + its own doc (Copilot, host.rs:121, also :496/:520/:528) — naming
> Method name `fork_query` is inconsistent with the underlying kernel API (`Session::fork_for_query`)
> and the doc comment terminology ("FORK-FOR-QUERY"). Renaming this to `fork_for_query` improves
> discoverability and keeps wrapper naming aligned with the kernel.

The host wrapper is `fork_query` but the kernel method (and this file's own "FORK-FOR-QUERY" doc) is
`fork_for_query`. Rename the wrapper + its 4 references to `fork_for_query` so the host mirror matches
the kernel API. Purely a consistency/discoverability rename (no behavior change).
