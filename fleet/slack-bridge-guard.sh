#!/usr/bin/env bash
# slack-bridge-guard.sh — keep the fleet↔Slack bridge alive OUT OF BAND, decoupled from any agent's loop
# (v-fleet-tooling, 2026-09-13; retargeted from the node bridge.js to the membrain daemon 2026-09-30).
#
# WHY: the operator's alert path runs THROUGH the bridge — notably the concierge-down alert (#8931), which
# posts to the operator's Slack via the bridge precisely BECAUSE the concierge (the normal path) is down. So
# "never let the concierge go down without anyone noticing" is only as reliable as the bridge. The live bridge
# is the membrain-skynet-bridge daemon: it is fail-soft internally (its inbound/outbound loops retry board and
# Slack I/O and do not crash on a transient error), but it runs as a bare process under no supervisor, so a
# panic or a host reboot leaves it down with no auto-revive until the durable systemd role (dotfiles #153,
# Restart=always) lands. This cron is that supervisor in the meantime — the same
# decouple-a-critical-function-from-an-agent pattern as reap-leases.sh: every few minutes it checks the daemon
# is up (exactly one instance) and revives it when it is gone, regardless of whether the v-slack-bridge agent
# is running, raising a board alarm (surfaced by `fleet status`) whenever it had to act.
#
# SINGLETON is a correctness property, not just hygiene: two daemons = double Slack relay (a message relayed
# twice, Frank answering twice). So this holds the daemon to exactly one — revive when none is up, and shed
# the extras (keeping the NEWEST) when more than one is. Keep-newest, not keep-oldest: the common cause of a
# second instance is a fresh deploy started alongside a not-yet-exited old one, so the newest process is the
# intended (just-deployed) binary — keeping the oldest would kill the deploy and revert to the stale binary
# (v-slack-bridge, 2026-09-30). A missing PROCESS is the only revive trigger; a stale
# ~/.midway/cookie is NOT (the daemon starts fine without it and simply cannot post/read until it is
# refreshed), so this never thrash-revives on a cookie expiry.
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
now() { date -Is 2>/dev/null || echo now; }

BRIDGE_DIR="${HOME}/membrain-skynet-bridge"
BRIDGE_BIN="${BRIDGE_DIR}/target/debug/membrain-skynet-bridge"
BOARD_API="http://127.0.0.1:8880/board/api"
STATE_DIR="${HOME}/.local/state/membrain-skynet-bridge"
# task_499 guard-side: the daemon rewrites this health file every tick (updated_epoch + a degraded flag).
HEALTH="$STATE_DIR/health.json"
# How long health.json may go un-updated before a still-UP process counts as WEDGED (alive but not ticking).
# Generous (5 min >> the per-tick write cadence) so a brief hiccup never trips it; env-overridable.
HEALTH_STALE_SECS="${CDZ_BRIDGE_HEALTH_STALE_SECS:-300}"

# The live daemon's PIDs, counted PRECISELY. A process counts only if BOTH its argv carries the instance
# anchor (`--bridge-instance membrain`) AND its executable IS the membrain binary — so a shell, an observer, a
# `ps`/grep pipeline, or this guard's own pgrep that merely MENTIONS the string in its command line is never
# miscounted. That precision matters: the count below drives a kill branch, and a false positive there would
# terminate an innocent process.
bridge_pids() {
  local p exe
  for p in $(pgrep -f -- '--bridge-instance membrain' 2>/dev/null || true); do
    # /proc/<pid>/exe is an absolute symlink to the running binary. Use PLAIN `readlink` (not -f): after a
    # rebuild the target reads ".../membrain-skynet-bridge (deleted)", and `readlink -f` canonicalizes a
    # non-existent target to EMPTY — which silently dropped a live daemon running an older binary from the
    # count, so the guard could neither see it nor shed it: an invisible DOUBLE RELAY. Strip a trailing
    # " (deleted)" before the basename match so a fresh AND a rebuilt-binary daemon are both counted.
    exe="$(readlink "/proc/$p/exe" 2>/dev/null || true)"
    exe="${exe% (deleted)}"
    case "$exe" in
      */membrain-skynet-bridge) printf '%s\n' "$p" ;;
    esac
  done
}

# Order PIDs newest-first by kernel start time (/proc/<pid>/stat field 22, higher = started later), so a
# kill-extras keeps the just-started (freshest-deployed) instance and sheds the older duplicates. A PID whose
# /proc/<pid>/stat is unreadable sorts to start-time 0 (oldest), so it is never the one kept.
newest_first() {
  local p st
  for p in "$@"; do
    st="$(awk '{print $22}' "/proc/$p/stat" 2>/dev/null || echo 0)"
    printf '%s %s\n' "$st" "$p"
  done | sort -rn | awk '{print $2}'
}

mapfile -t PIDS < <(bridge_pids)
COUNT="${#PIDS[@]}"

if [ "$COUNT" -eq 1 ]; then
  # One daemon is UP — but "up" is not "healthy" (task_499 guard-side). Cross-check its health.json: the daemon
  # rewrites $HEALTH every tick with updated_epoch + a degraded flag. FAIL-SAFE: act ONLY on a cleanly-parsed
  # signal — a missing / unreadable / unparseable health.json falls through to the original "one daemon =
  # healthy" path, so a parse hiccup never kills or false-alarms a live daemon.
  hd_updated=""; hd_degraded=""
  if [ -r "$HEALTH" ] && command -v jq >/dev/null 2>&1; then
    hd_updated="$(jq -r '.updated_epoch // empty' "$HEALTH" 2>/dev/null || true)"
    hd_degraded="$(jq -r '.degraded // empty' "$HEALTH" 2>/dev/null || true)"
  fi
  if [[ "$hd_updated" =~ ^[0-9]+$ ]] && [ "$(( $(date +%s) - hd_updated ))" -gt "$HEALTH_STALE_SECS" ]; then
    # WEDGED: the process is alive (COUNT==1) but health.json stopped advancing — stuck, not ticking (a
    # process-count check alone misreads this as healthy). Kill it and FALL THROUGH to the revive block below
    # (relaunch), i.e. treat it as COUNT==0. Numeric-gated + a generous threshold, so only a genuinely stale
    # epoch acts. The revive block writes the authoritative stamp/alarm; the journal line records the reason.
    echo "$(now): slack-bridge pid ${PIDS[0]} WEDGED (health.json idle $(( $(date +%s) - hd_updated ))s > ${HEALTH_STALE_SECS}s) — killing + reviving"
    kill "${PIDS[0]}" 2>/dev/null || true
    # (deliberately NO exit — fall through to the COUNT==0 revive block)
  elif [ "$hd_degraded" = "true" ]; then
    # DEGRADED but alive + ticking: raise an ops status, do NOT revive — the daemon is up; reviving won't fix an
    # expired auth / rate-limit and would only thrash. A human / v-slack-bridge acts on the reason.
    reason="$(jq -r '.degraded_reason // "unspecified"' "$HEALTH" 2>/dev/null || echo unspecified)"
    printf '%s: slack-bridge pid %s reports DEGRADED (%s) — up but unhealthy; NOT auto-revived (would thrash). A human/v-slack-bridge should check.\n' \
      "$(now)" "${PIDS[0]}" "$reason" > "$ALARM" 2>/dev/null || true
    printf '%s bridge=DEGRADED pid=%s reason=%s\n' "$(now)" "${PIDS[0]}" "$reason" > "$STAMP" 2>/dev/null || true
    exit 0
  else
    # Healthy: one daemon, ticking (fresh health.json — or none to parse, the fail-safe case), not degraded.
    rm -f "$ALARM" 2>/dev/null || true
    printf '%s bridge=up pid=%s\n' "$(now)" "${PIDS[0]}" > "$STAMP" 2>/dev/null || true
    exit 0
  fi
fi

if [ "$COUNT" -ge 2 ]; then
  # More than one daemon → double relay. Keep the newest (the just-deployed binary), kill the rest (SIGTERM —
  # the daemon exits cleanly).
  mapfile -t ORDERED < <(newest_first "${PIDS[@]}")
  keep="${ORDERED[0]}"
  killed=""
  for p in "${ORDERED[@]:1}"; do
    kill "$p" 2>/dev/null && killed="${killed}${killed:+,}${p}"
  done
  printf '%s: slack-bridge had %s instances (double relay) — kept newest pid %s, killed %s. A human/v-slack-bridge should check why a second instance started.\n' \
    "$(now)" "$COUNT" "$keep" "${killed:-none}" > "$ALARM" 2>/dev/null || true
  printf '%s bridge=MULTI kept=%s killed=%s\n' "$(now)" "$keep" "${killed:-none}" > "$STAMP" 2>/dev/null || true
  exit 0
fi

# COUNT == 0 → the daemon is down. Revive it from its own directory, detached (it re-parents to init). The
# subshell CLOSES fd 9 FIRST (`exec 9>&-`) so neither the subshell NOR the launched daemon inherits + holds
# this guard's singleton lock. A child holding fd 9 is exactly what wedged the guard: the revive's subshell
# OUTLIVED the fire still holding the lock, so every later `flock -n 9` failed and the guard skipped before
# stamping (`fleet status` read STALE while cron kept firing, and duplicate daemons piled up). Closing fd 9 at
# the SUBSHELL level (not only on the setsid command, which left the subshell itself holding it — the leaked
# holder observed) is the fix. A missing/unbuilt binary means it cannot be revived here — alarm for a human.
if [ -x "$BRIDGE_BIN" ]; then
  ( exec 9>&- 2>/dev/null; cd "$BRIDGE_DIR" && setsid "$BRIDGE_BIN" \
      --board-api "$BOARD_API" \
      --bridge-instance membrain \
      --state-dir "$STATE_DIR" \
      >/dev/null 2>&1 </dev/null & )
  printf '%s: slack-bridge (membrain daemon) was DOWN — relaunched %s. The operator alert path (concierge-down #8931) routes through it, so a human should confirm it recovered.\n' \
    "$(now)" "$BRIDGE_BIN" > "$ALARM" 2>/dev/null || true
  printf '%s bridge=DOWN ran-revive=membrain\n' "$(now)" > "$STAMP" 2>/dev/null || true
else
  printf '%s: slack-bridge (membrain daemon) DOWN and %s is missing/not executable — cannot auto-revive; a human must restart the bridge (the operator alert path is DOWN).\n' \
    "$(now)" "$BRIDGE_BIN" > "$ALARM" 2>/dev/null || true
  printf '%s bridge=DOWN no-binary\n' "$(now)" > "$STAMP" 2>/dev/null || true
fi
exit 0
