#!/usr/bin/env bash
# prune-tmp-inodes.sh — reclaim /tmp INODES from stale ephemera that accumulate without cleanup.
#
# WHY: /tmp is a tmpfs with a FIXED inode budget (~1M here) independent of its byte capacity. Two
# classes of tiny-but-numerous files exhaust its inodes at low BYTE usage (seen: 100% inodes / 16%
# bytes), after which every agent's Bash fails ENOSPC (it cannot write its output file) and the fleet
# wedges:
#   A. TOOLBOX TELEMETRY (the PRIMARY accumulator, operator-confirmed): `/tmp/toolbox-telemetry-*` dirs
#      created ~2/min by the internal toolbox EMF wrapper, each holding a few log/metric files, with NO
#      cleanup — hundreds pile up per hour.
#   B. CLAUDE TASK TRANSCRIPTS: `*.output`/`*.jsonl` under `/tmp/claude-<pid>/<session>/…` plus the
#      per-command `/tmp/claude-*-cwd` capture files, across ~20 active agents.
# `prune-stale-targets.sh` only reclaims worktree `target/` dirs on /local, so this covers the distinct
# /tmp-inode class.
#
# SAFETY (three nets):
#   1. THRESHOLD-GATED: sweeps only when /tmp inode-use% >= INODE_THRESHOLD_PCT (default 80). Under
#      normal load it is a no-op, so it never races live files needlessly.
#   2. AGE-GUARDED: removes only entries older than a per-class age — a live telemetry buffer / active
#      transcript has a recent mtime. TELEMETRY_STALE_MIN (default 30; operator: >~20min = flushed/safe)
#      and STALE_MIN (default 120) are separate knobs. NOTE a `/tmp/claude-<pid>/` dir is SHARED across
#      many sessions (not tied to one agent's liveness), so AGE — not pid-liveness — is the signal.
#   3. SCOPE: touches ONLY those two ephemeral classes. For Claude it excludes `journal.jsonl` (the
#      Workflow RESUME journal) so a paused workflow can still resume. Nothing else in /tmp is touched.
#
# DRY-RUN by default (prints WOULD-REMOVE counts). Pass --apply to actually delete.
# Meant to be run periodically (e.g. a maintenance cron) from the materialized hub copy.
set -euo pipefail

TMPDIR_ROOT="${TMPDIR_ROOT:-/tmp}"
INODE_THRESHOLD_PCT="${INODE_THRESHOLD_PCT:-80}"   # only sweep when /tmp inode-use% is at/above this
TELEMETRY_STALE_MIN="${TELEMETRY_STALE_MIN:-30}"   # remove toolbox-telemetry-* older than this (minutes)
STALE_MIN="${STALE_MIN:-120}"                      # remove claude task transcripts older than this (minutes)

iuse_pct() { df -i "$TMPDIR_ROOT" | awk 'NR==2 {gsub(/%/,"",$5); print $5}'; }

APPLY=0
[ "${1:-}" = "--apply" ] && APPLY=1

iuse="$(iuse_pct)"
iuse="${iuse:-0}"
printf 'prune-tmp-inodes: %s inode-use=%s%% threshold=%s%% telemetry-stale=%smin claude-stale=%smin apply=%s\n' \
  "$TMPDIR_ROOT" "$iuse" "$INODE_THRESHOLD_PCT" "$TELEMETRY_STALE_MIN" "$STALE_MIN" "$APPLY"

if [ "$iuse" -lt "$INODE_THRESHOLD_PCT" ]; then
  printf 'prune-tmp-inodes: inode-use %s%% below threshold %s%% — nothing to do.\n' "$iuse" "$INODE_THRESHOLD_PCT"
  exit 0
fi

# ── Class A: toolbox EMF telemetry buffers (`/tmp/toolbox-telemetry-*`, whole dirs). ─────────────────
telemetry="$(find "$TMPDIR_ROOT" -maxdepth 1 -name 'toolbox-telemetry-*' -mmin +"$TELEMETRY_STALE_MIN" 2>/dev/null | wc -l)"

# ── Class B: Claude task-root dirs (`/tmp/claude-<pid>/`). Enumerate as DIRS so the top-level ────────
# `claude-*-cwd` FILES are not treated as roots. `-print0`/read handles a (unlikely) space in a path.
roots=()
while IFS= read -r -d '' r; do roots+=("$r"); done \
  < <(find "$TMPDIR_ROOT" -maxdepth 1 -type d -name 'claude-*' -print0 2>/dev/null)
transcripts=0
if [ "${#roots[@]}" -gt 0 ]; then
  transcripts="$(find "${roots[@]}" -type f \( -name '*.output' -o -name '*.jsonl' \) \
    ! -name 'journal.jsonl' -mmin +"$STALE_MIN" 2>/dev/null | wc -l)"
fi
cwds="$(find "$TMPDIR_ROOT" -maxdepth 1 -type f -name 'claude-*-cwd' -mmin +"$STALE_MIN" 2>/dev/null | wc -l)"

if [ "$APPLY" = 1 ]; then
  # A: remove whole stale telemetry dirs (they are self-contained buffers).
  find "$TMPDIR_ROOT" -maxdepth 1 -name 'toolbox-telemetry-*' -mmin +"$TELEMETRY_STALE_MIN" \
    -exec rm -rf {} + 2>/dev/null || true
  # B: remove stale claude transcript files, then reclaim their emptied dir trees + the cwd files.
  if [ "${#roots[@]}" -gt 0 ]; then
    find "${roots[@]}" -type f \( -name '*.output' -o -name '*.jsonl' \) \
      ! -name 'journal.jsonl' -mmin +"$STALE_MIN" -delete 2>/dev/null || true
    find "${roots[@]}" -type d -empty -delete 2>/dev/null || true
  fi
  find "$TMPDIR_ROOT" -maxdepth 1 -type f -name 'claude-*-cwd' -mmin +"$STALE_MIN" -delete 2>/dev/null || true
  after="$(iuse_pct)"
  printf 'prune-tmp-inodes: removed %s telemetry dir(s) + %s transcript file(s) + %s cwd file(s) + empty dirs; inode-use now %s%%\n' \
    "$telemetry" "$transcripts" "$cwds" "${after:-?}"
else
  printf 'prune-tmp-inodes: WOULD remove %s telemetry dir(s) (>%smin) + %s transcript file(s) + %s cwd file(s) (>%smin, excl journal.jsonl); rerun with --apply\n' \
    "$telemetry" "$TELEMETRY_STALE_MIN" "$transcripts" "$cwds" "$STALE_MIN"
fi
