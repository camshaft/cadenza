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

WEDGED_CLIENT_MIN="${WEDGED_CLIENT_MIN:-180}"      # a `nix build .#checks` client older than this (min) is a CANDIDATE
PROGRESS_WINDOW_SEC="${PROGRESS_WINDOW_SEC:-300}"  # watch for daemon dispatch over this window (5min: strong no-progress signal)
PROGRESS_SAMPLE_SEC="${PROGRESS_SAMPLE_SEC:-3}"    # builder-presence sample interval
REAP_EXEMPT_REGEX="${REAP_EXEMPT_REGEX-wasm-opt-gaps}"  # check-name substrings NEVER reaped (known-long legit sweeps); "" disables

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REAP_LOG="${REAP_LOG:-$SCRIPT_DIR/reap-wedged-nix-clients.log}"

APPLY=0
[ "${1:-}" = "--apply" ] && APPLY=1

# Append a timestamped audit line to REAP_LOG (best-effort; never fail the reap on a log-write error).
log() { echo "$(date -Is) $*" >>"$REAP_LOG" 2>/dev/null || true; }

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
if [ -z "$candidates" ]; then
  echo "reap-wedged-nix-clients: no non-exempt 'nix build .#checks' client older than ${WEDGED_CLIENT_MIN}min — nothing to consider${REAP_EXEMPT_REGEX:+ (exempting /$REAP_EXEMPT_REGEX/)}."
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
