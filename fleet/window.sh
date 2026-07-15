#!/usr/bin/env bash
# window.sh — the per-window entry point for one fleet agent.
#
# This is the ONLY shell in the fleet design. `cargo xtask fleet up/add` creates a tmux window named
# after the agent and runs this script inside it. Everything with real logic (registry, worktrees,
# inbox delivery, tmux window management, merge routing) lives in the xtask; this just launches the
# `claude` session for one agent with the right model, denied tools, and kickoff prompt.
#
# Usage:  window.sh <agent-name>
#
# The runtime copy of this script lives at <hub>/.claude/fleet/window.sh (materialized from the
# tracked fleet/window.sh by `fleet up`). The tracked ROLE BODIES + contract travel with each
# worktree under its own `fleet/` (checked out from trunk), so an agent reads the role body that is
# git-synced with the code it works on.

set -euo pipefail

AGENT="${1:?usage: window.sh <agent-name>}"
# The hub root: this script sits at <hub>/.claude/fleet/window.sh → ../../.. is the hub.
HUB="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

# Resolve the agent's config from the registry. The hub is BARE (no Cargo workspace), so run the
# xtask from any worktree that has one — the pr-sync worktree always exists and holds trunk.
XTASK_WT="$HUB/.claude/worktrees/pr-sync"
CONFIG="$(cd "$XTASK_WT" && cargo xtask fleet describe "$AGENT")" || {
  echo "window.sh: no such agent '$AGENT' in the registry (or pr-sync worktree missing)" >&2
  exit 1
}
eval "$CONFIG"   # sets WORKTREE, ROLE, MODEL, INTERVAL, VERTICAL, AREA, DISALLOW_ASK

: "${WORKTREE:?registry gave no WORKTREE for $AGENT}"
: "${ROLE:?registry gave no ROLE for $AGENT}"
: "${MODEL:=us.anthropic.claude-opus-4-8[1m]}"
: "${EFFORT:=high}"
: "${INTERVAL:=10m}"

if [ ! -d "$WORKTREE" ]; then
  echo "window.sh: worktree $WORKTREE missing — run 'cargo xtask fleet up' to (re)create it" >&2
  exit 1
fi
cd "$WORKTREE"

# The kickoff. Role bodies + contract are read from the agent's OWN worktree tracked `fleet/` (git-
# synced with the code it works on). Runtime state (inbox, queue) is hub-anchored under .claude/fleet.
SRC="$WORKTREE/fleet"
FLEET="$HUB/.claude/fleet"
VNOTE=""
[ -n "${VERTICAL:-}" ] && VNOTE=" Your vertical is '$VERTICAL' in subsystem '${AREA:-rcdzc}'."
KICKOFF="You are the fleet agent named '$AGENT' (role: $ROLE), running UNATTENDED.$VNOTE \
FIRST read $SRC/AGENTS-fleet.md (the fleet contract — inbox protocol, the single-writer/no-CAS land \
model, and the rule that you never wait on the human). THEN read $SRC/loops/$ROLE.md (your role). \
Your worktree is $WORKTREE and your inbox is $FLEET/inbox/$AGENT/. Then run '/loop $INTERVAL' to \
execute one tick of your role per interval. Do the work yourself in this loop; coordinate with peers \
only through 'cargo xtask fleet send'. If you ever need a human decision, send an 'ask' to the \
concierge and keep working — never wait for a reply."

# ── How this session handles APPROVALS ──────────────────────────────────────────────────────────
# A fleet agent loops UNATTENDED, so a tool-permission prompt would stall its window exactly like an
# AskUserQuestion would. The operator EXPLICITLY authorized running these windows with the approval
# system OFF (the machine and repo are trusted; this matches how the prior /loop crons ran). That is
# why `--dangerously-skip-permissions` is set below. Do NOT copy this launcher for an interactive or
# untrusted session — the bypass is scoped to this trusted, unattended fleet on purpose.
CLAUDE_ARGS=(--effort "$EFFORT" --model "$MODEL" --dangerously-skip-permissions)

# Structural guard: every window EXCEPT the interactive roles (concierge, design) is denied the
# human-question tool, so no unattended agent can pop an interactive prompt in its window. The
# describe output sets DISALLOW_ASK=0 for the interactive roles. (Independent of the approval bypass.)
if [ "${DISALLOW_ASK:-1}" = "1" ]; then
  CLAUDE_ARGS+=(--disallowedTools AskUserQuestion)
fi

echo "window.sh: launching '$AGENT' (role=$ROLE model=$MODEL effort=$EFFORT interval=$INTERVAL) in $WORKTREE"
echo "           claude ${CLAUDE_ARGS[*]}"
exec claude "${CLAUDE_ARGS[@]}" "$KICKOFF"
