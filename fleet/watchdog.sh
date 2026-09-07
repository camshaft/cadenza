#!/usr/bin/env bash
# watchdog.sh — run the FULL fleet watchdog from a worktree whose source is CURRENT, so a landed
# fleet-tooling change is actually LIVE in the watchdog within a tick (v-fleet-tooling 2026-09-07,
# concierge-delegated deploy-gap fix).
#
# THE GAP THIS FIXES: the concierge's maintenance cron ran `cd .claude/worktrees/pr-sync && cargo xtask
# fleet watchdog` — but pr-sync is STOPPED, so its worktree never syncs, and `cargo xtask` (= `cargo run`)
# faithfully built+ran that STALE source. Result: watchdog fixes that had landed on main (#8566 dead-window
# recreate, #8568 concierge reap-guard, #8569 permission-dialog classification) were INERT — the watchdog
# was running an 11-day-old binary. Any single hardcoded worktree can go stale (stopped/rested/not-synced),
# especially during the perf-push when most cadenza worktrees are at rest.
#
# THE FIX: pick the FRESHEST-HEAD worktree (an ACTIVE agent's — it syncs to main each tick) and run the
# watchdog there via `cargo xtask` (= `cargo run`, which REBUILDS from that worktree's current source). So
# the running binary is guaranteed to match the freshest landed code, with NO dependency on any one
# possibly-stale worktree. Warm after the first build (cargo freshness → a no-op rebuild on most fires; it
# only recompiles right after a fleet-tooling landing).
#
# This REPLACES the concierge's ad-hoc `cd pr-sync && cargo xtask fleet watchdog --nudge-drain-stalls` cron
# line (it is the SAME single watchdog invocation, just from a fresh worktree) — it is NOT a second watchdog,
# so it does not double-act. Point the maintenance cron at `bash <hub>/.claude/fleet/watchdog.sh`.
#
# Tracked at <repo>/fleet/watchdog.sh; RUN from the hub copy `fleet up` materializes into
# <hub>/.claude/fleet/watchdog.sh (same tracked->runtime split as drain-nudge.sh / compact-nudge.sh).
set -uo pipefail

# SINGLETON GUARD: a full watchdog pass (2-capture recaptures, per-agent scans, an occasional cargo rebuild)
# can run longer than the cron interval; flock -n so only one runs at a time (a later fire skips, next retries).
# Lock in $HOME (own-user, survives, not in the inode-pressured /tmp). FAIL-OPEN if flock is absent.
if command -v flock >/dev/null 2>&1 && exec 9>"${HOME}/.cdz-watchdog.lock" 2>/dev/null; then
  flock -n 9 || exit 0
fi

HUB="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKTREES="$(cd "$HUB/../worktrees" 2>/dev/null && pwd || true)"
[ -n "${WORKTREES:-}" ] && [ -d "$WORKTREES" ] || { echo "watchdog: no worktrees dir under $HUB/../worktrees — skip." >&2; exit 0; }
SESSION="${CDZ_FLEET_SESSION:-main}"

# Pick the worktree with the FRESHEST HEAD (an active agent's — it has the newest landed source). Unlike
# drain-nudge.sh (which runs a prebuilt binary), we run `cargo xtask` FROM this worktree so cargo rebuilds
# the binary from its current source — guaranteeing the watchdog runs the freshest code, not a stale binary.
# Require a Cargo project (xtask/Cargo.toml) so `cargo xtask` can build there.
best="" best_ct=-1
for wt in "$WORKTREES"/*/; do
  [ -f "${wt}xtask/Cargo.toml" ] || continue
  ct="$(git -C "$wt" show -s --format=%ct HEAD 2>/dev/null || echo 0)"
  if [ "$ct" -gt "$best_ct" ]; then best_ct="$ct"; best="$wt"; fi
done
[ -n "$best" ] || { echo "watchdog: no worktree with an xtask Cargo project — skip (a fleet up provides one)." >&2; exit 0; }

# Run the FULL watchdog from the freshest worktree. `cargo xtask` (= cargo run) rebuilds from its current
# source first, so the running binary matches that worktree's HEAD (the deploy-gap fix). Server-direct
# (--session, no $TMUX). Best-effort + exit 0: a watchdog hiccup (tmux glitch, a transient build error) is
# not worth alarming — the next fire retries. Capture output for the .last-run stamp.
_out="$( cd "$best" && cargo xtask fleet watchdog --nudge-drain-stalls --session "$SESSION" 2>&1 )"
_rc=$?

# SILENT-CRON OBSERVABILITY (matches drain-nudge.sh / compact-nudge.sh): OVERWRITE a `.last-run` next to this
# script — its MTIME is proof the cron FIRED, content is the last result (+ which worktree it ran from).
_stamp="$(dirname "${BASH_SOURCE[0]}")/watchdog.last-run"
printf '%s rc=%s wt=%s %s\n' \
  "$(date -Is 2>/dev/null || echo now)" "$_rc" "$(basename "$best")" "$(printf '%s' "$_out" | tail -1)" \
  > "$_stamp" 2>/dev/null || true

[ "$_rc" = 0 ] || echo "watchdog: pass exited nonzero — next fire retries." >&2
exit 0
