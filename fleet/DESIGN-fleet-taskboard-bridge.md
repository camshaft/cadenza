# DESIGN: fleet ↔ task-board bridge (auto-register agents + track progress on tasks)

Status: **scoping (P0)** — operator-directed 2026-09-27 (Slack seq-1294, routed by concierge as an `assign`).
Owner: `v-fleet-tooling` (owns `fleet.rs` + the registry + `fleet add`/`fleet up`). Nothing is built until the
Open Questions below get an operator steer (the assignment says "SCOPE it first, then build incrementally,
coordinate design questions through the concierge").

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

## ⚠ Pivotal scoping finding: the task-board MCP is NOT in an agent's session

The board's MCP tools the assignment names — `register_agent`, `list_agents`, `create_project`,
`create_task`, `list_tasks`, `get_task`, `comment_task`, `update_task`, `set_status`, `set_task_props`,
`send_message`, `subscribe`, `get_events`, `check_notifications` — are **not available in this
(`v-fleet-tooling`) agent session**. VERIFIED: two `ToolSearch` passes of the deferred-tool registry with the
exact distinctive names return only Amazon Taskei/SIM + pipeline-assistant tools, none of the board tools.
They appear to be available only in the **concierge's** session (the assignment says "available in this
session" — i.e. the concierge's, where it was routed).

This SHAPES the whole architecture: **agents cannot self-register or self-comment via the MCP directly.** So
the bridge needs a PROXY path. Three candidate architectures (the #1 Open Question):

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

Recommendation pending the interface answer: if the board has an HTTP/CLI interface `fleet.rs` can call → **(A)**
(cleanest, agents self-serve, extraction-compatible). If the ONLY interface is the MCP tools inside a Claude
session → **(B)** first (concierge mirrors), because it needs no new transport and ships fastest, with **(C)**
as the scale valve if concierge load becomes the bottleneck.

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
   `127.0.0.1:8880/board/mcp`, reachable by any host process — so `fleet.rs` can be a direct client (option A
   is mechanically viable; see "Verified transport" above).** The only residual is the A/B/C **CHOICE**: build
   the direct `fleet.rs` client now (A), have the concierge mirror via its MCP session first (B-interim), or
   stand up a dedicated bridge daemon (C)? Concierge recommended **A-target / B-interim** to the operator.
2. **Coexist vs migrate:** tracking-mirror ON TOP of the inbox (recommended), or is the board meant to
   eventually REPLACE the message-bus transport?
3. **Granularity:** Project-per-vertical + Task-per-unit-of-work (proposed), or a different grain (e.g.
   Project-per-subsystem)?
4. **Priority:** proceed P0→P1 (auto-register) now and reassess, given the perf-push is the standing focus?

On answers → turn P1 into a concrete slice and build. Until then this doc is the scoping capture; nothing is
built (the assignment's "scope first" step).
