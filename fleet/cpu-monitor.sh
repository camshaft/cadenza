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

# SILENT-CRON OBSERVABILITY (v-fleet-tooling 2026-09-01; matches drain-nudge #7339 / prune-*.sh / baseline-
# drift-monitor). cpu-monitor only WRITES on an event (a reap, or a deadlock-suspect sample into its own
# logs); a quiet */2 pass produces nothing, so an all-silent run is indistinguishable from a DEAD cron.
# OVERWRITE a `.last-run` next to this script on EVERY run via an EXIT trap (fires on any exit path — the
# case-branch exits included); its MTIME is the fired-proof, distinct from the event logs. Best-effort; never
# affects behavior or the exit code.
_cdz_lastrun="$(cd "$(dirname "${BASH_SOURCE[0]}")" 2>/dev/null && pwd)/cpu-monitor.last-run"
trap 'printf "%s\n" "$(date -Is 2>/dev/null || echo now)" > "$_cdz_lastrun" 2>/dev/null || true' EXIT

LOG_DIR="${CDZ_CPU_MONITOR_DIR:-$HOME/.cdz-cpu-monitor}"
LOG="$LOG_DIR/samples.tsv"
MAX_LINES="${CDZ_CPU_MONITOR_MAX_LINES:-200000}"   # rotate: keep only the last N lines (~tens of MB cap)
TOP_N="${CDZ_CPU_MONITOR_TOP_N:-25}"               # per sample: log the N heaviest processes
MIN_PCPU="${CDZ_CPU_MONITOR_MIN_PCPU:-10}"         # ignore processes below this %CPU (noise floor)
CMD_CAP="${CDZ_CPU_MONITOR_CMD_CAP:-500}"          # cap each logged cmdline to N chars (bound line size)

HEAVY_CONCURRENCY_LOG="$LOG_DIR/heavy-concurrency.tsv" # per-tick concurrent-local-gate count + summed CPU + load
HEAVY_CONCURRENCY_WARN="${CDZ_HEAVY_CONCURRENCY_WARN:-3}"   # >= this many concurrent local-gates AND low CPU → DEADLOCK-SUSPECT
HEAVY_CONCURRENCY_LOG_MAX="${CDZ_HEAVY_CONCURRENCY_MAX:-5000}"

REAP_MIN="${CDZ_ORPHAN_CDZ_REAP_MIN:-30}"          # reap ORPHANED (PPID 1) `cdz` procs older than this (min)
REAP_LOG="$LOG_DIR/reap.log"                        # bounded audit of reaped orphans (mtime = last-fired proof)
REAP_LOG_MAX="${CDZ_REAP_LOG_MAX:-2000}"

OWNED_HUNG_MIN="${CDZ_OWNED_CDZ_HUNG_MIN:-45}"      # NOTE (never kill) the owner of an OWNED cdz hung > this (min)
ESCALATE_STATE="$LOG_DIR/escalated.tsv"             # rate-limit state: pid<TAB>last-notify-epoch (bounded)
ESCALATE_COOLDOWN="${CDZ_OWNED_CDZ_COOLDOWN:-3600}" # do NOT re-notify the same pid within this many seconds
ESCALATE_STATE_MAX="${CDZ_ESCALATE_STATE_MAX:-500}"

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
  local apply="${1:-0}" me min_secs reaped=0 seen=0 pid ppid euid et comm cmd n
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
    # Capture the untruncated argv BEFORE the kill so the audit line is CLASSIFIABLE (v-compiler-perf hang-
    # classification: comm alone is just "cdz"; the full argv names WHICH compile hung — e.g. `cdz test
    # implementation/compiler-ml …`, the match->let non-termination family). /proc/<pid>/cmdline is NUL-sep;
    # translate to spaces + trim. Best-effort (a proc that exits between the ps scan and here yields empty).
    cmd="$(tr '\0' ' ' < "/proc/$pid/cmdline" 2>/dev/null)"; cmd="${cmd% }"
    if [ "$apply" = 1 ]; then
      if kill -KILL "$pid" 2>/dev/null; then
        reaped=$((reaped + 1))
        printf '%s reaped orphaned cdz pid=%s ppid=1 etimes=%ss (>=%smin, own-user) cmd=%s\n' \
          "$(date -Is 2>/dev/null || echo now)" "$pid" "$et" "$REAP_MIN" "$cmd" >> "$REAP_LOG" 2>/dev/null || true
      fi
    else
      printf 'cpu-monitor: WOULD reap orphaned cdz pid=%s ppid=1 etimes=%ss (>=%smin, own-user) cmd=%s\n' \
        "$pid" "$et" "$REAP_MIN" "$cmd" >&2
    fi
  done < <(ps -eo pid=,ppid=,euid=,etimes=,comm= 2>/dev/null)
  if [ "$apply" = 1 ]; then
    # Bound the audit log (tail-rotate) so it can never become its own disk hog.
    if [ -f "$REAP_LOG" ]; then
      n="$(wc -l < "$REAP_LOG" 2>/dev/null || echo 0)"
      if [ "${n:-0}" -gt "$REAP_LOG_MAX" ]; then
        tail -n "$REAP_LOG_MAX" "$REAP_LOG" > "$REAP_LOG.rot" 2>/dev/null && mv "$REAP_LOG.rot" "$REAP_LOG" 2>/dev/null || true
      fi
    fi
    # Summarize only when we actually reaped (a quiet apply tick prints nothing — it's the cron's hot path).
    [ "$reaped" -gt 0 ] && printf 'cpu-monitor: reaped %s orphaned cdz proc(s) (>=%smin, own-user)\n' "$reaped" "$REAP_MIN" >&2
  else
    printf 'cpu-monitor: %s orphaned cdz candidate(s) (dry-run; pass --apply to reap)\n' "$seen" >&2
  fi
  return 0
}

# Resolve the OWNING agent of a pid from its cwd: /proc/<pid>/cwd → `…/.claude/worktrees/<agent>/…` → <agent>.
# Prints the agent name + returns 0 on a match; returns non-zero (prints nothing) if it can't attribute (no
# proc, or a cwd outside a fleet worktree — in which case we must NOT send a note to a guessed recipient).
owner_of_pid() {
  local pid="$1" cwd rest
  cwd="$(readlink "/proc/$pid/cwd" 2>/dev/null)" || return 1
  case "$cwd" in
    */.claude/worktrees/*)
      rest="${cwd#*/.claude/worktrees/}"   # strip up to and incl worktrees/
      printf '%s' "${rest%%/*}"            # first path component = the agent/worktree name
      return 0 ;;
  esac
  return 1
}

# ESCALATE (NOTE, never kill) the owner of an OWNED (live-parent) `cdz` hung > OWNED_HUNG_MIN (concierge item
# 2, greenlit 2026-08-30 as a low-pri safety net). Unlike the orphan reaper, an OWNED hung cdz has a LIVE
# parent — a real agent OWNS it, so we must NOT kill it; we send its owner a NOTE so the owner coordinates the
# stop. Rate-limited per-pid (ESCALATE_COOLDOWN) so a persistently-hung proc can't spam its owner. Owner is
# resolved from the proc cwd; if it can't be attributed to a fleet worktree we SKIP (never guess a recipient).
# arg1: apply (1 = actually `fleet send` the note, 0 = dry-run — print who WOULD be notified). FAIL-OPEN.
# NOTE: deliberately NOT wired into the auto sample tick yet — owner-resolution+send wants a live own-user
# owned-hung verification first; run via `--escalate-owned [--apply]` until then.
escalate_owned_hung_cdz() {
  local apply="${1:-0}" me min_secs now seen=0 notified=0 pid ppid euid et comm owner last hub xtask_wt
  me="$(id -u 2>/dev/null || echo -1)"
  min_secs=$(( OWNED_HUNG_MIN * 60 ))
  now="$(date +%s 2>/dev/null || echo 0)"
  # The xtask workspace to run `fleet send` from: this script lives at <hub>/.claude/fleet/cpu-monitor.sh,
  # so ../.. is the hub; pr-sync's worktree always exists + holds a workspace (same pattern as window.sh).
  hub="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." 2>/dev/null && pwd)"
  xtask_wt="$hub/.claude/worktrees/pr-sync"
  while read -r pid ppid euid et comm; do
    [ -n "$pid" ] || continue
    [ "$ppid" != "1" ] || continue         # OWNED only — orphans (PPID 1) are the reaper's job, not this
    [ "$euid" = "$me" ] || continue         # own-user only
    [ "$comm" = "cdz" ] || continue         # exactly the front-end binary
    [ "${et:-0}" -ge "$min_secs" ] 2>/dev/null || continue
    owner="$(owner_of_pid "$pid")" || continue   # unattributable cwd → never guess a recipient
    [ -n "$owner" ] || continue
    seen=$((seen + 1))
    # Per-pid cooldown (last-notify epoch from the state file; awk takes the latest row for this pid).
    last="$(awk -F'\t' -v p="$pid" '$1==p{t=$2} END{if(t)print t}' "$ESCALATE_STATE" 2>/dev/null || echo)"
    if [ -n "$last" ] && [ "$((now - last))" -lt "$ESCALATE_COOLDOWN" ] 2>/dev/null; then
      continue   # notified recently — stay quiet (no spam)
    fi
    if [ "$apply" = 1 ]; then
      # Send the owner a NOTE (never a kill). Plain-text subject/body (no backticks/$() — env-leak safe).
      if [ -d "$xtask_wt" ] && ( cd "$xtask_wt" && cargo xtask fleet send --from v-fleet-tooling --to "$owner" --kind note \
            --subject "cpu-monitor: your owned cdz pid $pid has been hung ~$((et/60))min — please stop/investigate (I do NOT kill owned procs)" \
            --body "Automated fleet-health note from cpu-monitor (v-fleet-tooling). An OWNED (live-parent) cdz front-end in your worktree has been running ~$((et/60))min (>= ${OWNED_HUNG_MIN}min), which matches the compiler-ml warm-only non-termination family (match->let / recursion-lowering; v-compiler-primitives root-caused it as tuple-destructure-LET init-re-eval, fix pending). It is pegging a core and may be starving the gate lease. I do NOT kill owned procs (only ORPHANED PPID-1 leaks get reaped) — please stop or investigate pid $pid. Re-notify cooldown is ${ESCALATE_COOLDOWN}s." >/dev/null 2>&1 ); then
        notified=$((notified + 1))
        printf '%s\t%s\n' "$pid" "$now" >> "$ESCALATE_STATE" 2>/dev/null || true
        printf '%s escalated owned-hung cdz pid=%s owner=%s etimes=%ss (note-only)\n' \
          "$(date -Is 2>/dev/null || echo now)" "$pid" "$owner" "$et" >> "$REAP_LOG" 2>/dev/null || true
      fi
    else
      printf 'cpu-monitor: WOULD note owner=%s of owned-hung cdz pid=%s etimes=%ss (>=%smin, note-only, never kill)\n' \
        "$owner" "$pid" "$et" "$OWNED_HUNG_MIN" >&2
    fi
  done < <(ps -eo pid=,ppid=,euid=,etimes=,comm= 2>/dev/null)
  # Bound the rate-limit state file (tail-rotate).
  if [ -f "$ESCALATE_STATE" ]; then
    local sn; sn="$(wc -l < "$ESCALATE_STATE" 2>/dev/null || echo 0)"
    if [ "${sn:-0}" -gt "$ESCALATE_STATE_MAX" ]; then
      tail -n "$ESCALATE_STATE_MAX" "$ESCALATE_STATE" > "$ESCALATE_STATE.rot" 2>/dev/null && mv "$ESCALATE_STATE.rot" "$ESCALATE_STATE" 2>/dev/null || true
    fi
  fi
  if [ "$apply" = 1 ]; then
    [ "$notified" -gt 0 ] && printf 'cpu-monitor: escalated %s owned-hung cdz proc(s) to owner(s) (note-only)\n' "$notified" >&2
  else
    printf 'cpu-monitor: %s owned-hung cdz candidate(s) (dry-run; pass --apply to note the owner)\n' "$seen" >&2
  fi
  return 0
}

case "${1:-sample}" in
  --escalate-owned | escalate-owned)
    # Manual inspection / operator drive. DRY-RUN by default (lists owned-hung cdz + resolved owner);
    # `--apply` actually sends each owner a note (rate-limited). NOT auto-wired into the sample tick yet.
    _apply=0
    [ "${2:-}" = "--apply" ] && _apply=1
    escalate_owned_hung_cdz "$_apply"
    exit 0
    ;;
  --owner-of)
    # Tiny self-test / diagnostic: print the resolved owning agent of a pid (or nothing + rc1 if unattributable).
    owner_of_pid "${2:?usage: cpu-monitor.sh --owner-of <pid>}" && echo || { echo "cpu-monitor: pid ${2} not attributable to a fleet worktree" >&2; exit 1; }
    exit 0
    ;;
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

    # HEAVY-BUILD CONCURRENCY visibility (v-fleet-tooling 2026-08-30): the per-proc snapshot above is FLOORED
    # at MIN_PCPU, so it CANNOT SEE a gate-lock DEADLOCK — deadlocked local-gates sit at ~0% CPU (starved on
    # the nix lock), below the floor (this nearly hid a live 8-concurrent deadlock during a measurement). So
    # SEPARATELY count CONCURRENT local-gate procs (ANY CPU%, ALL uids — a deadlock is fleet-wide) + their
    # SUMMED CPU each tick, into a bounded log, so "was there a gate-lock deadlock at time T?" is answerable.
    # DEADLOCK SIGNATURE = a HIGH count with LOW summed CPU (many builds, none progressing: a progressing
    # build is ~100%+, a starved one ~0%). FAIL-OPEN (a monitor tick must never error).
    read -r lg_count lg_cpu < <(ps -eo pcpu=,args= 2>/dev/null \
      | awk '/nix build \.#checks/ && /local-gate/ && !/awk/ { c++; s += $1 } END { printf "%d %d", c, s+0 }')
    lg_count="${lg_count:-0}"; lg_cpu="${lg_cpu:-0}"
    load1="$(cut -d' ' -f1 /proc/loadavg 2>/dev/null || echo 0)"
    printf '%s\t%s\t%s\t%s\n' "$ts" "$lg_count" "$lg_cpu" "$load1" >> "$HEAVY_CONCURRENCY_LOG" 2>/dev/null || true
    # Flag a deadlock-suspect: >= WARN concurrent local-gates averaging < 20% CPU each (starved, not building).
    if [ "$lg_count" -ge "$HEAVY_CONCURRENCY_WARN" ] && [ "$lg_cpu" -lt "$((lg_count * 20))" ] 2>/dev/null; then
      printf '%s DEADLOCK-SUSPECT: %s concurrent local-gate procs at %s%% summed CPU, load %s (starved on the nix lock?)\n' \
        "$(date -Is 2>/dev/null || echo now)" "$lg_count" "$lg_cpu" "$load1" >> "$HEAVY_CONCURRENCY_LOG" 2>/dev/null || true
    fi
    # Rotate the concurrency log (bounded — never a disk hog).
    clines="$(wc -l < "$HEAVY_CONCURRENCY_LOG" 2>/dev/null || echo 0)"
    if [ "${clines:-0}" -gt "$HEAVY_CONCURRENCY_LOG_MAX" ]; then
      tail -n "$HEAVY_CONCURRENCY_LOG_MAX" "$HEAVY_CONCURRENCY_LOG" > "$HEAVY_CONCURRENCY_LOG.rot" 2>/dev/null \
        && mv "$HEAVY_CONCURRENCY_LOG.rot" "$HEAVY_CONCURRENCY_LOG" 2>/dev/null || true
    fi
    exit 0
    ;;
esac
