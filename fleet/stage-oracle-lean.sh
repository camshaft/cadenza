#!/usr/bin/env bash
# stage-oracle-lean.sh — nightly-build the Lean `.#oracle-lean` and stage its `oracle-check` binary at a
# stable hub path, so cdz-smith's TYPE-DIFFERENTIAL sweep (fuzz-cycle.sh, v-cdz-smith #9314 — the third
# oracle dimension: compiler type judgment vs the Lean oracle) can ACTIVATE. As shipped that sweep is GATED
# on a discoverable fresh oracle (`CDZ_SMITH_ORACLE_CHECK` or `oracle-check` on PATH) and otherwise skips
# cleanly (a stale oracle would file false-reject/accept noise) — so nothing runs it in-fleet because nothing
# stages a fresh oracle. This cron closes that gap: it builds the oracle to `<hub>/.claude/fleet/oracle-lean`
# and window.sh exports `CDZ_SMITH_ORACLE_CHECK` to `<that>/bin/oracle-check` when present. (v-fleet-tooling,
# v-cdz-smith request 2026-09-19.)
#
# COST: `.#oracle-lean` builds LOCALLY (not on the binary cache — only small deps are fetched), so the FIRST
# build is real; but once realized it stays in the store, so a nightly re-run is a CHEAP no-op except when the
# Lean oracle sources change (same cost profile as warm-keep / baseline-drift). We EVAL the target outPath
# (cheap) and skip the build entirely when the out-link already resolves to it — so steady-state nightly runs
# do no work. Bounded by a wall-clock `timeout` so a wedged/starved build can't hang the cron, and FAIL-OPEN
# throughout (any hiccup exits 0): a missing/failed oracle just leaves the type sweep skipping, never breaks
# anything. DAILY at an off-minute (the build is heavy only rarely and timing is not correctness-sensitive).
#
# Paths derive from this script's hub location (tracked at <repo>/fleet/, RUN from the <hub>/.claude/fleet/
# copy `fleet up` materializes — same split as cpu-monitor.sh / baseline-drift-monitor.sh). `nix build` needs
# a worktree (the flake lives there, not in the bare hub), so it picks a current-main-ish worktree.
set -uo pipefail

HUB="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKTREES="$(cd "$HUB/../worktrees" 2>/dev/null && pwd || true)"
[ -n "${WORKTREES:-}" ] && [ -d "$WORKTREES" ] || { echo "stage-oracle-lean: no worktrees dir — skip." >&2; exit 0; }

OUTLINK="$HUB/oracle-lean"                          # stable out-link → result; bin/oracle-check underneath
BUILD_TIMEOUT="${CDZ_ORACLE_LEAN_BUILD_TIMEOUT:-2400}"  # wall-clock cap (s) so a starved build can't hang cron
LASTRUN="$HUB/stage-oracle-lean.last-run"           # mtime = fired proof (silent-cron observability)

# cargo/nix live under the toolchain dirs; cron's minimal PATH lacks them (cf. compact-nudge.sh). Prepend so
# `nix` resolves under cron. Harmless when already present.
export PATH="$HOME/.nix-profile/bin:/nix/var/nix/profiles/default/bin:$HOME/.cargo/bin:/usr/local/bin:$PATH"
command -v nix >/dev/null 2>&1 || { echo "stage-oracle-lean: no nix on PATH — skip (fail-open)." >&2; exit 0; }

# Pick a current-main-ish worktree with the flake (freshest HEAD at/behind origin/main), like warm-keep.sh /
# baseline-drift-monitor.sh — a stale worktree still evaluates the same flake output, but prefer the tip.
main_sha=""
for wt in "$WORKTREES"/*/; do
  [ -f "${wt}flake.nix" ] || continue
  main_sha="$(git -C "$wt" rev-parse --verify -q origin/main 2>/dev/null || true)"; [ -n "$main_sha" ] && break
done
best="" best_ct=-1 fallback="" fallback_ct=-1
for wt in "$WORKTREES"/*/; do
  [ -f "${wt}flake.nix" ] || continue
  head="$(git -C "$wt" rev-parse --verify -q HEAD 2>/dev/null || true)"; [ -n "$head" ] || continue
  ct="$(git -C "$wt" show -s --format=%ct HEAD 2>/dev/null || echo 0)"
  [ "$ct" -gt "$fallback_ct" ] && { fallback_ct="$ct"; fallback="$wt"; }
  if [ -n "$main_sha" ] && { [ "$head" = "$main_sha" ] || git -C "$wt" merge-base --is-ancestor "$head" "$main_sha" 2>/dev/null; }; then
    [ "$ct" -gt "$best_ct" ] && { best_ct="$ct"; best="$wt"; }
  fi
done
wt="${best:-$fallback}"
[ -n "$wt" ] && [ -f "${wt}flake.nix" ] || { echo "stage-oracle-lean: no worktree with a flake — skip." >&2; exit 0; }

# SINGLE-BUILDER FLOCK so an overrunning nightly build can't overlap the next fire (or a manual run).
exec 9>"$HUB/.stage-oracle-lean.lock" 2>/dev/null || exit 0
flock -n 9 || { echo "stage-oracle-lean: another run holds the lock — skip." >&2; exit 0; }

# FRESHNESS GATE (steady-state no-op): eval the target outPath (cheap) and skip the build when the out-link
# already resolves to it AND it's realized in the store. Only a changed oracle (new outPath) triggers a build.
want="$(cd "$wt" && timeout 120 nix eval --raw --accept-flake-config '.#oracle-lean.outPath' 2>/dev/null || true)"
if [ -n "$want" ] && [ -e "$want" ] && [ "$(readlink -f "$OUTLINK" 2>/dev/null || true)" = "$want" ] && [ -x "$OUTLINK/bin/oracle-check" ]; then
  printf '%s rc=0 fresh outPath=%s (no build)\n' "$(date -Is 2>/dev/null || echo now)" "$want" > "$LASTRUN" 2>/dev/null || true
  echo "stage-oracle-lean: oracle-check already fresh at $OUTLINK/bin/oracle-check — no build."
  exit 0
fi

# BUILD + stage. `--out-link` gives a stable, discoverable path (no ./result clutter in the worktree). Bounded
# by `timeout`; if it is starved/killed the out-link is untouched (nix is atomic) and the sweep just stays
# skipping. `--accept-flake-config` matches the flake's trusted-substituter config used elsewhere.
echo "stage-oracle-lean: building .#oracle-lean (out-link $OUTLINK, timeout ${BUILD_TIMEOUT}s) from $(basename "$wt") ..."
if (cd "$wt" && timeout "$BUILD_TIMEOUT" nix build --accept-flake-config --out-link "$OUTLINK" '.#oracle-lean' >/dev/null 2>&1) \
   && [ -x "$OUTLINK/bin/oracle-check" ]; then
  printf '%s rc=0 BUILT outPath=%s\n' "$(date -Is 2>/dev/null || echo now)" "$(readlink -f "$OUTLINK" 2>/dev/null)" > "$LASTRUN" 2>/dev/null || true
  echo "stage-oracle-lean: staged oracle-check at $OUTLINK/bin/oracle-check (type-differential sweep will activate for cdz-smith on its next relaunch)."
else
  # rc=1 so a PERSISTENT build failure surfaces in `fleet status` via cron_health_failures (a transient
  # starvation self-clears on the next successful night's rc=0). Fail-open behavior is unchanged.
  printf '%s rc=1 BUILD-FAILED-OR-TIMED-OUT\n' "$(date -Is 2>/dev/null || echo now)" > "$LASTRUN" 2>/dev/null || true
  echo "stage-oracle-lean: build failed or timed out — type sweep stays skipping (fail-open, retry next run)." >&2
fi
exit 0
