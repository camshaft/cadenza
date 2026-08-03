# PR #1633 review comment — cdz-run/src/cli.rs (v-cdz-tooling) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1633 (MERGED). Follow-on edge to the #1623 --store-scopes-NFC fix.

## `runtime_cache_dir` computed before peer-runtime resolution — peer-induced runtime not store-scoped (Copilot, cli.rs:132) — behavior [VERIFIED]
> `runtime_cache_dir` is computed before peers are loaded and before the later "peer needs the runtime"
> resolution (~184-198). If the consumer doesn't require a runtime but a peer does, `runtime` becomes
> `Some(...)` while `runtime_cache_dir` stays `None`, so an explicit `--store` won't scope NFC
> resolution/caching for that peer-induced runtime.

VERIFIED against the merged code: `runtime_cache_dir` is computed at cli.rs:134 as `if runtime.is_some()
&& cli.runtime.is_none()`, using the CONSUMER's `runtime` (resolved at :125). The peer-induced runtime is
only resolved LATER at :191-207 (`None if !peers.is_empty()` → loads a peer's `required_runtime`). So when
the consumer needs no runtime but a peer does: `runtime` ends up `Some(...)` (via the peer path) while
`runtime_cache_dir` was already fixed to `None` earlier → an explicit `--store` does NOT scope NFC
resolution/caching for that peer-induced runtime. This is the SAME class as #1623 (explicit --store must
scope NFC), on the peer path. Fix per Copilot: treat "has runtime" as true when peers are present (or
recompute runtime_cache_dir after the peer-runtime resolution). MED, narrow (needs consumer-no-runtime +
peer-with-runtime + explicit --store), fix-forward. RECOMMEND v-cdz-tooling confirm — this is adjacent to
your just-landed b872a1e46 fix.
