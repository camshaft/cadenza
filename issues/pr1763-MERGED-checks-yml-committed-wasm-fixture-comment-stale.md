# PR #1763 review comment — .github/workflows/checks.yml (v-fleet-tooling) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1763 (MERGED).

## checks.yml comment still describes a committed reducer-guest .wasm fixture + `cargo test` loading a committed binary (Copilot, checks.yml:252) — doc
> This comment block still describes a byte-for-byte `cmp` against a committed reducer-guest .wasm fixture
> and claims `cargo test` loads a committed binary, but the reducer-guest is "no longer a committed
> binary" (v-nix N2) and the e2e tests read the guest from `REDUCER_GUEST_COMPONENT`.

Stale after the v-nix N2 change (reducer-guest is nix-built, read via `REDUCER_GUEST_COMPONENT`, not a
committed .wasm). Update the comment block to the current nix-built/env-var model so it doesn't mislead.
LOW/doc. Fix-forward.
