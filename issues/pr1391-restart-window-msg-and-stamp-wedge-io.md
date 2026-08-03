# PR #1391 review comments — xtask/src/fleet.rs (v-fleet-tooling)

Mirrored from https://github.com/camshaft/cadenza/pull/1391 (PR: "[v-fleet-tooling] 4018b245c").

## 1. RelaunchFailed message claims window was "killed" but can fire on NotFound (Copilot, fleet.rs:3191, also :5802) — diagnostic accuracy
> The RelaunchFailed branch message/comment says the window was "killed" before relaunch failed, but
> `restart_window` can return RelaunchFailed even when the window was already NotFound. Rewording
> avoids emitting an inaccurate diagnostic during incident response.

During an incident, a RelaunchFailed message asserting "killed the window then relaunch failed" is
misleading if the window was actually NotFound — reword so the diagnostic doesn't claim a kill that
may not have happened.

## 2. `stamp_wedge_restart` silently ignores IO errors → thrash-guard silently disabled (Copilot, fleet.rs:3886) — robustness
> `stamp_wedge_restart` silently ignores IO errors. If these writes fail, the thrash-guard is
> effectively disabled and the watchdog may restart the same wedged window every sweep without any
> indication why. It seems worth surfacing a warning when the stamp cannot be recorded.

Real operational hazard: if the wedge-restart stamp write fails silently, the anti-thrash guard is
off and the watchdog can restart the same window every sweep with no clue why. Log a warning on the
stamp-write error so the failure is visible.
