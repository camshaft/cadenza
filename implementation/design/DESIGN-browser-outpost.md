# The browser outpost — the browser is another wasm-running reducer node on the platform

**Status:** design/scoping only — nothing landed. Written 2026-08-30 by the `design-browser-outpost`
fleet agent on the operator's spark (relayed via the concierge), verbatim: *"Can you spin up the
platform agent to extend the docs to be able to include a browser outpost? Basically I want the fleet
to be able to design and build websites that are able to be reducers themselves. So the browser just
acts as another outpost that we can ship code to cause it runs wasm. And then the browser application
can communicate with the whole platform so we could build highly interactive and deeply connected
applications that you can communicate with agents directly to drive it even."* The operator iterates
asynchronously; this design makes the engineering calls autonomously from the stated direction, records
each open fork with a chosen default, and escalates only a genuine unresolvable fork as an `ask`.

This doc EXTENDS the platform model. It is a companion to two existing docs it must not fork:
- `design/cadenza-platform.md` — the canonical kernel/reducer/session model. §3 "Everything is a wasm
  module" (L255) and the WASI edge-reducer boundary (L248–271) are the paragraphs this design specializes.
- `implementation/design/DESIGN-hub-federation-protocol.md` — the hub/outpost federation protocol (ws
  transport, `cadenza-ast` wire codec, `hello`/`welcome` handshake, star topology, `SessionId = Hash`).
  A browser outpost is a federation node; it reuses that protocol wholesale and adds no parallel wire.

It hands a build plan to a `vertical` owner and coordinates `v-platform` (the reducer kernel + platform
docs), the federation lane (the ws/`cadenza-ast` seams it rides), and the browser-boundary crates
(`cdz-wasm`, `rcdzc-wasm`).

---

## 0. The one architectural claim everything follows from

**A browser tab is an EDGE REDUCER whose "outside world" is the browser instead of native WASI — and
since the browser runs wasm and the platform's federation namespace is node-agnostic, the browser needs
no new platform concept: it is another federation NODE that (a) instantiates content-addressed reducer
components with a browser-native `ProgramStore` and (b) reaches the DOM/user through a browser host that
mirrors the native WASI edge exactly as `ws/*` mirrors the socket.**

Three facts the platform already gives us make the browser "just another outpost," not a new subsystem:

1. **Everything is a wasm module** (`cadenza-platform.md` §3, L255). A reducer's identity is its
   `ProgramHash` = the content hash of its wasm component (`cdz-platform/wit/world.wit` L42–45). The
   browser is a first-class wasm engine. So "ship code to the browser" = ship the SAME content-addressed
   component bytes the platform already publishes; the browser instantiates them. No bespoke build target.
2. **The reducer contract is host-agnostic.** `interface guest` (`on-message`/`on-response`/
   `on-notification` → `step`) is a pure fold over events returning requests + an outcome; it names no
   host. A browser reducer satisfies the identical WIT contract. What changes is only which imports
   (`state`, `blobs`, and a new `dom`) the host backs, exactly as the platform already anticipates for a
   WASI edge reducer.
3. **Federation is location-transparent** (`DESIGN-hub-federation-protocol.md` §0). `SessionId = Hash`
   is a flat, node-agnostic namespace; one session messages another with the same `Emit`→peer-`Inbound
   {family:"message"}` semantics whether in-process or over the ws wire. A browser tab that dials the hub
   and registers a `SessionId` is therefore reachable by any other node — including an **agent** — with
   zero new routing. **The agent-drive path the operator wants is not a feature; it is the federation
   default.** An agent drives the app by emitting a message to the tab's `SessionId`; the browser reducer
   handles it identically to a user click.

**The consequence — what is genuinely NEW (and it is small):** the browser cannot run the native
`WasmProgramStore` (`cdz-platform/src/host.rs` is wasmtime, `#[cfg(feature = "host")]`, native-only).
Two seams must be re-implemented against browser APIs, and NOTHING else in the platform changes:
- a **browser `ProgramStore`** that instantiates a WIT component in the tab's own wasm engine, and
- a **browser edge host** backing `state`/`blobs`/`dom` with IndexedDB/Cache/DOM, plus the browser's
  native `WebSocket` standing in for the federation `ws/dial`/`ws/send` primitive.

Everything above the transport — the handshake, routing, what federates, the reducer fold — is byte-for-byte
the federation protocol and the WIT contract already specified. The browser is a new HOST, not a new model.

---

## 1. The model — where the browser sits

```
  ┌──────────────────────── browser tab (a federation NODE) ────────────────────────┐
  │  Layer 3  APP REDUCER      Cadenza reducer component: on-message/on-response →step │  (guest wasm)
  │  Layer 2  BROWSER HOST     ProgramStore(browser) + edge host (state/blobs/dom)     │  (JS/wasm glue)
  │  Layer 1  WIRE CODEC       cadenza-ast value-form frames (reuse, compiled to wasm) │  (cadenza-ast)
  │  Layer 0  WS TRANSPORT     browser-native WebSocket ≙ ws/dial · ws/send · ws/frame │  (browser API)
  └───────────────────────────────────────────┬─────────────────────────────────────┘
                                               │ dials, hello/welcome (§2 federation)
                                               ▼
                                        ┌──────────────┐
                                        │     HUB      │  routes SessionId→SessionId (agents, other nodes)
                                        └──────────────┘
```

The right three layers (0, 1, 3) are IDENTICAL to a native federation node — the browser only substitutes
the transport implementation (native WebSocket for the host `ws/dial` plumbing) and the host bindings.
Layer 2 is the browser-specific work.

### Layer 0 — transport: the browser's WebSocket IS the `ws/*` seam
A native node dials the hub via the host `ws/dial` outbound effect and receives `ws/frame`/`ws/disconnect`
inbound events (`DESIGN-hub-federation-protocol.md` §1 Layer 0). In the browser there is no host to add a
primitive to — the browser natively provides `new WebSocket(hubUrl)`, `.send(frame)`, `onmessage`,
`onclose`. The browser host maps these one-to-one onto the reducer-facing `ws/dial`/`ws/send`/`ws/frame`/
`ws/disconnect` seam so that **the federation reducer fold is bit-identical whether it runs native or in
a tab**. The only asymmetry: a browser tab can only DIAL OUT (browsers cannot accept inbound ws), which
matches the star topology exactly — outposts dial the hub, the hub never dials an outpost.

### Layer 1 — wire codec: reuse `cadenza-ast`, compiled to wasm
Every frame is a `cadenza-ast` value-form binary document (operator directive "binary AST is THE
data-exchange format, no exceptions"; `DESIGN-binary-ast-abi.md`, `spec/contracts/ast-encoding.md`). The
browser must encode/decode the SAME frames — so it reuses the `cadenza-ast` codec compiled to wasm (the
`cdz-wasm` / `rcdzc-wasm` precedent), never a JSON wire. No new schema; the frame union from the
federation doc §1 Layer 1 is unchanged.

### Layer 3 — the app reducer: the WIT `guest` contract, unchanged
The in-tab app is a Cadenza reducer emitted as a WIT component (`rcdzc` guest emission,
`crates/rcdzc/src/backend/wasm/DESIGN-reducer-guest-emission.md`). It exports `on-message`/`on-response`/
`on-notification` returning a `step { requests, outcome }`. **The app IS the reducer:** its state is the
fold state, user events and agent commands both arrive as `message`s, and everything it does to the world
— render, fetch, store, message a peer — is a `request` in the returned `step`. This is the Elm
architecture expressed in the platform's existing reducer contract; we invent no new app framework.

### Layer 2 — the browser host (the genuinely new code)
Two seams, both browser-backed:

**(a) `ProgramStore` for the browser.** `trait ProgramStore { spawn(program, ctx) -> Reducer }`
(`cdz-platform/src/program.rs` L45) is the instantiation seam. The native impl (`WasmProgramStore`,
wasmtime) is native-only. The browser impl instantiates a WIT component in the tab's wasm engine. Because
browsers do not yet ship the Component Model natively, the default is **jco** (the WebAssembly
Component Model JS tooling): a component is transpiled to an ES module + core wasm, loaded dynamically by
`ProgramHash`. Hot-ship = fetch a new hash's transpiled module and re-instantiate — no page reload,
mirroring "a reducer evolves by publishing a new module hash, never a host redeploy" (`world.wit` L5–6).

**(b) the browser edge host — backing the reducer's imports with browser APIs.** The `reducer-world`
imports `state`, `blobs`, `identity`, `run` (`world.wit` L191–211). In a tab:
- `state` → IndexedDB (durable per-origin fold state, survives reload).
- `blobs` → Cache API / IndexedDB (content-addressed blob store, keyed by hash).
- `identity` / `run` → the tab's `SessionId` (a genesis `Hash`) + the compiler-as-wasm `run` path.
- **NEW `dom` interface (the browser's WASI-edge analogue)** — see §2.

---

## 2. The `dom` interface — the browser's edge, in the reducer's own language

Native edge reducers reach the outside world through WASI (`cadenza-platform.md` L262–271: capability-
oriented, mapping onto §5 resource-scoped capabilities). The browser's outside world is the DOM and the
user. So the browser outpost adds ONE new WIT interface, `dom`, that is the browser's WASI-edge — same
capability-scoped, host-supplied discipline, different surface:

- **Outbound (a `request`/effect the reducer emits in its `step`):** `render(patch)` — a `cadenza-ast`
  value-form describing a virtual-DOM patch (declarative UI diff). The host applies it to the tab. Keeping
  render a *patch value* (not imperative DOM calls) preserves determinism: the reducer is still a pure
  fold from events to a `step`, and the rendered UI is a projection of fold state. Optional siblings:
  `title`, `navigate`, `storage-write` (already covered by `state`).
- **Inbound (a `message`/event the host delivers):** user interactions — `click`, `input`, `submit`,
  `key`, `route-change` — arrive as `message`s with a `dom-event` contract. To the reducer these are
  ordinary events; a click and an agent command are the same `on-message` call, differing only by
  contract/sender. This is what makes the app "agent-drivable for free" (§0 fact 3).

`dom` is capability-scoped exactly like WASI: a reducer that never imports `dom` cannot touch the page.
The virtual-DOM patch schema is a `cadenza-ast` union (consistent with the wire codec — one encoding
everywhere), so the same value can be logged, replayed, and diff-tested by the platform-conformance suite.

---

## 3. The agent-drive path (why this is the whole point)

The operator wants apps "you can communicate with agents directly to drive." Because the tab is a
federation node with a `SessionId`, an agent (itself a reducer/node) drives it by emitting a message to
that `SessionId` over federation — the hub routes it, the browser host delivers it as an `on-message`
call, the app reducer folds it into new state + a `render` patch. No special "remote control" channel:

```
  agent reducer ──Emit{target: tab.SessionId, payload: cadenza-ast command}──▶ hub ──▶ tab reducer
       ▲                                                                                    │
       └────────────────── Emit{target: agent.SessionId, reply payload} ◀───── step.requests┘
```

Bidirectional by construction (the reply is just another `Emit`). An app can therefore expose a "drive
contract" (e.g. a set of message shapes an agent may send) that is nothing more than the message variants
its `on-message` handles — the same surface a human's clicks map onto. Authorization is the platform's
existing Cedar policy on the federation route (`cadenza-platform.md` §5), so "which agents may drive this
tab" is a policy question, not new mechanism.

---

## 4. Increments (top-to-bottom, the way a vertical lands them)

Each increment is independently gate-able and delivers a running artifact.

- **P0 — local reducer app (no network).** A Cadenza reducer component runs in a tab via the browser
  `ProgramStore` (jco), folds `dom-event` messages, emits `render` patches; a demo counter/todo app
  renders and responds to clicks. Proves: browser `ProgramStore`, the `dom` interface, `state`→IndexedDB.
  *Gate:* the app reducer passes the in-process platform-conformance FIFO drive identically to its
  in-browser run (same events → same fold state), plus a headless-browser smoke (reuse the browser driver
  gap noted in `DESIGN-browser-compound-property-test-driver.md`).
- **P1 — federate.** The tab dials the hub over native WebSocket, runs the `hello`/`welcome` handshake,
  registers its `SessionId`, and exchanges `cadenza-ast` frames. Proves: Layer 0/1 in the browser, the
  federation reducer fold running unchanged in a tab. *Gate:* a federated `Emit` tab→node is
  observationally identical to an in-process Emit under platform-conformance (the ready-made oracle from
  the federation doc §0).
- **P2 — agent-drive.** An agent node emits messages to the tab's `SessionId`; the app folds them and
  replies. Proves §3 end-to-end. *Gate:* a scripted agent drives the P0 demo app remotely; the resulting
  fold state matches the same command sequence applied locally.
- **P3 — hot-ship.** Publishing a new `ProgramHash` re-instantiates the app in-place (no reload),
  preserving fold state from `state`. Proves the "evolve by new hash, never redeploy" principle in-browser.

Later/out-of-scope-for-v1: server-side rendering / hydration, multi-tab shared session, offline-first sync
conflict resolution, a component-model-native browser path when browsers ship it (drops the jco transpile).

---

## 5. Seams & file anchors

- Extend `design/cadenza-platform.md` §3 (L248–271): add a "The browser as an edge reducer" subsection
  stating the tab is an edge node whose WASI-edge is the `dom` interface + browser WebSocket.
- `implementation/design/DESIGN-hub-federation-protocol.md` §1 Layer 0/1, §2 handshake — the browser
  reuses these; add a note (or a §ref here) that the browser substitutes native WebSocket for `ws/dial`.
- `cdz-platform/wit/world.wit` — add the `dom` interface + a `browser-reducer-world` (reducer-world +
  `dom`). No change to `interface guest`.
- `cdz-platform/src/program.rs` `ProgramStore::spawn` — the seam a browser `ProgramStore` implements
  (new crate, e.g. `cdz-platform-browser`, `#[cfg(target_arch = "wasm32")]`, NOT `#[cfg(feature=host)]`).
- `crates/cdz-wasm` / `crates/rcdzc-wasm` — reuse target for JS↔wasm marshaling + the `cadenza-ast` codec
  compiled to wasm.
- `crates/rcdzc/src/backend/wasm/DESIGN-reducer-guest-emission.md` — the app reducer is emitted here,
  unchanged; the browser just instantiates the product.

## 6. Open decisions (chosen default in **bold**)

1. **In-browser component engine:** **jco transpile at build time** (component → ES module + core wasm),
   loaded by hash. Alternative: wait for native browser Component Model (defers the whole capability).
   Default keeps us shippable today; swap to native when browsers ship it (P-later).
2. **Which host does the browser federate with?** **The federation protocol (Model B: hub + ws +
   cadenza-ast), satisfying the Model A reducer WIT contract for the in-tab reducer.** I.e. reuse the
   federation arc for comms and the `cdz-platform` WIT for the contract — this is the reconciliation of
   the two vocabularies. Escalate ONLY if the operator wants the browser to speak directly to
   `cdz-platform` primitives (`Request`/`deliver`) instead of federation `Emit`/`Inbound`.
3. **Render model:** **declarative virtual-DOM patch as a `cadenza-ast` value** (Elm architecture in the
   reducer contract). Alternative: a thin JS view layer subscribing to fold state (less deterministic,
   forks the value story). Default keeps one encoding everywhere + replayable UI.
4. **State durability:** **IndexedDB-backed `state`, per-origin, survives reload.** Alternative:
   in-memory only (simpler P0, no persistence) — acceptable for the P0 demo, upgrade in P3.

## 7. Watch-outs

- **`host.rs` is wasmtime + `#[cfg(feature=host)]` — it cannot run in a browser as-is.** The browser
  `ProgramStore` is a distinct impl; do not try to `wasm32`-compile the native host. This is the single
  biggest fork point (§0).
- **Two vocabularies.** The federation doc speaks `Emit`/`Inbound`/`SessionId`/`ws/*` (the outpost/harness
  host); `cdz-platform` speaks `Request`/`Message`/`deliver`/`ProgramStore` (in-repo WIT kernel). This
  design pins: comms = federation vocabulary, contract = `cdz-platform` WIT (decision 6.2). A vertical
  must not blur them.
- **Browser tabs dial out only.** Star topology is a hard fit, not a limitation — but any design that
  assumed a tab could accept inbound ws is wrong.
- **Determinism.** Keep `render` a *value* (patch) and route ALL user input as `message`s, so the tab
  reducer stays a pure fold the platform-conformance oracle can replay. An imperative DOM escape hatch
  would break the replay gate — resist it.
- **Missing browser driver.** The headless-browser test path is a known gap
  (`DESIGN-browser-compound-property-test-driver.md`) — P0's gate depends on closing it; coordinate.

## 8. Relationship to existing arcs

- **To `DESIGN-hub-federation-protocol.md`:** the browser outpost is a CONSUMER of the federation
  protocol — it adds no wire, no handshake, no topology; it substitutes the transport implementation
  (native WebSocket) and adds a host. If federation lands first, the browser is "federation, minus the
  native ws host, plus a `dom` edge."
- **To `cadenza-platform.md`:** this specializes the "edge reducer" / "everything is a wasm module"
  sections; it does not change the kernel, the session model, or `interface guest`.
- **To `cdz-wasm` / `rcdzc-wasm`:** those prove Cadenza wasm + the `cadenza-ast` codec run in a browser;
  the browser outpost builds the reducer-host layer on top of that boundary.
