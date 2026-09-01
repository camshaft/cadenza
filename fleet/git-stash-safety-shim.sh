#!/usr/bin/env bash
# git-stash-safety-shim — installed as ~/.local/bin/git (BEFORE the real git on the agent snapshot PATH), so
# it intercepts the fleet's `git` invocations for ONE narrow purpose: refuse an UNSAFE stash on the SHARED
# stash stack.
#
# WHY (v-fleet-tooling 2026-08-30, concierge-nodded): git worktrees SHARE one stash stack across ALL worktrees
# + the main checkout. So a bare `git stash pop` in one worktree applies whatever is on TOP of the shared
# stack — INCLUDING another agent's WIP — into this tree (observed: foreign uncommitted changes leaked into
# v-rust-backend's worktree, a stale c1-diagnostic diff that deleted a since-landed doc). The fleet contract
# already forbids bare stash (use a WIP commit, or `git stash push -u -m TAG` + recover via `git stash apply
# <sha>` + drop-by-tag), but git has NO stash hook to enforce it — so this shim is the enforcement point.
#
# SCOPE — MINIMAL (concierge: "intercept ONLY bare-stash-without-safe-flags"). It refuses exactly two shapes:
#   • `git stash pop [...]`            — pop is inherently top-of-(shared)-stack + drops on apply; steer to
#                                        `git stash apply <sha>` (explicit, non-destructive) instead.
#   • `git stash` / `git stash push`   WITHOUT a `-m`/`--message` tag — an untagged stash on the shared stack
#                                        can't be recovered-by-tag and invites an accidental cross-worktree pop.
# EVERYTHING ELSE is real git, unchanged: every non-stash command (commit/reset/rev-parse/…), and the SAFE
# stash forms (`push -m TAG`, `apply <sha>`, `drop`, `list`, `show`, `clear`, `branch`, …).
#
# SAFETY — this shadows `git` for the WHOLE fleet (fleet sync, commits, hooks all shell out to git), so a bug
# here is a fleet-wide wedge. Therefore, in strict priority order:
#   • FAIL-OPEN: the DEFAULT for every invocation is to exec the REAL git unchanged. Only a POSITIVELY
#     identified unsafe-stash shape is refused; ANY parse ambiguity falls through to real git.
#   • ROBUST NON-RECURSION: the real git is the first `git` on PATH that is NOT this shim; if none is found it
#     falls back to /usr/bin/git (a fleet with no git is catastrophic — never refuse to exec git itself).
#   • KILL-SWITCH: `CDZ_NO_GIT_SHIM=1` bypasses the shim entirely (exec real git) — emergency escape + the
#     documented way to deliberately run a bare pop when you understand the shared-stack risk.
#   • FLEET TOOLING NEVER STASHES (verified: no `git stash` in xtask/fleet), so this can't break sync/commits.
set -uo pipefail

# Resolve the REAL git: first `git` on PATH that is not THIS shim; fall back to /usr/bin/git.
_self="$(command -v -- "$0" 2>/dev/null || true)"
[ -n "$_self" ] || _self="$HOME/.local/bin/git"
_real=""
_oldifs="$IFS"; IFS=:
for _d in $PATH; do
  if [ -n "$_d" ] && [ -x "$_d/git" ] && [ "$_d/git" != "$_self" ]; then _real="$_d/git"; break; fi
done
IFS="$_oldifs"
[ -n "$_real" ] || _real="/usr/bin/git"

run_real() { exec "$_real" "$@"; }

# Emergency bypass — kill-switch execs real git unchanged.
[ -n "${CDZ_NO_GIT_SHIM:-}" ] && run_real "$@"

# Find the git SUBCOMMAND by skipping leading global options. Value-taking globals (-C, -c, --git-dir,
# --work-tree, --namespace) consume the next token too. ANY ambiguity → we simply won't match "stash" and
# fall through to real git (fail-open). This loop only DECIDES whether to inspect a stash; it never mutates argv.
_sub=""
_i=1
while [ "$_i" -le "$#" ]; do
  _a="${!_i}"
  case "$_a" in
    -C|-c|--git-dir|--work-tree|--namespace|--super-prefix)
      _i=$((_i + 2)); continue ;;                 # option + its value
    --*=*|-*)
      _i=$((_i + 1)); continue ;;                 # =-form or value-less global option
    *)
      _sub="$_a"; _subidx="$_i"; break ;;          # first non-option token = the subcommand
  esac
done

# Not a stash command → real git immediately (the 99.9% fast path).
[ "$_sub" = "stash" ] || run_real "$@"

# CWD-SCOPE GUARD (operator 2026-09-01): ~/.local/bin/git shadows `git` for EVERY process of THIS user, so on
# a shared box it would otherwise refuse a `git stash` in OTHER repos that have nothing to do with Cadenza —
# interfering with agents NOT working on Cadenza (operator directive: stop intercepting outside the cadenza
# directory). The shared-stash hazard this shim guards against is REAL only for Cadenza's cross-worktree
# stack; a non-Cadenza repo has its OWN independent stash and a bare `git stash` there is perfectly safe.
# So unless the CWD is inside a Cadenza checkout, exec the REAL git unchanged. Marker-based (symlink-safe —
# git resolves the real toplevel), NOT a hardcoded path: a Cadenza checkout/worktree uniquely has BOTH
# spec/semantics and fleet/loops. Use the RESOLVED real git for detection (calling bare `git` would re-enter
# this shim). FAIL-OPEN: not a git repo, or the marker absent → real git, no refusal. Reached only on the
# rare stash path (past the fast path above), so it adds no cost to the 99.9% of non-stash git calls.
_top="$("$_real" rev-parse --show-toplevel 2>/dev/null || true)"
if [ -z "$_top" ] || [ ! -d "$_top/spec/semantics" ] || [ ! -d "$_top/fleet/loops" ]; then run_real "$@"; fi

# It's `git … stash …`. The stash OPERATION is the token right after the subcommand (if any).
_opidx=$((_subidx + 1))
_op=""
[ "$_opidx" -le "$#" ] && _op="${!_opidx}"

refuse() {
  echo "git-stash-safety: $1" >&2
  echo "  The stash stack is SHARED across ALL worktrees + the main checkout, so this can apply/expose" >&2
  echo "  ANOTHER agent's WIP in your tree (fleet contamination). Safe path:" >&2
  echo "    • prefer a WIP commit; or" >&2
  echo "    • git stash push -u -m '<unique-tag>'  then  git stash list  →  git stash apply <sha>  (not pop)  →  git stash drop <ref>" >&2
  echo "  Deliberate bypass (you understand the shared-stack risk):  CDZ_NO_GIT_SHIM=1 git $*" >&2
  exit 1
}

case "$_op" in
  pop)
    refuse "refusing 'git stash pop' — pop takes the TOP of the shared stack and drops it on apply." "$@"
    ;;
  ""|push)
    # Bare `git stash` (no op) or `git stash push …`: allowed ONLY with a -m/--message tag (recoverable-by-tag).
    # Match the separate (`-m TAG` / `--message TAG`), attached (`-mTAG`), and =-form (`--message=TAG`) shapes.
    for _a in "$@"; do
      case "$_a" in
        -m*|--message|--message=*) exec "$_real" "$@" ;;  # tagged → safe, run it
      esac
    done
    refuse "refusing a bare/untagged 'git stash' — an untagged stash on the shared stack can't be recovered by tag." "$@"
    ;;
  *)
    # apply / drop / list / show / clear / branch / create / store / … — explicit + safe → real git.
    run_real "$@"
    ;;
esac
