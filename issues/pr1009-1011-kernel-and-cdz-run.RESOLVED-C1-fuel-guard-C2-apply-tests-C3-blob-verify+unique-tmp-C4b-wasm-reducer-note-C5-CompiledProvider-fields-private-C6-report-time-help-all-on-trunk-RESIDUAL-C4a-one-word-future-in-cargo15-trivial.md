# PR#1009/#1010/#1011 — cdz-kernel (v-agent-harness) + cdz-run/cdz-test (v-cdz-tooling)

## v-agent-harness (cdz-kernel) — gate = cdz-kernel own cargo test+clippy, NOT xtask check

### wasm_host.rs:162 (PR#1009, id 3695833535) ⚠ RUNAWAY-GUEST HANG
blame `55d851414` "ComponentReducer::apply — drive a fold through the wasm guest (§19b)". `apply` builds
`wasmtime::Store::new(&self.engine, …)` and `call_apply` with a DEFAULT engine + NO execution bounds
(fuel / epoch deadline). A buggy or malicious reducer that loops forever HANGS the kernel process — and
the new `Trap` variant doesn't help (a non-yielding guest never traps). Enable fuel OR epoch interruption
+ a per-apply limit so a runaway guest fails fast (fold failure the driver already handles). Robustness /
liveness — the kernel drives UNTRUSTED guest fold code.

### wasm_host.rs:144 (PR#1009, id 3695833544) — missing test
`apply` (instantiate + call guest + surface mutated KV) has NO test. Add one: a minimal reducer component
asserting (1) the `kv` import is callable from the guest and (2) KV mutations return to the host.

### blob.rs:118 (PR#1011, id 3695900540) ⚠ trust-existing + temp-collision
blame `1c44df3d9` "CAS blob store". `DiskBlobStore::put`: `if path.exists() { return Ok(hash); }` — trusts
an existing blob file as valid WITHOUT re-verifying, but `get` DOES detect corruption (self-verify hash) —
so `put` reports success while a later `get` fails on the same corrupt/tampered file (inconsistent). AND
the temp name is `<hash>.tmp` (hash-only) — two concurrent writers (processes/stores at the same root)
collide on the same temp path → clobber or spurious failure. Fix: `put` should VERIFY an existing file
(rewrite if invalid), and use a UNIQUE temp name (pid/time suffix) + handle `rename` dest-exists portably
(cf the PR#903 rename-over-existing class). Correctness/robustness of the content-addressed store.

### Cargo.toml:33 ×2 (PR#1010, ids 3695867211 + 3695867228) — stale comments
(a) header says the seed workspace is isolated so the main tree doesn't pay for the kernel's "(FUTURE)
async/wasmtime tree" — but wasmtime is now a CORE (non-optional) dep; drop "future". (b) the `wat`
dev-dep comment references "wasm-reducer tests" + a `#[cfg(feature = "wasm-reducer")]` path, but the
`wasm-reducer` feature was REMOVED this PR; update so it doesn't point at a non-existent gate.

## v-cdz-tooling (cdz-run / cdz test)

### cdz-run/src/lib.rs:541 (PR#1009, id 3695833523) — API leak
blame `e01dcdd65` "cdz test: JIT the shared provider ONCE per project". `CompiledProvider` exposes
wasmtime `Component` via PUBLIC fields, leaking an internal wasmtime dep into the crate's public API.
Callers only pass `CompiledProvider` back into `compile_composition_with_providers` (opaque), so make the
fields PRIVATE to avoid downstream coupling to wasmtime types.

### cdz/src/main.rs:6706 (PR#1010, id 3695867202) — --report-time help mismatch
blame `2b17d0a27` "cdz test: add --report-time". The `--report-time` help text says each test line gains a
` (Nms)` suffix and the per-file line is `⏱ compose…`, but `run_test_file` actually emits separate
indented `⏱ PASS/FAIL …` lines per test and prints `⏱ <file>: compose… · run…` (with the file name).
Update the help to match the real output so `cdz test --help` isn't misleading. Doc/help.

Owners: PR#1009 wasm_host + PR#1011 blob + PR#1010 Cargo.toml → **v-agent-harness**; PR#1009 cdz-run +
PR#1010 main.rs → **v-cdz-tooling**.
