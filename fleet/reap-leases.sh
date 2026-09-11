#!/usr/bin/env bash
# reap-leases.sh — the autonomous LEAKED-CHECK-LEASE reaper scheduler (v-fleet-tooling 2026-09-11,
# concierge coverage-hole flag). Runs `xtask fleet reap-leases` on a system cron, DECOUPLED from the
# window-touching `watchdog`, so leaked check-leases are reclaimed even when an operator has disabled the
# watchdog to stop window-killing (2026-09-10 no-window-kill directive) — otherwise leaked leases lose
# their only reaper and persist (a leaked PRIORITY lease stalls EVERY vertical's merge gate).
#
# `fleet reap-leases` is the SAME `reap_check_leases` reclaim the watchdog folds into its sweep, but it
# only removes dead-PID / TTL-stale `.lease` files and touches NO tmux window — so this is safe to run
# FREQUENTLY + spuriously (a no-op when clean). No `--session`: lease reclaim reads the shared HUB lease
# dir, never the tmux server.
#
# WHY A WRAPPER (not a raw `cargo xtask` in the crontab): the fleet HUB is a BARE repo (no cargo project),
# so `cargo xtask` needs a real worktree. This picks a worktree with a BUILT `xtask` binary and runs it
# DIRECTLY (no cargo → no rebuild on the cron hot path). The reclaim reads the shared HUB lease dir, so ANY
# worktree's binary works; freshest just means the least-stale reap logic. Same tracked->runtime split as
# drain-nudge.sh / compact-nudge.sh: TRACKED at <repo>/fleet/, RUN from the hub copy `fleet up`
# materializes into <hub>/.claude/fleet/.
set -uo pipefail

# SINGLETON GUARD: cheap, but flock -n anyway so two fires never race the same lease dir (a later fire skips
# + the next retries). Lock in $HOME (own-user, survives, not in the inode-pressured /tmp). FAIL-OPEN if
# flock is absent.
if command -v flock >/dev/null 2>&1 && exec 9>"${HOME}/.cdz-reap-leases.lock" 2>/dev/null; then
  flock -n 9 || exit 0
fi

HUB="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKTREES="$(cd "$HUB/../worktrees" 2>/dev/null && pwd || true)"
[ -n "${WORKTREES:-}" ] && [ -d "$WORKTREES" ] || { echo "reap-leases: no worktrees dir under $HUB/../worktrees — skip." >&2; exit 0; }

# Pick a worktree with a BUILT xtask binary, preferring the freshest HEAD (least-stale reap logic).
best="" best_ct=-1
for wt in "$WORKTREES"/*/; do
  bin="${wt}target/release/xtask"
  [ -x "$bin" ] || continue
  ct="$(git -C "$wt" show -s --format=%ct HEAD 2>/dev/null || echo 0)"
  if [ "$ct" -gt "$best_ct" ]; then best_ct="$ct"; best="$bin"; fi
done
[ -n "$best" ] || { echo "reap-leases: no worktree with a built target/release/xtask yet — skip (a fleet up/build provides one)." >&2; exit 0; }

# Best-effort + exit 0: reclaiming a leaked lease is benign (removes only a dead-PID/TTL-stale .lease file);
# a nonzero here (a stale binary lacking the subcommand) is not worth alarming — the next fire retries, and
# worktrees pick up the subcommand as they rebuild. Capture the output for the .last-run stamp below.
_out="$("$best" fleet reap-leases 2>&1)"
_rc=$?

# SILENT-CRON OBSERVABILITY (matches drain-nudge.sh / compact-nudge.sh; concierge convention 2026-08-29):
# OVERWRITE a `.last-run` next to this script — its MTIME is liveness proof the (silent) cron fired, its
# content the last result. Best-effort, never fails the run.
_stamp="$(dirname "${BASH_SOURCE[0]}")/reap-leases.last-run"
printf '%s rc=%s %s\n' \
  "$(date -Is 2>/dev/null || echo now)" "$_rc" "$(printf '%s' "$_out" | tail -1)" \
  > "$_stamp" 2>/dev/null || true

[ "$_rc" = 0 ] || echo "reap-leases: reap exited nonzero — next fire retries." >&2
exit 0
