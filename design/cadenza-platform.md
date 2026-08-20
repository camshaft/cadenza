# Cadenza platform

> An ultra-minimal, generic runtime for programs that react to events. This document is the
> vision — what the platform is, described on its own terms.

## Purpose

The Cadenza platform is a small, generic runtime for running programs that react to
events. Its whole job is: accept an event into a log, run the current program for
that log, authorize the requests the program makes, carry them out, and record the
results back into the log as more events.

The platform is general. Acting as an agent harness — running the loops that drive AI
agents — is one thing it does, but it is not specific to that: it is a substrate for any
program that reacts to events. The runtime knows nothing about agents, models, tools,
Cadenza, or any particular capability. Adding a new thing a program can do never means
changing the runtime. Everything specific is a program, a contract, or an event.

The word for the running instance is a **node**. Everything that runs on a node is a
**reducer** — the single kind of participant (section 3). A node hosts session reducers
and the edge reducers that carry out primitive input and output; there is no separate
executor. One binary; what a node does is configuration, not a different build. This
document specifies a single node running for a single operator; running many nodes for
many operators is later work (section 11).

---

## 1. Contracts are how everything communicates

This is the center of the design. Read it first; the rest follows from it.

A **contract** is a declared interaction: a name, the type of its input, and the type
of its output. The identity of a contract is the hash of that declaration.

```
contract    = (name, input: Type, output: Type)
contract-id = hash(contract)
```

Nothing communicates by string name or by an enumerated kind. A program that wants to
fetch a URL does not name an `"http"` family; it references the exact contract it was
built against, by hash. A program that answers requests declares which contract hashes
it answers. Routing a request means finding who answers that contract hash — an exact
content-addressed lookup, never a string match and never a version range.

Three consequences, all deliberate:

- **Identity is nominal.** The name is part of the hash, so two contracts with the same
  shape but different meaning are different contracts. `temp.celsius` taking a `Float`
  to a `Float` and `temp.fahrenheit` taking a `Float` to a `Float` have different hashes
  and never route to each other.
- **Identity is exact.** A caller's contract-id must equal what an answerer declares.
  There is no compatibility check, no tolerant reader, no version field to range-check.
  The hash either matches or it does not.
- **Evolution is a new contract.** Changing an input or output type produces a different
  declaration and therefore a different hash — a new contract. To move a program from an
  old contract to a new one, publish the new contract and, where the two must interoperate,
  publish an **adapter**: a program that answers the old contract by calling the new one.
  The runtime never infers that two contracts are related.

The `input` and `output` are Cadenza types in their canonical binary form. A contract
declaration is itself a Cadenza value with a canonical encoding, so its hash is
reproducible from the declaration alone. The runtime treats a payload as opaque bytes
addressed by hash, and a contract as an opaque identity addressed by hash; it never
parses either. What the bytes mean against a contract is the concern of the programs on
each end.

### One routing concept, used three ways

- **Effect** — a program requests a contract with an input and later receives its output.
- **Message** — sending to another reducer is just an effect (an addressed send, section 3):
  its output is the **delivery outcome** — whether the message reached the target or not —
  not a special "unit / no reply" case. A message is nothing more than an effect.
- **Event** — everything a reducer receives arrives as an event carrying a contract-id. An
  event is one of two things, and the reducer is told which (section 3): the **output** of
  an effect it performed (a response), or the **input** of an effect performed on it (a
  message injected by another reducer, carrying its source).

So "which requests may this program make", "which requests does this program answer",
and "what may be delivered to this program" are all sets of contract-ids. A program's
declared set of contracts is, at the same time, its **routing table** and the surface that
authorization is expressed over (section 5).

### No strings, no versions, no enumerated kinds

Identity is the contract hash and nothing else. There are no `family` strings, no separate
`version` tags, no enumerated set of effect kinds, and no unregistered-family fallback. A
schema change is a new contract — a new hash; a program answers a declared set of
contract-ids; an effect with no answering handler is a recorded failure. Routing is exact
hash equality, with no tolerant reader and no version range to match.

---

## 2. Principles

1. **The runtime knows nothing specific.** No built-in knowledge of any contract, effect
   name, verb, family, or namespace — and no per-effect branch in its code. It routes
   purely by looking up a contract-id's handler chain (section 3). A new capability is a new
   contract plus something that answers it — never a runtime change.
2. **Everything is an event.** Requests, results, messages, timers firing, a program
   being replaced, a session closing — all are ordered, content-addressed events in a
   log.
3. **A program is a pure function of its log.** It reads state, reacts to one event, and
   returns requests. All nondeterminism (a model's output, an HTTP response, the clock,
   randomness) enters only as recorded result events. Replay re-reads the recorded
   results; it never re-runs the outside world.
4. **Reads are effects; writes are local.** A program only ever appends to its own log.
   Reaching anything outside — another session, the network, the clock — is an effect
   whose recorded result becomes part of the program's own history.
5. **Reacting to an appended event is the only way a program runs.** There is no polling
   loop. Delivering a message, a timer firing, a result landing — each is an append, and
   each append runs the program once and returns.
6. **There is one kind of participant: a reducer.** Anything that takes part — a session
   running an agent's task and the thing that carries out an HTTP or shell effect alike —
   is a reducer with the same interface: it receives an event, updates its own state, and
   emits effects. There is no separate executor.
7. **Deploy the kernel once.** The kernel binary is the only part that cannot be
   hot-swapped, so it is kept as small as it can possibly be and everything that can be a
   reducer is one. Authorization, name resolution, log persistence, lifecycle, timers, and
   input/output are reducers referenced by content hash and swapped by reference — never a
   kernel redeploy. Minimality is not an aesthetic here; it is what makes deploy-once real.

---

## 3. The kernel, reducers, and sessions

### The whole kernel

The kernel is tiny, and it is not the router. It is a **reducer-execution engine**: it runs
a reducer step given `(reducer, event)`, schedules and interleaves those steps across
sessions, keeps the deterministic event log, provides the direct-access backends (the
key-value store, a reducer's own id, and the content-addressed store, section 8), and owns
the one piece of pending-future state — the durable `fire-after` timer (section 6). It reads
no payload and matches no name.

Assembling handler chains, moving an effect through them, tracking authority and correlation,
supervising handlers, enforcing deadlines — none of it lives in the kernel. Each is the work
of a **system reducer**: a wasm module, like every other reducer, instantiated **once per
event** to shepherd that one effect (section 4). Because it is per-event there is no shared
router state and no global chokepoint; because it is a reducer, its logic is content-addressed,
swappable by hash, and replayed deterministically like any other reducer.

Which system reducer governs an effect is a lookup. The kernel keeps a small **event-reducer
override registry** mapping a contract-id to the system-reducer implementation that shepherds
that contract's events, with a default for everything else. This is how the platform
customizes dispatch, supervision, or grant behavior for a particular contract without
touching the kernel. The default is itself a wasm module, bootstrapped at setup rather than
compiled in, so the kernel ships with no built-in dispatch logic at all.

The override registry is the **trust root**. Installing or changing an entry is the highest
privilege in the system, because the entire security model rests on the chosen system reducer
being correct — a wrong one could ignore authorization, forge grants, or misroute. So the
registry is genesis-level configuration, settable only by the root authority; it is the one
thing at the bottom that is not itself an event reducer, and everything above it trusts the
correct, root-installed system reducer to enforce the model.

So the kernel's whole irreducible core is: execute a reducer step; schedule and interleave,
carrying each response back to the reducer that emitted the request; keep the log; the direct
reducer-facing accesses with swappable backends (the key-value store, a reducer's own id, and
the content-addressed store); the `fire-after` timer; the root-only override registry; and
one privileged wire — on an emitted effect, look up the system reducer for its contract,
instantiate it, hand it the event, and honor its direct commands (run a reducer, respond,
attach, monitor, arm a timer, notify, retire a context). Everything specific — routing,
chaining, authorization, name resolution, lifecycle, what a timer means, input and output —
is a content-addressed reducer, not kernel code. This is the point of the entire design and
the line to hold: the kernel binary is the one thing that cannot be hot-swapped, so it is
deployed **once** and kept as small as possible, while everything that will ever need to
change is a content-addressed reducer, swapped by reference without redeploying the kernel.

### Handlers chain

A contract's handler is a **chain of reducer identifiers** — an ordered list of references
to other reducers, not a single one. It is a stack of interceptors: a rate limiter wrapping
an HTTP handler, an authorizer wrapping a credential mint, a mock wrapping a live edge
reducer, a logger wrapping anything. An effect request emitted by a **leaf** reducer travels
the chain, and each reducer in it in turn may:

- **answer it** — produce a response, which stops the ascent and bubbles back down;
- **transform and forward it** — rewrite the request (even into a different contract) and
  pass it to the next handler; or
- **emit its own effect requests** — which begin their own dispatch through their contracts'
  chains.

The order within a chain is the author's choice; the platform preserves it and interprets
nothing about it, so it needs only to be documented and consistent. Answering, forwarding,
and the single-use capability that discharges a response are the dispatch mechanics of
section 4.

**Chains span generations, and a child inherits nothing automatically.** When a handler is
registered on a reducer, the system reducer notifies it (`on_notification`, below), and that
reducer decides whether to propagate the handler to its children by registering it with them.
A child reaches only what its ancestors have explicitly passed down — least privilege by
default — and not propagating is how a handler stays private, with no barrier markers to
maintain and no way for an ignored notification to widen a child's authority.

What a child does gain always continues into its ancestors' chains. The system reducer wraps
a propagated capability in the ancestors' interceptors — their authorization middleware above
all — so every effect a descendant emits passes through every ancestor's guard before it can
reach an edge. Authority only attenuates downward: a child prepends its own handlers and may
restrict further, but can neither reach past an inherited guard nor remove one. Assembling the
effective chain and enforcing this is the system reducer's work (section 4); because
installing the system reducer is the highest privilege (the override registry, above), that
enforcement is trustworthy rather than a matter of a parent's diligence.

### Two roles a reducer plays

The one interface is shared, but a reducer plays one of two roles:

- **Session reducers** are logged and replayable. Their state is a projection of an
  append-only event log, and each fold is a pure function of `(event, state)` (principle
  3). Agents, supervisors, the authorizer, a memory store — everything with durable,
  auditable history — is a session reducer.
- **Edge reducers** are the node's boundary with the outside world. They carry out
  primitive input and output — network, subprocess, clock — and are where nondeterminism
  enters the system, as the recorded result of an effect. An edge reducer answers a message
  exactly as any reducer does: it receives the message and emits a result. Principle 3
  still holds for the sessions that call it, because the edge reducer's output is recorded
  in the caller's log and replay reads the recording.

### Everything is a wasm module

Every reducer — session and edge alike — is a content-addressed wasm module. The kernel
binary is the only part that is not wasm, which is exactly what lets it deploy once
(principle 7): anything expressed as a wasm module evolves by publishing a new hash, with
no redeploy.

Edge reducers reach the outside world through **WASI**, the standard capability-oriented
interface the runtime already hosts. An edge reducer for the filesystem, sockets, HTTP, the
clock, or randomness is a wasm module that imports the matching WASI interface
(`wasi:filesystem`, `wasi:sockets`, `wasi:http`, `wasi:clocks`, `wasi:random`); the runtime
supplies the WASI host, so these need no custom host code and no kernel change — they are
regular hashed contracts like any other reducer. So log persistence, an HTTP effect, a model
call (HTTP), and a metrics sink (network) can all be wasm modules over WASI — as can the
content-addressed store's backend (`wasi:filesystem` / `wasi:keyvalue`), even though the CAS
is reached by a direct call rather than an effect (section 8). WASI being capability-oriented — granted handles, no ambient authority — maps directly
onto the resource-scoped capabilities of section 5, and this WASI host is generic substrate
(like the wasm engine itself), not knowledge of any specific contract, so principle 1 holds.

**Rule: if a capability can be expressed through a standard WASI interface, it must be a
WASI-based wasm reducer — never a custom host import.** A custom host import bakes code into
the binary that cannot evolve without a redeploy (principle 7), so it is a last resort,
permitted only where no standard WASI interface can express the capability, and each one is
a deliberate, minimized, justified exception.

Only two capabilities genuinely cannot be pure WASI, and they are the entire native residue
beyond the kernel binary:

- **subprocess / shell** — WASI has no process-exec interface, so running a command needs
  either a small custom host import (a platform change) or a native reducer outside the
  sandbox. Isolate and minimize it.
- **the durable timer** — arming a future wake is the kernel's own reactive mechanism (its
  one piece of pending-future state, section 6); reading the clock is WASI, arming a wake is
  not.

The aim is to shrink that residue toward nothing: every capability that fits a standard WASI
interface is a plain wasm reducer, evolvable without touching the binary.

### Session

A **session** is a logged reducer. It is:

- a signed, content-addressed, append-only **event log** with a total order within the
  session (there is no order across sessions),
- an attached **state** (a key-value map, section 7) that is a projection of the log,
- a **program** (the reducer), named by content hash,
- its **handler chains** — what it answers and what it may perform, including the authz
  middleware that governs it (sections 3, 5).

A session is identified by the hash of its first event (its genesis), so its identity
certifies its own origin.

### Genesis

The first event names the program to run (by content hash) and a per-session
`spawn-nonce` supplied by whatever created the session. The runtime does not read a
clock or draw randomness (principle 3), so this entropy is provided from outside at
creation and recorded in the genesis event, which makes it replay-stable: recovery reads
it from the log and never re-mints it. A spawned session's genesis also records its
parent (section 7), so the child's identity certifies its provenance.

The session's id is the hash of its genesis event, so it cannot be a field inside that
event — and it needn't be delivered anywhere: a reducer reads its own id at any time as a
direct read (section 3), and lists its handlers via the `list-handlers` effect (section 7).
**Birth** is the reducer's first `on_message` — the seed the spawn delivered, an `id` plus a
payload. That is all the kernel imposes; there is no separate "capabilities" or "purpose"
field. A session's authority is the authz middleware in its handler chains (section 5), and
any purpose or seed config is just content in the birth payload, interpreted by the reducer.

Configuration is not runtime state — it is early events the program folds into its own
state. A program's initial prompt, policy, or seed data arrive as ordinary events. The
events that set up a session can themselves be the output of another session.

### The reducer interface

Everything a reducer receives is an event carrying a contract-id, and it is one of a few
kinds — so a reducer has **three entry points**, and the runtime calls the one that fits:

```
on_response(response: Response)         -> (list<Request>, Outcome)
on_message(message: Message)            -> (list<Request>, Outcome)
on_notification(notification: Notification) -> (list<Request>, Outcome)

type Response = {
  id:                 Hash,               # the contract-id this answers (schema hash; deref from CAS if needed)
  continuation-token: Bytes,              # correlates back to the original request — present on ok AND error
  payload:            Result<Ast, Error>, # the result: Ok(output value) or Err(runtime failure)
}

type Message = {
  id:                 Hash,   # the contract-id
  payload:            Ast,    # the input value of the effect being performed on this reducer
  from:               Hash,   # the source reducer — envelope metadata to authenticate / route on
  continuation-token: Bytes,  # correlates the reducer's reply back to the caller
}

type Notification = {
  id:      Hash,  # the contract-id of the notification's schema
  payload: Ast,   # the notification value — a plain typed value, no continuation-token, no Result
}

type Request = {
  id:                 Hash,             # the contract-id (= the schema's hash)
  payload:            Ast,              # the input value
  continuation-token: Bytes,            # the reducer's token, returned on the response — the correlation
  deadline:           Option<Duration>, # optional: Err(Timeout) if unanswered within it
}

type Error   = Timeout | MissingHandler
type Outcome = Continue | Break(schema: Hash, reason: Ast)
```

- **`on_response`** is the **output** side: a reply to a request this reducer performed. A
  `Response` always carries the `id` and `continuation-token` that correlate it to the
  original request, and its `payload` is the **result** — `Ok(output value)`, or a
  runtime-level failure `Err(Timeout)` (the deadline elapsed) / `Err(MissingHandler)` (nothing
  answers the contract). Putting the result in the payload — rather than wrapping the whole
  `Response` in a `Result` — is what lets a *failure* still carry its correlation, so the
  reducer can match a timeout back to the request that timed out. A handler's *own* failure —
  an HTTP 500, a domain error — is not an `Err`: the handler answered, so it rides in
  `Ok(output)`. Whether to retry is the reducer's judgment, not a kernel field.
- **`on_message`** is the **input** side: an effect another reducer performed on this one, or
  a message sent to it. The `Message` carries its **source** (`from`) as envelope metadata,
  so the reducer can authenticate and route on who sent it — the reason it is distinct from
  `on_response`. The reducer answers by emitting its reply, correlated by the message's
  `continuation-token`, which the runtime routes back to the caller's `on_response`.
- **`on_notification`** is the **control-plane** side: an unsolicited platform event, such as
  a new handler becoming available on this reducer (the trigger for propagation, above) or a
  lifecycle event it subscribed to (spawned, closed, failed — section 7). It is shaped like a
  response without a continuation-token: a contract-id plus a plain typed payload. There is no
  `Result` — a notification is an event that happened, not the success-or-failure of a request
  — and no correlation token, because nothing of the reducer's is being answered. Because the
  channel is typed by contract-id, one entry point carries every kind of platform event; the
  kernel hard-codes no notification vocabulary, and a reducer matches on the contract-id and
  ignores the ones it does not handle. Making these a distinct entry point (rather than folding
  them into `on_message`) means it is obvious a reducer must handle or explicitly ignore them.

Both return the same **product**: the `Request`s to perform *and* an `Outcome`. `Continue`
keeps the reducer running; `Break(schema, reason)` **terminates** it, carrying the reason for
closure as a typed value — a schema hash plus the reason payload, a value like any other so a
subscriber can decode it. The kernel imposes no normal-vs-error taxonomy on the reason; a
clean completion and a failure are both `Break`s, distinguished only by the reason value,
which a subscribing supervisor interprets (section 7). Because the return is a product, a
reducer can emit final requests *and* `Break` in one call (send a result, notify a peer, then
close); those final requests dispatch but their responses never fold (the session is now
terminal), so they are fire-and-forget. A reducer ends *itself* only by returning `Break`,
never by an effect. A reducer that instead traps or exhausts its fuel cannot return at all;
the runtime captures that as an uncontrolled fold-failure (below).

A request may carry a **deadline** (`Option<Duration>`): with `Some(d)`, no answer within `d`
delivers `Err(Timeout)` to `on_response` and cancels the dispatch (no late answer folds);
`None` means no timeout. The deadline is the reducer's own per-request control, set at
emission.

**Identity by hash, schema by reference.** Every message carries only its `id` — the
contract-id, which *is* the hash of the schema — plus the payload. The schema is not inlined;
a reducer that needs the decoded schema derefs it from the content-addressed store by the id
(section 8), lazily. So the wire and the kernel see an id + payload and route on the id, and
never decode anything.

Each call is a pure function of its input and the reducer's current state; a fresh instance
runs each call and holds no memory between calls. Beyond the requests it returns, a reducer
has a few **direct** accesses during a call — resolving within the same call, not routed
through the effect chain or logged as separate events: its **key-value store** (read/write,
section 7, where it stores what it needs to continue, keyed by a request's
`continuation-token`, and looks it up when the response returns), its own **id** (a fixed
read), and the **content-addressed store** (`cas-get`/`cas-put`, section 8, including the
lazy schema deref above). These are deterministic — own state, a fixed id, content addressed
by hash — which is why they are direct rather than effects; a direct call may still *await*
an async backend (a `cas-get` reads from disk/cache/S3 without blocking, section 8), it just
resolves within the same call. The async *effect* model is for the nondeterministic outside
world; these are not. Everything else — including listing the contracts a reducer has
handlers for, its own or another's — is an **effect** (section 7), filterable through the
middleware chain.

Two events belong in the same session only if they must be strictly ordered relative to
each other or share a retention lifecycle. Choose session boundaries by ordering and
shared fate, not by topic. The natural unit is one agent doing one bounded task.

This interface is the ordinary reducer. The **system reducer** that shepherds an effect
(section 4) is a distinct, privileged interface: it is driven by dispatch-lifecycle signals
and emits direct kernel commands rather than routing everything as effects. It is a fold like
any reducer — signals in, commands and state out — but its vocabulary differs, so it is its
own interface, described in section 4.

### Terminating and failing

A reducer ends itself by returning `Break(schema, reason)` (above), the reason being a typed
value describing its closure (a clean completion or a failure — the reducer's own
vocabulary). Distinct from that is an *uncontrolled* failure: a fold that traps, exhausts its
fuel, or fails to instantiate, so it
cannot return anything at all. The runtime captures that as a **fold-failed** event naming
the reason and the input whose fold failed, and moves on. Both are terminal and both notify
subscribers (section 7); the difference is only whether the reducer described its own exit
or the runtime had to. A failed fold is not re-folded (the same reducer on the same input
would fail again); recovery is a supervisor's decision (section 11) or a program
replacement (section 7).

---

## 4. Requests, dispatch, and results

The fold returns requests. Every request names a **contract** and carries an input. The
runtime routes each and records the outcome back into the log as a later event. There is no
kernel authorize step: authorization is middleware in the chain (section 5), a handler like
any other. A request also carries a **continuation-token** the reducer chooses, to
correlate the eventual result (below).

### The per-event system reducer

Dispatching a request is not a kernel table lookup; it is the work of a **system reducer**,
instantiated once for that effect. On an emitted request the kernel looks up the system
reducer for the request's contract in the override registry (the default if none),
instantiates it, and hands it the effect. That reducer does the rest: it assembles the
handler chain across generations (section 3), moves the effect through it, tracks correlation
and authority, and supervises the handlers — all as ordinary reducer state and direct kernel
commands, with no dispatch vocabulary in the kernel.

It is a **separate, privileged interface** from the ordinary reducer. It is driven by
dispatch-lifecycle signals — a dispatch starting, a handler step returning, a monitor firing,
a timer firing — and it emits **direct commands** rather than routed effects: run a reducer
with an event, respond against a capability, attach to a context, mint a capability, monitor
a handler, arm a timer, notify a handler, retire a context. Its own core loop must use these
direct commands and never emit routed effects, or each of its own effects would spawn another
system reducer without end.

Like every reducer it is an ephemeral instance whose state lives in the key-value store —
here keyed by the **context id** (below). That is what makes it both per-event and durable:
the instance is fresh per event, but the context it operates on persists and can outlive any
single dispatch, so a later event referencing the same context id gets a fresh instance that
loads it and continues. Because each event has its own instance, dispatches never share
router state and never serialize through a common cell — two effects, even within one
session, are handled independently.

Messaging, timers, and lifecycle are not special request kinds — they are contracts answered
by provided reducers (section 7), dispatched exactly like any other effect. There is no
hard-coded effect vocabulary anywhere in the kernel. A message sent point-to-point to a
specific reducer by its **id** (rather than routed by contract) is delivered directly, not
run through a chain — that is how a reducer queries a system reducer's context without
spawning a further dispatch.

### The request context

A leaf that emits a request chooses a **continuation-token** to correlate the eventual answer,
but that token never travels upward. The platform creates a **context** holding the leaf's
token and where the answer folds back, and issues an **unforgeable id** that stands for it.
That id — not the leaf's token — is what travels, so a handler can neither see nor spoof the
leaf's correlation.

Handlers **attach** metadata to the context, and each attachment records **who attached it**
(the handler id), the **schema** (contract-id) of the value, and the value itself.
Attachments are append-only: nothing is overwritten or removed, and a grant is reversed by
attaching a revocation, so the accumulated history — who established what, in what order — is
always reconstructable. Because the platform records each author, a later handler can trust
that, say, the authorization handler attached a particular grant, and gate on it. The
**lineage** — which handlers a request passed through — is the base case of this: a presence
record the platform stamps as the effect advances, with attachments the richer attributed
layer on top. Other reducers do not read the context directly; they **request** what they
need by sending the system reducer a message it chooses to answer, so context access is
itself a governed exchange rather than an open read.

The id is unforgeable without a secret: it is a kernel-issued handle, derived
deterministically (over the leaf session, its token, and a sequence) rather than drawn at
random, and validated against the platform's own record on use, with the holder tracked. A
context that never leaves one operator needs nothing more; a cryptographic, attenuating
construction is where this extends if a context ever crosses a trust boundary a peer cannot
take on faith (section 11).

### Forward, respond, and single-use capabilities

A handler acts on an effect by emitting one of two built-in effects, keyed by the context
rather than a raw token:

- **`forward`** — permit and pass the (possibly rewritten, even re-contracted) request on to
  the next hop.
- **`respond`** — answer or deny; the reply bubbles back down.

These are ordinary emitted effects, not a synchronous return value, so a handler may receive
an effect, store it in its key-value state, do other work across several folds, and emit its
`forward` or `respond` only later. Deferral is free — the correlation is the context, not a
suspended stack.

Each handler holds exactly one **single-use, handler-bound capability**: its obligation to
answer the party below it. It is bound to the handler (the platform checks the caller against
the capability on use, so a leaked capability is useless to anyone else) and consumed on
discharge (a second attempt is a deterministic, recorded rejection). The transforming
middleware pins the rule — a handler discharges its one obligation exactly once, however it
sources the answer:

- respond directly — consumes it;
- forward transparently — the platform discharges it on the handler's behalf when the
  upstream answer arrives, with the same bytes, without re-entering the handler;
- forward to transform — the handler emits a fresh upstream request (its own new capability
  for that), keeps its obligation open, receives the upstream answer in `on_response`, then
  responds, which consumes the obligation.

Capabilities are minted lazily, one hop at a time, because the chain is dynamic — the system
reducer does not statically know where an effect will land. Correlation is the context and
the capability, never the `id` (which is the contract-id, shared by every request of that
contract).

### Supervision

The per-event system reducer is the **supervisor** of its dispatch. An obligation has exactly
three ways to retire, all owned by it:

- the **respond capability** — the handler discharges voluntarily (success);
- a **monitor** on the handler — it detects an exit or crash and turns the open obligation
  into a failure that bubbles down;
- a **deadline timer** — on fire it notifies the working handlers that the deadline is
  exceeded, then bubbles `Err(Timeout)` down and retires the capability.

The deadline is the system reducer's policy, built on the raw `fire-after` timer the kernel
provides (section 6): what a timeout *means* — who is told, any grace, what bubbles — is
decided in the system reducer, not the kernel. The "deadline exceeded" notice reaches a working
handler through the ordinary `on_notification` channel, so there is no new handler-side
mechanism; it is cooperative — the handler may wind down its work, but it may also ignore the
notice, so the system reducer still hard-retires the capability and bubbles `Err(Timeout)`
regardless.

The supervision tree is the dispatch tree. A handler that emits its own effect starts a
nested dispatch with its own per-event supervisor; failures propagate up through the ordinary
response mechanism — a nested failure surfaces as an `Err` on the handler's `on_response`,
which the handler may let fail its own obligation, which its supervisor then sees — not
through a separate supervision channel. A context is retired once every obligation against it
has resolved by one of the three paths.

### Correlating a result to its request

A reducer emits a request and returns; the answer arrives as a *later* `Response` — its
payload the result (`Ok`/`Err`) — delivered to `on_response`. The reducer does not block or
resume a suspended stack — there is no stack to resume, because each call is a fresh
instance. Instead:

- The reducer chooses a **continuation-token** when it emits the request and stores whatever
  it needs to continue in its key-value store, keyed by that token.
- When the answer returns — always a `Response`, whether the result is `Ok` or `Err` — it
  carries the same `continuation-token`, so the next `on_response` reads the token, looks up
  its continuation, and proceeds.

Correlation is the `continuation-token`, not the `id` — the `id` is the contract-id, shared
by every request of that contract, so it can't identify one outstanding request. The
runtime keys durable dispatch and recovery on the token (unique per outstanding request in a
session). Concurrent requests are correlated independently by token, and results may arrive
in any order.

### Durable dispatch and at-most-once

Before an effect is routed to its handler, the runtime appends a **dispatched** event
recording the contract-id, the resolved input/target, an **idempotency key**, the deadline
(section 6), and the continuation-token (the per-request correlation). This is what makes
crash recovery correct: after a restart, a dispatched event with no matching result is a
known in-flight obligation. The runtime re-drives it using the idempotency key so a
side-effecting effect is not double-applied, or records a failure — never silently drops
it and never double-fires.

### What the reducer receives back

The `Response` delivered to `on_response` (section 3) carries the correlation (`id`,
`continuation-token`) and a `payload` that is one of:

- **Ok(output)** — a handler answered; the payload is the contract's output value. A
  handler's *domain* failure rides here too — the output type can encode it — because
  answering with an error is still answering.
- **Err(MissingHandler)** — no handler is registered for the contract; nothing could answer.
- **Err(Timeout)** — the request carried a deadline that elapsed with no answer. A timeout
  **cancels** the dispatch: the runtime guarantees no late answer for that request will ever
  fold, so a reducer never has to handle a response arriving after it gave up.

Because the correlation lives on the `Response` and not in the `Result`, a failure is
matched to its originating request the same way a success is.

The reducer decides what a failure means for it — retry (re-emit, perhaps under a new
deadline), escalate, or give up. Retryability is its judgment, not a kernel classification.

### Answering an effect

An effect is carried out by whichever **reducer** answers its contract-id — a peer session
that declared it answers that contract, or one of the node's edge reducers (section 3). The
answerer receives it through `on_message` (with the caller as `from`); there is no separate
executor, and routing is the same in both cases: the system reducer moves the effect to who
answers the contract-id. An answerer may reply immediately or accept the effect and reply
later; while it is unsettled its obligation stays open and the caller's continuation waits.
When the reply is ready the answerer emits `respond` against its single-use capability
(section above), the platform routes it back down the chain, and the caller resumes in
`on_response`.

An effect for which no reducer answers is a recorded failure, not a silent drop.

---

## 5. Authorization

Authorization is **middleware** — a reducer (or a chain of them) in front of the contracts it
guards (section 3). The kernel holds no capability model and enforces nothing; an effect is
routed up the chain, and the authz middleware either **forwards** it (permit) or **answers
with a denial** that bubbles back to the caller as the effect's outcome. Because the
middleware sits in the chain it sees the request's **resolved argument**, so it can gate on
*what* is being done to *which resource*, not merely which contract — but exactly how it
models that (capabilities, resource predicates, grants, a policy language) is the
middleware's own design, not the kernel's, and is deliberately out of this document. A policy
engine such as Cedar is one such middleware, carrying its policies as content-addressed data
referenced from the log; it is a wasm reducer, swapped by publishing a new hash, never a
redeploy.

An authz middleware records its decision by **attaching a grant to the request context**
(section 4): an attribution the platform stamps with the authz reducer's own id. A downstream
handler then trusts the grant because it can see who attached it — it can, for instance,
refuse to act unless an authorization handler it recognizes appears in the lineage with a
matching grant. This is how a permit travels with the request rather than being re-derived at
every hop, and why the context's attributions are unforgeable and append-only.

Enforcement rests on the chain being configured so the authz middleware wraps the contracts
it must guard — established at bootstrap and controlled by the authority to register
handlers, itself gated the same way (grounded at the trust root, section 11). A denial is an
ordinary recorded result, auditable like any other, not a special kernel event.

Down the spawn tree, enforcement compounds: the system reducer wraps every capability it
propagates to a child in the ancestors' authz middleware (section 3), so every effect a child
emits traverses every ancestor's guard before reaching an edge. Authority therefore only ever
attenuates downward — a child can add restriction but never reach past or remove an inherited
guard — so privilege escalation by spawning is structurally impossible.

### Down-scoping through a published program

A program can hold a broad, dangerous capability internally and export a narrow one. A
program that internally performs the arbitrary-shell contract but exports only a
`date.now` contract lets its callers hold just the cheap `date.now` grant; the dangerous
capability lives with one audited, published program instead of spreading to every caller.
This is the same attenuation as delegation down a spawn tree, applied through a published
contract. A capability may therefore be "may perform contract X", where X is answered by a
program that internally wields more than X exposes.

---

## 6. Time and reacting

Time is not an input to a fold; it is an event in the log. A program never reads a
wall-clock during a fold — that would make the same event fold differently tomorrow.
Two contracts cover all of time:

- **now** — an effect whose result is the current time, recorded in the log. This is how
  a program learns the time, deterministically. It is capability-gated: a program can be
  denied the clock entirely.
- **fire-after(duration)** — the only timer primitive. The runtime wakes the session
  later by delivering a timer-fired event carrying the recorded fire time. Absolute
  deadlines and crons are built on top: to fire at a wall-clock moment, a program reads
  `now`, computes the delay, and arms `fire-after`; a cron is a program that, on each fire,
  does its work and arms the next timer from the recorded fire time.

The timer's arm event records an **absolute deadline anchor** so that a session which
moves between nodes computes the right remaining time; the program still never reads the
clock, because it only ever sees the recorded fired time. Live, the timer runs against a
real clock; on replay, no clock runs — the recorded fire is read straight from the log.

Reacting is the whole scheduling model. There is no polling. A program instance runs,
returns, and the session waits in one of two ways, each with a clean wake:

- **Waiting on an outstanding effect.** A reducer may attach a **deadline** to a request
  (section 3). The kernel provides only the raw `fire-after` timer; enforcing the deadline is
  the system reducer's job (section 4): on fire it notifies the working handlers that the
  deadline is exceeded (cooperative cleanup, through `on_notification`), then hard-retires the
  obligation and bubbles `Err(Timeout)` down, so the waiting reducer wakes to recover and no
  late answer can fold. A hung model call or shell command becomes an ordinary `Err(Timeout)`,
  not a stuck session — so a reducer that must not wedge on a hung answer sets a deadline; the
  anti-stuck guarantee is per-request and opt-in.
- **Idle, waiting for input.** Any delivered message wakes the session. Idle costs
  nothing and is instantly revivable.

Because a fold always returns, there is no long-running turn that can wedge mid-stream,
and no external watchdog is needed to nudge one loose.

---

## 7. State and lifecycle

### State

A session's state is a key-value store defined by an **interface (a trait)**, not a
concrete in-memory structure — the backend is pluggable, so state is never forced to fit in
memory. Its operations are get, put, delete, a **streaming** prefix-scan (so a large scan
does not materialize everything at once), and a content-addressed **root hash**. A backend
may hold state in memory for tests, in a structurally-shared persistent map, or on disk or
over the network for state too large for RAM; the reducer only ever sees the interface. Two
obligations the reducer depends on: a **canonical key order** (so prefix-scan and the
effects a reducer emits over it are replay-deterministic), and a root hash after each
change. Prefix-scan is the primitive for the collections a reducer maintains (pending
children, seen items, per-target working state); richer querying is a reducer keeping its
own indexes, not new store operations. The operations are **async** (like the CAS,
section 8): the reducer awaits them, so a disk- or network-backed store fetches without
blocking the runtime, while a `get` stays deterministic — a pure function of the key against
the current state, regardless of backend latency.

A structurally-shared backend yields that root hash cheaply — sharing almost all structure
with the previous state — so the free-snapshot model (below) costs almost nothing; a
simpler backend still provides a root hash, just less cheaply. State reads during a fold are
point-in-time: folding event N sees state as left by folds up to N−1, never a live or
future value. State changes are not logged as their own events; they are the deterministic
side output of folding and rebuild themselves on replay, so the log stays thin.

Small values live inline; large values (transcripts, diffs, model payloads) are stored in
the shared content-addressed store and held in state as a hash.

### Snapshots and compaction

A **snapshot** is exactly `(event-index N, state-root-hash, program-hash)` — nothing more.
A valid snapshot exists at every event for free; which ones to keep is a retention choice,
not a compute one. Old raw events may be pruned behind a snapshot. A checkpoint that lets
recovery resume without the pruned prefix carries the derived resident facts the pruned
events would otherwise reconstruct: the state root, the id counter, the clock high-water,
the set of open (dispatched-but-unsettled) obligations and armed timers, the spawned-child
edges, and any close outcome.

**Compaction is controlled, not automatic.** The system decides when — and whether — to
prune, and the default is conservative: keep raw history, because the log is what lets an
operator replay and inspect a session after the fact to diagnose what went wrong. Pruning
trades that inspectability for space, so it is a deliberate retention policy (by session,
age, or tier), never an eager background sweep that quietly erases history and the ability
to debug from it.

### Replacing the program (self-modification)

Replacing a session's program is an authorized event naming a new program hash (a pinned
hash, never a mutable name). From the next event the runtime runs the new program, which
inherits the existing state. The only constraint is that the new program can read the
existing state's schema. Do not prune a raw event that would still be needed to
deterministically re-apply a replacement, and only compact behind a snapshot whose state
schema the current program can read.

### Reducer lifecycle (built-in effects)

Reducer lifecycle is the one thing the kernel manages, and it does so through built-in
effects — contracts the kernel's own built-in reducers answer. (Self-termination is the
exception: a reducer ends *itself* through the `Outcome` it returns, not an effect — see below.)
The built-in lifecycle effects:

- **spawn(program, handlers, init)** creates a new session running `program`. It carries the
  session's **handler configuration** — the chain of reducer identifiers for every contract
  that session will use (section 3), which is also where its authority lives (the authz
  middleware in those chains, section 5) — and an **init** `(Schema, Payload)` delivered as
  the new session's birth (section 3). There is no separate `capabilities` or `purpose`
  argument: authority is the authz middleware in the handlers, and purpose is content in the
  init payload. The runtime computes the child's id (the genesis hash), records a
  spawned-child edge in the parent's log and the parent link in the child; the link is
  immutable on both sides.

  **A child's authority is what its ancestors propagate to it.** A child inherits nothing
  automatically; a handler reaches it only when the reducer that handler is registered on
  chooses to propagate it (section 3). Whatever the child does hold, the system reducer wraps
  in the ancestors' middleware, so every effect the child emits passes through the parent's
  guards — and transitively every ancestor's — before it can reach an edge. A child's own
  handlers are **prepended** (adding interception or restriction); it can neither reach past
  an inherited guard nor remove one. This is what attenuates authority down the spawn tree and
  makes privilege escalation structurally impossible: a child can only ever do a subset of
  what its ancestors permit, because their authz middleware runs on everything it does
  (section 5).
- **set-handler(contract-id, chain)** installs or replaces the chain of reducer identifiers
  for a contract in a session — how a session is **upgraded over time** (a handler added, a
  chain extended or reordered) without respawning it; the handler analogue of replacing the
  program (above). It is answered by the system reducer, which owns the chain state, and
  registering a handler is what raises the new-handler notification (section 3) that lets a
  reducer decide whether to propagate it to its children.
- **list-handlers(reducer)** returns the contracts a reducer has handlers for — each as its
  `contract-id` and `schema`. The target may be the caller itself or any other reducer (so a
  reducer can discover what a peer can handle before messaging it). This is an **effect**: it
  passes through the middleware chain — so middleware can filter which handlers are visible
  or transform the result — and it is capability-gated like any other. Making even one's
  *own* listing an effect (rather than a direct read) is deliberate: it keeps handler
  visibility governable by middleware. Only the contract *surface* is exposed, never the
  chains behind it — the concrete reducer identifiers, the middleware, the credential
  brokers, the edge reducers answering a contract stay hidden. You see the interface a
  reducer has, not how it is implemented. (Only a reducer's own **id** is a direct read,
  section 3.)
- **subscribe(reducer, lifecycle-events)** asks the runtime to deliver another reducer's
  lifecycle events — spawned, closed (with outcome), failed — to the subscriber as effects.
  Any reducer with the capability may subscribe; supervision is one use (a parent subscribes
  to its children), but it is general pub/sub on lifecycle, not a hard-wired parent channel.
- **terminate(reason)** ends *another* session by authority over it. (A session ends
  *itself* by returning `Break(schema, reason)` from a reduce call — its reason for closure,
  section 3 — not by an effect.) However a session ends — a self-exit
  outcome, a terminate, or an uncontrolled fold-failure — subscribers receive the terminal
  outcome as a lifecycle event. Once a session's log tail is terminal the runtime refuses
  every further fold; the log and state are retained and queryable. There is no
  un-terminate — recovering from a bad state is a fresh spawn.

What a subscriber *does* with a terminal outcome or a failure it is watching — restart,
retry, escalate, give up — is a reducer's decision (a supervisor), not the runtime's. The
runtime provides the built-in lifecycle effects and records the first-class facts (spawned,
closed, terminated, failed); the strategy is out of it (section 11).

---

## 8. The store, and resolving dependencies

### One content-addressed store

There is exactly one store: a content-addressed **blob store** mapping a hash to its
bytes. That is its whole interface — put bytes and get a hash, get bytes by hash, ask
whether a hash is present. Everything the system keeps by hash lives in it: log blobs,
large state values, model payloads, contract declarations, and program
components. There is no separate component store. A WebAssembly component is not special —
it is bytes in the blob store, addressed by its hash exactly like any other value, and
fetching a program or one of its dependencies is the same get-by-hash as fetching a
payload.

Reducers and the kernel reach the store through **direct calls** — `cas-put(bytes) -> hash`
and `cas-get(hash) -> bytes` — not effects. This is deliberate: a reducer routinely resolves
and stores content mid-fold (deref a hash it was handed, put a value and keep the hash), and
routing every such touch through the async effect model would be needlessly clumsy. But
direct does not mean blocking: these are **async host calls** in the wasmtime setup — a
`cas-get` awaits, so the configured backend (in-memory, a local cache, disk, S3, whatever)
can fetch without blocking the runtime, and other sessions interleave at the await
(section 9). They stay deterministic despite the async: `cas-get(hash)` is a pure function of
the hash (content-addressed — the same bytes every time) and `cas-put(bytes)` a pure
function of the bytes, so awaiting a fetch changes only timing, never the fold's result.

**The CAS is unpermissioned: the hash is the capability.** You cannot forge bytes for a
hash, so possessing a hash both names and authorizes reading its bytes — there is nothing to
gate on a read. Confidentiality is not lost; it lives one layer up, at *name resolution*.
Which hashes a reducer ever comes to hold is controlled by the (userspace) name service and
whatever authorization wraps it, so a hash a reducer isn't meant to have simply never enters
its context, and it can freely `cas-get` anything it holds. Within a single operator that is
sufficient — you cannot read what you cannot name. (Cross-tenant confidentiality, where a
leaked hash would let one tenant read another's blob, needs more — e.g. per-tenant
encryption — and is out of scope here, section 11.)

For example, the **name service** is just a reducer storing name→hash mappings in its own
key-value store and answering resolution requests. Wrap it in an authorization reducer that
permits a child to resolve only a certain key prefix, and hashes outside that prefix never
reach the child. Access control lives there, in userspace, not on the CAS.

A hash is raw bytes. Wherever one is rendered as text — in a name, a log line, an error, a
display, or a textual wire field — it is **base64url** (the URL-safe alphabet, unpadded),
never hex.

### Resolving what a program needs

A program is a WebAssembly component (bytes in the store). It declares the other
components it depends on by content hash. The runtime resolves each dependency by fetching
its bytes from the blob store by hash and links them, treating every dependency
identically. The runtime has no built-in knowledge of any particular dependency — not by
name, not by interface prefix, not by identity. A Cadenza value-heap runtime, if a program
needs one, is just one more component in the store that the program happens to depend on;
the runtime never asks "is this the runtime?", only "what does this component declare it
needs, and can I fetch and link each by hash?". A missing dependency is an error;
transitive dependencies resolve by the same recursion.

### The kernel does not persist — storage is a reducer

The kernel holds the in-memory routing and the in-flight correlation state, and folds
reducers over their event streams. It does not write files and owns no log-store. Durably
recording a session's event stream — and reading it back to recover — is a **reducer
layer**: an edge reducer registered as the handler for the log's append and read contracts.
The kernel routes an append the same way it routes any effect; where the bytes land (a
local file, an in-memory buffer for tests, a replicated remote store) is that reducer's
business, swappable without touching the kernel. It composes with the chain (section 3): a
replication handler can wrap a local-file persistence handler, so one event is recorded
locally and shipped to a replica in a single chain, no kernel change.

This is the same shape as the blob store above — log persistence and content-addressed
storage are both boundary reducers the kernel knows nothing about. Determinism and replay
are unaffected: the kernel still defines the ordered event stream a reducer folds (single
writer per session, events in routed order, section 9); persistence only durably keeps and
replays that stream. The ordering discipline is the kernel's; the storage is the reducer's.

---

## 9. Determinism, replay, and scheduling

Determinism lives in the log, not in the scheduler. The guarantees:

- **A fold is a pure function of `(event, state)`.** The runtime is the only writer of a
  session's log, and it folds events in recorded order. The same event and state produce
  the same requests and the same state writes every time.
- **Nondeterminism enters only as recorded results.** A model output, an HTTP body, the
  time, a timer's fire — each is captured as a result event with its content and the order
  it landed. Replay re-reads those events; it never re-runs the effect or re-races
  concurrent effects.
- **Async scheduling does not affect fold results.** The node may run many sessions
  concurrently and yield a running fold at fuel intervals to interleave others. Which fold
  runs when is a scheduling decision, not a fold input. Fuel accounting is deterministic
  per `(event, state)`, and a fuel-exhaustion abort is a recorded fold-failure, so replay
  reaches the same outcome. If any host call the guest makes charged fuel in a way that
  varied with wall-clock or host state, that would be a determinism defect to fix, not to
  work around.
- **Canonical encodings.** Every event, value, state root, and contract has one canonical
  byte form, so equal things hash equal. This form is frozen within a runtime version.
  This document does **not** claim bit-identical replay across runtime versions; replay is
  within a version, which is what is needed now. Committing to a cross-version replay ABI
  (frozen engine, canonical float handling, frozen map layout forever) is deferred
  (section 11).

Determinism is also an audit and safety property: a program cannot act one way live and
replay benign, because replay reconstructs from the recorded log. Human inspection must be
able to render the raw log, not only a program's projection of it.

### Nothing blocks the runtime

Every reducer runs on the shared async runtime that interleaves sessions, so **no reducer
may make a blocking call on an async task.** A blocking filesystem read, a synchronous
network call, or a long CPU stretch inside an `async` body stalls the whole node and
defeats the multiplexing this design depends on — one blocked task freezes every session
sharing that thread. All input and output is non-blocking: edge reducers use async I/O, and
any operation that is inherently blocking (certain filesystem calls, spawning a subprocess)
is offloaded to a blocking pool, never awaited inline on the runtime thread. The blob
store, log persistence, and every edge reducer present an async interface for exactly this
reason; folds are bounded and yield by fuel (above). A blocking call in an `async` function
is a defect, not a shortcut.

### The log is not the model's context

The immutable log records what actually happened. The context a program assembles for a
model is a *projection derived from* the log, not the log itself. A program may present a
model a clean, compressed view (dropping resolved struggle) while the log keeps the full
truth — two objects, two consumers, one immutable source. This keeps replay and audit
intact while letting a program show a model a tidy history. It matters here because a
compiler gives a ground-truth check on what is valid, so a program can compress to a
verified outcome rather than a guess; the specifics are a program's concern, not the
runtime's.

---

## 10. What the log contains

The event vocabulary is small and grows only deliberately. Every event carries an
envelope: its position in the session, its causal parent (the event, possibly in another
session, that led to it), and room for a signature and producer identity (recorded when
multiple operators land, section 11). The body is one of:

- **genesis** — the session's first event: the program hash, the spawn-nonce, and the
  optional parent.
- **inbound(contract, input)** — something delivered into this session: a message from a
  peer, an ingress from outside, or an operator request. Identified by contract-id.
- **dispatched(contract, input, idempotency-key, deadline, continuation-token)** — the
  durable record written before an effect is routed, correlated by continuation-token
  (section 4).
- **result(contract, outcome, continuation-token)** — the outcome of a dispatched effect,
  correlated by its continuation-token. A denial is one such outcome (an error the authz
  middleware answered with, section 5), not a distinct kernel event.
- **timer-armed(deadline, continuation-token)** and **timer-fired(fired-time,
  continuation-token)** — the durable timer records (section 6).
- **fold-failed(reason, caused-event)** — a fold that trapped or exhausted fuel (section 3).
- **closed(reason)** / **terminated(by, reason)** — the session ended itself (it returned a
  `Break(schema, reason)` outcome), or was ended by another (section 7).
- **spawned(child)** — the immutable parent→child edge, recorded on spawn (section 7). A
  child's close/failure reaches subscribers as a delivered lifecycle effect, not a distinct
  kernel event.
- **checkpoint(descriptor)** — a durable snapshot of derived resident state for
  prune-and-recover (section 7).

No event carries a family, a content-type, or a version — each carries a single `contract`
(a contract-id).

---

## 11. Beyond this document

These belong to the larger vision but are outside this document's core. They are named so
the boundary is explicit — and, tellingly, each is composition over the primitives above,
needing no new kernel mechanism.

- **Supervision library** — reusable one-for-one restart, retry-with-backoff, and a
  restart-intensity ceiling, all as reducers composing the spawn / subscribe / close
  lifecycle effects of section 7. The built-in lifecycle effects are in scope; the reusable
  strategy library is later.
- **Shared memory** — capturing, distilling, recalling, and governing knowledge derived
  from logs (the promotion gate, reviewers, and gardener). A large userspace subsystem
  built entirely from sessions and effects; none of it needs new runtime mechanism.
- **Multiple operators and trust** — signed provenance, node enrollment and identity,
  short-lived scoped credentials brokered just-in-time, and cross-operator policy. The
  event envelope reserves room for a signature and producer identity from the start; the
  machinery that verifies them is later.
- **Federation** — many nodes in a mesh, effects routed to the right node by trust tier,
  and a hub that folds session reducers while edge nodes run only edge reducers. This
  document specifies a single node.
- **Ingress brokers** — turning outside happenings (chat, webhooks, streams) into signed
  events delivered to a session. Each is a narrow program that authenticates a source and
  translates a happening into an inbound event; not runtime mechanism.
- **Resource virtualization and a shared build cache** — separating the authority to fetch
  from elastic, unprivileged compute, and memoizing builds fleet-wide by content hash.
  Valuable, but a layer above the runtime.
- **Cross-version replay** — a frozen determinism ABI so a future runtime replays an old
  log bit-identically (section 9). Deferred in favor of within-version replay.

---

## 12. Implementation conventions

These bind the implementation — its code and the value model it marshals through — and they
are not negotiable.

- **A rich AST value model with first-class collections.** Everything crosses the wire as an
  `Ast` — payloads, schemas, contract declarations, birth state, break reasons — so the
  value model *is* the marshalling format. It carries first-class **collections (list, map,
  set)** and richer structured types (records, tagged variants); structured data is never
  encoded as string-tagged constructors over a bare atom/list s-expr, which is lossy (a list
  of pairs vs. a map), ambiguous, and slow to work with — a cost that would land on
  *everything*, since all marshalling flows through the AST.
- **`Bytes`, never `Vec<u8>`.** Every byte buffer is `bytes::Bytes` — no exceptions. A
  `Bytes` clone is an O(1) refcount bump, and payloads, event bodies, and blobs are cloned
  constantly as they thread through routing, dispatch, and results; a `Vec<u8>` on that path
  is a deep copy. Assemble a buffer as needed and freeze it into `Bytes`; readers borrow
  `&[u8]`.
- **`Str(Bytes)`, never `String`.** A text value is the newtype `struct Str(Bytes)` — a
  cheaply-clonable, `Bytes`-backed UTF-8 string — everywhere a value would otherwise be a
  `String` (or `Arc<str>`). No exceptions. An id, a name, a reason, a target render as text
  that is cloned as it flows through the system, and `Str` makes that a refcount bump rather
  than an allocation and copy. It also gives text and bytes one representation, so a value
  crosses the text/binary boundary without re-allocating.
- The other two standing conventions live with the topics they constrain: **base64url for
  every textual hash** (section 8) and **no blocking call on an async task** (section 9).
