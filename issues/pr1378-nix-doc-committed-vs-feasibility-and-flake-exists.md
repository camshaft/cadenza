# PR #1378 review comments — fleet/NIX-FLAKE-PIPELINE-SCOPING.md (v-fleet-tooling)

Mirrored from https://github.com/camshaft/cadenza/pull/1378 (PR: "[v-fleet-tooling] 4926f2394").
Follow-up on the #1353 nix-doc edits — the status was promoted to COMMITTED but stale lines remain.

## 1. "COMMITTED DIRECTION" status contradicts "not a commitment / feasibility read" lines (amazon-q :9 + Copilot :10) — doc
> [:9] Line 9 contradicts the updated status on line 3 ("COMMITTED DIRECTION" vs "not a commitment").
> [:10] The doc now states "COMMITTED DIRECTION", but the next paragraph still says "This is a
> FEASIBILITY … read, not a commitment" — internally contradictory. Update to reflect the committed
> direction while noting N1+ is held behind the CI-lanes cutover.

Reconcile the header status with the body: now that it's a committed direction, update the
"feasibility / not a commitment" paragraph (keep the "N1+ held behind CI-lanes cutover" caveat).

## 2. "ground-truth corrections" bullet claims no flake.nix/flake.lock, but both exist now (Copilot, :36) — doc
> This "ground-truth corrections" bullet claims there is NO `flake.nix`/`flake.lock` and that
> grepping "any flake files" yields 0 hits, but the repo currently contains both `/flake.nix` and
> `/flake.lock`. Since the doc also records "N0 DONE — flake.nix + flake.lock landed", this section
> should be narrowed to the actual stale part (missing `k-framework.yml` and no cachix wiring in CI)
> and avoid asserting the flake doesn't exist.

The bullet is self-contradicted by the doc's own "N0 DONE" line — narrow it to what's actually still
missing (no `k-framework.yml`, no cachix CI wiring) and drop the "no flake files" claim.
