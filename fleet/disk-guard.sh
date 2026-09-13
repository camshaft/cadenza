#!/usr/bin/env bash
# disk-guard.sh — root-FS BYTE-capacity early-warning (v-fleet-tooling, concierge-greenlit 2026-09-13).
#
# WHY: the fleet already watches /tmp INODE pressure (prune-tmp-inodes.sh), but NOTHING watched the root
# filesystem's BYTE capacity — so the 2026-09-13 root-FS-full incident slipped silently to 100% (ENOSPC
# blocked builds fleet-wide) with no early warning; it was only noticed when an agent hit ENOSPC. This is
# the missing early-warning: sample root-FS use% on a cron and RAISE an alarm well before the wall so the
# reclaim (often NOT this vertical's lane — e.g. the 855G brazil MembrainHivemind build) can be routed in
# time. It does NOT reclaim anything itself (alarm-only, concierge call): the big levers are not safely
# auto-reclaimable, and the target-reaper (prune-stale-targets.sh) already handles the cadenza recurrence.
#
# WHAT IT DOES (greenlit leans):
#   • 85% = WARN, 92% = HIGH. Below 85% = OK.
#   • ALARM FILE always reflects state: at/above WARN, (over)write `<hub>/disk-pressure.alarm` with a
#     one-line reason (level + use% + free + where-to-look); back below WARN, REMOVE it. `fleet status`
#     surfaces any `*.alarm` file, so the board shows disk pressure the moment it crosses.
#   • RATE-LIMITED concierge note ONLY on a FRESH ESCALATION (band increased vs the last recorded band:
#     OK→WARN, OK→HIGH, WARN→HIGH). No note while it stays in the same-or-lower band (dedup — no spam), and
#     recovery (→OK) just clears the alarm silently. The concierge surfaces disk pressure to the operator.
#
# Same tracked->runtime split + flock singleton + `.last-run` observability + FAIL-OPEN discipline as
# reap-leases.sh: TRACKED at <repo>/fleet/, RUN from the hub copy `fleet up` materializes into
# <hub>/.claude/fleet/ (it resolves the hub + picks a built xtask binary from its own hub location).
set -uo pipefail

WARN_PCT="${DISK_WARN_PCT:-85}"
HIGH_PCT="${DISK_HIGH_PCT:-92}"
ROOT_FS="${DISK_GUARD_FS:-/}"

# SINGLETON GUARD: flock -n so two fires never race the state/alarm files. Lock in $HOME (own-user, persists,
# not the inode-pressured /tmp). FAIL-OPEN if flock is absent.
if command -v flock >/dev/null 2>&1 && exec 9>"${HOME}/.cdz-disk-guard.lock" 2>/dev/null; then
  flock -n 9 || exit 0
fi

HUB="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ALARM="$HUB/disk-pressure.alarm"
STATE="$HUB/disk-pressure.state"     # last band: OK | WARN | HIGH (for escalation dedup)
STAMP="$HUB/disk-guard.last-run"

# Sample root FS: `df -P` gives portable columns — $5 = Capacity (NN%), $4 = Available (1K-blocks).
read -r use_pct avail_kb < <(df -P "$ROOT_FS" 2>/dev/null | awk 'NR==2 {gsub(/%/,"",$5); print $5, $4}')
use_pct="${use_pct:-0}"
avail_kb="${avail_kb:-0}"
free_h="$(awk "BEGIN{printf \"%.0fG\", $avail_kb/1024/1024}" 2>/dev/null || echo '?')"

# Current band from the thresholds.
if   [ "$use_pct" -ge "$HIGH_PCT" ]; then band="HIGH"
elif [ "$use_pct" -ge "$WARN_PCT" ]; then band="WARN"
else                                      band="OK"
fi

prev="$(cat "$STATE" 2>/dev/null || echo OK)"
case "$prev" in OK|WARN|HIGH) ;; *) prev="OK";; esac

# Rank the bands so we can detect an ESCALATION (a strictly higher band than last time).
rank() { case "$1" in HIGH) echo 2;; WARN) echo 1;; *) echo 0;; esac; }

reason="root FS ${ROOT_FS} ${use_pct}% used, ${free_h} free (warn=${WARN_PCT}% high=${HIGH_PCT}%). Likely lever: worktree target/ (prune-stale-targets.sh) or brazil builds (membrain lane). Detail: du -x --max-depth=2 \$HOME | sort -rn | head"

if [ "$band" = "OK" ]; then
  rm -f "$ALARM" 2>/dev/null || true      # recovered → clear the board alarm (silent, no note)
else
  printf 'DISK %s: %s\n' "$band" "$reason" > "$ALARM" 2>/dev/null || true
fi

# Fresh escalation → one rate-limited concierge note (routed to the concierge, who surfaces it upward).
if [ "$(rank "$band")" -gt "$(rank "$prev")" ]; then
  # Find a worktree with a BUILT xtask binary to send with (reap-leases.sh pattern) — freshest HEAD.
  WT="$(cd "$HUB/../worktrees" 2>/dev/null && pwd || true)"
  best="" best_ct=-1
  if [ -n "${WT:-}" ] && [ -d "$WT" ]; then
    for wt in "$WT"/*/; do
      bin="${wt}target/release/xtask"
      [ -x "$bin" ] || continue
      ct="$(git -C "$wt" show -s --format=%ct HEAD 2>/dev/null || echo 0)"
      if [ "$ct" -gt "$best_ct" ]; then best_ct="$ct"; best="$bin"; fi
    done
  fi
  if [ -n "$best" ]; then
    body_file="$(mktemp 2>/dev/null || echo /tmp/disk-guard-note.$$)"
    printf 'Root-FS byte pressure crossed into %s.\n%s\nAlarm is on the board (fleet status). No auto-reclaim was run (alarm-only). If the lever is cadenza worktree target/ the reaper handles it; if it is brazil/membrain builds it needs the membrain owner or a privileged sweep — routing to you to surface upward.\n' \
      "$band" "$reason" > "$body_file" 2>/dev/null || true
    "$best" fleet send --to concierge --from disk-guard --kind note \
      --subject "DISK $band: root FS ${use_pct}% used, ${free_h} free — byte-pressure early-warning (alarm-only)" \
      --body-file "$body_file" >/dev/null 2>&1 || true
    rm -f "$body_file" 2>/dev/null || true
  fi
fi

printf '%s\n' "$band" > "$STATE" 2>/dev/null || true

# SILENT-CRON OBSERVABILITY (matches reap-leases.sh / prune-*.sh): OVERWRITE `.last-run` — MTIME proves the
# (silent) cron fired, content = the last sample. Best-effort, never fails the run.
printf '%s band=%s use=%s%% free=%s\n' \
  "$(date -Is 2>/dev/null || echo now)" "$band" "$use_pct" "$free_h" \
  > "$STAMP" 2>/dev/null || true

exit 0
