#!/usr/bin/env bash
# cargo-nix-shim — installed as ~/.local/bin/cargo (which sits BEFORE rustup's ~/.cargo/bin on the agent
# snapshot PATH), so it intercepts the fleet's `cargo` invocations.
#
# WHY (all-nix mandate, operator 2026-08-29: "absolutely everything on nix, no ad-hoc cargo, no bloated
# target dirs" — phased): the SOFT-WARN rollout. For a build/test/gate `cargo` invocation this shim prints
# a deprecation warning naming the nix-flake equivalent (from the v-nix-owned `fleet/cargo-nix-hints.tsv`),
# LOGS a gap to `~/.cdz-cargo-gaps.tsv` when no equivalent is mapped yet (v-nix's build list), and STILL
# RUNS cargo (non-blocking) — the per-call warning IS the rollout; a later flip makes it hard-fail once the
# low-hanging fruit is nix-covered (v-nix owns that criterion). ALREADY-ROUTED: a clean `cargo test -p
# CRATE` execs `nix run <flake>#test -- CRATE` (#5129/#5136 — shared crane cargoArtifacts, top-crate-only
# recompile). `cargo xtask fleet …` (the orchestration control plane) is EXEMPT. There is deliberately no
# `test` PATH wrapper (it would shadow the shell `test` builtin), so this cargo shim is the interception point.
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

# Emergency bypass — kill-switch execs real cargo unchanged.
[ -n "${CDZ_NO_CARGO_SHIM:-}" ] && run_real "$@"

# CWD-SCOPE (OPERATOR DIRECTIVE 2026-09-01): this shim sits on the machine-wide agent PATH, so it MUST be a
# pure pass-through OUTSIDE the cadenza repo — otherwise its routing / sccache injection / deprecation
# warnings interfere with other agents & projects sharing this host (operator: "stop intercepting cargo
# invocation outside of the cadenza directory"). Walk up from $PWD; only if an ancestor is a cadenza checkout
# (a dir carrying BOTH flake.nix AND spec/semantics/ — true for the main repo and every git worktree) do we
# fall through to the routing below. Anywhere else → exec the REAL cargo immediately, before sccache/routing/
# warnings run. Cheap (a few stats up the tree); no dependency on shell/window env being set.
_cdz_root=""
_cdz_d="$PWD"
while [ -n "$_cdz_d" ] && [ "$_cdz_d" != "/" ]; do
  if [ -f "$_cdz_d/flake.nix" ] && [ -d "$_cdz_d/spec/semantics" ]; then _cdz_root="$_cdz_d"; break; fi
  _cdz_d="$(dirname -- "$_cdz_d")"
done
[ -n "$_cdz_root" ] || run_real "$@"

# SHARED COMPILE CACHE — no-restart propagation of #5878 (operator seq-267). window.sh sets RUSTC_WRAPPER at
# window LAUNCH only, so the ~32 ALREADY-RUNNING agents don't have it. This shim runs on EVERY cargo call, so
# injecting it HERE means a running agent's next cargo build picks up sccache with NO window restart (once
# refresh-tools re-installs this shim on its next `fleet sync`). Only the native-cargo (run_real) path uses
# it — the nix routes below are hermetic (RUSTC_WRAPPER doesn't enter the nix sandbox). NOTE: this does NOT
# make cranelift cross-worktree-HIT — build-script generated-source path keying structurally defeats that
# (that's warm-target seq-195's job); this delivers the registry-dep + same-worktree PARTIAL win. Respect an
# explicit override; skip if sccache isn't installed.
if [ -z "${RUSTC_WRAPPER:-}" ] && command -v sccache >/dev/null 2>&1; then
  export RUSTC_WRAPPER=sccache
  export SCCACHE_DIR="${SCCACHE_DIR:-$HOME/.cache/sccache-fleet}"
  export SCCACHE_CACHE_SIZE="${SCCACHE_CACHE_SIZE:-20G}"
fi

_sub="${1:-}"

# CONTROL-PLANE EXEMPT: `cargo xtask fleet …` is the fleet orchestration hot loop — NEVER warn/touch it
# (routing every heartbeat/inbox/sync through nix would add per-command eval overhead to the control loop;
# its xtask bin is tiny, not the target-dir bloat). Run the real cargo immediately, silently.
if [ "$_sub" = "xtask" ] && [ "${2:-}" = "fleet" ]; then run_real "$@"; fi

# ROUTE `cargo xtask build` → `nix run <flake>#build` (v-nix apps.build #5603) — the #1 rebuild-the-world
# hotspot: front-end (cdz/cdz-run/cdz-compile) + component-store materialized from the SHARED /nix/store,
# ZERO cargo, target/ becomes symlinks (kills the 174-worktree × multi-GB target/ recompile of cdz/rcdzc/
# deps). Only the bare `cargo xtask build` (no extra args that could change semantics); anything with a
# trailing arg → soft-warn + real cargo (conservative).
if [ "$_sub" = "xtask" ] && [ "${2:-}" = "build" ] && [ -z "${3:-}" ]; then
  _flake="$(git rev-parse --show-toplevel 2>/dev/null || echo .)"
  echo "cargo-shim: routing 'cargo xtask build' → cargo xtask fleet with-lease nix run $_flake#build (all-nix + LEASED: a full store rebuild is a heavy nix build, so it takes a check-lease slot — bounds the concurrent cold-build herd, e.g. the 43-agent input-addressed .#runtime rebuild that oversubscribes the big-nix-lock on a hash-bump flag-day; bypass with CDZ_NO_CARGO_SHIM=1)." >&2
  # with-lease acquires a VERTICAL check-lease (bounded-wait, fail-open) then runs the build → ≤cap concurrent
  # store rebuilds, the rest bounded-wait+trickle. It sets CDZ_LEASED_NIX=1 so the inner `nix run` passes the
  # nix-shim (no double-route/recursion); `cargo xtask fleet …` is cargo-shim control-plane exempt (above) so
  # this re-invocation runs real cargo, not this shim. Deadlock-safe: no lease-holder nests a .#build lease.
  exec cargo xtask fleet with-lease nix run "$_flake#build"
fi

# ROUTE `cargo xtask test` → `nix run <flake>#fast-gate` (v-fleet-tooling, coord v-xtask-decompose seq-202
# 2026-08-29). The xtask `test` GUARDRAIL (which merely REFUSED `cargo test --workspace` + pointed devs at
# dev-gate/fast-gate) is being DELETED to shrink xtask; routing here does the RIGHT cached thing instead of
# erroring — `#fast-gate` is the touched-crate test+clippy+fmt the guardrail already redirected to. This is a
# real EXEC route (a `cargo-nix-hints.tsv` entry only prints a WARN, it never routes — so a hint alone would
# dead-end `cargo xtask test` once `Cmd::Test` is gone). Mirrors the `cargo xtask build` route above; landed
# BEFORE the arm-removal so there is no dead-end window (this pre-empts the vanished subcommand).
if [ "$_sub" = "xtask" ] && [ "${2:-}" = "test" ]; then
  _flake="$(git rev-parse --show-toplevel 2>/dev/null || echo .)"
  echo "cargo-shim: routing 'cargo xtask test' → nix run $_flake#fast-gate (all-nix: cached touched-crate test+clippy+fmt — the guardrail's redirect target; bypass with CDZ_NO_CARGO_SHIM=1)." >&2
  exec nix run "$_flake#fast-gate"
fi

# ROUTE a whole-workspace / front-end `cargo build` (NO `-p`, NO `--target`) → `nix run <flake>#build`. Two
# escape hatches to REAL cargo (silent pass-through), because `.#build` materializes the HOST-NATIVE
# front-end only:
#   • `-p CRATE` — a SPECIFIC crate (maybe not the front-end); falls through to soft-warn + real cargo.
#   • `--target <triple>` — a CROSS-COMPILE (e.g. wasm-pack's INTERNAL `cargo build --target
#     wasm32-unknown-unknown` for cdz-wasm); `.#build` is host-native so routing it broke guide-wasm
#     (v-xtask/v-guide-infra regression on #5606). Pass it straight through, no route, no warn (often
#     tool-internal — warning would be noise).
# `--release`/`--bin cdz…` (no `-p`, no `--target`) still route: `.#build` materializes all front-end bins.
if [ "$_sub" = "build" ]; then
  _has_p=0
  for a in "$@"; do
    case "$a" in
      -p | --package | -p=* | --package=*) _has_p=1 ;;
      --target | --target=*) run_real "$@" ;; # cross-compile → real cargo, silent (no .#build equivalent)
    esac
  done
  if [ "$_has_p" = 0 ]; then
    _flake="$(git rev-parse --show-toplevel 2>/dev/null || echo .)"
    echo "cargo-shim: routing 'cargo build' → cargo xtask fleet with-lease nix run $_flake#build (all-nix + LEASED: the store rebuild takes a check-lease slot to bound the concurrent cold-build herd; a specific crate — cargo build -p CRATE — still runs cargo; bypass CDZ_NO_CARGO_SHIM=1)." >&2
    exec cargo xtask fleet with-lease nix run "$_flake#build"
  fi
fi

# Compute the all-nix hint KEY for this cargo shape (empty = a cargo cmd we don't warn on — `--version`,
# `+toolchain`, `cargo xtask <non-build-subcmd>`, …). `xtask:<sub>` for the build/gate/test xtask subcmds.
_key=""
case "$_sub" in
  xtask)
    case "${2:-}" in
      build | gate | dev-gate | check | test | codegen | bench) _key="xtask:${2}" ;;
    esac
    ;;
  build | run | bench | clippy | fmt) _key="$_sub" ;;
  test) _key="test" ;;
esac

# `cargo test -p CRATE` (clean whole-crate: exactly one crate, NO positional test-name filter, no `--`
# binary args) ROUTES to the nix per-crate test (#5136 — already all-nix: cached deps, top-crate recompile).
# Anything else falls through to the soft-warn + real cargo below.
if [ "$_sub" = "test" ]; then
  shift
  _ncrate=0; _crate=""; _positional=0; _want_crate=0
  for a in "$@"; do
    if [ "$_want_crate" = 1 ]; then _crate="$a"; _ncrate=$((_ncrate + 1)); _want_crate=0; continue; fi
    case "$a" in
      -p | --package) _want_crate=1 ;;
      -p=*) _crate="${a#-p=}"; _ncrate=$((_ncrate + 1)) ;;
      --package=*) _crate="${a#--package=}"; _ncrate=$((_ncrate + 1)) ;;
      --) _positional=1 ;; # everything after `--` goes to the test binary (a filter) → cargo
      -*) : ;;             # a flag (e.g. --release, --lib) — allowed
      *) _positional=1 ;;  # a bare positional (a test-name filter, or a flag's value) → cargo
    esac
  done
  if [ "$_ncrate" = 1 ] && [ -n "$_crate" ] && [ "$_positional" = 0 ]; then
    _flake="$(git rev-parse --show-toplevel 2>/dev/null || echo .)"
    echo "cargo-shim: routing 'cargo test -p $_crate' → nix run $_flake#test -- $_crate (all-nix: cached deps, top-crate recompile; bypass with CDZ_NO_CARGO_SHIM=1)." >&2
    exec nix run "$_flake#test" -- "$_crate"
  fi
  set -- test "$@" # restore argv for the soft-warn + real cargo below
fi

# SOFT-WARN (all-nix mandate, operator 2026-08-29): for a build/test/gate cargo shape, print the
# deprecation warning + the nix equivalent (from the v-nix-owned hint map), or LOG A GAP if unmapped, then
# STILL RUN real cargo (non-blocking — the per-call warning is the rollout; a later flip makes it hard-fail).
if [ -n "$_key" ]; then
  _top="$(git rev-parse --show-toplevel 2>/dev/null || echo .)"
  _hint=""
  if [ -f "$_top/fleet/cargo-nix-hints.tsv" ]; then
    _hint="$(grep -v '^#' "$_top/fleet/cargo-nix-hints.tsv" 2>/dev/null \
      | awk -F'\t' -v k="$_key" '$1 == k { sub(/^[^\t]*\t/, ""); print; exit }')"
  fi
  if [ -n "$_hint" ]; then
    echo "⚠ cargo is being DEPRECATED (all-nix mandate) — prefer: $_hint  [still running cargo; silence: CDZ_NO_CARGO_SHIM=1]" >&2
  else
    # No nix equivalent mapped → record the gap for v-nix to build an equivalent, then still run.
    _gaps="${CDZ_CARGO_GAPS_LOG:-$HOME/.cdz-cargo-gaps.tsv}"
    printf '%s\t%s\tcargo %s\n' "$(date +%s 2>/dev/null || echo 0)" "$_top" "$*" >> "$_gaps" 2>/dev/null || true
    echo "⚠ cargo is being DEPRECATED (all-nix mandate) — no nix equivalent mapped for 'cargo $_sub …' yet; logged for v-nix. [still running cargo; silence: CDZ_NO_CARGO_SHIM=1]" >&2
  fi
fi

run_real "$@"
