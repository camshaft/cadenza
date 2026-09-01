#!/usr/bin/env bash
# nix-shim — installed as ~/.local/bin/nix (BEFORE the real nix on the agent snapshot PATH), so it can
# WARN on the two lease-ESCAPE patterns behind the load-108 daemon incident (all-nix mandate, 2026-08-29).
#
# WHY: a BARE `nix build` of a HEAVY check attr (local-gate / corpus-* / gate-check-* / … — see
# fleet/nix-heavy-attrs.tsv, v-nix-owned) run OUTSIDE the leased `cargo xtask fleet gate-local` path escapes
# the check-lease concurrency cap and can starve the shared daemon; and `--option substitute false` forces a
# wasteful from-source rebuild that monopolizes build-slots. The check-lease (cap 2) can't see either. This
# shim surfaces both at the call site so the leased gate-local becomes the sanctioned heavy-nix entry.
#
# ROUTING (v-fleet-tooling 2026-08-30, concierge-nod — the gate-local STORM fix + the raw-heavy-check bypass):
# a BARE `nix build .#checks.<sys>.<heavy>` run OUTSIDE the lease is ROUTED so it takes the check-lease
# instead of escaping the cap. Two routes: (1) `local-gate` (the authoritative merge gate) → `cargo xtask
# fleet gate-local`, which builds the SAME attr under the WEIGHTED lease (#6004) + priority-lane; (2) ANY
# OTHER heavy attr (corpus-* / gate-check-* / … per the v-nix-owned nix-heavy-attrs.tsv) → `cargo xtask fleet
# with-lease nix build …`, which runs the SAME build under a vertical check-lease slot. WHY: a raw heavy
# `.#checks.*` build bypasses the lease entirely, so N agents run them CONCURRENTLY and self-induce a load
# storm / big-nix-lock thrash that starves the LEASED gate-locals (observed: 21 concurrent local-gates load
# 103 → 0-byte; then raw `.#checks.*.corpus-*` builds starving leased gates at 0.1% CPU, concierge 2026-08-30).
# Routing every heavy raw build to a leased command closes the escape at the call site. FAIL-OPEN: if cargo is
# unavailable neither route fires → WARN + real nix. The intentionally-UNLEASED warm-keep pass sets
# CDZ_LEASED_NIX=1 itself, so it is exempt at the top and never routed. Mirrors the cargo-shim's route pattern.
#
# SAFETY — this shadows `nix` for the WHOLE fleet (the cargo-shim's `nix run .#test/.#build` routes, gate-
# local's own `nix build`, refresh-tools, every `apps` invocation flow through it), so a bug is a fleet-wide
# wedge. Therefore it is FAIL-OPEN + heavily guarded:
#   • The DEFAULT for EVERY invocation is to exec the REAL nix unchanged. The shim only ever adds a stderr
#     WARNING or (for a bare local-gate ONLY) routes to the leased gate-local; it NEVER blocks, and never
#     touches the CDZ_LEASED_NIX-exempt sanctioned builds (so gate-local's OWN inner build is never routed).
#   • CDZ_LEASED_NIX=1 EXEMPT: the sanctioned leased builds (run_gate_local / run_gate_local_bounded /
#     with_lease / the cargo-shim `nix run` routes) set this before exec-ing nix — the shim passes them
#     through SILENTLY. Without this, the shim would warn on gate-local's OWN inner `nix build` of local-gate.
#   • _CDZ_NIX_SHIM_ACTIVE re-entry guard: set before exec-ing the real nix, so any nix-spawns-nix child
#     passes through silently (no recursion, no double-warn).
#   • CDZ_NO_NIX_SHIM=1 KILL-SWITCH: bypass entirely (exec real nix) — emergency escape.
#   • NON-RECURSION: the real nix is the first `nix` on PATH that is NOT this shim; none found → refuse (127).
set -uo pipefail

# Re-entry / exemption / kill-switch → exec the real nix immediately, silently.
_self="$(command -v -- "$0" 2>/dev/null || true)"
[ -n "$_self" ] || _self="$HOME/.local/bin/nix"
_real=""
_oldifs="$IFS"; IFS=:
for _d in $PATH; do
  if [ -n "$_d" ] && [ -x "$_d/nix" ] && [ "$_d/nix" != "$_self" ]; then _real="$_d/nix"; break; fi
done
IFS="$_oldifs"

run_real() {
  if [ -n "$_real" ]; then
    export _CDZ_NIX_SHIM_ACTIVE=1  # mark children so a nix-spawns-nix never re-enters the shim
    exec "$_real" "$@"
  fi
  echo "nix-shim: could not locate the real nix on PATH (refusing to recurse)." >&2
  exit 127
}

[ -n "${_CDZ_NIX_SHIM_ACTIVE:-}" ] && run_real "$@"   # a nix child of an already-shimmed nix → pass through
[ -n "${CDZ_NO_NIX_SHIM:-}" ] && run_real "$@"        # kill-switch
[ -n "${CDZ_LEASED_NIX:-}" ] && run_real "$@"         # sanctioned leased build → silent pass-through

# CWD-SCOPE GUARD (operator 2026-09-01): ~/.local/bin/nix shadows `nix` for EVERY process of THIS user, so on
# a shared box it would otherwise warn/route a `nix build` in OTHER repos that have nothing to do with Cadenza
# — interfering with agents NOT working on Cadenza (operator directive: stop intercepting outside the cadenza
# directory). Every warn/route here is Cadenza-specific (the heavy-attr TSV, the leased gate-local), so
# outside a Cadenza checkout there is nothing legitimate for this shim to do. Unless the CWD is inside a
# Cadenza checkout, exec the REAL nix immediately, unchanged. Marker-based (symlink-safe — git resolves the
# real toplevel), NOT a hardcoded path: a Cadenza checkout/worktree uniquely has BOTH spec/semantics and
# fleet/loops. FAIL-OPEN: not a git repo, or the marker absent → real nix, no interception.
_top="$(git rev-parse --show-toplevel 2>/dev/null || true)"
if [ -z "$_top" ] || [ ! -d "$_top/spec/semantics" ] || [ ! -d "$_top/fleet/loops" ]; then run_real "$@"; fi
_hints="$_top/fleet/nix-heavy-attrs.tsv"

# (a) --option substitute false → WARN (any nix cmd; forces a wasteful from-source rebuild).
_p2=""; _p1=""
for a in "$@"; do
  if [ "$_p2" = "--option" ] && [ "$_p1" = "substitute" ] && [ "$a" = "false" ]; then
    if [ -f "$_hints" ]; then
      _m="$(grep -v '^#' "$_hints" 2>/dev/null | awk -F'\t' '$1=="warn:substitute-false"{sub(/^[^\t]*\t/,"");print;exit}')"
      [ -n "$_m" ] && echo "⚠ nix-shim: $_m" >&2
    fi
    break
  fi
  _p2="$_p1"; _p1="$a"
done

# (b) bare `nix build … .#checks.<sys>.<name>` where <name> matches a heavy-attr glob → WARN.
if [ "${1:-}" = "build" ] && [ -f "$_hints" ]; then
  _attr=""
  for a in "$@"; do
    case "$a" in
      *"#checks."*)
        _rest="${a#*#checks.}"   # <sys>.<name>[...]
        _attr="${_rest#*.}"      # strip the <sys>. prefix → <name>
        _attr="${_attr%%.*}"     # keep only the first attr segment
        break ;;
    esac
  done
  if [ -n "$_attr" ]; then
    # ROUTE a BARE (unleased — we are past the CDZ_LEASED_NIX exempt at line 46) local-gate build through the
    # LEASED `cargo xtask fleet gate-local`: same attr, but it takes the weighted check-lease so a raw build
    # can no longer escape the concurrency cap (the 21-concurrent storm root). Only `local-gate`; other heavy
    # attrs fall through to the WARN below (no gate-local equivalent). FAIL-OPEN: if cargo is unavailable, do
    # NOT route — fall through to the warn + real nix. `cargo xtask fleet …` is control-plane (cargo-shim
    # exempts it) and gate-local's inner `nix build` sets CDZ_LEASED_NIX=1 → exempt here → no recursion.
    if [ "$_attr" = "local-gate" ] && command -v cargo >/dev/null 2>&1; then
      echo "⚠ nix-shim: routing a bare 'nix build .#checks.*.local-gate' → 'cargo xtask fleet gate-local' (the LEASED authoritative gate — a raw build ESCAPES the check-lease + fed the gate-local storm; bypass: CDZ_NO_NIX_SHIM=1)." >&2
      exec cargo xtask fleet gate-local
    fi
    _hit=0
    while IFS="$(printf '\t')" read -r _k _v; do
      [ "$_k" = "heavy-attr" ] || continue
      # shellcheck disable=SC2254  # $_v is a deliberate glob pattern from the v-nix-owned TSV
      case "$_attr" in $_v) _hit=1; break ;; esac
    done < <(grep -v '^#' "$_hints" 2>/dev/null)
    if [ "$_hit" = 1 ]; then
      # ROUTE any OTHER bare heavy check attr (corpus-* / gate-check-* / … — everything but local-gate,
      # which has its own leased command above) through `cargo xtask fleet with-lease nix build …` so it
      # takes a check-lease slot instead of ESCAPING the cap. This closes the raw-heavy-check bypass that
      # starved leased gate-locals (concierge 2026-08-30: ~21 concurrent heavy .#checks builds fleet-wide,
      # several raw multi-target `nix build .#checks.*.corpus-*` that the shim previously only WARNED on then
      # ran UNLEASED). with-lease sets CDZ_LEASED_NIX=1 before exec-ing nix, so this shim passes the inner
      # build straight through (the re-entry guard above → no recursion, no double-route). trailing_var_arg
      # captures `nix build .#… <flags>` verbatim. FAIL-OPEN: if cargo is unavailable we cannot route → fall
      # back to the WARN + real nix (never block). The intentionally-UNLEASED warm-keep pass sets
      # CDZ_LEASED_NIX=1 itself → it is exempt at the top and never reaches here.
      if command -v cargo >/dev/null 2>&1; then
        echo "⚠ nix-shim: routing a bare heavy 'nix build .#checks.*.$_attr' → 'cargo xtask fleet with-lease nix build …' (a raw heavy check ESCAPES the check-lease cap + starves leased gate-locals; bypass: CDZ_NO_NIX_SHIM=1)." >&2
        exec cargo xtask fleet with-lease nix "$@"
      fi
      _m="$(grep -v '^#' "$_hints" 2>/dev/null | awk -F'\t' '$1=="warn:heavy"{sub(/^[^\t]*\t/,"");print;exit}')"
      [ -n "$_m" ] && echo "⚠ nix-shim (heavy check '$_attr' built bare, escaping the lease; cargo absent, cannot route): $_m" >&2
    fi
  fi
fi

run_real "$@"
