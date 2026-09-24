#!/usr/bin/env bash
# throttle-unleased-nix.sh — NON-DESTRUCTIVE early throttle for the check-lease BYPASS hole (concierge
# policy call + v-nix concurrence 2026-09-24; closes the gap concierge note #083916 exposed: v-memory-safety
# ran a raw 31-chapter `nix build .#checks` aggregate that BYPASSED the check-lease (CDZ_CHECK_LEASE_MAX /
# `fleet with-lease`), held the nix build slots for ~4+ ticks and STARVED per-chapter gate-locals fleet-wide).
#
# THE GAP: a RAW `nix build .#checks…` (not wrapped in `fleet with-lease` / run_gate_local*) never acquires a
# check-lease, so it neither counts against the concurrency cap nor yields to pr-sync's merge gate. The
# advisory `nix-heavy-attrs.tsv` warn does not stop it (it was ignored), and reap-wedged-nix-clients.sh only
# ACTS after its ~180min wedge floor — far too late to prevent the starvation.
#
# THE FIX (policy: (b) THROTTLE — NOT (a) warn-only, which already failed, and NOT (c) reap, which stays
# OPERATOR-GATED because killing a build is destructive): renice the whole process TREE of any own-user
# `nix build .#checks` client that is UNLEASED (no CDZ_LEASED_NIX=1) and older than a SHORT floor (~8m), so it
# YIELDS CPU under contention yet still finishes at full speed on an idle box — renice, unlike a hard
# cpulimit, does not waste idle throughput or penalize a legit long unleased build (cpulimit is reserved as a
# future escalation only, per v-nix). Non-destructive + reversible → safe on a frequent autonomous cron.
#
# SHARED PRIMITIVES with reap-wedged-nix-clients.sh (v-nix's one blocking concern — this detector MUST share
# the EXACT leased-exemption): (a) the `nix build \.#checks` ps match, and (b) the `/proc/PID/environ`
# CDZ_LEASED_NIX=1 own-user-readable check that EXEMPTS sanctioned leased gates (gate_coarse / run_gate_local
# / run_gate_local_bounded / `fleet with-lease` all set CDZ_LEASED_NIX=1). Keep these in lockstep with
# reap-wedged-nix-clients.sh.
#
# DRY-RUN by default (prints WOULD-RENICE); pass --apply to actually renice. The `# fleet:throttle-unleased-
# nix` cron runs it with --apply. Silent no-op when nothing unleased is over the floor (cron-friendly).
set -u

THRESHOLD_MIN="${THROTTLE_UNLEASED_MIN:-8}" # a bounded single-attr leased check finishes well under this, so
                                            # only a raw unleased heavy aggregate trips it (v-nix: ~8m).
NICE="${THROTTLE_UNLEASED_NICE:-19}"        # +15..+19 (v-nix); +19 = maximum yield-under-contention.
APPLY=0
[ "${1:-}" = "--apply" ] && APPLY=1

log() { printf '%s throttle-unleased-nix: %s\n' "$(date -u +%H:%M:%S)" "$*"; }

me="$(id -u)"
# (1) Own-user `nix build .#checks` clients older than the short floor (etimes = elapsed seconds).
# EXCLUDE the `fleet with-lease -- nix build .#checks…` WRAPPER at detection time (`!/with-lease/`): it is a
# SANCTIONED leased launch, but while it BLOCKS waiting for a lease slot it has not spawned its child yet, so
# a subtree marker-check alone would miss it (no child → no marker) and throttle a leased-intent process
# whose future child would inherit the nice. The argv match catches it in every state; tree_is_leased below
# then covers the running-wrapper + actual-build cases. (`!/awk/` drops this pipeline's own awk.)
candidates="$(ps -eo pid,uid,etimes,args 2>/dev/null \
  | awk -v me="$me" -v minage=$((THRESHOLD_MIN * 60)) \
      '$2 == me && $3 > minage && /nix build \.#checks/ && !/with-lease/ && !/awk/ {print $1}')"

# renice a pid AND all its descendants: a `.#checks` aggregate forks N rustc (own-user here, since builds run
# as us not nixbld), and renicing only the parent leaves the children hogging (v-nix). Recursive + idempotent
# (nice caps at 19, so a repeated cron pass is a no-op). Own-user renice UP needs no privilege.
renice_tree() {
  local pid="$1" kid
  renice -n "$NICE" -p "$pid" >/dev/null 2>&1 || true
  for kid in $(pgrep -P "$pid" 2>/dev/null); do renice_tree "$kid"; done
}

# True if pid OR ANY descendant carries CDZ_LEASED_NIX=1 — the SAME env marker reap-wedged uses, but checked
# over the whole SUBTREE. WHY the subtree (not just the pid): a `fleet with-lease -- nix build .#checks…`
# WRAPPER process matches the `nix build .#checks` argv yet sets CDZ_LEASED_NIX=1 on its CHILD (the real nix
# build), NOT on its own environ — so a self-only check would MISS the wrapper and then renice_tree would
# throttle its LEASED children. Checking the subtree uniformly exempts every sanctioned leased build (the
# wrapper, run_gate_local, gate_coarse, …). FALSE-NEGATIVE-BIASED like reap-wedged: better to miss throttling
# than to slow a leased build. Short-circuits on the first marker found.
tree_is_leased() {
  local pid="$1" kid
  grep -qz 'CDZ_LEASED_NIX=1' "/proc/$pid/environ" 2>/dev/null && return 0
  for kid in $(pgrep -P "$pid" 2>/dev/null); do tree_is_leased "$kid" && return 0; done
  return 1
}

throttled=0
for p in $candidates; do
  # (2) EXEMPT a sanctioned leased build (self OR any descendant carries CDZ_LEASED_NIX=1).
  if tree_is_leased "$p"; then
    log "SKIP leased pid=$p (CDZ_LEASED_NIX=1 in its tree — sanctioned build / with-lease wrapper)"
    continue
  fi
  info="$(ps -o etimes=,args= -p "$p" 2>/dev/null | tr -s ' ' | cut -c1-100)"
  if [ "$APPLY" = 1 ]; then
    renice_tree "$p"
    log "THROTTLED tree pid=$p to nice +$NICE (unleased raw .#checks build > ${THRESHOLD_MIN}min — yields the pool):$info"
  else
    log "WOULD-RENICE tree pid=$p to nice +$NICE (dry-run; pass --apply):$info"
  fi
  throttled=$((throttled + 1))
done

[ "$throttled" = 0 ] && exit 0 # nothing unleased over the floor → silent no-op
log "$([ "$APPLY" = 1 ] && echo throttled || echo 'would throttle') ${throttled} unleased heavy build(s)."
