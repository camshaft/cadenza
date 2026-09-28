# DESIGN: per-agent workspaces + a shared bare-mirror repo store (the generic, repo-agnostic fleet)

Status: **operator-confirmed DIRECTION (2026-09-28)** — the unifying end-state. Owner: `v-fleet-tooling`.

**North star (operator, 2026-09-28, verbatim intent):** "over time I want the board to be the source of
truth. I really want to get away from all of the cadenza-specific tooling as much as possible. We need to
be able to work across repos and do more and more complex things." So the target is: the board is the
authoritative registry, the fleet tooling is repo-agnostic, and cadenza is one target repo among many.
Every step here is measured against that — additive first, but the destination is board-authoritative +
cadenza-nonspecific. The BUILD is gated behind the perf-push (same gate as the extraction lift); additive
mirror/observability steps that don't change a load-bearing read path proceed under the operator's GO.

This doc is the UMBRELLA that reconciles the two in-flight fleet designs into one coherent model:
- `DESIGN-fleet-extraction-standalone-multirepo.md` — lift the tooling out of `cadenza-xtask` into a
  standalone `~/.fleet` + a `fleet` binary on PATH (its 2026-09-05 rulings stand: `~/.fleet` root,
  standalone binary, core = messaging/windows/orchestration only, per-repo adapters for gate/build/merge).
- `DESIGN-fleet-board-backed-registered-agents.md` — the task board holds agent identity + charter +
  metadata; tooling reads the board instead of `registry.json`.

**What this doc DECIDES (and supersedes):** the extraction doc proposed *decentralized per-repo
checked-in rosters* (`fleet.toml` in each target repo) as the declared source-of-truth. The operator has
picked instead: **the BOARD is the registry** (declared identity/charter/metadata, including which repos
an agent works in). Per-repo checked-in rosters are NOT the authoring surface. The hub (`~/.fleet`) keeps
RUNTIME state (windows/heartbeats/inboxes/leases); the board keeps DECLARED state; `fleet up` reconciles
board-declared → running.

## The model (operator's words: "a directory per agent and then they work in there with each of the repos")

The unit of isolation is the **agent**, not "a cadenza worktree." Cadenza becomes just one target repo
among many.

```
~/.fleet/                          # the fleet root (host-global; extraction ruling, 2026-09-05)
  bin/fleet                        # the standalone fleet binary on PATH
  hub/                             # runtime state: registry-runtime, inboxes, leases, crons, slack-bridge
  mirrors/                         # the CENTRAL shared bare-mirror repo store — one bare clone per repo
    cadenza.git/                   #   bare mirror of camshaft/cadenza
    task-board.git/                #   bare mirror of camshaft/task-board
    capmeshd.git/  …               #   …one per repo any agent works in
  agents/<agent>/                  # the per-agent workspace directory
    cadenza/                       #   a git worktree off ~/.fleet/mirrors/cadenza.git (branch per agent)
    task-board/                    #   a worktree off mirrors/task-board.git (if this agent works there)
    …                              #   one checkout per repo in the agent's declared `repos` list
```

- **Checkout mechanism (operator-confirmed 2026-09-28): git worktrees off a shared bare mirror.** One
  bare mirror per repo under `~/.fleet/mirrors/<repo>.git`; each agent's per-repo checkout is a
  `git worktree add` off that mirror onto the agent's branch. This keeps today's shared-object-store disk
  win (a worktree is cheap; N agents share one object store per repo) generalized to N repos, and it
  dissolves the off-tree hack (capmesh/membrain/task-board self-serving `~/Projects/<repo>.<agent>`
  worktrees) — those become ordinary `agents/<agent>/<repo>` worktrees off a mirror.
- **The board declares which repos an agent works in.** The agent metadata bag (board task #82) carries a
  **`repos` LIST** — each entry `{repo, branch}`. An agent may span several repos. `fleet up` reads that
  list and materializes a worktree per entry under the agent's directory.
- **Charter comes from the board** (verified sessionless fetch, see the board-backed doc), with a
  **checked-in seed fallback**: if the board is unreachable at boot, fall back to the tracked
  `loops/<role>.md` seed so an unreachable board never bricks a boot. Board charter is an
  override/optimization layered over a seed that always exists.

## Bootstrap exceptions (the chicken-and-egg roots)

- **`v-task-board` sources its own charter from a SEED, not the board.** It owns and keeps the board
  alive, so it must not depend on the board being up to know its job — on a cold start (board down) is
  exactly when it must act. It may PUBLISH/refresh its charter onto the board for others to see, but it
  READS its own from a seed. General rule: an agent never sources its liveness dependency from the thing
  it is responsible for keeping alive.
- **Pilot charter-from-board on a NON-owner agent.** Any ordinary vertical can safely treat "board is up"
  as a normal runtime dependency at boot; that proves the mechanism without the circularity.
- **The `fleet` binary + `~/.fleet/hub` must exist before any agent** (they are the substrate). They are
  provisioned by the host/extraction bootstrap, not by an agent.

## What each doc owns after this unification

- **This doc:** the workspace/repo-checkout model (per-agent dir, shared bare-mirror store, worktree per
  (agent, repo)) + the registry-model decision (board over per-repo rosters).
- **Extraction doc:** the mechanics of lifting core out of cadenza into `~/.fleet` + the `fleet` binary +
  per-repo ADAPTERS for gate/build/merge (a repo's gate logic never lives in fleet core). Its
  "decentralized per-repo checked-in rosters" section is SUPERSEDED by board-as-registry here.
- **Board-backed doc:** the board registry surface — agent metadata bag + `update_agent` (task #82),
  charter storage, the verified sessionless fetch, and the source-of-truth flip phasing.

## `fleet up` in the new model (reconcile board-declared → running)

For each agent the board declares (identity + charter + metadata incl. `repos`):
1. Ensure `~/.fleet/agents/<agent>/` exists.
2. For each `{repo, branch}` in the agent's `repos`: ensure `~/.fleet/mirrors/<repo>.git` (clone/fetch the
   bare mirror if absent), then ensure a worktree at `agents/<agent>/<repo>` on `branch`.
3. Ensure the agent's inbox + heartbeat + tmux window; launch `window.sh` cd'd to the agent's directory
   (NOT a cadenza worktree). KICKOFF fetches the board charter (seed fallback).
4. Ensure the agent's cron per its declared interval.
Runtime state (heartbeats/leases/windows) stays hub-central; the board is not a runtime store.

## Phasing (additive-first, the source-of-truth flip LAST, BUILD gated on perf-push)

- **Now (design) — DONE:** this doc + board task #82 (metadata bag + `update_agent` + `get_agent`,
  shipped + deployed) + the verified sessionless charter fetch. Non-disruptive.
- **P-mirror (STARTED 2026-09-28):** `fleet/board-mirror-metadata.py` mirrors every ACTIVE agent's
  registry metadata bag onto the board (idempotent upsert; `registry.json` stays authoritative). First
  run mirrored all 35 active agents. `repos` deliberately not yet set (see `metadata_bag`). STILL TODO:
  stand up `~/.fleet/mirrors/` + the per-agent-directory materializer ALONGSIDE the current
  cadenza-worktree model; migrate one non-owner pilot agent to the new workspace shape; decide
  mirror-wiring (cron/`fleet up`) vs prompt-driven per-agent self-register.
- **P-lift:** the extraction lift (`~/.fleet` + `fleet` binary), per-repo adapters, slack-bridge move.
- **P-flip (high-risk, gated):** tooling reads the board as the registry; `registry.json` becomes a
  derived cache or is retired. Only after the pilot + mirror are proven AND explicit operator confirm.

Nothing is built until the perf-push gate lifts and the operator GOes; this captures the target so the
board-side (task #82 `repos` list) and the fleet-side land coherently rather than as throwaway.
