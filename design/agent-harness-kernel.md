# Agent Harness — Kernel Design (v2, from-scratch)

> **Status:** living design doc, captured mid-session. Not final. This supersedes the
> current `cdz-kernel`/`cdz-agent` code, which is being nuked and rebuilt from these
> learnings. Nothing is lost — the old code is in git history.

## 0. One-sentence vision

A **generic, minimal, if-this-then-that wasm runtime** whose entire job is: accept
events into an append-only log, invoke the current wasm "loop" program (the *reducer*),
authorize the effects it requests, dispatch them (locally via WASI or remotely via an
outpost), and fold the results back in as new events. The kernel knows **nothing** about
Cadenza, agents, models, or tools — all of that is plugins, wasm components, and events.

Deploy the kernel **once**; extend it forever with wasm components communicating over
wasm component interfaces. Even the authorization engine (Cedar) is a swappable component.

## 1. Non-negotiable principles

1. **The kernel knows nothing.** No hardcoded knowledge of Cadenza, agents, models, MCP,
   repos, or prod. Adding a capability like "query a data lake" must require **zero kernel
   changes** — only a new executor + capability. This is the litmus test for every feature.
2. **Everything is an event.** Model request/response, MCP request/response, tool calls,
   decisions, spawns, closes, reducer swaps (self-mod) — all one signed, ordered,
   content-addressed stream.
3. **Determinism / pure folds.** The reducer is a pure deterministic function of the log.
   ALL nondeterminism (model output, http result, shell output, clock, randomness) enters
   ONLY as recorded events. This is the crown jewel — replay, migration, audit, safe
   self-mod, and multi-tenant governance are all downstream of it.
4. **Minimal dependencies / own our destiny.** Build on `wasmtime` + `tokio` + our own
   content-addressed store. NOT wasmCloud (would fight their lattice/NATS/OTP opinions).
   The distributed runtime, log, capabilities, and federation are ours.
5. **Reads are effects, writes are local.** A reducer *appends* only to its own log. Reading
   anything external (another session, a data lake, http) is an effect whose result is folded
   back into its own log as immutable history. This is the load-bearing wall (see §5).
6. **Fully reactive — NO polling anywhere. Appending an event IS the trigger to run the reducer.**
   Message a session → delivery is an append → reducer runs immediately (concierge wakes the instant
   you message it). Timer fires, child completes, query returns, effect result lands → all appends,
   all wake the reducer. The kernel's durable scheduler exists so *time* is a reactive trigger too,
   not something a reducer polls. The current fleet polls ONLY because it has no kernel to
   deliver-and-wake; the 30-min tick is compensating for a missing reactive substrate. (See §9d.)

## 2. Core loop (the whole kernel)

```
on append(event) to session S:
    instantiate current reducer for S (from S's live reducer-hash)
    effects = reducer.handle(event, kv_view_of_S)
    for each effect:
        decision = authorizer.authorize(effect, S.capabilities)   # authorizer is a wasm component
        if permitted:
            result = route(effect) -> executor                    # WASI | outpost | peer-inbox | model | MCP
            append(result_event) to S                              # folds back in
        else:
            append(denied_event) to S
```

That's it. The kernel is `authorize(effect) → route to executor → fold result back as an
event`. If this stays true as features pile on, the design is sound. So far it holds through
supervision, cross-session reads, memory, distillation, recall, and scoping — none added
kernel code.

**Executors are uniform and pluggable:** local WASI (shell/http), remote outpost, peer-session
inbox, model invocation, MCP (both directions). "Multi-function across repos / prod / metrics"
= new executors + capabilities, never new kernel code.

## 3. Sessions

A **session** is the fundamental unit. It is:
- a signed, content-addressed, append-only **event log** with strict internal total order,
- an attached **key-value store** (the projected current state — see §4),
- a **reducer** (a wasm component, referenced by content hash),
- a **capability set**,
- **resource bindings** (virtualized compute/storage handles).

A session is the unit of **replay, migration, sandboxing, and retention**.

**Granularity rule:** two events share a session only if (a) they must be strictly ordered
relative to each other, OR (b) they share a retention/compaction lifecycle. Pick granularity
by *ordering + shared fate*, NOT by topic. Natural unit = one agent, one task, one bounded
lifetime.

**There is no global total order.** Total order *within* a session; causal/partial order
*across* sessions (via `cause` edges — see §5). The kernel is a multiplexer routing many
independent sessions. "Deploy once" = one kernel, many sessions.

### Genesis + context-as-events
- **Genesis event** names the reducer's content hash (self-describing → portable).
- **System prompt / context are just early events** the reducer folds into KV. Config isn't
  kernel state — it's history: auditable, diffable, replayable. Setup events can themselves be
  the *output of another session* (a "session factory" / template agent emits the setup events
  for a session it spawns). Configuring and running a session use the same mechanism.

## 4. State model — session-attached KV (decided)

The reducer is **stateless between events**. Its entire contract:

```
handle(event, kv) -> effect-requests        # kv: get / put / delete / prefix-scan
```

No returned state blob, no long-lived linear memory. Every fold = fresh wasm instance: read
what you need from KV, emit effects, write back. Consequences:

1. **Snapshotting never touches the reducer.** We never serialize opaque wasm linear memory.
   We snapshot the KV (kernel-owned application data). Checkpointing is fully transparent to
   the reducer — it cannot observe whether it woke fresh from a snapshot or mid-stream.
2. **Migration is trivial.** A session = `(log + KV + reducer-hash)`, all content-addressed.
   Ship those three; any worker can run the next fold. This makes the "virtualized, migratable,
   sandboxable" data plane mechanical, not aspirational.
3. **Version-mismatch softens.** KV values are application-level bytes the reducer defined, so
   a v2 reducer can read v1's keys given a compatible schema. Self-mod becomes normal data
   migration, not "impossible opaque-memory reinterpretation."

### Snapshots = free, per-event, via persistent-map KV
If the KV is a persistent/immutable map with structural sharing (CHAMP — already have it), then
**after every fold you have a KV root hash for free**; consecutive events share nearly all
structure. So:
- A **snapshot is formally `(event-index N, KV-root-hash, reducer-hash)`** — nothing more.
- The reducer never emits a `snapshot` effect; it doesn't participate.
- Checkpointing is a **retention decision, not a compute decision**: you get a valid checkpoint
  at every event; you just choose which root hashes to *keep* (all while hot, sparse while warm,
  latest while cold). GC policy over a free stream of checkpoints.

### Determinism rule for KV
KV reads during a fold are **point-in-time**: folding event N sees KV as mutated by folds up to
N-1, never live/future KV. Since we always fold in order, this is automatic. **KV mutations are
NOT logged as their own events** — they're the deterministic side output of folding; the KV
rebuilds itself on replay. Log stays thin (events + retained root hashes).

### KV richness — OPEN
`get/put/delete/prefix-scan` is the floor. Prefix-scan needed for agent collections (pending
children, seen-memories, per-repo working state). User expects "decently rich" since KV is a
projection of full session state. Open question: flat map + prefix-scan (lean) vs. range
queries / secondary indexes vs. treating KV as a real database. Provisional lean: keep KV
primitive minimal, push richer querying into a *reducer* that maintains its own indexes in KV
— but revisit; user leans richer.

### Blob boundary
Same rule as the log: small values inline, large values (transcripts, diffs, model payloads)
are **hashes into a shared content-addressed blob store**. A KV entry holding a 4k-token
transcript is `key → blob-hash`, not inline bytes. Log + KV share ONE blob store. (Reuse the
Membrain telemetry + value-heap content-addressing muscle.)

## 4b. Storage tiers — local vs. global bridge (decided)

There are **three** storage tiers, not two. The middle one answers "how does a supervisor
check a child's status" without the child having to heartbeat.

1. **Session-local KV** (§4) — the reducer's private, deterministic, replayable projection.
   Inbox, working set, pending-children, semantic status. Fast, isolated, part of the fold.
   **The inbox is local**: a message is *delivered into the target's local log* by the kernel
   (an authorized cross-session effect) and folded locally. NOT a global mailbox everyone polls
   — "global" is only the routing (session-id → hosting worker).
2. **Kernel log-metadata** — intrinsic, kernel-owned, **non-deterministic** structural facts about
   each log that the kernel already tracks to run it:
   `{state, last_event_at, event_count, in_flight_effect: Some(kind, since_T) | None, cursor}`.
   Not in anyone's KV, not part of any fold. This tier answers liveness/stall questions for FREE
   — the child never writes "I am alive." **In-flight effect since T with no result event and
   now−T > threshold = STALLED** — this is exactly the fleet's hand-rolled wedge-detection triad
   (CPU + heartbeat + wrapper), but as one structural fact the kernel can't be wrong about.
3. **Global stores** — cross-session: memory store (§9), session registry/directory,
   capability/policy store.

### The bridge rule: immutable-by-hash vs. mutable-current-view (NOT local-vs-global)
- **Immutable content addressed by hash → direct read, no effect, determinism-safe.** Memory
  artifact by hash, a past event, a snapshot, a reducer wasm — the bytes never change, so reading
  the global blob store directly is safe. *Immutability IS determinism.*
- **Mutable "what's current" → MUST be a query effect, frozen into the local log.** "Is session B
  alive?", "what memories about topic X are valid now?", "which sessions touch repo X?" — a live
  read would break replay. `query` → kernel answers as-of-now → answer freezes into your log at a
  hash → replay reads the frozen answer. Nondeterministic *when asked*, deterministic *once folded*
  (same as reading http).

### Status check, worked (the supervisor scenario)
Supervisor emits `query(child-id, status)`. Kernel answers from:
- **tier-2 log-metadata** — mechanical liveness: last-acted, in-flight, stalled?, closed? (free,
  can't-lie), plus
- **tier-1 published view** — semantic progress ("investigating auth module"): a `status` key /
  `public/` prefix the child chose to expose. Kernel cannot know semantics; only the child can
  publish them. (This is the raw-events-vs-published-view question §16.3: others see a session's
  *published view*, not its full KV/raw log; full access = higher privilege.)
Result lands frozen in the supervisor's log; its reducer decides nudge/wait/escalate/kill.

### Watch, not poll
Don't poll `query` on a timer. Generalize the `child-completed` auto-delivery (§6): supervisor
emits `watch(child, stall_after=5m)`; kernel delivers `child-stalled`/`child-blocked` to its inbox
when the log-metadata condition trips. **A watch/timer is an effect whose result is delivered
later** — same shape as a DES sleep, deterministic because the delivered event is recorded. Push,
event-driven, no polling loop. Retires the fleet's hand-rolled watchdog.

### Where each tier falls down (why the split must exist)
- **All-local (no global):** can't *discover* — a supervisor sees only sessions it spawned (their
  IDs are in its KV); can't find an un-spawned peer, share knowledge, or coordinate on a shared
  repo. Fails at any global view.
- **All-global (no local):** determinism dies (mutable shared reads in the fold), and the
  **single-writer bottleneck returns** — every inbox delivery becomes a contended global write with
  an ordering nightmare. Fails at hot-path, private, high-write, deterministic state.

### Structural payoffs
- **Global stores ARE sessions.** The memory store is a session: log = publish/retract stream, KV =
  current index, reducer = the gardener. `query` to recall, `emit` to publish (authorized). No
  special "global store" subsystem → keeps "kernel knows nothing." Caveat: single-writer-per-session
  → hot global stores are **partitioned** (per-scope/per-team) with a federating index. Scaling
  detail, not new machinery.
- **Session registry is the one hot mutable global on the critical path** — maps session-id →
  current host + metadata; must be reasonably fresh for routing (session-id is location-independent;
  kernel resolves current host, so migration is transparent to a querying peer). Flag as the primary
  scaling risk.

## 4c. Global store — mutable-name authority & anti-hijack (decided)

The global store provides exactly three verbs; the kernel has NO opinion on relevance/timing
(that's userspace — see §9 active-vs-passive):
- **put(document) → content-hash** (immutable blob).
- **index(doc)** — effectful (embeddings via a model call); the semantic index is itself a
  global-store session you query.
- **query(index, situation) → candidate hashes** — semantic recall; returns hashes (immutable) →
  direct-fetch safely.

### Two layers, different trust properties
- **Content-addressed blobs:** immutable, self-verifying, readable anywhere, **no write-auth**
  — the hash IS the authorization; you can't forge bytes without changing the hash.
- **Mutable name → hash pointers:** THE attack surface. A mutable name is the ONLY thing needing
  write control. (Compiler-hijack scenario: repoint `compiler-latest` at an evil hash.)

### Anti-hijack model
1. **A mutable name is an append-only SIGNED log.** `compiler-latest` isn't an overwritable slot —
   its value-over-time is a sequence of signed `set(name, hash)` events; current value = latest
   *authorized* set. Buys audit (who-set-what-when), rollback, attribution. ("Everything is a log"
   applied to pointers.)
2. **Write-authority via hierarchical namespaces + delegated, ATTENUATING capabilities.** The
   *prefix* determines who may write:
   - `system/compiler/latest` → only a system/release grant may set → a random agent's `set` is
     REJECTED by the authorizer (injection fails).
   - `team/rust-backend/*` → team membership; `session/<id>/*` → owned by that session.
   - Grants **attenuate down the spawn tree**: a session signs with a delegated identity chained to
     its spawner's grant, and **can never grant a child more than it holds.** Kills
     privilege-escalation-via-spawning (the scary multi-operator path).
3. **Security-critical refs PIN the hash; mutable pointers are discovery-only.** Genesis pins the
   EXACT compiler/reducer hash, not `compiler-latest`. Resolving a mutable pointer is an explicit
   act that **freezes the resolved hash into the resolver's log** (determinism). A hijacked pointer
   can only mislead FUTURE opt-in resolvers — it can't retroactively alter any running/past session;
   the malicious `set` is signed/attributable/revocable; blast radius traceable via `cause`. Hijack
   becomes loud + bounded instead of silent + total.
4. **Revocation** = a logged event; the store validates writes at write-time against *current live*
   grants, so a revoked cap stops working immediately; past writes stay attributed for forensics.
5. **Identity/root of trust:** each operator = a top-level namespace they root; cross-operator trust
   = explicit accepted-roots config. P0 single-operator = one root (you). Sessions get short-lived
   delegated identities (§10). **Session-local KV needs NO auth** — sole writer is its own reducer;
   the permission machinery exists ONLY for the global mutable-name layer.

**Unification:** "set `compiler-latest`" and "promote a memory to org-public" (§9) are the SAME
operation — an authorized write to a scoped mutable name. One mechanism (namespaced, cap-gated,
signed, logged writes) covers compiler pointers, memory promotion, and any shared authoritative ref.

## 5. Cross-session interaction — "reads are effects" (load-bearing wall)

Reducers **never** read or write another session's log directly. Crossing a boundary is an
explicit, logged, authorized **effect** — the same primitive as a shell/http call.

- **Send/signal:** Session A emits `emit(target=B, payload=hash)` — an event in A's log. Kernel
  authorizes (does A's capability set permit signalling B?) and delivers it as an inbound event
  in B's log: `signal-from(A, payload=hash, cause=A's-event-id)`. B folds it as a normal event;
  B never knew A "reached in." (This is the fleet file-inbox, formalized.)
- **Read/query:** Session A emits `query(target-session, filter)`. Kernel authorizes, runs the
  query against the target's log/view **as of now**, folds the result back as
  `query-result(payload=hash)` in A's own log. The answer is **frozen into A's history at a
  content hash** → replay reads the frozen answer, not the live target.

Three properties this buys:
1. **Determinism survives** — cross-session bits are frozen into local history at delivery/read
   time; the source session need not still exist to replay the receiver.
2. **Causality is explicit & auditable** — every cross-session event carries
   `cause = source-event-id`, giving a **global causal DAG** (partial order) with no global total
   order. "Why did agent X touch prod?" = follow `cause` edges backward across sessions.
3. **One choke point** — cross-boundary is the ONLY thing to secure/authorize/rate-limit; and it
   uses the same code path as tool calls, so the kernel has no special "cross-stream" logic.

**Read/write asymmetry** is the essence: writes are local + ordered (determinism lives here);
reads snapshot external state into local immutable history (safe touch of the outside world).

## 6. Supervision tree (durable, better than Erlang)

Erlang's tree is ephemeral (in-memory restart policy). Ours is **durable + replayable**.

- `spawn(reducer, caps, goal, cause=my-event)` is an effect. Kernel creates a child session,
  records `spawned-child(child-id)` in parent's log and `born(parent-id, goal, caps)` as child's
  genesis. Parent↔child link is immutable on both sides.
- `close(outcome)` in the child is an effect. Kernel sees the child had a parent and auto-delivers
  `child-completed(child-id, CloseOutcome)` to the parent's inbox — `outcome` is the structured
  `CloseOutcome { Success(payload) | Failure(reason) }` (§6a, BUILT slice-1), so the parent
  distinguishes success from failure. Not special kernel logic — just: the child's genesis recorded
  a parent, so `close` routes the outcome there.
- **Supervision strategy is userspace.** The parent's reducer folds `child-completed` and decides
  respawn / escalate / give up. Kernel hardcodes NO strategy — the parent reducer *is* the
  supervisor. (Minimalism test passes.)

**Scenario (works fully within this):** PM agent emits
`spawn(developer-reducer, caps=[repo-X, read-metrics], goal="fix bug 123")`; developer spawns its
own children; they report via `child-completed` up the chain. The whole engagement is a
causally-linked subtree — migrate/sandbox/audit as a unit.

**OPEN — orphan rule:** if a parent is closed/compacted while children live, what happens?
Default lean = close cascades down the subtree (like Erlang), BUT make it a capability/policy
choice (some children should outlive their spawner, e.g. a long-running monitor). Flag, don't
decide yet.

## 6a. Error resilience & self-heal (operator directive 2026-08-02)

**Directive:** errors must NOT stop a session dead or vanish into the void; the harness heads
toward Erlang/OTP-style supervision — failures captured, isolated, recovered by a supervisor
(restart/retry/escalate) so the system SELF-HEALS over time rather than needing a manual
jump-start. This is core to robustness, not nice-to-have. It's a **design direction realized in
incremental slices**, not one change — this subsection is the shape + the path, for the operator
to steer the tree design (esp. the OPEN decisions at the end).

### The failure taxonomy (every failure becomes a first-class event, never silent)
The kernel already turns most failures into ordered log events a supervisor can fold; the gap was
the reducer trap. Full taxonomy:
- **Effect failure** — `EffectResult::Err(reason)` / `TimedOut` (§9d anti-stuck). A hung effect
  becomes a normal event, never a wedge. Executors classify: `RETRYABLE:`/`PERMANENT:` prefix on
  the reason (a Bedrock 5xx/rate-limit is retryable; a 400 is permanent).
- **Reducer trap / fuel-exhaustion** — a guest fold that traps or burns its fuel is captured as a
  first-class `FoldFailed { reason, caused_event }` log event (**BUILT**, gap #1 closed), NOT a
  silent empty fold ("errors into the void", the old §17 fail-safe). v0 RECORDS it (a supervisor
  reading the log sees it); it is deliberately NOT re-folded (no recursion — a fold that fails
  can't be handed back to the same failing reducer this turn).
- **Session close** — a session ends with a structured **`CloseOutcome { Success(payload) |
  Failure(reason) }`** (**BUILT**, slice-1) — no longer an opaque payload, so a parent can tell a
  clean completion from a failure and react. This is the outcome `child-completed` carries up.

### The self-heal loop (userspace supervisor, kernel strategy-agnostic)
Supervision strategy stays **userspace** (§6): the parent reducer IS the supervisor. On a
`child-completed(child-id, CloseOutcome)` (or a `FoldFailed`/effect-`Err` it's watching), the
supervisor reducer decides:
- **Retry-with-backoff (transient):** re-emit the failed effect / re-spawn the child after a delay
  via a `Timer` effect. The delay comes from a backoff schedule (exponential + a max-attempts
  ceiling). **The retry DECISION is a logged event** (the supervisor's fold emits the Timer +
  records the attempt count in its KV) → replay-deterministic, ties into the durable log + resume
  already built. NOT host-side Rust auto-retry (that would bypass the log + duplicate the model).
- **Restart (one-for-one):** re-`spawn` a fresh child over the same goal/caps.
- **Escalate:** a repeatedly-failing child is isolated — the supervisor `close(Failure(…))`s
  itself, so ITS `child-completed(Failure)` escalates to the GRANDPARENT rather than crash-looping.
- **Restart-intensity ceiling (OTP's max-restarts-in-period):** N restarts within a window →
  escalate instead of looping. The counter lives in the supervisor's KV (replay-safe).

### Where the pieces live (layering — keeps the kernel minimal)
- **Kernel:** the first-class failure EVENTS (FoldFailed, structured CloseOutcome, effect Err/
  TimedOut) + `spawn`/`close`/`child-completed` auto-delivery (§6). Strategy-free.
- **Prelude / library (Cadenza, userspace):** a reusable **supervisor reducer** (one-for-one
  restart + the retry-with-backoff schedule helper — the backoff math lives HERE, in the layer the
  supervisor runs, so replay sees the same delays; NOT in host Rust). This is the layer an app's
  PM/developer reducers compose.
- **Host (cdz-agent-host):** executors surface real errors as classified `EffectOutcome::Err`
  (Bedrock/Http/Shell 5xx→retryable) — already done. The host does NOT auto-retry (confirmed with
  v-agent-harness-host): retry is the userspace supervisor's job.

### Incremental path (slices)
- ✅ **gap #1 — FoldFailed error event** (a trapped fold → first-class logged event). BUILT.
- ✅ **slice-1 — structured CloseOutcome** (Success|Failure vs opaque payload). BUILT — the
  precondition a supervisor needs to distinguish success from failure.
- ⏭️ **slice-2 — spawn as an effect** → child session with `born(parent, goal, caps)` genesis +
  `spawned-child` in the parent log (the immutable parent↔child link).
- ⏭️ **slice-3 — close auto-delivers `child-completed(child-id, CloseOutcome)`** to the parent
  inbox (the child's genesis recorded a parent → `close` routes the outcome there).
- ⏭️ **slice-4 — the userspace supervisor library** (prelude): one-for-one restart + retry-with-
  backoff + restart-intensity ceiling, consuming child-completed/FoldFailed.

### OPEN decisions for the operator to weigh
1. **Orphan rule** (from §6): parent closed while children live → close-cascade (Erlang default)
   vs. outlive-spawner (a monitor should survive its spawner)? Lean: make it a capability/policy
   flag per spawn, default cascade. **Needs a call.**
2. **Restart-intensity defaults:** the max-restarts-in-period ceiling (N, window) before escalate —
   what defaults, and is it per-supervisor-configurable or a kernel-wide safety floor?
3. **Backoff schedule shape:** exponential base + cap + jitter? A max-attempts ceiling before a
   retry becomes an escalate? (Affects the prelude helper's signature.)
4. **child-completed granularity:** does the parent get intermediate `child-stalled`/`child-blocked`
   signals (via `watch`, §4b) in addition to terminal `child-completed`, and does a supervisor act
   on those or only on terminal outcomes?
5. **FoldFailed recovery:** v0 records-but-doesn't-refold. Should a supervisor be able to
   `swap-reducer` + replay-from-snapshot to recover a trapped session in place, or is re-spawn the
   only recovery? (Ties self-mod §7 into supervision.)

## 7. Self-modification

`swap-reducer(new-hash)` is just an authorized event. From the next event the kernel instantiates
the new component; the new reducer **inherits the existing KV**. Because the reducer is stateless
and state lives in KV, the only real constraint is **KV-schema compatibility** across the swap.

- Snapshots are reducer-tagged (`reducer-hash` in the snapshot tuple). A snapshot is a valid
  fast-forward point for any reducer that can read its KV schema (relaxed from the earlier
  "same reducer only" because KV is app-level, not opaque memory).
- **Compaction interlock:** never prune a raw event you might still need to deterministically
  re-apply a reducer swap. Only compact behind a snapshot whose reducer schema the current
  reducer can read. Self-mod, snapshots, and compaction are one interlocking system — all in the
  log.

## 8. Retention / compaction lifecycle

Snapshots make transcript compaction safe. Lifecycle per session:
1. **Hot:** raw events + all/most KV root hashes; full replay.
2. **Warm:** raw events pruned up to last snapshot; replay-forward from snapshot only (fine-grained
   pre-snapshot history lost). **Distillation (§9) must have already run** — so `session-close`
   triggers distill *before* compaction eligibility. Distillation is on a "use-it-or-lose-it"
   clock tied to the compaction window.
3. **Cold:** snapshot + distilled memory only; raw transcript archived or dropped.

Retention rule: only compact behind a matching-reducer snapshot (see §7).

## 9. Shared memory (crown jewel + biggest trap)

Three separable sub-problems:

**(a) Capture — nearly free.** Don't ask agents to write memories (Claude-CLI model: lossy, biased
to what the agent *thought* mattered). Memory is a **derived view over the event log**. The raw
lived experience — goal, tool calls, failures, operator interventions, outcome — is already in the
log. Distill it; don't capture it.

**(b) Distillation — real work, itself a session.** A "librarian/distiller" reducer queries raw
session logs and emits **memory artifacts** into a shared content-addressed memory store. Turns
"session 4471 fought a stale value-heap store for 40 min" into "when you see `no runtime of content
address`, run `cargo xtask build` — stale store, not broken trunk." Auditable + replayable like any
session. Artifacts carry **provenance**: derived-from-which-sessions, under-what-goal,
acting-for-whom, validated-when (formalizes the current memory-file frontmatter + links back to the
lived events that justify it).

### Active-vs-passive recall — DECIDED: userspace
How aggressively a memory is injected is the **reducer's** job, not the kernel's — relevance is
semantic = userspace. Kernel provides only: **put document → hash, index it, semantic query.** The
reducer decides *when* to recall (session stalling → recall to get unstuck; context too big → stash
memories then compact) and *when* to publish. "How aggressive is injection" can itself be a
swappable reducer we iterate on without touching the kernel.

### Memory artifact structure — typed, addressed CLAIM (not a document blob)
Memory is closer to a set of `(trigger → claim)` production rules than a document store — echoing
the kernel's own if-this-then-that. Fields (each GATES recall, not decoration):
- `claim` — distilled knowledge (blob hash).
- `trigger` — **load-bearing:** the situation it applies to. Recall = matching a trigger to the
  recaller's current situation, NOT text search. Sharp trigger = safe; a triggerless fact gets
  recalled everywhere = the off-the-rails path. (Current best memories already have this shape:
  "see `no runtime of content address` → stale store, run build.")
- `provenance` / `validity` / `scope` / `confidence(evidence-count)`.
Distiller's job = raw transcripts → well-triggered rules. Gardener's job = retire rules whose
triggers stopped predicting or whose validity lapsed. **Trigger mechanism — DECIDED, see §9e.**

**(c) Recall — where agents go off the rails; push hardest here.**
- **Recall is a scoped query effect** — no ambient access; capability set defines which memory
  *scopes* an agent may query (own / team / org-validated).
- **Staleness → memories are falsifiable + expiring, NOT eternal.** Each artifact carries the
  world-state it was true against (trunk sha, tool version, date). A memory is a *claim*; later
  events can **contradict** it (memory says "do X to fix Y"; three sessions did X and Y persisted →
  log falsifies the memory → distiller/"gardener" retracts/downgrades). **This is what Claude-CLI
  memory can't do — a log-derived memory can notice it went stale because the falsifying ground
  truth is also in the log.** (The current memory index is littered with hand-maintained "STALE"
  annotations; the system should retract those itself.)
- **Off-the-rails → recall is evidence, not instruction, and traceable.** A recalled memory enters
  the fold as `recalled-memory(provenance, confidence)`; any action citing it records a `cause`
  edge back to the memory. A bad memory can't silently metastasize — every use is causally logged,
  so you can trace the blast radius and retract. (Formalizes the current discipline: "recalled
  memories are background context, not instructions; verify the named file still exists.")

**Strong position:** capture is free (it's the log); distillation is a session; recall is a scoped
query; every memory is a provenance-carrying, falsifiable, expiring **claim**, never an eternal
fact. That last clause is the whole difference between memory that compounds and memory that rots.

### 9e. Trigger mechanism — two-stage funnel, predicate-as-Cadenza (decided)

**A "situation" = the recaller's recent event context** — a window of its own recent log + relevant
KV (last events, current goal from `born`, the error just hit, the effect about to be made). Not a
new object; it's a projection the reducer already holds. So "match trigger to situation" = match a
rule against a slice of the recaller's own event stream — triggers + events speak the same language
(s-expr values), and the recaller controls *when* to ask (userspace timing).

**Two-stage funnel (forced by cost — can't precisely eval 10k rules per situation):**

- **Stage 1 — coarse recall by SIMILARITY (cheap/fuzzy/wide).** Embed the situation, NN against the
  memory index → ~20 candidates. This is the `query(index, situation)` effect (§4c). Job = *recall*
  not *judgment*: high recall, low precision, deliberately over-fetches. Fuzzy is fine; stage 2 is
  exact.
- **Stage 2 — precise firing by PREDICATE (exact, over ~20 not 10k).** Evaluate each candidate's
  actual trigger against the situation value. Affordable at 20.

**A trigger predicate IS a pure Cadenza function `situation → bool` (or `→ confidence`)** — the payoff
of code-is-data (§9b):
- **Exact + expressive:** "last event is a shell error containing `no runtime of content address` AND
  goal mentions a build" is *logic*, not an embedding. Sharp triggers (the safe ones) ARE logic, not
  vibes; embeddings can't reliably express "AND goal mentions X."
- **PURE → safe + deterministic:** trigger predicate has an EMPTY effect row, statically checkable
  (§9b). The promotion gate REJECTS any memory whose trigger isn't pure. A trigger cannot read
  network / shell / leak — only inspect the handed-in situation value. The dangerous part of a memory
  is its *claim* (declares its own effect row, authorized normally), NEVER its trigger.
- **Inspectable + gardenable as data:** gardener reads trigger Ast, clusters, finds dead rules
  ("offered N times, never fired"), refactors overlapping rules — memory reasons about its own
  relevance logic.
- **Runs as a normal fold:** a tiny no-capability wasm component; batch the ~20 candidates. Kernel
  stays generic.

**Neither stage alone works:** embedding-only fires on vibes (off-the-rails); predicate-only can't
scale AND can't find rules whose wording you didn't anticipate. Embedding finds *plausibly* relevant;
predicate decides *actually* relevant.

**Distiller now produces THREE things per memory:** (1) embedding anchor (situation text, stage-1
recall), (2) pure trigger predicate (Cadenza `situation → bool`, stage-2 firing), (3) claim (advice OR
executable remediation program with declared effect row). Meaty synthesis, but a reducer does it
(auditable/improvable) — exactly "write a small program from examples" that compiler+Ast tooling suits.

**Stage 2 returns RANKED, not a pile** (recaller has finite context budget; over-injection is its own
off-the-rails path). Rank = predicate-confidence × corroboration (evidence-count) × validity-freshness
× scope-proximity (my-team's memory outranks org-generic). Recaller's reducer applies its OWN budget
policy (userspace active-vs-passive): inject top-K / only-if-above-floor / surface-as-available-tools.

**OPEN — stage-1 recall ceiling (§16.13):** if the embedding misses a rule, its predicate never gets
to fire. Distiller must write anchors that generalize (multiple phrasings, the *error signature* not
just prose); gardener needs a retroactive "missed recall" signal (a session hit a problem, solved it,
and a matching rule existed but wasn't surfaced) to improve stage-1 over time. That audit is itself a
distiller/gardener task. Don't pretend embeddings are complete.

### Scoping model (three axes)
Applies to memory artifacts, sessions, and capabilities:
1. **Provenance scope** — acting-for-whom / which operator / which project (from `born`). *Where it
   came from.*
2. **Visibility scope** — private / team / operator / org-public. *Who may recall it.* Default
   private; promotion to broader scope is a logged, authorized act (a memory "graduates" when
   validated).
3. **Validity scope** — world-state it's true against + expiry/decay. *When it stops counting.*

**Promotion across visibility scopes = the governance choke point.** Org-public only via an
authorized event (human-approved and/or validated-by-N-independent-sessions). Same shape as the
fleet's pr-sync gate: nothing reaches "trunk" (org-public) without passing the gate. Memory needs
its own gate.

### OPEN — distiller trigger
Lean = two tiers: `session-close` triggers a *cheap* distill of that one session; a periodic
"gardener" does *cross-session pattern-mining* + staleness-retraction. Decides real-time vs batch
+ compute burn.

### 9f. The nightmare scenario + circuit breaker — DECIDED (no new machinery)
**Nightmare:** distiller derives a subtly-wrong memory → promoted org-public → agent after agent
recalls + follows it → correlated mistakes across operators' repos, propagating faster than a human
notices. Shared memory's strength (fast propagation) IS its danger (fast propagation of WRONG
knowledge). "Circuit breaker" = mechanism to (proactive) keep bad memories out of the shared pool AND
(reactive) stop the bleeding once one is loose. Worked out, it's NOT a new subsystem — it's existing
mechanisms + a promotion gate + an auto-trip monitor (both userspace):

**Proactive half — promotion is a TEST SUITE, not a decision** (memory equivalent of pr-sync's gate;
nothing reaches org-public/"trunk" without passing). A memory earns private→team→org-public by:
1. **Purity check** (free/static, §9e): trigger predicate has empty effect row or promotion is
   rejected.
2. **Held-out corroboration:** replay the trigger against past sessions the distiller did NOT derive
   it from — did it fire where the claim would've helped, and NOT where it'd be wrong? Precision/recall
   on held-out sessions = the promotion score. (A memory is a hypothesis; promotion requires it to
   predict on held-out data — your gate-baseline discipline applied to memory. §9e made this
   mechanical.)
3. **Independent corroboration:** ≥N *independent* sessions (different goals/operators) corroborate
   before org-public — kills "one fluke became gospel."
4. **Effect-row scrutiny for program-claims (§9b):** an org-public auto-remediation that can
   `Shell.exec` needs a far higher bar than a read-only one; bound the claim's effect row at promotion.

**Reactive half — kill-switch + blast-radius (mostly already have):**
- **Instant retract = one mutable-name write (§4c):** org-public status is a pointer; retracting is a
  single authorized `set` and every future `query` stops surfacing it immediately — no redeploy, no
  cross-operator cache invalidation.
- **Blast-radius via cause-DAG (§5):** every action citing the memory recorded a `cause` edge back to
  it → on retract, ENUMERATE every session that acted on it, across all operators, to
  notify/flag/roll-back. (Impossible with file memories: you can delete a bad note but can't know who
  read it or what they did.) The causal DAG IS the blast-radius report.
- **Auto-trip (the actual "breaker"):** a gardener monitor watches outcomes of sessions citing a given
  org-public memory; if failure-rate among citers spikes past a threshold → **auto-retract + escalate
  to human**, before a person notices by hand. This is what makes it a breaker vs. manual retraction.

**Only genuinely new commitments:** (a) promotion REQUIRES passing held-out validation; (b) a standing
gardener monitor auto-retracts on correlated failure. Everything else is composition of retract-via-
mutable-name (§4c) + cause-DAG blast-radius (§5) + trigger replay (§9e). Multi-operator memory safety
= a gate + a watchdog reducer, both userspace.

### 9g. Agent review of promotions — DECIDED (promotion = a pull request)
The mechanical gate (§9f) only filters the obviously-unfit; "is this actually good advice / correctly
scoped / not subtly misleading" is a SEMANTIC judgment mechanical thresholds can't make — so add
**agent review in the loop.** A reviewer is just another agent session. Promotion has two stages:
1. **Mechanical gate first** (cheap, automated) — purity / held-out precision-recall / corroboration /
   effect-row bounds. Fails here → never reaches a reviewer (protects reviewer attention).
2. **Agent review second** (semantic) — a reviewer session is spawned with the candidate memory + its
   provenance + held-out results + source sessions, and renders a verdict.

**This IS pr-sync applied to memory.** A promotion is a pull request: a proposed change to the shared
pool, mechanically checked then peer-reviewed. Three verdicts, three paths:
- **reject-with-feedback → send back for more work:** reviewer's feedback is an event delivered to the
  distiller session (§5); distiller folds it, produces a v2 memory — a logged REVISION LOOP, like a
  code-review round-trip. The promoted memory carries its whole review history in provenance (took 3
  rounds + why).
- **escalate-to-operator:** reviewer unsure / high-stakes (e.g. org-public `Shell.exec` remediation) →
  emits `ask` to operator; promotion WAITS on the human but nothing else blocks (idle session pending
  an inbound event, §9d). Reactive human-in-the-loop; operator answers whenever, session wakes.
- **approve:** the approval IS the authorized `set` that flips the mutable-name pointer to org-public
  (§4c). Approval = the *capability to promote*, which the reviewer holds and the distiller does NOT.

**Locked rules:**
1. **Derivation authority ≠ promotion authority.** Reviewer must be a different session/lineage than
   the distiller (else self-approval = gate is theater). For org-public in multi-operator, review
   draws from a DIFFERENT operator than the memory's provenance — cross-operator review prevents one
   operator rubber-stamping its memories into everyone's pool. **Promotion authority scales with
   visibility scope:** team-promotion needs a team peer; org-public needs cross-operator review or an
   operator.
2. **Review the reviewers.** A reviewer is a session → its verdicts are logged with `cause`. If a
   reviewer approves a memory that later trips the reactive breaker (§9f), that's a signal about the
   reviewer; the gardener flags reviewers whose approvals correlate with breaker-trips. (The
   "audit the auditor" turtle, §5, made useful.)
3. **Recursive but grounded:** reviewer authority is a capability grounded in the trust root
   (§10/§4c); escalate-to-operator is the base case. The review tree bottoms out at human authority
   exactly for high-stakes / low-confidence / novel promotions.

**Tension — review adds latency + cost** (a model call per promotion); don't throttle the
shared-knowledge flywheel with a review bottleneck. Mitigations (userspace policy): (a) **scale rigor
to scope + stakes** — private = none, team = lightweight peer, org-public = full, effect-bearing =
heaviest; (b) **batch low-stakes promotions** to a periodic reviewer. Rigor proportional to blast
radius, not uniform (same instinct as not running the full fleet gate on every trivial change).

### 9h. The promotion pipeline IS the memory-store reducer loop (decided)
The memory store is a session (§4b), so its reducer loop **is** the promotion pipeline — no separate
orchestrator. `propose-memory` arrives → the store's reducer runs the gate, pulls reviewers, folds
verdicts, flips the pointer. Governance isn't bolted on; it's the store's behavior. But split the work
via the supervision tree (§6) so the single-writer store doesn't serialize/bloat:
- **Memory-store session** owns only the *thin authoritative* facts — current pointers, `set`/retract
  events. On `propose-memory` its reducer **spawns a promotion-pipeline child** and stays out of the
  way.
- **Promotion-pipeline child** does the noisy work: runs the held-out gate (effects), spawns *reviewer*
  grandchildren, folds verdicts, runs the revision loop with the distiller. Resolves →
  `child-completed(approved, memory-hash)` up to the store.
- **Store reducer** folds the completion → performs the ONE authoritative `set` (mutable-name write,
  §4c). That single write is the only thing touching the canonical log.
Result: store stays thin/fast/authoritative; messy parallel review lives in disposable child sessions
(migrate/sandbox/audit/discard as units); multiple promotions run concurrently instead of queueing.
The **gardener** (staleness retraction + auto-trip monitor §9f) is another attached session — a
self-rearming cron reducer (§9c) that wakes on a timer, queries recent outcomes, retracts stale/tripped
memories.

**Confirmation of minimalism:** the ENTIRE memory subsystem — store, distiller, promotion pipeline,
reviewers, gardener — is nothing but sessions + effects + the supervision tree. Zero kernel code. The
memory design smuggled in no special subsystem; it's pure userspace composition over the primitives.

## 9b. Cadenza's role — lingua franca, kernel agnostic (decided §16.1)

**Cadenza is the lingua franca of the system's *contents*; the kernel is agnostic to it.** These
aren't in tension — the decoupling is the point: the language can evolve without redeploying
infrastructure. The kernel depends on `wasmtime`, never on Cadenza.

- **The compiler is a PEER, not a dependency.** It's just another content-addressed program installed
  in the store; **multiple versions coexist**; a session pins the exact compiler hash it was built
  with. The kernel never links it — agents *use* it as a published program. Toolchain-level you need
  *a* compiler to build reducers into wasm, but that's build-time producing content-addressed blobs,
  not a runtime edge.
- **Bootstrap:** `rcdzc` (Rust compiler) is the seed that compiles the first reducers + the first
  Cadenza-in-Cadenza compiler; once `compiler-ml` self-hosts, IT becomes a published program and
  rcdzc's role shrinks to "produced the genesis blob." (The active self-host work = making the
  compiler a first-class citizen of this system, not external scaffolding.)
- **Why Cadenza fits:** wasm-component-native, immutable, algebraic effects, code-is-data. It exists
  to build this system.

### PRINCIPLE: the effect signature IS the capability manifest (standout)
Because Cadenza has algebraic effects, a program's *type* statically declares its effect row
(`{Http.get, Shell.exec, Db.query}`). **That row IS the set of capabilities required to run it** —
derivable from the type BEFORE the program runs (static, total, no execution/interception).
- Capabilities stop being bolted-on ACLs → they ARE the effect row of the code. Cedar policy = "which
  principals may discharge which effects on which resources."
- **What a program needs, what an agent may do, and what the authorizer checks are the SAME object**
  (an effect row) seen from three sides.
- Request side: read a published program's effect row, diff against held capabilities, `request` the
  shortfall (capability-request is itself an effect, delegating down the attenuating spawn tree §4c).
  "Granted permission to request more permissions" = a meta-capability in that same lattice.
- The **authorizer** (wasm component, userspace) is what *understands* effect rows; the **kernel**
  still routes opaque effect requests. Minimalism preserved.

### Tooling-as-programs + code-is-data
Publishing a Cadenza program as shareable tooling = the same `put → hash` + mutable-name gate (§4c),
no new machinery. Code-is-data adds what prose can't:
- **A distilled memory (§9) can BE a program**, not just advice — a vetted remediation whose effect
  row declares exactly what it touches, so recall = "here's a program for your situation + precisely
  what it needs." (trigger→claim where `claim` is a program.)
- **Refactoring/query tooling operates on programs-as-values** (Ast). Auditing "what tool would've
  helped" can *synthesize* the tool as an Ast; reviewing a proposed tool = statically querying its
  Ast + effect row. Substrate = the landed quote/eval + `Ast` + macro work.

### Cadenza binary format as the wire format — with a hard boundary
Use Cadenza's binary exchange format as the ONE stable, strongly-typed, language-native format for
event payloads, KV values, effect payloads (vs. bolting on protobuf/JSON) — strong typing + rich
refactoring/query tooling built in, not bolted on.
- **BOUNDARY (must hold or the spine erodes):** the kernel + blob store treat payloads as **opaque
  bytes + hashes**. "These bytes are Cadenza-binary" is a *userspace agreement enforced at the edges*
  (reducers, authorizer) — NOT kernel knowledge. If the kernel parses Cadenza values to route, it's
  recoupled. Rule: **Cadenza binary = lingua franca of contents; hashes + a tiny envelope = lingua
  franca of the kernel.** Kernel sees `event{cause, producer, sig, payload=hash}`; what's behind the
  hash is Cadenza's business.
- **Format is stable — it's binary s-expr.** The wire format is just a self-describing binary
  encoding of s-expr STRUCTURE (atoms/lists/nesting); **semantics live entirely in the interpreter.**
  So the format layer basically never breaks: any version parses any version's bytes into a value.
  What drifts is *schema* (expected shape of a given s-expr), NOT format → this is a schema-evolution
  DISCIPLINE, not a format story: **additive-only fields, never repurpose a name, tolerant readers**
  (ignore unknown trailing fields, default missing, handle-or-reject unknown variants — the classic
  open-records/open-sums posture, forgiving in a way rigid binary schemas are not). §16.12 downgraded
  accordingly.

### Content-type tag in the ENVELOPE (decided)
The kernel needs an opaque payload, but a reducer must know what it's looking at *before committing to
decode* — and ideally without fetching the blob at all. Add `content_type` to the envelope alongside
the hash: `event{cause, producer, sig, content_type, payload=hash}`.
- **In the envelope, NOT the payload** — the kernel carries it as an opaque *label* (a routing key it
  copies but never interprets), preserving "kernel knows nothing about payloads." Exactly like HTTP
  `Content-Type`: transport carries it, only endpoints interpret it.
- **Payoff:** a reducer can dispatch / filter / skip / route-to-authorizer **without dereferencing the
  hash** ("don't care about `transcript-chunk`, skip"; "this is a `capability-request`, auth path").
  Scan cheap typed envelopes, fetch only the blobs you need — big win for large payloads. Authorizer
  can reason by type before touching contents.
- **Structured tag, not a mimetype string:** `family` + `version` (e.g. `model-request` / `v2`) so
  tolerant readers match on family and range-check version. Also lets a v1 reducer handle a
  `model-request/v2` honestly ("known family, unknown version → defer/reject") vs. decoding garbage.
- **BOUNDARY — the tag is a HINT, NOT authoritative.** A malicious producer can mislabel. Fine for
  routing/filtering/skipping; but a reducer that *acts* on a payload must still validate the decoded
  shape, and trust comes from signature+producer (§4c/§10), never the tag. Tag = "what I claim this is,
  for cheap dispatch"; signature + decode-validation = "what it actually is, for trust."

## 9c. Time — timers / crons / clock (decided)

**Rule (= §3 principle 3 applied to time): the reducer NEVER reads wall-clock. Time is not an input
to the fold; it's an event in the log.** A reducer calling `now()` mid-fold poisons replay (same
event folds differently tomorrow) — forbidden. The clock is an *executor*, exactly like http/shell.

### `now` is an effect; the kernel timer primitive is RELATIVE-ONLY (decided)
Two effects cover ALL of time; the kernel never understands absolute wall-clock:
- **`now` effect** → kernel injects `time-result(t)`, frozen in the log = how a reducer *learns* the
  time, deterministically. **Capability-gated:** a sandboxed reducer can be DENIED the clock entirely
  (real security property — some reducers shouldn't know the time).
- **`fire_after(duration)` — the ONLY timer primitive, purely relative.** Kernel needs only a
  *monotonic* elapsed sense to fire it, never time-of-day. Fire → kernel injects
  `timer-fired(id, fired_at)`; reducer folds it like any inbound message (`on timer-fired(id) → X`).
- **Absolute deadlines + crons are USERSPACE composition:** to fire at 9am, reducer does `now → t`,
  computes `delay = 9am − t`, then `fire_after(delay)` — anchored to a *recorded* time-query event, so
  fully deterministic. Cron reducer reads recorded `fired_at` to compute next occurrence. The kernel
  sheds absolute-wall-clock scheduling entirely: one relative timer verb + one `now` verb, both
  recorded effects. (= DES sleep/spawn model; watch/timer/sleep are one shape, §4b.)

### Determinism clincher: fired timestamp frozen at fire-time
The timer *fires* in real wall-clock (nondeterministic — scheduler/machine/load). But the kernel
**records `fired_at` into the event** → frozen history thereafter.
- **Live:** timer wheel is real, tokio-driven, nondeterministic.
- **Replay:** timer wheel is IRRELEVANT — you never re-arm/re-fire; you replay the recorded
  `timer-fired(fired_at=…)` straight from the log. Nondeterminism at the edge (kernel timer service),
  determinism in the fold (only ever sees recorded fires).

### Crons = self-rescheduling reducer, NOT a kernel feature
A cron is a reducer that, on each fire, does its work AND arms the next timer:
```
on timer-fired(id="cron"):
    <do scheduled work; emit effects>
    set-timer(id="cron", fire_at = next_occurrence_after(fired_at))
```
`next_occurrence` is a **pure function of the recorded `fired_at`** → deterministic + replayable.
Kernel has zero cron knowledge. (This IS the current 30-min fleet tick, expressed natively: a
self-rearming reducer instead of an external scheduler poking the session.) The old `schedule.rs`
overflow fix (saturating `occurrences_due`) carries over as a **pure-fold totality requirement** on
`next_occurrence`: total for any `fired_at`, no panic — the "can't-brick" discipline, now a property
of a userspace reducer.

### Two locked consequences
1. **Timer service = the ONE place the kernel holds "pending future" state** (everything else is
   reactive). Armed timers are LOGGED (`set-timer` is in S's log); on crash/restart the kernel
   rebuilds its wheel by scanning armed-but-unfired timers. **At-most-once fire even across restart**
   — fire is idempotent on `(session, timer-id)` and recorded (= old at-most-once cursor invariant,
   specialized to time).
2. **Migration + timers:** the arm lives in the session's log, so a migrated session's timer
   obligation moves with it — destination host reconstructs pending timers from the received log. The
   wheel is a rebuildable CACHE; the durable truth is the log (same pattern as KV).

### Locked subtlety — no firing during replay/catch-up
Because the only primitive is relative `fire_after` and fires are recorded, replay reads recorded
fires and never runs a clock. State loudly: **a timer armed during replay/catch-up does NOT fire
"now"**; catch-up folds see OLD recorded fires; only once caught up to live does a newly-armed timer
engage the real wheel. Don't write a reducer that assumes a catch-up-armed timer fires immediately.

## 9d. Reactive execution & anti-stuck (decided)

**Append-wakes-the-reducer is the whole scheduling model. There is no polling loop, anywhere.** The
reducer is a per-event fold that runs and returns — **there is no long-running loop that can hang**,
so "stuck" is not a state a reducer can be *in*. A session can only be waiting in one of two ways,
both with clean reactive escapes:

1. **Waiting on an outstanding effect** (a hanging model call, a shell command that won't return).
   → **Every dispatched effect carries a deadline** (just a `fire_after` the kernel arms at dispatch).
   No result by the deadline → kernel injects `effect-timed-out(id)` → reducer wakes to recover. A
   hung model call becomes a normal timeout event, not a wedge. (Same timer machinery, applied to
   effects.)
2. **Idle, waiting on external input.** → **Any message wakes it.** Send it something → the append
   runs the reducer → it responds. Idle is free + correct (consumes nothing, instantly revivable).

**Why this fixes today's pain:** Claude Code "gets stuck" because it's a single long-running turn that
can wedge mid-stream, and nothing external can nudge it without the hand-rolled wedge triad
(CPU+heartbeat+wrapper). In the fold model there's no mid-stream to wedge — the reducer already
returned; the session is just waiting on a *named* thing that has a deadline or a wake. Stuck-detection
becomes the structural fact "outstanding effect past its deadline?" (§4b tier-2); recovery is an
injected `effect-timed-out` event, NOT a kill-and-restart. The two elegant unsticking mechanisms:
**deadlines on every effect (auto-timeout)** + **message-delivery-wakes-the-reducer (external nudge)**
— both fall out of "append schedules a fold" + "timers are relative effects." Concierge responsiveness,
mid-task nudges that inject content into the agent stream, and watchdog-free recovery all come from
this.

## 9i. Transcript rewriting — LOG ≠ CONTEXT (decided)

**The load-bearing distinction (was implicit, now explicit):** there are TWO artifacts, not one.
1. **Event log** — immutable, signed, content-addressed record of what ACTUALLY happened (agent emitted
   a broken program, got a type error, fixed it). NEVER rewritten (rewriting kills replay/determinism/
   audit).
2. **Model context** — the prompt assembled for the NEXT inference. **NOT the log** — a *projection
   derived from* the log by the reducer.
**The model never sees the log; it sees a context the reducer builds.** So "rewrite the transcript the
model sees" touches the log NOT AT ALL — it changes how the reducer projects log → prompt. Log keeps the
messy truth; context shows the clean version; both true because they're different objects. (= the same
"KV/context is a derived projection of the log" already in §4, now applied to the model prompt.)

### The rewind, concretely
1. Model emits program v1 → reducer records `model-output(v1)`.
2. Reducer VALIDATES via a `compile(v1)` effect → `compile-result(type-error)`. **The system knows
   AUTHORITATIVELY that v1 is invalid — a compiler verdict, not a guess.** Recorded.
3. Feed error back → v2 → ... → `compile-result(ok)` for vN. Full struggle in the log forever.
4. **Projection choice:** building the NEXT turn's context, the reducer projects a CLEAN transcript
   ("you wrote vN") instead of the v1→error→v2→error… chain. Model proceeds from tidy history —
   genuinely doesn't know it stumbled.

### Why it's SAFE here specifically — the validity oracle
Rewriting a model's context is normally dangerous (fabricating undetectable history). Safe HERE because
Cadenza gives a **ground-truth validity oracle**: you never rewrite to a heuristic guess, only to a
program the COMPILER PROVED well-typed/valid. Rewrite is always "replace the struggle with the *verified*
outcome"; the info dropped (the errors) is info the model itself would discard once fixed. Without the
oracle = sketchy prompt-hacking; WITH it = "compress the transcript to its verified fixpoint."

### Mechanism — context is a REDUCTION over the log; rewriting is a reduction rule
First-class concept: **transcript reductions** applied when projecting log → context:
- *struggle-elision:* `model-output → compile-error → … → model-output(valid)` collapses to the valid
  output.
- *tool-noise-elision:* failed-then-retried tool call collapses to the success.
- *summarization:* an old resolved sub-episode collapses to a one-line outcome.
Pure functions of the immutable log — **same shape as memory distillation (§9) and stream distillation
(§11b)** ("faithful high-volume record → compressed actionable view"). Transcript rewriting IS
context-distillation — the same primitive a third time (the sign it belongs).

### Three payoffs from keeping the log intact (vs. naively editing the message array)
1. **Deterministic + replayable:** reduction rules are pure functions of the log → replay reconstructs
   the SAME clean context every time. Mutating a message array would make replay impossible (input
   destroyed).
2. **Auditable — you can prove what you hid:** context shows the clean version; the log shows the 4 real
   attempts. Always answerable: "what did we actually show the model, and what actually happened." A
   mutated transcript can answer neither.
3. **Feeds the distiller:** the struggle elided FROM the model is GOLD for memory (§9) — "stumbled 4× on
   this type error before fixing" → a memory so agents don't stumble again. Hide the mess from the model
   AND mine it for improvement — both.

### Two honest constraints
- **Don't erase load-bearing context.** Sometimes the struggle is info the model needs later ("already
  tried X, doesn't work — don't resuggest"). Elide *syntactic/type* thrash (pure noise once fixed)
  freely; be careful eliding *semantic* dead-ends. The oracle says a program COMPILES, not that the
  APPROACH was right — "compiles now" is safe to rewind to; "this whole strategy was abandoned" may need
  to stay.
- **Context ≠ KV — keep distinct.** The model context is one projection; the reducer's own working KV is
  another. Rewriting the model's view must NOT corrupt the reducer's own memory of what really happened
  (needed to feed the distiller). Two projections of one log, two consumers; don't collapse them.

## 10. Multi-operator → trust root (decide early, build late)

Multi-*tenant-me* is a policy problem; multi-*operator* is a **trust** problem.

- **Events are signed.** Every event carries a signature over `(content-hash, cause,
  producer-identity)`. Content-addressing → add *provenance*-addressing.
- **Authorizer is per-operator-configurable**, and its *decision* is a logged event
  (`authz-granted(request-hash, policy-version)`) — audit can replay not just what happened but
  under which policy it was permitted, across operator boundaries.
- **Outposts have identities + scoped, expiring capabilities**, NOT standing access. (Negative
  lesson: the ada-2FA / fleet-wide credential-starvation freeze = standing creds that expire
  together. Fix = scoped, per-effect, short-lived capability tokens issued by the kernel; these
  federate.)

**Decide now even though we verify later:** the event envelope carries `signature` +
`producer-identity` + `cause` from day one (optional/unverified in P0). Retrofitting provenance into
a log format later is agony; three optional fields cost nothing now.

## 11. Integration — no forced adoption (MCP both directions)

Team uses claude / codex / kiro. Don't replace — expose benefits.
- **As MCP server:** expose federated tools, virtualized resources, and the audit log to any
  MCP-speaking agent. They connect today, unchanged, and get federation + audit + policy for free.
- **As MCP client:** invoke external tools as effects. An external agent's tool call becomes just
  another event source.

### 11a. Slack bridge = a broker role over a different protocol (decided)
Slack isn't special — it's the **broker role (§12.0) speaking Slack instead of MCP.** MCP-broker
bridges local *agents* in; Slack-broker bridges a *human* in. Same shape: external messages → events;
events → messages out.
- **Ingress:** a Slack message arrives at a Slack-broker node. Broker's job is NARROW + dumb —
  authenticate which Slack user sent it (Slack signing secret / request verification), map to an
  operator identity, emit `operator-request(from=operator-id, text, thread-ref)` into the appropriate
  session's log. It does NOT interpret "fix this" — which session (standing per-operator concierge vs.
  freshly spawned task session) is a *routing-reducer* decision, not broker logic.
- **Egress:** a session emits `slack.post(thread-ref, text)` → routed to a Slack-broker node → posts.
  **A Slack thread maps to a session**, so the conversation IS the session's inbound/outbound event
  stream. Concierge-feels-dead is fixed here: the moment a Slack message folds in, the session runs and
  replies immediately (delivery-wakes-the-reducer, §9d).
- **CRITICAL boundary — the broker authenticates the CHANNEL, not the AUTHORITY.** It establishes
  "this genuinely came from Slack-user-U" and STOPS. It does NOT grant U's permissions to the resulting
  session — that's delegation (§12f). Keeping this crisp is what stops "anyone who can post in a channel
  can act as the operator."

## 11b. Ingress patterns — external world → events (decided)

Every external input is a **broker** (§12.0) turning world-happenings into signed events delivered into
a session's log. Slack/GitHub/Zoom/email/PagerDuty = same role, different protocol. A broker's whole
job: **authenticate the source, translate the happening into a signed event, deliver into a session.**
So "how do we react to X" is NEVER a kernel question — it's "write a broker for X's protocol + a
routing reducer that picks the session." Three ingress patterns cover everything (naming them makes
future integrations trivial):
- **Request/response** — Slack message, MCP call. Someone asks, a session answers.
- **Fire-and-observe** — GitHub webhook, GHA failure, alarm. The world notifies; a session reacts. No
  reply expected, often triggers action.
- **Continuous stream** — Zoom transcript, log tail, market data. Unbounded feed a session must
  *distill*, not just record. (New sub-problem — see below.)

### GitHub events (fire-and-observe)
Webhook (issue opened / workflow failed / review requested) → **GitHub-broker node** verifies HMAC
signature → emits `github-event(kind, repo, run-id, payload=hash)` into a session (standing per-repo
watcher OR spawn a triage session — routing-reducer choice). Watcher reducer folds + ACTS: read logs
(`github.query`), check out repo (resource §13), propose a fix (spawn worker w/ delegation §12f). The
reaction is a normal effect-driven agent session, kicked off by an inbound event instead of a message.
**Two locks:**
1. **Acting on a GitHub event is DELEGATION (§12f), never ambient.** The event arriving does NOT grant
   write access; reading needs the watcher's read cap, pushing a fix needs the on-behalf-of chain. A
   webhook is *notification/intent*, NEVER *authority*. Matters MORE than Slack because webhooks are
   attacker-reachable — "the event happened" must never imply "the action is permitted."
2. **Self-improvement loop closes here:** GHA failure → triage → propose fix → *outcome* feeds the
   distiller (§9). Reacting to CI + learning from it = the self-improve vision; GitHub events are among
   the richest triggers. (Literally what pr-sync does by hand today.)

### Continuous streams (Zoom transcript, log tail) — TWO-STAGE broker (new pattern)
A naive "every transcript line = an event" firehose is wrong on 3 axes: **volume/noise** (mostly not
actionable → floods log, thrashes reducer, violates §9d spirit), **determinism** (raw stream must enter
as recorded events but the fold shouldn't re-reason per partial line), **segmentation** (an actionable
unit spans multiple utterances, unknown until enough context arrives). Design = **distillation at the
edge**, generalizes to any high-volume stream:
1. **Stage 1 — capture broker (dumb/cheap/faithful):** ingest raw stream into a **stream-buffer
   session** (blob-backed rolling log), no reasoning. The faithful record; **compact aggressively**
   (§8 — raw transcript is low-value long-term).
2. **Stage 2 — segmenter/distiller (smart):** a reducer watches the buffer, does **debounced/windowed**
   distillation (waits for a pause / topic-shift / periodic `fire_after` tick), asks "complete
   actionable intent in this window?", and on a hit emits a *distilled* `action-request(intent,
   transcript-ref, participants)` into an acting session. Raw firehose stays in the buffer; only
   distilled intents reach the actionable log (with a `cause` pointer to the transcript span).
**Two hard sub-questions the stream case FORCES:**
- **Overheard-intent authorization:** transcript says "deploy to prod?" → the system must NOT act on
  the utterance. A meeting is a low-integrity, multi-party, spoofable channel — the confused-deputy
  problem (§12f) at its most dangerous. **HARD RULE: stream-derived intent is PROPOSAL-ONLY; it can
  never be the root of a delegation chain.** At most it spawns a draft / opens a suggestion / `ask`s a
  participant to confirm via a higher-integrity authenticated channel. Whose authority would it act
  under? Nobody's, until a real operator confirms.
- **Consent/privacy:** tapping a call into an autonomous actor is a consent+recording minefield
  (legal + trust). Treat "may this call be tailed" as an explicit, logged, per-call authorization;
  surface to participants that an agent is listening. The audit log (who authorized the tap, what it
  acted on) is what makes it defensible.

## 12. Nodes, roles & outposts (federated mesh)

### 12.0. Terminology — ONE binary everywhere, roles are capabilities not builds (decided)
The system is a **mesh of identical kernels.** "Central vs. edge" is a *topology + enrollment*
distinction, NOT a code distinction.
- **Node** = one running instance of the kernel binary. The unit of deployment. **Same binary
  everywhere.**
- **Roles** a node plays (enrolled/configured capabilities, not separate builds):
  - **executor** — runs dispatched effects locally (WASI).
  - **host** — runs sessions (reducers) locally.
  - **broker** — bridges local external agents (claude/kiro/cadenza) into the federation via MCP (§11).
  - **authority** — holds authoritative logs / global store / session registry.
- **Hub / core** = node(s) in the authority role (the "centralized kernel").
- **Outpost** = a node in an EDGE POSITION (laptop / random EC2): executor + host + broker, but NOT
  authority. A deployment *position*, not a different binary. (= the fleet's bare-hub shape: one
  authoritative log, many workers.)

### 12a. Node trust model — a node is NEVER a principal (decided; RECONCILED with §14a central folding)
> **CORRECTION (adversarial review, soundness-F2 / security-F2):** the earlier text here said a laptop
> HOSTS its session (log/KV/**signing keys** on the edge node). That CONTRADICTS §14a (only the hub folds;
> nodes hold NO session state). §14a wins. Sessions fold at the hub; **session signing keys NEVER live on
> edge nodes.** The text below is rewritten to that reading — the old "edge-tamper of hosted session"
> threat model was analyzing an architecture we don't build.

Authority attaches to two things, **never to the node** — the node is *plumbing in both directions*:
1. **Sessions ORIGINATE at the hub (pull).** A laptop agent's *session folds at the hub*; the laptop runs
   only a **broker role** (§11a) that relays the human/agent's authenticated *intent* to the hub, where the
   session's reducer folds it and authorizes effects against **the session's** capabilities. The laptop
   never holds the session's key, log, or KV.
2. **Scoped tokens EXECUTE (receive).** When the hub dispatches an effect TO a node's executor role, it
   carries a single-effect kernel-minted token; the node-qua-executor holds zero standing authority.
An idle node with no dispatched tokens can do NOTHING.

**Corrected edge-compromise blast radius (security-F2).** With keys at the hub, a compromised edge node
CANNOT forge session-signed effects. What it CAN still do: (a) **lie about effect results** it executes
(return a fabricated `effect-result` — mitigated by result signing + cross-checking + not trusting a
single executor for high-consequence effects); (b) **exfiltrate any brokered credential currently in its
hands** (§12f/F6 — mitigate by proxying credentialed calls so the raw token never lands on the executor);
(c) **abuse its broker role** to inject *intent* (bounded by "channel ≠ authority," §11a — the injected
intent still can't exceed the target session's pre-delegated authority). **The remaining hard problem:
authenticating a laptop-broker's relayed intent to the hub WITHOUT conferring authority** — the same
"channel not authority" problem as Slack, now for a laptop; currently UNDESIGNED (→ §16c).

**The outpost's log is OPERATIONAL, not authoritative** — records `dispatched(effect-id, token,
deadline)`, `effect-result(id, output-hash, sig)`, `heartbeat`, `inventory`; useful for debug/restart,
but nobody trusts it for authority (authority = kernel authz §10 + signatures; authoritative result
lives in the requesting session's log).

### 12b. Node identity / PKI (decided)
Slots into the SAME signed-events + `producer-identity` + namespace-trust-root PKI as §10/§4c — not a
separate system.
- **Keypair = identity.** Node signs everything it emits (results, heartbeats) → non-repudiable,
  attributable.
- **Enrollment (the hard part — bootstrapping trust):** a new node proves it's authorized to join and
  gets a signed cert binding its key to an identity in an operator's namespace + a capability ceiling +
  permitted roles. Options by environment trust: (a) **join token** (one-time secret — laptops);
  (b) **attestation-backed** — EC2 instance identity doc (AWS-signed) proves "really instance i-xxx in
  your account." NATURAL FIT: the IMDS-signed-identity → scoped-role primitive already built (the
  `DevEnvInstanceProfile` off-ramp + Bedrock cred-broker: prove environment identity → scoped
  credential). Node enrollment = that, generalized. (c) hardware attestation (TPM/Nitro) — overkill now.
- **Cert binds key → identity AND capability CEILING AND permitted roles:** "node-X may execute effects
  up to ceiling C, in roles {executor,host,broker}, for operator O." Max a node can EVER do is bounded
  at enrollment (a laptop certified for sandboxed `shell.exec`, never `prod.db.query`).

### 12c. Three-way authorization (the whole model in one rule)
An effect executes on a node only if ALL THREE hold — compromise any one, the other two contain it:
1. **Requesting session** holds the capability — and (SECURITY-F1 correction) that capability is
   **resource-scoped**: not just `Http.get` but `Http.get(host ∈ allow-list)`, not `Github.push` but
   `Github.push(repo=X)`. Checked against the **resolved runtime argument**, not just the effect kind.
   — *Is the asker allowed to do THIS to THIS target?*
2. **Executing node** is enrolled with a ceiling admitting the effect. — *May this executor do this
   kind of thing at all?*
3. **Per-effect token** (kernel-minted, scoped, short-lived, signed) authorizes THIS dispatch. — *Is
   this specific act, now, blessed?*
Only the kernel mints (3), only when (1)+(2) check out. Rogue session can't exceed grants; rogue node
can't exceed ceiling or forge a token; leaked token is single-use/expiring/single-effect.
> **CRITICAL (security-F1):** the effect ROW alone is NOT the capability manifest — it names effect
> *kinds*, not *targets*. Resource scope + argument-provenance live in runtime args the static type
> doesn't carry. Capabilities MUST carry resource predicates checked against resolved arguments; §9b's
> "effect-row IS the manifest" is necessary-but-NOT-sufficient. See §16c-SEC.

### 12d. Locked constraints
- **Node/executor SELECTION is part of authorization, not just load-balancing.** Match effect
  *sensitivity* to node *trust-tier + environment*: `prod.db.query` routes ONLY to prod-enrolled nodes,
  never a random laptop (defense in depth even though tokens would stop it). Policy = userspace
  authorizer + routing reducer. (NOTE: the "where a sensitive session may be HOSTED" clause is DELETED —
  per §12a-corrected/§14a, sessions fold at the hub, not on edge nodes; only EFFECT EXECUTION is routed.)
- **Short-lived certs, auto-renewed via re-attestation — NOT standing secrets.** Cautionary tale = the
  ada-freeze (standing creds expiring fleet-wide = mass failure, §10). Stale/decommissioned nodes age
  out on their own; revocation is a positive act (retract enrollment = mutable-name write §4c).

### 12e. One binary + bootstrap + updates (decided)
**Why one binary is deeper than convenience:** the kernel binary is the ONLY expensive-to-update part
(it's the wasm host — can't hot-swap itself); everything in a *program* is cheap-to-update
(content-addressed, pushed, hot-swapped = §7 self-mod / §4c pointer-resolution). **Minimalism's
operational payoff: the more you push out of the binary into programs, the less you ever redeploy the
binary.** That — not elegance — is the real reason to keep the kernel tiny.

**Bootstrap flow (fresh node from scratch):**
1. Minimal kernel binary starts — knows nothing (log + store + scheduler + wasmtime + enroll/fetch
   primitives only).
2. **Enroll** (§12b) — prove identity, receive cert + ceiling + permitted roles.
3. **Resolve the bootstrap-program pointer** (authorized, from the hub), fetch by hash.
4. **Run the bootstrap program** — specializes the generic node into what it's enrolled to be
   (executor/host/broker): wires local WASI, the MCP endpoint, etc.
A hub node and a laptop node run the SAME binary; bootstrap-program + enrolled role/ceiling is the only
difference.

**Two update tiers:**
- **Program updates (common, cheap):** push a new content-addressed program; a node watching an
  authorized `node-bootstrap` pointer (§4c mutable name) fetches by hash + swaps. Secured by the
  mutable-name authority model (only an authorized publisher repoints; node verifies signature; same
  anti-hijack story as the compiler pointer). Covers ~all behavior changes. Fleet update = repoint one
  pointer; nodes fetch+swap on a pushed `swap` signal (delivery-wakes-the-reducer §9d — no polling).
- **Binary updates (rare, heavy):** actual kernel replacement — download signed binary, verify,
  restart. The one non-hot-swap thing. Kept rare BY keeping the binary minimal.

### 12f. On-behalf-of & credentials (decided)

**Acting on behalf of an operator** ("Cameron says 'go fix this' in Slack → system acts as Cameron").
Core principle: **a session NEVER inherits an operator's authority by being asked. Authority is a
delegated, attenuated, auditable capability grant; the delegation is itself a logged, authorized
event.** Two independent things — conflating them = the classic confused-deputy bug:
- **Intent** ("fix bug 123") — authenticated at ingress (§11a), ephemeral, per-request. Says WHAT.
- **Authority** (may-write-repo-X-as-Cameron) — a STANDING, scoped, revocable **on-behalf-of policy**
  Cameron granted the system ahead of time; invoked + attenuated per task. Says HOW MUCH.
A Slack message can only *invoke* authority Cameron pre-delegated, never *expand* it.

Flow for "go fix this":
1. Slack-broker emits `operator-request(from=cameron, "fix bug 123")` — authenticated origin, ZERO
   authority attached.
2. Concierge/router session folds it, spawns a worker with a **delegation**: "act on behalf of Cameron,
   scoped `{repo-X write, ci read}`, expires 1h, purpose=bug-123."
3. Delegation **can't exceed what the delegator holds** (attenuating spawn tree §4c/§9b) AND **can't
   exceed Cameron's standing on-behalf-of policy.** If the job needs authority Cameron never delegated
   for agent use (e.g. prod), the worker CANNOT silently get it — it must `request` escalation → routes
   back to Cameron as explicit approval (`ask` / human-in-loop §9d/§9g). = confused-deputy defense.
4. Every step logged with `cause`: action → delegation → `operator-request` → authenticated Slack user.
   **Full provenance from a repo write back to "Cameron said so in #eng at 2:03pm."** This is what makes
   act-on-behalf-of safe rather than terrifying.

**Credentials — the hard core.**
- **Are credentials owned by the node? NO** (critical). A node holding standing creds (AWS keys, GH
  tokens, prod DB creds on a laptop) means node-compromise = credential-theft = the whole capability
  model bypassed = the ambient-authority anti-pattern = the ada-freeze lesson generalized (standing
  creds = the failure mode).
- **Credentials are brokered, just-in-time, scoped to the authorized effect — never stored at the
  edge.** Template = the Bedrock cred-broker already built (IMDS-signed identity → API-GW → scoped
  short-lived role), generalized:
  - A **credential broker** (a role, on a trusted hub-adjacent node) holds the *ability to mint/fetch*
    real creds, but hands out only **narrow, short-lived, single-purpose** ones.
  - An authorized effect needing a real credential (`github.push` needs a GH token) triggers the broker
    to mint a token scoped to EXACTLY that operation (this repo, push only, 5-min TTL), bound to the
    delegation chain. The edge node receives the ephemeral cred WITH the dispatched effect, uses it, it
    expires. **A compromised idle node has nothing to steal.**
- **"How can it trust the requested operation was valid?"** The broker mints ONLY when it can verify the
  full authorization chain — which it can, because everything is signed + causally linked:
  1. effect signed by a session (§10); 2. session presents its **delegation**, chaining signed-link by
  signed-link back to Cameron's standing grant (§4c attenuating chain); 3. three-way authz (§12c) passed;
  4. broker verifies this effect, from this session, under this chain rooted in Cameron's grant, is in
  scope → mints a cred scoped to NO MORE than the effect's declared effect-row ∩ the delegation ceiling.
  The broker trusts neither the node nor the request TEXT — it trusts the **cryptographic authorization
  chain** it can independently verify. "Is this valid?" = "does an unbroken, in-scope, unexpired signed
  delegation chain from a real operator justify EXACTLY this effect?" — checkable, not a judgment call.
  **Confused-deputy solved by construction:** can't use a cred beyond what it was minted for (scope),
  can't retain it (TTL), can't obtain one without a verifiable chain (provenance).

**Hard part to flag — the broker is the trust-boundary translator + highest-value target.** External
systems (GitHub/AWS/Slack) understand THEIR tokens, not our delegation chains. The broker converts
"verified internal chain" → "real external credential the outside world accepts." Real secrets MUST
live here (standing power to mint GH/AWS creds) → **the broker is the highest-value target; harden it
most:** minimal, attested, heavily audited, and ideally itself brokering to cloud-native short-lived-cred
services (STS AssumeRole, GitHub App installation tokens) rather than holding long-lived secrets. Design
goal: **push the standing-secret surface to ONE hardened/attested/audited place; everywhere else holds
only ephemeral scoped tokens.**

## 13. Resource virtualization — decouple fetch/privilege from compute (decided, abstract)

**Motivating pain:** today all agents run on ONE box, fusing credentials + source + dependency closure
+ CPU. When compute (builds/tests) saturates, the WHOLE thing — including the privileged bits — falls
over; concurrency is clamped as an unloved compromise. Fix = the very first goal of this system
(§0/the vision): decouple *processing* from the *compute/storage resources* an agent uses.

### Core reframe — a resource is (authority-to-fetch) + (materialization) + (compute), three things
currently fused into one place. Separate them:
1. **Authority to fetch** — credentials / special-environment access. Scarce, privileged, must stay on
   a **trusted node**.
2. **Materialization** — the actual bytes: source tree + resolved dependency closure. Content-
   ADDRESSABLE once fetched.
3. **Compute over it** — build/test. CPU/mem-hungry but needs ZERO privilege once it has the bytes.

Architecture falls out: a **privileged, low-compute FETCH role** produces a content-addressed
workspace; **unprivileged, elastic, high-compute BUILD/EXECUTOR roles** consume it. Fetch node holds/
brokers credentials (§12f); compute nodes NEVER see a credential — handed a materialized `workspace@hash`,
run the build, return results. (= the §12 node/role split, applied to resources.)

### Why this solves saturation
Compute becomes **horizontally elastic + disposable**: each build/test on a fresh sandboxed compute
node bound to a workspace hash; torn down after. No shared host to saturate — N concurrent builds =
N isolated nodes. Concurrency stops being a global fragility-clamp and becomes an **elasticity dial**
("how many compute nodes am I willing to pay for"). Compute nodes are unprivileged + stateless
(workspace in, results out) → safe to run anywhere, safe to kill.

### Binding time — RESOLVED (the split resolves the old open question)
The two halves bind DIFFERENTLY:
- **Privilege binds EARLY** to a trusted fetch node — privilege isn't fungible; a credentialed fetch
  MUST route to a node enrolled with that environment's access (§12d: match resource-sensitivity to
  node trust-tier). Can't late-bind "fetch from a privileged environment" to a random public node.
- **Materialized bytes bind LATE** — `workspace@hash` is location-independent; ANY compute node pulls
  it by hash. This is what buys elasticity + migration (a build runs wherever there's capacity).
So: **privilege early-bound to a trusted fetch node; content late-bound to any compute node.** The
split point = where the resource stops needing privilege and becomes pure content.

### Two-phase resource request
1. **Materialize (privileged, fetch node):** authorize fetch vs. session delegation + on-behalf-of
   credential (§12f); fetch source AND resolve the dependency closure (may be TWO separate credentialed
   steps — abstractly "source + its resolved dependency set"); produce content-addressed `workspace@hash`.
   Credential broker mints JIT, uses it, resulting bytes are **decredentialed** (just content).
2. **Compute (unprivileged, elastic node):** dispatch build/test + `workspace@hash`; node pulls
   workspace by hash, runs, returns content-addressed results. No credential ever reaches it.
**The workspace hash is the clean handoff boundary:** before it = privileged+scarce; after it =
fungible+elastic. And **cacheable** — same source+closure → shared workspace by hash (dependency-set
resolution is expensive; content-address it → resolve once, reuse across many builds).

### OPEN — materialized-workspace transport cost (§16.14)
A resolved dependency closure can be LARGE; shipping `workspace@hash` fetch→compute isn't free.
Content-addressing helps (dedup / cache / incremental — a compute node likely has 90% of the closure
from a prior build, pulls only the delta), but this is the real engineering cost of the split and
determines whether elastic compute actually beats the single box. Named as the open cost.

> **NOTE:** internal-environment specifics (which privileged systems the fetch role talks to, what the
> dependency closure concretely is, dev-desktop-as-fetch-node tradeoffs) are deliberately kept OUT of
> this public doc per operator directive; they live in local memory
> `[[agent-harness-internal-resource-federation-context]]`. The abstract model above is complete on
> its own — it accommodates such environments via trust-tier matching without depending on their
> details.

### 13a. Reducing environment churn — warm state = a cache of content-addressed layers (decided)
**Gap in the base model:** it content-addresses *source + deps* but the expensive warm state is the
**build outputs** (compiled dep closure, `target/`, incremental artifacts). "Fungible = stateless =
cold every build" would make elastic compute SLOWER than the single box (re-download + recompile every
dep). **Reframe that dissolves pinned-vs-fungible:** warmth is NOT machine-local mutable state — it's a
**materialization of content-addressed artifacts any node can reconstruct.** Once warm state is
addressable-by-hash, pinning stops being a correctness requirement and becomes a mere optimization. Two
complementary mechanisms:

**A. Effect memoization — fleet-wide build cache (strongest lever).** A build is a PURE function of
content-addressed inputs: `build(workspace@hash, toolchain@hash, command) → result@hash`. Pure ⇒
memoizable ⇒ output addressable by that input key. Before building, check if ANY node in the fleet
already produced `result@hash` → **fetch the result, don't build** (cold-build cost → download cost).
Fleet-wide, not per-machine: every build warms the GLOBAL cache; 10 agents on the same commit build it
ONCE total. (= Bazel RBE / sccache, but as the effect layer's memoization applied to a build effect —
works for any deterministic effect.) **Precondition = HERMETICITY, and it's free:** memoization is
sound only if output is a function of *declared* inputs (no ambient network/clock/env mid-build) — which
IS the effect-row/capability discipline (§9b). Same discipline that gives determinism gives cache
soundness; a non-hermetic build doesn't just break replay, it poisons the cache.

**B. Soft affinity + warm-cache attach — "looks pinned but stays fungible" (handles INCREMENTAL).**
Memoization handles *identical* rebuilds; the common case is "changed 3 lines, want warm `target/`."
Warm caches are **local materializations of content-addressed layers** (reconstructable, not trapped).
Route a session's builds with **soft affinity** — preferentially to a node already warm for that project
(fast) — but affinity is a HINT, not a pin: if that node is busy/dead or you need N concurrent builds,
**any node reconstructs warmth by pulling layers by hash**, paying only the DELTA (already shares
toolchain + most deps). Machine *feels* pinned (warm = fast) but *is* fungible (warmth rebuildable
anywhere; scheduler can fall back / fan out). Pinning = a violable optimization, exactly what elasticity
needs.

**Layered materialization (makes deltas small; also attacks §16.14 transport cost).** Split warm state
into layers by CHANGE-RATE, each content-addressed separately, shipped only if absent:
- **Toolchain** (rustc/cargo/cadenza) — huge, ~never changes → near-permanent per node, ~never
  re-shipped.
- **Compiled dependency closure** — big, changes only when deps change → shared across every build of
  that dep set, cached fleet-wide. (THE big win — deps dominate cold-build cost.)
- **Source + project artifacts** — churny/small. A warm node pulling a new revision ships ~only the
  source diff; a cold node skips toolchain + ~90% of deps. Overlay at build time (Docker-layer /
  overlay-fs insight applied to build envs: stratify by change-rate, dedup by content, transport
  deltas).

**Honest gap — sub-crate incremental sharing is hard.** Clean huge win = **dep-level caching** (stable,
cleanly hash-addressable, dominates cost — directly kills "N agents each compile the same closure cold,"
the thing that saturated the box). What does NOT cache cleanly across nodes = cargo's fine-grained
**incremental** fingerprints (keyed on details that don't reduce to a stable workspace hash).
Recommendation: dep-level caching = the primary SHARED win; incremental stays a **warm-node-local
affinity optimization** (incremental speed when affinity routes you home; fall back to "dep-cached but
recompile-this-crate" — still >> cold — otherwise). Don't try to make sub-crate incremental artifacts
globally fungible.

## 14. The fleet is the prototype + first workload

The current fleet is a hand-rolled instance of exactly this architecture:
- single-writer trunk = the log; file inboxes = event queues; `pr-sync` = single-writer kernel;
  agents = reducers/outposts.
- Every fleet trap in memory (sync contamination, foreign-commit replay, single-writer bottleneck,
  credential-starvation freeze) is a distributed-log problem already paid for in learning.

→ The fleet both **validates** the model and is the **obvious first real workload** (dogfood by
rebuilding the fleet on the kernel). But the longer-term vision is bigger: **multi-operator** (not
just one person) and **multi-function** (not tied to the Cadenza repo) — agents acting across
multiple repositories, interacting with production systems, querying metrics + data lakes,
investigating problems, proposing changes, iterating, self-improving.

## 14a. Central folding — fold at the hub, execute everywhere (decided; REVERSES earlier DHT lean)

**Decision: only the hub (a small replicated set) FOLDS sessions. Nodes are stateless effect
executors that hold NO session state.** This reverses the earlier home-node/DHT/session-mobility lean —
that machinery solved a problem we don't have.

### The conflation that caused the wrong turn — folding ≠ effect execution
Two different workloads:
- **Folding a reducer** — instantiate a tiny wasm component, read KV, apply one event, emit effect
  requests, write KV. **Cheap, fast, CPU-light, STATEFUL** (needs log + KV).
- **Executing effects** — a build / model call / shell / fetch. **Heavy, slow, STATELESS-per-call**
  (workspace in, result out).
The saturation pain that drove §13 was NEVER folding — it was builds/tests = **effects**, already
distributed to elastic executors (§12/§13). **The heavy work is already fanned out.** Folding was never
the bottleneck, and it *wants* to be near the log/KV. Spreading sessions across nodes bought ~nothing.

### The asymmetry (same seam rediscovered a 4th time)
- **Fold centrally** (hub / small replicated hub set): stateful, cheap, near the log. The "brain."
- **Execute effects everywhere** (elastic executors / outposts / brokers): stateless-per-call, heavy,
  disposable. The "hands."
Nodes hold NO session state → a crashed hand is trivially replaced; the brain persists. = fetch/compute
(§13) + node-is-never-a-principal (§12a) generalized: **state stays central + protected; work fans out +
disposable.** (Same shape as KV-vs-effects, fetch-vs-compute, privileged-fetch-vs-elastic-build — the
repetition is the tell it's the right seam.)

### Resilience: a node crash loses NOTHING
Executor dies mid-effect → that effect times out (§9d) + retries elsewhere; the SESSION never left the
hub → nothing to recover, no orphan, no "who's authoritative for S now." The session just folds
`effect-timed-out` and carries on. **The entire §14a-old registry problem (home-node sharding, migration
races, forwarding windows, routing-slice replication) EVAPORATES** — it existed ONLY because sessions
could live on many nodes. Central folding ⇒ "where is S?" has one answer (the hub); routing = a local
table lookup; **no DHT, ever.**

### Where the remaining risk relocates — to a MUCH easier place
Central folding makes the hub the thing that must not lose data — but its job is now narrow + the
easiest-possible-durable shape: **append-only signed logs + content-addressed KV.**
- Append-only + content-addressed = trivially replicable: no consensus on mutable state, just replicate
  an immutable hash-verified append stream (log shipping — boring, solved).
- Hub = a **small replicated set**: primary folds; replicas tail the log; failover = a replica promotes
  and resumes folding **from the last durable event** — deterministically, because *the log IS the
  state*. No session migration: only the *folder* failed over; sessions never moved.
- Traded a HARD problem (mobile stateful sessions on crash-prone laptops) for an EASY one (replicate an
  append-only log).

### Honest limit + escape hatch (eyes-open)
Central fold throughput is bounded by the hub set. Ceiling is VERY high (folding is cheap; you're
effect-bound on model/build latency long before fold-bound). If ever exceeded: **shard folding by
session-id across a few hub-SHARDS** (static partition; a session lives on its shard = a small replicated
hub, NOT a random laptop) — still no crash-migration dance, a far tamer evolution than the DHT. **Don't
pay for it now:** central folding is correct + simple from P0 through "quite large"; shard-later exists
if needed.

### The three "registry" questions, now trivial
1. **Routing** ("deliver to S?") → local hub table lookup. Not a distributed problem anymore.
2. **Discovery/liveness** ("does S exist?") → hub-local (§4b tier-2).
3. **Search** ("which sessions touch repo X?") → a `query` effect against an index (never hot).

## 15. Phasing — SUPERSEDED by §15b

The old spine-first list (P0 kernel → P1 multi-session → P2 outposts → P3 MCP → P4 resources) predates
the adversarial review (§16c). It's directionally fine but scoped as a platform project. §15b is the
real plan: a small v0 that ships value in weeks, with the correctness-critical primitives designed right
from the start. This section retained only for history.

## 15b. v0 scope — the actual build plan (post-review)

**Guiding test (from §16c scope review):** *how few weeks until one agent does one useful thing, with
the fleet never going dark* — NOT "does a feature need zero kernel changes." Everything speculative
(multi-operator, memory promotion pipeline, federation, Zoom, bit-exact cross-version replay,
resource-virtualization) is DEFERRED to the vision doc above; v0 builds the spine + the one high-ROI
capability.

### v0 non-negotiables (the review's must-fix primitives — design these in, don't retrofit)
1. **Durable effect-dispatch record (S1).** Append `dispatched(effect-id, idempotency-key, deadline)` to
   the authoritative log BEFORE routing; correlate the result to it; on restart, un-resulted dispatches
   are re-driven using the idempotency key. This is the at-most-once/crash-consistency backbone.
2. **Effect-id-keyed KV continuations (S4).** The reducer control-flow pattern IS: emit effect → store
   continuation in KV keyed by effect-id → resume on the result/timeout event. Specify timeout semantics:
   **timeout CANCELS the dispatch** (kernel guarantees no late result) — simplest correct choice.
3. **Resource-scoped capabilities (SEC-F1).** A capability is `(effect-kind, resource-predicate)` checked
   against the RESOLVED runtime argument (host allow-list, repo, DB), never the effect-kind alone.
4. **Absolute deadline anchor on timers (S5).** Logged arm event records an absolute (or hybrid-logical)
   deadline, not just a duration.
5. **Determinism discipline (S3/S8), scoped honestly for v0:** single kernel version, no cross-version
   replay claim yet; canonical KV encoding + fixed CHAMP hash-order + banned nondeterministic float/NaN.
   Replay is "within this kernel version," which is all v0 needs.
6. **Boring substrate (scope review):** wasmtime + tokio + a straightforward content-addressed store on
   local disk. NO Cadenza dependency on the critical path — reducers can be authored in Rust-compiled
   wasm to start; Cadenza-native reducers (and effect-row=cap) become a MIGRATION once the compiler is
   stable. NO PKI/mesh/multi-node in v0 (single hub, single trusted operator = you).

### v0 milestones (each shippable, fleet never dark)
- **v0.1 — the kernel spine (single node, in-process).** Content-addressed append-only log + KV
  (persistent map) + one wasm reducer + the durable-dispatch/continuation loop (1,2) + a handful of
  local effects (shell, http, model-invoke) + resource-scoped authorizer hook (3) + relative+absolute
  timers (4) + append-wakes-reducer reactivity. Deliverable: **one agent session runs a real task
  loop reactively (no polling), survives a kill/restart mid-effect without double-firing.** Prove
  determinism replay within-version.
- **v0.2 — replace the fleet's polling loop.** Port ONE current fleet agent (start with the concierge —
  the dead-feeling one) to run as a v0 session: message-delivery wakes it instantly; deadlines on
  effects replace the wedge-detection triad. Deliverable: **a responsive concierge; no 30-min tick.**
  This is the "fleet never goes dark" bridge — the old fleet keeps running until each agent is ported.
- **v0.3 — elastic builds behind a content-addressed cache (the #1 ROI pain).** Effect-execution fans
  out: `build`/`test` effects dispatch to elastic worker processes/hosts; memoize by
  `(workspace@hash, toolchain@hash, cmd)` (§13a mechanism A) so identical builds are fetched not rebuilt;
  layered materialization for warm-attach. Fetch/compute split (§13) with the credential-broker pattern
  you ALREADY designed (Bedrock cred-broker generalized) so compute never holds creds. Deliverable:
  **build saturation gone — concurrency is an elasticity dial, not a fragility clamp.** (Internal
  fetch/privilege specifics: [[agent-harness-internal-resource-federation-context]].)
- **v0.4 — embedding recall over existing memory files (the 5% that's 95% of "share learnings").** Index
  the existing markdown memories; a `recall` effect does semantic query; human (you) approves any
  promotion. NO distiller/gardener/promotion-pipeline/reviewer yet — those are vision-doc, revisit when
  a second operator or real scale exists.

### Explicitly DEFERRED (vision doc, not v0)
Multi-operator + federation (§10) · full memory promotion/gate/reviewer/gardener (§9f–h) · Slack/GitHub/
Zoom brokers beyond what a ported agent needs (§11) · outpost PKI/enrollment/attestation (§12b) ·
per-tenant blob encryption + in-hub multi-tenant isolation (SEC-F4) · integrity/taint labels (SEC-F5) —
needed BEFORE untrusted ingress drives high-consequence effects, so it gates GitHub-webhook-driven
actions, not v0 · cross-version replay + frozen determinism ABI (S3) · hub HA/fencing/failover (S7) ·
Cadenza-native reducers + effect-row=capability (recouple only once the compiler self-hosts + stabilizes).

### Migration note
Operator decision (2026-08-01): **nuke the current `cdz-kernel`/`cdz-agent` and start clean.** The v0
non-negotiables above ARE the good instincts from the old code (durable cursor, reducer identity,
totality) — now designed correctly (durable dispatch, resource-scoped caps) rather than retrofitted.
Keep the old fleet running until v0.2 ports agents onto the kernel, so nothing goes dark during the
rebuild.

## 16. Open questions (consolidated)

1. **Cadenza's role** — RESOLVED (§9b): lingua franca of *contents*, kernel agnostic. Compiler is a
   PEER (content-addressed program, multi-version), not a dependency. Effect signature = capability
   manifest. Cadenza binary = wire format for contents (kernel still sees opaque hashes).
2. **KV richness** (§4) — flat map + prefix-scan vs. richer DB semantics.
3. **Query surface** — does a session query another's *raw events* (powerful, brittle across
   versions/operators) or only a *published view* (transcript / outcomes / distilled memories)?
   Lean: sessions are objects with a public interface; raw-event access is a separate higher
   privilege.
4. **Orphan rule** (§6) — cascade-close subtree vs. let children outlive; make it policy.
5. **Distiller trigger** (§9) — two-tier (close-triggered cheap + periodic gardener).
6. **Nightmare-scenario circuit breaker** — RESOLVED (§9f): proactive promotion-as-held-out-validation
   gate + reactive instant-retract (mutable-name write) + cause-DAG blast-radius + gardener auto-trip
   on correlated failure. No new machinery; a gate + a watchdog reducer, both userspace.
7. **Resource binding time** (§13) — RESOLVED: privilege binds EARLY (trusted fetch node), materialized
   content binds LATE (`workspace@hash`, any elastic compute node). Fetch/compute split solves saturation.
14. **Materialized-workspace transport cost** (§13) — shipping a large resolved dependency closure
    fetch→compute; content-addressing dedup/incremental helps but it's the real cost of the split.
8. **Snapshot cadence** — RESOLVED: free per-event via persistent KV; cadence = retention/GC choice.
9. **Trigger mechanism** — RESOLVED (§9e): two-stage funnel = embedding-similarity recall (wide) →
   pure Cadenza `situation → bool` predicate (exact, empty effect row, promotion-gate-checked). Ranked
   output; recaller applies own budget policy.
13. **Stage-1 recall ceiling** (§9e) — embeddings can miss a rule so its predicate never fires;
    needs generalizing anchors + a retroactive "missed recall" gardener signal.
10. **Session registry freshness** — RESOLVED (§14a, REVERSED the DHT lean): fold centrally at the hub,
    nodes are stateless effect executors → routing = a local hub lookup, NO DHT/session-mobility.
    Resilience via replicating the append-only log (folder failover; sessions never move). Shard folding
    by id later only if fold-bound (rare — effect-bound first).
11. **Cross-operator root-of-trust** (§4c/§10) — accepted-roots config; deferred past P0.
12. **Cadenza binary format evolution** (§9b) — DOWNGRADED: format is stable binary s-expr (structure,
    not semantics), so this is a schema-evolution *discipline* (additive-only, tolerant readers), not a
    format story. Content-type envelope tag makes cross-version dispatch honest.

**Resolved this session:** Cadenza's role (§9b); effect-row = capability manifest (§9b);
active-vs-passive recall (→ userspace); global-store mutable-name authority + anti-hijack (§4c);
storage tiers local/global bridge (§4b); memory-as-triggered-rules structure (§9); content-type
envelope tag (§9b); time/timers/cron as relative effects + `now`-as-effect (§9c); reactive
no-polling + anti-stuck (§9d, principle #6); trigger mechanism two-stage funnel (§9e); nightmare
circuit breaker (§9f); agent review of promotions (§9g); promotion pipeline = store reducer loop +
supervision tree (§9h); node/role terminology + node-is-never-a-principal trust model + PKI + three-way
authz + one-binary/bootstrap/two-tier-updates (§12.0–12e); Slack bridge = broker role (§11a);
on-behalf-of delegation + JIT-brokered credentials + confused-deputy defense (§12f); ingress patterns
(request/response · fire-and-observe · stream) + GitHub events + two-stage stream broker (§11b);
resource virtualization = fetch/compute split, privilege-early/content-late binding (§13); env-churn
reduction = fleet-wide build memoization + soft-affinity warm-attach + layered materialization (§13a);
transcript rewriting = log≠context, validity-oracle-gated context-distillation (§9i); central folding =
fold at hub (small replicated set), nodes are stateless effect executors, resilience via log replication,
NO DHT (§14a).

## 16b. Gap audit — what's NOT yet covered (pre-scoping)

We've designed the *architecture* thoroughly. "Solid" needs two more layers: the **contracts below**
(exact interfaces + correctness proofs) and the **operations around** (running it). Gaps, grouped by
danger. ★ = the two genuinely dangerous ones that should shape P0.

### ★ A. The kernel↔reducer interface contract (THE most important gap)
Everything rests on the reducer contract, and we've only described it prose-deep. Undefined:
- **Exact fold signature + the effect ABI.** `handle(event, kv) → effects` — but the concrete wasm
  component-model interface (WIT): event shape, KV verbs (§4 richness still OPEN), how effects are
  *expressed* and *returned*, how the effect-row/capability manifest (§9b) is declared and read. This is
  the spine's actual API and it's unspecified.
- **Effect result correlation.** A fold emits effect *requests*; results come back as *later events*. How
  does the reducer correlate a result to the request (ids? the reducer stores a continuation in KV)? This
  is the core control-flow pattern of EVERY reducer and it's undesigned. (Algebraic-effect resume vs.
  event-loop callback vs. KV-stored continuation — pick one.)
- **Fold failure semantics.** Reducer traps / OOMs / exceeds fuel / emits a malformed effect — what does
  the kernel do? (Poison event? Dead-letter? Retry? Halt the session?) "Can't-brick totality" (§17) is a
  *goal*; the enforcement mechanism (fuel limits, trap→event) is undesigned.
- **Determinism enforcement, not just intention.** We ASSERT folds are pure. What structurally PREVENTS
  a reducer reading wall-clock / network / randomness directly? (wasm sandbox denies ambient I/O — good —
  but wasm float nondeterminism, iteration-order, NaN bits, growth-dependent behavior can still break bit-
  identical replay. Needs a determinism spec: canonical encoding, no non-repeatable float, etc.)

### ★ B. Determinism across time = the replay/upgrade contract (the sleeper risk)
Determinism isn't just "pure at one instant" — it must hold **across kernel/compiler/reducer versions,
forever**, or replay + audit silently rot. Undesigned:
- **Does a v2 kernel replay a v1 log bit-identically?** If any fold-affecting behavior changed, historical
  replay diverges. Need either strict backward-compat of the fold semantics OR "replay uses the pinned
  engine/reducer version the events were produced under" (versions are content-addressed — feasible, but
  the RULE is unstated).
- **Snapshot validity across reducer swaps** (§7) was noted but the KV-schema-migration mechanism (how v2
  reads v1's KV) is hand-waved. This is the concrete form of §16.12.
- Without this, the crown-jewel properties (replay/audit/migration) degrade the moment anything upgrades.

### C. Log/event physical model
- **Log storage engine** — the actual on-disk format, segment/rotation, the blob store implementation,
  GC of unreferenced content (content-addressed = need refcounting/mark-sweep, undesigned).
- **Ordering/consistency of append** — single hub folder makes this easy (one writer), but the exact
  "append → durable → fold → effects" commit ordering + crash-consistency (what if we crash between
  append and effect-dispatch? between dispatch and result?) is the heart of at-most-once (§9c) and needs
  a precise write-ahead/commit protocol.
- **Backpressure** — a reducer that emits effects faster than they execute, or an inbox flooded faster
  than folded. Unbounded queues = OOM. No flow-control designed.

### D. Effect executor contract + the effect catalog
- **The executor interface** (kernel↔executor, incl. remote) — how an effect is serialized, dispatched,
  results signed/returned. Parallel to the reducer WIT, equally unspecified.
- **The concrete effect vocabulary** — we named `shell/http/model/db/query/emit/spawn/close/now/
  fire_after/compile/slack.post/github.*`. Real system needs the actual typed catalog + which are
  primitive (kernel-routed) vs. composed (reducer-level). No catalog exists.
- **Effect idempotency/retry** — retried effects (timeout→retry, §9d) that AREN'T idempotent (a shell
  command with side effects, a payment) → exactly-once is impossible; need idempotency keys or
  at-least-once-with-dedup semantics per effect. Undesigned and it's a correctness footgun.

### E. Bootstrapping the whole thing (chicken-and-egg)
- The authorizer is a wasm component; the memory store is a session; the compiler is a program. **What
  exists at t=0 before any of these are installed?** The genesis/bootstrap of the FIRST kernel (not a
  fresh node joining — §12e covers that — but the very first hub) is undesigned. Some minimal trusted
  seed must be baked in or the "kernel knows nothing" purity has no starting point.
- **Trust root establishment** — §10 assumes an operator roots a namespace; the ceremony that creates
  the first root identity/key is undesigned.

### F. Observability / operating the system
- **How does an operator SEE what's happening?** The log is the truth, but a human needs a live view:
  which sessions active, what they're doing, cost/token spend, error rates, the causal DAG rendered. This
  is a read-model over the log (queries/projections) — architecturally "just reducers/queries," but it's
  a whole surface we haven't scoped and it's how you actually run the thing.
- **Debugging a bad fold** — replay-to-a-point + inspect is a superpower we get for free, but the tooling
  (time-travel debugger over the log) is unbuilt.
- **Cost governance** — model calls + compute cost money; multi-tenant + autonomous = runaway spend risk.
  No budget/quota/rate-limit design (relates to the fleet's concurrency-clamp pain — this is the
  principled version). Capabilities could carry spend limits; undesigned.

### G. Security surfaces we named but didn't fully design
- **Sandbox strength / multi-tenant isolation** — reducers are wasm (memory-safe), but effect *execution*
  (a shell on an executor) needs real OS-level sandbox/container isolation across tenants. Named, not
  designed.
- **Prompt-injection / adversarial content in the log** — a malicious repo/webhook/transcript feeds
  content that becomes part of a reducer's context (§9i) and could hijack the agent. The capability model
  bounds *actions*, but injection→bad-action-within-granted-caps is real. No mitigation designed.
- **DoS via spawning** — the supervision tree (§6) lets sessions spawn sessions; a runaway (or malicious)
  reducer spawns unboundedly. Needs spawn quotas in the capability. Named implicitly, not designed.

### H. Smaller/deferrable gaps
- Multi-hub / cross-operator log FEDERATION protocol (how two operators' hubs interoperate — §10 trust
  root is the auth half; the wire protocol is undesigned).
- Blob-store GC / retention enforcement mechanism (§8 lifecycle is policy; the collector is unbuilt).
- Exactly-once vs at-least-once delivery guarantees for cross-session `emit` (§5) — assumed, not proven.
- Testing strategy for a nondeterminism-sensitive system (property tests for replay-determinism,
  fault-injection for crash-consistency) — the fleet's existing property-testing muscle applies.

### Summary — what to resolve BEFORE scoping P0
The two ★ gaps (A: reducer/effect ABI + result-correlation + fold-failure; B: cross-version replay
contract) are **load-bearing and cheap to get wrong** — they define the spine's actual API and its
forever-property. P0 cannot be scoped without pinning at least A. C (commit/crash protocol) and E
(first-hub bootstrap) are needed to build P0 at all. D/F/G/H layer on after the spine exists but F (obs)
and G (injection, isolation) must not be forgotten — they're how it survives contact with real use.

## 16c. Adversarial review findings (3 independent red-teams — soundness / security / scope)

Three subagents attacked the doc with distinct lenses. Findings ranked; **[F]** fatal-as-written,
**[S]** serious, **[X]** fixable. Inline corrections already applied to §12a/§12c/§12d. The rest are
recorded here as required-before-relied-upon fixes; several reshape P0 (§15b).

### The unifying root cause (both soundness + security converged here)
**Determinism-critical and crash-critical state lives OUTSIDE the log/snapshot**, and **the effect row
names kinds but not targets/provenance.** Everything below is a consequence. The single most valuable
correction: force *every obligation the kernel must honor across crash/failover/migration* into the
authoritative log + snapshot tuple, and make capabilities *resource-scoped + integrity-labeled*.

### SOUNDNESS (determinism / event-sourcing / pure-fold)
- **[F] S1 — dispatched-but-unresulted effect has no durable home.** Core loop `fold→authorize→route→
  append(result)` has NO authoritative record that "P1 was dispatched" between fold and result. Crash
  after a real `github.push` but before the result append → replay must re-dispatch (double push, breaks
  at-most-once) or drop (lost effect). Also breaks snapshot-restore (in-flight effects/armed timers not
  in the `(N, kv-hash, reducer-hash)` tuple → restore = deadlock, §9d anti-stuck evaporates) and
  compaction (pruning a `set-timer` whose fire is post-snapshot). **Fix:** durable `dispatched(effect-id,
  idempotency-key, deadline)` appended BEFORE routing; snapshot tuple must include the open-obligation
  set (or compaction refuses to cross an unresolved obligation); per-effect **idempotency key** is part
  of the effect ABI, not a footnote.
- **[F/contradiction] S2 = SEC-F2 — §12a vs §14a: where does session state/key live?** FIXED inline
  (§12a-corrected): hub folds, edge nodes hold no session key/state.
- **[S→F] S3 — "free verifiable snapshots forever" freezes the wasm engine + KV encoding + hash + NaN
  canonicalization for all time.** Bit-identical replay (incl. root-hashes) requires: mandatory NaN
  canonicalization (wasm NaN bits are impl-defined → different bytes → different CHAMP node → different
  root-hash → §8 compaction pruned behind an unverifiable checkpoint); same pinned wasmtime (float/SIMD
  not stable across releases → contradicts §12e deploy-once binary); frozen CHAMP layout + value
  encoding + hash forever. **Fix/decision:** either commit to a frozen determinism ABI (canonical
  encoding, mandatory NaN canon, pinned engine per-version-replay, frozen hash) OR accept snapshots are
  valid only within a kernel version (then compaction can't prune history a future kernel needs). Pick.
- **[S] S4 — effect-await CANNOT use algebraic-effect resume** (fresh instance per fold destroys the
  stack). Every "do effect, continue after" must be a **KV-stored continuation keyed by effect-id** — a
  userspace async runtime inside every reducer. Undercuts "reducers are clean / algebraic effects are
  why Cadenza fits." Also undesigned: concurrent/out-of-order results, and the **timeout-races-result**
  case (§9d fires `effect-timed-out`, reducer abandons, then the real result lands with no
  continuation). **Fix:** ABI mandates effect-id-keyed KV continuations; specify timeout semantics —
  does a timeout **cancel** the dispatch (kernel guarantees no late result) or must every reducer handle
  result-after-timeout? One must be true; currently neither is.
- **[S] S5 — relative-only timers break across failover/migration.** Monotonic clocks aren't comparable
  across machines; `fire_after(1h)` with 59min elapsed carries elapsed-progress only in local monotonic
  state (NOT in the log). Failover/migration → wrong deadline. **Fix:** record an ABSOLUTE wall-clock (or
  hybrid-logical) deadline anchor in the logged arm event — the kernel's timer *service* does understand
  wall-clock (reducer still doesn't; determinism preserved by recording the fired timestamp). Contradicts
  §9c's "kernel never understands wall-clock" purity — small honest concession.
- **[S] S6 — "immutable-by-hash → direct read" conflates immutability with AVAILABILITY.** A GC'd/
  not-yet-replicated blob read *during a fold* forces blocking I/O (breaks cheap-fold) or a fault (breaks
  totality) — a hidden nondeterminism/totality leak in the path §4b calls safe. **Fix:** either all
  blob derefs that can miss are effects, or the kernel guarantees residency of all hashes reachable from
  current KV/event before folding (a pinning/pre-fetch contract that constrains GC).
- **[S] S7 — "no consensus needed" understates the hub.** Data replication is easy, but exclusive
  *folding leadership* is a consensus problem (leader election + fencing). Without a fencing token,
  split-brain = two primaries folding → double-dispatch (compounds S1). **Fix:** lease + fencing token
  checked at append; reject writes from a stale primary.
- **[X] S8 — prefix-scan iteration order must be a frozen deterministic total order** or effect-emission
  order diverges on replay. Commit to CHAMP canonical hash-order; forbid any KV API whose order depends
  on insertion history/platform hashing. (Under S3's frozen-ABI umbrella.)

### SECURITY (trust / authz / credentials / isolation / injection)
- **[F] SEC-F1 — effect-row-as-manifest is TARGET-BLIND.** The row names kinds (`Http.get`), not targets
  (`http://169.254.169.254/…` IMDS, `attacker.com/exfil?secret`). Within-cap SSRF/exfil; the broker
  can't scope a GH token to a repo from `{Github.push}` alone. FIXED inline (§12c): capabilities carry
  resource predicates checked against resolved runtime args; effect-row is necessary-not-sufficient.
- **[F] SEC-F2 — compromised edge node holding a session KEY forges valid-signature effects + gets real
  broker-minted creds via a genuinely-valid chain.** FIXED inline (§12a-corrected): keys at hub only.
  Residual: authenticate a laptop-broker's relayed INTENT without conferring authority (→ §16c open).
- **[F] SEC-F3 — memory promotion gate defeatable on all three legs.** (a) **purity ≠ safety**: a *pure*
  trigger can be an ML-style backdoor branching on a validation-marker (fires benign in held-out, malign
  in prod) — purity only bans I/O. (b) held-out is **overfittable** by a code-is-data trigger author.
  (c) the **LLM reviewer is prompt-injectable** by the attacker-authored claim/provenance it reviews —
  and approve IS the `set`, so an injected reviewer IS the promotion. **Fix:** triggers = a whitelisted
  **non-Turing predicate DSL** over declared situation fields (no backdoor branching); executable claims
  **never auto-apply** cross-operator + per-use human auth; present artifacts to the reviewer as
  delimited untrusted data and DON'T let the reviewing model hold the `set` cap for effect-bearing
  org-public memories (human/non-LLM gate).
- **[F] SEC-F4 — content-addressed store has NO read authorization by design.** "The hash IS the
  authorization" = write-integrity, NOT read-confidentiality. Hashes leak constantly (cause-DAG, memory
  provenance, query results, registry) → a tenant that observes a hash reads the blob, no check anywhere.
  Plus the hub folds all tenants in one process (§16b.G). **Fix:** encrypt blobs per-tenant (hash
  addresses ciphertext; decryption key = the capability) OR add a read-ACL layer for non-public blobs;
  design in-hub tenant isolation before centralizing all tenants.
- **[S] SEC-F5 — no integrity/taint model.** Kernel authorizes effect kinds but tracks neither targets
  nor argument PROVENANCE, so "channel ≠ authority" (§11a) and "stream intent proposal-only" (§11b) are
  userspace conventions, not enforced invariants. Injected webhook/repo/transcript content → permitted-
  but-harmful action; worse, the concierge/router that scopes delegations is steerable by injected text.
  **Fix:** kernel-enforced integrity labels on events (authenticated-operator > internal >
  attacker-ingress > recalled-memory) that propagate into effect args; authorizer can REQUIRE a minimum
  integrity level for high-consequence effects (prod writes, delegations, credential mints). Harden the
  router: its scoping decision must not be steerable by low-integrity content.
- **[S] SEC-F6 — broker tokens are broad + live on busy nodes.** "Idle node has nothing to steal" — but
  busy privileged nodes always have a fresh token; and GitHub-App/STS tokens can't be scoped per-effect
  (coarser than one effect). **Fix:** proxy credentialed calls through the trusted broker so the raw
  token never lands on elastic compute (extend the §13 fetch/compute split to ALL credentialed effects);
  fine-grained/repo-scoped tokens + seconds-TTL/single-use where the API allows.
- **[S→F if botched] SEC-F7 — PKI gaps:** join-token theft; IMDS identity-doc **replay** (bind a fresh
  hub-chosen nonce!); revocation lag (per-effect authz must consult LIVE revocation or use very short
  cert TTLs); and the **first-hub root ceremony is undesigned** — the unaudited crown key (→ HSM /
  M-of-N threshold, audited).
- **[X] SEC-F8 — determinism is a security control but isn't enforced** → a fold could act malicious live
  but replay benign, defeating audit/blast-radius/provenance. And a malicious projection (§9i / obs
  read-model) can render a benign VIEW over a malicious log nobody reads raw. **Fix:** enforce the
  determinism contract (ties to S3); ensure human observability can render the RAW log + detect
  projection/log divergence.
- **[X] misc:** spawn-DoS (no spawn quota in caps; §6 — and the hub is shared across tenants →
  cross-tenant availability risk); self-mod pointer hijack (swap-reducer must target a PINNED hash, never
  a mutable `-latest`); non-idempotent effect retry (= S1's idempotency key); cost governance
  (capabilities carry spend limits, else runaway-spend DoS).

### SCOPE / COMPLEXITY / BUILDABILITY (the cold-water review)
- **Scope vastly exceeds the real pain** (fragile single box · dead-feeling polling · hand-rolled wedge
  detection · credential freeze · share-learnings). We designed an event-sourced wasm OS + PKI + IAM +
  Bazel-class cache + cross-operator memory governance. Most of §9f–h, §10 multi-operator, Zoom
  distillation, bit-exact cross-version replay solve problems that **don't exist yet** (one operator).
- **"Kernel knows nothing" relocated complexity, didn't remove it** — ~15 "just a reducer" programs
  (authorizer, distiller, gardener, promotion pipeline, reviewers, router, matcher, broker, …) are each
  hard pure/total/deterministic programs someone must write.
- **Skyscraper on scaffolding:** every *differentiator* (effect-row=cap, pure triggers, code-is-data
  memories, sexpr wire) needs Cadenza, which is itself mid-self-host. Two in-progress megaprojects
  taking each other hostage. → build v0 on a boring stable substrate; treat Cadenza-native properties as
  later migrations.
- **Rebuild cost:** rejecting Temporal/wasmCloud/Bazel-RBE to reimplement all three from scratch on
  wasmtime+tokio is a multi-year commitment; §16b concedes ~everything below the architecture line is
  undesigned. Adopt aggressively for the boring-hard layers.
- **Determinism value is narrower than sold:** model outputs are recorded, so replay replays *stored*
  outputs → mostly buys AUDIT, which a plain append-only log gives WITHOUT bit-exact-forever determinism.
- **Second-system effect:** don't nuke landed/working code to build a 10× version → no working runtime
  for quarters. *(Operator decision 2026-08-01: nuke anyway — the current code isn't where it needs to
  be; a clean start is easier. Recorded, honored — but v0 stays small per below.)*
- **Verdict: build a radically smaller subset.** The right test is "how few weeks until one agent does
  one useful thing, with the fleet never going dark," NOT "does a feature need zero kernel changes."

- The determinism instinct (at-most-once cursor, "replay re-folds with no live effect") — RIGHT,
  carry forward as the whole ballgame.
- Explicit reducer identity — RIGHT, now connected to snapshotting + self-mod.
- Cedar authorization — RIGHT, but must become a swappable wasm component, not hardcoded.
- "Can't-brick" totality of pure folds — RIGHT, keep.

What we're dropping: baking "agent" / "Cadenza" / "Bedrock" into the kernel; the reducer returning
serialized state (replaced by session-attached KV); two fixed Rust crates as the architecture (the
kernel is generic; agent/Bedrock/etc. become reducers + executors).

## 18. Operator design directives on landed v0.1 (2026-08-01) — plan

Two operator directives (via concierge) after reviewing the landed v0.1 kernel. Folding in as design
revisions; log-format is foundational (do before more builds on the current custom format).

### 18a. LOG FORMAT → Cadenza binary s-expr, length-prefixed/streaming
**Directive:** drop the custom binary event encoding (§ event.rs `encode`/`decode`); encode each event
as **Cadenza binary s-expr**, length-prefix it, append. Rationale: the log becomes self-describing
Cadenza AST/values — "programs meta-inspect logs just by decoding them like a regular AST," no bespoke
decoder, consistent with the language. This is §9b's "Cadenza binary = wire format" pulled forward into
v0.1 (was deferred).
**Plan:** event → (represent as an s-expr value) → binary-sexpr encode → u32/varint len-prefix → append;
recover = read len-prefixed frames, sexpr-decode each (streaming, one frame at a time — matches the
current LogStore framing, only the per-frame codec changes). The framing/torn-tail/corrupt logic
(§LogStore, PR#990 #4) stays; the `Event::encode/decode` body is what's replaced.
**⚠ OPEN (looped to operator/concierge):** there is no lightweight binary-sexpr codec available to the
kernel today. `cadenza-syntax`'s sexpr module is TEXT (`read(&str)`/`print()->String`), and the crate
is dependency-heavy (cedar-policy, pulldown-cmark, num-bigint) — pulling it into the minimal standalone
kernel workspace (blake3-only) contradicts the "minimal deps / kernel knows nothing" principle. Options
to resolve: (a) a small standalone `cadenza-binary` codec crate (structure-only: atoms/lists/bytes/ints
+ len-framing — the stable wire layer, no interpreter) the kernel + others depend on; (b) text s-expr
for v0.1 (self-describing + meta-inspectable NOW, binary later — but not the operator's stated binary
ask); (c) accept the cadenza-syntax dep. **Lean = (a)** — it satisfies "self-describing s-expr,
streaming, meta-inspectable" without the heavy dep and keeps the kernel codec-agnostic (it just frames
opaque bytes; the s-expr codec is a separable crate). Needs operator/concierge nod on introducing a new
small crate vs. their exact intent.

### 18b. SHELL invocation — stderr, pipelines, LOCK DOWN the surface
**Directives:** (a) capture STDERR too (keep distinguishable from stdout); (b) model PIPELINES
(`cmd | cmd`), not just single commands; (c) SECURITY — operator "not comfortable with arbitrary shell
invocation"; rethink the surface: structured/allowlisted/capability-gated/sandboxed, not arbitrary
strings to `sh -c`.
**Plan:**
- (a) STDERR: `EffectOutcome::Ok` payload becomes a structured `{ exit, stdout, stderr }` s-expr value
  (once 18a lands the s-expr codec) rather than raw stdout bytes; distinguishable, and debuggable.
- (b) PIPELINES: replace the single `sh -c <string>` target with a **structured command model** — a
  pipeline is a list of stages, each `{ program, args: [..] }`, wired stdout→stdin. No shell string
  interpolation → no shell-injection surface. (The reducer emits the structured pipeline as the effect
  payload, not a string.)
- (c) SECURITY: this IS the lock-down — a structured `{program, args}` model executed via direct
  `Command` (no `sh -c`, no shell metacharacter parsing) removes arbitrary-shell injection by
  construction. Layer the existing SEC-F1 capability on the *program* (allowlist of program names +
  arg predicates) instead of a command-string prefix. So: structured command model + program-allowlist
  capability + direct-exec (no shell) + optional sandbox later. The current `ShellExecutor`
  (`sh -c <target>`, string target, Prefix-on-string capability) is the interim; this replaces it.
**Sequencing:** 18b's structured-outcome/`{exit,stdout,stderr}` and structured-command payloads want the
18a s-expr codec first (they're s-expr values), so: land 18a (log/value codec) → then 18b (structured
command model on top). Both are design revisions to the landed v0.1, not greenfield.

## 19. Operator architecture refinements (2026-08-01, round 2) — supersedes parts of §18

After reviewing the §18 plan + my codec-blocker/generic-async questions, the operator refined the
architecture. These SUPERSEDE the earlier "trait-ify log+kv+reducer / generic+async" framing where they
conflict. Reply with the shape was sent; folding here.

### 19a. Binary-sexpr codec = EXTRACT the existing `cadenza-syntax::codec` into a shared bottom crate (REVISED)
Resolves the §18a open codec question. **The codec ALREADY EXISTS and is already shared** — the earlier
"build cadenza-binary" is superseded (would have greenfielded a 4th impl that drifts from the spec pins).
Facts (from v-syntax, the codec-surface owner, 2026-08-01):
- `cadenza-syntax::codec` (`src/codec.rs`, ~838 lines) IS the canonical binary-sexpr wire format:
  structure-only Atom/List tree, encode+decode, 8-byte version header `cdzast\x00\x01`, **TOTAL decode**
  (refuses wrong header / malformed len-tag / out-of-range id / **cycles or shared subtrees → no
  decode-bomb** / trailing bytes — never panics, never a wrong tree). Spec-pinned by
  `spec/contracts/ast-encoding.md` + `constitution.md` (bijection, versioned; duvet `//= //#` anchors),
  corpus-gated (`cadenza-syntax/tests/corpus_roundtrip.rs`: `ml_surface_round_trips_the_corpus` +
  `binary_surface_round_trips_the_corpus` + `all_surface_paths_round_trip_the_corpus`, 3/3 green).
- **rcdzc shares this exact wire format** — it has its OWN `crate::codec` (rcdzc/src/codec.rs) that is
  byte-compatible with cadenza-syntax's, driven from `rcdzc/src/{compile,tests,link,backend/rust/tests}`
  (`crate::codec::encode/decode`); it decodes what `cadenza_syntax::codec::encode` produces (rcdzc keeps
  its own decode on its trusted derive path — v-rust-backend confirmed — so it consumes the shared
  FORMAT, not the cadenza-syntax crate). → 2 of the 3 consumers already share the wire format. (PR#1000
  corrected the earlier imprecise file list: rcdzc/src/sidecar.rs only *mentions* codec in a comment.)
- **Value model** the leaf vocab MUST carry (to round-trip real programs): Int (sign+radix in the kind
  tag, no signed-zero), Float (sign+i64-exp+bigint significand), Str/Name/Sym (distinct UTF-8 kinds),
  Bytes, Char, Bool, BadChar/BadEscape MARKER leaves (malformed literal = a leaf the compiler rejects
  downstream, NOT a decode failure — decode stays total), numeric SUFFIX bodies. **Spans NOT in the
  codec** (span-independent canonical form; spans ride a separate sidecar). Arena StructId is an
  encoding detail (post-order child refs), not the wire contract — you decode to a tree.
- **The one real gap = extraction:** codec.rs lives INSIDE cadenza-syntax today, dragging
  num-bigint/sha2/unicode-normalization/pulldown/toml_edit/cedar/clap — the kernel wants none. Honest
  dep floor after extraction = **num-bigint + hand-rolled leb128** (NOT zero — arbitrary-precision
  Int/Float is the non-negotiable value model).

**PLAN (v-syntax LEADS, kernel consumes):** extract `codec.rs` + `leb128.rs` + the Leaf/Arenas value
types into a new BOTTOM crate (`cadenza-ast`/`cadenza-binary` — v-syntax's naming); cadenza-syntax
RE-EXPORTS (public API + corpus gate unchanged); rcdzc + cdz-kernel depend on the same crate. v-syntax
owns the extraction (their invariants + duvet anchors + corpus acceptance gate — must be byte-for-byte
identical, `cdzast\x00\x01` preserved, decode stays total, tests stay green) and coordinates rcdzc.
**Kernel-facing seam I need:** `encode(&tree)->Vec<u8>`, TOTAL `decode(&[u8])->Result<tree,DecodeError>`
with `DecodeError` exposed (I map it to the log's corrupt/torn distinction — total decode is load-bearing
for crash-recovery), the num-bigint+leb128-only dep floor, and a stable constructible/matchable tree
type. Kernel's current custom event codec is the INTERIM; the log-event codec (§18a) swaps to the bottom
crate once it lands. Not blocked meanwhile.

### 19b. WASM COMPONENT MODEL for the kernel core; log+kv stay traits (decided — reduces abstractions)
Operator steer: "focus on the wasm component model for the agent kernel rather than a Rust trait that
later maps to it — reduce the number of abstractions." So:
- **Guest boundary = a WIT world (component model), from the start.** The REDUCER is a wasm component
  the kernel instantiates; the host↔reducer contract is a WIT world (events in → effect-requests out),
  NOT a Rust `Reducer` trait later mapped to wasm. This SUPERSEDES §18/earlier "trait-ify the reducer."
- **Host-backend boundary = Rust traits.** LOG + KV STAY traits (operator carve-out — they're a property
  of the HOST, swappable backends; trait-abstraction is correct there).
- So: component-model at the GUEST boundary, traits at the HOST-backend boundary. The current Rust
  `Reducer` trait is the INTERIM until the WIT world + wasmtime component host lands (the next big slice
  after the codec).

### 19e. Wiring a WASM reducer into the kernel loop — correlation ABI (decided: KERNEL-OWNS, tier A)
The kernel `drive` loop keys continuations by a kernel-assigned `EffectId`; a WASM `ComponentReducer`
speaks its OWN opaque continuation TOKEN (echoed back verbatim as `resumes`; the guest never sees the
`EffectId` — the operator's earlier collapse-to-one-token ruling). Bridging them is §16c-gap-A, and the
correlation-ownership fork was ruled (concierge, faithful reading of the standing token ruling):

- **DECIDED: (A) KERNEL-OWNS.** The kernel keeps its `EffectId` INTERNAL-ONLY (durable-dispatch recovery
  §16c-S1, the open-obligation set, the timer table all need it, for a WASM reducer as much as a Rust
  one). The guest stays token-only. A small `EffectId ↔ token` map bridges them at the boundary: when the
  guest emits an effect carrying a token, the kernel assigns an `EffectId` and records the pairing; on the
  result event it looks the token back up to feed the guest's `resumes`. This is the faithful
  implementation of the existing token ruling — not a re-decision.
- **Rejected (B) guest-token-native loop** — would fork the loop by reducer-kind and risk the WASM path
  losing recovery keying (the kernel needs `EffectId` for replay regardless of reducer kind). **Rejected
  (C) add a correlation field to the kernel `EffectRequest`** — leaks the guest concept into the kernel's
  clean `{kind, target, payload}` type; the map keeps it out.
- **⚠ HARD GUARD (durable/replay path):** the `EffectId↔token` map is session state that MUST be
  replay-deterministic and REBUILDABLE FROM THE LOG on `Session::recover` — NOT volatile-only. So the
  guest token is **persisted IN the `Dispatched` frame** (which already records the dispatch + its
  `EffectId`); recovery rebuilds the map from the recorded frames. A crash must not lose the mapping.
- **Build order:** (1) add the token to `Dispatched` (event.rs — re-pins the frozen golden) + rebuild the
  map on replay; (2) `impl Reducer for ComponentReducer` translating `Event` → `(content_type, payload,
  resumes-token)` via the map. The current Rust `Reducer` trait stays the interim path until this lands.

### 19c. WASI-as-host-ABI — SPLIT verdict (my read, sent to operator; open for their call)
Operator mused WASI-as-ABI might help the shell-security concern, then doubted it. My read:
- **WASI is a GOOD fit for filesystem / network / clock / random** capability-gating — it's
  capability-oriented by construction (explicit granted handles, no ambient authority), maps directly
  onto SEC-F1 resource-scoped caps, and is the component-model-native way to sandbox what a reducer can
  reach. Adopt it for those effect kinds.
- **WASI is the WRONG fit / no help for SHELL** — and the operator's instinct is right: WASI has NO
  subprocess/exec (`wasi:process/exec` doesn't exist in preview 2; spawning is deliberately outside
  WASI). So the shell surface CAN'T be a WASI capability — it stays a host executor + the §18b
  structured-command lock-down, independent of WASI.
- **Synthesis:** WASI is COMPLEMENTARY — use it where it models the effect (fs/net/clock/random), keep
  shell a first-class kernel effect with the structured-command lock-down. Not a replacement for the
  shell design.

### 19d. Net sequencing (the forward plan)
(a) `cadenza-binary` shared crate (coordinate w/ v-syntax+rcdzc) → log-format swap to it (§18a).
(b) WIT reducer world + wasmtime component host (§19b); log+kv stay traits. Current Rust `Reducer` trait
    is interim until this lands.
(c) structured-command shell lock-down (§18b); WASI for fs/net where it fits (§19c).

## 20. Operator authz directives (2026-08-01, round 3) — reshape the capability/authz story

Two operator directives (verbatim relayed) that reshape authz. Both fold into the capability model
(§4c/§9b/§12c) and the effect-row-as-manifest thinking (as corrected by SEC-F1 to resource-scoped caps).

### 20a. RESOURCE-RESCOPING components — "Rust unsafe-block for capabilities" (decided)
Operator: a published component can DOWN-SCOPE a broad, dangerous capability into a narrow,
provably-safe resource that callers can be granted freely. Example: a program that runs `date` and
returns it. Under the raw model, invoking it needs the arbitrary-`shell` capability (huge blast
radius). Instead, publish a component that INTERNALLY holds the shell capability but EXPORTS only a
narrow `date`-shaped resource — callers need just the narrow `date` grant, and the code is provably
safe for anyone to invoke. Analogy: Rust's `unsafe` blocks — encapsulate a powerful operation behind a
safe, minimally-scoped interface so the danger is contained + audited in one place, not spread to every
caller.

**How it fits the model:** this is **capability attenuation via a published component** — the exact dual
of the §12f attenuating delegation (which narrows DOWN a spawn tree). A rescoping component is a
content-addressed program whose own effect row includes the broad cap (`shell(target="date")` — already
resource-scoped per SEC-F1) but whose EXPORTED interface is a new, narrow *virtual resource*
(`date.now → string`) requiring only a `date` capability to invoke. The authorizer treats "may invoke
the date-component" as a distinct, cheaply-grantable capability; the component, once invoked under its
own identity/grant, performs the broad effect internally. So the powerful grant lives with ONE audited,
published, content-addressed program, and N callers hold only the safe narrow grant. This makes
capability surface *composable*: dangerous primitives get wrapped into safe, named, shareable resources
(and a rescoping component is itself a tool that can be published/recalled like any other — §9b
tooling-as-programs). First-class authz primitive: **a capability can be a "may-invoke-component-X"
grant, and X re-exports a narrower resource than it internally wields.**

### 20b. CEDAR as a content-addressable wasm COMPONENT + genesis bootstrap (decided — don't reinvent)
Operator: the kernel is "building a poor man's Cedar." Do NOT hand-roll authz. **Cedar (the policy
engine) is ANOTHER content-addressable wasm component** (like the authorizer already is in §1/§9b — this
directive makes it concrete + names Cedar as THE engine, not a bespoke one). The harness has its own
**GENESIS program + log** that bootstraps the whole runtime — authz included. Requirements:
- **Update Cedar WITHOUT redeployment:** the Cedar-engine component is content-addressed and referenced
  by hash from the log; swapping it = an authorized `set` of the engine pointer (§4c mutable name +
  §7 self-mod), no binary redeploy. This is exactly the "kernel deploys once, everything else is a
  swappable component" thesis (§12e) applied to the authorizer.
- **Ship Cedar policies + reference them ON the log:** policies are content-addressed artifacts; a
  session's/operator's active policy set is named on the log (a `set(policy-name, hash)` event, §4c), so
  policy changes are logged, auditable, versioned, and shippable between operators like any content.
- **Genesis bootstrap:** the runtime's own genesis program + log installs the Cedar engine + initial
  policies as its first events (§3 genesis + context-as-events) — the authz layer is itself set up
  through the log, not baked into the kernel binary. The kernel stays authz-agnostic: it invokes
  whatever authorizer component the log currently points at, passing it the effect + the session's
  capability context; Cedar (as that component) renders the decision.
- **Don't reinvent the wheel:** the v0 in-kernel capability check (§4c/authz.rs) is the INTERIM; the
  end-state authorizer is the Cedar component. The kernel's job shrinks to "invoke the current
  authorizer component"; Cedar owns policy semantics.

**Net authz shape:** kernel = mechanism (invoke the authorizer component, carry capability context,
enforce the decision). Cedar-component = policy engine (swappable by hash, no redeploy). Policies =
content-addressed artifacts referenced on the log. Capabilities = resource-scoped (SEC-F1) grants,
including "may-invoke-component-X" grants that enable resource-rescoping (§20a). All bootstrapped by the
genesis program/log. This supersedes any notion of a hand-rolled kernel authz engine — the kernel
authorizes by DELEGATING to a component, exactly as it treats every other capability.

## 20c. Model-invocation effect: realtime-vs-batch priority class (FORWARD-LOOKING — capture, no build)

Operator forward-looking idea (2026-08-01) — captured for when the kernel models AI-model invocation as
a first-class effect; NOT a build item now (v0's `model` EffectKind is a placeholder). When the
model-invocation effect is designed, its request should carry a **realtime-vs-batch priority/latency
class**:
- **realtime** → standard synchronous model invocation (interactive / user-facing turns).
- **batch** → route to the provider's cheaper batch interface (e.g. Bedrock's batch API, ~half the price
  of standard invocations), for LOW-PRIORITY async-analysis tasks that don't need an immediate answer.

Fits the effect model cleanly: it's a field on the model-invocation `effect-request` (like `target`
carries the model id), and it's a routing/executor concern — the executor picks the standard vs. batch
backend by the class. It composes with the async/deferred nature of the kernel: a batch invocation is
just an effect whose result arrives (much) later as a recorded result event (§16c-S4 continuation), no
different in shape from any other deferred effect — the reducer already resumes on the result whenever
it lands. So "batch" is a latency-class hint the executor honors, not new kernel machinery. Note when
model-invocation modeling lands: thread this priority class through the model effect's request record.

## 21. Operator kernel-comments reorder (2026-08-01, round 4) — wasmtime core, CAS-first, real-Cadenza E2E

Three operator comments on the wasm-kernel work, reshaping the §19b slice order. Standing principle
throughout: "raise issues and get them fixed, don't work around them."

### 21a. wasmtime is CORE, not optional (done)
The kernel IS a reactive wasm runtime — wasmtime is the ENGINE, not an optional add-on. The earlier
`wasm-reducer` feature-gate imported cdz-run's "wasmtime is edge-only" isolation (right for a
compiler-adjacent tool, wrong for the kernel). wasmtime + the component host are now a non-optional
kernel dependency; the feature is gone. (`live-exec` stays gated — a real security/spawn gate.)

### 21b. Component-dependency linking requires CAS FIRST (reorders the plan)
> ⚠ **Superseded framing — see §23.** The dependency ORDER below (CAS → linking → real-Cadenza) stands,
> but the "resolve THE Cadenza runtime" framing is replaced by GENERIC declared-dependency resolution:
> the kernel has no knowledge of any specific runtime; a component declares its deps by hash and the
> kernel resolves each from CAS uniformly (operator directive, §23). Read §23 as the current design.

Real Cadenza reducers IMPORT the value-heap runtime component (they can't run standalone), so the kernel
must LINK component dependencies — compose the reducer component with the runtime component, resolving
each by content hash. That requires a **content-addressable blob store** (hash→bytes fetch). Current CAS
state: content-ADDRESSING exists (`hash.rs` + content-addressed events/KV-roots), but there is NO blob
store yet (the §4 blob store is designed, not built). So CAS is a genuine prerequisite. Dependency order:
**(i) CAS blob store → (ii) component-dependency linking → (iii) real-Cadenza reducers.**

### 21c. Real Cadenza reducers are the END-TO-END target; the Rust fixture is INTERIM
The wit-bindgen Rust guest (Option A) is fine to bring up the host machinery + prove the WIT binds, but
the operator's real bar is a **Cadenza-authored reducer via rcdzc→wasm-component**, run end-to-end, to
prove the reducer INTERFACE is right + not over-specialized for Rust, and to **surface missing
Cadenza/rcdzc functionality**. A Rust guest doesn't need the runtime component (no Cadenza runtime
import), so it can bring up the host in parallel; the real E2E (Cadenza reducer) is gated on 21b
(CAS + linking). When a real Cadenza reducer surfaces a gap (rcdzc can't emit the component/import
shape, Cadenza can't express the reducer signature, the WIT assumes Rust-isms), **RAISE it to the owning
lane** (rcdzc = v-rust-backend/v-compiler-ml; runtime = v-runtime; syntax = v-syntax) — do NOT work
around.

### 21d. Reordered slice plan (supersedes the §19b/§19d tail)
[A: Rust wit-bindgen fixture — INTERIM host bring-up, in flight] → 21a wasmtime-core (done) →
CAS blob store → component-dependency linking (compose reducer + runtime by hash) → real-Cadenza
reducer end-to-end (surfacing + routing any Cadenza/rcdzc gaps). The in-process Rust `Reducer` trait
stays the working reducer path until the component path is proven end-to-end with a real Cadenza reducer.

## 22. Async runtime: tokio, gas, preemptive yields, session multiplexing (2026-08-01, operator directive)

Operator: "Blob store needs to be async so we can do network fetches without blocking the whole runtime.
And really everything should be async and we should pull in tokio so we can do gas and preemptive yields
and are able to multiplex sessions more easily." Foundational execution-model stance for the kernel.

### 22a. Async is the substrate (tokio)
The kernel already uses tokio (v0.1); this leans INTO it. Async is what enables the next three, each a
design driver:
- **Non-blocking I/O:** effect execution + blob fetches (esp. a NETWORK-backed CAS fetch — §12 outposts /
  a remote blob store) must not block the runtime. A fold that awaits a slow http/model effect or a
  remote blob fetch yields the executor to other sessions instead of stalling everything.
- **GAS (metered fuel):** bound how much a reducer/effect may consume (wasmtime's fuel/epoch mechanism —
  cdz-run already uses an epoch deadline to trap a runaway run). A reducer can't monopolize or hang the
  kernel; exhausting fuel is a bounded, recoverable outcome (surfaced as an effect-timeout-like event).
- **PREEMPTIVE YIELDS:** the runtime can yield a running task at await points (and fuel/epoch checkpoints)
  so one session/fold can't starve others.
- **SESSION MULTIPLEXING:** run many sessions concurrently, interleaved, on the tokio executor — async
  makes N-session hosting natural (ties to §14a central folding: the hub folds many sessions; async is
  how it interleaves them without one blocking the rest).

### 22b. Async blob store (reshapes the just-landed CAS API — next code slice)
The §4 CAS blob store (landed sync in `blob.rs`: `BlobStore` trait + Mem/Disk) must become **async**:
`async fn get/put/has`, so a network-backed backend fetches without blocking. Local Mem/Disk backends
implement the async trait trivially (their bodies are sync, wrapped); the motivating case is a remote
backend (fetch a component/blob by hash over the network). NEXT CODE SLICE: convert `BlobStore` to an
async trait (async-trait or native), and thread async through `ComponentReducer::resolve_runtime` +
`apply`. This is why the operator raised it now — the blob store was next on the plan; build it async.

### 22c. ⚠ THE SUBTLE CONSTRAINT: async must NOT break replay-determinism (§16c-S3 crown jewel)
Async scheduling introduces nondeterministic *task interleaving* — but the kernel's proven replay
determinism (the whole value prop) requires the FOLD ORDER + EFFECT RESOLUTION to be deterministic. These
are reconciled because determinism lives in the LOG, not the scheduler:
- **Per-session fold order is already serialized by the log.** A session folds events in log-sequence
  order; that order is recorded, not scheduler-dependent. Async interleaving happens BETWEEN sessions
  (which have no shared mutable state — §5 cross-session is via logged effects), and WITHIN a session
  only at effect-await points where the fold is *suspended waiting on a recorded result event* — the
  result's content + the order it folds back are recorded (§16c-S1 durable dispatch + S4 correlation),
  so replay re-applies recorded results in recorded order regardless of live timing.
- **Effect completion order is nondeterministic live, deterministic on replay.** Two concurrent effects
  may complete in either wall-clock order live; the kernel records EACH result event as it lands (with
  its cause/id), freezing the order into the log. Replay reads that frozen order — it does NOT re-run
  the effects or re-race them. (This is the §9c timer discipline generalized: nondeterminism at the edge,
  determinism in the fold, because the outcome is recorded.)
- **Gas/preemption yields are scheduler events, NOT logged fold inputs.** A preemptive yield or a
  fuel-exhaustion pause changes WHEN a fold runs, not WHAT it folds or in what order — the fold is a pure
  function of (event, kv), re-run identically on replay. Fuel exhaustion that ABORTS a fold IS a logged
  outcome (like a trap → fold-failure, §16c gap A), so it replays deterministically.
- **The invariant to hold:** the kernel may interleave/yield/meter freely, but it must record every
  determinism-relevant fact (which event folded, each effect's result + the order results landed, any
  fuel-abort) into the log as it happens. Replay then reconstructs from the log, never from the
  scheduler. RAISE any place where async would leak a scheduler decision into fold state (that would be a
  determinism bug) — don't work around it.

### 22d. Fold-loop + fuel — a bounded fold, in two steps (PR#1009)
**Step 1 (LANDED, `4782e74cc`, batch #132):** a hard per-fold fuel bound. The engine enables
`Config::consume_fuel(true)`; `apply` gives instantiation headroom then resets `Store::set_fuel` to a
per-fold budget (`DEFAULT_FOLD_FUEL`) right before the fold, so the budget bounds the FOLD, not
instantiation; `Trap::OutOfFuel` is classified DISTINCTLY as `ComponentError::FuelExhausted{budget}`
(resource exhaustion) vs a semantic `Trap`. This closes the runaway-guest hang (PR#1009) synchronously —
a runaway fold TRAPS at the budget rather than hanging. It's the interim, sync form.

**Step 2 (CHOSEN mechanism, §22e): cooperative async yield, not a hard trap.**

### 22e. Gas = `Store::fuel_async_yield_interval` — cooperative yield (2026-08-01, operator directive)
Operator (verbatim): *"For the gas consumption I want it to be using `Store::fuel_async_yield_interval` so
it can yield to other tasks."* This is the CHOSEN gas mechanism for §22 — the concrete form of the
"preemptive yields + session multiplexing" substrate (§22a).

- **Mechanism.** Instead of only trapping when fuel hits zero (step-1 hard bound), configure the guest
  Store with `fuel_async_yield_interval(Some(N))`: every `N` fuel units consumed, the guest **yields the
  async task** (requires the async wasmtime config + tokio — which §22a/§22b are already pulling in). A
  long-running reducer yields at fuel-interval boundaries so other sessions/tasks interleave, instead of
  monopolizing the runtime. This is the multiplex substrate: cooperative preemption keyed on fuel, not
  wall-clock.
- **Relationship to step 1.** The hard budget stays as the OUTER bound (a reducer that blows a large total
  budget is still aborted → `FuelExhausted`, a logged outcome); the yield interval is the INNER,
  finer-grained cooperative-scheduling knob. Yield-interval = "let others run"; budget-exhaustion = "this
  fold is a runaway, abort + record." Both are fuel-driven, so both are deterministic (below).
- **This closes PR#1009 the operator's way.** A runaway reducer now YIELDS (and can be observed/bounded/
  aborted by the scheduler) rather than either hanging (old) or only hard-trapping (step 1) — the DoS
  guard becomes a scheduling property, not just a kill switch.
- **⚠ Determinism guard (the §22c invariant, restated for gas).** A fuel-yield changes WHEN a fold runs,
  never WHAT it folds or in what order — it's a scheduling event, not a logged fold input. The
  fold stays a pure function of `(event, kv)`. Two hard requirements: **(a)** any fuel-EXHAUSTION-ABORT
  outcome MUST be recorded in the log (like a trap → fold-failure, §16c gap A) so replay reconstructs it;
  **(b)** fuel ACCOUNTING itself must be deterministic per `(event, kv)` — the same fold must charge the
  same fuel every run, so replay reaches the same yield/abort points. wasmtime fuel is
  instruction-counted (deterministic), so this holds — **but** if any host call the guest makes (an import,
  a future async effect) charged fuel in a way that varied with wall-clock or host state, THAT would be a
  determinism leak. RAISE it, don't paper over it. (Replay itself need not yield — yielding is a live
  multiplex concern; replay just re-folds recorded events + recorded abort outcomes in log order.)
- **Sequencing.** Gated on the async substrate (`fuel_async_yield_interval` requires async Store support +
  the async `call_apply`), which follows the §22b async-`BlobStore` conversion. Until then, step 1's sync
  hard bound is the live DoS guard. The impl slice touches `wasm_host.rs`.

## 23. The kernel is RUNTIME-AGNOSTIC — deps are declared + CAS-resolved by hash (2026-08-01, operator directive, EMPHATIC — supersedes the Cadenza-runtime specifics of §21b)

Operator (verbatim, emphatic): *"Why is the kernel hard-coding the fact that the cadenza runtime exists!?
The component should simply declare it has a dependency on a set of components and the kernel should
resolve it from the cas. It should have no knowledge of the cadenza runtime!"*

**The principle (firm).** The kernel MUST have ZERO special knowledge of "the Cadenza value-heap
runtime." A reducer component **declares** its dependencies — a set of component references by content
hash — and the kernel **resolves** each from CAS and links it, treating every dependency identically. The
Cadenza runtime is *just one more content-addressed component* a reducer happens to depend on; it is NOT
a built-in the kernel knows by name, interface prefix, or identity. This sharpens the §21d critical path:
**linking = generic "resolve each declared component dep from CAS by hash, then link"** — never "if the
reducer needs the Cadenza runtime, load the Cadenza runtime."

### 23a. Where it's hard-coded today (to remove)
> ✅ **DONE (code landed as the refactor below).** `RUNTIME_IFACE`/`required_runtime`/`RuntimeReq`/
> `resolve_runtime*` are GONE from `wasm_host.rs`, replaced by generic `ComponentDep`/`declared_deps`/
> `resolve_deps` (§23b). PR#1013 (3) split `DepMissing`/`DepStoreError`, (4) lowercase-hash enforced, (5)
> fixture comment reconciled — all folded in. This subsection is kept as the before-state record.

All in `wasm_host.rs`, from the §21b slice (`4614bdb75`, landed) — this is exactly the machinery the
operator is pointing at, and PR#1013's Copilot findings (1)/(2) are symptoms of the same hard-coding:
- `const RUNTIME_IFACE: &str = "cadenza:runtime/heap"` — a **named, kernel-baked identity** for one
  specific dependency. This must go.
- `required_runtime()` scans imports for `starts_with(RUNTIME_IFACE)` and returns a single `RuntimeReq`.
  This bakes in (a) that there is exactly ONE special dep, (b) its name, (c) that "heap" is meaningful to
  the kernel. PR#1013(1): it silently takes the FIRST of multiple matches; PR#1013(2): `starts_with`
  false-matches `cadenza:runtime/heap2@…`. Both dissolve once the mechanism is generic — there's no
  privileged prefix to match against or pick "the first" of.
- `RuntimeReq { import_name, hash }` + `resolve_runtime()` / `resolve_runtime_bytes()` — a bespoke
  "resolve THE runtime" path. Replace with a generic `ComponentDep { import_name, hash }` list and a
  `resolve_deps(&[ComponentDep], &dyn BlobStore)` that resolves ALL declared deps uniformly.

### 23b. The generic mechanism (replacement)
1. **Declare:** a reducer component's imports carry their dependency's content address as the import
   name's build-metadata (`+<hash>`), per component-abi.md v3 — but the kernel reads this for ANY
   dependency import, not a name-matched one. The set of "imports that carry a `+<hash>` content address"
   IS the declared dependency set. (Imports the host itself satisfies — the `kv` host import — are not
   content-addressed deps; the distinction is "does this import name carry a `+<hash>`," not a name
   allow-list.)
2. **Resolve:** for each declared dep, fetch its component bytes from CAS by that hash. Missing dep →
   error (can't run a half-linked reducer) — but a GENERIC "dependency `<hash>` unresolved," naming no
   runtime.
3. **Link:** compose each resolved dep component into the linker under its declared import name. Uniform
   for all deps; transitive deps (a dep that itself declares deps) resolve by the same recursion.
The kernel never asks "is this the Cadenza runtime?" — it asks "what does this component declare it needs,
and can I fetch + link each by hash?"

### 23c. PR#1013 findings — folded into this refactor (don't patch the doomed code)
The generic refactor SUPERSEDES the narrow PR#1013(1)/(2) fixes — don't patch `RUNTIME_IFACE` prefix
logic that's being deleted. The orthogonal PR#1013 findings still get fixed alongside:
- **(3)** `RuntimeUnresolved` conflates "missing by hash" with a blob-store IO error → split into
  `DepMissing { hash }` vs `DepStoreError { hash, source }` so callers can distinguish (a genuine bug in
  the generic mechanism too).
- **(4)** `parse_hash_hex` accepts uppercase though the doc says lowercase → enforce lowercase (content
  addresses are canonical lowercase hex; accepting both invites two spellings of one address).
- **(5)** fixture `reducer-guest/Cargo.toml` header says `wasm32-wasip1` but the regen recipe + CI build
  `wasm32-unknown-unknown` → reconcile the comment (the target IS unknown-unknown; WASI imports break
  `component new`).

### 23d. Bootstrap check — no chicken-egg (raised + cleared)
Removing the hard-coded runtime does NOT strand the kernel: the genesis/bootstrap program is itself a
content-addressed component put into CAS at init (§16b-E bootstrapping); its deps (if any) resolve by the
same generic path. Nothing about bring-up requires the kernel to KNOW a specific runtime — it only
requires the deps to be present in CAS by the time a component that declares them is folded (a reducer's
deps are put into CAS before/with the reducer). No genuine dependency the kernel can't express generically
was surfaced; if one appears during the refactor, RAISE it to concierge rather than reintroducing a
hard-code. **Net:** §21b's dependency ORDER (CAS → linking → real-Cadenza) stands; its "resolve THE
runtime" FRAMING is replaced by generic declared-dep resolution.
