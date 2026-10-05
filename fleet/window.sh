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

# HOST CONFIG: host-specific values (tool dirs, idea areas, and the like) live in the local, untracked
# `${FLEET_HOST_CONF:-$HOME/.config/fleet/host.conf}`, never in this public tree. Source it with auto-export
# (`set -a`) so the agent and every tool it spawns (e.g. `cargo xtask fleet`) see the same values. It holds
# host paths and names only, no credentials. Absent file = no extras.
_host_conf="${FLEET_HOST_CONF:-${HOME:-}/.config/fleet/host.conf}"
if [ -f "$_host_conf" ]; then
  set -a
  # shellcheck disable=SC1090
  . "$_host_conf"
  set +a
fi

# task_347: GUARANTEE a known-good PATH for the agent process and every shell it spawns. A fleet agent's Bash
# tool-calls intermittently spawned with a stripped PATH (coreutils / git / curl / nix all "command not found",
# recoverable only via absolute /usr/bin/... paths) - a per-invocation tax seen across agents + days
# (corroborated). The tmux window can inherit a minimal/empty PATH from the launching daemon, and a tool shell
# that then fails to source a login profile has no usable PATH. APPENDING the standard system + nix-profile bin
# dirs here (before `exec claude`, so claude and all its child shells inherit it) makes the baseline PATH always
# complete while leaving any existing entries FIRST (a repo-/user-preferred tool still wins); the essentials are
# guaranteed present as a fallback, so a bare `git`/`curl`/`nix`/coreutil always resolves. A dir that does not
# exist on this host is harmless (the shell just skips it).
#
# task_812: ALSO append the host's vendor tool dirs + the mise tool dirs. An MCP server the agent spawns via a
# bare-command wrapper (a vendor launcher, which in turn needs a mise-managed `node`) failed with
# CONNECTION_CLOSED because the launcher/`mise`/`node` were not on the stripped agent PATH -- the same class as
# above. The vendor tool dirs are host-specific, so they come from `FLEET_EXTRA_PATH` (colon-separated, set in
# the host config above); `.local/bin` resolves `mise`; the mise `shims` dir resolves `node`/`npx` to the
# mise-active version. Appended (so a user-preferred tool still wins); absent dirs are skipped harmlessly. (NB:
# the mise shim resolves node to the GLOBAL default version -- a server needing a newer node than that default
# is an operator mise-version decision, not a PATH gap.)
export PATH="${PATH:+$PATH:}/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:${HOME:-}/.nix-profile/bin:/nix/var/nix/profiles/default/bin${FLEET_EXTRA_PATH:+:$FLEET_EXTRA_PATH}:${HOME:-}/.local/bin:${HOME:-}/.local/share/mise/shims"

# task_781 / task_596: put the fleet's shared live bin dir on PATH so a CLI materialized into it
# (paste_create, the `fleet` binary, and any future tool) is callable by THIS agent AND by every
# ALREADY-RUNNING agent WITHOUT a relaunch. The dir sits next to this script (<hub>/.claude/fleet/bin) and
# fleet-materialize populates it live from cadenza origin/main:fleet/bin/; because PATH lookup scans the dir
# per-exec, a newly-dropped CLI appears to a running agent's next Bash-tool shell with no relaunch (the
# operator requirement in task_781). Prepend BEFORE `exec claude` so claude + every child shell inherit it
# (same mechanism as the PATH guard above). CLIs are committed files in fleet/bin/, NOT written inline here,
# so adding a tool is a drop-in with no window.sh edit and no per-agent relaunch.
FLEET_SHARED_BIN="$(cd "$(dirname "${BASH_SOURCE[0]}")" 2>/dev/null && pwd)/bin"
[ -d "$FLEET_SHARED_BIN" ] && export PATH="$FLEET_SHARED_BIN:$PATH"

AGENT="${1:?usage: window.sh <agent-name>}"
# The hub root: this script sits at <hub>/.claude/fleet/window.sh, so dirname is <hub>/.claude/fleet
# and ../.. climbs the two levels (fleet → .claude → <hub>) up to the hub.
HUB="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

# ── cdz-smith TYPE-DIFFERENTIAL oracle activation (v-cdz-smith #9314) ──────────────────────────────
# fuzz-cycle.sh runs its Lean type-differential sweep only when a fresh `oracle-check` is discoverable
# (`CDZ_SMITH_ORACLE_CHECK` or on PATH), else it skips cleanly. The `# fleet:oracle-lean` nightly cron
# (stage-oracle-lean.sh) builds + stages it here; point every window at it when present so the smith
# agent's next relaunch picks it up. Harmless for other roles — only fuzz-cycle consults this var. Respect
# an explicit operator override if already set.
if [ -z "${CDZ_SMITH_ORACLE_CHECK:-}" ] && [ -x "$HUB/.claude/fleet/oracle-lean/bin/oracle-check" ]; then
  export CDZ_SMITH_ORACLE_CHECK="$HUB/.claude/fleet/oracle-lean/bin/oracle-check"
fi

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

# SHARED COMPILE CACHE via sccache (operator seq-250 native-half, v-nix-led interim 2026-08-30). The #1
# fleet CPU sink was the wasmtime/cranelift dep-closure recompiled PER-WORKTREE by the tight-loop
# `cargo build -p cdz-run` (cdz-run → wasmtime → cranelift): measured 23 of 35 worktrees each rebuilt it
# in their OWN isolated target/ (~3.2G deps/worktree, ~70G redundant; ~55769 %CPU-sum in the monitor).
# sccache's cache is user-global (SCCACHE_DIR), so an EXTERNAL dep (cranelift/wasmtime/serde/…) is compiled
# ONCE and cache-HIT by every worktree's native cargo thereafter. Concurrency-safe by design. KEY: we do
# NOT force CARGO_INCREMENTAL=0 — external deps are always non-incremental so sccache caches them regardless,
# while WORKSPACE crates (rcdzc/cdz) stay cargo-incremental (sccache passes incremental compiles through
# uncached) → the expensive deps are shared WITHOUT regressing the top-crate tight-loop hot path. Only
# affects NATIVE cargo (nix builds are hermetic — RUSTC_WRAPPER doesn't enter their sandbox). Fully
# reversible: unset RUSTC_WRAPPER. Respect an explicit override; skip if sccache isn't installed.
if [ -z "${RUSTC_WRAPPER:-}" ] && command -v sccache >/dev/null 2>&1; then
  export RUSTC_WRAPPER=sccache
  export SCCACHE_DIR="${SCCACHE_DIR:-$HOME/.cache/sccache-fleet}"
  export SCCACHE_CACHE_SIZE="${SCCACHE_CACHE_SIZE:-20G}"
fi

# Concurrent-heavy-check cap: NO LONGER PINNED HERE (operator seq-208 2026-08-29). The cap is now a
# LOAD-ADAPTIVE compiled default in `check_lease_max()` — generous (up to ceil 5) when the box has spare
# run-queue capacity so queued agents build concurrently instead of idle-waiting on the lock, tightening to
# the saturation-safe floor 2 as loadavg approaches nproc. Pinning `CDZ_CHECK_LEASE_MAX` here would DISABLE
# that adaptivity (an explicit value wins), so the earlier interim =5 stopgap pin is dropped: windows pick
# up the adaptive default gradually as they relaunch. To force a fixed cap for host tuning / an incident,
# set CDZ_CHECK_LEASE_MAX in the environment before launch (it still wins as an explicit operator override).

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
#
# THROTTLED (v-fleet-tooling 2026-09-11): pass REFRESH_MIN_INTERVAL_SEC so a RELAUNCH within the window
# SKIPS the heavy `nix build .#cdz-shell-wrappers` (the wrappers are already fresh from the last refresh <
# window ago; the shims persist too). The load-bearing reason: when an agent FLAPS (recreated repeatedly
# before its first heartbeat — the concierge under heavy perf-push load, 2026-09-11), an un-throttled
# refresh ran a nix build on EVERY relaunch, piling load onto an already-saturated box and WORSENING the
# flap (a feedback loop). A genuine cold launch (no recent refresh → no stamp / stamp older than the
# window) still refreshes fully, so a new agent always gets current wrappers; only rapid relaunches skip.
# The git hooks + `fleet sync` keep the wrapper set current between launches regardless.
REFRESH_MIN_INTERVAL_SEC="${CDZ_LAUNCH_REFRESH_THROTTLE_SEC:-600}" \
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
TICK="Run one tick of your role ($ROLE)$VNOTE: (1) 'fleet heartbeat' (stop cleanly if a stop-file \
exists), AND — coexisting with the file hub, per the board-backed migration — refresh your BOARD presence \
with the task-board MCP set_status (agent_id '$AGENT') to a one-line note of what this tick did; the board \
MCP tools are in your session, so this is a normal tool call (the board does NOT bind your session, and \
re-registering does NOT rebind it — pass your identity EXPLICITLY on EVERY board call for the whole session, \
compaction-resume included: agent_id on check_notifications/set_status/get_messages/list_tasks, from_agent on \
send_message, author on comment_task, actor on update_task, created_by on create_task; a call that omits it \
fails 'no identity for this session'), BUT it is best-effort: if the board is \
unreachable, note it and continue — NEVER block or stall a tick on the board; (2) drain BOTH inboxes \
oldest-first, acting on each message: your FILE-HUB inbox via 'fleet inbox' (the load-bearing transport + \
RESOLVER — it prints the canonical HUB inbox path; NEVER ls a worktree-relative '.claude/fleet/inbox/...' \
glob, which silently matches an empty shadow dir and stalls you), moving each to processed/ ('fleet inbox \
--processed <msg>'), AND your BOARD inbox via the task-board MCP check_notifications (agent_id '$AGENT'); \
(3) sync your base with 'fleet sync' (the safe base-sync: resets onto trunk + replays only your \
not-yet-upstream commits by patch-id, so it never orphans a queued merge-request's --ref like a bare \
'git reset --hard trunk' would), then do ONE well-scoped unit of work per $SRC/loops/$ROLE.md and gate it \
green before sending pr-sync a merge-request; mirror progress on the board (comment_task / set_status) — \
the board is the tracking mirror, the file hub stays the load-bearing transport. Coordinate with peers via \
'fleet send' (or board send_message for a board-native peer); if you need a human decision send the \
concierge an 'ask' and keep working — never wait for a reply."

KICKOFF="You are the fleet agent named '$AGENT' (role: $ROLE), running UNATTENDED.$VNOTE \
FIRST read $SRC/AGENTS-fleet.md (the fleet contract — inbox protocol, the single-writer/no-CAS land \
model, and the rule that you never wait on the human). THEN read $SRC/loops/$ROLE.md (your role). \
Your worktree is $WORKTREE. Your fleet verbs are first-class commands on your PATH (no 'cargo xtask' \
prefix): 'fleet inbox' / 'fleet heartbeat' / 'fleet sync' / 'fleet send' — and 'fleet inbox'/'heartbeat' \
auto-target THIS agent from your window, so you never pass or mistype your own name. LIST your inbox with \
'fleet inbox' (the RESOLVER — it prints the canonical HUB inbox path; NEVER ls a worktree-relative \
'.claude/fleet/inbox/...' glob, which silently matches an empty shadow dir and stalls you). Then start \
your recurring loop by running EXACTLY \
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
# ── NATIVE AUTO-COMPACT HEADROOM (concierge-wedge fix, operator native-compact directive 2026-09-13) ──
# Claude Code's native auto-compact is ON by default (`autoCompactEnabled: true`) and the launcher does
# NOT disable it — but on the 1M-window model (opus-4-8[1m] on Bedrock) the DEFAULT trigger sits at ~967K
# of the 1M window, only ~3% headroom. Auto-compact fires at a TURN BOUNDARY (before the next model
# request), so a single HEAVY tick — reading a >1MB file, or a 30-window capture-pane sweep = 300K+ tokens
# in ONE tool result — balloons from below 967K straight past the 1M wall WITHIN one turn, before any
# boundary where compaction could fire. Result: the session hits 100% mid-turn and WEDGES (it can't submit,
# and it can't self-`/compact` — that's an interactive command, not a tool). This is the recurring concierge
# outage (the concierge is no-kill/no-auto-restart, so a wedge is fatal → manual resume).
#
# THE NATIVE FIX (operator's preferred path over an external `/compact` keystroke, verified viable against
# claude 2.1.270: `--autocompact <auto|tokens>` is a real flag; `autoCompactWindow`/CLAUDE_CODE_AUTO_COMPACT_WINDOW
# are real config): LOWER the auto-compact window so compaction fires with enough HEADROOM that a single
# heavy turn can't reach the wall. 600K on the 1M model leaves ~400K headroom — more than the biggest
# plausible single-turn tool result — while still a large working context (bigger than the entire 200K
# default window of non-1M models), so agents don't churn-compact. On a smaller-window model the flag caps
# at that window (a no-op there). Non-destructive, native, needs NO watchdog and NO concurrency change.
# Overridable: a per-agent AUTOCOMPACT from `describe`, else the env CDZ_AUTOCOMPACT_WINDOW, else 600K.
: "${AUTOCOMPACT:=${CDZ_AUTOCOMPACT_WINDOW:-600000}}"

# ── Belt-and-suspenders non-interactive override ──────────────────────────────────────────────────
# `describe` derives DISALLOW_ASK from the ROLE. As of operator directive seq-1360 NO role is
# terminal-interactive (`role_is_terminal_interactive` returns false for every role — `design` became
# non-interactive + board-driven, `concierge` is Slack-first), so `describe` sets DISALLOW_ASK=1 for
# EVERY agent and this override is normally redundant. It is kept as a durable structural guard: the hub
# SENTINEL file `<hub>/.claude/fleet/non-interactive` (or env CDZ_FORCE_DISALLOW_ASK=1) forces
# DISALLOW_ASK=1 unconditionally, so even if a future role were made interactive again, a restart can be
# pinned non-interactive fleet-wide (survives relaunches via the hub, unlike a tmux-dropped env).
if [ -e "$HUB/.claude/fleet/non-interactive" ] || [ "${CDZ_FORCE_DISALLOW_ASK:-0}" = "1" ]; then
  DISALLOW_ASK=1
fi

CLAUDE_ARGS=()
# Structural guard: every window is denied the human-question tool (no role is interactive anymore —
# seq-1360 made `design` board-driven; `concierge` is Slack-first), so no unattended agent can pop an
# interactive prompt in its window and block forever. (Independent of the approval bypass.)
if [ "${DISALLOW_ASK:-1}" = "1" ]; then
  CLAUDE_ARGS+=(--disallowedTools AskUserQuestion)
fi
CLAUDE_ARGS+=(--effort "$EFFORT" --model "$MODEL" --autocompact "$AUTOCOMPACT" --dangerously-skip-permissions)

# ── LAUNCH-TIME HEARTBEAT (concierge flap fix, concierge-approved 2026-09-12) ──
# Stamp the agent's heartbeat NOW, at launch, so a session that is SLOW on its FIRST /loop tick — the
# concierge reads its charter + drains a deep inbox + runs its in-tick watchdog before the loop's step-1
# `fleet heartbeat` stamps — is NOT misread as never-heartbeated → "died before heartbeat" / failed
# cold-start by the watchdog + the compact-nudge flap detector (the intermittent concierge boot-window
# false-positive). The /loop's step-1 heartbeat refreshes it every tick after; this is just the initial
# "I launched" stamp. Matches `heartbeat_refresh_liveness` exactly (respect the stop-file, create the dir,
# write "tick\n" — heartbeat_age_secs only reads the MTIME). Raw write, NOT `cargo xtask fleet heartbeat`:
# a cargo call here could trigger a rebuild and ADD boot latency (the opposite of the #8796 throttle) —
# this is instant + fail-open.
if [ ! -e "$HUB/.claude/fleet/stop/$AGENT" ]; then
  mkdir -p "$HUB/.claude/fleet/heartbeat" 2>/dev/null || true
  printf 'tick\n' > "$HUB/.claude/fleet/heartbeat/$AGENT" 2>/dev/null || true
fi

# ── KICKOFF-ENSURE (fresh-session kickoff-submission robustness, concierge-greenlit 2026-09-12, option c) ──
# A freshly launched claude OCCASIONALLY does not process its positional "$KICKOFF" (boots to a bare ❯ →
# /loop never arms → a cold-start stall; the SAME intermittent fresh-session-boot root as the concierge
# flap). SELF-HEAL without changing the common path: spawn a DETACHED job (survives the `exec` below) that
# after a grace checks whether the agent actually TICKED — its heartbeat mtime advanced past the
# launch-time stamp above. Re-submits the kickoff ONLY if BOTH: (1) the heartbeat is STILL frozen at the
# launch stamp (no /loop tick ran — a real first tick stamps its heartbeat at step 1, well within the
# grace), AND (2) the pane shows the IDLE `❯` prompt (a confirmed dropped-kickoff stall, NOT a slow-but-
# working first tick). DOUBLE-GUARDED so it can NEVER disturb a healthy launch (heartbeat advanced OR pane
# working → no-op); worst case on a real stall is a no-op, never harm. Re-submit via the tmux paste-buffer
# (reliable for a large prompt, unlike per-char send-keys) + Enter. Fail-open throughout.
SESSION="$(tmux display-message -p '#S' 2>/dev/null || echo main)"
LAUNCH_HB_MTIME="$(stat -c %Y "$HUB/.claude/fleet/heartbeat/$AGENT" 2>/dev/null || echo 0)"
(
  sleep 90
  cur_mtime="$(stat -c %Y "$HUB/.claude/fleet/heartbeat/$AGENT" 2>/dev/null || echo 0)"
  pane="$(tmux capture-pane -t "$SESSION:$AGENT" -p 2>/dev/null || true)"
  if [ "$cur_mtime" = "$LAUNCH_HB_MTIME" ] && printf '%s\n' "$pane" | grep -qxE ' *❯ *'; then
    # Confirmed dropped-kickoff stall: no tick ran AND the pane is idle. Re-submit the kickoff.
    if tmux set-buffer -b "cdz-kickoff-$AGENT" "$KICKOFF" 2>/dev/null; then
      tmux paste-buffer -d -b "cdz-kickoff-$AGENT" -t "$SESSION:$AGENT" 2>/dev/null || true
      sleep 2
      tmux send-keys -t "$SESSION:$AGENT" Enter 2>/dev/null || true
    fi
  fi
) </dev/null >/dev/null 2>&1 &

# ── OOM PROTECTION for the concierge (operator-approved 2026-09-14, option 2) ──
# The recurring concierge deaths were KERNEL OOM KILLS: the fleet's cgroup oom_kill counter climbs, the box
# has NO swap, and the fleet spikes toward host RAM (~345G/494G) — a SIGKILL'd claude leaves no graceful exit,
# so its tmux window just vanishes ("clean tick then window gone"). The operator approved sparing the
# CONCIERGE specifically: give its process a strongly-negative OOM score (-900) so the kernel picks another
# victim (peers auto-recover via the out-of-band guardian; the concierge is the operator's only channel).
# Lowering oom_score_adj below 0 needs CAP_SYS_RESOURCE, so this uses a NOPASSWD `choom` sudoers grant (the
# operator installs it: `<fleet-user> ALL=(root) NOPASSWD: /usr/bin/choom -n -900 -p *`). oom_score_adj is
# PRESERVED across execve, so setting it on THIS shell's pid ($$) now carries to the claude that replaces it
# below. FAIL-OPEN: if the grant isn't in place yet (sudo/choom errors), log + launch UNPROTECTED — never
# block the concierge on the protection (the guardian still recovers + alerts on an OOM kill regardless).
if [ "$AGENT" = "concierge" ] && command -v choom >/dev/null 2>&1; then
  if sudo -n choom -n -900 -p $$ >/dev/null 2>&1; then
    echo "window.sh: concierge OOM-protected (oom_score_adj=-900 — the kernel will spare it under memory pressure)"
  else
    echo "window.sh: concierge NOT OOM-protected — the NOPASSWD choom grant is missing; launching anyway (guardian still recovers+alerts on an OOM kill). Grant: '${USER:-<fleet-user>} ALL=(root) NOPASSWD: /usr/bin/choom -n -900 -p *'" >&2
  fi
fi

echo "window.sh: launching '$AGENT' (role=$ROLE model=$MODEL effort=$EFFORT interval=$INTERVAL) in $WORKTREE"
echo "           claude ${CLAUDE_ARGS[*]} <kickoff>"
exec claude "${CLAUDE_ARGS[@]}" "$KICKOFF"
