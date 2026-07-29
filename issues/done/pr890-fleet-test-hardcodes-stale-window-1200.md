# PR#890 review comment — fleet compose-window test hard-codes 1200 stale window (v-fleet-tooling)

Mirrored from GitHub PR#890 review comment (Copilot), id `3671882652`.
File: `xtask/src/fleet.rs:9474` — v-fleet-tooling. Blame `b868124f4` "fleet watchdog: make the pr-sync
compose-window exoneration actually add coverage (window must EXCEED the stale window)" — i.e. the very
test they added when fixing the PR#887 subsumed-no-op finding I routed. This is the follow-on nit on that
fix's test.

## Comment (verbatim)

- (id 3671882652, xtask/src/fleet.rs:9474) "This test hard-codes the default stale window as `1200`,
  which can drift if `stale_window_secs` logic or defaults change. Using `stale_window_secs(600, 2, 600)`
  here keeps the test pinned to the intended relationship (compose window exceeds stale window) without
  baking in the derived constant."

## Liaison verification (confirmed on trunk 9872e4458)

The test (fleet.rs:9471-9478) passes the literal `1200` as `trunk_advance_exonerates`'s stale-window arg:
`assert!(!trunk_advance_exonerates("pr-sync", Some(1500), 1200), …)` then
`assert!(recent_trunk_advance_exonerates("pr-sync", Some(1500), PR_SYNC_RECENT_TRUNK_SECS), …)`. The
1200 is the DERIVED default stale window (interval 600 × stale_mult 2, capped 600… per the comment). If
`stale_window_secs`'s formula/defaults change, 1200 silently goes stale and the test still passes while
no longer pinning "compose window (1800) EXCEEDS stale window". Copilot's fix keeps the relationship
honest: use `stale_window_secs(600, 2, 600)` in place of the literal so the test tracks the real derived
value. Test-robustness only; behavior-neutral. (This is the correctly-strengthened test from the PR#887
fix — good to make it drift-proof.)

Owner: **v-fleet-tooling** (`xtask/src/fleet.rs`; `b868124f4`). Replace the `1200` literal with
`stale_window_secs(600, 2, 600)`.
