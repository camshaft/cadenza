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
#   CDZ_SMITH_DIFF_COUNT  differential-sweep programs/cycle (default: 200; 0 disables the sweep)
#   CDZ_SMITH_DIFF_CAP    differential-sweep wall-clock backstop, s (default: fit under the tick after
#                         the campaign — a KILL mid-sweep is safe, findings file incrementally to disk)
#   CDZ_SMITH_CDZ         the `cdz` binary for the differential rust side (default: auto-discover)
#   CDZ_SMITH_STORE       the value-heap runtime store for the differential wasm side (default: <root>/target/cadenza-store)
#   CDZ_SMITH_OPT_COUNT   opt-invariance-sweep programs/cycle (default: 100; 0 disables the sweep)
#   CDZ_SMITH_OPT_CAP     opt-invariance-sweep wall-clock backstop, s (default: 1/4 of the tick's
#                         post-campaign budget — a KILL mid-sweep is safe)
#   CDZ_SMITH_DET_COUNT   determinism-sweep programs/cycle (default: 200; 0 disables). Compile-only —
#                         needs no store/cdz, so it runs even when the other sweeps skip.
#   CDZ_SMITH_DET_CAP     determinism-sweep wall-clock backstop, s (default: 1/4 of the leftover)
#   CDZ_SMITH_TYPE_COUNT  type-differential-sweep programs/cycle (default: 200; 0 disables). Runs ONLY when
#                         a fresh Lean oracle is staged (below) — the oracle can drift from the compiler,
#                         so an absent/unstaged oracle skips cleanly rather than filing false findings.
#   CDZ_SMITH_ORACLE_CHECK  the Lean `oracle-check` binary for the type-differential (default: PATH lookup;
#                         build with `nix build .#oracle-lean` and stage result/bin/oracle-check to enable)
#   CDZ_SMITH_TYPE_CAP    type-differential-sweep wall-clock backstop, s (default: a 1/3 leftover slice)
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

# ── differential-oracle sweep (SEPARATE, lower-cadence pass) ─────────────────────────────────────
# After the crash/invalid-wasm campaign, run the DIFFERENTIAL oracle over a modest batch of seeds:
# each program is run on BOTH backends (wasm in-process via cdz-run; rust by shelling `cdz run-rust`)
# and their canonical values compared — a disagreement is a valid-artifact wrong-value miscompile the
# crash/validity oracles can't see. It is deliberately lower-cadence (it `rustc`-compiles every
# program, orders of magnitude slower), and gated behind the off-by-default `differential` cargo
# feature (whose `cdz-run`/wasmtime dep must NOT link into the instrumented libFuzzer target). Findings
# file into the SAME fleet queue as `differential-*.smith.{sexp,md}`. Best-effort: skip cleanly if the
# `cdz` binary (+ its cdz-rt/cdz-num rlibs) or the runtime store isn't available.
DIFF_COUNT="${CDZ_SMITH_DIFF_COUNT:-200}"
if [ "$DIFF_COUNT" -gt 0 ]; then
  # The rust side needs the `cdz` binary with its rlibs beside it (target/<profile>/). The fleet agent
  # rebuilds `cdz` release each tick; discover it (env override wins), else look beside the workspace target.
  DIFF_CDZ="${CDZ_SMITH_CDZ:-}"
  if [ -z "$DIFF_CDZ" ]; then
    for cand in "$ROOT/target/release/cdz" "$ROOT/target/debug/cdz"; do
      [ -x "$cand" ] && { DIFF_CDZ="$cand"; break; }
    done
  fi
  DIFF_STORE="${CDZ_SMITH_STORE:-$ROOT/target/cadenza-store}"
  if [ -z "$DIFF_CDZ" ] || [ ! -x "$DIFF_CDZ" ]; then
    log "differential: no cdz binary found (build \`cargo build --release --bin cdz\` or set CDZ_SMITH_CDZ); skipping sweep"
  elif [ ! -d "$DIFF_STORE" ]; then
    log "differential: runtime store $DIFF_STORE absent (\`cargo xtask build\`); skipping sweep"
  else
    log "differential sweep | count $DIFF_COUNT | cdz $DIFF_CDZ | store $DIFF_STORE"
    # Build the differential-featured cdz-smith binary (pulls cdz-run/wasmtime — NOT the fuzz target,
    # so no libFuzzer link concern). A build failure just skips the sweep this cycle.
    if ( cd "$CRATE_DIR" && cargo build -q --release --features differential 2>/dev/null ); then
      DIFF_BIN="$CRATE_DIR/target/release/cdz-smith"
      # TICK-AWARE backstop: the crash/invalid-wasm campaign already consumed ~CYCLE_CAP of the tick,
      # so bound the sweep to what's LEFT under a 10-min tick (600 - CYCLE_CAP - 60s slack). That leftover
      # is SHARED across the three always-on oracle sweeps below: this wasm-vs-rust pass gets 1/2 (it
      # rustc-compiles every program, the slowest side), the in-process opt-invariance pass gets 1/4, and the
      # compile-only determinism pass gets 1/4. Floored at 60s. A KILL when the backstop trips is SAFE — the
      # sweep files each finding to disk as it goes, so a clipped sweep just does fewer programs this cycle.
      DIFF_CAP="${CDZ_SMITH_DIFF_CAP:-$(( (600 - CYCLE_CAP - 60) / 2 ))}"
      [ "$DIFF_CAP" -lt 60 ] && DIFF_CAP=60
      CDZ_SMITH_COMMIT="$COMMIT" timeout --signal=KILL "$DIFF_CAP" \
        "$DIFF_BIN" differential --count "$DIFF_COUNT" --seed "$(date +%s)" \
          --findings "$FINDINGS" --store "$DIFF_STORE" --cdz "$DIFF_CDZ" 2>&1 | tail -4 || true
      # (export-param) — the ENTRY-PARAM boundary-marshal VALUE guard: single-export shapes CALLED with
      # scalar args on BOTH backends via `cdz run-rust --arg` (#9670). A mis-coerced / wrong-width /
      # wrong-sign marshal on either backend corrupts the returned value → a mismatch the NULLARY
      # differential structurally cannot reach. Small count (it shells `cdz` per program, like the sweep
      # above) under its own cap, reusing the already-resolved cdz + store. A KILL at the cap is SAFE
      # (findings stream to disk per program); the count sizes the SLOWEST (cdz-shelling) pass to fit.
      # generate_export_param has 16 shapes (6 scalar + 3 List-entry-borrow + const-sum-field E0282 family
      # #9586/#9684/#9687 + #9689 List-consume + #9694 String-consume + #9699 scalar-fielded Record + #9701
      # rpp3 heap-carrying Record). NOTE: EP_COUNT is HELD FLAT at 210 (now ~13 draws/shape, was ~20) — the
      # cdz-shelling cost neared the tick budget (~67s at 13x20), so we bound the pass by keeping the count
      # flat (all shapes still drawn ~13x/cycle, measured ~56s at 16 shapes) rather than growing it linearly.
      # SPLIT-THRESHOLD (measured, not a fixed shape count): when this pass exceeds ~70s or draws/shape falls
      # below ~10, split it (fast subset per cycle + full nightly). A KILL at the cap is safe (findings stream).
      EP_COUNT="${CDZ_SMITH_EXPORT_PARAM_COUNT:-210}"
      EP_CAP="${CDZ_SMITH_EXPORT_PARAM_CAP:-75}"
      if [ "$EP_COUNT" -gt 0 ]; then
        log "export-param differential mini-pass | count $EP_COUNT | cdz $DIFF_CDZ | store $DIFF_STORE | cap ${EP_CAP}s"
        CDZ_SMITH_COMMIT="$COMMIT" timeout --signal=KILL "$EP_CAP" \
          "$DIFF_BIN" differential --export-param --count "$EP_COUNT" --seed "$(date +%s)" \
            --findings "$FINDINGS" --store "$DIFF_STORE" --cdz "$DIFF_CDZ" 2>&1 | tail -3 || true
      fi
    else
      log "differential: cdz-smith --features differential build failed; skipping sweep"
    fi
  fi
fi

# ── opt-invariance sweep (SEPARATE, in-process, no cdz) ──────────────────────────────────────────
# Complements the wasm-vs-rust differential: each program is compiled+run at the O0 baseline and at
# every higher level (O1/O2/O3), and the values are cross-checked. A divergence is a pure-optimizer
# miscompile (the O2/O3 global-CSE / lifted-analysis reclaim class the differential oracle, both sides
# at O1, cannot reach). WASM-only + IN-PROCESS (no `cdz` subprocess, no rustc), so it is faster per
# program than the differential sweep — it just needs the runtime store. Findings file into the SAME
# fleet queue as `differential-*.smith.{sexp,md}` (tagged `opt-invariance`). Shares the post-campaign
# budget: it gets 1/4 the leftover (see DIFF_CAP above). Best-effort: skip cleanly if the store is absent
# or the featured build fails.
OPT_COUNT="${CDZ_SMITH_OPT_COUNT:-100}"
if [ "$OPT_COUNT" -gt 0 ]; then
  OPT_STORE="${CDZ_SMITH_STORE:-$ROOT/target/cadenza-store}"
  if [ ! -d "$OPT_STORE" ]; then
    log "opt-invariance: runtime store $OPT_STORE absent (\`cargo xtask build\`); skipping sweep"
  else
    # Reuse the differential-featured binary if the sweep above already built it; otherwise build it now
    # (a build failure just skips this pass). Same feature set — no libFuzzer link concern.
    OPT_BIN="$CRATE_DIR/target/release/cdz-smith"
    if [ -x "$OPT_BIN" ] || ( cd "$CRATE_DIR" && cargo build -q --release --features differential 2>/dev/null ); then
      OPT_CAP="${CDZ_SMITH_OPT_CAP:-$(( (600 - CYCLE_CAP - 60) / 4 ))}"
      [ "$OPT_CAP" -lt 60 ] && OPT_CAP=60
      log "opt-invariance sweep | count $OPT_COUNT | store $OPT_STORE | cap ${OPT_CAP}s"
      CDZ_SMITH_COMMIT="$COMMIT" timeout --signal=KILL "$OPT_CAP" \
        "$OPT_BIN" opt-differential --count "$OPT_COUNT" --seed "$(date +%s)" \
          --findings "$FINDINGS" --store "$OPT_STORE" 2>&1 | tail -4 || true
    else
      log "opt-invariance: cdz-smith --features differential build failed; skipping sweep"
    fi
  fi
fi

# ── determinism sweep (SEPARATE, COMPILE-ONLY — no store, no cdz) ─────────────────────────────────
# The fourth oracle dimension: compile each program TWICE and require byte-identical output. A divergence
# is a compiler-nondeterminism bug (the spec mandates each phase be a deterministic function of its input;
# the content-addressed pipeline — binary-AST exchange, the CAS runtime store, caching — depends on it,
# classically a std-HashMap iteration-seed leak into codegen). COMPILE-ONLY + in-process: needs neither the
# runtime store NOR a `cdz` binary, so it is the cheapest + most robust sweep (runs even when the store is
# absent). Findings file into the same fleet queue (`determinism-*.smith.{sexp,md}`). Gets 1/4 the leftover.
DET_COUNT="${CDZ_SMITH_DET_COUNT:-200}"
if [ "$DET_COUNT" -gt 0 ]; then
  DET_BIN="$CRATE_DIR/target/release/cdz-smith"
  if [ -x "$DET_BIN" ] || ( cd "$CRATE_DIR" && cargo build -q --release --features differential 2>/dev/null ); then
    DET_CAP="${CDZ_SMITH_DET_CAP:-$(( (600 - CYCLE_CAP - 60) / 4 ))}"
    [ "$DET_CAP" -lt 60 ] && DET_CAP=60
    log "determinism sweep | count $DET_COUNT | cap ${DET_CAP}s"
    CDZ_SMITH_COMMIT="$COMMIT" timeout --signal=KILL "$DET_CAP" \
      "$DET_BIN" determinism --count "$DET_COUNT" --seed "$(date +%s)" \
        --findings "$FINDINGS" 2>&1 | tail -4 || true
  else
    log "determinism: cdz-smith --features differential build failed; skipping sweep"
  fi
fi

# ── reclaim-shapes mini-pass (VALUE-OBSERVABLE guard on the reclaim-PRECISION churn) ─────────────
# The `--reclaim` grammar (astgen::generate_reclaim_shapes, `variant(17)` = 17 shapes) is a family of
# owned-aggregate / self-recursive / closure-env / borrowed-CHAMP-key / flat-scalar-container programs that
# each return a KNOWN value. Each pins a landed reclaim-UAF fence family — the F5 SITE-A closure-env
# admit+decline, the F7 non-tail invariant-borrow-param admit+decline, the #9497 flat-scalar-container
# heap-return admit + its #9502 MatchList-return DECLINE edge, the borrowed-Map-key shared borrow, … A LEAK
# is invisible to a value oracle, but an OVER-aggressive reclaim that frees a still-live cell corrupts the
# returned VALUE (or traps) — which these oracles catch. This runs a dedicated slice each cycle so the
# standing cron CONTINUOUSLY guards the reclaim-precision work the fleet is grinding (the value-oracle
# counterpart to the corpus `(live-objects N)` leak pins, which the value oracles structurally cannot
# observe). The count must SCALE with the shape count so every shape is reliably drawn EACH cycle (a
# UAF-regression tripwire is only a guard if its shape is exercised): the default is ~20 draws/shape across
# the 17 shapes. `determinism --reclaim` is compile-only (always runs, fast, saturates the full count); the
# `opt-differential --reclaim` value+validity pass runs only when the store resolves and is cap-bounded (it
# sweeps seeds in order until the cap, so a higher count reaches more shapes before the backstop). Findings
# file into the SAME fleet queue (`determinism-*` / `opt-invariance-*`, tagged `reclaim-shapes`).
# NOTE: bump this in step with generate_reclaim_shapes's `variant(N)` whenever a new shape lands (was 100
# for the original 5-shape family; 500 ≈ 25×20 as of the 25-shape generator). This count sizes the
# COMPILE-ONLY determinism pass (fast: it saturates the full 500 well inside the cap). The opt-invariance
# pass is SLOWER (~78ms/prog): at 500 it is intentionally SIGKILLed by the RECLAIM_CAP after ~385 seeds —
# that is BY DESIGN (it sweeps seeds in order until the cap) and loses NOTHING, since findings stream to
# the FindingStore incrementally per program (driver.rs file_and_tally) and only the cosmetic end tally is
# dropped; all 25 shapes are still reached (~15x) before the cap. So do NOT lower this count to "make opt
# finish under the cap" — that would shrink the determinism pass's full-count reach for no gain. Raise
# RECLAIM_CAP (budget permitting) if you want opt to sweep more seeds per cycle.
RECLAIM_COUNT="${CDZ_SMITH_RECLAIM_COUNT:-500}"
if [ "$RECLAIM_COUNT" -gt 0 ]; then
  RC_BIN="$CRATE_DIR/target/release/cdz-smith"
  if [ -x "$RC_BIN" ] || ( cd "$CRATE_DIR" && cargo build -q --release --features differential 2>/dev/null ); then
    RECLAIM_CAP="${CDZ_SMITH_RECLAIM_CAP:-30}"
    # (a) determinism --reclaim — compile-only, no store/cdz, always runnable (the narrow family compiles fast).
    log "reclaim mini-pass (determinism) | count $RECLAIM_COUNT | cap ${RECLAIM_CAP}s"
    CDZ_SMITH_COMMIT="$COMMIT" timeout --signal=KILL "$RECLAIM_CAP" \
      "$RC_BIN" determinism --reclaim --count "$RECLAIM_COUNT" --seed "$(date +%s)" \
        --findings "$FINDINGS" 2>&1 | tail -3 || true
    # (b) opt-invariance --reclaim — O0-vs-O1/O2/O3 VALUE + per-level validity; needs the store (skip cleanly if absent).
    RECLAIM_STORE="${CDZ_SMITH_STORE:-$ROOT/target/cadenza-store}"
    if [ -d "$RECLAIM_STORE" ]; then
      log "reclaim mini-pass (opt-invariance) | count $RECLAIM_COUNT | store $RECLAIM_STORE | cap ${RECLAIM_CAP}s"
      CDZ_SMITH_COMMIT="$COMMIT" timeout --signal=KILL "$RECLAIM_CAP" \
        "$RC_BIN" opt-differential --reclaim --count "$RECLAIM_COUNT" --seed "$(date +%s)" \
          --findings "$FINDINGS" --store "$RECLAIM_STORE" 2>&1 | tail -3 || true
    else
      log "reclaim mini-pass: store $RECLAIM_STORE absent; ran determinism-only (compile-only)"
    fi
  else
    log "reclaim mini-pass: cdz-smith --features differential build failed; skipping"
  fi
fi

# ── effect mini-pass (VALUE-OBSERVABLE coverage of the effects LOWERING) ─────────────────────────
# The `--effect` grammar draws `effect`/`handle`/`resume`/`abort` programs (single-handler · nested-handler ·
# effect+collection · multi-op state fold) that each return a KNOWN Int64. Unlike host imports (which the
# value differential DECLINES — no host to run), an algebraic effect is PURE-GUEST: it lowers to guest
# continuation + handler-frame code and RUNS to a value, so it is fully value-observable. A mislowering of a
# captured continuation / handler-stack frame / the abort-drop path shows as a wrong value (or a byte-diff /
# opt divergence) — which these oracles catch. Effects lowering is complex + bug-prone (active v-effects
# vertical) and had ready generators but fed NO standing sweep until now. Small dedicated slice each cycle so
# the cron continuously guards it; counts/caps SMALL to fit the tick slack. `determinism --effect` is
# compile-only (always runs); `opt-differential --effect` (value + per-level validity) runs when the store
# resolves. Findings file into the SAME fleet queue (`determinism-*` / `opt-invariance-*`, tagged `effect`).
# NOTE: scale with generate_effect's form count (variant(N)); 140 ~= 8 forms x ~17 as of the 8-form generator
# (added the #9642 splat-in-handler form). Bump when a new effect form lands.
EFFECT_COUNT="${CDZ_SMITH_EFFECT_COUNT:-140}"
if [ "$EFFECT_COUNT" -gt 0 ]; then
  EF_BIN="$CRATE_DIR/target/release/cdz-smith"
  if [ -x "$EF_BIN" ] || ( cd "$CRATE_DIR" && cargo build -q --release --features differential 2>/dev/null ); then
    EFFECT_CAP="${CDZ_SMITH_EFFECT_CAP:-30}"
    # (a) determinism --effect — compile-only, no store/cdz, always runnable.
    log "effect mini-pass (determinism) | count $EFFECT_COUNT | cap ${EFFECT_CAP}s"
    CDZ_SMITH_COMMIT="$COMMIT" timeout --signal=KILL "$EFFECT_CAP" \
      "$EF_BIN" determinism --effect --count "$EFFECT_COUNT" --seed "$(date +%s)" \
        --findings "$FINDINGS" 2>&1 | tail -3 || true
    # (b) opt-invariance --effect — O0-vs-O1/O2/O3 VALUE + per-level validity; needs the store (skip cleanly if absent).
    EFFECT_STORE="${CDZ_SMITH_STORE:-$ROOT/target/cadenza-store}"
    if [ -d "$EFFECT_STORE" ]; then
      log "effect mini-pass (opt-invariance) | count $EFFECT_COUNT | store $EFFECT_STORE | cap ${EFFECT_CAP}s"
      CDZ_SMITH_COMMIT="$COMMIT" timeout --signal=KILL "$EFFECT_CAP" \
        "$EF_BIN" opt-differential --effect --count "$EFFECT_COUNT" --seed "$(date +%s)" \
          --findings "$FINDINGS" --store "$EFFECT_STORE" 2>&1 | tail -3 || true
    else
      log "effect mini-pass: store $EFFECT_STORE absent; ran determinism-only (compile-only)"
    fi
  else
    log "effect mini-pass: cdz-smith --features differential build failed; skipping"
  fi
fi

# ── type-differential sweep (SEPARATE, Lean TYPE oracle) ─────────────────────────────────────────
# The third oracle dimension: for each program, compare the compiler's TYPE judgment (accept/reject)
# against the Lean `oracle-check` — a divergence is a false-reject (compiler rejects a well-typed
# program), false-accept (compiler accepts an ill-typed one), or a capability-gap. Uses `--typegen`
# (the dense in-fragment grammar — ~97% of programs are judged, vs ~13% on the broad text grammar).
# Findings file into the same fleet queue (`type-*.smith.{sexp,md}`) for v-lean-oracle triage.
#
# GATED ON A DISCOVERABLE ORACLE (`CDZ_SMITH_ORACLE_CHECK` or `oracle-check` on PATH), NOT the stale-prone
# `result/` symlink: the Lean oracle is an INDEPENDENT artifact that can DRIFT from the compiler, and a
# stale oracle would file false-reject/accept NOISE. So this pass runs ONLY when an operator has
# deliberately staged a FRESH oracle (`nix build .#oracle-lean` → put result/bin/oracle-check on PATH or
# set CDZ_SMITH_ORACLE_CHECK); otherwise it skips cleanly. When it does run it shares the post-campaign
# budget (its own CDZ_SMITH_TYPE_CAP slice — staging an oracle is a deliberate type campaign, so tune
# CDZ_SMITH_DIFF_COUNT/OPT_COUNT down if a single tick must hold all three).
TYPE_COUNT="${CDZ_SMITH_TYPE_COUNT:-200}"
TYPE_ORACLE="${CDZ_SMITH_ORACLE_CHECK:-}"
if [ -z "$TYPE_ORACLE" ]; then
  command -v oracle-check >/dev/null 2>&1 && TYPE_ORACLE="$(command -v oracle-check)"
fi
if [ "$TYPE_COUNT" -gt 0 ] && [ -n "$TYPE_ORACLE" ] && [ -x "$TYPE_ORACLE" ]; then
  TYPE_BIN="$CRATE_DIR/target/release/cdz-smith"
  if [ -x "$TYPE_BIN" ] || ( cd "$CRATE_DIR" && cargo build -q --release --features differential 2>/dev/null ); then
    TYPE_CAP="${CDZ_SMITH_TYPE_CAP:-$(( (600 - CYCLE_CAP - 60) / 3 ))}"
    [ "$TYPE_CAP" -lt 60 ] && TYPE_CAP=60
    log "type-differential sweep | count $TYPE_COUNT | oracle $TYPE_ORACLE | cap ${TYPE_CAP}s"
    CDZ_SMITH_COMMIT="$COMMIT" timeout --signal=KILL "$TYPE_CAP" \
      "$TYPE_BIN" type-differential --typegen --count "$TYPE_COUNT" --seed "$(date +%s)" \
        --oracle "$TYPE_ORACLE" --findings "$FINDINGS" 2>&1 | tail -4 || true
  else
    log "type-differential: cdz-smith --features differential build failed; skipping sweep"
  fi
else
  [ "$TYPE_COUNT" -gt 0 ] && log "type-differential: no oracle-check on PATH / CDZ_SMITH_ORACLE_CHECK (nix build .#oracle-lean); skipping sweep"
fi

after="$(ls "$FINDINGS"/*.smith.md 2>/dev/null | wc -l | tr -d ' ')"
new=$(( after - before ))
if [ "$new" -gt 0 ]; then
  log "surfaced $new NEW finding bucket(s) → $FINDINGS (total $after)"
else
  log "no new buckets this cycle (total findings $after)"
fi
exit 0
