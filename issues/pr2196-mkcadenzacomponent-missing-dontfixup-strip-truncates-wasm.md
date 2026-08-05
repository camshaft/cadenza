# PR #2196 review — flake.nix (v-nix) — OPEN — correctness/artifact-corruption [VERIFIED, MED] (on the fix for MY #2182)

https://github.com/camshaft/cadenza/pull/2196 (seq-144 review-fold — root reducerCadenzaTestSrc at the
fixture dir + installPhase consistency; THE fix for MY #2182 review). Copilot 1 inline — a NEW correctness
risk in the same PR, of a class the flake already documented.

## `mkCadenzaComponent` produces a single `.wasm` output but does NOT set `dontFixup`, so stdenv's `fixupPhase` runs `strip` on it → truncates the wasm to a corrupt artifact in the nix store; `rcdzcWasm` (same file) documents + guards exactly this (Copilot, flake.nix:523) — correctness/artifact-corruption [VERIFIED, MED]
> mkCadenzaComponent produces a single `.wasm` file output, but unlike `rcdzcWasm` (same file) it does
> not disable stdenv's `fixupPhase`. `rcdzcWasm` documents that fixup runs `strip` on single-wasm outputs
> and truncates them; the same risk applies here and could yield a corrupted component artifact in the
> nix store.

VERIFIED against the diff + the flake's own documented precedent. `mkCadenzaComponent` (#2196 diff:58-67):
`cdz compile … -o component.wasm` then `cp component.wasm "$out"` — a single wasm FILE output, NO
`dontFixup`. The SAME file's `rcdzcWasm` derivation (flake.nix:591-608) has the explicit trap note: "🪤
dontFixup = true: the output is a single wasm FILE; stdenv's fixupPhase runs `strip` on it … (Verified:
with fixup, out=54B; with dontFixup, out=5309060B.)" and sets `dontFixup = true` (flake.nix:608). So fixup's
`strip` truncates a single-wasm output to ~54 bytes — a CORRUPT component. `mkCadenzaComponent` is missing
that same guard, so `reducerCadenzaB1..Genesis` (diff:71+) risk emitting corrupted 54-byte components into
the nix store. MED (a real artifact-corruption, not cosmetic — a downstream consumer of `.#reducer-cadenza-b1`
gets a truncated wasm). NOTE: `rcdzcWasm` uses `stdenvNoCC` AND still needs `dontFixup`, so "it's stdenvNoCC"
doesn't exempt it — the risk applies. Fix per Copilot + matching rcdzcWasm: add `dontFixup = true;` to
`mkCadenzaComponent` (with the same one-line rationale comment). v-nix owns flake.nix. PR OPEN → foldable
pre-merge. (Owning the chain: my #2182 review drove this re-fold; the re-fold's new component-build helper
inherited the single-wasm-fixup trap the flake had ALREADY solved once for rcdzcWasm — worth cross-checking
any other single-wasm derivation added here has dontFixup too.)
