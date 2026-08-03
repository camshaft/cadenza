# PR #1071 review comments — xtask/src/fleet.rs `ci-status` (v-fleet-tooling)

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1071
(PR: "cand: v-fleet-tooling — next stack commit (fleet.rs)"). Two substantive points on the new
`cargo xtask fleet ci-status` command / `parse_gh_checks`.

## 1. `ci_status` ignores `gh` exit status → potential false GREEN (Copilot, fleet.rs:6489) — 🛑 correctness
> `ci_status` ignores the `gh` process exit status: a non-zero exit (auth/network/invalid target) is
> still treated as success and parsed, which can hide diagnostics and could theoretically yield a
> false GREEN if `gh` ever emits JSON alongside an error. Check `status.success()` and, on failure,
> print stderr and exit with NO-CHECKS (2) as the doc comment describes.

This one matters: `ci-status` is meant to be the polling primitive for a CI-gated pr-sync land
path, so a false GREEN here could let an un-landable PR through.

## 2. Fragile string-split parse where serde_json is already a dep (Copilot, fleet.rs:6543) — robustness
> `parse_gh_checks` uses fragile string-splitting and its doc comment claims this avoids a
> serde_json dependency, but `xtask/src/fleet.rs` already depends heavily on `serde_json`. Using
> `serde_json` here is simpler and avoids misparsing if values contain `}` or escaped quotes.
