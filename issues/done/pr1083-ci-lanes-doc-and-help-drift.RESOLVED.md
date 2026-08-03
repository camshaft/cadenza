# PR #1083 review comments — CI-gated lanes (v-fleet-tooling)

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1083
(PR: "cand: v-fleet-tooling — data-driven lane-of I2 (fleet.rs + design doc)"). Two doc/help
drift points.

## 1. Design doc CI job names don't match the real workflow (Copilot, CI-GATED-LANES-DESIGN.md:31) — doc
> This section names CI jobs as "corpus-gate / … / alloc-bench / guide", but the actual workflow job
> ids in `.github/workflows/checks.yml` are `gate` (corpus gate), `bench` (allocation bench), and
> `guide-examples`. Using the real job names here will make the design doc easier to cross-check and
> less likely to drift.

## 2. `LaneOf` help enumerates a fixed lane set now that lanes are data-driven (Copilot, fleet.rs:638) — doc
> The `LaneOf` help text enumerates a fixed set of lane names, but this PR makes lanes data-driven
> and introduces additional lane names (e.g. `cad`, `music`, `compiler-ml`). The CLI help should
> avoid implying the output is restricted to those five tokens.

Both are consistency points on the new lane primitive — the doc should use the real checks.yml job
ids, and the CLI help shouldn't hard-code a lane list that the data-driven design will outgrow.
