#!/usr/bin/env bash
# cargo-nix-shim — installed as ~/.local/bin/cargo (which sits BEFORE rustup's ~/.cargo/bin on the agent
# snapshot PATH), so it intercepts the fleet's `cargo` invocations.
#
# WHY (all-nix cutover, operator 2026-08-28: "use nix for test runs"): route `cargo test -p CRATE` to the
# nix per-crate test app (`nix run <flake>#test -- CRATE`, v-nix #5129) — which inherits the shared crane
# cargoArtifacts (deps compiled ONCE fleet-wide) and recompiles only the top crate — instead of a bare
# cargo recompile per worktree. The nix test app is the run surface; there is deliberately no `test` PATH
# wrapper (it would shadow the shell/coreutils `test` builtin), so this cargo shim is the interception point.
#
# SAFETY — this shadows `cargo` for the WHOLE fleet (every `cargo xtask fleet …` orchestration call flows
# through it), so a bug here is a fleet-wide wedge. Therefore:
#   • FAIL-OPEN: the DEFAULT for every invocation is to exec the REAL cargo unchanged. Only a narrow,
#     unambiguous `cargo test -p CRATE` (single crate, NO positional test-name filter) is redirected.
#   • FAST-PATH: `cargo` invocations that are not `test` (build/run/clippy/xtask/…) exec real cargo
#     immediately — orchestration (`cargo xtask fleet …`) is never touched.
#   • CONSERVATIVE: a positional filter (`cargo test -p CRATE some_test`), a bare `cargo test` (no -p),
#     multiple `-p`, or ANY ambiguity → real cargo (nix's per-crate app can't honor a test-name filter,
#     and cargo must stay correct). Only the clean whole-crate case migrates to nix.
#   • KILL-SWITCH: `CDZ_NO_CARGO_SHIM=1` bypasses the shim entirely (exec real cargo) — emergency escape.
#   • NON-RECURSION: the real cargo is the first `cargo` on PATH that is NOT this shim; if none is found
#     the shim refuses (exit 127) rather than exec itself.
set -uo pipefail

# Resolve the REAL cargo: first `cargo` on PATH that is not THIS shim.
_self="$(command -v -- "$0" 2>/dev/null || true)"
[ -n "$_self" ] || _self="$HOME/.local/bin/cargo"
_real=""
_oldifs="$IFS"; IFS=:
for _d in $PATH; do
  if [ -n "$_d" ] && [ -x "$_d/cargo" ] && [ "$_d/cargo" != "$_self" ]; then _real="$_d/cargo"; break; fi
done
IFS="$_oldifs"

run_real() {
  if [ -n "$_real" ]; then exec "$_real" "$@"; fi
  echo "cargo-shim: could not locate the real cargo on PATH (refusing to recurse)." >&2
  exit 127
}

# Emergency bypass + fast-path everything that isn't `cargo test`.
[ -n "${CDZ_NO_CARGO_SHIM:-}" ] && run_real "$@"
[ "${1:-}" = "test" ] || run_real "$@"

# Parse the `test` args. Redirect ONLY `test -p CRATE [flags-only]` (exactly one crate, no positional
# test-name filter, no `--` binary args) → nix. Anything else → real cargo (safe default).
shift
_ncrate=0; _crate=""; _positional=0; _want_crate=0
for a in "$@"; do
  if [ "$_want_crate" = 1 ]; then _crate="$a"; _ncrate=$((_ncrate + 1)); _want_crate=0; continue; fi
  case "$a" in
    -p|--package) _want_crate=1 ;;
    -p=*) _crate="${a#-p=}"; _ncrate=$((_ncrate + 1)) ;;
    --package=*) _crate="${a#--package=}"; _ncrate=$((_ncrate + 1)) ;;
    --) _positional=1 ;;   # everything after `--` goes to the test binary (a filter) → cargo
    -*) : ;;               # a flag (e.g. --release, --lib) — allowed
    *) _positional=1 ;;    # a bare positional (a test-name filter, or a flag's value) → cargo
  esac
done

if [ "$_ncrate" = 1 ] && [ -n "$_crate" ] && [ "$_positional" = 0 ]; then
  _flake="$(git rev-parse --show-toplevel 2>/dev/null || echo .)"
  echo "cargo-shim: routing 'cargo test -p $_crate' → nix run $_flake#test -- $_crate (all-nix: cached deps, top-crate recompile; bypass with CDZ_NO_CARGO_SHIM=1)." >&2
  exec nix run "$_flake#test" -- "$_crate"
fi
run_real test "$@"
