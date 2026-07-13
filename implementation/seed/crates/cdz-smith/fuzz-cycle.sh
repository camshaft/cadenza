#!/usr/bin/env bash
# fuzz-cycle.sh — one cron cycle of continuous compiler fuzzing.
#
# Each cycle: sync a dedicated worktree to the latest `spec`, rebuild cdz-smith against THAT
# compiler, and run a time-boxed fuzz batch. Findings (a runnable `.sexp` + a `.md` triage note per
# distinct crash SITE, deduped) are written into the MAIN checkout's `spec/semantics/failures/` —
# the queue the semantics-failures monitoring loop already watches and fixes. So "pull in the latest
# compiler and fuzz it continuously, dumping crashes/timeouts for an agent to triage" is exactly one
# invocation of this script on a timer.
#
# Design notes:
#   * The fuzzer runs in a SUBPROCESS with an external hard `timeout`, on top of cdz-smith's own
#     in-process watchdog. Two layers: the watchdog files a Timeout finding + aborts on a single
#     hung compile (so we learn WHICH program hung); the outer `timeout` is a backstop that bounds
#     the whole cycle no matter what. A non-zero exit from either is normal (a finding was surfaced,
#     or the batch was cut off) — NOT a script failure.
#   * Findings go to the main checkout by absolute path, because that is the working tree the
#     monitoring cron reads; the worktree's own copy would be watched by nobody.
#   * Everything is best-effort and idempotent: a failed sync/build just skips this cycle; the next
#     one retries against whatever `spec` has become.
#
# Env overrides (all optional):
#   CDZ_SMITH_MAIN        main checkout root (default: the repo this script lives in)
#   CDZ_SMITH_WORKTREE    fuzzing worktree dir (default: <main>/.claude/worktrees/cdz-smith)
#   CDZ_SMITH_ITERATIONS  programs per cycle    (default: 50000)
#   CDZ_SMITH_TIMEOUT     per-compile budget, s (default: 10)
#   CDZ_SMITH_CYCLE_CAP   outer wall-clock cap  (default: 480 = 8 min, under a 10-min cron)
set -uo pipefail

# ── locate the checkouts ──────────────────────────────────────────────────────────────────────
# This script lives at <root>/implementation/seed/crates/cdz-smith/fuzz-cycle.sh; walk up to <root>.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEFAULT_ROOT="$(cd "$SCRIPT_DIR/../../../.." && pwd)"
MAIN="${CDZ_SMITH_MAIN:-$DEFAULT_ROOT}"
WORKTREE="${CDZ_SMITH_WORKTREE:-$MAIN/.claude/worktrees/cdz-smith}"
FINDINGS="$MAIN/spec/semantics/failures"

ITERATIONS="${CDZ_SMITH_ITERATIONS:-50000}"
TIMEOUT_S="${CDZ_SMITH_TIMEOUT:-10}"
CYCLE_CAP="${CDZ_SMITH_CYCLE_CAP:-480}"

log() { echo "[fuzz-cycle $(date -u +%H:%M:%S)] $*"; }

# ── 1. sync the worktree to the latest spec ─────────────────────────────────────────────────────
# The worktree tracks `spec` so we always fuzz HEAD. Use fetch + reset (not pull) so a diverged or
# dirty worktree is force-realigned rather than left on a stale/conflicted state.
if [ ! -d "$WORKTREE" ]; then
  log "creating fuzzing worktree at $WORKTREE"
  git -C "$MAIN" worktree add -B cdz-smith-fuzz "$WORKTREE" refs/heads/spec || {
    log "worktree add failed; skipping cycle"; exit 0; }
fi

git -C "$WORKTREE" fetch --quiet "$MAIN" spec 2>/dev/null
if ! git -C "$WORKTREE" reset --hard refs/heads/spec --quiet 2>/dev/null; then
  # Fall back to whatever `spec` the main repo has locally.
  SPEC_SHA="$(git -C "$MAIN" rev-parse spec 2>/dev/null)"
  git -C "$WORKTREE" reset --hard "$SPEC_SHA" --quiet 2>/dev/null || {
    log "could not sync worktree to spec; skipping cycle"; exit 0; }
fi
COMMIT="$(git -C "$WORKTREE" rev-parse --short HEAD 2>/dev/null || echo unknown)"
log "fuzzing spec @$COMMIT | $ITERATIONS iters | ${TIMEOUT_S}s/compile | cap ${CYCLE_CAP}s"

# ── 2. build the fuzzer against THIS compiler ───────────────────────────────────────────────────
# release-debug = optimized (fast fuzzing) but keeps line info, so a panic backtrace symbolizes to
# the exact rcdzc crash site the finding store dedups on. No CARGO_TARGET_DIR (per repo gotcha).
if ! ( cd "$WORKTREE" && cargo build -p cdz-smith --profile release-debug 2>&1 | tail -3 ); then
  log "build failed @$COMMIT; skipping cycle (next cycle retries against newer spec)"
  exit 0
fi
BIN="$WORKTREE/target/release-debug/cdz-smith"
[ -x "$BIN" ] || { log "binary missing at $BIN; skipping"; exit 0; }

# ── 3. run a time-boxed fuzz batch ──────────────────────────────────────────────────────────────
mkdir -p "$FINDINGS"
before="$(ls "$FINDINGS"/*.smith.md 2>/dev/null | wc -l | tr -d ' ')"

# Seed the run from the epoch second so each cycle explores a different region deterministically.
RUN_SEED="$(date +%s)"

CDZ_SMITH_COMMIT="$COMMIT" timeout --signal=KILL "$CYCLE_CAP" \
  "$BIN" fuzz \
    --iterations "$ITERATIONS" \
    --seed "$RUN_SEED" \
    --timeout "$TIMEOUT_S" \
    --findings "$FINDINGS"
rc=$?

after="$(ls "$FINDINGS"/*.smith.md 2>/dev/null | wc -l | tr -d ' ')"
new=$(( after - before ))

# rc: 0 = clean batch; 1 = a new bucket surfaced (cdz-smith's own exit code); 124/137 = the outer
# timeout cut the batch off (KILL) — all EXPECTED. Report and always exit 0 so the cron treats the
# cycle as completed regardless (findings, not exit codes, are the signal the triage loop consumes).
if [ "$new" -gt 0 ]; then
  log "surfaced $new NEW finding bucket(s) → $FINDINGS (total $after)"
else
  log "no new buckets this cycle (rc=$rc, total findings $after)"
fi
exit 0
