#!/usr/bin/env bash
# slack-bridge-guard.sh — keep the fleet↔Slack bridge alive OUT OF BAND, decoupled from the v-slack-bridge
# agent's loop (v-fleet-tooling, 2026-09-13).
#
# WHY: the operator's alert path runs THROUGH the bridge — notably the concierge-down alert (#8931), which
# posts to the operator's Slack via the bridge daemon precisely BECAUSE the concierge (the normal path) is
# down. So "never let the concierge go down without anyone noticing" is only as reliable as the bridge. The
# bridge is normally kept up by (a) run.sh's own crash-restart loop and (b) the v-slack-bridge AGENT's loop
# calling revive.sh — but if that agent is itself down AND run.sh has died, the bridge stays down and the
# operator stops being notified (the alerter fails silently). This cron closes that gap, the same
# decouple-a-critical-function-from-an-agent pattern as reap-leases.sh (which decouples lease reclaim from
# the watchdog): it runs the bridge's OWN idempotent revive.sh — a no-op when a bridge worker is already up,
# a detached run.sh launch when none is — and raises a board alarm (surfaced by `fleet status`) when it had
# to act. It REUSES v-slack-bridge's revive.sh (their tool + their liveness signal); this only guarantees it
# runs regardless of any agent's liveness.
#
# Tracked at <repo>/fleet/, RUN from the hub copy `fleet up` materializes into <hub>/.claude/fleet/.
set -uo pipefail

# SINGLETON GUARD: flock -n so two fires never race a revive. Lock in $HOME (own-user, not the inode-pressured
# /tmp). FAIL-OPEN if flock is absent.
if command -v flock >/dev/null 2>&1 && exec 9>"${HOME}/.cdz-slack-bridge-guard.lock" 2>/dev/null; then
  flock -n 9 || exit 0
fi

HUB="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ALARM="$HUB/slack-bridge-down.alarm"
STAMP="$HUB/slack-bridge-guard.last-run"
WORKTREES="$(cd "$HUB/../worktrees" 2>/dev/null && pwd || true)"

# Is a bridge worker already up? Match the SAME signal revive.sh keys on (`[b]ridge.js` — the live node impl;
# the `[b]` trick keeps pgrep from matching itself). If up: clear any stale alarm + done (cheap common case).
if pgrep -f '[b]ridge.js' >/dev/null 2>&1; then
  rm -f "$ALARM" 2>/dev/null || true
  printf '%s bridge=up\n' "$(date -Is 2>/dev/null || echo now)" > "$STAMP" 2>/dev/null || true
  exit 0
fi

# Bridge DOWN → run the bridge's OWN idempotent revive.sh, detached. revive.sh is tracked in every worktree's
# fleet/slack-bridge/; pick the freshest-HEAD one (least-stale launcher), same as reap-leases.sh picks the
# freshest xtask binary. revive.sh self-derives the shared hub FLEET_DIR + reads ~/.cadenza-env for tokens,
# so it works from any worktree; it re-checks liveness itself, so a concurrent bring-up is safe (idempotent).
best="" best_ct=-1
if [ -n "${WORKTREES:-}" ] && [ -d "$WORKTREES" ]; then
  for wt in "$WORKTREES"/*/; do
    r="${wt}fleet/slack-bridge/revive.sh"
    [ -f "$r" ] || continue
    ct="$(git -C "$wt" show -s --format=%ct HEAD 2>/dev/null || echo 0)"
    if [ "$ct" -gt "$best_ct" ]; then best_ct="$ct"; best="$r"; fi
  done
fi

if [ -n "$best" ]; then
  setsid bash "$best" >/dev/null 2>&1 </dev/null &
  printf '%s: slack-bridge DOWN — ran revive.sh (%s). The operator alert path (concierge-down #8931) routes through the bridge, so a human should confirm it recovered.\n' \
    "$(date -Is 2>/dev/null || echo now)" "$best" > "$ALARM" 2>/dev/null || true
  printf '%s bridge=DOWN ran-revive=%s\n' "$(date -Is 2>/dev/null || echo now)" "$best" > "$STAMP" 2>/dev/null || true
else
  printf '%s: slack-bridge DOWN and NO worktree fleet/slack-bridge/revive.sh found — cannot auto-revive; a human must restart the bridge (the operator alert path is DOWN).\n' \
    "$(date -Is 2>/dev/null || echo now)" > "$ALARM" 2>/dev/null || true
  printf '%s bridge=DOWN no-revive-found\n' "$(date -Is 2>/dev/null || echo now)" > "$STAMP" 2>/dev/null || true
fi
exit 0
