# PR #2182 review — flake.nix (v-nix) — OPEN — 1 functional (MED) + 2 LOW [VERIFIED]

https://github.com/camshaft/cadenza/pull/2182 (seq-144 — get the agent-harness reducers ALL onto nix:
@tests-through-nix + B1-B4 component builds). Copilot 3 inline on flake.nix.

## `reducerCadenzaSrc` is rooted at the REPO ROOT (`root = ./.`) with the fixture nested 6 levels down, but `testCadenzaProject` runs `cdz test .` from the unpacked src cwd (= repo root) → `cdz test .` won't find the fixture's nested `Project.cdz`, failing the derivation or falling back to the upward-manifest search the code explicitly avoids (Copilot, flake.nix:507) — functional [VERIFIED, MED]
> `reducerCadenzaSrc` is rooted at the repo and only includes the fixture directory under
> `implementation/.../reducer-cadenza`, but `testCadenzaProject` runs `cdz test .` from the source root.
> That means `cdz test` won't see the fixture's `Project.cdz` (it's nested), so this derivation will fail
> or fall back to manifest-upward search (which the earlier comment says we want to avoid). Run tests
> against the fixture directory explicitly.

VERIFIED against source. `reducerCadenzaSrc` (#2182 diff:19-24): `root = ./.` (repo root), fileset =
`[ ./implementation/seed/crates/cdz-kernel/tests/fixtures/reducer-cadenza, .../wit ]`. So the unpacked src
tree has the fixture (with its `Project.cdz`, confirmed present) nested at
`implementation/seed/crates/cdz-kernel/tests/fixtures/reducer-cadenza/`. But `testCadenzaProject`'s
buildPhase (flake.nix:470-471) runs `cdz test .` where `.` = "the unpacked src cwd" (= repo root), with an
explicit comment "Test THIS project explicitly (`.`…), not via the upward manifest search — same
sandbox-escape guard as buildCadenzaProject." So `cdz test .` executes at the ROOT, where there is NO
`Project.cdz` (it's 6 dirs down) → either the derivation fails ("no project here"), or `cdz` walks
DOWNWARD/upward to find one — the exact manifest-search fallback the guard forbids. Contrast
`exampleProjectTests` (flake.nix:482) which roots its src AT the project dir so `cdz test .` lands on the
Project.cdz. So this reducer wiring is mis-rooted → the seq-144 goal (reducer @tests through nix) doesn't
actually run as intended. MED/functional. Fix per Copilot: root `reducerCadenzaSrc` AT the fixture dir
(so the unpacked cwd IS the project), i.e. `root = ./implementation/seed/crates/cdz-kernel/tests/fixtures/
reducer-cadenza` (+ carry `wit/` alongside), OR `cd` into the fixture subdir before `cdz test .`. Match the
`exampleProjectTests` rooting pattern.

## the comment block embeds verification counts + coordination detail (exact test totals "14 @tests b1=2/b2=2/b3=5/genesis=5", external-lane notes) likely to go stale (Copilot, flake.nix:496) — doc-staleness [VERIFIED, LOW]
> This comment block embeds verification counts and coordination details (exact test totals, external lane
> notes, etc.) that are likely to go stale … reduce it to the durable invariants (what is built/tested and
> why the fileset includes `wit/`).
VERIFIED (diff:11-16 embeds "14 @tests pass (b1=2/b2=2/b3=5/genesis=5)" + lane notes). LOW/doc-staleness —
a per-count comment drifts the moment a test is added. Fix: keep the durable invariant (what's built +
why `wit/` is in the fileset), drop the exact counts.

## the component derivation writes `$out` in `buildPhase` + `dontInstall = true`, inconsistent with the flake's single-wasm derivations that write `$out` in `installPhase` (Copilot, flake.nix:531) — consistency [VERIFIED, LOW]
> This derivation writes the final output to `$out` in `buildPhase` and then disables `installPhase` via
> `dontInstall = true`. Elsewhere … derivations that produce a single wasm artifact keep `$out` writes in
> `installPhase` … keeps build vs install responsibilities consistent.
VERIFIED (diff:46-53: `buildPhase` does `cdz compile … -o "$out"`, then `dontInstall = true`). LOW/
consistency — works, but diverges from the flake's build-vs-install split (cf `testCadenzaProject` itself,
which writes `$out` in `installPhase`). Fix: move the `-o "$out"` to `installPhase` (or note why this one
differs). No functional bug.

c1 is the one that matters — it means the reducer @tests may not actually be gated through nix as seq-144
intends. v-nix owns flake.nix. PR OPEN → all foldable pre-merge.
