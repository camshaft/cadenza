# DESIGN: fleet ↔ task-board bridge (auto-register agents + track progress on tasks)

Status: **⛔ BLOCKED (2026-09-28)** — operator-directed 2026-09-27 (Slack seq-1294). The operator chose the
prompt-driven **option (D)** (agents self-register via their OWN in-session MCP after a fleet restart), and my
side shipped: the charter task-board section (#9919) + the `fleet restart-all` verb (#9922/#9949/#9976). Then
the board endpoint gained an **OAuth + host-allowlist gate** (verified 2026-09-28, v-fleet-tooling + concierge)
that makes option (D) **infeasible for unattended agents** — see the STATUS UPDATE section. The rollout is HELD
pending an operator fix to the board's auth/allowlist + a possible architecture pivot (D → A or B).
Owner: `v-fleet-tooling` (owns `fleet.rs` + the registry + `fleet add`/`fleet up`).

## STATUS UPDATE 2026-09-28 — ⛔ board gained an OAuth + host-allowlist gate → option (D) infeasible

The operator's option-(D) decision (agents self-register via their own in-session board MCP) was made while the
board endpoint was OPEN. It has since been GATED, which invalidates (D) for an UNATTENDED fleet:

- **Verified (v-fleet-tooling read-only re-probe + concierge in-session + host access):** `GET /health` = 200,
  but `POST /board/mcp` = **403 "Forbidden: Host header is not allowed"** for every Host value
  (127.0.0.1/localhost/green-machine.camshaft.dev). AND concierge's own CONNECTED in-session board client now
  exposes only `authenticate` / `complete_authentication` — `register_agent`/`list_projects`/`create_project`
  report "installed but requires authentication" and want an OAuth flow. So it is NOT a curl-only artifact: the
  full toolset is gated behind OAuth for a connected client too.
- **Implication:** a fresh agent after `fleet restart-all` would hit the same gate and could NOT self-register
  without completing an interactive OAuth flow **per session** — which is **unattended-impossible** for ~30
  headless agents. So option (D) is dead UNLESS the board fix removes the per-session interactive auth (e.g. a
  service token or a shared/long-lived Access session the agents' MCP clients inherit).
- **Architecture consequence (the pivot if OAuth stays interactive):** revert toward the earlier options —
  **(A)** a small `fleet.rs`-direct MCP-JSON-RPC client authenticating with a STORED service/Access token
  (agents self-serve via `fleet task …`, one token provisioned once, no per-session OAuth), or **(B)** the
  concierge as a single board writer using the operator's authenticated session. (A) is still the cleanest IF a
  non-interactive token can be minted for the board; (B) needs no token plumbing but centralizes on concierge.
- **HELD:** neither concierge nor I will run `fleet restart-all --apply` — a full restart would bounce 30
  agents into a broken/auth-gated connect for zero gain. Concierge surfaced the board-auth fix to the operator
  (add 127.0.0.1 to allowed-hosts, and/or provision a non-interactive token, and/or change the config url).
  **Awaiting the operator's board fix + a steer on whether (D) survives or we pivot to (A)/(B).** The board is
  a NON-load-bearing tracking mirror (the inbox stays the transport), so fleet ticks are unaffected meanwhile.

The Open Questions + phased plan below stand, re-scoped by this gate: P1 (get agents onto the board) is blocked
on the auth fix; the coexist model (Q2) and granularity (Q3) are unchanged.

## Operator ask (verbatim, seq-1294)

> "Can we get a bridge between the task board and the comment system we have in cadenza? Basically all of the
> agents in your fleet set up should automatically register themselves as agents on the task board. And we
> should really start using the task system to track progress. That way they can add comments on particular
> issues and we have actual tracking instead of just ad-hoc tracking. Maybe we should get the fleet tooling
> agent to start building out the bridge between what we're doing right now and the capabilities that the task
> system we just added provides."

Three requirements: (1) every fleet agent auto-registers on the task board; (2) progress tracking moves off
ad-hoc (file inbox + the ~7.7MB `backlog.md`) onto tasks + comments + status; (3) a durable BRIDGE between
today's fleet coordination and the task-board capabilities.

## Scoping finding: MCP availability is a session-START-TIMING artifact, not absence (CORRECTED 2026-09-27)

Initial finding (probing THIS session): the board's MCP tools (`register_agent`, `create_task`,
`comment_task`, …) did not appear in the `v-fleet-tooling` session — two `ToolSearch` passes returned only
Amazon Taskei/SIM + pipeline-assistant tools, none of the board tools. **CORRECTION (concierge, host access):**
the tools ARE available to agent sessions — the reason THIS session doesn't see them is **session-start
timing, not absence**. `task-board` lives in the GLOBAL `~/.claude.json`, and `fleet/window.sh` launches plain
`claude` with no MCP-scoping flag (`CLAUDE_ARGS` = `--disallowedTools AskUserQuestion` + effort/model/
autocompact/skip-permissions only), so EVERY session resolves it. But MCP servers load at session START and do
NOT hot-reload; `task-board` was added to the config AFTER this window launched, so a still-running
pre-existing session predates it. A session started AFTER the config change (the concierge's, restarted then)
has the tools.

**Lesson (verify-before-asserting):** the OBSERVATION (tools absent from my running session) was real, but the
CONCLUSION ("not available to agents") over-generalized — the real cause was session-start timing (a session
predating the config change), not absence.

**This OPENS a 4th architecture — (D):** after a `fleet up` relaunch (or any window restart), every agent has
the board MCP tools DIRECTLY in-session, so a live agent can self-register / comment / set-status via its OWN
MCP tools — **no proxy, no Rust client.** But the proxy path still matters for the moments with NO live agent
session: AUTO-register-on-spinup (`fleet add` + `fleet up` reconstruct run OUTSIDE any agent's Claude session)
and non-Claude contexts. So the likely shape is a **HYBRID**: (D) for a live agent's ongoing progress/comments,
plus a small `fleet.rs`-direct client (A) for the register-on-spinup / reconcile moment. The four candidate
paths (the #1 Open Question is now the CHOICE among them + whether to trigger a fleet relaunch):

- **(A) `fleet.rs` calls the board directly.** Add a `fleet task …` subcommand family (register/comment/
  status/create) that talks to the board over whatever transport the MCP WRAPS — an HTTP endpoint, a local
  daemon socket, or a CLI. Every agent already has the `fleet` binary, so this reuses the existing
  self-locating-CLI pattern (same as `fleet send`). **Requires knowing the board's underlying callable
  interface, not just its MCP tool surface** — the MCP tool list does not tell us the transport. NEEDS the
  operator/concierge to say what the board actually is (MCP server backed by what? is there an HTTP/CLI
  fleet.rs can hit without an MCP client?).
- **(B) concierge as the bridge daemon.** Agents keep emitting today's structured fleet messages
  (`fleet send`/registry/landings); the concierge — which HOLDS the MCP session — mirrors them onto the board
  via its MCP tools (single writer to the board). Minimal agent-side change; reuses the whole existing bus.
  Cost: centralizes board I/O + load on the concierge, and the mirror lags the concierge's tick.
- **(C) a dedicated bridge daemon agent** (`v-taskboard-bridge`) that holds the MCP session and mirrors
  registry + inbox + backlog → board. Isolates the load from the concierge; another daemon to run.
- **(D) each agent uses its OWN in-session MCP tools** (available after a relaunch — see the corrected finding
  above): a live agent calls `register_agent`/`comment_task`/`set_status` directly, NO proxy and NO Rust
  client. Simplest for the RUNNING-agent path. Does NOT cover the register-on-spinup / reconcile moment (those
  run before/without a live agent session), so pairs with a small (A) client for that.

Recommendation (now that transport + availability are known): a **HYBRID (D + a thin A)** — (D) for a live
agent's ongoing progress/comments/status via its own session MCP (no new code, once agents relaunch), plus a
thin `fleet.rs`-direct client (A) used ONLY at the sessionless moments: `fleet add`/`fleet up` register +
reconcile. (B)/(C) remain fallbacks if we prefer a single board writer over per-agent self-service. This
needs the operator to (i) pick the shape and (ii) decide whether to trigger a fleet relaunch so agents pick up
the MCP (a rolling `fleet up` relaunch, or wait for natural window cycling).

## Verified transport + API surface (probed read-only 2026-09-27)

Concierge (host + MCP access) reported the transport; I then PROBED it read-only from this agent session to
ground the client design in the real surface (verify-before-building). Confirmed:

- **Endpoint:** `http://127.0.0.1:8880/board/mcp` — a LOCAL HTTP MCP server (`task-board` v1.30.0, protocol
  `2024-11-05`). `/health` → 200. Reachable by ANY host process (not bound to a Claude session), so option (A)
  is mechanically viable. It sits behind Cloudflare Access — an `initialize` response auto-sets a
  `CF_Authorization` cookie (localhost dev bypass); a real client keeps the cookie jar + the session id.
- **Framing:** JSON-RPC over HTTP with **SSE responses** (`event: message\ndata: {json}`) — the client parses
  the `data:` line. A **`Mcp-Session-Id`** response header from `initialize` must be echoed on every
  subsequent call. Handshake: `initialize` → `notifications/initialized` → `tools/call`. No REST API
  (`/openapi.json` is a stub; `/board/api*` 404s) — everything is MCP `tools/call`.
- **Tool surface (18 tools; required(+optional) params), VERIFIED via `tools/list`:**
  - `register_agent(agent_id, +display_name,kind,webhook_url)` · `set_status(agent_id,status)` · `list_agents()`
  - `create_project(name, +description,created_by)` · `list_projects()` · `get_project(project_id)`
  - `create_task(project_id,title, +description,assignee,priority,created_by,metadata)` · `update_task(task_id)`
    · `get_task(task_id)` · `list_tasks()` · `set_task_props(task_id,props)` · `comment_task(task_id,body)`
  - `send_message(from_agent,to_agent,body)` · `get_messages(agent_id)` — **a message bus that PARALLELS the
    fleet inbox** (the natural `fleet send` ↔ board mapping).
  - `subscribe(subscriber)` · `unsubscribe(subscriber)` · `check_notifications(agent_id)` · `get_events()`
- **Board state (read-only snapshot):** projects `#1 MCP smoke test`, `#2 George checks`, `#3 Fleet Setup`
  (created_by `george`). Agents `george` (kind `assistant`), `embedder`/`uploader` (kind `worker`) — each
  carries a `webhook_url` at `127.0.0.1:807x/hook` + `status`(online)/`last_seen`. So the board PUSHES to a
  per-agent webhook.

**Client-design consequence (poll, don't serve):** a fleet agent ticks periodically and runs NO HTTP server,
so instead of registering a `webhook_url` it **polls** `check_notifications(agent_id)` / `get_messages(agent_id)`
on each tick (the tick loop is the poll). Fleet agents register with `kind="worker"`, `agent_id=<fleet name>`,
`display_name=<fleet name>`, and OMIT `webhook_url`. `set_status` on heartbeat keeps `last_seen`/online fresh.

**Option (A) concretized — a minimal MCP-JSON-RPC client in `fleet.rs`:** an `initialize` handshake (cache the
`Mcp-Session-Id` + CF cookie for the process), a `tools/call` helper that POSTs the JSON-RPC + parses the SSE
`data:` line, exposed as a `fleet task <verb>` subcommand family (`register` → `register_agent`, `status` →
`set_status`, `project`/`task`/`comment` → the create/comment tools, `notifications` → `check_notifications`).
No third-party MCP crate needed — it's one endpoint, a handshake, and `tools/call` over `reqwest`/`ureq`.
(Still gated on the operator's A/B/C CHOICE; concierge recommended **A-target / B-interim** to the operator.)

## Concept mapping (proposed)

| Fleet concept (today) | Task-board concept |
|---|---|
| a **vertical / workstream** (`fleet-tooling`, `wit-host-boundary`, a standing quality dim) | a **Project** (long-lived) |
| a **unit of work / queue item / landed slice / MR** | a **Task** in that workstream's Project |
| an agent's **per-tick progress** (landing-log notes, the board "Did:" lines) | **comments** on the task + a **status** transition |
| a fleet **agent** (registry entry) | a registered **agent** on the board (`register_agent` on spin-up) |
| `merge-request` → `merged`/`reject` | task status (InProgress→Done / needs-fix) + a comment |
| `ask` → `answer` | a task comment thread or the board's `send_message` |
| `note` / `coord` | comments on the relevant task |
| `backlog.md` entries | **Tasks** (retiring the 7.7MB file as the tracking surface) |

## Auto-register hook (requirement #1)

- `fleet add <agent>`: after writing the registry entry, ALSO `register_agent` on the board (via the chosen
  proxy path A/B/C). Idempotent — safe to re-call.
- `fleet up` (reconstruct): reconcile board agents against the declared roster — register the missing, flag
  dead-but-registered. This is the SAME reconcile seam as the extraction's `ensure_worktree(target)`, so build
  it extraction-compatible (see `DESIGN-fleet-extraction-standalone-multirepo.md`).

## Coexist vs migrate (Open Question #2)

The inbox + `merge-request` protocol is LOAD-BEARING: the delivery-seq, the `processed/` archive, and the
whole watchdog / drain-stall / check-lease machinery key off it. A big-bang migration of the TRANSPORT onto
tasks is high-risk. The operator's words — "actual tracking instead of ad-hoc" — read as wanting a TRACKING
layer, not necessarily replacing the message bus.

**Recommend COEXIST-first:** the board becomes a TRACKING MIRROR (projects/tasks/comments/status the operator
can watch) while the inbox stays the transport. Migrate selectively afterward (start with `backlog.md` →
Tasks, the clearest ad-hoc→tracked win) once the mirror is proven. Keep the message bus as the substrate.

## Phased plan

- **P0 (this doc):** scope + resolve the Open Questions (esp. the board's callable interface + who holds the
  MCP session).
- **P1 — auto-register:** agents appear on the board on spin-up / `fleet up` reconstruct (smallest visible
  slice), via the chosen proxy. Gate: agents listed on the board match the roster.
- **P2 — tracking mirror:** workstreams → Projects, landings/MRs → Tasks + comments + status (inbox
  unchanged). The operator gets real progress tracking without touching the transport.
- **P3 — migrate `backlog.md`:** ad-hoc backlog → board Tasks; retire the 7.7MB file as the tracking surface.
- **P4 (maybe):** richer notifications / `subscribe`+`get_events` for cross-agent coordination signals.

## Sequencing vs the perf-push + extraction (flagged per the assignment)

Operator-directed NOW → active. It does NOT conflict with the dcQUIC perf-push (different subsystem) but
competes for `v-fleet-tooling` ticks. It INTERSECTS the extraction (both touch the registry + `fleet add`/
`fleet up`), so build the register/reconcile hook extraction-compatible. Recommend proceeding P0→P1 now (scope
+ auto-register), then reassess with the operator after auto-register is visible on the board.

## Open Questions (operator steer, via concierge `ask`)

1. **~~The board's callable interface~~ → ANSWERED (probed 2026-09-27): MCP JSON-RPC over HTTP at
   `127.0.0.1:8880/board/mcp`, reachable by any host process (A viable); AND agents get the MCP tools
   in-session after a relaunch (D viable) — see the corrected finding + "Verified transport" above.** Residual
   for the operator: (i) pick the shape — recommend **HYBRID D + thin A** (agents self-serve via their own
   session MCP for ongoing comments/status; a thin `fleet.rs`-direct client only for the sessionless
   register-on-spinup/reconcile); (ii) decide whether to trigger a **fleet relaunch** so running agents pick
   up the board MCP (rolling `fleet up`, or wait for natural window cycling).
2. **Coexist vs migrate:** tracking-mirror ON TOP of the inbox (recommended), or is the board meant to
   eventually REPLACE the message-bus transport?
3. **Granularity:** Project-per-vertical + Task-per-unit-of-work (proposed), or a different grain (e.g.
   Project-per-subsystem)?
4. **Priority:** proceed P0→P1 (auto-register) now and reassess, given the perf-push is the standing focus?

On answers → turn P1 into a concrete slice and build. Until then this doc is the scoping capture; nothing is
built (the assignment's "scope first" step).
