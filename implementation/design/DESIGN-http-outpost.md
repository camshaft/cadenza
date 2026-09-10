# The HTTP outpost — an inbound HTTP/WebSocket edge served by content-addressed wasm handlers

**Status:** design/scoping only — nothing landed. Written 2026-09-10 by the `design-wasm-http` fleet
agent on the operator's spark (relayed via the concierge), verbatim:

> *"I want to build an http server that's powered by mini wasm modules. Basically the server would
> have a control server it connects to over websockets. And then the first message from the control
> server would tell it all of the current routes and all of the handlers for those routes. The server
> would download all of the handlers from the CAS store and set up the router. And then it would create
> local sessions with only in-memory kv stores. And then it would spawn sessions per request. We also
> want to be able to have websocket endpoints as well. And that would basically be our front-end to the
> agent platform where we can connect browsers or outposts and then these adapter http servers could
> handle MCP requests or return HTML or build APIs or whatever. The server would just need to be
> deployed once and we can install whatever software on it. We'll likely need to define some contracts
> for http requests and responses so the modules can respond to those events. For now we can just have
> modules that just respond inline just as a proof of concept."*

This design was worked autonomously from that spark against the current platform kernel
(`implementation/seed/crates/cdz-platform/`) and the up-to-date agent-platform design set in
`MembrainDev/docs/designs/agent-platform/`. It records each fork with a chosen default and escalates
only the one genuine operator fork (D1) as an `ask`.

### Lineage — this is the HTTP instance of an already-designed pattern

The operator's idea maps onto prior art almost one-to-one; this doc is the concrete HTTP realization,
not a new subsystem. (**Note on references:** `MembrainDev/…` paths below are **EXTERNAL** — they name
docs in the separate *MembrainDev* repository, the operator's agent-platform requirements/design set,
checked out locally alongside this repo; they are **not** in-tree paths. `implementation/design/…` and
`cdz-platform/…` paths **are** in this repo.)

- **`MembrainDev/.../patterns/api-gateway.md`** — "Federated API gateway (MCP and custom protocols)."
  The general pattern: *an edge reducer fronts the fleet for a foreign client over the network,
  authenticates it, and translates its calls into platform effects and results back.* Its own words:
  *"Nothing here is MCP-specific… the same adapter bridges any external API: an OpenAPI service, a gRPC
  endpoint, an LSP client, a custom developer API… expose the platform out or bring a custom API in."*
  **An HTTP server serving wasm handlers IS this pattern with HTTP as the wire.**
- **`MembrainDev/.../cadenza-platform-outposts.md`** — the trust/federation model this rides: outposts
  **dial *in*** to the fleet (§2); trust leans on an identity provider + **SigV4-style symmetric-key
  signing**, **bearer tokens rejected** (§3); the **baked-in surface is tiny** (§9: transport dial-in +
  opaque frames, signature verify + a root-designated authenticator, location-transparent message
  routing, **fetch-a-program-by-content-hash**, capability grants + `(reducer, host)` attribution,
  break-glass admin) — *"everything else is a governing program."*
- **`implementation/design/DESIGN-hub-federation-protocol.md`** — the **host = plumbing ONLY**
  discipline and the ws-edge seam shape (a listener surfaces `ws/connect`/`ws/frame`/`ws/disconnect` as
  inbound events; the reducer owns all protocol logic; **every frame is a `cadenza-ast` value-form doc**).
  Treat as raw material: its `ws_listen.rs`/`cdz-kernel` anchors are the superseded `agent-harness`
  codebase; the CURRENT kernel is `cdz-platform`, which has **no network edge yet** — this design adds
  the first one.
- **`implementation/design/DESIGN-browser-outpost.md`** — the browser is the *other* front-end (edge =
  DOM); an HTTP outpost is the front-end a browser or a foreign client connects *to* over the network.
- **`implementation/design/DESIGN-per-spawn-limits-and-spawn-capability.md`** — the per-spawn resource
  budget + `spawn` capability the per-request session lifecycle (§4) leans on.

### Grounding — the requirements this serves (MembrainDev agent-platform docs)

Mined from `problem-statement.md` (gaps), `tenets.md`, `goals.md`, and `how-it-works/*`:

- **Serves gaps:** 6 (*meet people where they work* — a browser/HTTP/MCP client reaches agents as peers),
  16 (*tools discoverable, inspectable, strongly typed over a standard protocol*), 14 (*harnesses are
  take-it-or-leave-it* — a federated endpoint lets any client inherit the fleet), 7 (*scoped, non-blocking
  authority* — a denial is a `403`, not a stall), 13/1 (*agents/clients reach each other by identity*).
- **Leans on tenets:** 1 (contracts are the API surface — an HTTP request maps onto a contract-id), **6**
  (*the core knows nothing specific* — the gateway is an ordinary program/reducer, never a kernel
  feature), 9 (sandboxed — the edge holds only granted capabilities), 10 (attenuation — authority a
  request inherits only narrows), 12 (auth/naming/routing are governing programs), 13 (location
  transparency — route to a session id wherever it runs).
- **Fits the mechanics** (`how-it-works/`): the gateway is a **handler in a chain** — the
  dispatch-and-supervision doc names *"a rate limiter wrapping an HTTP handler"* as the intended edge
  shape; handlers act via `forward`/`respond` and **deferral is free** (a handler may hold a request
  across folds and answer later — the async-response path for a handler that calls out before replying).
  A per-request session is **spawned** (synchronous, returns the id in-fold; id = hash of genesis =
  program + an externally-supplied **spawn-nonce** + parent), gets its `http-request` as its **seed**
  init payload, and ends with `Break(reason)`. The **minimal-core** doc is categorical: nothing
  HTTP-specific belongs in the kernel — which is exactly why the standalone gateway (§0.1) is the right
  home. **Honest durability caveat** (`state-durability-and-recovery.md`): in-memory-only state is *not*
  durable — a crash mid-request drops that in-flight request; for a per-request HTTP handler this is
  acceptable (the client sees a dropped connection and retries), and it is the operator's chosen
  "only in-memory kv stores" — but the design surfaces it as a caveat rather than pretending durability.

---

## 0. The one architectural claim everything follows from

**An "HTTP server powered by mini wasm modules" is a platform *outpost* whose *edge* is an inbound
HTTP/WebSocket listener. The listener is thin host plumbing that turns each request into a platform
event; every decision above the socket — routing, which handler answers, per-request session lifecycle,
auth — is a reducer fold over content-addressed programs. The handlers are ordinary reducers that fold
an `http-request` event and answer with an `http-response` value; nothing about HTTP reaches below the
edge.**

This follows the outposts tenet *"bake in as little as possible — the platform offers primitives and
builds its own operations as governing programs."* Concretely, the split is:

| Concern | Where it lives | Prior-art anchor |
|---|---|---|
| Accept a TCP/HTTP connection, parse a request, write a response | **native host edge** (new `HttpListener`, plumbing only) | federation `WsListener` (host = plumbing) |
| Dial the control server, keep the link, receive frames | **native host edge** (dial-in transport) | outposts §9 "transport — dial-in, opaque frames" |
| Fetch a handler program by hash | **host primitive** (the CAS `BlobStore`, `blobs` import) | outposts §9 "fetch-a-program-by-content-hash"; `blob_store.rs` |
| The **route table** (path/method → handler `ProgramHash` + contract) | **governing program** (a *router* reducer) | api-gateway "the registry lists what to expose"; outposts §4 |
| Spawn a per-request session, give it a fresh in-memory KV, fold the request, collect the response | **kernel** (`System::spawn`, `ProgramStore`, `InMemoryKvStore`) | `system.rs`, `program.rs`, `kv.rs` |
| Produce the HTTP response from the request | **handler reducer** (a shipped wasm module) | `reducer.rs` `on_message` → `(requests, Outcome::Break)` |

The host never learns what a route IS, what a handler DOES, or what MCP/HTML/an API is — exactly the
reviewer bar the federation doc sets (*"does the host know what a frame IS — it must not"*).

## 0.1 The gateway is decoupled from the core platform — build it NOW (operator directive 2026-09-10)

**Operator steer, verbatim:** *"I'm still trying to figure out the core platform implementation. But the
part that doesn't really care about the core implementation and persistence layer is this http gateway
component. And we can start iterating on that and building it quite quickly. So I think we should start
there."*

This is the load-bearing constraint on the whole design: **the HTTP gateway must NOT depend on the
core platform's session model, event registry, graph routing, supervision, or persistence layer** —
all of which are still being designed. It depends only on a **thin, already-frozen seam**, every part
of which has a trivial standalone in-memory implementation the gateway ships itself:

| Seam the gateway needs | Frozen? | Standalone v0 impl (in the gateway) | Later: the real core platform provides |
|---|---|---|---|
| **The handler ABI** — instantiate a wasm component and drive `on_message`/`on_response`/`on_notification` → `step` | ✅ the reducer WIT world (`cdz-platform/wit/world.wit`) is stable | a minimal `wasmtime` host in the gateway crate that binds that world | the kernel's `ProgramStore`/`host.rs` (identical world) |
| **`state`** — a KV the handler reads/writes | ✅ the `state` WIT interface | a fresh `InMemoryKvStore` per session (reuse `kv.rs`, or a 30-line map) | a swappable durable/replicated `KvStore` |
| **`blobs`** — fetch a handler component by hash | ✅ the `blobs` WIT interface | an in-memory / local-dir / HTTP-backed blob map | the real CAS (`blob_store.rs`) |
| **`identity`/`run`** — the session's id; a pure sub-run | ✅ the WIT interfaces | a minted `Hash` id; `run` may be unimplemented in v0 | the kernel's memoizing `run` |
| **spawn-a-session** | not needed as a kernel call | the gateway instantiates a fresh component instance per request itself | `System::spawn` with real supervision/persistence |

**Because a handler is nothing but a wasm component implementing the reducer WIT world, the gateway can
instantiate and drive one with a `wasmtime` host and in-memory backends — no core platform required.**
Concretely (verified against the current tree): a handler targets **`reducer-world`** (imports
`state`/`blobs`/`identity`/`run`; the privileged `event-reducer-world` adding `graph`/`deliver`/
`provenance` is kernel-only and the gateway does NOT wire it). `cdz-platform`'s `host.rs` (behind
`feature = "host"`) **already** binds this world via `wasmtime::component::bindgen!` on `wasmtime = "37"`
(Cargo features `runtime` + `cranelift` + `component-model` + `async`), with the `Config` set for
`async_support(true)` + `wasm_component_model(true)` + `epoch_interruption(true)`, keys the linker imports by
`ReducerKind` (an ordinary handler wires exactly `run`+`identity`+`blobs`+`state`), compiles+caches a
`Component` by content digest, and drives `call_on_message`/`_on_response`/`_on_notification` async — so
**the gateway can REUSE that host driver directly** (or copy its ~4-import subset), feeding it a
per-request fresh `InMemoryKvStore` (`kv.rs`) and a shared `InMemoryBlobStore` (`blob_store.rs`) — both
**already `pub` in `lib.rs`, not behind any feature**. Reusing `host.rs`'s *instantiation driver* does
NOT pull in `System`/`EventRegistry`/`ReducerGraph`/lifecycle/persistence — those are the decoupled core.
Per-request compute/memory bounding is the same **epoch-deadline + `StoreLimitsBuilder` memory ceiling**
`host.rs` already arms (not fuel). Persistence is explicitly out of scope for v0: per-request sessions
are ephemeral and
in-memory by design (the operator's "only in-memory kv stores"), which is *exactly* the case that needs
no durable-persistence layer. When the core platform lands, the gateway's seam is satisfied by the real
kernel (real CAS, durable state option, `System::spawn`, the federation control link) **with no gateway
rewrite** — the WIT world is the same on both sides.

**Consequence for where the code lives:** the gateway is a **standalone crate/binary** (e.g.
`cdz-http-gateway`) that depends on `wasmtime` + the reducer WIT world + `cadenza-ast` (for the
`http-request`/`http-response` codec), NOT on the full `cdz-platform` kernel. It can be built, tested,
and iterated today. (It MAY reuse `cdz-platform`'s `InMemoryKvStore`/`InMemoryBlobStore`/host bindgen as
libraries where convenient, but it does not depend on `System`/`EventRegistry`/`ReducerGraph`/lifecycle.)

## 1. The cast

Bottom-up, mirroring the federation doc's layering. Only the edge is native host code; everything above
it is a reducer fold + the wire schema.

```
  Handlers        per-route wasm reducers: fold http-request → http-response value      (reducer fold)
  Router          one governing program: holds the route table, spawns a session/request (reducer fold)
  Control link    dial-in ws to the control server; receive route table + fetch by hash  (reducer fold + CAS)
  HTTP EDGE       accept HTTP/WS, surface http/request + ws/* as events, write responses  (HOST plumbing)
```

| Participant | Kind | Behavior |
|---|---|---|
| **HTTP edge** | native host | Binds a listen port; accepts inbound HTTP requests and WebSocket upgrades; surfaces each as an inbound event to the router session; writes back the response the fold produces. Speaks no routing, no auth, no HTTP semantics beyond parse/serialize. |
| **Control link** | native host + reducer | Dials *in* to the control server over ws (outposts §2); the router reducer folds the first frame (the route table) and subsequent updates; handler bytes are fetched from the CAS by hash. |
| **Router** | session (governing program) | Holds the route table (path/method → `ProgramHash` + contract-id), received from the control server. On an `http/request` event, matches a route and **spawns a per-request handler session** (a fresh in-memory KV), delivers the request to it, and routes its `http-response` back to the edge. On no match → a 404 response. |
| **Handler** | session (per request) | A content-addressed wasm reducer. Folds the `http-request` message through `on_message`, computes the response, and returns `(requests, Outcome::Break{http-response})` — inline for the PoC (§7). Its state is a fresh `InMemoryKvStore` that dies with the request. |
| **WS session** | session (per connection) | For a WebSocket endpoint: spawned on upgrade, lives for the connection, folds `ws/frame` events and emits `ws/send` effects; closed on `ws/disconnect`. |

## 2. The HTTP edge (native host plumbing — the one new primitive)

`cdz-platform` today has no network edge (`src/`: `host.rs` wasmtime instantiation, `system.rs`
reducers, `blob_store.rs` CAS, `kv.rs`, `program.rs`, `graph.rs`, `deliver.rs`, `lifecycle.rs`,
`timer.rs` — no socket). This design adds the first one, gated behind a feature like `host` so the
routine build/gate never pays for it.

**`HttpListener` — symmetric to the federation `WsListener`.** It binds a configured port and, per
inbound request:

1. Parses the request into an `http-request` value (§5) — method, path, query, headers, body — and
   mints a **request-id** (`Hash`, the same unguessable-token scheme as a `SessionId`).
2. Delivers an **`http/request` inbound event** to the **router session** (a well-known local id, from
   genesis config), carrying the `http-request` payload and the request-id as the correlation token.
3. Awaits the settled response for that request-id — the router routes an `http-response` back (via the
   `deliver`/response path the kernel already has), and the edge **serializes it and writes it to the
   socket**. A request with no route or a faulted handler yields a host-synthesized `500`/`404` only as
   a floor; the router owns the normal error responses.

WebSocket upgrade is the same seam as the federation ws transport, reused verbatim:
`ws/connect(conn-id)` / `ws/frame(conn-id, bytes)` / `ws/disconnect(conn-id)` inbound events; `ws/send`
outbound. See §6.

**Discipline (the operator's twice-emphasized federation constraint):** the edge is opaque plumbing. It
does not hold the route table, does not decide which handler runs, does not authenticate. If an
increment wants the edge to match a path or check a token, that logic belongs in the router reducer.

**The dial-in control transport** is the same ws dial-out primitive the federation doc specifies
(`ws/dial(url) → conn-id`, dispatched-with-result, Cedar-gated on the URL). The adapter dials the
control server at boot; the router folds the connection's frames. *This is the same host transport the
federation lane needs; building it here is building it for federation too.*

## 3. The control link — route table + handler download

The operator's "first message tells it all the routes and handlers; download them from CAS; set up the
router" is, in platform terms: **the router reducer is shipped a route table as a message, and fetches
each handler program by hash from the content-addressed store.** No new mechanism — it composes the
outposts §9 primitives (transport dial-in + fetch-program-by-hash).

**The route-table message** (a `cadenza-ast` value; schema `route-table`, §5) is a list of routes:

```
route-table = [ { method, path-pattern, handler: ProgramHash, contract: ContractId } … ]
```

On receipt the router:
- stores the table in its own KV (its durable state; `state` import),
- **warms the CAS**: for each distinct `handler` hash, ensures the component bytes are present via the
  `blobs` import (`get(hash)`; if the control server is the source of truth, the bytes are pushed to the
  local blob store or fetched from a fleet-side store — a `blob-get`/`blob-put` contract already exists,
  `contracts/userspace/blob-{get,put}.cdz`).

A handler is **content-addressed** (`ProgramHash` = the hash of its wasm component, `ids.rs`); shipping
a new version of a route is publishing a new hash and sending an updated `route-table` — no adapter
redeploy (the outposts "swap by hash" property). Route-table **updates** are ordinary follow-up messages
the router folds; the table is live-swappable.

**Relationship to federation:** the "control server" is the fleet hub of `DESIGN-hub-federation-protocol.md`;
the route table is the specific governing-program payload the hub ships an HTTP outpost on connect. This
design does **not** require federation to land first — v0 uses a minimal direct control protocol (D2) —
but is built to converge with it (the wire is `cadenza-ast` frames either way).

## 4. The router + per-request session lifecycle

On an `http/request` event the **router** (`on_message`):

1. Matches `(method, path)` against its route table. **No match → an `http-response` 404** returned to
   the edge; done.
2. **Match →** spawn a fresh per-request handler session. **In the standalone v0 (§0.1) the gateway does
   this itself** — its own `wasmtime` host instantiates a fresh component instance of the handler,
   wiring it a fresh in-memory `state`. When the core platform lands (P5) the identical step is
   `System::spawn` with a `Spawn` describing the handler's `ProgramHash` and a `SpawnContext { id, kind,
   limits }` (`system.rs`, `program.rs`) — same WIT world, so the handler is unchanged. Either way the
   spawn:
   - gives the session a **fresh `InMemoryKvStore`** (`cdz-platform::kv`) as its `state` backend — the
     operator's "local sessions with only in-memory kv stores." Per-request state is born empty and
     reclaimed when the session closes; nothing leaks between requests.
   - clamps resources via `SpawnLimits` (`cdz-platform::config`) — a per-request session gets a bounded
     **epoch-deadline (compute) + linear-memory-ceiling** budget (the platform bounds via
     `epoch_interruption`/`StoreLimits`, NOT wasmtime fuel;
     `DESIGN-per-spawn-limits-and-spawn-capability.md`), so a hostile or runaway handler cannot wedge
     the node.
   - is created under the router's `spawn` capability (privileged; the router is a trusted governing
     program, the handlers are not — attenuation, api-gateway §authority).
3. **Delivers the `http-request`** as the handler's first `on_message` (after its `Spawned` birth
   notification, `spawned.rs`), correlated by the request-id token.
4. The handler answers with an `http-response` value and **closes** (`Outcome::Break { schema:
   http-response, reason: <response bytes> }`, `reducer.rs`) — the response rides the close reason, or,
   for handlers that need to emit effects first, as a `deliver`-response before the `Break`. The router
   (as the session's supervisor/lifecycle subscriber, `lifecycle.rs`) reads the response off the
   session's exit and routes it to the edge for the request-id.

This is the "spawn a session per request" model exactly: **ephemeral, isolated, budget-bounded, in-memory
state, one fold, closed.** A long-lived handler that wants to accumulate state across requests is a
variant (a persistent session the router messages instead of respawning) — reserved, not v0 (D4).

## 5. The HTTP contracts — the event interface the modules implement

Everything on the platform is a **contract** — a typed, named, content-addressed schema (a `.cdz` file
under `contracts/`, codegen-checked by `cdz`; `contracts/userspace/*.cdz`). The operator's "contracts
for http requests and responses so the modules can respond to those events" are two new userspace
contracts. Sketch (house style, cf. `contracts/kernel/deliver.cdz`):

**`contracts/userspace/http-request.cdz`**
```
type Method = | Get | Post | Put | Delete | Patch | Head | Options
type Header  = Record(name: String, value: String)
type Request =
  | Request(Record(method: Method,
                   path: String,
                   query: String,
                   headers: List(Header),
                   body: Bytes))
```

**`contracts/userspace/http-response.cdz`**
```
type Header   = Record(name: String, value: String)
type Response =
  | Response(Record(status: Int,
                    headers: List(Header),
                    body: Bytes))
```

A handler is a reducer whose `on_message` decodes the `http-request` payload against this contract,
computes a `Response`, encodes it, and returns it on close. Because these are ordinary `cadenza-ast`
value-form contracts, they carry over the wire (control link, cross-node) and to the browser outpost
unchanged (one codec everywhere, the federation-doc rule). MCP, HTML, and REST are **not** platform
concerns: an MCP handler folds `http-request` bodies that happen to be JSON-RPC; an HTML handler returns
`body` = HTML bytes with `content-type: text/html`. The edge and kernel stay protocol-neutral (api-gateway
tenet 6).

**`route-table` and control frames** (§3) are likewise `cadenza-ast` contracts; v0 needs `route-table`
plus a `hello`/`welcome` handshake pair (borrowed from the federation frame schema, D2).

## 6. WebSocket endpoints

A route may be a **WebSocket endpoint** rather than a request/response handler. On an HTTP upgrade the
edge surfaces `ws/connect(conn-id)`; the router matches the path and **spawns a per-connection session**
(not per-request) bound to that conn-id, again with a fresh in-memory KV. Thereafter:
- inbound `ws/frame(conn-id, bytes)` → delivered to the connection session's `on_message`;
- the session emits `ws/send(conn-id, frame)` effects to push;
- `ws/disconnect(conn-id)` → the session closes and is reclaimed.

This is the *same* ws seam the federation transport defines — an HTTP outpost's WebSocket endpoint and a
federation link are the same host plumbing, differing only in which reducer owns the connection. A
browser connecting a WebSocket to an HTTP outpost is thus "another wasm-running participant messaging a
session," the browser-outpost vision realized over this edge.

## 7. The PoC path — inline-responding modules

The operator's "for now we can just have modules that just respond inline as a proof of concept" is the
degenerate, most-valuable first slice: a handler that computes its `http-response` **purely in
`on_message`, emits no effects, and closes immediately**. Sketch (house style, cf. the default-handler
guest):

```
// guests/http-hello/reducer.cdz — an inline PoC handler
import { Outcome } from "reducer-lib"
import { Request } from "http-request"
import { Response } from "http-response"

def on-message(msg) =
  // decode msg.payload as an http-request, build a 200 text/plain "hello"
  let resp = Response.Response({ status = 200,
                                 headers = [{ name = "content-type", value = "text/plain" }],
                                 body = b"hello from a wasm handler" }) in
  { requests = [],
    outcome = Outcome.Break({ schema = http-response-id(), reason = encode(resp) }) }

export { on-message, on-response, on-notification }
```

This proves the whole spine end-to-end — edge → router → spawn → fold → response → socket — with the
smallest possible handler, and every later capability (routing, CAS download, MCP, HTML, effect-emitting
handlers) is an increment on top.

## 8. Increments (each its own commit + gate; top-to-bottom, vertical-landable)

The ordering front-loads the **standalone gateway** (P0–P2, buildable NOW with no core platform, §0.1)
and defers everything that touches the still-unsettled kernel/persistence/federation to non-blocking
later increments.

- **P0 — the `http-request`/`http-response` contracts.** Add the two `.cdz` files (in the gateway crate,
  or `contracts/userspace/` if the `cdz` codegen lives there), with `@test` conformance proofs (a
  fully-literal value of each ascribed against the schema, cf. `deliver.cdz`). Gate: `codegen --check`
  green + the contract `@test`s pass. Pure schema — no host, no edge, no kernel. Independent.
- **P1 — the standalone gateway spine (new `cdz-http-gateway` crate; NO core platform).** An HTTP
  listener (greenfield — there is **no** existing http/ws/socket code in the tree to reuse or conflict
  with; pick a `tokio`-based server, e.g. `hyper`/`axum`); a **static** route table (one route → one
  handler component `.wasm`, loaded from a local path); and a `wasmtime` host driving `reducer-world`
  handlers — **reusing `cdz-platform`'s `host.rs` driver (feature=`host`) or a copied 4-import subset**
  (§0.1). Per request: instantiate a fresh handler `Store` with a fresh `InMemoryKvStore` (`state`) +
  the shared `InMemoryBlobStore` (`blobs`) + a minted id (`identity`), armed with an epoch deadline +
  memory limiter; decode the `http-request` into the handler's first `on_message`, fold it, read the
  `http-response` off the close, serialize it. Handler `.wasm` is produced by `cdz compile <src.cdz> -o
  handler.wasm` (`rcdzc::compile_component`, a codegen-time tool — it never ships in the gateway). Deps:
  `wasmtime = 37` + `cdz-platform` (for the bindgen'd world + the in-memory stores) + `cadenza-ast` (the
  `http-request`/`http-response` codec) — NOT `System`/`EventRegistry`/persistence. **This is the
  operator's "start there, build quickly" slice** — the whole spine end-to-end with the **inline PoC
  handler (§7)**. Gate: a hermetic in-process test — `GET /` served by the PoC handler returns its 200;
  an unmatched path returns 404; the per-request `state` is empty on each request (no cross-request
  leak); a handler that spins is epoch-trapped → 500 (bounded).
- **P2 — the router as a governing program + per-connection isolation.** Lift the static match from P1
  into a **router reducer** (a shipped wasm component) holding the route table as its own state, so
  routing is a fold, not gateway code (the "bake in as little as possible" cut). Per-request handler
  instances get bounded resources (a wasmtime **epoch-deadline + linear-memory-ceiling** budget — the
  standalone analogue of `SpawnLimits`, the same mechanism `host.rs` arms; not fuel). Gate: routing
  across ≥2 routes through the router reducer; a hostile handler that spins is bounded (epoch-trapped →
  500), not able to wedge the gateway.
- **P3 — the control link + handler download.** The gateway dials the control server (ws dial-in); the
  router folds a `route-table` frame and fetches each handler component **by hash** from a blob source
  (an in-memory/local/HTTP-backed `blobs` impl in v0; the real CAS later). Gate: a two-endpoint hermetic
  test — a stub control server ships a route table + a handler blob; the gateway downloads it, sets up
  the route, and serves it; a route-table update swaps a route live.
- **P4 — WebSocket endpoints.** Upgrade → a per-connection component instance → `ws/frame` fold →
  `ws/send`. Gate: a hermetic ws echo endpoint — a client connects, sends a frame, the session echoes it.
- **(auth — NONE in v0.)** Per D1, authn/authz is an external front proxy's job for now; the gateway
  trusts its authenticated ingress. No auth increment in this arc. The in-platform SigV4/outposts trust
  model arrives with the P5 federation convergence, replacing the external proxy.
- **P5 — hook up the real core platform (NON-BLOCKING; when the kernel lands).** Swap the gateway's
  standalone seam impls (§0.1) for the real kernel: `blobs` → the real CAS, `state` → the durable/
  replicated `KvStore`, per-request instantiation → `System::spawn` with real supervision, the control
  link → the federation hub (and the in-platform SigV4 trust model replaces the external proxy). **Zero
  handler or contract change** — the WIT world is identical on both sides. This is where the gateway and
  the core platform converge; nothing above depends on it landing.
- **P6 — RESERVED.** MCP-over-HTTP handler (the api-gateway instance: a handler folding JSON-RPC bodies,
  registry-driven discovery), effect-emitting handlers, persistent per-route sessions (D4), streaming
  responses (SSE / chunked). Each rides P0–P4 with no edge change.

(P0 independent. **P1 is the standalone spine and depends on nothing but P0 + wasmtime** — build it
first. P2 depends on P1; P3 on P2; P4 reuses the ws seam independently. **P5 is deliberately last and
non-blocking** — the gateway is fully useful on its own in-memory seam, behind an external auth proxy,
before the core platform exists. Each increment is independently green.)

## 9. Open decisions (each a chosen default; escalate only a genuine fork)

- **D1 — the trust model. RESOLVED by the operator (2026-09-10): auth is an EXTERNAL PROXY LAYER for
  now.** *"For auth we can just rely on another proxy layer for now."* So **v0 does NOT do authn/authz in
  the gateway** — a separate front proxy (TLS termination + authentication, a standard reverse proxy /
  sidecar) sits ahead of the HTTP edge and the gateway trusts its authenticated ingress. This drops auth
  entirely off the v0 critical path (the old P5 is deleted; the spine P1–P4 needs no handshake secret).
  The eventual in-platform trust model — identity-provider + **SigV4-style symmetric-key signing**,
  bearer tokens rejected, governance in programs (outposts §3) — is the *later* story that lands when the
  gateway federates directly into the fleet (the P5 convergence), replacing the external proxy. Recorded;
  no open fork.
- **D2 — the v0 control protocol: minimal-bespoke vs. full federation frames.** Default: a **minimal
  control protocol** (`hello`/`welcome` + `route-table` + update, as `cadenza-ast` frames) that is a
  strict subset of the federation frame schema, so it converges when federation lands. Rejected:
  blocking on the full federation protocol (deferred). No escalation — the subset is forward-compatible.
- **D3 — the HTTP edge: native `HttpListener` vs. a `wasi:http` incoming-handler reducer.** The
  cadenza-platform vision has a hard rule — *"if a capability can be expressed through a standard WASI
  interface, it must be a WASI-based wasm reducer, never a custom host import."* `wasi:http` has an
  incoming-handler. **Default: native `HttpListener` plumbing** — because the adapter is a *node* that
  also maintains the control link, downloads handlers, and spawns per-request sessions (orchestration
  that is the node's job, not a single wasi:http reducer's), and because it reuses the federation
  host=plumbing precedent exactly. The handlers stay pure reducers. The `wasi:http`-reducer alternative
  is noted for a future "the edge itself is a shipped wasm reducer" refactor; it does not change the
  handler contract (§5). Flag the WASI-rule tension to the reviewer; no v0 escalation (the operator's
  "deploy once, install software on it" framing describes a native node).
- **D4 — per-request session vs. persistent handler session.** Default: **per-request ephemeral session**
  (spawn → fold → close) — the operator's literal "spawn sessions per request," maximally isolated. A
  persistent per-route session (state accumulates across requests; the router messages it instead of
  respawning) is a reserved variant (P6) for handlers that need continuity. No escalation.
- **D5 — where the route table originates in v0.** Default: **shipped by the control server** (§3), with
  a **config-seeded static table** as the P2 stand-in before the control link (P3) lands, so P2 is
  testable without a control server. No escalation.
- **D6 — response-carrying mechanism: `Outcome::Break` reason vs. a `deliver`-response before close.**
  Default: **the closing `Break` reason carries the `http-response`** for a pure inline handler (§7);
  a handler that must emit effects first sends a `deliver`-response then `Break`s. Both are supported;
  the router reads whichever arrives. No escalation.

## 10. Watch-outs (for the implementing vertical)

- **Host = plumbing ONLY.** The edge must not know a route, a handler, MCP, HTML, or a token. Reviewer
  bar: *does the host decide anything above the socket?* — it must not. All policy is the router fold.
- **`cadenza-ast` value-form everywhere — invent no wire format.** The `http-request`/`http-response`
  payloads, the `route-table`, and every control/ws frame are the one canonical codec (the federation +
  fold-boundary rule). No JSON framing on the wire; a handler that serves JSON puts JSON in the
  `body: Bytes`, it does not reframe the envelope.
- **Per-request isolation is the correctness bar.** A per-request session's `InMemoryKvStore` is born
  empty and reclaimed on close; assert no cross-request state leak and that `SpawnLimits` bound a runaway
  handler (a per-request fold cannot exhaust the node).
- **Content-addressed handlers, no redeploy.** A route change is a new `ProgramHash` + a `route-table`
  update the router folds live; never a native adapter rebuild. The "deploy once" property depends on
  this staying true.
- **Reuse the ws seam, don't fork it.** The WebSocket edge and the federation control link are the same
  host ws plumbing; build one transport, not two.
- **Validate foreign input at the boundary (api-gateway §4).** An HTTP request is the canonical
  "came from outside" case — the router/handler validates the decoded `http-request` against the contract
  schema before trusting it; a malformed body is a 400, not a fold fault.
- **MCP impedance is real (defer to P6, but design around it).** MCP is JSON-RPC/JSON-Schema and (as of
  2026-07-28) stateless with OAuth 2.1 **bearer tokens** — which the platform trust model *rejects*. An
  MCP handler is an ordinary HTTP handler whose `body` is JSON-RPC; the JSON↔`Ast` schema marshalling and
  the OAuth-terminate-at-edge / re-sign-inward seam are the known strains (api-gateway §6.2). In v0 this
  is entirely inside the external proxy + a handler module — the edge and kernel stay protocol-neutral.

## 11. Coordination + gate

- **The gateway vertical (new — suggest `v-http-gateway`, area = the `cdz-http-gateway` crate)** — owns
  the standalone spine end-to-end: the contracts (P0), the `wasmtime` host + HTTP edge + per-request
  instantiation + in-memory `state` (P1), the router-as-governing-program (P2), the control link (P3),
  and the ws seam (P4). **This vertical can start NOW and does not block on `cdz-platform`** (§0.1) — it
  is the operator's "start there, build quickly" component.
- **`v-platform` / cdz-platform kernel** — owns the P5 convergence only: satisfying the gateway's seam
  with the real CAS, durable state, `System::spawn`, and federation. Not on the gateway's critical path.
- **transport plumbing** — the ws dial-in/listen the control link (P3) and ws endpoints (P4) reuse. In
  the standalone gateway this is an ordinary `tokio`/`tungstenite`-style ws client/server the crate owns;
  it converges with the federation `ws/dial`/`ws/send`/`ws/frame` seam (federation D4) at P5.
- **`v-syntax`** — owns `cadenza-ast`; P0/P3 use the codec, no codec change.
- **the guest lane** — authors the router governing program and the PoC handler (§7) as Cadenza guests.
- **Gate discipline:** every increment proves on an **in-process hermetic test** first (deterministic,
  no real network — a stub control server + stub edge in-process), then a hermetic live-socket E2E per
  the nix rule. The PoC handler served end-to-end (P2) is the spine's acceptance gate; each later
  increment adds its own case. A handler's behavior is corpus-gated as an ordinary reducer fold.
