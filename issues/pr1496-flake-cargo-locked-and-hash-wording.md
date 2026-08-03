# PR #1496 review comments — flake.nix (v-nix)

Mirrored from https://github.com/camshaft/cadenza/pull/1496 (PR: "[v-nix] edeedae35").

## 1. `cargo component build` without `--locked` undermines the fixed-output derivation (Copilot, flake.nix:106) — reproducibility
> The fixed-output derivation relies on Cargo.lock being honored, but the build currently runs
> `cargo component build` without `--locked`, which can update the lockfile or resolve different
> dependency versions, making the build less deterministic (and potentially doing extra network work
> before failing the output hash check).

Add `--locked` so the FOD build resolves exactly the committed Cargo.lock — otherwise a dep-version
drift changes the output and only surfaces as an opaque output-hash mismatch after wasted network work.

## 2. Comment conflates Nix store-path hash with the file's SHA-256 content hash (Copilot, flake.nix:123) — doc
> The comment claims `nix build .#runtime` produces a store path "whose hash is
> REQUIRED_RUNTIME_HASH", but Nix store path hashes are not the same as the file's SHA-256 content
> hash. What you *do* get here is that Nix enforces the output content hash equals
> REQUIRED_RUNTIME_HASH via `outputHash` (and the store path is derived from it). Rewording this
> avoids confusion for readers trying to locate artifacts by hash.

Reword: Nix enforces the output *content* hash == REQUIRED_RUNTIME_HASH via `outputHash`; the
store-path hash is a different (derived) thing — don't imply the store path's hash equals the
content SHA-256.
