#!/usr/bin/env bash
# prune-stale-targets.sh — reclaim disk by removing STALE worktree Rust `target/` dirs.
#
# WHY: on this host the nix store is tiny (~8G) but the worktree `target/` dirs are 30-46G EACH
# across ~40 worktrees (~1.5T). A disk-full here is a target/-hygiene problem, NOT a nix-GC problem
# (the auto-GC cannot reclaim target/), and a disk-full forces perpetual futile store-GC that holds the
# big-GC-lock and stalls ALL nix builds/gates fleet-wide. This prunes the stale target/ dirs safely.
#
# SAFETY (a target/ is pruned only when its OWNER is provably not building into it right now):
#   • STOPPED agent (heartbeat missing / older than ACTIVE_WINDOW_MIN) + target/ mtime older than
#     TARGET_STALE_MIN → prune (the long-standing dead-agent path).
#   • ALIVE agent (all-nix idle-reclaim, pending-operator-confirm 2026-08-29) → prune ONLY IF target/ mtime
#     is older than the STRICTER IDLE_TARGET_STALE_MIN (default 48h). RATIONALE: under the all-nix mandate an
#     active agent builds via `nix run .#build` (the shared store), so it no longer writes its own target/;
#     a target/ untouched for 2 days is idle BLOAT even for a live agent. This is SAFE because target/ mtime
#     is a valid liveness proxy for "a build is running" — cargo/rustc write into target/ CONSTANTLY during a
#     build (see [[mtime-age-floor-is-a-liveness-proxy-only-for-dirs-an-active-process-writes-to]]), so a
#     48h-cold target/ has no active build. The tight-loop cargo carve-out (CDZ_NO_CARGO_SHIM=1) DOES write
#     target/, but its builds are seconds/minutes → its target/ stays FRESH → protected by the mtime gate.
#   • RACE GUARD: the target/ mtime is RE-CHECKED immediately before `rm` — if a build started during the
#     scan (mtime went fresh), the prune is ABORTED for that dir.
# The live coordination agents are ALWAYS excluded. `target/` is a regenerable cache independent of nix
# builds (which use the store), so a pruned agent only recompiles once on its next cargo build.
#
# DRY-RUN by default (prints WOULD-PRUNE). Pass --apply to actually delete.
# Paths derive from this script's location in the canonical <repo>/.claude/fleet/ hub. This file is
# TRACKED at <repo>/fleet/ but RUN from the hub copy that `fleet up` materializes into
# <hub>/.claude/fleet/ (same tracked→runtime split as window.sh) — running the tracked source directly
# would resolve WORKTREES/heartbeat against the wrong dir, so invoke the deployed hub copy.
set -euo pipefail

HUB="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKTREES="$(cd "$HUB/../worktrees" && pwd)"
ACTIVE_WINDOW_MIN="${ACTIVE_WINDOW_MIN:-180}"   # heartbeat within this many minutes => agent alive
TARGET_STALE_MIN="${TARGET_STALE_MIN:-1440}"    # STOPPED-agent path: target/ mtime younger than this => skip
IDLE_TARGET_STALE_MIN="${IDLE_TARGET_STALE_MIN:-2880}" # ALIVE-agent idle-reclaim: prune only if target/ older than this (48h; stricter than the dead-agent path since a live agent may resume)
# Never prune these worktrees' target/. concierge/github-liaison/slack-bridge are the live coordination
# agents. pr-sync is included too: its registry row is STOPPED, but the concierge maintenance cron runs
# `cargo xtask fleet watchdog` FROM the pr-sync worktree every tick — so pruning its target/ forces the
# next watchdog to recompile xtask cold (~32s measured), a recurring penalty (pr-sync report 2026-08-27).
# It is effectively a live-coordination host (the watchdog's home), so keep its build warm.
EXCLUDE_AGENTS="${EXCLUDE_AGENTS:-concierge github-liaison slack-bridge pr-sync}"

APPLY=0
[ "${1:-}" = "--apply" ] && APPLY=1
now="$(date +%s)"

printf 'prune-stale-targets: hub=%s active-window=%smin target-stale=%smin idle-target-stale=%smin apply=%s\n' \
  "$HUB" "$ACTIVE_WINDOW_MIN" "$TARGET_STALE_MIN" "$IDLE_TARGET_STALE_MIN" "$APPLY"

pruned=0 skipped_alive=0 skipped_fresh=0 skipped_excl=0
for tdir in "$WORKTREES"/*/target; do
  [ -d "$tdir" ] || continue
  agent="$(basename "$(dirname "$tdir")")"

  case " $EXCLUDE_AGENTS " in *" $agent "*) skipped_excl=$((skipped_excl+1)); continue;; esac

  hb="$HUB/heartbeat/$agent"
  hb_age="none"; alive=0
  if [ -f "$hb" ]; then
    hb_age=$(( (now - $(stat -c %Y "$hb")) / 60 ))
    [ "$hb_age" -lt "$ACTIVE_WINDOW_MIN" ] && alive=1
  fi

  # Applicable staleness threshold: an ALIVE agent's target/ must be idle LONGER (IDLE_TARGET_STALE_MIN,
  # 48h) than a STOPPED agent's (TARGET_STALE_MIN, 24h) before we reclaim it — a live agent may resume, so
  # only reclaim a target/ that has been cold well beyond any tight-loop build.
  if [ "$alive" = 1 ]; then thresh="$IDLE_TARGET_STALE_MIN"; else thresh="$TARGET_STALE_MIN"; fi

  t_age=$(( (now - $(stat -c %Y "$tdir")) / 60 ))
  if [ "$t_age" -lt "$thresh" ]; then
    if [ "$alive" = 1 ]; then skipped_alive=$((skipped_alive+1)); else skipped_fresh=$((skipped_fresh+1)); fi
    continue
  fi

  kind="$([ "$alive" = 1 ] && echo alive-idle || echo stopped)"
  if [ "$APPLY" = 1 ]; then
    # RACE GUARD: re-stat with a FRESH clock immediately before rm — if a build started during the scan,
    # target/ mtime is now fresh (< thresh) → ABORT this prune (never rm a dir a build is writing into).
    now2="$(date +%s)"
    t_age2=$(( (now2 - $(stat -c %Y "$tdir" 2>/dev/null || echo "$now2")) / 60 ))
    if [ "$t_age2" -lt "$thresh" ]; then
      printf 'SKIP (build started mid-scan) %s (target_age now %sm < %sm)\n' "$tdir" "$t_age2" "$thresh"
      skipped_fresh=$((skipped_fresh+1)); continue
    fi
    printf 'PRUNE %s (%s; hb_age=%sm target_age=%sm)\n' "$tdir" "$kind" "$hb_age" "$t_age"
    rm -rf "$tdir"
  else
    printf 'WOULD-PRUNE %s (%s; hb_age=%sm target_age=%sm thresh=%sm)\n' "$tdir" "$kind" "$hb_age" "$t_age" "$thresh"
  fi
  pruned=$((pruned+1))
done

printf 'prune-stale-targets: %s target/ dir(s) %s; skipped alive=%s fresh=%s excluded=%s\n' \
  "$pruned" "$([ "$APPLY" = 1 ] && echo pruned || echo would-be-pruned)" \
  "$skipped_alive" "$skipped_fresh" "$skipped_excl"

# Heartbeat (best-effort): OVERWRITE a `.last-run` file next to the script — mtime = liveness proof the
# (silent) cron fired, content = last result. See prune-tmp-inodes.sh for the rationale (concierge silent-
# cron observability, 2026-08-29). Never fails the prune.
printf '%s apply=%s pruned=%s\n' "$(date -Is)" "$APPLY" "$pruned" \
  > "$(dirname "${BASH_SOURCE[0]}")/prune-stale-targets.last-run" 2>/dev/null || true
