#!/usr/bin/env bash
# drain-nudge.sh — the autonomous "mail-present drain heartbeat" scheduler (v-fleet-tooling 2026-09-01,
# operator-GO'd wake-path hardening). Runs `xtask fleet drain-nudge --session <s>` FREQUENTLY and DECOUPLED
# from the concierge, so an idle agent with unconsumed ACTIONABLE hub mail self-drains within a couple of
# minutes. It is the fix for the wake-miss stall: `fleet send`'s tmux send-keys nudge is SKIPPED when the
# recipient is mid-tick, and nothing re-nudged it once it went idle — so the mail sat until the recipient's
# next /loop (up to 3h) or the concierge's */4 `watchdog --nudge-drain-stalls`. A slow/busy/stalled
# concierge therefore meant idle agents sat with unconsumed mail (the recurring drain-stall across 7+
# agents). An autonomous cron closes that gap.
#
# The `fleet drain-nudge` scan is a strict SUBSET of `watchdog --nudge-drain-stalls`: it detects a CONFIRMED
# drain-stall (idle pane + actionable mail + a 2-capture confirming recapture + the queue-draining
# exoneration) and sends the SAME keystroke nudge, but takes NO other action (no re-arm, no restart, no
# escalation — those stay in the full, concierge-driven watchdog). That restraint is what makes it safe to
# run frequently. It SHARES the watchdog's drain-nudge rate-limit marker, so overlapping the concierge's
# watchdog never double-nudges. It EXCLUDES pr-sync (whose drain-stall shape needs the watchdog's
# trunk/gate/lease exonerations).
#
# WHY A WRAPPER (not a raw `cargo xtask` in the crontab): the fleet HUB is a BARE repo (no cargo project),
# so `cargo xtask` needs a real worktree. This picks a worktree with a BUILT `xtask` binary and runs it
# DIRECTLY (no cargo → no rebuild on the cron hot path). The scan reads the shared HUB registry and talks to
# the tmux SERVER via `--session` (no $TMUX needed), so ANY worktree's binary works; freshest just means the
# least-stale drain-nudge logic. Same tracked->runtime split as cpu-monitor.sh / warm-keep.sh: TRACKED at
# <repo>/fleet/, RUN from the hub copy `fleet up` materializes into <hub>/.claude/fleet/.
set -uo pipefail

# SINGLETON GUARD: a scan pays a 2s confirming recapture per SUSPECTED stall, so a pass can occasionally run
# longer than the cron interval; flock -n so only one runs at a time (a later fire skips + the next retries).
# Lock in $HOME (own-user, survives, not in the inode-pressured /tmp). FAIL-OPEN if flock is absent.
if command -v flock >/dev/null 2>&1 && exec 9>"${HOME}/.cdz-drain-nudge.lock" 2>/dev/null; then
  flock -n 9 || exit 0
fi

HUB="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKTREES="$(cd "$HUB/../worktrees" 2>/dev/null && pwd || true)"
[ -n "${WORKTREES:-}" ] && [ -d "$WORKTREES" ] || { echo "drain-nudge: no worktrees dir under $HUB/../worktrees — skip." >&2; exit 0; }
SESSION="${CDZ_FLEET_SESSION:-main}"

# Pick a worktree with a BUILT xtask binary, preferring the freshest HEAD (least-stale drain-nudge logic).
best="" best_ct=-1
for wt in "$WORKTREES"/*/; do
  bin="${wt}target/release/xtask"
  [ -x "$bin" ] || continue
  ct="$(git -C "$wt" show -s --format=%ct HEAD 2>/dev/null || echo 0)"
  if [ "$ct" -gt "$best_ct" ]; then best_ct="$ct"; best="$bin"; fi
done
[ -n "$best" ] || { echo "drain-nudge: no worktree with a built target/release/xtask yet — skip (a fleet up/build provides one)." >&2; exit 0; }

# Best-effort + exit 0: a drain-nudge is benign (a keystroke into an idle pane), rate-limited + guarded; a
# nonzero here (a tmux hiccup, or a stale binary lacking the subcommand) is not worth alarming — the next
# fire retries, and worktrees pick up the subcommand as they rebuild post-#7327. Capture the output so the
# .last-run stamp below records the result (the crontab's >/dev/null only silences the cron's OWN stdout).
# --drain-nudge-grace 300 (5min, vs the 900s default): this autonomous cron fires every 3min, so a SHORTER
# re-nudge window lets it clear a re-stalling agent fast without hand-nudging (v-metaprogramming, a high-mail
# agent getting frequent breaker issues, re-stalls between drains when the send-wake is missed mid-tick;
# concierge 2026-09-01). Still bounded (a genuinely-wedged agent isn't keystroke-spammed faster than ~5min,
# and the SAME-message stuck path + the watchdog's restart-escalation are unchanged). Overridable via env.
_out="$("$best" fleet drain-nudge --session "$SESSION" --drain-nudge-grace "${CDZ_DRAIN_NUDGE_GRACE:-300}" 2>&1)"
_rc=$?

# SILENT-CRON OBSERVABILITY (matches prune-*.sh / baseline-drift-monitor.sh; concierge convention 2026-08-29):
# OVERWRITE a `.last-run` next to this script — its MTIME is liveness proof the (silent) cron actually FIRED,
# and its content is the last result. Without this an all-silent cron is invisible ("is drain-nudge even
# running?"). The scan is quiet on a no-op pass, so the summary may be empty — the mtime is the proof either
# way. Best-effort, never fails the run.
_stamp="$(dirname "${BASH_SOURCE[0]}")/drain-nudge.last-run"
printf '%s rc=%s %s\n' \
  "$(date -Is 2>/dev/null || echo now)" "$_rc" "$(printf '%s' "$_out" | tail -1)" \
  > "$_stamp" 2>/dev/null || true

[ "$_rc" = 0 ] || echo "drain-nudge: scan exited nonzero — next fire retries." >&2
exit 0
