#!/usr/bin/env bash
# rearm-stale.sh — the autonomous STALE-HEARTBEAT RE-ARM scheduler (v-fleet-tooling, BUILD-BUT-LEAVE-OFF
# per concierge 2026-09-10). Runs `xtask fleet rearm-stale --session <s>` FREQUENTLY and DECOUPLED from the
# concierge, so an active agent whose `/loop` recurring cron DROPPED (heartbeat goes stale past its window)
# is re-armed with a send-keys `continue`/`/loop` within minutes instead of sitting dead until a manual
# `resume`. It is the non-destructive half of the (currently-disabled) watchdog's self-heal.
#
# The `fleet rearm-stale` scan is a strict SUBSET of `watchdog`, exactly like `drain-nudge.sh` is: it uses
# the SAME heartbeat-staleness verdict, the SAME never-interrupt-a-heads-down-agent pane-busy guard +
# 2-capture confirming recapture, the SAME anti-thrash grace, and the SAME dead-cron escalation + streak
# bookkeeping (sharing the watchdog's rearm markers so running both never double-arms) — but it takes NONE of
# the watchdog's DESTRUCTIVE actions (no window recreate/reap, no Escape, no dead-letter reap, no compaction
# restart). That non-destructive-BY-CONSTRUCTION restraint is what would make it safe on a frequent cron.
# It EXCLUDES pr-sync (whose stale-mid-batch shape needs the watchdog's trunk/gate/lease exonerations).
#
# ENABLED (operator seq 1251, ASK1 GREEN 2026-09-24): the `# fleet:rearm-stale` crontab line now ships LIVE
# (`ensure_rearm_stale_cron` keyed on REARM_STALE_ENABLED=true). It is the sanctioned NON-destructive self-heal
# — token-delta-gated (#9644: never nudges work-in-flight) + a send-keys `/loop` re-arm NOT a window-kill
# (ban-compliant) — with an escalation rung (#9660) that SURFACES a not-sticking reissue as a likely session
# wedge (rate-limited note + watchdog.log) needing an operator restart, instead of re-arming forever. The
# DESTRUCTIVE early-reap remains unbuilt + operator-gated (1251 authorizes non-destructive automation only).
# The command is also runnable by hand (dry-run especially) for inspection.
#
# WHY A WRAPPER (not a raw `cargo xtask` in the crontab): the fleet HUB is a BARE repo (no cargo project), so
# `cargo xtask` needs a real worktree. This picks a worktree with a BUILT `xtask` binary and runs it DIRECTLY
# (no cargo → no rebuild on the cron hot path). The scan reads the shared HUB registry + talks to the tmux
# SERVER via `--session` (no $TMUX needed), so ANY worktree's binary works; freshest just means the least-stale
# re-arm logic. Same tracked->runtime split as drain-nudge.sh / cpu-monitor.sh: TRACKED at <repo>/fleet/, RUN
# from the hub copy `fleet up` materializes into <hub>/.claude/fleet/.
set -uo pipefail

# SINGLETON GUARD: a scan pays a 2s confirming recapture per SUSPECTED-stale agent, so a pass can occasionally
# run longer than the cron interval; flock -n so only one runs at a time (a later fire skips + the next
# retries). Lock in $HOME (own-user, survives, not in the inode-pressured /tmp). FAIL-OPEN if flock is absent.
if command -v flock >/dev/null 2>&1 && exec 9>"${HOME}/.cdz-rearm-stale.lock" 2>/dev/null; then
  flock -n 9 || exit 0
fi

HUB="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKTREES="$(cd "$HUB/../worktrees" 2>/dev/null && pwd || true)"
[ -n "${WORKTREES:-}" ] && [ -d "$WORKTREES" ] || { echo "rearm-stale: no worktrees dir under $HUB/../worktrees — skip." >&2; exit 0; }
SESSION="${CDZ_FLEET_SESSION:-main}"

# Pick a worktree with a BUILT xtask binary, preferring the freshest HEAD (least-stale re-arm logic).
best="" best_ct=-1
for wt in "$WORKTREES"/*/; do
  bin="${wt}target/release/xtask"
  [ -x "$bin" ] || continue
  ct="$(git -C "$wt" show -s --format=%ct HEAD 2>/dev/null || echo 0)"
  if [ "$ct" -gt "$best_ct" ]; then best_ct="$ct"; best="$bin"; fi
done
[ -n "$best" ] || { echo "rearm-stale: no worktree with a built target/release/xtask yet — skip (a fleet up/build provides one)." >&2; exit 0; }

# Best-effort + exit 0: a re-arm is a keystroke into an idle/stale pane, rate-limited + guarded; a nonzero
# here (a tmux hiccup, or a stale binary lacking the subcommand) is not worth alarming — the next fire
# retries, and worktrees pick up the subcommand as they rebuild. Capture the output so the .last-run stamp
# below records the result (the crontab's >/dev/null only silences the cron's OWN stdout).
_out="$("$best" fleet rearm-stale --session "$SESSION" 2>&1)"
_rc=$?

# SILENT-CRON OBSERVABILITY (matches drain-nudge.sh / prune-*.sh; concierge convention 2026-08-29): OVERWRITE
# a `.last-run` next to this script — its MTIME is liveness proof the (silent) cron actually FIRED, and its
# content is the last result. The scan is quiet on a no-op pass, so the summary may be empty — the mtime is
# the proof either way. Best-effort, never fails the run.
_stamp="$(dirname "${BASH_SOURCE[0]}")/rearm-stale.last-run"
printf '%s rc=%s %s\n' \
  "$(date -Is 2>/dev/null || echo now)" "$_rc" "$(printf '%s' "$_out" | tail -1)" \
  > "$_stamp" 2>/dev/null || true

[ "$_rc" = 0 ] || echo "rearm-stale: scan exited nonzero — next fire retries." >&2
exit 0
