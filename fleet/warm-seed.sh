#!/usr/bin/env bash
# warm-seed.sh — a shared NATIVE cranelift/wasmtime dep-closure seed, hardlinked into fresh worktree
# target/ dirs so the tight-loop `cargo build -p rcdzc -p cdz -p cdz-run` SKIPS recompiling the closure
# (operator seq-250/195). The #1 fleet CPU sink was 23 worktrees each recompiling cranelift into their own
# target/ (~55769 %CPU-sum, ~70G redundant). A NATIVE seed's fingerprints MATCH a native build cross-
# worktree (same toolchain + shared ~/.cargo registry source-id + no extra rustflags) — VALIDATED
# 2026-08-30: hardlink-copy a native target/debug to a DIFFERENT target dir → `cargo build -p cdz-run`
# finished 0.21s, 0 cranelift recompiled. (A nix/crane seed FAILS here — crane's vendored source-id ≠
# native's crates.io source-id → fingerprint mismatch → recompile. So the seed MUST be natively built.)
#
#   warm-seed.sh --build   : (re)build the seed at $SEED/target IF Cargo.lock changed (else a cheap no-op).
#                            Cron-driven (deps bump rarely, so it's a no-op most runs; ~40s when it rebuilds).
#   warm-seed.sh --seed    : hardlink the seed's target/debug into $PWD/target/debug IF that's empty/fresh
#                            (a new or #5649-reclaimed worktree). window.sh-driven (run from the worktree).
#
# SAME-FS REQUIREMENT: $SEED must be on the SAME filesystem as the worktrees so `cp -al` HARDLINKS (~0 disk,
# instant); a cross-fs fallback would COPY the multi-GB closure into every worktree (worse than the recompile
# it replaces), so --seed REFUSES to cross-fs-copy (warns instead). $HOME and the worktrees are same-fs here.
# FAIL-OPEN throughout: any hiccup exits 0 (never block a launch or a build); cargo just recompiles as before.
set -uo pipefail

SEED="${CDZ_WARM_SEED_DIR:-$HOME/.cdz-warm-seed}"
SEED_TARGET="$SEED/target"
LOCKHASH="$SEED/cargo-lock.hash"
MARK=".cdz-warm-seeded"          # per-worktree marker (in $PWD) so --seed runs once per worktree

FLAKE="$(git rev-parse --show-toplevel 2>/dev/null || echo .)"

case "${1:-}" in
  --build)
    lock="$FLAKE/Cargo.lock"
    [ -f "$lock" ] || exit 0
    h="$(sha256sum "$lock" 2>/dev/null | cut -d' ' -f1)"
    [ -n "$h" ] || exit 0
    # Fresh already? (hash matches + a real seed exists) → cheap no-op.
    if [ "$(cat "$LOCKHASH" 2>/dev/null || echo)" = "$h" ] && [ -d "$SEED_TARGET/debug/.fingerprint" ]; then
      exit 0
    fi
    mkdir -p "$SEED" 2>/dev/null || exit 0
    # SINGLE-BUILDER FLOCK: a cron fires this on EVERY agent, so a Cargo.lock bump would otherwise have all
    # ~23 rebuild the seed AT ONCE (23× the very cranelift storm we're eliminating). flock -n → exactly ONE
    # agent rebuilds; the rest fail the lock + exit (the winner records the hash, so their NEXT run no-ops).
    exec 9>"$SEED/.build.lock" 2>/dev/null || exit 0
    flock -n 9 || exit 0
    # Re-check freshness INSIDE the lock (another builder may have just finished while we waited to try-lock).
    if [ "$(cat "$LOCKHASH" 2>/dev/null || echo)" = "$h" ] && [ -d "$SEED_TARGET/debug/.fingerprint" ]; then
      exit 0
    fi
    # Build the dep-closure NATIVELY into the seed target dir with the SAME `-p` set the tight-loop uses
    # (feature unification must match, or cargo's per-crate metadata hash differs and the seed won't apply —
    # VALIDATED 2026-08-30: matching `-p rcdzc -p cdz -p cdz-run` → a seeded fresh worktree recompiles 0
    # cranelift; a MISMATCHED `-p cdz-run` recompiles it). cdz-run pulls the wasmtime/cranelift closure. NO
    # extra rustflags / CARGO_HOME (must match the native tight-loop's fingerprint inputs). Bounded by the
    # inherited CARGO_BUILD_JOBS; 10-min cap so a wedged build can't hang the cron. Bypass the cargo shim
    # (this IS the canonical native build). On success, record the lock hash so subsequent runs no-op.
    if CDZ_NO_CARGO_SHIM=1 CARGO_TARGET_DIR="$SEED_TARGET" timeout 600 \
         cargo build -p rcdzc -p cdz -p cdz-run >/dev/null 2>&1; then
      echo "$h" > "$LOCKHASH" 2>/dev/null || true
    fi
    ;;

  --seed)
    [ -d "$SEED_TARGET/debug/.fingerprint" ] || exit 0        # no seed built yet → nothing to seed
    [ -e "$MARK" ] && exit 0                                  # already seeded this worktree
    # Only seed a FRESH/empty target (never clobber an in-progress build's artifacts).
    if [ ! -d target/debug/.fingerprint ]; then
      # Same-fs guard: only hardlink; refuse a cross-fs GB copy (would be worse than the recompile).
      mkdir -p target 2>/dev/null || exit 0
      seed_dev="$(stat -c %d "$SEED_TARGET/debug" 2>/dev/null || echo x)"
      tgt_dev="$(stat -c %d target 2>/dev/null || echo y)"
      if [ "$seed_dev" != "$tgt_dev" ]; then
        echo "warm-seed: SEED ($SEED_TARGET) is on a different filesystem than $PWD/target — refusing a" >&2
        echo "  cross-fs GB copy (set CDZ_WARM_SEED_DIR to a same-fs path). Skipping; cargo will recompile." >&2
        exit 0
      fi
      # Hardlink the whole target/debug (cargo re-checks first-party crates vs this worktree's source and
      # rebuilds only those; the unchanged deps — cranelift/wasmtime — keep their matching fingerprints and
      # are SKIPPED). Preserves mtimes (cargo freshness). Fail-open on any error.
      if cp -al "$SEED_TARGET/debug" target/debug 2>/dev/null; then
        touch "$MARK" 2>/dev/null || true
        echo "warm-seed: hardlinked the cranelift/wasmtime dep-closure into $PWD/target/debug (skips recompile)." >&2
      fi
    else
      touch "$MARK" 2>/dev/null || true   # target already populated → mark seeded, don't re-check
    fi
    ;;

  *)
    echo "usage: warm-seed.sh --build | --seed" >&2
    exit 2
    ;;
esac
exit 0
