# PR#901 review comments — provider-cache: non-atomic write (corruption risk) + test picks any .wasm + provider_cache_dir doc says None but never returns it (v-cdz-tooling)

Mirrored from GitHub PR#901 review comments (Copilot), ids `3677648876` (main.rs:4044, atomic-write) +
`3677648918` (test_per_file_cli.rs:245, filter) + `3677648942` (main.rs:4079, doc). All from `fb5e0a7f5`
"cdz test: cross-invocation provider cache" → v-cdz-tooling (`cdz` CLI + its test).

## Comment 1 (verbatim) — main.rs:4044, cache-corruption risk (most substantive)

- (id 3677648876, cdz/src/main.rs:4044) "Persisting the cached provider with `std::fs::write` directly to
  the final path can leave a truncated/partial file if the process crashes mid-write, creating persistent
  cache corruption. Since the cache is meant to self-heal and be safe under interruption, prefer an atomic
  write pattern (write to a temp file in the same dir, then rename)."

### Liaison verification (confirmed on trunk 1b69ac7f0)

main.rs:4043: `let _ = std::fs::write(dir.join(format!("{key}.provider.wasm")), bytes);`. The "best-effort
persist" comment (4041-4042) addresses a WRITE FAILURE (full/RO FS → degrades to no-cache) but NOT a
PARTIAL write: a crash/kill mid-`fs::write` leaves a truncated `{key}.provider.wasm` at the FINAL
content-addressed path. On the next run that key is a cache HIT that loads a corrupt/truncated component —
persistent corruption that does NOT self-heal (the key still "exists"), worse than a miss. Fix (Copilot's,
standard): write to a temp file in the SAME dir then `rename` (atomic on POSIX) — a crash leaves only the
temp, the final path is either absent (clean miss) or complete. Robustness — worth doing for a
crash-safe self-healing cache.

## Comment 2 (verbatim) — test_per_file_cli.rs:245, flaky file pick

- (id 3677648918, cdz/tests/test_per_file_cli.rs:245) "`cached_provider()` currently returns the first
  `.wasm` file in the cache dir. If the cache directory ever contains other `.wasm` files (e.g., future
  cache entries or unrelated artifacts), this test could corrupt the wrong file and become flaky. Since
  the production cache name is `{hash}.provider.wasm`, filter specifically for the provider suffix."

### Liaison verification (confirmed on trunk 1b69ac7f0)

test_per_file_cli.rs:241-244: `read_dir(&cache)….find_map(|e| (p.extension()==Some("wasm")).then_some(p))`
— returns the FIRST `.wasm`. The test later corrupts/rewrites this file to prove self-heal; if the cache
dir ever holds >1 `.wasm` (multiple cache entries, or a shared store dir), it may hit the wrong one →
flaky. Fix: filter on the `.provider.wasm` suffix (the production name from main.rs:4043). Test-robustness.

## Comment 3 (verbatim) — main.rs:4079, doc says None but always Some

- (id 3677648942, cdz/src/main.rs:4079) "The `provider_cache_dir` doc comment says it can return `None`
  if no store is resolvable, but the function always returns `Some(...)` (it falls back to
  `target/cadenza-store/providers`). This makes the docs misleading for callers and future maintainers."

### Liaison verification (confirmed on trunk 1b69ac7f0)

`provider_cache_dir` (4080-4088): doc (4077) says "`None` if no store is resolvable (⇒ no caching…)", but
the body returns `Some($CDZ_PROVIDER_CACHE)` or unconditionally `Some(default_store().join("providers"))`
— it NEVER returns `None`. Doc/code mismatch. Fix: either drop the `None` clause from the doc (the return
type could even become non-Option), or make the fallback genuinely fail to `None` when no store resolves,
whichever matches intent. Doc-only unless the owner wants the None path to actually exist.

Owner: **v-cdz-tooling** (`cdz` CLI + test, provider cache `fb5e0a7f5`). Comment 1 = crash-safe atomic
write (robustness), comment 2 = test suffix-filter, comment 3 = doc/return reconcile. Bundled.
