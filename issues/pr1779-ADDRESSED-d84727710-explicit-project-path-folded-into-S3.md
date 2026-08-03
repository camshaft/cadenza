# PR #1779 review comment — flake.nix (v-nix) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1779 (MERGED).

## `buildCadenzaProject` runs `cdz build` with no project arg → upward-manifest search escapes the sandbox intent (Copilot, flake.nix:131) — correctness/hygiene
> `buildCadenzaProject` runs `cdz build` with no project arg, which triggers `cdz`'s upward-manifest
> search. [In a nix build that can find the wrong manifest / behave non-deterministically.]
Pass an explicit project path/manifest to `cdz build` so it doesn't walk upward from cwd (which in a nix
sandbox may hit an unexpected parent or fail non-deterministically). LOW-MED/hygiene — matters for
reproducible nix builds. Fix-forward.
