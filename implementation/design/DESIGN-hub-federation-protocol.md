# The hub/outpost federation protocol — cross-node sessions over a websocket wire

**Status:** design/scoping only — nothing landed. Written 2026-08-10 by the `design-hub-federation`
fleet agent on the operator's standing directive (relayed via the concierge, itself relayed via
`v-agent-harness-host`): *"if the harness host vertical runs out of work it should start building out
the hub/outpost functionality — specify a hub, connect, federate; use websockets; send binary ASTs
everywhere. Collaborate on a design if not defined."* The operator is iterating asynchronously; this
design makes the engineering calls autonomously from the stated direction, records each open fork with
a chosen default, and escalates only a genuine unresolvable fork as an `ask`.

This doc specifies the **federation PROTOCOL that rides the websocket transport** `v-agent-harness-host`
is building. It is the missing companion to `DESIGN-the-outpost-host-websocket-federation-node`
(memory: `design-the-outpost-host-websocket-federation-node`), which covers the host SERVING side
(peers dial *in*) and the host = plumbing-only discipline, but NOT the hub-federation protocol itself —
the handshake, what federates, hub-side topology, and the wire format. It hands a build plan to a
`vertical` owner and coordinates `v-agent-harness-host` (the ws transport seams), `v-agent-harness`
(kernel session semantics), `v-syntax` (`cadenza-ast`, the wire codec), and the guest/reducer lane
(`v-metaprog`) that authors the federation policy.

---

## 0. The one architectural claim everything follows from

**Federation is cross-session messaging with a network hop — and the kernel's session model is already
location-transparent, so the federation protocol is entirely a userspace REDUCER protocol; the host
adds exactly one new plumbing primitive.**

The kernel already gives every session a location-independent identity and a fire-and-forget mailbox:

- A `SessionId` **IS** a genesis `Hash` — a `Copy [u8;32]` blake3 content hash (`host.rs:74`, operator
  ruling #2362). It carries no host/node coordinate. The same `Hash` scheme identifies ws connections
  (`ws_socket.rs:94` `mint_conn_id`) — the operator's "anything stateful looks like a session" capstone.
- One session messages another with the `Emit` effect (family `"emit"`, `effect.rs:61`): `target` =
  the peer's `SessionId` hex, `payload` = opaque bytes. The host's `EmitExecutor` (`emit.rs`) routes an
  `Inbound { content_type { family: "message", version: 1 }, payload, reply_to: Some(sender) }` into the
  recipient's inbox. Undeliverable → a `delivery-failure` Inbound bounces back to `reply_to`, echoing
  the payload so the sender correlates (`async_host.rs:38,93`). Fire-and-forget; the host never inspects
  the message schema (provenance is whatever the sender bakes into the payload).

Nothing in that triad names a *node*. `SessionId = Hash` is a flat, global, node-agnostic namespace. So
"session A on node N1 messages session B on node N2" is the SAME `Emit`→`Inbound{family:"message"}`
semantics — with a wire hop inserted where the local `EmitExecutor` would have delivered in-process.

`DESIGN-platform-conformance-suite.md` (MD2) makes this binding explicit and load-bearing: it names THIS
outpost/federation work as the production cross-node router carrying *"the same `Emit`→peer-`Inbound`
messaging and the same effect-handler-session deferral over a WebSocket wire between federation nodes,"*
and positions the platform-conformance suite as **the in-process deterministic conformance oracle** for
exactly this behavior — identical semantics (family `"message"`, target=`SessionId`, fire-and-forget,
`delivery-failure` bounce, cause provenance) whether delivered in-process or over the ws transport. That
gives this design a ready-made gate: a federation route must be observationally identical to an
in-process Emit under the platform-conformance FIFO drive.

**The consequence for the host (the HARD operator constraint — host = plumbing ONLY):** the host learns
NOTHING about federation. It moves opaque frames over ws (both directions), surfaces connect/frame/
disconnect as `Inbound` events, and routes an outbound frame by conn-id. ALL federation logic — the
handshake, authentication, the routing table, what to federate, topology — is the **federation
reducer's fold over its own event log**, plus the Cedar authorizer. This is the same plumbing/policy
split as `admin_socket.rs` and the existing `ws_listen.rs`; federation adds no new host policy.

## 1. The layers

Four layers, bottom-up. Only Layer 0 is host code; Layers 1–3 are the reducer's fold + the wire schema.

```
  Layer 3  FEDERATION PROTOCOL   handshake · message routing · directory · topology   (reducer fold)
  Layer 2  FEDERATION SESSION    one session per node owns the peer ws connections     (reducer fold)
  Layer 1  WIRE CODEC            every frame is a cadenza-ast value-form document       (cadenza-ast)
  Layer 0  WS BYTE TRANSPORT     dial-out (NEW) + listen (exists); opaque frames        (HOST plumbing)
```

### Layer 0 — the ws byte transport (host plumbing; one NEW primitive)

The host already has the **server** half (`ws_listen.rs` / `ws_socket.rs`): a `WsListener` binds a port,
accepts peers that dial *in*, mints a conn-id `Hash` per connection, registers its outbound sink via a
`WsControlOp`, and surfaces `ws/connect` (inbound event, conn-id), `ws/frame` (inbound opaque bytes),
`ws/disconnect` (inbound event) to the outpost session; the reducer sends outbound with the `ws/send`
effect (`ws_exec.rs`, target = conn-id hex, `effect.rs:317` `is_ws_family`).

Federation needs the **symmetric client half**: a node must be able to *dial out* to a hub. This is the
ONE new host primitive (`v-agent-harness-host` is building it now, symmetric to `WsListener`). Its seam,
mirroring the listener:

- **`ws/dial` — a NEW OUTBOUND effect family** (reducer → host): `target` = the hub URL bytes (e.g.
  `ws://hub.host:PORT/`), `payload` = none. The host dials, performs the ws client handshake, mints a
  conn-id `Hash` for the connection (same `mint_conn_id` scheme), registers its outbound sink in the
  SAME `LiveWsConnRegistry` the listener populates, and — on success — surfaces the minted conn-id back
  so the reducer can address `ws/send` to it. After that, an outbound-dialed connection is
  **indistinguishable** from an inbound-accepted one: the reducer sends via `ws/send` and receives
  `ws/frame` / `ws/disconnect` on the same seam.
  - **⚠ Naming: do NOT reuse `ws/connect`.** `ws/connect` is already taken as an INBOUND EVENT family
    (host → reducer, "a peer connected"; `effect_ct::WS_CONNECT`, an event, NOT in the grantable `ALL`
    set). The dial-out EFFECT (reducer → host, "please dial this URL") is a distinct, grantable,
    authz-gated family — **`ws/dial`** — so it does not collide. Its result surfaces the minted conn-id;
    see D4 for whether the conn-id comes back as the effect RESULT (dispatched-with-result, http/model
    shape) or as a fresh `ws/dial-ok` inbound event. **Chosen default: dispatched-with-result** — the
    reducer emits `ws/dial(target=url)` and the settled `EffectResult` carries the conn-id hex (or a
    permanent Err on dial failure). This matches `mcp/call`'s dispatched-with-result shape and needs no
    new event family. A subsequent `ws/disconnect` inbound still fires when the dialed link drops.
  - Authz: `ws/dial` is Cedar-gated on `target` = the URL (a `HostIn` predicate — the SSRF guard already
    used for `http`, `effect.rs`), so which hubs a node may dial is policy, granted per node config. The
    host dials only an authorized URL; it never decides WHICH hub — the reducer emits the effect, Cedar
    admits or denies the target.

Everything above Layer 0 is opaque application bytes to the host — it never parses a federation frame.

**The three transport-seam questions `v-agent-harness-host` raised (answered, so the dialer builds to
the protocol).** They own the ws plumbing and are building the base client dialer (connect + frame pump +
lifecycle events, NO reconnect) first. Their questions and the resolutions — all of which keep the host
oblivious to federation, so none add host policy:

1. **Does the client dialer need any transport knob beyond `{hub_url, outpost_session_id, reconnect
   policy}`?** — No knob beyond `{hub_url, outpost_session_id}`, and reconnect is NOT a transport knob
   (see Q3). The dialer needs the URL to dial and the session to address inbound events/`ws/dial` result
   to. Everything else (identity, auth, what to send) is a frame the reducer emits AFTER the connection is
   up — it is not dialer configuration. So the base dialer they are building is exactly right; no extra
   knob. (If a TLS/timeout transport detail is later needed it is a dialer config, still not federation
   policy — but v0 needs none.)
2. **Is conn-id the right federation-peer handle, or does the protocol need a stable hub identity
   surfaced at connect?** — **conn-id is the right — and ONLY — transport handle; the transport must NOT
   surface a stable identity.** conn-id (a per-connection `Hash`) is the EPHEMERAL routing token for one
   live link (it changes across a reconnect). The STABLE federation identity is the node/hub's genesis
   `Hash` (D5), which is established IN-FOLD by the `hello`/`welcome` handshake (§2), not by the
   transport. The reducer maintains the `conn-id ↔ node-Hash` mapping in its fold state (learned at
   `welcome`); on a `ws/disconnect` it prunes the dead conn-id but keeps the node identity, and on
   reconnect it re-binds a fresh conn-id to the same node-Hash. Surfacing a "stable hub identity" at the
   transport layer would put federation identity semantics INTO the host — the exact coupling the
   host=plumbing constraint forbids. The host mints an opaque conn-id and stays ignorant of who is on the
   other end; the reducer learns identity from the handshake frame. So: keep the seam as-is (conn-id
   `Hash` only); the protocol layers stable identity on top.
3. **Reconnect/resume: transport auto-reconnect vs reducer-driven?** — **Reducer-driven.** On a
   `ws/disconnect` the reducer decides whether/when to re-`ws/dial` (its reconnect policy is fold logic:
   backoff, give-up, failover to another hub — all policy). The transport does NOT auto-reconnect: an
   auto-reconnecting transport would be making a policy decision (when/whether to retry, which is a
   federation concern) and would hide connection lifecycle from the reducer's log (breaking the
   durable-fold model — a reconnect must be a logged decision, not an invisible host action). So their
   "base dialer, no reconnect first" is not just acceptable, it is the CORRECT end state: reconnect never
   belongs in the transport. Resume semantics (does a reconnected link resume mid-protocol or re-handshake
   from `hello`?) is a reducer protocol decision (default: re-handshake — a fresh conn-id is a fresh link;
   the hub folds a new `hello` and re-establishes routing; §2). No transport resume state.

4. **Is the ws connection registry SHARED per-node or PER-SESSION?** (the F0→F1 runtime-wiring bridge —
   `v-agent-harness-host` found the `AsyncAgentHost` loop doesn't yet drain `WsControlOp` / hold a
   `LiveWsConnRegistry`, and is wiring it next.) — **ONE registry SHARED per node.** A node has one set of
   hub/peer connections; conn-ids are node-global unique `Hash`es (OS-entropy minted, `mint_conn_id`), so
   the registry is naturally node-scoped, drained on the single `AsyncAgentHost` loop (mirroring how it
   drains `lifecycle_rx` / reply-settles per turn). Why shared is correct AND safe:
   - **The federation (outpost) session is the sole ws actor in v0** — it is the `session` the listener/
     boot-dialer address `ws/connect`/`ws/frame`/`ws/disconnect` inbounds to, so it is the only session
     that ever LEARNS a conn-id. A per-session registry would buy nothing in v0 (only one session has
     conns) while blocking the natural multi-actor case (ARC 2: an MCP handler session that later holds
     its own connections needs to `ws/send` to a node-scoped conn-id).
   - **A conn-id is a capability token.** It is an unguessable 256-bit `Hash`; possessing one is the
     capability to `ws/send` to it, and Cedar gates the `ws/send` target on top. So a shared registry is
     not a leak: a session can only address a conn-id it was handed (via a `ws/connect` inbound or a
     `ws/dial` result), and the authorizer gates the send. Registry partitioning is not the security
     boundary — capability possession + Cedar are. This matches how `SessionId`/conn-id are already one
     unguessable-`Hash` namespace (the operator capstone).
   - So: wire ONE `LiveWsConnRegistry` per node into the loop; `WsSendExecutor`/`WsDialExecutor` resolve
     against it; the outpost session owns the hub link in v0, and the shared scope keeps multi-session
     federation (ARC 2) possible with zero rework. This is the "a v0 node dials its hub once at boot then
     the reducer routes" model `v-agent-harness-host` read correctly.

Net for the dialer + loop wiring: build `{hub_url, outpost_session_id}` → connect → mint conn-id →
register sink (into the ONE node-shared registry the loop drains) → surface `ws/connect`+`ws/frame`+
`ws/disconnect` to the session → drain `ws/send`/`ws/dial` against that registry. No reconnect, no stable
identity, no resume, no per-session registry — all of that lives in the reducer's fold. This is the exact
base dialer + loop wiring they described.

### Layer 1 — the wire codec: every frame is a binary AST (`cadenza-ast` value-form)

Operator directive, verbatim: *"send binary ASTs everywhere."* We do NOT invent a wire format. Every
federation frame is a **`cadenza-ast` value-form document** — the exact self-describing binary codec the
kernel↔reducer fold boundary already uses (`DESIGN-binary-ast-abi.md`: `apply(list<u8>)->list<u8>` over
`cadenza-ast` value-form bytes) and the same codec the event log and value-interchange paths use
(`spec/contracts/ast-encoding.md`, `deterministic-value-form.md`). One byte form, everywhere.

Concretely: a `ws/frame` inbound payload (after the host's conn-id length-prefix, `ws_socket.rs:219`) is
a `cadenza-ast` document. The federation reducer decodes it with the same `cadenza-ast` codec its own
`apply` boundary uses (a Cadenza guest via the runtime's `value-decode`/`value-encode`, B0/B3 of the
binary-AST-ABI design; a Rust guest via `cadenza_ast::codec` directly). An outbound `ws/send` payload is
likewise a `cadenza-ast` document the reducer encodes. **No bespoke framing, no JSON, no second codec.**

The federation FRAME SCHEMA is a small value-form union — a tagged s-expr, one variant per frame type
(§3). Because `cadenza-ast` is additive-by-symbol (a new node kind is a new symbol, no container-version
bump), the frame schema evolves by adding a variant/field with no wire-format break — the same evolution
story the fold boundary gets. This schema is defined in `cadenza-ast` terms as a shared contract both a
node and a hub agree on (analogue of binary-AST-ABI's B1 kernel-side schema, but between two reducers).

### Layer 2 — the federation SESSION

On each node, ONE session is the **federation session** (the "outpost session" the ws transport events
are already addressed to — `ws_connect_inbound(session, conn_id)` takes a `SessionId`, `ws_socket.rs:193`).
This session's reducer:

- owns the node's ws connections to peers/hub (by conn-id `Hash`),
- is the on-node endpoint the host surfaces `ws/connect`/`ws/frame`/`ws/disconnect` to and that emits
  `ws/dial`/`ws/send`,
- runs the federation protocol fold (Layer 3),
- bridges: it translates a LOCAL `Emit` addressed to a remote `SessionId` into an outbound `route` frame,
  and an inbound `route` frame into a LOCAL `Emit` to the target session. It is the sole federation
  actor; ordinary sessions stay oblivious — they just `Emit` to a `SessionId`, which the local
  `EmitExecutor` delivers if local or a `delivery-failure` bounces if not... which is where the
  federation session intercepts (§3.2, D6).

A node that is configured as a HUB runs the same binary with a hub-profile federation reducer (the
same-binary decision from the OUTPOST doc). Hub vs node is a reducer-profile + config difference, not a
separate codebase.

### Layer 3 — the protocol (§2–§4)

## 2. The handshake — how a node dials a hub and registers

A node's federation reducer, on startup (a genesis-config `Inbound`, or an operator `control` nudge),
emits `ws/dial(target = hub_url)`. On the settled conn-id (D4 default: the effect result):

1. **`hello`** (node → hub): the node sends a `hello` frame carrying its **node identity** (a `Hash` — a
   node's identity is a genesis `Hash`, uniform with `SessionId`/conn-id; D5), a **capability token**
   (scoped, expiring — see below), a protocol-version, and the node's advertised role/capabilities
   (what it serves — e.g. which effect families or named sessions it hosts).
2. The **hub reducer folds `hello`** and AUTHORIZES it — validates the capability token (in the fold; the
   host never sees the token semantics), checks the protocol-version, and decides admit/reject. This is
   pure reducer policy (+ optionally Cedar on a `federation/*` family if the hub wants effect-level
   authz). `agent-harness-kernel.md`'s framing (line 884): *"outposts have identities + scoped, expiring
   capabilities NOT standing access"* — the token is a scoped capability, not a standing credential.
3. **`welcome`** (hub → node) on admit: confirms the node's id, assigns/echoes the node's place in the
   hub topology, and returns any bootstrap directory state (e.g. the set of families/names the hub can
   route to). OR **`reject`** (hub → node) with a reason (bad token / version mismatch / unauthorized) —
   the node folds it and closes or retries per its own policy.
4. After `welcome`, the connection is an established federation link; `route`/`publish`/`resolve` frames
   flow (§3). A `ws/disconnect` (host-surfaced) tears the link down; both reducers prune their state
   (the node re-dials per its reconnect policy — reducer policy, host just re-runs `ws/dial`).

**The capability token (auth) — chosen default + the one genuine operator fork (D3).** The token is
validated ENTIRELY by the hub reducer (policy in wasm; the host is oblivious). For v0 the default is an
**operator-provisioned shared secret / static capability token** baked into each node's genesis config
and the hub's authorized-token set — enough to stand up a real federated hivemind. The PRINCIPLED
end-state is **JIT-scoped, expiring capability tokens** minted by a trust authority (relate to the
credential-broker pattern in memory `bedrock-cred-broker-transport-blocked-by-scp` and
`agent-harness-internal-resource-federation-context` — the privileged fetch-node/broker split), bound to
the node's `Hash` identity, with short TTLs and explicit scopes (which families/names a node may route).
The token FORMAT is a `cadenza-ast` value in the `hello` frame; the issuance/rotation MECHANISM is the
open fork — see D3. The build (F0–F4) does not block on it: v0 uses the shared-token path; the token
validation is one fold branch that swaps issuance later without touching the wire.

## 3. What federates

Three things federate, in priority order. The guiding cut: **federate the MESSAGE STREAM and the
DIRECTORY (naming/discovery); do NOT replicate event logs.** Sessions are the unit of durability and
they stay home; only what MUST cross to make a distributed hivemind work crosses.

### 3.1 Messages (the core — this is what "federate" primarily means)

The cross-session `Emit`→`Inbound{family:"message"}` stream goes cross-node. When session A on node N1
sends to session B whose `SessionId` resolves to node N2:

- N1's federation reducer emits a **`route`** frame (to the hub, or direct in mesh — §4): carries the
  target `SessionId` (a `Hash`), the message payload (opaque bytes — the original Emit payload, which
  itself may be a `cadenza-ast` value; federation does not interpret it), and the `reply_to` sender
  `SessionId` (for the bounce path).
- The hub routes the frame to the node hosting the target (directory lookup, §3.2/§4).
- N2's federation reducer receives the `route` frame and re-injects it as a LOCAL `Emit`
  (target = B, payload, reply_to = A) — which the local `EmitExecutor` delivers to B's inbox as the
  ordinary `Inbound{family:"message", reply_to: A}`. **B cannot tell the message came from another
  node** — location transparency, exactly as platform-conformance MD2 requires.
- **Delivery-failure crosses back symmetrically**: if the target is unknown/gone at N2 (or unroutable at
  the hub), a `delivery-failure` `route` frame bounces back to A's node → local `delivery-failure`
  Inbound to A, echoing the payload. Same triad, wire-transported.

This is the minimal, sufficient thing to federate: agents on different nodes message each other with the
kernel's existing semantics. Effect-handler-session deferral (a deferred effect delivered to a remote
handler as `effect-request/<family>`, the handler's `effect/reply` settling the caller's open
`EffectId`) rides the SAME `route` mechanism — it is just another cross-session message pair — so remote
effect handlers fall out for free once messages federate (platform-conformance MD2 names both).

### 3.2 Directory / discovery (the routing table)

Routing a `route` frame requires knowing WHICH node hosts a given `SessionId` (and resolving a NAME to a
`SessionId`). The hub is the **federated directory authority** (star, §4):

- Each node **`publish`**es its local sessions' names/ids to the hub (a `publish` frame: name → `Hash`
  bindings, or a group membership add/remove). This rides the session-directory model
  (`DESIGN-session-directory.md`): a name is a pointer (`resolve → latest Hash`) XOR a group (OR-set
  CRDT, `resolve_all → BTreeSet<Hash>`), backed by the GNS `name_store.rs` and the `store/*` effect
  verbs (`STORE_SET/RESOLVE/ADD/REMOVE/RESOLVE_ALL`, `effect.rs:118+`). The FEDERATED directory is that
  same name model, hub-hosted, keyed additionally by hosting-node.
- A node **`resolve`**s a remote name/id through the hub (a `resolve` frame → the hub answers with the
  `SessionId` + hosting node). Multicast to a federated GROUP = the hub resolves-all + fans out `route`
  frames (or the requesting reducer resolves-all then routes per member — the same freeze-then-loop
  discipline as `DESIGN-session-directory.md` D4, replay-safe).
- Because `SessionId = genesis Hash`, `resolve('session/alice') → Hash` IS the target session id (no
  extra indirection); the directory adds only the **node** coordinate (which link to route over).

The directory is thus the routing table AND the discovery surface — "who is out there" is "what names
the hub can resolve." This is where an external MCP-connected agent (ARC 2, out of scope here) would
appear once it registers via the outpost: as a name the federated directory resolves.

### 3.3 State / events — DEFERRED (reserved increment)

We do **NOT** federate the raw event log or shared KV in v0. Rationale:

- The gateway-first ruling in the OUTPOST doc (fork 2, operator-confirmed): the outpost is a pure
  router/gateway; it does not host remote sessions' logs. Each node owns its sessions' durability
  locally (the log-decouple design keeps a session's log host-cold locally; shipping it cross-node is a
  distributed-consistency problem, not a routing one).
- Messages + directory give a fully working hivemind (cross-node messaging + discovery) WITHOUT the hard
  problem of distributed log replication / cross-node consistency. Sessions are the unit; their logs stay
  home; only the inter-session message stream + naming crosses.
- **Reserved (F5):** cross-node state federation (log shipping / shared federated KV / event-stream
  replication) and its consistency model is a later increment. The session-directory OR-set CRDT already
  anticipates the multi-authority merge this would need. Explicitly out of v0 scope; noted so the wire
  schema reserves room (a `state`/`log-append` frame variant is a future additive symbol, §1).

## 4. Hub-side topology

**v0 = STAR.** A hub is a well-known node (a URL); outposts/nodes dial the hub and register (§2). The
hub is (a) the routing rendezvous — every `route` frame transits the hub, which forwards to the hosting
node — and (b) the federated directory authority (§3.2). Discovery = the hub's directory.

Why star for v0:

- It matches the operator's phrasing exactly: *"specify a hub and connect to it and start federating."*
- Single directory authority → no cross-peer CRDT merge needed for v0 (the hub is the one writer of the
  federated routing table; nodes publish, the hub is authoritative).
- Routing is hub-mediated store-and-forward: `route` to `SessionId` X → hub looks X up in the federated
  directory → forwards over X's hosting-node link. Unknown target → `delivery-failure` route-frame back
  to the sender (same bounce semantics as local, §3.1).
- A node can also BE a hub (same binary, hub reducer profile) — so a "hub" is not privileged
  infrastructure, just a node others dial. Hierarchies of hubs are a topology the star generalizes to.

**Reserved (F5) — mesh / multi-hub.** Direct node-to-node links (a node dials a peer node directly,
bypassing the hub for the data path after hub-mediated discovery) and multi-hub federation (hubs peer;
the directory becomes a CRDT merged across hubs — the OR-set model from `DESIGN-session-directory.md`
generalizes to this) are later increments. Star is the v0 that ships a working hivemind; mesh is the
optimization/scale story. Escalation D1 records the default.

## 5. Increments (each its own commit + gate; top-to-bottom, vertical-landable)

**F0 — ws-client dial-out primitive (`v-agent-harness-host`) — BOTH HALVES LANDED (2026-08-10).** Dial a
URL, ws client handshake, mint conn-id, register in `LiveWsConnRegistry`, pump frames both ways, surface
`ws/connect`/`ws/frame`/`ws/disconnect` to the outpost session. The whole host transport lane is delivered
on origin; F1–F4 (guest lane) build on the complete foundation below:

- **F0a — boot-dial (LANDED, `v-agent-harness-host`, `dial_hub` `7de3040d2`).** The daemon dials a
  *configured* hub at BOOT (a Rust fn the daemon calls from its config, symmetric to `WsListener::bind`).
  Sufficient for the entire v0 handshake + routing + directory (F2–F4): a node dials its configured hub
  once at startup, then all federation is frames over that link. No kernel dependency — the dial is host
  config, not a reducer decision.
- **F0b — the reducer-emittable `ws/dial` EFFECT (LANDED, `WsDialExecutor` `3731a3fb0`).** A reducer
  emits `ws/dial(hub_url)` → the executor mints + returns the conn-id hex synchronously (dispatched-with-
  result, D4) then spawns the dial; a connect failure surfaces as a `ws/disconnect` (so reducer-driven
  reconnect works). `WS_DIAL` family const added to `cdz-kernel/src/effect.rs` (v-agent-harness kernel
  territory) alongside `WS_SEND`/`WS_CONNECT`/`WS_DISCONNECT`, Cedar-gated on the URL target (`HostIn`).
  Built EAGERLY (ahead of the reconnect increment that first needs it), so **reducer-driven reconnect and
  dynamic/mesh dial are unblocked from the start of the guest lane** — the F1–F4 vertical does NOT have to
  defer reconnect to a later kernel-const coordination; the effect is already there.
- **The full transport surface a federation reducer folds against (all LANDED, byte-opaque, host=plumbing,
  Cedar-gated egress):** INBOUND events `ws/connect(conn-id)` / `ws/frame(conn-id, bytes)` /
  `ws/disconnect(conn-id)`; OUTBOUND effects `ws/dial(hub_url) → conn-id` / `ws/send(conn-id, frame)`. The
  wire payload is binary (the reducer owns the `cadenza-ast` codec). This is the complete foundation for
  F1–F4; the host lane is at-rest until F5 (state/log federation).

**F1 — the federation frame schema in `cadenza-ast` terms (guest lane + `v-syntax`).** Define the
value-form AST union for the frame types (`hello` / `welcome` / `reject` / `route` / `publish` /
`resolve` / `delivery-failure`) as a shared schema contract, reusing the `cadenza-ast` codec (no codec
change — coordinate with `v-syntax` who owns `cadenza-ast`; this REQUIRES no codec edit, only its use).
Gate: round-trip codec tests over each frame variant (encode → decode structurally equal), a node-side
encoder and a hub-side decoder agreeing on the wire (the analogue of binary-AST-ABI B1). No transport
touched; gate-neutral.

**F2 — the handshake reducer (guest lane; `v-metaprog`/guest owns policy).** The node dials its configured
hub at boot (F0a) → on the resulting `ws/connect` the federation reducer sends `hello` → hub folds
`hello` → `welcome`/`reject` (§2), with the v0 shared-token validation branch (D3). Gate: a
`platform-conformance` case (the in-process FIFO oracle, MD2) with two sessions — a node reducer and a hub
reducer — exchanging handshake frames, asserting `welcome` on a valid token and `reject` on a bad one;
then the same over the live boot-dial↔`WsListener` transport (a two-endpoint hermetic ws E2E). Uses F0a
only — no `WS_DIAL` kernel constant needed (the dial is host-config boot-dial, not a reducer decision).

**F3 — cross-node message routing (guest lane).** Local `Emit`-to-remote → `route` frame → hub forward →
remote local re-`Emit`; `delivery-failure` bounces cross back (§3.1). Gate: a `platform-conformance`
federated-message case — session A on "node N1" messages session B on "node N2" through a "hub" session;
assert B receives an `Inbound{family:"message", reply_to: A}` observationally identical to an in-process
Emit (the MD2 oracle equivalence), and that an unroutable target bounces a `delivery-failure` to A. Then
a live two-process `ws` E2E (two daemons + a hub) proving the same over the wire.

**F4 — the federated directory (guest lane, rides `DESIGN-session-directory.md`).** `publish` local names
to the hub; `resolve` a remote name/id; the hub as directory authority; group multicast via resolve-all
+ per-member `route` (§3.2). Gate: a case where N1 `publish`es `session/alice`, N2 `resolve`s it through
the hub, and a `route` to the resolved id reaches alice; a group `resolve_all` fans a message to all
members.

**F5 — RESERVED (deferred, not built in this arc).** Cross-node state/log federation (§3.3) and mesh /
multi-hub topology with a CRDT-merged directory (§4). Explicitly out of v0 scope; the wire schema (F1)
reserves additive room for a `state`/`log-append` frame variant and hub-peering frames so adding them
later is additive-by-symbol, no break.

(F0a + F0b are BOTH LANDED (`dial_hub` `7de3040d2`, `WsDialExecutor` `3731a3fb0`) — the whole host
transport lane is done. F1 is independent (pure schema). F2 depends on F1 (+ the landed F0). F3 depends
on F1+F2. F4 depends on F3. Because F0b landed eagerly, reducer-driven reconnect is available to the guest
lane from the start (no deferred kernel-const coordination). Each increment is independently green — F1
touches no transport; the live-ws E2Es in F2/F3 are hermetic per the nix rule.)

## 6. Open decisions (each with a chosen default; escalate only a genuine fork)

- **D1 — topology: star vs mesh (§4).** Default: **STAR** for v0 (hub = rendezvous + directory
  authority; a node can be a hub via reducer profile). Mesh / multi-hub with a CRDT directory is reserved
  (F5). No escalation — star ships a working hivemind and generalizes; mesh is a scale optimization the
  operator can prioritize later.
- **D2 — what federates: messages+directory vs full state (§3).** Default: federate the **message stream
  + the naming directory**, NOT raw event-log replication (gateway-first, sessions own their logs
  locally). State/log federation reserved (F5). No escalation — this is the minimal sufficient hivemind
  and matches the gateway-first ruling.
- **D3 — the capability-token issuance mechanism (§2). THE ONE GENUINE OPERATOR FORK.** The token FORMAT
  (a `cadenza-ast` value in `hello`, validated by the hub reducer) and the v0 path (operator-provisioned
  shared/static token) are settled and unblock the build. The open fork is the PRINCIPLED issuance/
  rotation: JIT-scoped expiring tokens minted by a trust authority, bound to the node `Hash`, with TTL +
  scope — and WHO is that authority (the hub itself? a separate broker, per the cred-broker pattern?).
  **Escalate to an `ask`** once F2 is close (not before — v0 shared-token unblocks F0–F4). The wire is
  identical either way; only the fold's token-validation branch differs.
- **D4 — how the dialed conn-id comes back (§Layer 0).** Default: `ws/dial` is
  **dispatched-with-result** (http/model shape) — the settled `EffectResult` carries the conn-id hex, a
  permanent Err on dial failure. Alternative (a fresh `ws/dial-ok` INBOUND event) is REJECTED: it adds an
  event family where the result channel already carries the id, and dispatched-with-result matches
  `mcp/call`. Coordinate with `v-agent-harness-host` on the WsControlOp/registry seam so a dialed
  connection registers identically to an accepted one (it should — same `LiveWsConnRegistry`). Recorded
  for the F0 implementer; no operator fork.
- **D5 — node identity (§2).** Default: a node's identity is a **genesis `Hash`** (uniform with
  `SessionId`/conn-id) — the node's federation-session genesis hash. Authentication of that identity is
  the capability token (D3), which binds to the `Hash`. Alternative (a separate keypair/PKI identity) is
  reserved with the JIT-token work (D3) if the trust authority needs asymmetric proof. No escalation for
  v0.
- **D6 — how the federation session intercepts a local Emit to a remote target (§Layer 2/§3.1).** A local
  session just `Emit`s to a `SessionId`; if that id is remote the local `EmitExecutor` would bounce a
  `delivery-failure`. Two ways the federation session gets the message to forward: (A) the sender first
  `resolve`s the target through the directory and, learning it is remote, `Emit`s to the FEDERATION
  session (which forwards) instead of directly; (B) the host/EmitExecutor, on an unknown-local target,
  routes to the federation session as a fallback before bouncing (a host seam — heavier, touches the
  emit path). **Default: (A)** — pure reducer/directory policy, NO host change, keeps the host oblivious
  to federation (the constraint). The directory resolve tells a sender "this id is remote → address the
  federation session"; a convention (a well-known local `session/federation` name) makes this uniform.
  (B) is reserved only if transparent forwarding without a resolve step becomes required — it is the one
  option that would add host logic, so it stays rejected unless the operator wants full transparency at
  the cost of a host seam.

## 7. Watch-outs (for the implementing vertical)

- **Host = plumbing ONLY — the whole protocol is a fold.** No federation logic in the host beyond
  `ws/dial` (dial + surface frames). If an increment wants the host to parse a frame, decide a route, or
  validate a token, that logic belongs in the reducer. This is the operator's twice-emphasized
  constraint; the reviewer gate is "does the host know what a `hello`/`route` frame IS" — it must not.
- **Binary ASTs everywhere — reuse `cadenza-ast`, invent no wire format.** Every frame is a value-form
  document decoded by the same codec the fold boundary + log use. Coordinate with `v-syntax` before ANY
  `cadenza-ast` touch (this design needs none — only its use). No JSON, no bespoke length-framing beyond
  the host's existing conn-id prefix (`ws_socket.rs:219`).
- **Location transparency is the correctness bar.** A federated message MUST be observationally identical
  to an in-process Emit (family `"message"`, `reply_to` preserved, `delivery-failure` bounce on
  unroutable) — that is precisely what `platform-conformance` (MD2) gates. If a federation route is
  distinguishable from a local Emit by the receiver, it is a bug.
- **`ws/connect` is an inbound EVENT, not the dial effect.** The dial-out effect is `ws/dial`. Reusing
  `ws/connect` (an event family, not in `ALL`) as an effect would break the event/effect distinction the
  ws seam is built on (`effect.rs:276-319`).
- **`SessionId = genesis Hash` is the whole namespace.** Do NOT introduce a node-qualified id type; the
  id stays a flat `Hash` and the directory adds the node coordinate as routing metadata, not as part of
  the identity. This is what keeps federation = messaging-with-a-hop.
- **Gate against the conformance oracle, then the wire.** Prove each protocol increment on the
  `platform-conformance` in-process FIFO drive FIRST (deterministic, no network), THEN add the hermetic
  live-ws E2E. The oracle catches semantic drift; the E2E catches transport wiring.

## 8. Coordination

- **`v-agent-harness-host`** — owns the ws transport (Layer 0). Confirm the `ws/dial` seam shape (D4:
  dispatched-with-result conn-id; dialed connection registers in the same `LiveWsConnRegistry` as an
  accepted one). This protocol is designed to the transport they are building; the `ws/dial` family +
  the no-`ws/connect`-collision are the asks.
- **`v-agent-harness`** — owns kernel session semantics (`Emit`, `Inbound`, `delivery-failure`,
  `SessionId=Hash`, the `store/*` and `effect/*` families). Federation reuses these verbatim; confirm no
  kernel edit is needed (the design is that none is — federation is fold + one host effect).
- **`v-syntax`** — owns `cadenza-ast`. F1's frame schema uses the codec; no codec change required.
- **guest lane (`v-metaprog`)** — authors the federation reducer (the policy: handshake, routing,
  directory, token validation). This is where Layers 2–3 live.
- **`v-platform-conformance`** — the F2–F4 gates are `platform-conformance` cases; coordinate the
  federated-message case shape with them (they own the `(platform-case …)` FIFO drive + the MD2 binding
  that names this work).

## 9. Relationship to the MCP-outpost arc (ARC 2 — out of scope here)

The operator's second queued arc (memory `queued-hub-outpost-federation-and-mcp-outpost-arcs`, ARC 2)
is the outpost exposing an MCP SERVER surface so external agents (Claude Code, Codex) connect via MCP and
join the hivemind. That is a SEPARATE, already-framed piece: the outpost-as-userspace-effect-handler
(memory `design-the-outpost-host-websocket-federation-node`, the reframe — the host knows nothing about
MCP; MCP is a userspace handler session folding JSON-RPC frames). It DEPENDS on this federation protocol
existing (an external MCP agent becomes a name the federated directory resolves, §3.2) but is not part of
this doc. Sequence: land hub-federation (F0–F4) → the MCP-outpost surface rides it as a subsequent arc.
