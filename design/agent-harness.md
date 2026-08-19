# Agent harness

> An ultra-minimal, generic runtime for programs that react to events. This document is the
> vision — what the harness is, described on its own terms.

## Purpose

An agent harness is a small, generic runtime for running programs that react to
events. Its whole job is: accept an event into a log, run the current program for
that log, authorize the requests the program makes, carry them out, and record the
results back into the log as more events.

The runtime knows nothing about agents, models, tools, Cadenza, or any particular
capability. Adding a new thing an agent can do never means changing the runtime.
Everything specific is a program, a contract, or an event.

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

- **Effect** — a program requests `perform(contract, input)` and later receives the
  `output`. The output type in the contract is the result the program will get back.
- **Message** — a program requests `send(to, contract, input)`. A message is an effect
  whose contract output is unit: it is delivered as an inbound event in the target
  session's log and produces no reply to the sender.
- **Event** — everything delivered into a session's log arrives as `(contract, input)`.
  A program reacts to an inbound event by matching on its contract-id.

So "which requests may this program make", "which requests does this program answer",
and "what may be delivered to this program" are all sets of contract-ids. A program's
declared set of contracts is, at the same time, its **routing table** and its
**capability manifest** (section 5).

### Grouping contracts into names

A **group** is a named set of contracts. It is content-addressed and nominal exactly like
a contract:

```
group    = (name, members)        # members: contract-ids, and (nested) other group-ids
group-id = hash(group)
```

A group is a way to name, grant, and answer a set of contracts at once. Its uses:

- **Capabilities.** A capability may name a group instead of listing each contract:
  granting a group grants exactly its member contracts (section 5).
- **Answering.** A reducer may declare it answers a group, meaning it answers every
  member contract — an edge reducer answers the whole `http` group rather than each
  operation separately.
- **Organizing names.** Contract names stay plain labels; a group is how they are gathered
  into a named namespace (a `github` group over the `github.push` and `github.pull`
  contracts).

A group is an organizing and granting convenience, **not** a new matching rule. It never
softens dispatch: an effect always targets exactly one contract and routes by that
contract's exact hash (there is no "route to the group"). And membership is part of the
group's hash, so a group identity pins its members exactly — granting group `G`
authorizes precisely those members, and adding a contract to a group produces a *different*
group with a new hash rather than silently widening any existing grant. Referring to a
group by its hash is exact; referring to "the current version of a named group" is a
mutable-name resolution that freezes to a specific hash when used (the mutable-name
machinery is later work, section 11).

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
   purely by looking up a contract-id's handler in the registry (section 3). A new
   capability is a new contract plus something that answers it — never a runtime change.
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

The kernel is tiny. It is a single **reducer** interface plus a router that moves messages
between registered reducers — nothing more. A reducer receives a message, updates its own
state, and emits messages (effect requests) to be routed onward. That one interface is the
same for everything that runs: an agent's task, a handler that answers a single contract,
the boundary that performs input and output. There is no second trait — the thing that
folds events and the thing that carries out effects are the one reducer trait.

The kernel keeps a **registry** and does one thing with it: deliver a message to the
reducer that should receive it. A reducer is reached two ways:

- **as a handler for a contract** — `set-handler(contract-id, reducer)` registers a
  reducer to answer a given contract (or a group of contracts, section 1). A message
  tagged with that contract-id is delivered to that handler.
- **as an addressable reducer** — an **enveloped** effect addressed to a specific reducer by
  its **id**, point-to-point rather than routed by contract. The envelope names the target id
  and wraps an **interior effect**. The send is itself an effect with its own middleware
  chain: it bubbles up like any other (so authz, rate-limiting, and transform middleware
  enforce who may send what to whom — not a privileged bypass). At the top the platform
  **unwraps the envelope**, delivers the interior effect to the target reducer (which reduces
  it as its own inbound, tagged with the interior effect's contract), and returns a
  **delivery outcome** to the originator — delivered, or an error if no such reducer exists.
  That outcome is a delivery *acknowledgement*, not the target's reply to the interior
  effect; any reply is a separate message the target sends back.

Every message carries the **schema-hash — the contract-id — of what it is**, so the kernel
looks it up in the registry and moves the bytes. The kernel reads nothing else: it never
parses a payload and never matches a name. Registering a handler is the one registration
primitive; routing a message by its contract-id is the one dispatch step.

A handler is itself a reducer, so while answering one contract it can emit its own effect
requests, and those route onward to their handlers by the same lookup. Reducers emitting
messages that route to reducers, all the way down — that composition is the whole system,
and the kernel is only the router in the middle. Everything specific — input and output,
name resolution, authorization, lifecycle, timers, log persistence, an agent's brain — is a
reducer registered as a handler for some contract-id; the kernel itself contains none of it.

So the kernel is only: the reducer interface; the registry and the router that delivers a
message to the reducer for its contract-id; the scheduling that interleaves reducers and
carries each response back to the reducer that emitted the request; and a few direct
reducer-facing accesses with swappable backends — the key-value store, a reducer's own id,
and the content-addressed store (section 8). That is the whole irreducible core.
Authorization, name resolution, log persistence, lifecycle, timers, and input and output are
**not** in it — each is a reducer. This is the
point of the entire design and the line to hold: the kernel binary is the one thing that
cannot be hot-swapped, so it is deployed **once** and kept as small as possible, while
everything that will ever need to change is a content-addressed reducer, swapped by
reference without redeploying the kernel.

### Handlers chain

A contract's handler is a **chain of reducer identifiers** — an ordered list of references
to other reducers, not a single one. Registering a handler installs or extends that chain
(the set-handler effect, section 7). An effect request emitted by a **leaf** reducer
bubbles **up** the chain, and each reducer in it in turn may:

- **answer it** — produce a response, which stops the ascent and bubbles back down;
- **transform and forward it** — rewrite the request (even into a different contract) and
  pass it to the next handler up; or
- **emit its own effect requests** — which begin their own ascent through their contracts'
  chains.

When a handler answers, the **response bubbles back down** the same path — each handler
that forwarded may transform the response on the way — until it reaches the leaf reducer
that first emitted the effect. A chain is thus a stack of interceptors: a rate limiter
wrapping an HTTP handler, an authorizer wrapping a credential mint, a mock wrapping a live
edge reducer, a logger wrapping anything.

The kernel gains no new machinery for this. Chaining falls out of handlers being reducers:
forwarding an effect up is a reducer emitting an effect, and a response bubbling down is
that effect's result — resumed and re-emitted by the forwarding handler (the same
emit-and-resume of section 4). The kernel still only moves a message to the next reducer in
the chain and moves the response back; the stack is an ordered registry entry, not special
logic.

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
  either a small custom host import (a harness change) or a native reducer outside the
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
direct read (above), and lists its handlers via the `list-handlers` effect (section 7).
**Birth** is simply the
first `reduce` — its `Response` carries an initial schema and payload, the session's seed
state. That is all the kernel imposes; there is no separate "capabilities" or "purpose"
field. A session's authority is the authz middleware in its handler chains (section 5), and
any purpose or seed config is just content in the birth payload, interpreted by the reducer.

Configuration is not runtime state — it is early events the program folds into its own
state. A program's initial prompt, policy, or seed data arrive as ordinary events. The
events that set up a session can themselves be the output of another session.

### The reduce interface

A reducer is called with the response to a prior request (or a runtime-level error), and
returns the requests it now wants performed:

```
reduce(response: Result<Response, Error>) -> (list<Request>, Outcome)

type Response = {
  id:                 Hash,   # the contract-id = hash(schema); the routing key, not a per-message id
  schema:             Ast,    # the contract; its hash is the id
  payload:            Ast,    # the value delivered — an effect result, or an inbound message
  continuation-token: Bytes,  # echoes the request's token, so the reducer resumes
}

type Request = {
  id:                 Hash,             # the contract-id = hash(schema)
  schema:             Ast,              # the contract to perform; its hash is the id
  payload:            Ast,              # the input value
  continuation-token: Bytes,            # the reducer's token, returned on the response — the correlation
  deadline:           Option<Duration>, # optional: Err(Timeout) if unanswered within it
}

type Error   = Timeout | MissingHandler
type Outcome = Continue | Break(Ast)
```

What comes in is a `Result`. `Ok(Response)` is a handler's answer — an effect result, or an
inbound message delivered to the session. `Err(Error)` is a runtime-level failure to get any
answer: **Timeout** (the request's deadline elapsed) or **MissingHandler** (no handler is
registered for the contract). The error identifies the request it pertains to by its
`continuation-token`, so the reducer resumes the right recovery path. A handler's *own*
failure — an HTTP 500, a domain error — is not an `Error`: the handler answered, so it is an
`Ok(Response)` whose payload is the contract's output, which may itself encode a failure.
Whether to retry is therefore the reducer's judgment (from the error or the payload), not a
kernel field.

What goes out is a list of `Request`s, and a request may carry a **deadline**
(`Option<Duration>`): with `Some(d)`, if no answer arrives within `d` the reducer receives
`Err(Timeout)` and the runtime cancels the dispatch so no late answer ever folds; with
`None`, the request has no timeout. The deadline is the reducer's own per-request control
over how long it waits — attached at emission, not a fixed kernel policy.

The return is a **product**: the `Request`s to perform *and* an `Outcome`. `Continue` means
the reducer keeps running; `Break(Ast)` means it **terminates**, and the `Ast` is the reason
for closure — always present. The kernel imposes no normal-vs-error taxonomy on that reason;
whether a break is a clean completion or a failure is semantic content in the `Ast`, which a
subscribing supervisor interprets (section 7). Because the return is a product, a reducer
can emit final requests *and* `Break` in the same call (send a result, notify a peer, clean
up, then close); those final requests are dispatched, but the session will not fold their
responses (it is now terminal), so they are effectively fire-and-forget. A reducer ends
itself by returning `Break`, not by emitting a close effect. A reducer that instead *traps*
or exhausts its fuel cannot return at all; the runtime captures that as an uncontrolled
fold-failure (below).

`schema` and `payload` are Cadenza `Ast` values — the decoded form the reducer works with.
On the wire and to the kernel a message is its `id` (the contract-id, = `hash(schema)`) plus
the payload bytes; the kernel routes on that `id` and never decodes anything. The `schema`
Ast is resolved from the `id` for the reducer's view. So "a message carries its schema hash"
(the kernel's view) and "`schema: Ast`" (the reducer's view) are the same thing at two
levels: an id + bytes routed by hash below, a decoded schema and payload above.

`reduce` is a pure function of its input and the reducer's current state; a fresh instance
runs each call and holds no memory between calls. Alongside the response, the reducer has
access to **its own key-value store**, which it reads and writes during `reduce` to persist
state (section 7); state is not passed in the signature, and its changes are the
deterministic side output of the call, not separate events. The `continuation-token` is how
a reducer bridges the gap between emitting a request and its response arriving as a later
`reduce`: it stores what it needs in its key-value store keyed by the token, and looks it up
when the response returns.

Beyond the effects it returns, a reducer has a few **direct** accesses during `reduce` —
called in-line and resolving within the same call, not routed through the effect chain or
logged as separate events: its **key-value store** (read/write, above), its own **id** (a
fixed read), and the **content-addressed store** (`cas-get`/`cas-put`, section 8). All three
are deterministic — a reducer's own state, its fixed id, and content addressed by hash —
which is why they are direct rather than effects. A direct call may still *await* an async
backend (a `cas-get` reads from disk/cache/S3 without blocking the runtime, section 8); it
just resolves within the same `reduce`. The async *effect* model is for the nondeterministic
outside world; these three are not. Everything else — including listing the contracts a reducer has
handlers for, its own or another's — is an **effect** (section 7), so it passes through the
middleware chain and can be filtered or transformed there. (An effect's result is recorded,
so replay stays deterministic regardless.)

Two events belong in the same session only if they must be strictly ordered relative to
each other or share a retention lifecycle. Choose session boundaries by ordering and
shared fate, not by topic. The natural unit is one agent doing one bounded task.

### Terminating and failing

A reducer ends itself by returning `Break(Ast)` (above), the `Ast` being its reason for
closure (a clean completion or a failure — the reducer's own vocabulary). Distinct from that
is an
*uncontrolled* failure: a fold that traps, exhausts its fuel, or fails to instantiate, so it
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

### Dispatch is a registry lookup

Dispatching a request uses the registry from section 3: the request's contract-id resolves
to its handler, and the runtime moves the input bytes there and records the result bytes
that come back. That is the whole of dispatch — no enumerated effect kinds, no family
strings, no namespace prefixes, no per-effect branches; the runtime never recognizes a
request by a name in its own code, only by the handler a contract-id resolves to. Handlers
are registered as data at bootstrap and come from three places, all resolved the same way:

- **peer sessions** that declared they answer a contract (or a group of contracts,
  section 1);
- **edge reducers** the node provides for primitive input and output (network, subprocess,
  clock);
- the **runtime's own built-in reducers** for its structural operations — the reducer
  lifecycle effects (spawn / set-handler / list-handlers / subscribe / terminate, section 7),
  delivering a message to another session's log, and arming a timer — reached by a registered
  contract-id like any other reducer, never by a verb the runtime hard-codes. (The
  content-addressed store is reached by direct call, not an effect — section 8.)

So messaging, timers, and lifecycle are not special request kinds — they are contracts
answered by runtime-provided reducers. Sending a message is a request whose contract input
names a target session and a payload; arming a timer is a request against the timer
contract; spawning a child, closing, and replacing a program are requests against the
lifecycle contracts. The fold emits all of them the same way, and the runtime dispatches
all of them the same way. There is no hard-coded effect vocabulary anywhere in the kernel —
one registry, one dispatch path keyed on the contract-id.

### Correlating a result to its request

A reducer emits a request and returns; the answer arrives as a *later* `Response` (or an
`Error`), in a later `reduce`. The reducer does not block or resume a suspended stack —
there is no stack to resume, because each call is a fresh instance. Instead:

- The reducer chooses a **continuation-token** when it emits the request and stores whatever
  it needs to continue in its key-value store, keyed by that token.
- When the answer returns — the `Response` (or `Error`) — it carries the same
  `continuation-token`, so the reducer's next `reduce` reads the token, looks up its
  continuation, and proceeds.

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

A dispatched request resolves to one of (section 3, the `reduce` signature):

- **Ok(Response)** — a handler answered; the payload is the contract's output value. A
  handler's *domain* failure rides here too — the output type can encode it — because
  answering with an error is still answering.
- **Err(MissingHandler)** — no handler is registered for the contract; nothing could answer.
- **Err(Timeout)** — the request carried a deadline that elapsed with no answer. A timeout
  **cancels** the dispatch: the runtime guarantees no late answer for that request will ever
  fold, so a reducer never has to handle a response arriving after it gave up.

The reducer decides what a failure means for it — retry (re-emit, perhaps under a new
deadline), escalate, or give up. Retryability is its judgment, not a kernel classification.

### Answering an effect

An effect is carried out by whichever **reducer** answers its contract-id — a peer session
that declared it answers that contract, or one of the node's edge reducers (section 3).
There is no separate executor; answering is the ordinary reducer interface, and routing is
the same in both cases: find who answers the contract-id. An answering reducer may answer
immediately or accept the effect and settle it later; while it is unsettled the dispatched
event stays open and the caller's continuation waits. When the answer is ready, the
runtime records the outcome against the request's continuation-token and the caller resumes.

An effect for which no reducer answers is a recorded failure, not a silent drop.

---

## 5. Capabilities and authorization

A **capability** is permission to perform a contract against a resource:

```
capability = (contract-id, resource-predicate)
```

The contract-id names the interaction; the resource predicate constrains it and is
checked against the **resolved runtime input**, not just the contract. A capability is
not "may perform the HTTP-get contract" but "may perform the HTTP-get contract with a URL
whose host is in this allow-list"; not "may perform the push contract" but "with this
repository". The contract-id alone is necessary but not sufficient — the predicate over
the actual argument is what bounds the blast radius.

There is one grant shape: a contract-id (or a group) plus a predicate. What a session may
do is the set of contract-ids it is granted — not a probe over a fixed catalog of effect
kinds.

A capability may name a **group** (section 1) instead of a single contract; it authorizes
exactly the group's member contracts, each still checked against the resource predicate.
Because a group's identity pins its membership, a group-granted capability never widens
when someone publishes a differently-named or extended group.

A program's declared set of performable contracts is its manifest, derivable before it
runs. What a program needs, what a session may do, and what the authorizer checks are the
same set seen three ways.

### Authorization is middleware

Authorization is not a kernel step — it is **middleware in the chain** (section 3). An
authz handler is installed in the chain for the contracts it guards; an effect bubbling up
hits it, and it either **forwards** the request onward (permit) or **answers with a denial**
that bubbles back down to the leaf as the effect's outcome. The capability set and the
resource predicates are that middleware's own data and logic — the kernel holds none of it
and enforces nothing; it only routes through the chain. A policy engine such as Cedar is
one authz middleware, carrying its policies as content-addressed data referenced from the
log; it is a wasm reducer, swapped by publishing a new hash, never a redeploy.

Enforcement therefore rests on the chain being configured so the authz middleware wraps the
contracts it must guard — established at bootstrap and controlled by the authority to
register handlers, which is itself a capability gated the same way (grounded at the trust
root, section 11). A denial is an ordinary recorded result, auditable like any other — not
a special kernel event.

Down the spawn tree, enforcement compounds: a child **inherits its parent's middleware**
(section 7), so every effect a child emits traverses every ancestor's authz middleware
before reaching an edge. Authority therefore only ever attenuates downward — a child can add
restriction but never remove an inherited guard — so privilege escalation by spawning is
structurally impossible.

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
  (section 3); if no answer arrives in time, it receives `Err(Timeout)` and wakes to
  recover, and the runtime cancels the dispatch. A hung model call or shell command becomes
  an ordinary `Err(Timeout)`, not a stuck session — so a reducer that must not wedge on a
  hung answer sets a deadline; the anti-stuck guarantee is per-request and opt-in.
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
own indexes, not new store operations.

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
exception: a reducer ends *itself* through its `reduce` return, not an effect — see below.)
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

  **The child inherits the parent's middleware.** For every contract, the child's handler
  chain continues *into the parent's chain*, so every effect a child emits passes through the
  parent's middleware — and transitively every ancestor's — before it can reach an edge. A
  child's own handlers may only be **prepended** (adding interception or restriction); it can
  never detach that inherited tail, and `set-handler` on the child cannot remove it. This is
  what attenuates authority down the spawn tree and makes privilege escalation structurally
  impossible: a child can only ever do a subset of what its parent could, because the
  parent's authz middleware runs on everything the child does (section 5).
- **set-handler(contract-id, chain)** installs or replaces the chain of reducer identifiers
  for a contract in a session. This is how a session is **upgraded over time** — a new
  handler added, a chain extended or reordered — without respawning it; the handler
  analogue of replacing the program (above).
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
  *itself* by returning `Break(Ast)` from `reduce` — the `Ast` its reason for closure,
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
large state values, model payloads, contract and group declarations, and program
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
- **closed(reason)** / **terminated(by, reason)** — the session ended itself (its `reduce`
  returned `Break(Ast)`, the `Ast` being the reason), or was ended by another (section 7).
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
