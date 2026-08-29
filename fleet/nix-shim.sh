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
# SAFETY — this shadows `nix` for the WHOLE fleet (the cargo-shim's `nix run .#test/.#build` routes, gate-
# local's own `nix build`, refresh-tools, every `apps` invocation flow through it), so a bug is a fleet-wide
# wedge. Therefore it is WARN-ONLY + FAIL-OPEN + heavily guarded:
#   • The DEFAULT for EVERY invocation is to exec the REAL nix unchanged. The shim only ever adds a stderr
#     WARNING; it NEVER blocks, routes, or alters the nix command.
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

_top="$(git rev-parse --show-toplevel 2>/dev/null || echo .)"
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
    _hit=0
    while IFS="$(printf '\t')" read -r _k _v; do
      [ "$_k" = "heavy-attr" ] || continue
      # shellcheck disable=SC2254  # $_v is a deliberate glob pattern from the v-nix-owned TSV
      case "$_attr" in $_v) _hit=1; break ;; esac
    done < <(grep -v '^#' "$_hints" 2>/dev/null)
    if [ "$_hit" = 1 ]; then
      _m="$(grep -v '^#' "$_hints" 2>/dev/null | awk -F'\t' '$1=="warn:heavy"{sub(/^[^\t]*\t/,"");print;exit}')"
      [ -n "$_m" ] && echo "⚠ nix-shim (heavy check '$_attr' built bare, escaping the lease): $_m" >&2
    fi
  fi
fi

run_real "$@"
