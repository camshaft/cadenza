#!/usr/bin/env bash
# warm-keep.sh — periodically re-root the LOCAL nix warm layer (corpus aggregates + crane deps + store +
# local-gate) so gate-local cache-HITS instead of triggering a COLD whole-corpus sweep (the 2026-08-28
# fleet-wide daemon wedge). This is the SCHEDULER for the flake app `apps.warm-keep`: v-nix owns the app
# (flake.nix), v-fleet-tooling owns the cron that runs it (this script), per the concierge/v-nix handoff
# 2026-08-30. Without a periodic invocation the corpus GC-roots drop to ZERO (observed all-session
# 2026-08-30) and the cold-sweep wedge risk returns.
#
# WHY A WRAPPER (not a raw `nix run .#warm-keep` in the crontab): the fleet HUB is a BARE repo (no
# flake.nix, no working tree), and `apps.warm-keep` builds its targets with a CWD-relative `nix build
# ".#$t"` flake ref (its --out-links ARE absolute, so only the inner flake ref is CWD-sensitive — v-nix
# confirmed 2026-08-30). So warm-keep must be invoked FROM a real flake working tree. This wrapper picks a
# CURRENT-MAIN-ish flake worktree (freshest HEAD that is at/behind origin/main — warm-keep warms the corpus
# of WHATEVER rev that worktree is on, and roots only cache-HIT for agents on the SAME rev; a feature-branch
# worktree warms the wrong rev, though unchanged cases still hit → graceful degrade), cd's there, and runs
# `nix run .#warm-keep`. (v-nix's follow-up CDZ_WARM_FLAKE parameterization will later make the app itself
# ref-explicit, retiring the worktree-pick — but this wrapper unblocks the scheduler now.)
#
# UNLEASED by design (v-nix-agreed): one warm-keep is strictly better than N agents cold-sweeping the
# corpus, and the HOURLY cadence IS the retry-supervisor — warm-keep is sequential + best-effort + exit-0,
# so a run KILLED mid-corpus under contention still roots what it finished and the next hourly pass
# cache-HITS those + retries the rest → the corpus roots CONVERGE over a few passes. A check-lease would
# just starve it (the seq-43 problem). Best-effort + exit 0 so the cron never alarms on a partial pass.
#
# Paths derive from this script's location in the canonical <hub>/.claude/fleet/ dir (same tracked->runtime
# split as window.sh / cpu-monitor.sh: TRACKED at <repo>/fleet/, RUN from the hub copy `fleet up`
# materializes). Running the tracked source directly would resolve WORKTREES against the wrong dir.
set -uo pipefail

# SILENT-CRON OBSERVABILITY (v-fleet-tooling 2026-09-01; matches drain-nudge #7339 / prune-*.sh / baseline-
# drift-monitor). warm-keep is SILENT on a routine pass (it only speaks on a skip/nonzero), so an all-quiet
# run is indistinguishable from a DEAD cron — nothing proved the hourly cron actually fired. OVERWRITE a
# `.last-run` next to this script on EVERY run via an EXIT trap (fires on ANY exit path — the flock-skip
# early-exit, the no-worktree skip, or a normal finish); its MTIME is the fired-proof. Set BEFORE the flock
# guard so even a skipped pass stamps. Best-effort; never affects behavior or the exit code.
_cdz_lastrun="$(cd "$(dirname "${BASH_SOURCE[0]}")" 2>/dev/null && pwd)/warm-keep.last-run"
trap 'printf "%s\n" "$(date -Is 2>/dev/null || echo now)" > "$_cdz_lastrun" 2>/dev/null || true' EXIT

# SINGLETON GUARD: warm-keep is idempotent + best-effort + can run 15-40min (much longer under a COLD /
# input-addressed-churning corpus, where it may not finish within the hour at all). So the hourly cron (or a
# concurrent manual run) can fire a SECOND invocation while the first is still building → duplicate warm-keeps
# compete for the SAME corpus derivation build = wasted load that ADDS to the very contention warm-keep exists
# to relieve (observed: 3 concurrent cdz-warm-keep under the cold-corpus churn, 2026-08-30). flock -n so only
# ONE warm-keep runs at a time; a later fire SKIPS (exit 0 — the next hourly fire retries, and warm-keep is
# idempotent so nothing is lost). Held for the whole run (fd 9 stays open until this script exits, covering the
# child `nix run`). FAIL-OPEN: if flock is absent or the lock can't be opened, run anyway (never block a
# re-warm on the guard). Lock lives in the warm-root dir (own-user, survives; the app's `rm -f warm*` glob
# never matches a dotfile, so it is not swept).
_lock_dir="${CDZ_WARM_ROOT:-$HOME/.cdz-warm-roots}"
mkdir -p "$_lock_dir" 2>/dev/null || true
if command -v flock >/dev/null 2>&1 && exec 9>"$_lock_dir/.warm-keep.lock" 2>/dev/null; then
  if ! flock -n 9; then
    echo "warm-keep: another warm-keep is already running (flock held) — skipping this invocation (idempotent; next hourly fire retries)." >&2
    exit 0
  fi
fi

HUB="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKTREES="$(cd "$HUB/../worktrees" 2>/dev/null && pwd || true)"
if [ -z "${WORKTREES:-}" ] || [ ! -d "$WORKTREES" ]; then
  echo "warm-keep: no worktrees dir under $HUB/../worktrees — nothing to run from; skipping." >&2
  exit 0
fi

# Pick the flake working tree to invoke warm-keep from. Prefer a CURRENT-MAIN-ish worktree (HEAD == or an
# ancestor of origin/main — i.e. at/behind main, no unlanded feature commits), freshest among those; fall
# back to the freshest-HEAD flake worktree overall if none is at/behind main (degrades gracefully — most
# corpus cases are unchanged and still cache-HIT). origin/main is resolved from the shared object store (no
# fetch — agents' per-tick `fleet sync` keeps it fresh enough, and warming a slightly-behind main is fine).
main_sha=""
for wt in "$WORKTREES"/*/; do
  [ -f "${wt}flake.nix" ] || continue
  main_sha="$(git -C "$wt" rev-parse --verify -q origin/main 2>/dev/null || true)"
  [ -n "$main_sha" ] && break
done

best="" best_ct=-1 fallback="" fallback_ct=-1
for wt in "$WORKTREES"/*/; do
  [ -f "${wt}flake.nix" ] || continue
  head="$(git -C "$wt" rev-parse --verify -q HEAD 2>/dev/null || true)"
  [ -n "$head" ] || continue
  ct="$(git -C "$wt" show -s --format=%ct HEAD 2>/dev/null || echo 0)"
  # freshest overall = the fallback
  if [ "$ct" -gt "$fallback_ct" ]; then fallback_ct="$ct"; fallback="$wt"; fi
  # current-main-ish: HEAD is at origin/main OR an ancestor of it (behind main, no feature commits)
  if [ -n "$main_sha" ]; then
    if [ "$head" = "$main_sha" ] || git -C "$wt" merge-base --is-ancestor "$head" "$main_sha" 2>/dev/null; then
      if [ "$ct" -gt "$best_ct" ]; then best_ct="$ct"; best="$wt"; fi
    fi
  fi
done

flake_wt="${best:-$fallback}"
if [ -z "$flake_wt" ]; then
  echo "warm-keep: no flake worktree found under $WORKTREES — skipping." >&2
  exit 0
fi

echo "warm-keep: invoking \`nix run .#warm-keep\` from $flake_wt (main-ish=$([ -n "$best" ] && echo yes || echo no-fallback-freshest))"
cd "$flake_wt" || { echo "warm-keep: could not cd into $flake_wt — skipping." >&2; exit 0; }
# EXEMPT from the nix-shim: warm-keep's inner `nix build .#checks.corpus-*/.local-gate` are heavy check
# attrs, which the shim otherwise WARN-spams and (post the raw-heavy-check routing) would ROUTE through
# `with-lease`. But warm-keep is INTENTIONALLY UNLEASED (v-nix-agreed): one warm-keep is strictly better than
# N agents cold-sweeping the corpus, and leasing it would just STARVE it (the seq-43 problem) — the hourly
# cadence is its supervisor, not a lease. CDZ_LEASED_NIX=1 marks it sanctioned so the shim passes every inner
# build straight through (unleased, no warn). Inherited by the `nix run` → the app → its `nix build`s.
export CDZ_LEASED_NIX=1
# Best-effort: warm-keep is itself sequential + best-effort + exit-0; a nonzero here (killed under
# contention) is expected and the next hourly pass retries — never alarm the cron.
nix run .#warm-keep || echo "warm-keep: run exited nonzero (likely killed under contention) — next hourly pass retries." >&2
exit 0
