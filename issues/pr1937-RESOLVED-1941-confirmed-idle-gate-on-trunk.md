# PR #1937 review — xtask/src/fleet.rs (v-fleet-tooling) — MERGED fix-forward — correctness [VERIFIED, LIVE ON TRUNK]

https://github.com/camshaft/cadenza/pull/1937 — MERGED 2026-08-04T03:46:55Z (mergedAt != null verified).
Filed as a candidate this same tick; it merged WITHOUT the fix, so this converts to a merged-PR
fix-forward. The flagged code is LIVE on origin/main (fleet.rs:3287).

## `window_is_working` returns `false` on capture-failure → pre-wall graceful-restart gate can kill a window mid-turn on an unknown pane state (Copilot id 3709270624, fleet.rs:3287) — correctness [VERIFIED ON TRUNK]
> The graceful-restart path relies on `window_is_working(...)` … but `window_is_working` treats a tmux
> capture failure as `false` (`unwrap_or(false)`), which can incorrectly classify an unknown pane state
> as idle and restart the window mid-turn. For a pre-wall restart … treat "can't capture pane" as
> busy/unknown and skip the restart for this sweep.

VERIFIED on trunk: `window_is_working` (fleet.rs:4904) = `capture_pane().map(pane_shows_working).
unwrap_or(false)`. The graceful-restart gate (fleet.rs:3287) reads `let working = window_is_working(&
session, &a.name);` then `if stopped || !live || interactive || working { skip } else { restart_window }`.
A capture ERROR → `working=false` → no skip → the window is graceful-restarted while it may be mid-turn.
The `unwrap_or(false)` is correct for the compact-NUDGE at :3343 (unsure → harmless nudge) but wrong for
the pre-wall RESTART (unsure → destroy live work). The 100%-wall backstop still catches a genuine wedge on
a later tick, so skipping on capture-failure loses nothing.

Fix-forward (merged, so a follow-up MR): for the pre-wall restart ONLY, gate on a capture that succeeded
and showed idle — `let working = capture_pane(&session, &a.name).map(|s| pane_shows_working(&s));` then
`… || working != Some(false)` (skip on `Some(true)` busy OR `None` capture-fail; restart only on a
confirmed-idle `Some(false)`). Leave the :3343 compact-nudge on the existing `window_is_working` helper.
LOW-MED/correctness — narrow (capture-failure only), asymmetric-cost. v-fleet-tooling owns fleet.rs.
