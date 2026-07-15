#!/usr/bin/env bash
# fuzz-cycle.sh — one tick of continuous, COVERAGE-GUIDED compiler fuzzing for the FLEET.
#
# Each cycle fuzzes the compiler in the INVOKING worktree for a time-boxed campaign. The `fuzzer`
# fleet agent has already `git rebase`d that worktree onto `trunk` and rebuilt the runtime store
# before it runs this, so we fuzz current `trunk` in place — there is no separate throwaway
# worktree to sync anymore (the old `spec`→`spec-worktree` reset dance is retired with the fleet).
#
# Findings (a runnable `.sexp` + a `.md` triage note per distinct crash SITE, deduped) are written
# into the FLEET QUEUE — `.claude/fleet/queue/` — where the `corpus-bugfix` PM routes them to `fix`
# agents. That queue, NOT `spec/semantics/failures`, is the fleet's bug intake.
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
# Everything is best-effort + idempotent: a failed build just skips this cycle; the next tick
# retries against whatever `trunk` has become.
#
# Env overrides (all optional):
#   CDZ_SMITH_ROOT        repo root to fuzz (default: the invoking worktree's toplevel)
#   CDZ_SMITH_FINDINGS    findings queue dir (default: <root>/.claude/fleet/queue)
#   CDZ_SMITH_CYCLE_CAP   campaign wall-clock, s (default: 420 = 7 min, under a 10-min tick)
#   CDZ_SMITH_TIMEOUT     per-input compile budget, s (default: 10)
#   CDZ_SMITH_ITERATIONS  PRNG-fallback programs/cycle (default: 50000)
#   CDZ_SMITH_ENGINE      force "libfuzzer" or "prng" (default: auto-detect)
set -uo pipefail

# ── locate the checkout ─────────────────────────────────────────────────────────────────────────
# Fuzz the INVOKING worktree (the fleet agent already synced it to `trunk`). Anchor on the cwd's
# git toplevel; robust regardless of where the script FILE lives (a cron/agent may pipe it in via
# process substitution, so `$BASH_SOURCE` may point outside the repo). A `CDZ_SMITH_ROOT` wins.
CWD_TOP="$(git -C "$PWD" rev-parse --show-toplevel 2>/dev/null)"
if [ -n "$CWD_TOP" ]; then
  DEFAULT_ROOT="$CWD_TOP"
else
  SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" 2>/dev/null && pwd)"
  DEFAULT_ROOT="$(cd "$SCRIPT_DIR/../../../.." 2>/dev/null && pwd)"
fi
ROOT="${CDZ_SMITH_ROOT:-$DEFAULT_ROOT}"
CRATE_REL="implementation/seed/crates/cdz-smith"
CRATE_DIR="$ROOT/$CRATE_REL"
if [ -z "$ROOT" ] || [ ! -f "$CRATE_DIR/Cargo.toml" ]; then
  echo "[fuzz-cycle] cannot locate the cdz-smith crate (ROOT='$ROOT'); set CDZ_SMITH_ROOT. Skipping."
  exit 0
fi

# Findings land in the FLEET QUEUE. The `.claude/fleet/queue` dir may live in the shared git
# common-dir's parent (the fleet's main checkout), not the per-agent worktree; resolve it there.
COMMON_DIR="$(git -C "$ROOT" rev-parse --path-format=absolute --git-common-dir 2>/dev/null)"
FLEET_ROOT="$(dirname "${COMMON_DIR:-$ROOT/.git}")"
FINDINGS="${CDZ_SMITH_FINDINGS:-$FLEET_ROOT/.claude/fleet/queue}"

# The persistent corpus lives under the SHARED git common-dir so coverage progress survives across
# ticks and is shared regardless of which worktree drives the campaign.
CORPUS="${CDZ_SMITH_CORPUS:-${COMMON_DIR:-$ROOT/.git}/cdz-smith-corpus}"

CYCLE_CAP="${CDZ_SMITH_CYCLE_CAP:-420}"
TIMEOUT_S="${CDZ_SMITH_TIMEOUT:-10}"
ITERATIONS="${CDZ_SMITH_ITERATIONS:-50000}"

log() { echo "[fuzz-cycle $(date -u +%H:%M:%S)] $*"; }
export PATH="$HOME/.cargo/bin:$PATH"

COMMIT="$(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo unknown)"

# ── pick the engine ───────────────────────────────────────────────────────────────────────────
ENGINE="${CDZ_SMITH_ENGINE:-auto}"
if [ "$ENGINE" = "auto" ]; then
  if rustup run nightly true 2>/dev/null && command -v cargo-bolero >/dev/null 2>&1; then
    ENGINE="libfuzzer"
  else
    ENGINE="prng"
  fi
fi
log "fuzzing trunk @$COMMIT | engine=$ENGINE | cap ${CYCLE_CAP}s | ${TIMEOUT_S}s/input | queue → $FINDINGS"

mkdir -p "$FINDINGS"
before="$(ls "$FINDINGS"/*.smith.md 2>/dev/null | wc -l | tr -d ' ')"

if [ "$ENGINE" = "libfuzzer" ]; then
  # ── coverage-guided libFuzzer campaign ────────────────────────────────────────────────────────
  mkdir -p "$CORPUS"
  CRASHES="$CRATE_DIR/target/smith-crashes"
  rm -rf "$CRASHES"; mkdir -p "$CRASHES"
  # `-T` bounds the campaign; `-fork=1` isolates + continues past a fault; `-timeout` catches hangs;
  # the ignore_* flags keep the campaign RUNNING past a fault (we triage the saved artifacts after).
  #
  # SPURIOUS-CRASH AVOIDANCE. Two sources of fork-mode NON-reproducing "crash" artifacts, both
  # addressed here (diagnosed 2026-07-14 — trivial 6-byte inputs saved as `crash-`, 0/N reproduce
  # even under the same instrumented binary, correlated with low exec/s under contention):
  #   1. The outer hard KILL clipping libFuzzer mid-campaign kills in-flight fork children, each
  #      recorded as a crash against its last input. Fix: `-T` already bounds the run, so libFuzzer
  #      exits on its OWN; the outer `timeout` is a pure backstop with a WIDE margin (2×+120s) so it
  #      effectively never fires during a healthy run. On the rare backstop trip, triage discards the
  #      dying-child artifacts anyway (they don't reproduce).
  #   2. AddressSanitizer (cargo-bolero's default) false-positives on the hand-managed 64 MB
  #      guard-stack thread (`run_with_compiler_stack`). `ASAN_OPTIONS` disables the stack-use-
  #      after-return fake-stack machinery that misfires there, and keeps ASan from aborting the
  #      whole process on a container-RSS ceiling. rcdzc is pure safe Rust, so ASan can only produce
  #      false positives on the compile path anyway; we keep it solely for the sancov RUNTIME that
  #      SanitizerCoverage links against (a plain `-s NONE` build fails to link `__sancov_*`).
  backstop=$(( CYCLE_CAP * 2 + 120 ))
  ( cd "$CRATE_DIR" \
      && CDZ_SMITH_COMMIT="$COMMIT" \
         ASAN_OPTIONS="detect_stack_use_after_return=0:allocator_may_return_null=1:handle_segv=0:abort_on_error=0" \
         timeout --signal=KILL "$backstop" \
      rustup run nightly cargo bolero test cdz_smith_never_panics \
        --engine libfuzzer -T "${CYCLE_CAP}s" --timeout "${TIMEOUT_S}s" \
        --corpus-dir "$CORPUS" --crashes-dir "$CRASHES" \
        -E-fork=1 -E-ignore_timeouts=1 -E-ignore_crashes=1 -E-ignore_ooms=1 \
      2>&1 | grep -iE "cov:|SUMMARY|artifact|ERROR|panic|NEW crash" | tail -8 )
  # Convert artifacts → deduped findings. A `crash-` artifact that does NOT reproduce on replay is a
  # fork-mode phantom (see above), silently dropped by triage — expected, not a lost finding.
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
