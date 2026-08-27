# Outside-world I/O for the platform: a streaming effect DAG behind privileged event reducers

**Status:** design/scoping — nothing landed. Shaped 2026-08-26 by the `design-wasi` fleet agent with
the operator interactively, informed by `v-platform` (the platform host/ABI owner). Hands a
vertical-ready plan to the platform vertical. File anchors are landmarks at `origin/main` `4fac6ce37`.

> **Operator directive (verbatim).** "The platform needs to start adding support for wasi instead.
> Those interfaces should only be available for event reducers."
>
> **How the design session reshaped it.** Exploring the directive surfaced that adopting WASI as a
> guest interface buys little and costs a lot: WASI calls block, they do not stream, they do not
> compose into pipelines, and putting nondeterministic I/O inside the deterministic event-reducer fold
> fights the platform's replay guarantee. The operator's own reframing, verbatim across the session:
> "what could be really interesting is if we emitted regular requests and handled them like other
> requests in the event reducers. then you could stream request bodies or responses ... and we could
> build a DAG that the platform carries out ... what if i want to pipe an http request to a file? or
> ... a file into a shell command and then the output of that goes into an http body? ... it would be
> great if the event reducer only got a notification when things happened that it was interested in";
> "or even pipe this file into this other reducer and then that reducer would pipe its output and you
> could redirect that to wherever you want"; "the platform needs to know how to operate on these. we
> can't keep it generic all the way down. but hopefully there's few (and generic) enough of them that
> we don't need to expand it infinitely."

The design that follows is that reframing. Outside-world I/O is modeled as a **streaming dataflow
graph the platform carries out** — stages wired by named pipes — rather than as WASI calls a reducer
makes. WASI is demoted to a minimal direct-call convenience for trivial one-shots and, otherwise, an
internal implementation detail. The whole surface is a privilege of the event reducer.

---

## 1. What this is

A reducer today has no way to reach the outside world. This design gives it two ways, both confined to
the platform's privileged **event reducers** (§6), with the graph as the primary model:

1. **The streaming effect DAG (primary).** An event reducer describes a graph of I/O **stages** —
   an HTTP request, a file, a shell command, a socket, or another reducer — wired together by **named
   pipes** (streaming channels). Two WIT APIs govern it (§3): it **submits** the graph and gets back a
   **token** (an opaque handle to the now-running DAG, used to **cancel** it and to correlate its
   results); the platform carries the graph out, streaming bytes stage-to-stage internally; and the
   running DAG **emits events back** into the reducer, which handles the ones it **explicitly
   subscribed to** and eventually **responds to the invoker** with what happened. The reducer never
   blocks and is never in the byte path unless it chooses to be. This is how anything composed or
   streamed is done — piping an HTTP response into a file, piping a file through a shell command into an
   HTTP body, piping a file into another reducer and redirecting that reducer's output wherever.

2. **Direct one-shot calls (secondary, minimal).** For a trivial one-shot need — reading the
   wall-clock time, drawing randomness — an event reducer may call a small, fixed set of WASI
   interfaces (`wasi:clocks/wall-clock` and `wasi:random`) directly. Their results are journaled so
   the fold stays deterministic (§7). This is a facility *event reducers* hold, not a built-in
   platform effect: there is no `now` and no time primitive in the kernel. Time is not special — a
   reducer that wants the time emits a request against a time *contract* that an event reducer
   answers, exactly as it would for any other capability, and the event reducer reads the wall clock
   to answer it. `monotonic-clock` is deliberately excluded (§7): a monotonic instant is a local,
   arbitrary-epoch value with no meaning across nodes or across a session that migrates, so it does
   not cohere in a distributed system.

Both are a privilege of the event reducer, enforced structurally (§6). An ordinary reducer holds
neither; it reaches the outside world only by emitting an effect that an event reducer answers.

### Why the DAG rather than WASI-as-guest-imports

The alternative first considered — event reducers importing `wasi:http`/`wasi:filesystem`/etc. and
calling them directly — was set aside because it is worse on every axis this system cares about:

- **Blocking.** A WASI call blocks the reducer's fold until it returns. A graph streams stages
  concurrently and the reducer reacts to notifications; nothing blocks.
- **No composition.** WASI gives no way to wire one capability's output into another's input. The
  operator's motivating cases (HTTP into a file, a file through a shell into an HTTP body) are exactly
  composition, and are graph edges here.
- **Determinism.** Putting nondeterministic WASI calls inside the deterministic event-reducer fold
  fights the replay guarantee (vision §9) and would need a full streaming record/replay shim. With the
  DAG the reducer's fold only emits the graph request and folds recorded results/notifications; the
  nondeterminism lives in the platform's execution and enters the log as recorded events (§7). No shim.

So "what does WASI get us" as a guest interface is: little. It survives only as the minimal one-shot
convenience above and as a possible internal implementation choice for how the platform performs an
HTTP or file stage (§5).

---

## 2. What exists today

The platform host surface is `implementation/seed/crates/cdz-platform/wit/world.wit`. Its reducer ABI
is strongly-typed WIT with per-contract values carried as opaque bytes; a reducer folds events through
`on-message` / `on-response` / `on-notification`, emits `request`s, and ends with an `outcome`
(vision §3). Its imports are internal-substrate interfaces under the `cadenza:platform` package: `run`
(the pure fold primitive), `identity`, `blobs` (the content-addressed store, §8), `state` (the
key-value store, §7), and — for an event reducer — `graph` / `deliver` / `provenance` (the routing
substrate, §3/§4).

These are wired per reducer kind. `ReducerKind` (`host.rs`) is `Pure`, `Ordinary`, or `Event`, and
`add_host_imports(linker, kind)` (`host.rs:796`) builds each kind's capability set: `Pure` gets only
`run`; `Ordinary` adds `identity`/`blobs`/`state`; `Event` adds `graph`/`provenance`/`deliver`. There
is one `Linker` per kind (`pure_linker` / `ordinary_linker` / `event_linker`), and instantiating a
component against its kind's linker is what enforces its capabilities: an import the linker does not
wire cannot resolve, so the component fails to instantiate. There is no outside-world I/O and no WASI
anywhere today.

The event registry (`event_registry.rs`) is the trust root that resolves which event reducer governs a
contract; routing carries an ordinary reducer's effect up to that event reducer.

---

## 3. The streaming effect DAG

### Stages — a small, fixed, generic set the platform knows how to operate

A **stage** is a node with input and output streams. The platform must know how to carry out each
stage kind — this is the one place the system is deliberately not generic all the way down, because at
the point real I/O happens something must actually open the socket, exec the process, or issue the
request. The set is therefore fixed and kept small and generic, chosen so a handful composes into most
needs without proliferating:

- **file** — a source (read) or sink (write) over a path the stage is capabilised for. Its content is
  a stream.
- **http** — an outgoing HTTP request whose request body and response body are streams (so a body can
  be piped in or out rather than buffered whole).
- **shell** — a sandboxed subprocess: `argv` / `env` / `cwd`, with `stdin` / `stdout` / `stderr` as
  streams. This is the subprocess capability the vision names as one of the two genuine non-WASI
  residues (§3 of the vision); it is isolated here as a single stage kind the platform sandboxes.
- **socket** — a TCP/UDP connection whose read and write halves are streams.
- **reducer** — a wasm reducer acting as a stream processor: it consumes an input stream and emits an
  output stream. This is the generality escape hatch — arbitrary logic participates in a graph as a
  reducer stage — so the fixed set of stage *kinds* does not limit what a graph can express. It keeps
  the platform's stage vocabulary small (vision §1's "route by contract, no enumerated kinds" tension
  is reconciled: the executor knows a fixed set of node *kinds*, but the `reducer` kind admits any
  content-addressed program, so behavior stays open).

Adding a stage kind is a deliberate platform change, not a routine extension — the bar is that a new
kind is both genuinely primitive and generic enough to earn a permanent place in this set.

### Named pipes — the edges

A **named pipe** is a streaming channel that wires one stage's output stream to another stage's input
stream. Pipes carry the graph's topology: a stage names the pipes it reads from and writes to; the
platform connects producers to consumers. A pipe may fan out (one producer to several consumers) and a
consumer may be redirected to any sink — a file, an HTTP body, a socket, or another reducer — which is
the "redirect that to wherever you want" the operator described. The named pipe is the generalization
of a shell pipe, now typed by stream and able to connect any stage kind to any other.

### Two WIT APIs

The interaction between an event reducer and the platform's graph executor is **two WIT APIs**,
deliberately separate: one to submit and control a graph, and one over which the running graph emits
events back and the reducer responds. Only event reducers hold either (§6).

**API #1 — submit and control the DAG.** The management surface the reducer *calls*:

```wit
/// Submit a declarative graph spec to be carried out. Returns a token — an opaque handle to the
/// now-running DAG, scoped to the submitting event reducer — used to cancel it and to correlate the
/// events and final response it produces. (Async host call: the DAG starts; the token returns.)
submit: func(dag: dag-spec) -> result<dag-token, submit-error>;

/// Cancel an in-flight DAG by its token: the platform tears down its running stages and closes their
/// streams cleanly, releases resources, and reflects a cancelled outcome back through API #2.
cancel: func(token: dag-token);
```

The `dag-spec` is the declarative graph: the set of stages (each with its kind-specific configuration
and the pipes it reads/writes), the pipe wiring, and the set of events the reducer subscribes to be
told about (§below). The graph is submitted **whole**, not built call-by-call, which keeps it
replay-safe: the submitted spec is one recorded request, and the platform's execution of it is a
bounded, recordable unit. The DAG is **associated with the submitting event reducer** — it is that
reducer's DAG: its emitted events return to that reducer, its token is scoped to it, and only it may
cancel it. Dynamic behavior comes from **follow-on submission**: a reducer reacts to an emitted event
and calls `submit` again to continue the work — a long, adaptive workflow is a sequence of submitted
graphs, each atomic, correlated across the reducer's folds by their tokens, never a live-mutated graph.

**API #2 — the event/response loop.** As the DAG runs, it **emits events back into the submitting event
reducer** — each tagged with the DAG's token so the reducer knows which submission it belongs to. The
reducer handles the events it **explicitly subscribed to** (in the spec) by folding them through its
event entry points; unsubscribed events are not delivered, so it sees only what it asked for. The
milestones a DAG emits are its lifecycle: it started, a named stage began (the HTTP request opened), a
stage produced a subscribed value, a stage failed, the whole DAG finished, or the DAG was cancelled.
Working from those events, the reducer eventually **responds to the original invoker** — whoever asked
it to do this work — with the outcome, closing the loop (§4).

So the shape is: `submit(dag) -> token`; the DAG runs and emits token-tagged events back; the reducer
folds its subscribed events and may `cancel(token)`; and eventually the reducer responds to its invoker
with what happened. The token is the single correlator across all three (cancel, events, response).

### The byte path

The platform streams bytes **stage-to-stage internally**. The orchestrating event reducer that
submitted the graph is **not** in the byte path — piping an HTTP response into a file moves those bytes
inside the platform, never through the reducer's memory. A reducer is in the byte path only when it is
itself a `reducer` **stage**, and then only for that stage's own input and output streams. An
orchestrating reducer that wants to observe or inject bytes on a pipe does so by placing a `reducer`
stage on that pipe (a tap), not by default. This keeps the common case (wire A to B) free of reducer
round-trips and keeps folds bounded.

### Not blocking

Across both APIs the reducer never blocks and never polls. `submit` returns the token promptly (the
DAG starts running asynchronously); the reducer's fold returns; and the DAG's events arrive later,
folded through the reducer's event entry points (`on-notification` / `on-message`, already in
`world.wit`), only for the milestones it subscribed to. "Tell me when it's done" is a single
subscription; fine-grained progress is more subscriptions. This is why the model beats a reducer
calling WASI directly: a direct WASI call blocks the fold until it returns, whereas here the fold emits
`submit` and yields, and reacts to token-tagged events as they come.

---

## 4. Events, outcomes, and the response to the invoker

A stage produces two kinds of thing: the bytes that stream along its pipes (which stay inside the
platform unless piped to a reducer stage), and the **events** the reducer subscribed to — a milestone
reached, a subscribed value (an HTTP status, a shell exit code, the terminal contents of a pipe the
reducer asked to receive), a stage failure, DAG completion, or DAG cancellation. Every emitted event is
tagged with the DAG's token (§3, API #2) and delivered as a recorded event the reducer folds. A small
subscribed value (an exit code, a status) rides the event; a large one (a downloaded body it wants to
keep) is written by the platform into the content-addressed store and delivered as a hash (vision §8),
so it is never shuttled through the fold.

**The response to the invoker.** An event reducer usually runs a DAG because an ordinary reducer asked
it to — the ordinary reducer emitted an effect against a contract this event reducer governs (§6), and
that effect carries the invoker's continuation-token (vision §4). The event reducer submits the DAG,
folds the DAG's events it subscribed to, and when it has what it needs (the DAG finished, a stage
failed, or the DAG was cancelled) it **responds to the invoker** by answering that original effect —
correlated by the invoker's continuation-token — with what happened. So there are two correlations,
kept distinct: the **DAG token** ties the DAG's events/cancel/completion to the event reducer that owns
it, and the **invoker's continuation-token** ties the event reducer's eventual answer back to the
ordinary reducer that requested the work. A cancelled DAG produces a cancelled outcome the reducer can
turn into whatever answer the contract defines.

---

## 5. WASI's place: demoted

WASI is not the guest model for outside-world I/O. It survives in two limited roles:

- **The direct one-shot calls of §7** — `wasi:clocks/wall-clock` and `wasi:random`, held only by
  event reducers — a genuine but tiny use of WASI as a guest interface, for triviality's sake.
- **An internal implementation detail** — the platform's `http` and `file` stages must be implemented
  by *something*, and that something may be a WASI host the platform links internally, or plain Rust
  async libraries. This choice is invisible to guests and is the platform's to make per stage; it is
  not part of the guest ABI and imposes nothing on a reducer.

Everything the original "adopt WASI" framing would have exposed to guests (`wasi:http`,
`wasi:filesystem`, `wasi:sockets`) is instead a stage kind in the DAG.

---

## 6. Privilege and gating

The whole outside-world surface — both graph APIs (`submit`/`cancel` and the event/response loop),
holding the I/O capabilities the stages act through, and the direct one-shot WASI calls — is a
privilege of the **event reducer**, and the gate is structural, reusing the per-kind linker split
of §2:

- The two graph APIs and the direct one-shot WASI imports are wired into `add_host_imports`
  under the `ReducerKind::Event` branch only (`host.rs:817`). The ordinary and pure linkers wire
  none of it, so an ordinary or pure reducer that tries to import them fails to instantiate — denied by
  absence, not a runtime check, exactly as the privileged `graph`/`deliver`/`provenance` imports are
  today.
- On the guest side, only the event-reducer world declares these imports; rcdzc emits them only for a
  component targeting that world.

An event reducer is the trusted actor that enforces the capability policy over what a graph may do —
which paths a `file` stage may touch, which hosts an `http` stage may reach, whether a `shell` stage is
allowed at all — the resource-scoped capability model of vision §5. Ordinary reducers reach the outside
world only by emitting an effect against a contract that an event reducer answers; the event reducer
authorizes it and, typically, carries it out by submitting a graph. So the dangerous capabilities live
with one audited reducer kind and attenuate outward through contracts, never spreading ambiently.

---

## 7. Determinism and replay

The platform's guarantee is that a session fold is a pure function of `(event, state)` and that replay
never re-runs the outside world (vision §9). The two surfaces preserve it differently:

- **The DAG.** The orchestrating event reducer stays a deterministic folder: its fold only *emits* the
  graph-submission request and later *folds recorded* notification/result events. All nondeterministic
  I/O happens inside the platform's execution of the graph, and enters the reducer's log only as the
  recorded notifications/results it subscribed to (the `dispatched` / `result` mechanism the platform
  already has, vision §4/§10). On replay the reducer reads those recordings and the graph is never
  re-executed. No new determinism machinery, and no separate non-replayed reducer world is needed —
  the event-reducer-world reducer can remain the deterministic session folder it is today.
  - A `reducer` **stage** inside a graph that is a pure stream transform is itself deterministic; a
    stage that performs I/O is a boundary whose output is captured like any other stage's. Either way
    the orchestrating reducer only sees recorded outcomes.

- **The direct one-shot calls.** `wasi:clocks/wall-clock` / `wasi:random` called directly in the fold
  are nondeterministic, so the host **journals** each call's result into the reducer's log as it is
  made and returns the journaled value on replay. This is a minimal, scalar-only record/replay —
  bounded to a handful of small one-shot results (a `datetime`, a `u64`, a short `list<u8>`), not the
  streaming shim the WASI-as-guest-imports approach would have needed. It is the one place a
  record/replay exists, and it is small by construction. `monotonic-clock` is excluded: a monotonic
  instant is nanoseconds since an arbitrary, host-local epoch — not comparable across nodes and
  meaningless if a session migrates between nodes (the timer's arm event already records an absolute
  anchor precisely to avoid local-clock dependence, vision §6), so journaling it would faithfully
  reproduce a value that has no portable meaning. Only `wall-clock` (absolute Unix time) is exposed;
  elapsed-time is a difference of two wall-clock readings or is read off recorded timer fire times.

- **Async.** Nothing blocks the node's runtime thread. The DAG streams stages on the async executor;
  the direct one-shot WASI calls target Preview2 (component model) backed by async host functions
  (wasmtime fiber suspension), the same async-host model `world.wit` already states for every import
  ("all host imports are async ... invisible to the reducer"). Preview3-native async is not required.

---

## 8. Increments

A vertical lands these top-to-bottom, each independently green and proven end-to-end through the
conformance suite (§9). The internal substrate (state, blobs, identity, run, routing) and the
ordinary/pure worlds are untouched throughout.

- **Increment 0 — the direct one-shot floor: `wasi:random` + `wasi:clocks/wall-clock` (no
  `monotonic-clock`), event-reducer-gated, journaled.** The smallest surface, proving the gating (the
  event linker wires them; a test confirms an ordinary reducer importing them fails to instantiate),
  the async host wiring, and the journaling determinism path (a reducer reads the wall clock, the value
  is recorded, replay reproduces it). No graph
  yet — this establishes the privilege gate and the journaling mechanism the rest reuses.
- **Increment 1 — the graph spine and both APIs, with two stage kinds: `file` and `reducer`.**
  Introduce the two WIT APIs (`submit(dag-spec) -> token` and `cancel(token)`; the event/response loop
  with token-tagged events folded through `on-notification`/`on-message` for the subscribed set), the
  declarative spec, the named-pipe streaming substrate, node-to-node byte flow, and the eventual
  response to the invoker. Prove it on the two stage kinds with no external nondeterminism beyond the
  filesystem: pipe a file through a `reducer` stage into another file, subscribing to done/failed;
  submit returns a token; a second test cancels an in-flight DAG by token and asserts the cancelled
  outcome + clean stream teardown. This lands the whole graph machinery — submit, token, cancel,
  event/response loop — on the simplest stages before the networked ones.
- **Increment 2 — `http`.** Add the HTTP stage with streaming request/response bodies. Prove piping an
  HTTP response body into a file and piping a file into an HTTP request body; large terminal bodies land
  in the CAS and are delivered as hashes (§4).
- **Increment 3 — `shell`.** Add the sandboxed subprocess stage (`argv`/`env`/`cwd`, streaming
  `stdin`/`stdout`/`stderr`). Prove the operator's pipeline: a file into a shell command whose output
  goes into an HTTP body. This is where the subprocess sandbox and its capability enforcement land.
- **Increment 4 — `socket`.** Add TCP/UDP stages, reusing the streaming substrate.

The capability policy an event reducer enforces over a graph (which paths, hosts, and whether shell is
permitted — the §5 authz middleware, e.g. Cedar-backed) is a separate userspace concern layered on
top; these increments provide the stages, the gate, and the recording, not the policy.

---

## 9. The gate that protects it

Per the operator's standing rule that any behavior driven through the platform (spawn / route / run /
dispatch) is covered by the conformance suite and never a Rust `#[test]`, each increment's end-to-end
coverage lives in the platform integration-test harness (owned by `v-platform-itest`): a Cadenza-AST
harness runs an event reducer that submits a graph (or makes a direct one-shot call) to quiescence, the
recording captures the events, notifications, and host calls, and the Cadenza checker asserts them.
Per increment the suite asserts:

- an event reducer instantiates with the new imports and the operation runs;
- an ordinary reducer declaring the same imports fails to instantiate (the privilege gate holds);
- the operation's outcomes reach the reducer only as recorded notification/result events (or journaled
  one-shot results), so replay reproduces them without re-running the outside world (§7).

Host-crate correctness (the linker wiring, the stage implementations, the pipe substrate) is covered by
`cargo xtask dev-gate` on `cdz-platform` during development, with the conformance suite as the
behavioral authority. Rust unit tests stay only for no-platform-drive pure-function invariants.

---

## 10. Seams and file anchors

At `origin/main` `4fac6ce37`, all under `implementation/seed/crates/cdz-platform` unless noted:

- `wit/world.wit` — add, to the event-reducer world only: the two graph APIs (API #1 `submit(dag-spec)
  -> dag-token` / `cancel(dag-token)`, and the API #2 event/response shapes — the token-tagged DAG
  events and the subscription set carried in the spec) and the direct one-shot WASI imports. The
  ordinary `reducer` and `pure-reducer` worlds are unchanged.
- `src/host.rs` — `add_host_imports` (`:796`): wire the two graph APIs and the one-shot WASI imports
  under the `ReducerKind::Event` branch (`:817`). Thread the new host state (the graph executor and its
  token table, the one-shot WASI contexts, the journal) into `HostState` (`:199`) and its `HasData`
  projection. The per-kind linkers and the `bindgen!` for the event world (`:33`) are the generation
  seam.
- a new module (e.g. `src/graph_exec.rs`) — the DAG executor: validating a submitted graph, minting and
  tracking the DAG token, running the fixed set of stage kinds, streaming named pipes stage-to-stage on
  the async executor, emitting the subscribed token-tagged events back to the owning reducer, handling
  `cancel(token)` (stage teardown + stream cleanup + cancelled outcome), and recording outcomes. The
  one genuinely new subsystem.
- `src/event_registry.rs` — the trust root resolving which event reducer governs a contract; the
  routing that carries an ordinary reducer's effect up to its event reducer is used as-is.
- rcdzc guest-import emit (owned by `v-rust-backend`): emit the new imports for a component targeting
  the event-reducer world. The one-shot WASI imports and any resource-handle shapes in the graph
  contract are the emit delta; `v-rust-backend` wants the platform's read on the resource-handle
  marshal.
- `cadenza-ast` codec (owned by `v-syntax`): unchanged — per-contract values, including the graph
  spec, cross as canonical bytes; no codec change.

---

## 11. Open decisions, with chosen defaults

- **Primary model** — the streaming effect DAG; WASI demoted (§1, §5). Chosen by the operator.
- **Two WIT APIs** — API #1 submit/cancel, API #2 the event/response loop (§3). Chosen by the operator
  ("we'll need two wit APIs. One is to create a dag and submit it... the dag would emit events back into
  the event reducer... it can decide how to handle the events it's explicitly subscribed to... and
  eventually it will respond back to the invoker with what happened").
- **DAG token** — `submit(dag) -> token`; the token is an opaque handle scoped to the submitting event
  reducer, used for `cancel(token)` and to correlate the DAG's emitted events and its influence on the
  final response (§3, §4). Chosen by the operator ("one API to submit it. And that returns a token that
  can be used to cancel it").
- **Cancellation** — `cancel(token)` tears down the in-flight DAG (stage teardown + stream/resource
  cleanup) and yields a cancelled outcome through API #2 (§3, §4). Chosen by the operator ("we also
  need to be able to cancel a dag"). Open detail for the vertical: exact teardown ordering and whether a
  partially-streamed sink is left truncated or removed — a per-stage-kind cleanup contract.
- **Direct one-shot WASI kept** — `wasi:clocks/wall-clock` + `wasi:random` only, held by event
  reducers, journaled for determinism (§7). `monotonic-clock` excluded (no distributed meaning, §7).
  Chosen by the operator ("keep direct WASI for simple one-shot calls").
- **No built-in `now`/time effect** — time is not a platform primitive; an event reducer reads the
  wall clock and answers a time contract, like any other capability (§1). Chosen by the operator ("we
  shouldn't have a `now` effect; we should only have event reducers"). This diverges from the vision
  doc §6, which frames `now` as a built-in effect; that framing is superseded here.
- **Graph construction** — declarative spec per submission, dynamic across notifications (§3). Chosen
  by the operator; not live-mutable.
- **Stage vocabulary** — a fixed, small, generic set (`file`, `http`, `shell`, `socket`, `reducer`);
  the platform knows how to operate each; the `reducer` stage keeps behavior open (§3). Chosen by the
  operator; expanding the set is a deliberate platform change.
- **Byte path** — node-to-node inside the platform; the orchestrating reducer is out of the byte path;
  a `reducer` stage taps a pipe when observation/injection is wanted (§3). Chosen default (supersedes
  an earlier "reducer-mediated only" answer given before the graph model existed); open to the operator
  reverting it if they want every pipe mediated.
- **Large terminal values** — written to the CAS and delivered as a hash rather than through the fold
  (§4). Chosen default, consistent with vision §7's inline-small / hash-large rule.
- **Stage implementation** — how the platform performs an `http`/`file` stage internally (a linked
  WASI host vs. Rust async libraries) is the platform's per-stage choice, invisible to guests (§5).
  Left to the vertical.
- **Capability policy shape** — the event reducer's own §5 authz design; out of scope here. Named so
  the boundary is explicit.

---

## 12. Coordination

- **Platform vertical** — owns `cdz-platform`: the `world.wit` change, the `add_host_imports` gating,
  the new graph executor and pipe substrate, the stage implementations, the one-shot journaling, and
  the `HostState` threading. Builds the increments.
- **`v-rust-backend`** — the guest-import emit for the new event-world imports, including any
  resource-handle shapes; wants the platform's read on the resource-handle marshal.
- **`v-platform-itest`** — the conformance coverage for each increment (instantiation, the privilege
  gate, the recorded-outcome determinism property).
- **`v-syntax`** — owns `cadenza-ast`; no change expected (the graph spec crosses as canonical bytes),
  included for awareness.
