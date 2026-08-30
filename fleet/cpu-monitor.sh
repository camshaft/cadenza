#!/usr/bin/env bash
# cpu-monitor.sh — sample the heaviest processes into a ROTATING, BOUNDED log; `--report` aggregates the
# most-repeated TASKS so we can find "rebuilding the world" patterns to cache.
#
# WHY (operator directive 2026-08-29, v-fleet-tooling lane): the fleet burns CPU re-running the same
# heavy tasks across ~30 worktrees — e.g. every worktree compiling `rustc --crate-name cdz`/`rcdzc` into
# its OWN target/ (a redundant rebuild the shared nix store/CA-cache could serve), plus concurrent
# gate-local/corpus nix builds that starve each other. This daemon samples `ps` periodically into a
# rotating log; `--report` turns "task X ran in N samples across W worktrees using C CPU" into an
# actionable hotspot list. v-fleet-tooling owns the monitor + report; v-nix consumes the log to drive
# the caching fixes it reveals.
#
# BOUNDED like the prune crons: the log is capped to CDZ_CPU_MONITOR_MAX_LINES (tail-rotated each sample),
# per-sample only the TOP_N heaviest processes above MIN_PCPU are logged, and each cmdline is length-capped
# — so this monitor never becomes its own disk/inode hog.
#
# RUN: `cpu-monitor.sh` takes ONE snapshot + rotates (wire to a per-2min user crontab — autonomous, no
# agent involvement). `cpu-monitor.sh --report [N]` prints the top-N repeated tasks. FAIL-OPEN: any error
# exits 0 so a sampler tick never spams cron mail.
set -uo pipefail

LOG_DIR="${CDZ_CPU_MONITOR_DIR:-$HOME/.cdz-cpu-monitor}"
LOG="$LOG_DIR/samples.tsv"
MAX_LINES="${CDZ_CPU_MONITOR_MAX_LINES:-200000}"   # rotate: keep only the last N lines (~tens of MB cap)
TOP_N="${CDZ_CPU_MONITOR_TOP_N:-25}"               # per sample: log the N heaviest processes
MIN_PCPU="${CDZ_CPU_MONITOR_MIN_PCPU:-10}"         # ignore processes below this %CPU (noise floor)
CMD_CAP="${CDZ_CPU_MONITOR_CMD_CAP:-500}"          # cap each logged cmdline to N chars (bound line size)

REAP_MIN="${CDZ_ORPHAN_CDZ_REAP_MIN:-30}"          # reap ORPHANED (PPID 1) `cdz` procs older than this (min)
REAP_LOG="$LOG_DIR/reap.log"                        # bounded audit of reaped orphans (mtime = last-fired proof)
REAP_LOG_MAX="${CDZ_REAP_LOG_MAX:-2000}"

mkdir -p "$LOG_DIR" 2>/dev/null || exit 0

# Light per-LOG-TIME normalization: collapse nix-store hashes + /tmp scratch (huge variance, low value),
# KEEP the worktree path (so the report can count DISTINCT worktrees = the redundant-rebuild signal).
log_normalize() {
  sed -E \
    -e 's#/nix/store/[a-z0-9]{32}-#/nix/store/HASH-#g' \
    -e 's#/tmp/[A-Za-z0-9._-]+#/tmp/X#g'
}

# Reap ORPHANED (PPID 1) `cdz` front-end procs older than REAP_MIN (concierge fleet-health ask 2026-08-30).
# WHY: a leaked warm-compile whose owning window/agent DIED gets reparented to init (PPID 1) — no agent
# owns it, so nothing reaps it; it pegs a core for HOURS and starves the gate check-lease (observed: an
# orphaned cdz at 99.7% for 1h34m starved gates ~1.5h before a hand-reap). PPID 1 + own-user makes it
# UNAMBIGUOUS: the owner is gone, there is no one to coordinate a graceful stop with, so a leaked hung
# front-end is safe to kill outright. Deliberately SCOPED to comm == exactly `cdz` (the front-end binary),
# NEVER cdz-run / cdz-compile / rcdzc, and NEVER an OWNED (live-parent) hung cdz — that latter class is
# escalate-to-owner policy, not this reaper's to kill. arg1: apply (1 = SIGKILL, 0 = dry-run report only).
# FAIL-OPEN: any hiccup just returns (a monitor tick must never error out or spam cron mail).
reap_orphaned_cdz() {
  local apply="${1:-0}" me min_secs reaped=0 seen=0 pid ppid euid et comm n
  me="$(id -u 2>/dev/null || echo -1)"
  min_secs=$(( REAP_MIN * 60 ))
  # pid ppid euid etimes comm — comm is the executable basename (holds `cdz` untruncated). `read` word-splits.
  while read -r pid ppid euid et comm; do
    [ -n "$pid" ] || continue
    [ "$ppid" = "1" ] || continue          # ORPHANED — reparented to init (owning window/agent died)
    [ "$euid" = "$me" ] || continue        # own-user only (never touch a peer uid's proc)
    [ "$comm" = "cdz" ] || continue        # exactly the front-end binary (not cdz-run/cdz-compile/rcdzc)
    [ "${et:-0}" -ge "$min_secs" ] 2>/dev/null || continue
    seen=$((seen + 1))
    if [ "$apply" = 1 ]; then
      if kill -KILL "$pid" 2>/dev/null; then
        reaped=$((reaped + 1))
        printf '%s reaped orphaned cdz pid=%s ppid=1 etimes=%ss (>=%smin, own-user)\n' \
          "$(date -Is 2>/dev/null || echo now)" "$pid" "$et" "$REAP_MIN" >> "$REAP_LOG" 2>/dev/null || true
      fi
    else
      printf 'cpu-monitor: WOULD reap orphaned cdz pid=%s ppid=1 etimes=%ss (>=%smin, own-user)\n' \
        "$pid" "$et" "$REAP_MIN" >&2
    fi
  done < <(ps -eo pid=,ppid=,euid=,etimes=,comm= 2>/dev/null)
  # Bound the audit log (tail-rotate) so it can never become its own disk hog.
  if [ "$apply" = 1 ] && [ -f "$REAP_LOG" ]; then
    n="$(wc -l < "$REAP_LOG" 2>/dev/null || echo 0)"
    if [ "${n:-0}" -gt "$REAP_LOG_MAX" ]; then
      tail -n "$REAP_LOG_MAX" "$REAP_LOG" > "$REAP_LOG.rot" 2>/dev/null && mv "$REAP_LOG.rot" "$REAP_LOG" 2>/dev/null || true
    fi
    [ "$reaped" -gt 0 ] && printf 'cpu-monitor: reaped %s orphaned cdz proc(s) (>=%smin, own-user)\n' "$reaped" "$REAP_MIN" >&2
  else
    printf 'cpu-monitor: %s orphaned cdz candidate(s) (dry-run; pass --apply to reap)\n' "$seen" >&2
  fi
  return 0
}

case "${1:-sample}" in
  --reap-orphans | reap-orphans)
    # Manual inspection / operator drive. DRY-RUN by default (lists candidates); `--apply` actually reaps.
    _apply=0
    [ "${2:-}" = "--apply" ] && _apply=1
    reap_orphaned_cdz "$_apply"
    exit 0
    ;;
  --report | report)
    REPORT_N="${2:-25}"   # `cpu-monitor.sh --report [N]` — N top tasks (default 25)
    if [ ! -s "$LOG" ]; then
      echo "cpu-monitor: no samples yet at $LOG (sampler not run, or nothing above ${MIN_PCPU}% CPU)."
      exit 0
    fi
    total_samples="$(cut -f1 "$LOG" 2>/dev/null | sort -u | wc -l)"
    span_lines="$(wc -l < "$LOG" 2>/dev/null || echo 0)"
    printf 'cpu-monitor report: %s distinct sample-times, %s logged process-rows, log=%s\n' \
      "$total_samples" "$span_lines" "$LOG"
    echo "── TOP repeated TASKS (worktree-collapsed) — rows = process-samples, wts = distinct worktrees, cpu% = summed ──"
    printf '%8s %5s %10s  %s\n' "rows" "wts" "sum_cpu%" "task"
    # Field 4 = cmd (worktree KEPT). Collapse the worktree to <wt> for the TASK key, but track the distinct
    # worktree NAME per task so cross-worktree redundancy is visible. awk aggregates in one pass.
    awk -F'\t' '
      {
        cmd = $4
        # extract worktree name (…/.claude/worktrees/<name>/…) before collapsing, for the distinct count
        wt = "-"
        if (match(cmd, /\.claude\/worktrees\/[^/]+/)) {
          wt = substr(cmd, RSTART+18, RLENGTH-18)
        }
        task = cmd
        gsub(/\/[^ ]*\.claude\/worktrees\/[^/ ]+\//, "<wt>/", task)
        rows[task]++
        cpu[task] += $2
        key = task SUBSEP wt
        if (!(key in seen)) { seen[key] = 1; wts[task]++ }
      }
      END {
        for (t in rows) printf "%d\t%d\t%.0f\t%s\n", rows[t], wts[t], cpu[t], t
      }
    ' "$LOG" | sort -t"$(printf '\t')" -k1,1 -rn | head -n "$REPORT_N" \
      | while IFS="$(printf '\t')" read -r rows wts cpu task; do
          printf '%8s %5s %10s  %.160s\n' "$rows" "$wts" "$cpu" "$task"
        done
    exit 0
    ;;
  *)
    # First, reap any ORPHANED (PPID 1) hung `cdz` front-end (leaked warm-compile whose owner died). This
    # runs on every 2-min sampler tick — no separate cron needed — and is fail-open, so a monitor tick both
    # OBSERVES the hotspots and CLEARS the owner-less ones that starve the gate lease. Set
    # CDZ_ORPHAN_CDZ_REAP_MIN=0 to widen or a huge value to effectively disable.
    reap_orphaned_cdz 1

    # ONE snapshot: the TOP_N heaviest processes above MIN_PCPU, appended as ts<TAB>pcpu<TAB>etimes<TAB>cmd.
    ts="$(date +%s 2>/dev/null || echo 0)"
    ps -eo pcpu=,etimes=,args= --sort=-pcpu 2>/dev/null \
      | awk -v n="$TOP_N" -v min="$MIN_PCPU" -v ts="$ts" -v cap="$CMD_CAP" '
          ($1 + 0) >= min {
            pcpu = $1; et = $2
            $1 = ""; $2 = ""; cmd = $0
            sub(/^[ \t]+/, "", cmd)
            cmd = substr(cmd, 1, cap)
            print ts "\t" pcpu "\t" et "\t" cmd
            if (++c >= n) exit
          }
        ' \
      | log_normalize >> "$LOG" 2>/dev/null || true

    # Rotate: keep only the last MAX_LINES (bounded — never a disk hog).
    lines="$(wc -l < "$LOG" 2>/dev/null || echo 0)"
    if [ "${lines:-0}" -gt "$MAX_LINES" ]; then
      tail -n "$MAX_LINES" "$LOG" > "$LOG.rot" 2>/dev/null && mv "$LOG.rot" "$LOG" 2>/dev/null || true
    fi
    exit 0
    ;;
esac
