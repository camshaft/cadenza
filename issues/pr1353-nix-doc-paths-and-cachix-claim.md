# PR #1353 review comments — fleet/NIX-FLAKE-PIPELINE-SCOPING.md (v-fleet-tooling)

Mirrored from https://github.com/camshaft/cadenza/pull/1353 (PR: "cand: v-fleet-tooling — ca2d37dd8").

## 1. Wrong on-disk fixture paths (Copilot, NIX-FLAKE-PIPELINE-SCOPING.md:40) — doc
> The referenced on-disk locations for the committed reducer fixture and the Cedar guest sources
> don't match the repository layout. The only committed `.wasm` is under
> `implementation/seed/crates/cdz-kernel/tests/fixtures/…`, and the Cedar guest fixture sources are
> under `implementation/seed/crates/cdz-agent-host/tests/fixtures/…`. Using the correct paths will
> make this note easier to verify and follow.

## 2. Self-falsifying "NO cachix anywhere / grep = 0 hits" claim (Copilot, NIX-FLAKE-PIPELINE-SCOPING.md:27) — doc
> This claims there is "NO `cachix` reference anywhere in the current tree" with "grep = 0 hits", but
> this document itself introduces multiple `cachix` references, so the statement will be false for
> future readers. Consider scoping the claim to CI/config (e.g., `.github/` / Nix files) rather than
> the whole repo tree.

Both doc-accuracy on the new Nix scoping design note: fix the fixture paths to the actual layout, and
scope the "no cachix" claim to CI/Nix config (the doc itself is a counterexample to the whole-tree
grep=0 claim).
