# Design — the Cadenza agent runtime: a log-native agent operating system

**Author:** design-agent-runtime (interactive design agent, operator-directed 2026-07-16).
**Audience:** the operator; v-agent-harness (owns implementation, builds increments *against* this
vision); v-verification (proof-carrying governance); v-effects / v-peer-linking (the capability
boundary); v-inference + the compiler-port vertical (compiler-as-tool, the query DB); the concierge.
**Status:** 🔵 **VISION DOC — the big picture, not an increment plan.** This is the north star the
existing `DESIGN-agent-harness.md` builds toward. That doc is implementation-grounded (Inc 0–3 shipped:
the Bedrock embedder, the Cedar authorizer, the Cadenza loop package) and stays the authority on *what
is built*; this doc is the *why* and the *where this goes* — deliberately ahead of what exists, shaped
live with the operator. Written against `trunk` @`633bbe0dc`.

> **Operator framing (2026-07-16, verbatim intent):** "Just build hivemind here. Totally stateless
> agents that aren't bound to a single machine — their whole existence is on the log, their entire
> governance dictated by Cadenza smart contracts. A very small minimal core with root capabilities for
> the host, that tails the log and folds events using a Cadenza program which invokes other Cadenza
> programs. The whole runtime as minimal as possible. Agents make proposals to add tools to the log
> that other agents can invoke; those expose capabilities that constrain the root runtime down. The
> Cadenza compiler itself is a tool that can compile/typecheck/query other Cadenza programs. This single
> instance handles context threading of every invocation and schedules Bedrock calls."

---

## 0. The one-sentence vision

**An agent operating system where the log is the disk, a minimal Rust microkernel holds all ambient
authority and does nothing but fold the log with a Cadenza program and execute the effect-requests that
fold returns, and everything with judgment in it — agents, tools, governance, coordination, memory —
is a Cadenza program on the log, sandboxed by the compiler's effect discipline and governed by
proof-gated smart contracts.**

This is [hivemind](https://github.com/camshaft/hivemind) (`/tmp/hivemind-ref`), built *here*, and made
**smaller and more radically log-native** than the reference implementation — because Cadenza's effect
system, metaprogramming, and verification kernel let capability, governance, and self-modification be
*language* facts instead of infrastructure.

**Why this is credible and not just ambitious:** the coordination model at its heart is not
speculative — **the Cadenza fleet has been running on it for months.** `fleet send` / `merge-request` /
`reject` / `assign` / `ask`→`answer` is a working, sustained demonstration that a pool of agents
coordinating purely by typed messages can build a real compiler with no central brain. This vision
takes that proven model and gives it its proper substrate: one indexed immutable log instead of
files-plus-tmux-plus-memory-markdown. The reader can look at the fleet and see it already half-works.

---

## 1. The mental model: an operating system, and what plays each part

| OS concept | In this runtime |
|---|---|
| **The disk** | The immutable, append-only, **indexed event log**. Every agent's *entire existence* lives here. |
| **The microkernel** | A minimal Rust core holding ambient authority (host root, AWS creds, the Bedrock edge, the wasm engine, the fold-owner lease). It has no judgment; it interprets effect-requests. |
| **A process** | A **stateless agent** = `(identity + a Cadenza program + its slice of the log)`. Runnable on *any* host by re-folding — never pinned to a machine. |
| **Userspace** | Every Cadenza program: agents, tools, governance contracts, reducers, reporters, the compiler-as-tool. Extend the system by adding these, never by growing the kernel. |
| **The CPU / scarce compute** | **Bedrock** (model calls) and the **decoupled compute pool** (Rust builds, big compiles, tests). Scheduled by the kernel, executed off-thread. |
| **syscalls** | **Effects.** A program requests the world (`Model.converse`, `Aws.…`, `Log.append`, `Compile.check`) as a typed effect; the kernel services it under attenuation. |
| **The scheduler** | The single-threaded **fold owner** (a leased role) that tails the log, folds it, and dispatches **subscription programs**. |
| **Capabilities / permissions** | **The Cadenza compiler.** A program can only cause effects that appear in its effect type; the compiler *is* the sandbox (§4). |
| **The init system / desired state** | The log's **roster**: which agents *should* be running, reconciled against which are. |

The load-bearing inversion: **almost nothing is in the kernel.** The kernel tails, folds, and executes
effect-requests. The fold — the thing that decides what happens next — is a Cadenza program, and it
invokes other Cadenza programs. This is the "very small minimal core" the operator asked for.

---

## 2. The spine: CQRS, but write and fold are decoupled

Hivemind's spine is CQRS / event sourcing: one immutable log is the source of truth; everything else is
a deterministic **fold** (projection) over it. We keep that and sharpen the concurrency story into two
independent planes.

### 2.1 The write plane — many-writer, DynamoDB (ordering authority)

**Anyone appends, concurrently, without coordinating.** Agents, hosts, finishing compute workers, the
Slack adapter, operator actions — all append. **DynamoDB is the ordering + dedup authority**: an atomic/
conditional write assigns each event a monotonic `seq`, so N concurrent appends get a total order and no
duplicates *without the writers coordinating with each other or with the folder*. This is exactly what
DynamoDB is good at, and it is why the write side is decoupled from the fold side — a single-threaded
appender would bottleneck the high-fan-in of results, messages, status pings, and capacity ads.

### 2.2 The fold plane — single-threaded owner (the simplification)

**One leased single-threaded owner tails the ordered log and folds it.** It does *not* gate writes; it
is the one reader-of-record that holds an always-fresh materialized view and drives the loop. Why single-
threaded folding is the right call:

- **No stale-view races in the decision path.** The reason hivemind needs the task **lease + fencing
  token + conditional claim** is that two *stale distributed folders* could both decide "this task is
  open, I'll claim it." With **one** folder making assignment decisions, that race cannot exist — the
  fold picks the next runnable work. **The entire per-task claim/lease/fencing machinery is deleted.**
  (We keep exactly *one* lease: "who is the current owner," so the role survives a host dropping — §6.)
- **Read-your-writes and the ~1–2s consistency window vanish for the owner** — it folds its own tail.
- **Throughput is a non-issue** because the fold is cheap metadata work; everything *expensive* (model
  calls, builds) is dispatched *off* the owner thread and returns as events the owner folds (§5). The
  owner never blocks — it is an event-driven dispatcher.

**Net vs. the reference implementation:** N per-agent daemons → one folder; per-task claim races → one
decider; but we *keep* the many-writer DynamoDB log because concurrent append is genuinely useful. The
distributed-systems complexity (claim races, fencing, stale-view reconciliation) is deleted; the good
part (ordered many-writer append) is retained. **The change feed** (DynamoDB Streams, ~1s poll fallback)
now feeds *one* folder instead of N — cheaper than the original, not more expensive.

### 2.3 Determinism survives non-determinism, because the world is a recorded effect

The fold must be deterministic to replay/fork/hand-off. Model calls, builds, and clocks are not. The
resolution — already how `cdz-run`'s `RunOpts::host_responses` works — is that **every non-deterministic
touch is an effect-request the fold emits, and its result is appended as an immutable event.** Live: the
kernel performs the call and appends the response. Replay: the recorded response is reused. Non-
determinism is quarantined at the kernel edge and instantly frozen into the log, so the fold over
`(request-event, response-event)` is pure. This single trick is what makes cognition replayable.

---

## 3. Stateless agents = re-fold; the log is their whole existence

An agent has **no session state**. It is an identity, a program, and its slice of the log. To *run* it,
any owner on any host folds its slice into a context and schedules a model call; the result is reified as
events. Consequences the operator asked for, made mechanical:

- **Not bound to a machine.** Resuming an agent is *re-folding* — so a host dying loses nothing; another
  host re-folds and continues. There is no local agent memory to lose.
- **Forkable / hand-off-able cognition — via re-fold, NOT continuation capture.** Because context is a
  fold over a log slice, you fork an agent by folding the same slice into two futures, hand a task
  between agents by appending a `MESSAGE` with the refs (the receiver folds them), and **time-travel
  debug** by folding up to seq N. Crucially this needs **no multi-shot continuation** — Cadenza's
  effects are single-shot tail-resumptive (v-agent-harness constraint #2), so "resume this agent
  elsewhere" is *re-run the fold from the log*, which is exactly what a stateless agent already is. The
  log *is* the continuation. This is a case where the log-native design and the shipped effect model
  agree rather than fight.
- **The kernel owns context threading, which is *why* agents are stateless.** The owner — not the
  agent — assembles each invocation's context by fold and owns the Bedrock budget. The agent carries
  nothing between turns; it is reconstituted fresh every time. This is the operator's "single instance
  handles context threading of every invocation and schedules Bedrock calls."

---

## 4. Capability = effect type; the compiler is the sandbox

**Decision (operator, 2026-07-16): type-only capability enforcement.** We trust the compiler's effect
discipline as the security boundary; we do **not** reify each tool to a minted IAM role. This is the
minimal-kernel choice.

### 4.1 How attenuation works with no runtime permission machinery

The host runs with broad ambient authority (say AWS admin), **but never hands it out.** A tool is a
Cadenza program (an effect handler) that consumes the kernel's broad effect internally but **exposes only
a narrow effect to its callers.** Attenuation *is* handler composition:

- The kernel provides a root `Aws` effect (serviced with the real admin creds).
- A **provisioner tool** handles `Aws` internally and re-exposes only `CreateEc2`. Its effect signature,
  checked by the compiler, is `… <Aws> … → … <CreateEc2> …`.
- Any agent that wants to create an instance is handed only `CreateEc2`. It **physically cannot express**
  `DeleteBucket` or a raw `Aws` call, because that effect is not in the type it was given. The compiler
  rejects the program at typecheck. No STS role, no IAM policy, no runtime permission check — the *type*
  is the permission.

This is the operator's EC2 example exactly: "wrapping the root admin permissions with a scale-down role
enforced by the Cadenza program; any agent that wants to invoke it needs only the scope of creating an
EC2 instance." Here the "scale-down role" is an **effect handler**, and the enforcement is the
compiler's effect-tracking soundness plus the wasm sandbox. **The TCB for capability is: the compiler's
effect soundness + the wasm boundary.** Nothing else.

**This is already largely true in shipped code** (v-agent-harness's Inc-2 loop): it declares `effect
Model`, `effect Tools`, `effect Cedar`, `effect Inbox`; a program that never declares/handles an effect
*cannot perform it* — there is no ambient `perform`. "An agent can only do what its effect-type grants"
is real *today*, at compile time, not a runtime check. It is the single best-supported pillar of this
vision, so lean on it — with two honest boundaries:

- **Granting a capability = installing a handler / binding a peer** (precedence: in-source default <
  compile `--bind` < in-program `handle`, nearer-handler-wins). So the type says *which* effects **can**
  occur; the **handler stack says who answers them**, and it is *dynamic* (installed at a program point),
  not pure static type-erasure. **The authority seam governance must gate is therefore the
  handler-install / peer-bind step** — which is exactly where the shipped Cedar authorizer already sits
  (authorize *before* dispatch). §7's governance gates that step, not an abstract type check.
- **Effect rows are NOT a full capability lattice today — no *type-level attenuation*.** The type system
  gives you "has `Model` vs. does not" — it does **not** give you a strictly-weaker `Model` (e.g. "`Model`
  but `max_tokens ≤ 100`") at the type level. A *narrower* capability like that is a **value-level guard**
  (as in the shipped fail-fast `max_tokens` check), not a compiler-enforced type. So §4's "narrow effect"
  means *a distinct, smaller effect* (`CreateEc2` instead of `Aws`) — coarse-grained attenuation the type
  system *does* enforce — **not** parameterized/refined attenuation of one effect, which stays value-level
  until (if ever) effect rows grow a subtyping lattice. The vision deliberately does not over-claim this.

### 4.2 This puts weight on the compiler — proof-gated expansion is the counterweight

Because "what capabilities exist and what a tool may expose" is enforced by the compiler, the dangerous
operation is not *invoking* a tool — it is *expanding the capability surface* (adding a new root effect,
widening what a shared tool exposes). §7 gates exactly that with a machine-checked proof. **Trust the
discipline; gate the changes to the discipline.**

### 4.3 The compiler-as-a-tool closes the self-extension flywheel

The Cadenza compiler is itself a tool an agent can invoke (compile / typecheck / query — the compiler-
port + `DESIGN-query-engine.md` substrate). So the loop that grows the toolset is:

1. An agent authors a new tool as Cadenza source (or as a quoted `Ast` via metaprogramming).
2. It submits the source as a **tool proposal** (a log event).
3. The **compiler-tool typechecks + compiles it** — rejecting it if it doesn't typecheck *or if it uses
   more capability than it declares* (the effect signature is the contract).
4. **Governance admits it** (§7), possibly requiring a proof for a capability expansion.
5. It is now content-addressed on the log and invocable by any agent holding the exposed scope.

The compiler is the gatekeeper that makes "agents grow the toolset" *safe*: nothing lands that doesn't
typecheck against its declared capability. And this is the self-hosted compiler running *inside* the
system it compiles for.

---

## 5. The decoupled compute pool — what makes single-threaded viable

The owner is single-threaded and must never block. So **everything slow is dispatched off-thread as a
request-event and folded back as a result-event.** This is the operator's "schedule an EC2 build
instance, keep a pool of decoupled compute, and intelligently route compute requests to hosts with
capacity."

- **Slow work is an effect-request.** A Bedrock call, a Rust build, a `cargo test`, a wasm compile of a
  submitted tool → the owner appends `COMPUTE_REQUESTED{kind, payload, capability, requirements}` and
  moves on. A worker on some pool host picks it up, runs it under its **attenuated capability** (§4), and
  appends `COMPUTE_COMPLETED{result-hash}` — *directly to DynamoDB, not through the owner* (§2.1). The
  owner folds it on the next pass. Latency of a heavy op is just "how long until the completion event
  appears," and the owner's thread is free throughout.
- **Routing is a fold, not a subsystem.** Hosts advertise capacity as events (`HOST_AVAILABLE{cpu, mem,
  arch, has-docker, region, internal-access}`); requests carry requirements; a **routing subscription**
  (§8) matches request→host by folding both streams. This is *identical machinery* to matching desired-
  agents→hosts (§6) — capacity routing and agent placement are the same reconciliation.
- **Elastic capacity is a governed capability-invocation.** No host with `linux/arm64+docker` capacity →
  the router invokes the provisioner tool's `CreateEc2` scope → a fresh host boots, advertises capacity,
  runs the build, and is **kept warm** for the next job until an idle-reaper subscription scales it back
  down. Scale-up and scale-down are both bounded by the exact `ec2` scope, never the root grant, and both
  are auditable events.
- **Why the pool is the answer to "the Rust build."** The runtime's own compute (folding, agent loops)
  is cheap and single-threaded; compiling Rust or running a big test suite is heavy and wants a real
  machine. Decoupling means the owner is *never* the bottleneck for those — it dispatches and folds. The
  pool is *what makes* the single-threaded owner viable; they are not in tension.
- **⚠ Two different "compiles" — do not conflate them (v-agent-harness constraint).** Authoring a
  **Cadenza tool** is an *in-process `rcdzc` library call* (Cadenza source → a content-addressed wasm
  component) — fast, no build-tool spawn, **not** a pool job. This is the **MVP self-extension path** and
  the compile-repair loop's engine (a structured CDZ diagnostic fed back to the model → it corrects →
  converges, already dogfooded). The heavy **compute pool** is for authoring **Rust** (e.g. a new embedder
  backend) or running big test suites — a *much rarer, heavier* path. So §5's pool is real, but the
  *common* case (agents growing the Cadenza toolset) is a cheap in-process call, not an EC2 build. State
  both; don't imply every self-authored tool needs a Rust build.
- **Content-addressing already gives the "pool artifact" shape.** The value-heap runtime is content-
  addressed today (`target/cadenza-store/<hash>.wasm`); a newly-compiled tool is just another content-
  addressed component. "Capacity-routed pool of artifacts" maps cleanly onto the existing store — no new
  addressing scheme.
- **⚠ First self-authored tool interfaces must respect the String-crossing asymmetry.** A compiled tool
  becomes a **peer** the loop routes to, and (constraint #1) a String *argument* to a peer op doesn't emit
  yet — only String *results* cross. So the first self-authored tools should be shaped **result-only /
  scalar-arg**, or go through an embedder-style host closure, until Route A (host-result ABI widening)
  lands. Design the tool ABI around what crosses, exactly as the model call had to.

---

## 6. The log as control plane: roster, status, and placement

The log is not only the agents' memory — it is the org's **control plane** and **observability plane**.

- **Desired state (the roster).** "These agents *should* be running" is an authoritative set of events
  (declared by the operator or a governance contract). This answers *how a stateless agent gets
  scheduled*: an agent is runnable when it is in desired-state and an event it subscribes to lands (§8) —
  most often **a message addressed to it** (§9).
- **Actual status is a projection, never a heartbeat table.** "The exact status of each agent" — last
  activity seq+ts, current owner-lease holder, what tool it is mid-invocation on, token budget consumed,
  last completion — is a *fold over that agent's own event trail*. It is always exactly reconstructable
  and never drifts, because it is derived, not separately maintained. **Liveness = "is its lease fresh,"**
  the same mechanism task-claiming uses today. "Know the exact status of each agent and query it" is
  `SELECT … FROM the-fold`.
- **The owner-lease** (the *one* lease we keep, §2.2) is "who is the current fold/driver," not "who may
  write." On owner-host death, the log is intact in DynamoDB (writes never depended on the owner); a new
  host acquires the lease, re-folds from a snapshot + tail, and resumes. A small, clean lease.
- **Placement is reconciliation** between desired-agents and available-hosts — the same fold as compute
  routing (§5).

---

## 7. Governance: normative folds and proof-gated expansion

Today hivemind's fold is *descriptive* (computes state). Ours is *normative* — a **smart contract** (a
Cadenza program) that can **admit or reject** events. A tool proposal, a capability expansion, a new
subscription: each is an event, and a governance contract decides whether it takes effect. This
generalizes hivemind's task state-machine ("can't complete an unclaimed task") into "can't invoke a
tool governance didn't admit; can't widen a capability without meeting the contract."

**Decision (operator, 2026-07-16): operator-as-genesis-constitution + proof-gate on expansion.**

- **Operator is the root of trust (single trusted team).** v1 governance is a Cadenza contract whose
  authority traces to the operator. No agent quorum/voting yet — but the mechanism (a normative fold
  admitting/rejecting proposal events) is general enough that voting is a *later contract swap*, not a
  re-architecture.
- **Capability-EXPANDING proposals must carry a machine-checked proof.** Adding a root effect, or
  widening what a `shared`-scope tool exposes, must be accompanied by a HOL-kernel `Thm` that the
  expansion preserves the stated invariants (e.g. "no reachable effect path from a shared tool touches
  `s3:DeleteBucket`"; "ack-before-drop still holds"; "the fold stays deterministic"). The kernel checks
  the `Thm` before admitting. **Proof-carrying governance is the sound form of "the collective rewrites
  itself"** — and it is the thing only this stack (compiler + effects + HOL kernel) can do. Ordinary
  proposals that do *not* expand capability need no proof (keeps authoring friction low).
- **Cedar is one contract, not the mechanism.** The shipped Cedar authorizer (Inc-3) is *a* governance
  contract; the general primitive is arbitrary deterministic Cadenza governance, of which Cedar
  evaluation is one instance.

---

## 8. Subscription programs: the one reactive primitive

"Extend by adding agents, not core code" needs a first-class way for a Cadenza program to say **"wake me
when events matching predicate P appear, and here's what I do."** That is a **subscription**:

```
subscription = { predicate over the event stream,  handler: a Cadenza program,  capability }
```

- **A subscription is itself a log event** (`SUBSCRIBE{predicate, program-ref, capability}`) — durable,
  auditable, governed (§7), revocable by supersession.
- **The owner's fold dispatches them.** The single-threaded owner already folds every event; matching
  subscriptions is *part of the fold* (event lands → which predicates match → schedule those handlers as
  compute/model invocations under the subscription's capability). Subscriptions need **no separate
  daemon or poller** — they ride the fold that already exists. This is why the single-threaded-owner
  architecture *enables* cheap subscriptions rather than fighting them.
- **Triggered programs call other programs — this is the composition core.** A handler is a Cadenza
  program that, in response to its triggering event, **invokes other Cadenza programs** (a tool, the
  compiler, another agent, or the model) via effects, and typically **appends new events** that in turn
  trigger *further* subscriptions. So behavior is not one monolithic loop but a **cascade of small
  programs reacting to each other over the log**: event → program → (invoke programs + append events) →
  those events trigger more programs → … . The operator's minimal core is exactly this — "fold events
  using a Cadenza program which invokes other Cadenza programs" — and the fold is the only kernel
  machinery the cascade needs. Chains are bounded and auditable because every link is an event (a run-
  away cascade is visible on the log and governable by a contract, §7).
- **It unifies what looked like separate features — a strong sign the abstraction is right:**
  - **The agent loop** is a subscription: "wake me on messages addressed to me."
  - **Auto-compaction** (§10) is a subscription: "wake me when a context projection exceeds budget."
  - **Insights-to-operators** is a subscription: "wake me on `merge-request`/`reject`/stuck events → post
    a digest" (hivemind's beat-reporter / ask-the-hive).
  - **The compute router** (§5) is a subscription: "wake me on `COMPUTE_REQUESTED` → match to a host."
  - **hivemind's standing-queries north star** (subscribe to a *question*, get pushed matching memories)
    is a subscription whose predicate is a semantic match — pull→push for knowledge, same machinery as
    pull→push for work.

**So the runtime's actual core, stated minimally:** *an indexed immutable log (many-writer via
DynamoDB), a single-threaded owner that folds it and dispatches subscription programs under attenuated
capabilities, and a decoupled compute pool for heavy work.* Agents, tools, governance, reporters,
compaction, routing, and messaging inboxes are **all subscription programs and folds.** That is the
whole thing.

---

## 9. Agent-to-agent messaging: the proven primitive (an event, but load-bearing)

Messaging is not a nice-to-have that "falls out of the log" — it is the coordination substrate the fleet
**has already validated in production.** Everything the fleet does is messaging: `merge-request` →
`reject` → `assign` → `ask`→`answer` → `note`. This vision must treat it as a first-class, proven pillar.

- **A message is an addressed, typed, durable event** — `MESSAGE{from, to, kind, subject, refs[], body}`.
  The `kind` is the fleet's earned vocabulary (`merge-request`, `reject`, `assign`, `ask`, `answer`,
  `note`, `issue`, `status`, `backlog`). Typed, so a handler pattern-matches it as a Cadenza value.
- **The inbox is a projection, not a queue.** "My unread" is a fold over `MESSAGE` events addressed to
  me; "mark processed" is an `ACK` event. This *deletes* the file-inbox machinery the fleet fakes today
  (JSON files in a hub dir + `processed/` moves + tmux nudges) — and with it a whole class of traps
  (`drain-inbox-via-fleet-inbox-resolver`, `fleet-send-needs-from-else-reply-lost`, hub-vs-worktree path
  confusion) that are *artifacts of not having a shared log.*
- **Reply-then-ack is durable and crash-safe for free.** The fleet's hard-won rule (reply before ack so a
  crash never drops a reply) is native: the reply event is appended *before* the ack event; a crash
  between them leaves the reply landed and the message un-acked, so it is re-driven. No message is ever
  lost.
- **Delivery-wakes-recipient becomes a subscription, not a tmux `send-keys`.** A message addressed to
  agent B is exactly the wake the owner uses to make B runnable (§6, §8). **An addressed message is a
  scheduling event** — this is the cleanest driver of agent scheduling.
- **The human is just another addressable identity.** `ask`→concierge→operator→`answer` is agent↔agent
  messaging where one participant is a person reached through the Slack adapter. The "await event"
  semantics come free from the log (no workflow engine) — and this is literally the relay that shaped
  *this document*.

---

## 10. Context lifecycle: rollback, hoisting, and log-keyed auto-compaction

Because context is a fold, keeping it clean is a **first-class, testable, governable fold** — not ad-hoc
pruning. Three mechanisms, one primitive (a **context-reduction fold** that projects a bounded log span
`[seqA..seqB]` down to its resolved outcome, keyed by a marker event; the raw span is **never deleted**,
only elided from the context projection):

- **Semantic hoist (the type query).** An agent asks the compiler-tool for a type. Instead of feeding the
  tool-call/tool-result exchange back into the model, the kernel **rolls back the exchange and injects
  the type as a durable premise** into the system-prompt "known facts" block. The reasoning that led to
  asking is elided; only the fact survives — cheaper (a premise, not a turn) and it does not teach the
  model "ask again."
- **Failure elision (the compile-repair loop).** An agent writes program P, it fails, it repairs, loops
  N times, P′ finally compiles+runs. Reduction folds the whole loop to one clean fact: *"authored P′; it
  compiled and ran."* The N−1 broken attempts stay on the log (auditable; available to a *learning* fold
  that mines recurring mistakes) but never enter the working context.
- **Auto-compaction (budget-triggered) — deterministic-on-replay even though summarization isn't.** When
  a context projection exceeds a budget, an auto-compaction **subscription** (§8) fires. Summarizing 40
  turns into a paragraph means *calling the model* — non-deterministic — so, exactly like the Bedrock
  call (§2.3), **the summary is itself a recorded event**: `COMPACTED[seqA..seqB] → summary-hash`, frozen
  immutably. Live: call the model, append the summary, use it. Replay: reuse the recorded summary. Auto-
  compaction and manual rollback are the *same* reduction fold with different triggers (budget vs.
  semantic-completion).

**Soundness under supersession (operator decision, 2026-07-16): keyed to log, degrades to cache-miss.**
A hoisted premise or a compaction summary is *itself a projection keyed to its source span*. If a later
event supersedes the underlying span (the type of `foo` changed), the fold **invalidates it and re-
derives** rather than trusting a stale premise. So a reduction can never make context *wrong*, only
*stale-then-refreshed* — which is the exact context-hygiene lesson the CodeAct spike already learned
(`[[cadenza-agent-harness-codeact-spike]]`), now generalized and made first-class.

---

## 11. The indexed, queryable log (the read plane)

"The whole log indexed so an agent can query over it and find what happened" has a strong existing home:
the **lazy query-DB substrate** the compiler-port vertical is already building (`[[port-compiler-to-
cadenza-ml]]` — mirror rcdzc `db.rs` columns + backward-demand memoization) plus `DESIGN-query-engine.md`.
That backward-demand memoized query DB *is* an incremental-projection engine — which is what a log index
is. So we build the log index **on the query DB the compiler port is already building**, not on new
infrastructure:

- **Metadata tier** (small, structured, always local: seq, actor, kind, scope, refs, status, snippet) —
  what queries filter/rank/traverse on. Materialized as the query DB's columns.
- **Body tier** (full transcripts, attachments) — content-addressed in S3, lazily fetched, cached
  forever (immutability → caches never go stale, identical bodies dedup).
- **Query is a tool.** The compiler-as-tool already has a "query other Cadenza programs" verb; the log
  query surface is the same engine pointed at the event projection. An agent asks a typed query; results
  come back as Cadenza values. Insight-to-operator reporters (§8) are just query subscriptions.

---

## 12. Why Cadenza (the thesis a Python/TS framework structurally cannot match)

The pure reasoning-structure (the fold + decision logic) is separated *at the type level* from every
outside touch (model, tools, clock, memory, randomness, AWS) as **effects**. What that unlocks, free
here and impossible elsewhere:

- **Mock-vs-live handler stacks** for replay/test/simulate at zero cost (the fold doesn't know or care
  whether `Model.converse` is Bedrock or a recorded response).
- **Tool schemas are Cadenza types**, so a mis-shaped tool call is a *compile error* fixed by the
  compile-repair loop, not a runtime crash.
- **Capability = effect type** (§4) — the sandbox is the compiler, so the kernel stays minimal.
- **Units/quantities carried through numeric reasoning** — no unit-confusion bug class in an agent's
  arithmetic (the CAD/units work already exists).
- **The killer app: proof-carrying self-modification** (§7) — a self-modification that carries a HOL
  `Thm` proving the new state preserves an invariant. Only compiler + effects + verification kernel
  together make this expressible.

---

## 13. The fleet-convergence north star

This runtime is the substrate the fleet itself converges onto (operator-confirmed: build-first, migrate-
later). The fleet's inbox / `merge-request` / pr-sync / roles / tick-loop are re-expressed as agents,
messages (§9), a roster (§6), governance contracts (§7), and subscriptions (§8) over the log. We are not
building an agent framework — we are building the durable, replayable, self-organizing version of the
collective the fleet *already is*, run today by hand with files and tmux. The single most compelling
proof that the vision works is that its coordination core is already in production.

---

## 14. What is genuinely NEW vs. already in the tree

~80% of this is **composition of shipped parts under the microkernel framing** — which is what makes it
buildable rather than a moonshot:

| Pillar | Already in the tree |
|---|---|
| Capability = effect type | Effect system + handler stacks (v-effects); nearer-handler-wins. |
| Recorded-effect determinism | `cdz-run` `RunOpts::host_responses` (record/replay host responses in call order). |
| The Bedrock edge | `cdz_run::run_agent` embedder + `cdz-agent` (Inc-1). |
| One governance contract | The Cedar authorizer (Inc-3). |
| Proof-carrying expansion | The HOL-Light LCF kernel (v-verification), FEATURE-COMPLETE. |
| Compiler-as-tool | The compiler port (`[[port-compiler-to-cadenza-ml]]`) + `DESIGN-query-engine.md`. |
| The log index / read plane | The lazy query DB (rcdzc `db.rs` mirror) being built by the compiler-port vertical. |
| Authoring tools as data | Metaprogramming quote/eval/`Ast` (v-metaprogramming). |
| The agent loop package | `implementation/agent-harness/` (Inc-2). |
| Messaging vocabulary | The **live fleet** (`fleet send` kinds). |

**Shipped constraints this vision deliberately designs around** (from v-agent-harness, the impl owner —
these are real trunk limits, not speculation, and the vision is compatible with each):

- **String-crossing ABI is asymmetric** — a Cadenza String crosses a boundary as a `u32` handle into the
  shared value-heap runtime; a peer op can *return* a String but a String *arg*/host-result/entrypoint-
  escape don't all emit yet. This is *why* the Bedrock edge is a Rust embedder (option c), not a pure
  Cadenza peer. The vision keeps the model call as the one non-Cadenza edge (§2.3) and treats Route A
  (widen the host-result ABI) as the durable collapse-to-pure-Cadenza fix — not a blocker.
- **Effects are single-shot tail-resumptive** — no captured/replayed continuation. The vision's fork/
  hand-off/replay is **re-fold from the log**, not continuation capture (§3), so it needs nothing the
  effect model doesn't already give.
- **Self-mod needs rcdzc-as-a-dep + quote/eval + a compile-repair loop** — "author new tools with a
  fixed loop" is the cheap rung; "the agent rewrites its own loop" is a much bigger lift + proof story.
  The §15 ladder sequences accordingly (L4/L8 author-tools before any loop-rewrite), matching the self-
  mod ceiling.
- **No Cadenza-native TLS/SigV4/HTTP yet** — any direct network capability beyond the embedder's SDK
  call is future v-runtime work, off the critical path.

**The genuinely new builds, small and nameable:**
1. **The single-threaded fold owner** (tail DynamoDB → fold → dispatch subscriptions → execute effect-
   requests) — the microkernel loop.
2. **The normative governance fold** (admit/reject proposal events; proof-check on expansion).
3. **The context-reduction fold** (rollback / hoist / log-keyed auto-compaction).
4. **The stateless-agent-as-refold loop** (context threading owned by the kernel).
5. **The subscription primitive** (`SUBSCRIBE` event + fold-time dispatch).
6. **The decoupled compute pool** (request/complete events + capacity-routing subscription + governed
   elastic provisioning).

---

## 15. An honest increment ladder (toward the vision, from what is shipped)

This doc is a north star; v-agent-harness owns sequencing. A *plausible* ladder that keeps every rung
useful (each reports language gaps REPORT/FIX, not work-around; each lands with gate coverage):

- **L0 — shipped.** Bedrock embedder, Cedar authorizer, the Cadenza loop package (Inc 0–3).
- **L1 — the fold owner over a real log.** A single-threaded Rust owner that tails a DynamoDB log, folds
  it with a Cadenza program, and drives one agent loop end-to-end (reusing the embedder for the model
  call). Proves the microkernel shape + recorded-effect determinism against a real log.
- **L2 — messaging + inbox as a fold.** Re-express one fleet interaction (`merge-request`/`reject`) as
  `MESSAGE`/`ACK` events with the inbox as a projection. The first fleet-convergence dogfood.
- **L3 — subscriptions.** The `SUBSCRIBE` event + fold-time dispatch; re-cast the agent loop and one
  reporter as subscriptions. Unifies scheduling.
- **L4 — capability = effect type, end to end.** A tool that exposes a narrow effect over a broad kernel
  effect; a program requesting more than its scope is a compile error. The provisioner/`CreateEc2`
  worked example.
- **L5 — the compute pool.** `COMPUTE_REQUESTED`/`COMPLETED` + a capacity-routing subscription; run a
  real off-thread build; keep a warm pool; governed elastic provisioning.
- **L6 — normative governance + proof-gated expansion.** Tool proposals admitted by a governance fold;
  capability expansions require a HOL `Thm`.
- **L7 — context lifecycle.** Rollback/hoist + log-keyed auto-compaction subscription.
- **L8 — self-hosting the read plane + compiler-as-tool.** Log index on the query DB; agents author +
  submit + compile new tools; standing queries.
- **L9 — fleet migration (deliberate, proven).** Migrate fleet roles onto cdz-agent instances — the
  substrate cutover, build-first/migrate-later.

Rungs may reorder or merge; the invariant is that each is independently useful and gated.

---

## 16. Open decisions with chosen defaults

- **Capability boundary (§4):** ✅ **type-only** (compiler is the sandbox). Reified IAM is *not* built;
  it stays on the shelf if a future lower-trust mode ever needs a runtime backstop.
- **Governance v1 (§7):** ✅ **operator-as-genesis + proof-gate on expansion.** Agent quorum/voting is a
  later contract swap.
- **Scheduler shape (§2.2, §6):** ✅ **single leased owner, any host holds it.**
- **Compaction/hoist soundness (§10):** ✅ **keyed to log, degrades to cache-miss** (never wrong, only
  stale-then-refreshed).
- **Self-mod ambition (§7 / §4.3):** build load-a-tool → rewrite-own-handlers → rewrite-own-loop; proof-
  required for anything above "load a tool." (Rewrite-own-*goals* deferred.) Confirm the exact ceiling
  for the first cut with the operator when L6 approaches.
- **Still open (leaf-level, don't affect the shape):** snapshot cadence for owner re-fold on failover;
  the exact predicate language for subscriptions; whether a `BLOCKED`-on-human agent holds or releases
  attention budget; the S3-vs-query-DB split for the body tier at scale.

---

## 17. Coordination

- **v-agent-harness** — owns implementation; builds increments against this. `[[agent-harness-vertical-
  log]]`.
- **v-verification** — proof-carrying governance / self-mod (§7); the HOL `Thm` admission check.
- **v-effects / v-peer-linking** — the capability-as-effect-type boundary (§4); handler-stack attenuation.
- **compiler-port vertical / v-inference** — compiler-as-tool + the query DB that becomes the log index
  (§11). `[[port-compiler-to-cadenza-ml]]`.
- **v-metaprogramming** — authoring tools as data; the compile-repair loop (§4.3, §10).
- **fleet-orchestration** — the convergence target (§13); the messaging vocabulary (§9).

## 18. References

- Hivemind (`/tmp/hivemind-ref`): `VISION.md`, `ARCHITECTURE.md`, `DECISIONS.md`, `BUILD_SPEC.md`.
- The implementation-grounded sibling: `implementation/design/DESIGN-agent-harness.md` (Inc 0–3 shipped).
- Query engine / self-hosted query: `implementation/design/DESIGN-query-engine.md`.
- CodeAct spike (context-hygiene lessons, the String-ABI history): `[[cadenza-agent-harness-codeact-
  spike]]`.
- Effects / host delegation: `implementation/design/DESIGN-effects-rcdzc.md`.
- Recorded-effect determinism: `implementation/seed/crates/cdz-run/src/lib.rs`
  (`bind_host_imports`, `RunOpts::host_responses`).
