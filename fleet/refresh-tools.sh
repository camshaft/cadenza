#!/usr/bin/env bash
# refresh-tools.sh — (re)build the nix entrypoint wrappers from the CURRENT worktree flake and (re)symlink
# them onto ~/.local/bin, so an agent's tool PATH tracks the flake's current app SET.
#
# WHY (all-nix cutover, operator 2026-08-28): the wrappers (cdz/cdz-run/cdz-compile/roundtrip/gate/
# fast-gate/cdz-help; v-nix `packages.cdz-shell-wrappers`) each `exec nix run <worktree>#app`, so `nix run`
# RE-EVALUATES the flake + rebuilds from the dirty worktree on every call — TOOL BEHAVIOR is therefore
# ALWAYS current with local source without any refresh. What THIS syncs is the SET of wrappers on PATH:
# when an app is ADDED or RENAMED in the flake (e.g. as xtask subcommands are decomposed into nix apps),
# the ~/.local/bin symlinks must be re-created to pick it up. So run this after the flake can have changed.
#
# CALLERS: window.sh (agent launch), the post-merge/post-checkout git hooks (git-native pull/checkout),
# and `cargo xtask fleet sync` (the AGENT update path — sync uses `git reset --hard`, which does NOT fire
# git hooks, so the hook alone would miss agents). The sync caller sets REFRESH_MIN_INTERVAL_SEC (throttle)
# so a per-tick sync does not re-eval nix every time; launch/hook callers leave it 0 (always refresh).
#
# FAIL-OPEN + timeout-bounded + idempotent: any failure exits 0 (never block a launch/sync/checkout on the
# tool refresh); a wedged daemon can't hang the caller (300s cap); re-symlinking is harmless if unchanged.
set -uo pipefail

BIN="$HOME/.local/bin"
ROOT="$HOME/.cdz-warm-roots/cdz-shell-wrappers"
STAMP="$ROOT.last-refresh"
MIN_INTERVAL="${REFRESH_MIN_INTERVAL_SEC:-0}"   # 0 = always; a frequent caller (sync) sets e.g. 1800 to throttle

# Throttle: skip if we refreshed within MIN_INTERVAL seconds (keeps the per-tick sync path cheap).
if [ "$MIN_INTERVAL" -gt 0 ] && [ -f "$STAMP" ]; then
  now="$(date +%s 2>/dev/null || echo 0)"
  last="$(cat "$STAMP" 2>/dev/null || echo 0)"
  [ $((now - last)) -lt "$MIN_INTERVAL" ] && exit 0
fi

# The flake is the invoking worktree (callers run with cwd in the worktree); fall back to `.`.
FLAKE="$(git rev-parse --show-toplevel 2>/dev/null || echo .)"
mkdir -p "$BIN" "$(dirname "$ROOT")" 2>/dev/null || exit 0

if timeout 300 nix build "$FLAKE#packages.aarch64-linux.cdz-shell-wrappers" --out-link "$ROOT" 2>/dev/null; then
  ln -sf "$ROOT"/bin/* "$BIN"/ 2>/dev/null || true
  date +%s > "$STAMP" 2>/dev/null || true
fi
exit 0
