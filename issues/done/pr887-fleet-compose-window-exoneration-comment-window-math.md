# PR#887 review comment — compose-window exoneration comment's window-relationship rationale is misleading (v-fleet-tooling)

Mirrored from GitHub PR#887 review comment (Copilot), id `3670767609`.
File: `xtask/src/fleet.rs:2615` — v-fleet-tooling's exclusive file. Blame `a60ffe00d` "fleet watchdog:
exonerate a pr-sync drain-stall in the post-land COMPOSE window via a recent trunk advance".

## Comment (verbatim)

- (id 3670767609, xtask/src/fleet.rs:2615) "The new compose-window exoneration comment implies it
  'closes' a gap caused by `trunk_exonerates` being bounded by pr-sync's stale window, but
  `PR_SYNC_RECENT_TRUNK_SECS` (15m) is actually *shorter* than the default pr-sync stale window (20m with
  interval=10m, stale_mult=2, stale_cap=600). As written, it reads like this logic changes the default
  behavior, when it only has impact if the stale window is tightened via `--stale-mult/--stale-cap`.
  Consider clarifying the rationale in the comment to match the actual relationship between the windows."

## Liaison verification (confirmed on trunk d2ae042a7)

Fifth-exoneration comment (fleet.rs:2609-2614): "…`trunk_exonerates` is bounded by pr-sync's tight
~20min stale window (interval 10m). A purpose-built fixed 15min window on the same commit age closes it".
And `PR_SYNC_RECENT_TRUNK_SECS = 15 * 60` (fleet.rs:3889). Copilot's arithmetic is right: 15m < 20m, so
a `trunk_commit_age < 15m` is ALSO `< 20m` — i.e. under the DEFAULT stale window, `trunk_exonerates`
(which the doc says is bounded at ~20m) already covers everything the new 15m window would. The new
window only adds distinct coverage when the stale window is tightened BELOW 15m via
`--stale-mult`/`--stale-cap`. So the comment's "closes it" rationale is misleading for the default case —
the fix is real but its window-math justification as written doesn't hold at defaults. Reword the comment
to state the actual relationship (e.g. "adds a floor of 15min independent of a tightened stale window",
or whatever the true intent is — owner knows the design rationale). Comment-only, behavior-neutral (the
exoneration logic itself is unchanged; only its explanatory comment is questioned).

Owner: **v-fleet-tooling** (`xtask/src/fleet.rs` watchdog; `a60ffe00d`). Comment-rationale clarification
— confirm the intended window relationship and reword to match.
