# DESIGN: extract the fleet into a standalone multi-repo orchestrator

Status: **greenlit to design-doc stage** (operator, 2026-09-05); actual repo-creation / migration is
**gated on operator approval of THIS doc + a repo name**. Owner: `v-fleet-tooling`. Nothing is created or
moved until approval.

## Motivation

The fleet is embedded in cadenza but has organically become general-purpose orchestration (messaging,
window management, watchdog, worktree lifecycle). The `dcquic-perf` agent (works in `camshaft/s2n-quic`,
uses the fleet only for comms) proved a fleet agent can drive a NON-cadenza repo end-to-end. The operator
wants to extract the fleet into its OWN repo so it can run parallel work across MANY repos, with cadenza
becoming just one target among many.

## Confirmed operator rulings (2026-09-05)

- **Hub location: `~/.fleet`** — host-global, outside any target repo.
- **A standalone `fleet` binary on PATH** — used by every agent for all comms; no repo checkout needed to
  talk to the hub.
- **No gates/build/corpus in fleet core.** Fleet core = **general messaging + window management +
  orchestration ONLY.** Per-repo gate/build/merge logic lives in that repo's adapter, never in core.
- **The slack-bridge + its tooling MOVE into the fleet repo** (it's general messaging infra).
- **Decentralized per-repo rosters**: each target repo carries its OWN checked-in fleet config declaring
  that repo's persistent agents; the hub holds runtime state; `fleet up` reconciles declared → running.

## Architecture

### Fleet CORE (the standalone repo) — messaging + windows + orchestration only

- Message bus: registry of live agents, `inbox`/`send`/`heartbeat`, delivery-seq, `processed/` archive.
- Window/session management: `window.sh` launcher, tmux session lifecycle, the loop kickoff, DISALLOW_ASK.
- Orchestration: the watchdog (compact-nudge / drain-nudge / reissue-loop / wedge-restart / update-banner),
  `check-leases` (a general concurrency limiter — NOT gate-specific), the git-worktree-per-agent model.
- Host-health crons: cpu-monitor, prune-tmp-inodes, prune-stale-targets, warm-keep, reap-wedged-nix-clients,
  drain-nudge, compact-nudge (all guard host/disk/tmux, not cadenza).
- **Slack-bridge**: the inbound/outbound Slack↔fleet-message daemon + its config (general messaging infra).
- The generic role bodies + the fleet contract (`AGENTS-fleet.md`), with `perf-agent` as the generic
  foreign-repo template.
- `rebase-freshness` advisory, the send/leak guards — already repo-agnostic, stay in core.

### The HUB decouple (the central change)

Today `Fleet::new` resolves the hub via `git --git-common-dir` of the cadenza worktree →
`<cadenza>/.claude/fleet`. Standalone: the hub is `~/.fleet` (or `$FLEET_HUB`), selected by **explicit
config, not git-common-dir**. Every agent's `fleet` binary resolves the SAME hub regardless of which target
repo it works in. The binary already self-locates independent of cwd (proved by the foreign-repo pattern);
we swap the derivation from git-common-dir to explicit hub config. Hub layout unchanged
(`registry.json`, `inbox/`, `queue/`, `check-leases/`, `.delivery-seq`, cron `.last-run` stamps).

### Per-repo ADAPTER + decentralized roster (checked into each target repo)

Each target repo carries a checked-in `fleet.toml` (or `.fleet/config.toml`) declaring:
- **Identity/location**: repo path on the host, remote, base branch to cut agent worktrees from.
- **Gate/build/merge model** (the per-repo adapter, NOT in core): how to gate a change (a command, or
  "none"), how to land (direct push / `gh pr create` + admin-merge / a pr-sync-style integrator), any
  pre-merge hooks. Cadenza plugs in its nix gate + pr-sync + `.gate-baseline` merge driver + corpus guards
  here; a plain repo declares a trivial gate or none.
- **Declared roster (desired-state)**: this repo's PERSISTENT agents — name, role, model, effort, interval.

**Declared-vs-runtime reconciliation** (the design nuance the operator flagged): the per-repo config is
DECLARATIVE desired-state (checked in, decentralized). The hub holds ACTUAL runtime state (live windows,
heartbeats, inboxes, leases — inherently host-central). `fleet up <target>` reconciles: read the target's
declared roster → for each declared agent not running, mint its worktree (off that repo's base) + launch;
report drift (running-but-undeclared, declared-but-dead). The central `registry.json` becomes pure runtime
state; the DECLARED set is the union of all targets' checked-in rosters. One host runs one hub serving many
targets; a target's roster travels with the repo (clone the repo elsewhere → its fleet config comes too).

### What STAYS in cadenza (as its adapter)

Cadenza's build/gate/corpus/codegen/bench `xtask` subcommands stay entirely in cadenza (they are the
compiler's build tool, never fleet). Cadenza's `fleet.toml` adapter declares: base = trunk/origin-main,
gate = the nix local-gate, merge = pr-sync/self-merge, plus its `.gate-baseline` merge driver +
corpus-vanished pre-commit guard + baseline-drift cron (all cadenza-corpus concepts). The cadenza-shaped
`vertical` role becomes a cadenza-adapter role; `perf-agent` is the generic template in core.

### window.sh role-aware tick (the deferred generalization, now in-scope)

The kickoff/watchdog TICK is currently hardcoded cadenza framing ("cargo xtask fleet sync + pr-sync").
Generalize it to per-role/per-target: the tick recipe comes from the role + the target adapter, so a
foreign-repo agent gets a correct tick natively instead of overriding it in prose (as `perf-agent` does now).

## Phased plan (live-fleet-safe: ~33 agents + dcquic-perf must not break)

- **P1 — Scaffold (non-disruptive):** create the fleet repo; lift `fleet.rs` into a standalone `fleet`
  crate/binary; make the hub explicit config DEFAULTING to the current `<cadenza>/.claude/fleet`, so
  behavior is byte-identical and the live fleet is untouched. Build + unit-test in isolation. Audit + cut
  the small shared surface with cadenza-xtask (e.g. the check-lease pool shared with `gate`).
- **P2 — Adapter + generalize (non-disruptive):** define the `fleet.toml` schema (identity + gate/merge +
  declared roster); write cadenza's adapter reproducing today's exact behavior; land the role-aware tick +
  `ensure_worktree(target)` + the reconciliation `fleet up <target>`. Migrate the slack-bridge into the
  repo (still pointed at the current hub). Verify cadenza adapter is behavior-identical.
- **P3 — CUTOVER (the one risky step):** flip the live fleet to the standalone `fleet` binary + `~/.fleet`
  hub, cadenza as a target. Quiet window; migrate the hub in place (or symlink) so registry/inbox/queue/
  leases survive; re-home crons (system crontab → new paths) + hooks + the slack-bridge to the new hub.
  Cadenza-xtask fleet stays working as ROLLBACK until the cutover is proven.
- **P4 — Prove multi-repo:** stand up a 2nd target repo (s2n-quic) through the generalized native path,
  retiring the comms-shim special-case.

## Effort + risk

Medium-large; strongly favors INCREMENTAL. P1+P2 are low-risk and independently valuable (a cleaner,
testable, repo-agnostic core + adapter model) EVEN IF we never cut over — which is why I recommend doing
them first and reassessing before the P3 cutover. Big risks + mitigations:
- Breaking the live fleet mid-migration → cadenza-xtask fleet stays fully working until P3; rollback = point
  crons/launcher back at cadenza-xtask.
- Hub relocation losing runtime state → in-place/symlink migration + a backup snapshot; atomic.
- Cron/hook/slack-bridge re-homing → careful re-pointing (owned by v-fleet-tooling, low-surprise).
- xtask coupling → audit + cut the shared surface in P1.

## Repo name (for operator approval)

Recommend **`fleet`** (`github.com/camshaft/fleet`) — matches the binary, obvious, minimal. Alternatives if
a distinct name is preferred: `armada`, `flotilla`, `conductor`. (Binary stays `fleet` regardless.)

## Recommended sequencing

Do **P1+P2 (non-disruptive) first, then reassess** before committing to the P3 cutover — NOT a big-bang.
This lets the operator SEE the standalone binary + cadenza adapter + a 2nd-repo dry-run working with the
live fleet unchanged, and gates the concentrated risk behind that evidence.

## Open items gating execution

1. Operator approval of this doc.
2. Repo name (recommend `fleet`).
3. Confirm P1+P2-first-then-reassess (vs full-commit).

On approval → turn the phased plan into a concrete task breakdown and begin P1. Until then, nothing is
created or moved. See `AGENTS-fleet.md` (the contract that moves to core) and the fleet memory
`foreign-repo-fleet-agent-pattern` (the mechanics this builds on).
