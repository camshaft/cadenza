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
# The hub root: this script sits at <hub>/.claude/fleet/window.sh, so dirname is <hub>/.claude/fleet
# and ../.. climbs the two levels (fleet → .claude → <hub>) up to the hub.
HUB="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

# Silence cargo's global-registry auto-clean GC for every `cargo xtask …` this window runs. The whole
# fleet shares one ~/.cargo registry, so cargo's periodic GC tries to delete peer-owned cache files
# this uid can't remove and prints a bare `Caused by: Permission denied (os error 13)` — NON-fatal
# (the xtask still runs, `fleet sync` still advances trunk) but a context-less `Caused by:` that a
# peer flagged as something that could mask a REAL error. Disabling the GC removes the noise at the
# source; nothing in the fleet relies on the shared cache being pruned. Exported so it reaches the
# recurring `cargo xtask …` calls the loop makes, not just the one below.
export CARGO_CACHE_AUTO_CLEAN_FREQUENCY=never

# Cap each agent's build fan-out. The whole fleet (~30+ agents) shares ONE box; by default `cargo build`
# and `rustc`'s codegen threads fan out to ALL cores, so a SINGLE agent's release build can spawn ~ncpu
# rustc/codegen jobs and, multiplied across agents building concurrently, oversubscribe the box into a
# load spike that starves pr-sync's merge gate — the wasmtime epoch deadline then interrupt-traps trivial
# gate cases (false REDs) and integration deadlocks (observed 2026-08-15: one agent ran 53 rustc procs,
# loadavg hit 585, pr-sync froze ~5.5h). Bounding CARGO_BUILD_JOBS per agent caps that fan-out at the
# SOURCE, so no single agent can monopolize the cores. `~ncpu/8` (min 2) keeps a single agent's build
# reasonably fast while leaving headroom for peers + the priority merge gate; the check-lease cap
# (CDZ_CHECK_LEASE_MAX) limits how many gate-heavy runs happen at once, and this limits how wide EACH
# one goes — the two compose. Respect an explicit operator override if already set in the environment.
if [ -z "${CARGO_BUILD_JOBS:-}" ]; then
  _ncpu="$(nproc 2>/dev/null || echo 8)"
  _jobs=$(( _ncpu / 8 ))
  [ "$_jobs" -lt 2 ] && _jobs=2
  export CARGO_BUILD_JOBS="$_jobs"
fi

# Concurrent-heavy-check cap in the MATERIALIZED env (concierge 2026-08-29, load-108 acute relief). The
# #5611 default is already 2, but that only takes effect when an agent REBUILDS its xtask binary; exporting
# it here makes the CURRENT binary honor cap-2 at runtime (acquire_check_lease reads the env each call) the
# moment a window (re)launches — no rebuild wait. Every agent agreeing on the same max keeps the shared
# lease consistent. Respect an explicit operator override if already set.
if [ -z "${CDZ_CHECK_LEASE_MAX:-}" ]; then
  export CDZ_CHECK_LEASE_MAX=2
fi

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

# ── ALL-NIX cutover (operator 2026-08-28) — put the nix entrypoint wrappers (cdz/gate/fast-gate/…) on the
# agent's EFFECTIVE PATH so agents use the warm nix closure instead of cold bare-cargo builds. Delegated
# to the shared refresh-tools.sh (also called by the post-merge/post-checkout git hooks + `fleet sync`),
# so the wrapper SET stays in sync with the flake from one place. FAIL-OPEN: it exits 0 on any failure, so
# a launch is never blocked on the all-nix setup. (The cargo-redirect shim is a SEPARATE, policy-gated step.)
bash "$HUB/.claude/fleet/refresh-tools.sh" 2>/dev/null || true

# The kickoff. Role bodies + contract are read from the agent's OWN worktree tracked `fleet/` (git-
# synced with the code it works on). Runtime state (inbox, queue) is hub-anchored under .claude/fleet,
# but the kickoff never hands the agent that raw path — it points at the `cargo xtask fleet inbox`
# resolver instead, so an agent can't glob a worktree-relative shadow dir and silently stall.
SRC="$WORKTREE/fleet"
VNOTE=""
[ -n "${VERTICAL:-}" ] && VNOTE=" Your vertical is '$VERTICAL' in subsystem '${AREA:-rcdzc}'."
# The recurring TICK prompt — passed as the PROMPT ARGUMENT to `/loop`. It MUST be non-empty:
# `/loop <interval>` with NO prompt is treated by the loop skill as an EMPTY prompt and does NOTHING
# (schedules no cron, runs no tick body), which silently breaks a freshly-launched agent — it may stamp
# a heartbeat once but then never drains its inbox or does any work (the fresh-fix-agent cold-start
# stall). So the kickoff runs `/loop <interval> <TICK>` with this explicit tick recipe, guaranteeing
# the loop both SCHEDULES the recurring cron AND runs the role body each fire.
TICK="Run one tick of your role ($ROLE)$VNOTE: (1) 'cargo xtask fleet heartbeat $AGENT' (stop cleanly \
if a stop-file exists); (2) drain your inbox by listing it with 'cargo xtask fleet inbox $AGENT' (the \
RESOLVER — it prints the canonical HUB inbox path; NEVER ls a worktree-relative '.claude/fleet/inbox/...' \
glob, which silently matches an empty shadow dir and stalls you), oldest-first — act on each message, \
then move it to processed/; (3) sync your base with 'cargo xtask fleet sync' (the safe base-sync: \
resets onto trunk + replays only your not-yet-upstream commits by patch-id, so it never orphans a \
queued merge-request's --ref like a bare 'git reset --hard trunk' would), then do ONE well-scoped unit \
of work per $SRC/loops/$ROLE.md and gate it green before sending pr-sync a merge-request. Coordinate \
with peers only via 'cargo xtask fleet send'; if you need a human decision send the concierge an 'ask' \
and keep working — never wait for a reply."

KICKOFF="You are the fleet agent named '$AGENT' (role: $ROLE), running UNATTENDED.$VNOTE \
FIRST read $SRC/AGENTS-fleet.md (the fleet contract — inbox protocol, the single-writer/no-CAS land \
model, and the rule that you never wait on the human). THEN read $SRC/loops/$ROLE.md (your role). \
Your worktree is $WORKTREE. LIST your inbox with 'cargo xtask fleet inbox $AGENT' (the RESOLVER — it \
prints the canonical HUB inbox path; NEVER ls a worktree-relative '.claude/fleet/inbox/...' glob, which \
silently matches an empty shadow dir and stalls you). Then start your recurring loop by running EXACTLY \
this — the interval AND a non-empty tick prompt ('/loop $INTERVAL' with no prompt is a no-op that \
schedules nothing): /loop $INTERVAL $TICK"

# ── How this session handles APPROVALS ──────────────────────────────────────────────────────────
# A fleet agent loops UNATTENDED, so a tool-permission prompt would stall its window exactly like an
# AskUserQuestion would. The operator EXPLICITLY authorized running these windows with the approval
# system OFF (the machine and repo are trusted; this matches how the prior /loop crons ran). That is
# why `--dangerously-skip-permissions` is set below. Do NOT copy this launcher for an interactive or
# untrusted session — the bypass is scoped to this trusted, unattended fleet on purpose.
#
# ⚠ ARG ORDER MATTERS: `--disallowedTools` is a SPACE-SEPARATED VARIADIC flag — if it is the last
# flag before the positional prompt, clap slurps the whole KICKOFF string into it (splitting it into
# bogus "tool names") and the agent gets NO prompt. So the disallow flag goes FIRST (immediately
# followed by another flag that stops its consumption), and the args END with the boolean
# `--dangerously-skip-permissions`, so the final `"$KICKOFF"` lands as the positional prompt.
CLAUDE_ARGS=()
# Structural guard: every window EXCEPT the interactive roles (concierge, design) is denied the
# human-question tool, so no unattended agent can pop an interactive prompt in its window. The
# describe output sets DISALLOW_ASK=0 for the interactive roles. (Independent of the approval bypass.)
if [ "${DISALLOW_ASK:-1}" = "1" ]; then
  CLAUDE_ARGS+=(--disallowedTools AskUserQuestion)
fi
CLAUDE_ARGS+=(--effort "$EFFORT" --model "$MODEL" --dangerously-skip-permissions)

echo "window.sh: launching '$AGENT' (role=$ROLE model=$MODEL effort=$EFFORT interval=$INTERVAL) in $WORKTREE"
echo "           claude ${CLAUDE_ARGS[*]} <kickoff>"
exec claude "${CLAUDE_ARGS[@]}" "$KICKOFF"
