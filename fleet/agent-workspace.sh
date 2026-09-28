#!/usr/bin/env bash
# fleet/agent-workspace.sh — materialize a per-agent workspace from a SHARED bare-mirror repo store.
#
# The generic, repo-agnostic workspace primitive (fleet/DESIGN-fleet-per-agent-workspaces.md): the agent
# is the unit; each agent has its own directory holding one git worktree PER REPO it works in, and every
# worktree of a given repo shares ONE bare mirror (one object store per repo, cheap worktrees). This lets
# an agent work across MANY repos and lets N agents share a repo's objects — the north star's substrate.
#
# This is a STANDALONE, ADDITIVE primitive: it is NOT wired into `fleet add` / `fleet up` yet, so it does
# not touch the live cadenza-worktree model. It is the building block those will call once the generic
# model is adopted (operator-gated). Root defaults to ~/.fleet (operator ruling) — override with FLEET_ROOT.
#
# Usage:
#   agent-workspace.sh ensure <repo-url-or-path> <agent> <branch> [--apply]   # ensure mirror + worktree
#   agent-workspace.sh list [<agent>]                                          # show materialized worktrees
# Env: FLEET_ROOT (default $HOME/.fleet). Dry-run is the default; --apply performs the git operations.
set -euo pipefail

FLEET_ROOT="${FLEET_ROOT:-$HOME/.fleet}"
MIRRORS="$FLEET_ROOT/mirrors"
AGENTS="$FLEET_ROOT/agents"

repo_name() {  # strip trailing slash + .git, take basename
  local u="${1%/}"; u="${u%.git}"; basename "$u"
}

mirror_default_base() {  # the remote-tracking ref to branch a new agent worktree FROM (never a local head)
  local mir="$1" b
  b="$(git -C "$mir" symbolic-ref --short refs/remotes/origin/HEAD 2>/dev/null || true)"  # e.g. origin/main
  if [ -z "$b" ]; then
    for cand in origin/main origin/master; do
      git -C "$mir" show-ref --verify --quiet "refs/remotes/$cand" && { b="$cand"; break; }
    done
  fi
  printf '%s' "$b"
}

cmd_ensure() {
  local url="$1" agent="$2" branch="$3" apply="${4:-}"
  local name mir wt
  name="$(repo_name "$url")"
  mir="$MIRRORS/$name.git"
  wt="$AGENTS/$agent/$name"

  # 1) shared bare mirror (one per repo). Upstream branches land under refs/remotes/origin/*; agent
  #    worktree branches live under refs/heads/*. So `fetch --prune` only prunes remote-tracking refs and
  #    NEVER an agent's branch (a --mirror clone's +refs/*:refs/* refspec would prune a peer's branch —
  #    that bug detached a live worktree). init+remote+fetch pins the safe refspec deterministically.
  if [ -d "$mir" ]; then
    echo "mirror: $mir exists — would fetch origin --prune (remote-tracking only)"
    if [ "$apply" = "--apply" ]; then
      git -C "$mir" fetch origin --prune --quiet
      git -C "$mir" remote set-head origin -a >/dev/null 2>&1 || true
    fi
  else
    echo "mirror: MISSING — would init bare + fetch origin '$url' -> $mir"
    if [ "$apply" = "--apply" ]; then
      mkdir -p "$MIRRORS"
      git init --quiet --bare "$mir"
      git -C "$mir" remote add origin "$url"   # default refspec: +refs/heads/*:refs/remotes/origin/*
      git -C "$mir" fetch origin --prune --quiet
      git -C "$mir" remote set-head origin -a >/dev/null 2>&1 || true
    fi
  fi

  # 2) per-(agent,repo) worktree off the shared mirror
  if [ -d "$wt" ]; then
    echo "worktree: $wt exists — leaving as-is (idempotent)"
  else
    echo "worktree: would add $wt on branch '$branch' (off $name.git)"
    if [ "$apply" = "--apply" ]; then
      mkdir -p "$AGENTS/$agent"
      if git -C "$mir" show-ref --verify --quiet "refs/heads/$branch"; then
        git -C "$mir" worktree add --quiet "$wt" "$branch"   # resume an existing agent branch
      else
        local base; base="$(mirror_default_base "$mir")"
        [ -n "$base" ] || { echo "ERROR: no remote-tracking base in $mir to create '$branch' from" >&2; exit 1; }
        git -C "$mir" worktree add --quiet -b "$branch" "$wt" "$base"  # new agent branch off origin base
      fi
      echo "worktree: created $wt (shares object store $mir)"
    fi
  fi
}

cmd_list() {
  local only="${1:-}"
  [ -d "$AGENTS" ] || { echo "(no agents materialized under $AGENTS)"; return; }
  for adir in "$AGENTS"/*/; do
    [ -d "$adir" ] || continue
    local a; a="$(basename "$adir")"
    [ -n "$only" ] && [ "$a" != "$only" ] && continue
    for rdir in "$adir"*/; do
      [ -d "$rdir" ] || continue
      printf '%s\t%s\n' "$a" "$(basename "$rdir")"
    done
  done
}

main() {
  local sub="${1:-}"; shift || true
  case "$sub" in
    ensure) [ "$#" -ge 3 ] || { echo "usage: agent-workspace.sh ensure <repo> <agent> <branch> [--apply]" >&2; exit 2; }
            cmd_ensure "$@" ;;
    list)   cmd_list "${1:-}" ;;
    *)      echo "usage: agent-workspace.sh {ensure <repo> <agent> <branch> [--apply] | list [<agent>]}" >&2; exit 2 ;;
  esac
}
main "$@"
