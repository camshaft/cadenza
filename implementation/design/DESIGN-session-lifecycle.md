# Session lifecycles for the agent harness — spawn / suspend / resume / terminate + supervision

Owner: TBD (a `vertical`; kernel-facing slices are `v-agent-harness`'s zone, host-facing slices
`v-agent-harness-host`'s — see "Ownership & build sequencing"). Design by `design-session-lifecycle`.
Status: **PROPOSAL — peer-converged, awaiting a build owner.** Operator directive via concierge
2026-08-06 (verbatim): *"After session naming, I want to get session lifecycles designed and
implemented. … I won't be able to do a design session. So spawn an agent to get it designed with the
other agents and then we can assign it to an owner after everything is ready to go."* So this is a
proposal shaped WITH the two harness owners (`v-agent-harness`, `v-agent-harness-host`) autonomously,
not decided in a live operator session; the forks the operator may want to own on return are flagged
**⟨operator may ratify⟩**, each with a chosen default so the build is not blocked.

Subsystem: `cdz-kernel` (the durable lifecycle events + the terminal-fold guard) + `cdz-agent-host`
(the host-side executors that mutate the session registry). Coordinated with `v-agent-harness` (owns
the session model, `effect_ct` family vocabulary, `name_store` §4c, and the `SessionId =
genesis-hash-hex` decision landing now), `v-agent-harness-host` (owns `EmitExecutor` / `AsyncAgentHost`
loop / shared `Inbox` / Cedar `ComponentAuthorizer` §20b), and `design-session-directory` (owns the
multi-value directory / group membership — shares the membership-on-death seam, §8 below).

> **Ownership & build sequencing.** This layers strictly ON TOP of two things already designed/landing:
> (a) the §6/§6a supervision-tree design in `design/agent-harness-kernel.md` — `CloseOutcome`
> (Success|Failure) is BUILT (slice-1); `spawn`→child-`born` (slice-2), `close`→`child-completed`
> auto-delivery (slice-3), and the userspace supervisor library (slice-4) are DESIGNED there and
> referenced here, not re-derived; and (b) the just-landed cross-session messaging (`EffectKind::Emit`
> → peer `Inbox` → `Inbound` event, Cedar-gated). The genuinely-NEW surface this doc adds is
> **lifecycle-control of ANOTHER session** — `suspend` / `resume` / `terminate` a peer-or-child as
> first-class Cedar-gated effect families, plus the durable state model that makes them replay-safe.
> A session ending ITSELF (`close`, §6a slice-1/3) already exists in the roadmap; controlling a
> *different* session's lifecycle does not. Increments I1–I2 (kernel: terminal marker + fold guard,
> `Spawned` edge) are `v-agent-harness`'s zone; I3–I5 (host executors for spawn/suspend/resume/
> terminate + the bounce path) are `v-agent-harness-host`'s zone; I6 (Cedar descendant-authority) and
> I7 (prelude supervisor library = §6a slice-4) close it out.

This DOVETAILS with the extensible-effects arc: routing + authz already key on the effect **family
string** (`effect_ct::{SHELL,HTTP,MODEL,NOW,TIMER,EMIT,…}`, register-by-string via
`EffectRequest::new_with_family`, `CompositeExecutor::with_effect`). A lifecycle op is exactly a new
family the host can serve and Cedar can gate — so this REUSES that vocabulary rather than inventing a
control plane. No kernel recompile is needed to ADD the families; the kernel changes here are only the
two that must be durable+replay-safe (the terminal marker + its fold guard, and the `Spawned` edge).

## The problem

A Cadenza session is a durable event-log + KV. It is CREATED via `Session::genesis(reducer_hash)`;
"suspend/resume" is IMPLICIT today (a session IS its log, so resume = replay-from-log; crash-recovery
via `Session::recover` exists). What is NOT built: a session **terminating, suspending, or spawning
ANOTHER session** as a first-class operation. There is no lifecycle-control effect. Concretely:

- The only child today is `fork_for_query` (`kernel.rs:238`) — ephemeral, read-only, dropped after one
  drive; not a durable child.
- `EventBody::Closed { outcome: CloseOutcome }` (`event.rs:209`) — the type is BUILT but has **no
  kernel production path**; nothing appends it, and it is a session closing ITSELF, not being closed
  by another.
- The registry `AgentHost.sessions` (`host.rs:395`) is mutated only by the out-of-band admin plane
  (`AdminCommand::{InstallSession, StopSession}`, `admin.rs:46`) — an operator/driver action, NOT an
  effect a reducer can emit under Cedar authority.

The operator wants sessions to spawn, suspend, resume, and terminate each other as governed, audited,
replay-safe operations — the substrate for a supervision tree (who may kill/spawn/pause whom).

## The load-bearing constraint (durability + replay-safety)

A lifecycle transition that another session can observe MUST be a **durable, ordered log event**, never
ambient host state — for the same reason capability discovery must be a frozen query (`DESIGN-host-
capability-discovery.md`, §4b bridge rule): a live read of mutable "is B alive / suspended" poisons
replay. So:

- **Terminate is a durable event** appended to the *target's* log; a session whose log tail is that
  marker refuses further folds. Replay re-derives "terminated" from the log — deterministic.
- **Suspend/resume are NOT kernel state.** The kernel is already clock-free and replay-derived;
  "suspended" is purely a **host-scheduler** state (the host stops/starts scheduling the target's
  ticks). The durable log is untouched by a suspend. (Refinement from `v-agent-harness`: keep
  "suspended" out of the kernel entirely — it is a scheduler bit, not session state.) For AUDIT, the
  *controller's* intent is still logged on the controller side (a `lifecycle` Inbound the target folds,
  or an effect-result the controller folds), but suspension does not mutate the target's fold state.
- **Spawn records a durable parent↔child edge** on both sides (`Spawned{child_hash}` in the parent;
  parent-provenance in the child's genesis) — this edge IS the supervision tree and the authority
  substrate (§6, §7).

## The lifecycle state model

States, all derivable from durable state (no ambient truth):

```
            spawn (by parent)                terminate (by controller)
   ────────────────────────────►  Running  ─────────────────────────►  Terminated
        Created (genesis)          ▲    │                               (log tail =
                                   │    │ suspend                        Terminated;
                            resume │    ▼ (host scheduler only)          folds refused)
                                  Suspended  ── terminate ──►  Terminated
```

- **Created** — `Session::genesis`. For a spawned child, genesis carries parent-provenance.
- **Running** — normal; folds apply, effects dispatch.
- **Suspended** — host is not scheduling ticks. Durable log/KV intact. The inbox STILL ACCEPTS Emits;
  they are **QUEUED** (held), not dropped, so resume replays them (suspend is transparent to senders;
  §8). Purely a host-scheduler state — a per-session drive-eligible bool checked before `host.deliver`
  (⟨queue-not-drop chosen⟩: a dropped inbound during suspension is a lossy correctness hole, same
  reasoning as the terminate-bounce; §4).
- **Terminated** — a durable `Terminated` marker is the log tail; the kernel refuses further folds
  (`FoldRefused` guard, I1). log + KV RETAINED (queryable, frozen); `name_store` entry → tombstone;
  in-flight Emits to it bounce as Failure-to-sender (§4, §8).

Terminated is TERMINAL — there is no un-terminate (recovery from a bad state = re-`spawn` a fresh
session, §7). Suspend↔Resume is the only reversible transition.

## The lifecycle effect families (register-by-string, Cedar-gated)

A new **authz-GATED** partition — NOT the `control/*` prefix (that partition is authz-EXEMPT and
host-surfaced, wrong for kill/spawn which MUST be gated), and NOT `store/*`. Proposed namespace
`lifecycle/` (parallels `store/`), each an addable family (no kernel recompile):

| family                 | target        | effect result to caller           | durable events                         |
|------------------------|---------------|-----------------------------------|----------------------------------------|
| `lifecycle/spawn`      | reducer_hash  | child SessionId (= genesis-hash)  | `Spawned{child_hash}` in parent; child genesis w/ parent-provenance |
| `lifecycle/suspend`    | SessionId     | Ok / Failure                      | (none in target; host scheduler flag)  |
| `lifecycle/resume`     | SessionId     | Ok / Failure                      | (none in target)                       |
| `lifecycle/terminate`  | SessionId     | Ok / Failure                      | `Terminated{by,reason}` in target      |

`lifecycle/spawn` OVERLAPS §6 slice-2 (`spawn` effect). We do NOT build it twice: this doc adopts §6's
`spawn`/`born`/`spawned-child` shape verbatim and treats spawn as the family that establishes the edge
the other three families' authority derives from. If §6 slice-2 lands first under `v-agent-harness`,
this design consumes it; if this vertical lands first, it IS §6 slice-2. Either way one implementation.

Any "list my children / child status" surface is a **log-frozen QUERY** (bridge rule), not a live read
— out of scope for v1 beyond the terminal `ChildExited` signal (§5); a `lifecycle/children` query is a
follow-on.

## Supervision — reuse Emit/Inbound (do NOT build a separate channel)

Parent observes a child's terminal outcome via the just-landed cross-session plumbing: when a child
`close`s (or is terminated), the host emits a `ChildExited{child, CloseOutcome}` **Inbound event into
the parent's inbox**, discriminated by family string (e.g. `lifecycle` / `supervision`). This is
literally cross-session messaging with the host as sender — no new mechanism. This is §6 slice-3's
`child-completed`, named consistently. Restart/escalate/give-up is **userspace supervisor policy** (the
parent reducer folds `ChildExited` and decides — §6a slice-4 prelude library, I7); the host is
mechanism, not policy, consistent with the whole kernel/host split.

`CloseOutcome{Success(payload) | Failure(reason)}` (BUILT, slice-1) is the ready payload, so the parent
distinguishes a clean completion from a failure. `FoldFailed` (`kernel.rs:957`, BUILT) and effect
`Err`/`TimedOut` are the other watchable failure events a supervisor folds.

## Terminate semantics (the calls, made)

- **log + KV**: RETAINED, durable, queryable — but FROZEN (the `FoldRefused` guard, I1). A terminated
  session is a readable tombstone, not a deletion; audit/forensics survive.
- **`name_store` entry**: TOMBSTONE — resolves but flagged terminated (so a dangling name resolves to a
  clear "terminated", not a `NotFound` that reads like a bug). Coordinated with `design-session-
  directory`: DIRECT name = tombstone; GROUP membership = auto-evict (§8).
- **in-flight Emits to a terminated target**: BOUNCE as **Failure-to-sender**, NOT silent drop.
  Rationale (`v-agent-harness` concurs): the effect model is Success/Failure-distinct — a dropped Emit
  is an invisible correctness hole; a Failure the sender's reducer folds lets it retry/escalate/dead-
  letter as ITS policy. This mirrors fire-and-forget Emit's `Ok(None)` success shape — a terminated
  target flips it to a delivered Failure. A dead-letter note is a nice-to-have on top; the primary
  signal is the folded Failure. A terminated target is a **PERMANENT** failure (distinct from a
  transiently-closed inbox, which is RETRYABLE — the host executor must classify these differently, I5).

## Cedar authority — tree-derived descendant scope (no ambient authority, no bearer token)

A session may suspend/resume/terminate only its own **transitive spawn-descendants**. Given `SessionId =
genesis-hash-hex` (the decision `v-agent-harness` is pinning now — hashes are NOT prefix-hierarchical), a
`ResourcePredicate::Prefix` descendant check does NOT work. Instead the authority is the durable
**spawn-edge tree**: the `Spawned{child_hash}` events form the parent→child DAG; a controller may act on
any target reachable in its own spawn-subtree. Concretely a `Capability{kind: Lifecycle}` whose predicate
is "target ∈ my transitive Spawned-descendants", checkable from the log-derived tree — no ambient
authority, nothing to leak or revoke (the durable edge IS the grant). This directly answers the
operator's supervision-tree question: **the spawn-edge log IS the supervision tree, and IS the authority
model.** (A bearer capability TOKEN is the alternative — rejected as the default: a token can leak and
needs revocation machinery, whereas the edge is self-authorizing and audit-native. **⟨operator may
ratify⟩** if cross-tree control — e.g. a global operator session that may kill anything — is wanted, that
is an explicit broad grant on top, not the default.)

The Cedar `action` is the family string automatically (`ComponentAuthorizer` maps
`req.content_type.family` → action, `wasm_host.rs:3453`), so `lifecycle/terminate` etc. gate through the
same §20b path as Emit. The descendant-set predicate is the one new piece for `v-agent-harness-host` to
express (a `ResourcePredicate::DescendantOf` variant vs the existing `Prefix`, I6).

## The seams (file:line anchors)

- effect families / partition: `effect.rs:55-183` (`effect_ct` consts, `CONTROL_PREFIX:70`,
  `STORE_PREFIX:86`, `is_control_family:102`, `is_store_family:110`) → add a `LIFECYCLE_PREFIX` +
  `is_lifecycle_family`. Register-by-string: `EffectRequest::new_with_family` (`effect.rs:307`),
  `CompositeExecutor::with_effect` (`executor.rs:79`), routes on `content_type.family`
  (`executor.rs:101`).
- drive-loop partition branch: `drive_worklist` (`kernel.rs:669-944`) — lifecycle slots alongside the
  control (`kernel.rs:745`) / store (`kernel.rs:788`) branches, AFTER the Cedar gate (`kernel.rs:757`).
- kernel terminal marker + fold guard (I1): `EventBody` (`event.rs:89-210`) add `Terminated{by,reason}`;
  fold path `fold_tip` (`kernel.rs:~957`) refuses when log tail is terminal (new `FoldRefused`).
  `EventBody::Closed{CloseOutcome}` (`event.rs:209`, type BUILT) is the self-close analogue.
- `Spawned` edge (I2): `EventBody` + genesis parent-provenance (`Session::genesis` `kernel.rs:116`;
  today `Genesis` has NO parent field — I2 adds it, = §6 slice-2).
- host executors (I3-I5): mirror `EmitExecutor` (`emit.rs:40-121`). **Key mechanism (settled with
  `v-agent-harness-host`):** lifecycle effects run DURING a session's `deliver`, ON the `AsyncAgentHost`
  loop task — which OWNS the `!Send` registry. So they must NOT route-and-await like `AdminChannel`
  (`admin.rs:46-276`): the loop is busy running this very deliver, so a send-to-loop-then-await-reply
  would **self-deadlock** (the reply can't be processed until the deliver returns). Instead a lifecycle
  executor mutates the registry **INLINE via a `&mut AgentHost` handle** (it is already on the loop
  task, unlike admin which originates off-task on a socket). spawn ≈ inline `AgentHost::spawn`
  (`host.rs:474`) returning the child id synchronously; terminate ≈ inline `remove` (`host.rs:575`) +
  Terminated marker + bounce. Suspend/resume ≈ flip a new per-session drive-eligible bool checked
  before `host.deliver`. (AdminChannel routing stays correct for OFF-task producers — the operator
  socket — but NOT for on-task lifecycle effects.) Bounce is a NEW PERMANENT delivery-failure route
  (below), not the RETRYABLE closed-inbox path.
- Cedar (I6): `ComponentAuthorizer` action=family (`wasm_host.rs:3453`); `Capability::for_family`
  (`effect.rs:453`), `with_family_grants` (`authz.rs:71`); new `ResourcePredicate::DescendantOf`.
- naming/directory coordination (§8): `name_store.rs` (`NameAuthority:53`, `session/<id>/` prefix:65,
  `resolve` freezes:150); shared-store spawn/merge (`host.rs:415` `replay_of`, `host.rs:550`
  `merge_appends_from`) — a spawned child needs its store replay-seeded + writes merged back.

## §8 — membership-on-death (shared seam with `design-session-directory`)

A terminated session's directory presence, split cleanly (proposal sent to `design-session-directory`,
reply pending — this is the chosen default):

- **DIRECT name** (`session/<id>` in `name_store`): TOMBSTONE — resolves, flagged terminated.
- **GROUP membership** (the multi-value directory layer `design-session-directory` owns): AUTO-EVICT on
  terminate, so a group multicast does not fan out to dead sessions. The `Terminated` event is the hook
  the directory layer subscribes to (a host-driven eviction), keeping directory ownership with that
  design — terminate just emits the signal.
- **SUSPEND is transparent to the directory**: a suspended session stays a group member; messages queue
  in its inbox for resume. Only terminate evicts. (Consistent with suspend = host-scheduler-only state.)

If `design-session-directory` prefers termination to fan out the removals itself (vs a host hook it
subscribes to), that is a coordination detail settled between the two verticals at build time; the
semantic split (direct=tombstone, group=evict, suspend=transparent) is the contract.

## §6a OPEN decisions — resolved with defaults (⟨operator may ratify⟩)

These are the five open decisions from `design/agent-harness-kernel.md` §6a. Landing defaults so the
build proceeds; the operator may override any on return:

1. **Orphan rule** (parent terminated while children live): default = **CASCADE terminate down the
   subtree** (Erlang default), BUT a per-spawn capability/policy flag `outlive_parent` lets a long-
   running monitor survive its spawner. The cascade walks the durable `Spawned` edges.
2. **Restart-intensity ceiling**: kernel-wide safety FLOOR (default **5 restarts / 60s window →
   escalate**), per-supervisor tightenable in the prelude library. Counter lives in the supervisor's KV
   (replay-safe).
3. **Backoff schedule**: exponential base + cap + jitter, with a max-attempts ceiling → escalate. Lives
   in the prelude helper (userspace, so replay sees the same delays — NOT host Rust). Default base 1s,
   cap 60s, max-attempts 6.
4. **`ChildExited` granularity**: v1 = TERMINAL only (`Success`/`Failure`). Intermediate
   `child-stalled`/`child-blocked` (via a `watch` §4b) is a deferred follow-on.
5. **`FoldFailed` recovery**: v1 = re-`spawn` only. `swap-reducer` + replay-from-snapshot in place
   (ties self-mod §7) is deferred.

## Increment plan (top-to-bottom, the way a vertical lands it)

Each increment is independently green + a coherent unit. Kernel slices (I1–I2) are `v-agent-harness`'s
zone; host slices (I3–I5) are `v-agent-harness-host`'s; land in this order (later slices depend on
earlier durable events).

- **I1 — kernel: `Terminated` marker + `FoldRefused` guard.** Add `EventBody::Terminated{by,reason}`;
  the fold path refuses to apply folds to a session whose log tail is terminal (a first-class kernel
  guard, not a host convention — a terminated session can't be re-driven even by a buggy host). Gate: a
  fold-unit test (terminate → next fold refused) + a replay test (recovered session stays terminated).
- **I2 — kernel: `Spawned` edge + genesis parent-provenance.** `EventBody::Spawned{child_hash}` in the
  parent; parent-hash in the child's genesis events (= §6 slice-2). This builds the durable tree I6's
  authority + §8's cascade walk consume. Gate: spawn-a-child unit + the parent↔child edge is on both
  logs + child genesis-hash is provenance-dependent.
- **I3 — host: `lifecycle/spawn` executor (inline registry mutation).** The executor runs on the loop
  task and holds a `&mut AgentHost` handle: it instantiates a child `HostedSession` from the supplied
  reducer_hash and inserts it into the registry INLINE (SessionId = child genesis-hash-hex),
  replay-seeds its shared store, and returns the child id to the parent fold SYNCHRONOUSLY. Inline (not
  route-and-await) is REQUIRED to dodge the on-loop self-deadlock. Gate: wasmtime run — a reducer
  spawns a child, child exists in the registry, parent folds the returned id.
- **I4 — host: `lifecycle/suspend` + `lifecycle/resume` executors.** A new per-session drive-eligible
  bool checked before `host.deliver` in the inbound arm; suspend stops scheduling the target's ticks
  (log untouched) and QUEUES (holds) inbound, resume re-enables + replays the held inbound. Gate: a
  suspended session's queued Emits are not folded until resume, then are (not dropped).
- **I5 — host: `lifecycle/terminate` executor + a NEW PERMANENT Emit-bounce route.** Inline: appends the
  `Terminated` marker (I1), removes from the registry, and adds a terminated-target check in
  `EmitExecutor`/routing (target absent from registry AND in the terminated-set) → bounces in-flight
  Emits as **PERMANENT** Failure-to-sender — a NEW delivery-failure route, distinct from the RETRYABLE
  closed-inbox path (a terminated target is gone for good; a closed inbox is a transient host restart).
  Gate: terminate → target refuses folds + a pending Emit to it lands as a PERMANENT Failure the sender
  folds (and does not retry).
- **I6 — Cedar: descendant-authority.** `ResourcePredicate::DescendantOf` + `Capability{kind:
  Lifecycle}`; the authorizer checks target ∈ controller's transitive `Spawned`-descendants.
  Gate: a reject test — a session cannot terminate a non-descendant (AuthzDenied), can terminate its
  own child.
- **I7 — prelude: userspace supervisor library** (= §6a slice-4). One-for-one restart + retry-with-
  backoff + restart-intensity ceiling, consuming `ChildExited`/`FoldFailed`. Gate: a supervisor reducer
  restarts a failed child up to the ceiling, then escalates (its own `close(Failure)` → grandparent).

## Coordination points — resolved with the harness owners

All three host-mechanics questions are answered (with `v-agent-harness-host`); recorded here so the
build owner inherits the decisions:

- **Child-id shape**: SessionId = genesis-hash-hex (with `v-agent-harness`; the operator is pinning
  this now). Spawn returns the child genesis-hash as the effect result. (Confirm the pin has landed
  before I3 freezes the effect-result type — the only remaining external dependency.)
- **Host request/response shape (I3)**: RESOLVED — lifecycle effects run ON the loop task, so a
  route-and-await would self-deadlock; the executor mutates the registry INLINE via a `&mut AgentHost`
  handle and returns the child id synchronously. NOT AdminChannel-style (that is for off-task socket
  producers). `v-agent-harness-host` owns this seam.
- **Suspend drop-vs-queue (I4)**: RESOLVED — QUEUE (hold) inbound during suspension; resume replays.
  Drop is lossy. New per-session drive-eligible bool before `host.deliver`.
- **Bounce route (I5)**: RESOLVED — a terminated target needs a NEW PERMANENT delivery-failure route
  (registry-absent + terminated-set check), distinct from the RETRYABLE closed-inbox path.
  `v-agent-harness-host` owns it in `EmitExecutor`/routing.
- **Cedar `DescendantOf` (I6)**: RESOLVED — a NEW `ResourcePredicate` variant (§20b today has only
  `{Any, Exact, Prefix}`); carries the ancestor id, the authorizer walks the durable spawn-edge log to
  test subtree membership. Its own increment (a real new predicate + a spawn-edge-log read in the authz
  path — not free). `v-agent-harness-host` owns it.
