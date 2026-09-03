#!/usr/bin/env bash
# compact-nudge.sh — the autonomous OUT-OF-BAND concierge-compaction scheduler (v-fleet-tooling 2026-09-03).
# Runs `xtask fleet compact-nudge --session <s>` on a system cron, DECOUPLED from any agent tick, so the
# CONCIERGE gets its pre-wall `/compact` (or an at-wall restart) even though the in-tick watchdog CANNOT do
# it: the watchdog that sends `/compact` is FOLDED INTO the concierge's maintenance tick, so when it
# evaluates the concierge, the concierge is BY DEFINITION mid-tick → it can never send-keys `/compact` to
# itself, and the concierge climbs to the 100% context wall with no recovery (it can't self-`/compact` — a
# built-in, not a tool). This cron runs OUTSIDE the concierge tick and catches it while IDLE between fires.
#
# The `fleet compact-nudge` scan is a strict SUBSET of the watchdog's compaction: it sends the SAME pre-wall
# `/compact` (via send-keys) when the concierge is IDLE in the [85,100) band, or the SAME `restart_window`
# at the 100% wall, and takes NO other action. It SHARES the watchdog's COMPACT_NUDGE_GRACE +
# WEDGE_RESTART_GRACE marker stamps, so overlapping never double-acts. That restraint is what makes it safe
# to run frequently. It targets ONLY the concierge (the one agent whose watchdog runs inside its own tick;
# every other agent IS compacted by the concierge-tick watchdog).
#
# WHY A WRAPPER (not a raw `cargo xtask` in the crontab): the fleet HUB is a BARE repo (no cargo project),
# so `cargo xtask` needs a real worktree. This picks a worktree with a BUILT `xtask` binary and runs it
# DIRECTLY (no cargo → no rebuild on the cron hot path). The scan reads the shared HUB registry + talks to
# the tmux SERVER via `--session` (no $TMUX needed), so ANY worktree's binary works; freshest just means the
# least-stale scan logic. Same tracked→runtime split as drain-nudge.sh: TRACKED at <repo>/fleet/, RUN from
# the hub copy `fleet up` materializes into <hub>/.claude/fleet/.
set -uo pipefail

# SINGLETON GUARD: a scan captures the concierge pane + may send-keys; flock -n so only one runs at a time
# (a later fire skips + the next retries). Lock in $HOME (own-user, survives, not in inode-pressured /tmp).
# FAIL-OPEN if flock is absent.
if command -v flock >/dev/null 2>&1 && exec 9>"${HOME}/.cdz-compact-nudge.lock" 2>/dev/null; then
  flock -n 9 || exit 0
fi

HUB="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKTREES="$(cd "$HUB/../worktrees" 2>/dev/null && pwd || true)"
[ -n "${WORKTREES:-}" ] && [ -d "$WORKTREES" ] || { echo "compact-nudge: no worktrees dir under $HUB/../worktrees — skip." >&2; exit 0; }
SESSION="${CDZ_FLEET_SESSION:-main}"

# Pick a worktree with a BUILT xtask binary, preferring the freshest HEAD (least-stale scan logic).
best="" best_ct=-1
for wt in "$WORKTREES"/*/; do
  bin="${wt}target/release/xtask"
  [ -x "$bin" ] || continue
  ct="$(git -C "$wt" show -s --format=%ct HEAD 2>/dev/null || echo 0)"
  if [ "$ct" -gt "$best_ct" ]; then best_ct="$ct"; best="$bin"; fi
done
[ -n "$best" ] || { echo "compact-nudge: no worktree with a built target/release/xtask yet — skip (a fleet up/build provides one)." >&2; exit 0; }

# Best-effort + exit 0: a compact-nudge is benign (a `/compact` keystroke into an IDLE pane, or a restart of
# a wall-wedged window — both watchdog-grace-guarded); a nonzero here (a tmux hiccup, or a stale binary
# lacking the subcommand) is not worth alarming — the next fire retries, and worktrees pick up the subcommand
# as they rebuild. Capture the output so the .last-run stamp below records the result.
_out="$("$best" fleet compact-nudge --session "$SESSION" 2>&1)"
_rc=$?

# SILENT-CRON OBSERVABILITY (matches drain-nudge.sh / prune-*.sh; concierge convention 2026-08-29): OVERWRITE
# a `.last-run` next to this script — its MTIME is liveness proof the (silent) cron actually FIRED, and its
# content is the last result. The scan is quiet on a no-op pass (concierge below the band / mid-tick), so the
# summary may be empty — the mtime is the proof either way. Best-effort, never fails the run.
_stamp="$(dirname "${BASH_SOURCE[0]}")/compact-nudge.last-run"
printf '%s rc=%s %s\n' \
  "$(date -Is 2>/dev/null || echo now)" "$_rc" "$(printf '%s' "$_out" | tail -1)" \
  > "$_stamp" 2>/dev/null || true

[ "$_rc" = 0 ] || echo "compact-nudge: scan exited nonzero — next fire retries." >&2
exit 0
