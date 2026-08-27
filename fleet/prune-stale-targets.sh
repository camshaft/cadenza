#!/usr/bin/env bash
# prune-stale-targets.sh — reclaim disk by removing STALE worktree Rust `target/` dirs.
#
# WHY: on this host the nix store is tiny (~8G) but the worktree `target/` dirs are 30-46G EACH
# across ~40 worktrees (~1.5T). A disk-full here is a target/-hygiene problem, NOT a nix-GC problem
# (the auto-GC cannot reclaim target/), and a disk-full forces perpetual futile store-GC that holds the
# big-GC-lock and stalls ALL nix builds/gates fleet-wide. This prunes the stale target/ dirs safely.
#
# SAFETY (dual net — a target/ is pruned ONLY IF BOTH hold):
#   1. the agent is NOT alive: its fleet/heartbeat/<agent> file is missing or older than ACTIVE_WINDOW_MIN
#      (a live agent — even one that builds via nix and has a stale target/ — is protected);
#   2. the target/ itself is stale: mtime older than TARGET_STALE_MIN (a recent build is protected).
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
ACTIVE_WINDOW_MIN="${ACTIVE_WINDOW_MIN:-180}"   # heartbeat within this many minutes => agent alive => skip
TARGET_STALE_MIN="${TARGET_STALE_MIN:-1440}"    # target/ mtime younger than this => recent build => skip
# Never prune these worktrees' target/. concierge/github-liaison/slack-bridge are the live coordination
# agents. pr-sync is included too: its registry row is STOPPED, but the concierge maintenance cron runs
# `cargo xtask fleet watchdog` FROM the pr-sync worktree every tick — so pruning its target/ forces the
# next watchdog to recompile xtask cold (~32s measured), a recurring penalty (pr-sync report 2026-08-27).
# It is effectively a live-coordination host (the watchdog's home), so keep its build warm.
EXCLUDE_AGENTS="${EXCLUDE_AGENTS:-concierge github-liaison slack-bridge pr-sync}"

APPLY=0
[ "${1:-}" = "--apply" ] && APPLY=1
now="$(date +%s)"

printf 'prune-stale-targets: hub=%s active-window=%smin target-stale=%smin apply=%s\n' \
  "$HUB" "$ACTIVE_WINDOW_MIN" "$TARGET_STALE_MIN" "$APPLY"

pruned=0 skipped_alive=0 skipped_fresh=0 skipped_excl=0
for tdir in "$WORKTREES"/*/target; do
  [ -d "$tdir" ] || continue
  agent="$(basename "$(dirname "$tdir")")"

  case " $EXCLUDE_AGENTS " in *" $agent "*) skipped_excl=$((skipped_excl+1)); continue;; esac

  hb="$HUB/heartbeat/$agent"
  hb_age="none"
  if [ -f "$hb" ]; then
    hb_age=$(( (now - $(stat -c %Y "$hb")) / 60 ))
    if [ "$hb_age" -lt "$ACTIVE_WINDOW_MIN" ]; then
      skipped_alive=$((skipped_alive+1)); continue
    fi
  fi

  t_age=$(( (now - $(stat -c %Y "$tdir")) / 60 ))
  if [ "$t_age" -lt "$TARGET_STALE_MIN" ]; then
    skipped_fresh=$((skipped_fresh+1)); continue
  fi

  if [ "$APPLY" = 1 ]; then
    printf 'PRUNE %s (hb_age=%sm target_age=%sm)\n' "$tdir" "$hb_age" "$t_age"
    rm -rf "$tdir"
  else
    printf 'WOULD-PRUNE %s (hb_age=%sm target_age=%sm)\n' "$tdir" "$hb_age" "$t_age"
  fi
  pruned=$((pruned+1))
done

printf 'prune-stale-targets: %s target/ dir(s) %s; skipped alive=%s fresh=%s excluded=%s\n' \
  "$pruned" "$([ "$APPLY" = 1 ] && echo pruned || echo would-be-pruned)" \
  "$skipped_alive" "$skipped_fresh" "$skipped_excl"
