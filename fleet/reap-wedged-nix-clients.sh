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
#   (1) there is a `nix build .#checks` client older than WEDGED_CLIENT_MIN, AND
#   (2) the daemon is DEMONSTRABLY NOT PROGRESSING — NO build-worker process (rustc/cdz-run/cargo/
#       cdz-compile) appears in ANY sample across a PROGRESS_WINDOW_SEC window. A progressing daemon
#       (even a slow legit sweep) dispatches builders that WILL show in a sample → we then do NOTHING.
# Any sign of dispatch aborts the reap. DRY-RUN by default (prints WOULD-KILL); pass --apply to kill.
set -uo pipefail

WEDGED_CLIENT_MIN="${WEDGED_CLIENT_MIN:-180}"      # a `nix build .#checks` client older than this (min) is a CANDIDATE
PROGRESS_WINDOW_SEC="${PROGRESS_WINDOW_SEC:-120}"  # watch for daemon dispatch over this window
PROGRESS_SAMPLE_SEC="${PROGRESS_SAMPLE_SEC:-3}"    # builder-presence sample interval

APPLY=0
[ "${1:-}" = "--apply" ] && APPLY=1

# Count active build-worker processes. NOTE: `pgrep -c` already PRINTS the count (0 when none) and
# exits 1 on no match, so a `|| echo 0` would DOUBLE-print ("0\n0") and break the integer test — capture
# its output and default an empty/failed result to 0 instead.
builders() {
  local n
  n="$(pgrep -c 'rustc|cdz-run|cargo|cdz-compile' 2>/dev/null)"
  echo "${n:-0}"
}

# (1) Candidates: nix-build clients targeting the corpus/oracle checks, older than the threshold.
candidates="$(ps -eo pid,etimes,args 2>/dev/null \
  | awk -v min=$((WEDGED_CLIENT_MIN * 60)) '$2 > min && /nix build \.#checks/ && !/awk/ {print $1}')"
if [ -z "$candidates" ]; then
  echo "reap-wedged-nix-clients: no 'nix build .#checks' client older than ${WEDGED_CLIENT_MIN}min — nothing to consider."
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

# (3) Confirmed wedge: old clients waiting AND zero dispatch across the whole window → free the pool.
echo "reap-wedged-nix-clients: WEDGE confirmed — 0 build workers across ${PROGRESS_WINDOW_SEC}s yet ${n_cand} \
'nix build .#checks' client(s) waiting >${WEDGED_CLIENT_MIN}min. $([ "$APPLY" = 1 ] && echo 'KILLING (freeing the daemon pool):' || echo 'WOULD-KILL (dry-run; pass --apply):')"
for p in $candidates; do
  ps -o pid,etime,args -p "$p" 2>/dev/null | tail -1 | cut -c1-100
  if [ "$APPLY" = 1 ]; then kill "$p" 2>/dev/null || true; fi
done
if [ "$APPLY" = 1 ]; then
  echo "reap-wedged-nix-clients: killed ${n_cand} wedged client(s) — the daemon should resume dispatching queued builds."
else
  echo "reap-wedged-nix-clients: dry-run — re-run with --apply to kill."
fi
