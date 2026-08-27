#!/usr/bin/env bash
# prune-tmp-inodes.sh — reclaim /tmp INODES by removing STALE Claude task-transcript files.
#
# WHY: /tmp is a tmpfs with a FIXED inode budget (~1M here) independent of its byte capacity. Claude
# subagent task transcripts (`<session>/tasks/*.output`, per-message/agent `*.jsonl`) plus per-command
# `/tmp/claude-*-cwd` cwd-capture files are tiny but NUMEROUS, and accumulate across ~20 active agents.
# They can exhaust /tmp's inodes at low BYTE usage (seen: 100% inodes / 16% bytes) — every agent's Bash
# then fails ENOSPC (it cannot write its output file) and the fleet wedges. `prune-stale-targets.sh`
# only reclaims worktree `target/` dirs on /local, so this covers the distinct /tmp-inode class.
#
# SAFETY (three nets):
#   1. THRESHOLD-GATED: sweeps only when /tmp inode-use% >= INODE_THRESHOLD_PCT (default 80). Under
#      normal load it is a no-op, so it never races live transcripts needlessly.
#   2. AGE-GUARDED: removes only files whose mtime is older than STALE_MIN (default 120m). An ACTIVE
#      transcript is being written (recent mtime), so a conservative age protects it. Note a single
#      `/tmp/claude-<pid>/` dir is SHARED across many sessions (not tied to one agent's liveness), so
#      age — not pid-liveness — is the correct signal.
#   3. SCOPE: touches only Claude task ephemera — `*.output`/`*.jsonl` under `/tmp/claude-*/` and the
#      top-level `/tmp/claude-*-cwd` capture files. It EXCLUDES `journal.jsonl` (the Workflow RESUME
#      journal) so a paused workflow can still resume. Nothing else in /tmp is ever touched.
#
# DRY-RUN by default (prints WOULD-REMOVE counts). Pass --apply to actually delete.
# Meant to be run periodically (e.g. a maintenance cron) from the materialized hub copy.
set -euo pipefail

TMPDIR_ROOT="${TMPDIR_ROOT:-/tmp}"
INODE_THRESHOLD_PCT="${INODE_THRESHOLD_PCT:-80}"  # only sweep when /tmp inode-use% is at/above this
STALE_MIN="${STALE_MIN:-120}"                     # only remove transcript files older than this (minutes)

APPLY=0
[ "${1:-}" = "--apply" ] && APPLY=1

# /tmp inode use% — column 5 of `df -i`, with the trailing '%' stripped.
iuse="$(df -i "$TMPDIR_ROOT" | awk 'NR==2 {gsub(/%/,"",$5); print $5}')"
iuse="${iuse:-0}"
printf 'prune-tmp-inodes: %s inode-use=%s%% threshold=%s%% stale=%smin apply=%s\n' \
  "$TMPDIR_ROOT" "$iuse" "$INODE_THRESHOLD_PCT" "$STALE_MIN" "$APPLY"

if [ "$iuse" -lt "$INODE_THRESHOLD_PCT" ]; then
  printf 'prune-tmp-inodes: inode-use %s%% below threshold %s%% — nothing to do.\n' "$iuse" "$INODE_THRESHOLD_PCT"
  exit 0
fi

# The Claude task-root dirs (`/tmp/claude-<pid>/`). Enumerate as DIRS so the top-level `claude-*-cwd`
# FILES are not treated as roots. `-print0`/read handles the (unlikely) space in a path safely.
roots=()
while IFS= read -r -d '' r; do roots+=("$r"); done \
  < <(find "$TMPDIR_ROOT" -maxdepth 1 -type d -name 'claude-*' -print0 2>/dev/null)

# Count + delete the stale transcript files under the task-root dirs.
transcripts=0
if [ "${#roots[@]}" -gt 0 ]; then
  transcripts="$(find "${roots[@]}" -type f \( -name '*.output' -o -name '*.jsonl' \) \
    ! -name 'journal.jsonl' -mmin +"$STALE_MIN" 2>/dev/null | wc -l)"
fi
# The top-level per-command cwd-capture files (`/tmp/claude-<hex>-cwd`).
cwds="$(find "$TMPDIR_ROOT" -maxdepth 1 -type f -name 'claude-*-cwd' -mmin +"$STALE_MIN" 2>/dev/null | wc -l)"

if [ "$APPLY" = 1 ]; then
  if [ "${#roots[@]}" -gt 0 ]; then
    find "${roots[@]}" -type f \( -name '*.output' -o -name '*.jsonl' \) \
      ! -name 'journal.jsonl' -mmin +"$STALE_MIN" -delete 2>/dev/null || true
    # Reclaim the now-empty transcript dir trees too (safe: only empty dirs).
    find "${roots[@]}" -type d -empty -delete 2>/dev/null || true
  fi
  find "$TMPDIR_ROOT" -maxdepth 1 -type f -name 'claude-*-cwd' -mmin +"$STALE_MIN" -delete 2>/dev/null || true
  after="$(df -i "$TMPDIR_ROOT" | awk 'NR==2 {gsub(/%/,"",$5); print $5}')"
  printf 'prune-tmp-inodes: removed %s transcript file(s) + %s cwd file(s) + empty dirs; inode-use now %s%%\n' \
    "$transcripts" "$cwds" "${after:-?}"
else
  printf 'prune-tmp-inodes: WOULD remove %s transcript file(s) + %s cwd file(s) (>%smin, .output/.jsonl excl journal.jsonl, + claude-*-cwd); rerun with --apply\n' \
    "$transcripts" "$cwds" "$STALE_MIN"
fi
