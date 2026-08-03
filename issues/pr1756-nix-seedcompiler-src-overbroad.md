# PR #1756 review comment — flake.nix (v-nix) — OPEN

https://github.com/camshaft/cadenza/pull/1756 (S1 — build the native seed compiler).

## `seedCompiler` `src = ./.` depends on the ENTIRE flake tree → spurious rebuilds (Copilot, flake.nix:84) — efficiency/hygiene
> `seedCompiler` uses `src = ./.;`, making the derivation depend on the entire flake source tree
> (including spec docs, unrelated crates), so unrelated edits bust its cache.
Scope the src to the actual build inputs (the compiler crate(s) + Cargo.lock/toml + rust-toolchain), so a
spec-doc or unrelated-crate edit doesn't force a seed-compiler rebuild. Same discipline as the runtime
derivation's scoped src. LOW/efficiency — matters for the nix-store determinism/caching north-star.
Fix-forward or before-land.
