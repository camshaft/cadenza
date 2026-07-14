#!/usr/bin/env bash
# fuzz-cycle.sh — one cron cycle of continuous, COVERAGE-GUIDED compiler fuzzing.
#
# Each cycle: sync a dedicated worktree to the latest `spec`, then fuzz that compiler for a
# time-boxed campaign. Findings (a runnable `.sexp` + a `.md` triage note per distinct crash SITE,
# deduped) are written into the MAIN checkout's `spec/semantics/failures/` — the queue the
# semantics-failures monitoring loop already watches and fixes.
#
# TWO ENGINES, auto-selected:
#   * PREFERRED — coverage-guided libFuzzer via `cargo bolero` (needs nightly + `cargo-bolero`).
#     libFuzzer mutates a byte seed, our `generate()` decodes it into a structured program, and
#     SanitizerCoverage feedback keeps inputs that reach NEW compiler edges — driving past the
#     type-checker into the backend where the dense panic clusters live. A PERSISTENT corpus dir
#     accumulates that progress ACROSS cycles. `-fork=1` isolates a crash/hang/OOM to one child and
#     saves an artifact WITHOUT stopping the campaign (fixing the old whole-batch-abort). After the
#     campaign, `cdz-smith triage-artifacts` converts libFuzzer's crash/timeout artifacts into the
#     deduped `.sexp`/`.md` findings the queue expects.
#   * FALLBACK — the built-in PRNG driver (`cdz-smith fuzz`), used when nightly/cargo-bolero are
#     absent. Blind (no coverage), and its watchdog aborts the batch on the first hang, but it needs
#     no extra toolchain. Same findings format.
#
# Everything is best-effort + idempotent: a failed sync/build just skips this cycle; the next one
# retries against whatever `spec` has become. Findings go to the main checkout by absolute path,
# because that is the working tree the monitoring cron reads.
#
# Env overrides (all optional):
#   CDZ_SMITH_MAIN        main checkout root (default: resolved from cwd's git common-dir)
#   CDZ_SMITH_WORKTREE    fuzzing worktree dir (default: <main>/.claude/worktrees/cdz-smith-fuzz)
#   CDZ_SMITH_CYCLE_CAP   campaign wall-clock, s (default: 420 = 7 min, under a 10-min cron)
#   CDZ_SMITH_TIMEOUT     per-input compile budget, s (default: 10)
#   CDZ_SMITH_ITERATIONS  PRNG-fallback programs/cycle (default: 50000)
#   CDZ_SMITH_ENGINE      force "libfuzzer" or "prng" (default: auto-detect)
set -uo pipefail

# ── locate the checkouts ──────────────────────────────────────────────────────────────────────
# Anchor the MAIN repo on the cwd's git common-dir (parent of `.git`) — robust regardless of where
# the script FILE lives (the cron pipes it in via process substitution, so `$BASH_SOURCE` may point
# outside the repo). Fall back to a path-walk; a `CDZ_SMITH_MAIN` override wins.
CWD_COMMON="$(git -C "$PWD" rev-parse --path-format=absolute --git-common-dir 2>/dev/null)"
if [ -n "$CWD_COMMON" ]; then
  DEFAULT_ROOT="$(dirname "$CWD_COMMON")"
else
  SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" 2>/dev/null && pwd)"
  DEFAULT_ROOT="$(cd "$SCRIPT_DIR/../../../.." 2>/dev/null && pwd)"
fi
MAIN="${CDZ_SMITH_MAIN:-$DEFAULT_ROOT}"
if [ -z "$MAIN" ] || [ ! -d "$MAIN/.git" ]; then
  echo "[fuzz-cycle] cannot locate the main repo (MAIN='$MAIN'); set CDZ_SMITH_MAIN. Skipping."
  exit 0
fi
WORKTREE="${CDZ_SMITH_WORKTREE:-$MAIN/.claude/worktrees/cdz-smith-fuzz}"
FINDINGS="$MAIN/spec/semantics/failures"
CRATE_REL="implementation/seed/crates/cdz-smith"
# The persistent corpus lives OUTSIDE the worktree (which gets `reset --hard` each cycle) so
# coverage progress survives. Under the main repo's git dir, which is never reset.
CORPUS="${CDZ_SMITH_CORPUS:-$MAIN/.git/cdz-smith-corpus}"

CYCLE_CAP="${CDZ_SMITH_CYCLE_CAP:-420}"
TIMEOUT_S="${CDZ_SMITH_TIMEOUT:-10}"
ITERATIONS="${CDZ_SMITH_ITERATIONS:-50000}"

log() { echo "[fuzz-cycle $(date -u +%H:%M:%S)] $*"; }
export PATH="$HOME/.cargo/bin:$PATH"

# ── 1. sync the worktree to the latest spec ─────────────────────────────────────────────────────
if [ ! -d "$WORKTREE" ]; then
  log "creating fuzzing worktree at $WORKTREE"
  git -C "$MAIN" worktree add -B cdz-smith-fuzz "$WORKTREE" refs/heads/spec || {
    log "worktree add failed; skipping cycle"; exit 0; }
fi
git -C "$WORKTREE" fetch --quiet "$MAIN" spec 2>/dev/null
if ! git -C "$WORKTREE" reset --hard refs/heads/spec --quiet 2>/dev/null; then
  SPEC_SHA="$(git -C "$MAIN" rev-parse spec 2>/dev/null)"
  git -C "$WORKTREE" reset --hard "$SPEC_SHA" --quiet 2>/dev/null || {
    log "could not sync worktree to spec; skipping cycle"; exit 0; }
fi
COMMIT="$(git -C "$WORKTREE" rev-parse --short HEAD 2>/dev/null || echo unknown)"
CRATE_DIR="$WORKTREE/$CRATE_REL"
[ -f "$CRATE_DIR/Cargo.toml" ] || { log "cdz-smith crate not present @$COMMIT; skipping"; exit 0; }

# ── 2. pick the engine ──────────────────────────────────────────────────────────────────────────
ENGINE="${CDZ_SMITH_ENGINE:-auto}"
if [ "$ENGINE" = "auto" ]; then
  if rustup run nightly true 2>/dev/null && command -v cargo-bolero >/dev/null 2>&1; then
    ENGINE="libfuzzer"
  else
    ENGINE="prng"
  fi
fi
log "fuzzing spec @$COMMIT | engine=$ENGINE | cap ${CYCLE_CAP}s | ${TIMEOUT_S}s/input"

mkdir -p "$FINDINGS"
before="$(ls "$FINDINGS"/*.smith.md 2>/dev/null | wc -l | tr -d ' ')"

if [ "$ENGINE" = "libfuzzer" ]; then
  # ── coverage-guided libFuzzer campaign ────────────────────────────────────────────────────────
  mkdir -p "$CORPUS"
  CRASHES="$CRATE_DIR/target/smith-crashes"
  rm -rf "$CRASHES"; mkdir -p "$CRASHES"
  # `-T` bounds the campaign; `-fork=1` isolates + continues past a fault; `-timeout` catches hangs;
  # the ignore_* flags keep the campaign RUNNING past a fault (we triage the saved artifacts after).
  # The outer `timeout` is a hard backstop (cap + 60s slack for the final merge/exit).
  ( cd "$CRATE_DIR" && CDZ_SMITH_COMMIT="$COMMIT" timeout --signal=KILL "$(( CYCLE_CAP + 60 ))" \
      rustup run nightly cargo bolero test cdz_smith_never_panics \
        --engine libfuzzer -T "${CYCLE_CAP}s" --timeout "${TIMEOUT_S}s" \
        --corpus-dir "$CORPUS" --crashes-dir "$CRASHES" \
        -E-fork=1 -E-ignore_timeouts=1 -E-ignore_crashes=1 -E-ignore_ooms=1 \
      2>&1 | grep -iE "cov:|SUMMARY|artifact|ERROR|panic|NEW crash" | tail -8 )
  # Convert artifacts → deduped findings.
  ( cd "$CRATE_DIR" && cargo build -q 2>/dev/null && \
      ./target/debug/cdz-smith triage-artifacts "$CRASHES" --findings "$FINDINGS" --commit "$COMMIT" 2>&1 | tail -3 )
  corp="$(ls "$CORPUS" 2>/dev/null | wc -l | tr -d ' ')"
  log "libfuzzer done | corpus $corp entries (persistent)"
else
  # ── PRNG fallback ─────────────────────────────────────────────────────────────────────────────
  if ! ( cd "$CRATE_DIR" && cargo build --release 2>&1 | tail -2 ); then
    log "build failed @$COMMIT; skipping"; exit 0
  fi
  BIN="$CRATE_DIR/target/release/cdz-smith"
  [ -x "$BIN" ] || { log "binary missing; skipping"; exit 0; }
  CDZ_SMITH_COMMIT="$COMMIT" timeout --signal=KILL "$CYCLE_CAP" \
    "$BIN" fuzz --iterations "$ITERATIONS" --seed "$(date +%s)" \
      --timeout "$TIMEOUT_S" --findings "$FINDINGS"
fi

after="$(ls "$FINDINGS"/*.smith.md 2>/dev/null | wc -l | tr -d ' ')"
new=$(( after - before ))
if [ "$new" -gt 0 ]; then
  log "surfaced $new NEW finding bucket(s) → $FINDINGS (total $after)"
else
  log "no new buckets this cycle (total findings $after)"
fi
exit 0
