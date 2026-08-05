# PR #1982 review — flake.nix (v-nix) — OPEN — robustness [VERIFIED, LOW]

https://github.com/camshaft/cadenza/pull/1982 (full-CI-in-nix increment 1 — fmt + clippy as nix checks).
Copilot (id 3710512239) flags the new `lintCheck` derivation omits `CARGO_NET_OFFLINE=true` that its
sibling cargo-in-nix derivations set.

## `lintCheck` buildPhase sets HOME/CARGO_HOME + a vendored-sources config but NOT `CARGO_NET_OFFLINE=true`, unlike `rcdzcWasm`/`mkStripComponent` (Copilot, flake.nix:136) — robustness [VERIFIED, LOW]
> `lintCheck` relies on the crates-io vendor config, but it does not set `CARGO_NET_OFFLINE=true` (unlike
> other cargo-in-nix derivations in this flake, e.g. rcdzcWasm and mkStripComponent). Setting it
> explicitly makes the check more robust and guarantees we don't accidentally hit the network if cargo's
> source resolution changes.

VERIFIED: the new `lintCheck` buildPhase (in the #1982 diff) exports `HOME`/`CARGO_HOME` and writes a
`replace-with = "vendored-sources"` config, but does NOT `export CARGO_NET_OFFLINE=true`. The sibling
derivations DO: `rcdzcWasm` (flake.nix:246 `export CARGO_NET_OFFLINE=true`) and `mkStripComponent`
(:358, "Network is blocked by CARGO_NET_OFFLINE"). Vendoring already blocks the network in practice (a nix
derivation has no network anyway), so this is belt-and-suspenders — but it's a cheap consistency/robustness
win: if cargo's source resolution ever changes, the explicit flag guarantees the lint check fails loudly
offline rather than attempting a fetch. LOW/robustness. Fix: add `export CARGO_NET_OFFLINE=true` to the
`lintCheck` buildPhase, matching the sibling derivations. v-nix owns flake.nix. (PR still open → foldable
pre-merge.)
