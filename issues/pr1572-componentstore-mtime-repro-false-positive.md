# PR #1572 review comment — flake.nix (v-nix)

Mirrored from https://github.com/camshaft/cadenza/pull/1572 (PR: "[v-nix] 6f40ef8c9").
R2 slice — content-addressed component store (`packages.store` = every nix-built wasm as `<hash>.wasm`).

## `componentStore` cp mtimes make packages.store non-reproducible (Copilot, flake.nix:302) — VERIFIED FALSE POSITIVE (mtime part); pipefail part optional
> `componentStore` copies artifacts with `cp`, which will assign fresh mtimes in the build sandbox.
> Nix output hashes include mtimes, so this can make `packages.store` non-reproducible and defeat
> binary cache sharing. Consider normalizing timestamps … and adding `set -euo pipefail`.

The CORE premise — "Nix output hashes include mtimes" — is **FALSE**. Nix's NAR serialization (what
output hashes are computed over) records only file-type, the executable bit, and contents; it does
NOT include mtimes (Nix canonicalizes mtime to a fixed value when a path is registered in the store —
this is foundational to Nix reproducibility). EMPIRICALLY CONFIRMED on the host nix: `nix-store
--dump` of the same content with mtimes 23 years apart yields the IDENTICAL sha256. So the sandbox
`cp` mtimes cannot make `packages.store` non-reproducible or defeat cache sharing — the concern is a
hallucination of Nix semantics.

The SECONDARY suggestion (`set -euo pipefail` in the runCommand script) is a legitimate but OPTIONAL
defensive nit — with a small fixed component list and `cp`/`sha256sum` that fail loudly, it's not
required for correctness. Owner's call. Do NOT act on the mtime/reproducibility claim.
