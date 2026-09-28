# DESIGN: board-backed registered agents (charter + metadata from the task board, not registry.json)

Status: **SCOPING (2026-09-28)** — operator-directed via concierge (assign seq under #10003 tick).
Owner: `v-fleet-tooling` (sole owner of `fleet.rs` + `window.sh` + `fleet/loops/*.md` + `registry.json`).
Nothing is built yet; this is the plumbing assessment + phased plan + the decisions that gate it.
Sibling designs this INTERSECTS: `DESIGN-fleet-taskboard-bridge.md` (auto-register + tracking mirror)
and `DESIGN-fleet-extraction-standalone-multirepo.md` (decentralized per-repo rosters) — see
"Intersection + the competing source-of-truth models" below; this doc must be reconciled with both.

## Operator directive (verbatim, via concierge)

> "can you spin up a new agent v-task-board? and it should have access to the task-board mcp server and
> it should be able to fetch its charter from there instead of the fleet. oh that's another thing. i want
> to move all of the fleet tooling to use registered agents. so we should put all of their metadata in
> there along with their charters. and then we should use that instead of the cadenza json stuff."

Two parts:
1. **PILOT (immediate):** mint a new agent `v-task-board` that (a) has task-board MCP access in its
   window and (b) fetches its charter FROM the board instead of a `fleet/loops/<role>.md` file.
2. **MIGRATION (broader):** move ALL fleet agents to board-registered agents whose metadata + charters
   live on the board; the tooling reads the board instead of `registry.json` — a SOURCE-OF-TRUTH shift.

## Plumbing assessment (grounded in the current code, 2026-09-28)

What exists today, and therefore what is genuinely net-new:

- **Charter source = a repo file.** `window.sh`'s KICKOFF (≈L146-148) instructs each agent to read
  `$SRC/loops/$ROLE.md` from its OWN worktree's materialized `fleet/loops/`; `fleet add` (fleet.rs, the
  `add` fn) VALIDATES `loops/<role>.md` exists in the tracked source before creating the agent. There is
  NO board-fetch path. **Net-new:** a "charter-from-board" source — a KICKOFF variant that tells the
  agent to fetch its charter via its in-session board MCP, plus a place on the board to store it.
- **MCP access = the GLOBAL `~/.claude.json`, not per-window.** `window.sh` launches plain `claude` with
  `CLAUDE_ARGS = --disallowedTools AskUserQuestion + --effort/--model/--autocompact/--dangerously-skip-permissions`
  and NO MCP-scoping flag. `task-board` is already in the global config, so ANY freshly launched agent
  inherits it (this is exactly what the taskboard-bridge rollout relies on). **Consequence:** the pilot
  does NOT need special MCP-access wiring — `v-task-board` gets the board MCP for free once launched.
  Per-agent MCP SCOPING (only `v-task-board` sees the board, others don't) is a SEPARATE, optional gap
  (`--mcp-config`/`--strict-mcp-config` per window) — not required for the pilot.
- **Agent metadata = `registry.json`.** The `Agent` struct carries name/role/vertical/area/worktree/
  branch/interval/model/effort/status/disallow_ask — no charter, no mcp, no repo field. Runtime state
  (heartbeats, inboxes, leases, windows) is hub-central and inherently NOT board-native.
- **The board now HAS a charter slot (VERIFIED live 2026-09-28).** `register_agent(agent_id, charter,
  display_name, kind, webhook_url)` gained a **`charter`** field, and `list_agents` returns it on each
  agent record. So charter-from-board is NATIVELY supported — no convention or new capability needed for
  the pilot. See the "VERIFIED live board facts" section below for the full 20-tool surface + the agent
  record shape.

**Chicken-and-egg (concierge correctly did NOT blind-spin the pilot):** `v-task-board` as specified
needs the charter-from-board fetch path to exist FIRST — spinning it with a stale `fleet/loops/*.md`
charter would defeat its whole purpose. So the pilot is gated on the charter-from-board plumbing, not on
MCP access (which it inherits).

## VERIFIED live board facts (v-fleet-tooling HTTP probe, 2026-09-28)

Probed `POST http://127.0.0.1:8880/board/mcp` read-only (handshake → `tools/list` → `list_agents`).
Server `rmcp` 3.5.0. **20 tools** (was 18 on 2026-09-27 — the board is actively growing):
- Agents: `register_agent(agent_id*, charter, display_name, kind, webhook_url)` · `set_status(agent_id*,
  status*, status_message)` · `list_agents()`.
- Projects: `create_project(name*, description, created_by, metadata)` · `update_project(project_id*,
  name, description, status, metadata, actor)` · `get_project` · `list_projects(status)`.
- Tasks: `create_task(project_id*, title*, description, assignee, priority, created_by, metadata)` ·
  `update_task(task_id*, title, description, assignee, priority, status, metadata, actor)` ·
  `set_task_props(task_id*, props*)` · `move_task(task_id*, to_project_id*, actor)` · `get_task` ·
  `list_tasks(project_id, assignee, status)` · `comment_task(task_id*, body*, author)`.
- Bus/notify: `send_message(from_agent*, to_agent*, body*)` · `get_messages(agent_id*, limit, mark_read)`
  · `check_notifications(agent_id*, limit, mark_read)` · `get_events(limit, since_seq)` ·
  `subscribe`/`unsubscribe`.
- **REST API also exists** at `/api` (per the v-task-board charter on the board; a fleet-side reader may
  prefer it over the MCP SSE + session-id handshake — CONFIRM the `/api` agent-read surface next).

**Agent record shape** (`list_agents`): `{ id, charter, display_name, kind, status, status_message,
webhook_url, created_at, last_seen }`. **KEY GAP for using the agent list AS the registry:** there is
**NO arbitrary `metadata` bag on agents and NO `update_agent` tool** — projects/tasks have `metadata` +
`update_*`, but agents only get `register_agent` (create, incl. charter) + `set_status`. So the fleet
metadata the registry holds (role/model/effort/interval/worktree/branch/area) has no native home on an
agent record. To make the board's agent list the registry, the BOARD needs an agent `metadata` column +
an `update_agent` tool (or those fields folded into `kind`/`charter`) — and since `v-task-board` OWNS the
board repo, that is cleanly a v-task-board board-side feature. Division of labor: **v-task-board builds
the board-side agent-metadata schema; v-fleet-tooling builds the fleet-side reader.**

**`v-task-board` already exists on the board** with a full charter — and it is an OFF-TREE agent (owns
`github.com/camshaft/task-board`, works in `~/Projects/camshaft/task-board`, self-merges, NOT a cadenza
vertical — the board binary exposes MCP `/mcp` + REST `/api` + a React UI over SQLite). So the "spin up
v-task-board" pilot is really an OFF-TREE repo agent (capmesh-class, see the extraction design's off-tree
section) whose charter is board-sourced — reinforcing that this work must reconcile with the off-tree /
decentralized-roster model, not just cadenza's central registry.

## Does it warrant a design pass first? YES.

Part 1 (the pilot) is a small, additive, low-risk slice once the charter-from-board path is drawn. But
Part 2 is a **source-of-truth shift** (board authoritative over `registry.json`) — load-bearing for the
whole fleet's launch/reconcile/watchdog machinery, and it directly collides with the extraction design's
end-state. That MUST be designed before any flip, not grown ad hoc.

## Intersection + the competing source-of-truth models (the key tension)

This directive and the extraction design propose DIFFERENT source-of-truth end-states for the SAME seam:

- **This directive:** the BOARD holds all agent metadata + charters; tooling reads the board.
- **`DESIGN-fleet-extraction-standalone-multirepo.md`:** each TARGET REPO carries a checked-in
  declarative roster (`fleet.toml`), the hub holds runtime state, `fleet up` reconciles declared→running.

These are reconcilable, and the reconciliation is the core design decision: **DECLARED (desired-state)
vs RUNTIME (actual-state) are two different things.** A coherent unified model:
- **Declared/desired-state** = where an agent's IDENTITY + charter + metadata are authored. Candidates:
  the board (this directive) OR checked-in per-repo rosters (extraction). These can COEXIST — the board
  can be the authoring/source-of-truth surface, and a checked-in roster an export/mirror, or vice-versa.
- **Runtime state** = live windows/heartbeats/inboxes/leases — stays hub-central regardless; the board
  is NOT a runtime-state store (it has no lease/heartbeat semantics).

So the cleanest framing: the board REPLACES `registry.json`'s DECLARED/metadata role (names, roles,
charters, models, intervals), while the hub keeps the RUNTIME role. `fleet up` reconciles board-declared
→ running, exactly as it would reconcile a checked-in roster → running. This unifies the two designs
instead of forking them — but it needs the operator to confirm the board (not per-repo checked-in files)
is the authoring surface. (Also overlaps `DESIGN-fleet-taskboard-bridge.md` P1 auto-register: registering
every agent on the board is a shared prerequisite step.)

## Phased plan (proposed — additive-first, the source-of-truth flip LAST)

- **P0 (this doc):** scope + operator confirms (a) the source-of-truth shift and (b) who owns the deeper
  architecture design (see Open decisions). Reconcile with the extraction + bridge designs.
- **P1 — PILOT `v-task-board` (small, additive, low-risk):** draw the charter-from-board fetch path:
  (i) a KICKOFF variant / an agent field (e.g. `charter_source = board:<id>`) that tells the agent to
  fetch its charter via its in-session board MCP at boot instead of reading `loops/<role>.md`; (ii) store
  `v-task-board`'s charter as a board task/project body (convention, no new board capability); (iii) it
  inherits the board MCP from the global config. `registry.json` STILL holds its runtime metadata. Gate:
  `v-task-board` launches, fetches its charter from the board, runs a tick. This proves the mechanism on
  ONE agent with zero risk to the other ~30.
- **P2 — metadata MIRROR (additive):** register every fleet agent on the board with its metadata
  (name/role/model/interval/…). `registry.json` stays AUTHORITATIVE; the board is a mirror the operator
  can read. (This is the taskboard-bridge P1 auto-register — build once, shared.)
- **P3 — charters to the board:** move all role charters onto the board; tooling/KICKOFF reads the
  charter from the board (still reconciling against the tracked `loops/*.md` as a fallback until proven).
- **P4 — the SOURCE-OF-TRUTH FLIP (high-risk, gated):** tooling reads agent metadata from the board
  instead of `registry.json`; `registry.json` becomes a derived cache or is retired. This is the
  load-bearing flip — do it only after P1-P3 are proven AND the operator explicitly confirms, AND it is
  reconciled with the extraction's declared-roster model so we don't build a throwaway.

## Open decisions (operator / concierge steer — flagged, not blocking)

1. **Confirm the source-of-truth shift** (board authoritative over `registry.json`). Load-bearing;
   concierge already surfaced it to the operator for an explicit confirm.
2. **Who owns the deeper architecture design** — an interactive design agent (concierge offered the
   operator one) vs me. Recommendation: I own the PLUMBING (fleet.rs/window.sh/registry) so I own the
   pilot + implementation regardless; if the operator wants a dedicated design agent for the
   source-of-truth architecture debate, it drafts and I implement. Either way the pilot (P1) is mine.
3. **Reconcile with the extraction's decentralized checked-in rosters** — board-as-authoring-surface vs
   per-repo `fleet.toml`. Recommendation: board replaces `registry.json`'s DECLARED role, hub keeps
   RUNTIME, `fleet up` reconciles — unifying the two designs (see the tension section). Needs a pick.
4. **Charter representation on the board** — a convention (a well-known task/project body fetched via the
   agent's in-session MCP; NO new board capability) vs a new board charter capability. Recommend the
   convention first (cheapest, no board changes).
5. **Per-agent MCP scoping** — needed, or is global-config inheritance fine? Recommend global inheritance
   for the pilot (it's already how every agent gets the board MCP); per-agent scoping is a later refinement.

Recommendation: land P0 (this doc) + get the operator's confirm on (1)+(3); the pilot P1 is then a small
additive build once (4)'s charter representation is picked. Do NOT do P4 (the flip) until the pilot +
mirror are proven and the extraction reconciliation is settled. Not urgent — nothing is broken; this is
net-new capability.
