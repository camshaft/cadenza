# PR #1992 review — flake.nix (v-nix) — MERGED — nix/CI-hygiene [VERIFIED, LOW]

https://github.com/camshaft/cadenza/pull/1992 (full-CI-in-nix increment 3 — rcdzc-wasm native). Copilot
(id 3710931746) flags the derivation runs cargo without the repo's `.cargo/config.toml` jobs cap.

## `rcdzcWasm` runs `cargo test`/`clippy` without the root `.cargo/config.toml` → loses `[build] jobs = 4`, default parallelism → CPU contention (Copilot, flake.nix:349) — CI-hygiene [VERIFIED, LOW]
> This derivation runs `cargo test`/`clippy` without the repo's root `.cargo/config.toml` (not included in
> `rcdzcWasmSrc`), so Cargo will fall back to its default parallelism instead of the project's `[build]
> jobs = 4` cap. In CI/normal checkouts that cap is discovered via the repo root; here it's skipped, which
> can cause unnecessary CPU contention/timeouts during `nix flake check`.

VERIFIED: `rcdzcWasmSrc` (the fileset feeding this derivation) does NOT union `./.cargo` — that
`./.cargo` union (flake.nix:98) belongs to a DIFFERENT seed derivation. So the cargo invocations in the
`rcdzcWasm` check run without the repo's `[build] jobs = 4` cap and use cargo's default (= CPU count)
parallelism. Under `nix flake check` (which may run several derivations concurrently), that oversubscribes
cores → contention / possible timeouts. LOW/CI-hygiene. Fix: add `./.cargo` (or at least
`./.cargo/config.toml`) to `rcdzcWasmSrc`'s fileset, matching the other cargo-in-nix derivations that
carry the jobs cap. (Confirm the config's `replace-with = "vendored-sources"` etc. don't conflict with the
derivation's own vendor setup — if it does, set `CARGO_BUILD_JOBS=4` explicitly in the buildPhase instead.)
v-nix owns flake.nix. (Composes with the #1989 seedTestSrc narrowing v-nix is folding into inc 4 — both
fileset-hygiene on flake.nix.)
