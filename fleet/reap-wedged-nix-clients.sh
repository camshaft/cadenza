#!/usr/bin/env bash
# reap-wedged-nix-clients.sh — SAFETY NET for the recurring nix-daemon WEDGE (concierge-greenlit
# 2026-08-28; primary fix is v-nix's warm-keep GC-rooting of the corpus CA cache, #4831, which removes
# the cold-sweep pressure that CAUSES the wedge — this bounds a wedge's DURATION if one still recurs).
#
# THE WEDGE: multiple agents launch heavy `nix build .#checks.<corpus|wasm-opt-gap>-*` sweeps; the nix
# daemon's worker/connection pool gets exhausted by long-held stuck client connections, so it stops
# DISPATCHING queued builds (0 active builders, 0 completions) while the clients wait many hours. A
# `systemctl restart` did NOT clear it (only trivial builds dispatched); KILLING the stuck clients frees
# the pool and the daemon resumes (v-fleet-tooling 2026-08-28: killed 12 stuck 11-12h clients → +78
# store builds in 25s). This script automates that surgical recovery.
#
# ‼ SAFETY (concierge caveat, FALSE-NEGATIVE-BIASED — better to MISS a wedge than kill real work): a
# legit COLD whole-corpus from-source sweep runs for HOURS and MUST NOT be killed. So we reap ONLY on
# the full WEDGE SIGNATURE, never on elapsed time alone:
#   (1) there is a NON-EXEMPT `nix build .#checks` client older than WEDGED_CLIENT_MIN, AND
#   (2) the daemon is DEMONSTRABLY NOT PROGRESSING — NO build-worker process (rustc/cdz-run/cargo/
#       cdz-compile) appears in ANY sample across a PROGRESS_WINDOW_SEC window. A progressing daemon
#       (even a slow legit sweep) dispatches builders that WILL show in a sample → we then do NOTHING.
# Any sign of dispatch aborts the reap. DRY-RUN by default (prints WOULD-KILL); pass --apply to kill.
#
# HARDENING (concierge 2026-08-28, v-wasm-opt exit-144 report): a legit multi-hour `wasm-opt-gaps` sweep
# under serial `cores=1` can momentarily show 0 builders BETWEEN derivations, so a short window risks a
# false-positive on real work. Two guards, both widen the false-negative bias:
#   • REAP_EXEMPT_REGEX (default `wasm-opt-gaps`) — known-long legit sweeps are NEVER candidates, so the
#     reaper can't kill them even under a confirmed wedge (it still frees the pool by killing the OTHER
#     stuck clients). Set REAP_EXEMPT_REGEX="" to disable the exemption.
#   • PROGRESS_WINDOW_SEC defaults to 300s (was 120): 5 straight minutes of zero dispatch is a much
#     stronger no-progress signal than 2 — a progressing sweep will dispatch SOME builder inside 5min.
# Every KILL (and each confirmed-wedge decision) is appended to REAP_LOG so "did the reaper fire on X?"
# is answerable after the fact (there was no durable record before).
set -uo pipefail

# SILENT-CRON OBSERVABILITY (v-fleet-tooling 2026-09-01; matches drain-nudge #7339 / prune-*.sh / baseline-
# drift-monitor). This reaper only writes REAP_LOG on a WEDGE event; a quiet pass (nothing wedged — the
# healthy common case) is silent, so an all-silent run is indistinguishable from a DEAD cron. OVERWRITE a
# `.last-run` next to this script on EVERY run via an EXIT trap (fires on any exit path — the "daemon
# dispatching" / "no candidates" early exits included); its MTIME is the fired-proof, distinct from the
# event REAP_LOG. Best-effort; never affects behavior or the exit code.
_cdz_lastrun="$(cd "$(dirname "${BASH_SOURCE[0]}")" 2>/dev/null && pwd)/reap-wedged-nix-clients.last-run"
trap 'printf "%s\n" "$(date -Is 2>/dev/null || echo now)" > "$_cdz_lastrun" 2>/dev/null || true' EXIT

WEDGED_CLIENT_MIN="${WEDGED_CLIENT_MIN:-180}"      # a `nix build .#checks` client older than this (min) is a CANDIDATE
PROGRESS_WINDOW_SEC="${PROGRESS_WINDOW_SEC:-300}"  # watch for daemon dispatch over this window (5min: strong no-progress signal)
PROGRESS_SAMPLE_SEC="${PROGRESS_SAMPLE_SEC:-3}"    # builder-presence sample interval
REAP_EXEMPT_REGEX="${REAP_EXEMPT_REGEX-wasm-opt-gaps}"  # check-name substrings NEVER reaped (known-long legit sweeps); "" disables

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REAP_LOG="${REAP_LOG:-$SCRIPT_DIR/reap-wedged-nix-clients.log}"

APPLY=0
ORPHANS_ONLY=0
for _arg in "$@"; do
  case "$_arg" in
    --apply) APPLY=1 ;;
    # --orphans-only: run ONLY the guarded step-0 orphan-leak reap, then STOP (skip the stateful wedge-check
    # in steps 1-3). This is the AUTO-CRON mode (concierge-greenlit 2026-08-31): a ~30min fleet-up cron runs
    # `--orphans-only --apply` so orphaned `.#checks` leaks auto-clean, while the heavier deliberate wedge-kill
    # (which samples builders over 300s) stays MANUAL. Safe to automate — step-0 kills only ppid=1 + own-user
    # + .#checks + >180min-floor + non-exempt + non-leased, so a live build (live parent, <180min) is never hit.
    --orphans-only) ORPHANS_ONLY=1 ;;
  esac
done

# Append a timestamped audit line to REAP_LOG (best-effort; never fail the reap on a log-write error).
log() { echo "$(date -Is) $*" >>"$REAP_LOG" 2>/dev/null || true; }

# (0) ORPHANED-LEAK branch (v-fleet-tooling 2026-08-30; AGE-FLOORED after a false-positive, v-deferral-declines
# #6163; SCOPE-BROADENED to all `.#checks` after v-deferral-declines found 4 orphaned corpus/corpus-06/local-gate
# builds 4h41m-5h22m old that the local-gate-only scope MISSED while they held derivation-output locks + starved
# the gate lane). An ORPHANED (ppid=1) own-user `nix build .#checks.*` MIGHT be a leak (owner died) — BUT ppid==1
# ALONE is NOT sufficient: a LEGIT check runs 15-40min, and its nix client can transiently/briefly show ppid==1
# (or be freshly orphaned by an unrelated wrapper kill) while its build is still ACTIVELY progressing + wanted.
# Reaping "regardless of age" therefore CULLED HEALTHY in-progress builds → a fleet-wide land-block. FIX: only
# reap an orphan OLDER than WEDGED_CLIENT_MIN (180min) — far beyond ANY legit check build (15-40min), so no
# in-progress build is ever reaped, while a truly-leaked orphan lingering >3h is still cleaned.
# WHY ALL `.#checks` (not just local-gate, the original 2026-08-30 scope): the local-gate scope was over-narrow —
# an orphaned corpus/corpus-* build is a leak too (holds a corpus derivation-output lock → gate-lane starvation),
# and warm-keep does NOT fire-and-forget corpus builds (its flake app builds each target SEQUENTIALLY + blocking,
# so a LIVE warm-keep corpus build has ppid≠1; only a DEAD warm-keep leaves an orphaned build, which is genuinely
# leaked + won't be rooted). So the original "corpus-* may be warm-keep fire-and-forget" caveat was wrong. Still
# own-user only; honors REAP_EXEMPT_REGEX (wasm-opt-gaps stays exempt); and SKIPS a CDZ_LEASED_NIX=1 owned build
# (belt-and-braces with step-1). (A sub-180min genuine leak self-resolves: the daemon finishes + the client exits.)
orphans="$(ps -eo pid,ppid,euid,etimes,args 2>/dev/null \
  | awk -v me="$(id -u)" -v exempt="$REAP_EXEMPT_REGEX" -v minage=$((WEDGED_CLIENT_MIN * 60)) \
      '$2 == 1 && $3 == me && $4 > minage && /nix build \.#checks/ && !/awk/ && (exempt == "" || $0 !~ exempt) {print $1}')"
# SKIP a CDZ_LEASED_NIX=1 owned build even if orphaned (mirrors step-1's leased-skip; an owned build gets extra
# protection, and a truly-leaked leased orphan is rare + the TTL/dead-holder lease reclaim handles its slot).
_orph_kept=""
for _op in $orphans; do
  if grep -qz 'CDZ_LEASED_NIX=1' "/proc/$_op/environ" 2>/dev/null; then
    log "SKIP leased orphan pid=$_op (CDZ_LEASED_NIX=1 — owned build, never reaped)"
  else
    _orph_kept="${_orph_kept:+$_orph_kept }$_op"
  fi
done
orphans="$_orph_kept"
if [ -n "$orphans" ]; then
  n_orph="$(printf '%s\n' "$orphans" | grep -c .)"
  echo "reap-wedged-nix-clients: ${n_orph} ORPHANED (ppid=1, own-user) '.#checks' LEAK(s) older than ${WEDGED_CLIENT_MIN}min (beyond any legit build) — owner dead. $([ "$APPLY" = 1 ] && echo 'KILLING:' || echo 'WOULD-KILL (dry-run; pass --apply):')"
  for p in $orphans; do
    oinfo="$(ps -o pid,etime,args -p "$p" 2>/dev/null | tail -1 | cut -c1-100)"
    echo "  $oinfo"
    if [ "$APPLY" = 1 ]; then kill -KILL "$p" 2>/dev/null || true; log "KILLED-ORPHAN-LEAK pid=$p ($oinfo)"; fi
  done
fi

# AUTO-CRON mode: `--orphans-only` runs ONLY the guarded step-0 orphan-leak reap above, then STOPS —
# skipping the stateful wedge-check (steps 1-3), which is a deliberate heavy intervention (300s builder
# sampling) that stays MANUAL. The ~30min fleet-up cron uses this so orphan `.#checks` leaks auto-clean.
if [ "$ORPHANS_ONLY" = 1 ]; then
  exit 0
fi

# Count active build-worker processes. NOTE: `pgrep -c` already PRINTS the count (0 when none) and
# exits 1 on no match, so a `|| echo 0` would DOUBLE-print ("0\n0") and break the integer test — capture
# its output and default an empty/failed result to 0 instead.
builders() {
  local n
  n="$(pgrep -c 'rustc|cdz-run|cargo|cdz-compile' 2>/dev/null)"
  echo "${n:-0}"
}

# (1) Candidates: NON-EXEMPT nix-build clients targeting the corpus/oracle checks, older than the
# threshold. An empty REAP_EXEMPT_REGEX means "exempt nothing" (guard against awk's `$0 !~ ""` which
# would otherwise match — and thus exclude — every line).
candidates="$(ps -eo pid,etimes,args 2>/dev/null \
  | awk -v min=$((WEDGED_CLIENT_MIN * 60)) -v exempt="$REAP_EXEMPT_REGEX" \
      '$2 > min && /nix build \.#checks/ && !/awk/ && (exempt == "" || $0 !~ exempt) {print $1}')"
# (1b) LEASED/OWNED SKIP (v-nix ask 2026-08-29): a candidate whose process env carries CDZ_LEASED_NIX=1 is
# a SANCTIONED leased build (set by run_gate_local / run_gate_local_bounded / with_lease / a `fleet
# with-lease -- nix build …`) — its owner manages it, so NEVER reap it, even under a confirmed wedge (the
# reaper still frees the pool via the OTHER, unleased/orphaned clients). /proc/<pid>/environ is NUL-
# separated + own-user-readable (the client runs as us, not nixbld). Belt-and-braces atop the 180min floor.
_kept=""
_leased=0
for _p in $candidates; do
  if grep -qz 'CDZ_LEASED_NIX=1' "/proc/$_p/environ" 2>/dev/null; then
    _leased=$((_leased + 1))
    log "SKIP leased pid=$_p (CDZ_LEASED_NIX=1 — owned build, never reaped)"
  else
    _kept="${_kept:+$_kept }$_p"
  fi
done
candidates="$_kept"
if [ -z "$candidates" ]; then
  echo "reap-wedged-nix-clients: no non-exempt REAPABLE 'nix build .#checks' client older than ${WEDGED_CLIENT_MIN}min — nothing to consider${REAP_EXEMPT_REGEX:+ (exempting /$REAP_EXEMPT_REGEX/)}$([ "$_leased" -gt 0 ] && echo " (${_leased} leased/owned build(s) skipped — never reaped)")."
  exit 0
fi
n_cand="$(printf '%s\n' "$candidates" | grep -c .)"

# (2) WEDGE-SIGNATURE gate: is the daemon progressing? Sample builder presence over the window; ANY
# builder in ANY sample ⇒ the daemon IS dispatching ⇒ the old clients are SLOW, not wedged ⇒ do nothing.
elapsed=0
progressing=0
while [ "$elapsed" -lt "$PROGRESS_WINDOW_SEC" ]; do
  if [ "$(builders)" -gt 0 ]; then progressing=1; break; fi
  sleep "$PROGRESS_SAMPLE_SEC"
  elapsed=$((elapsed + PROGRESS_SAMPLE_SEC))
done
if [ "$progressing" = 1 ]; then
  echo "reap-wedged-nix-clients: daemon IS dispatching (a build worker appeared within ${PROGRESS_WINDOW_SEC}s) — \
${n_cand} client(s) >${WEDGED_CLIENT_MIN}min are SLOW, not wedged. Leaving them untouched (never kill real work)."
  exit 0
fi

# (3) Confirmed wedge: old NON-EXEMPT clients waiting AND zero dispatch across the whole window → free the pool.
echo "reap-wedged-nix-clients: WEDGE confirmed — 0 build workers across ${PROGRESS_WINDOW_SEC}s yet ${n_cand} \
non-exempt 'nix build .#checks' client(s) waiting >${WEDGED_CLIENT_MIN}min. $([ "$APPLY" = 1 ] && echo 'KILLING (freeing the daemon pool):' || echo 'WOULD-KILL (dry-run; pass --apply):')"
log "WEDGE confirmed: ${n_cand} non-exempt client(s) >${WEDGED_CLIENT_MIN}min, 0 builders/${PROGRESS_WINDOW_SEC}s, apply=${APPLY}, exempt=/${REAP_EXEMPT_REGEX}/"
for p in $candidates; do
  info="$(ps -o pid,etime,args -p "$p" 2>/dev/null | tail -1 | cut -c1-100)"
  echo "$info"
  if [ "$APPLY" = 1 ]; then
    kill "$p" 2>/dev/null || true
    log "KILLED pid=$p ($info)"
  fi
done
if [ "$APPLY" = 1 ]; then
  echo "reap-wedged-nix-clients: killed ${n_cand} wedged client(s) — the daemon should resume dispatching queued builds."
else
  echo "reap-wedged-nix-clients: dry-run — re-run with --apply to kill."
fi
