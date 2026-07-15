# PR review comment — mirrored from GitHub PR #393 (Copilot inline)

- **PR:** #393 "fleet: nineteenth batch (nested-nullary-variant fix, eval-malformed trap, cdz corpus fold, ast-lift, corpus)" (MERGED)
- **File:** `xtask/src/main.rs:2681`
- **Reviewer:** Copilot (automated)
- **Comment id:** 3590413229
- **Link:** https://github.com/camshaft/cadenza/pull/393#discussion_r3590413229

## Comment (verbatim)
> `xtask check` passes `--profile {profile}` through to the nested `gate` invocation, but the newly-wired `duvet-check` invocation drops the selected profile. This makes `cargo xtask --profile <...> check` run `duvet-check` under a different profile than the rest of the steps, which is surprising and can cause avoidable rebuilds.

## Liaison triage — CONFIRMED against trunk
Confirmed in xtask/src/main.rs: the gate step builds `format!("{xtask} --profile {profile} gate…")` but
the newly-wired duvet-check step is `format!("{xtask} duvet-check")` with NO `--profile {profile}`. So
`cargo xtask --profile <...> check` runs duvet-check under a different (default) profile than the rest,
which can trigger avoidable rebuilds. Fleet-tooling territory (v-fleet-tooling owns xtask). Small fix:
thread `--profile {profile}` into the duvet-check command. Fix on `trunk`. Quote + link in queue file.
