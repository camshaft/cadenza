# The browser outpost — the browser is another wasm-running reducer node on the platform

**Status:** design/scoping only — nothing landed; NO build vertical yet (operator holds the build
greenlight until this design is complete). First landed 2026-08-30 (PR #6223); **expanded to a
build-ready spec 2026-08-30** on the operator's decision (option C, via concierge): *"I don't want to
build the browser platform just yet. Let's just get the design fully spaced out."* plus the operator's
component-decomposition input: *"Maybe we need to split up the cdz wasm bundle we ship to the browser
into smaller pieces? Right now I think we are shipping the whole compiler and syntax… What would be
better is to have smaller components that communicate with binary AST and then the wasm could be a bit
more focused."* The operator iterates asynchronously and may join the design window; this design makes
each engineering call autonomously from the stated direction, records every open fork with a chosen
default + alternative, and escalates only a genuine unresolvable fork as an `ask`.

This doc EXTENDS the platform model. It is a companion to two existing docs it must not fork:
- `design/cadenza-platform.md` — the canonical kernel/reducer/session model. §3 "Everything is a wasm
  module" (L255) and the WASI edge-reducer boundary (L248–271) are the paragraphs this design specializes.
- `implementation/design/DESIGN-hub-federation-protocol.md` — the hub/outpost federation protocol (ws
  transport, `cadenza-ast` wire codec, `hello`/`welcome` handshake, star topology, `SessionId = Hash`).
  A browser outpost is a federation node; it reuses that protocol wholesale and adds no parallel wire.

It hands a build plan to a `vertical` owner (once greenlit) and coordinates `v-platform` (the reducer
kernel + platform docs), the federation lane (the ws/`cadenza-ast` seams it rides), and the browser-
boundary crates (`cdz-wasm`, `rcdzc-wasm`).

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

**Corollary (the operator's component-decomposition direction): the browser RUNS, it does not COMPILE.**
Compilation (`rcdzc` + `cadenza-syntax`) is a build-node concern; what reaches a tab is an already-
compiled, content-addressed reducer component. So the browser ships a SMALL set of FOCUSED components that
talk binary AST at their seams — never the monolithic "whole compiler + syntax" `cdz-wasm` bundle. §2
makes this the load-bearing "what ships to the browser" decision.

**The consequence — what is genuinely NEW (and it is small):** the browser cannot run the native
`WasmProgramStore` (`cdz-platform/src/host.rs` is wasmtime, `#[cfg(feature = "host")]`, native-only).
Two seams must be re-implemented against browser APIs, and NOTHING else in the platform changes:
- a **browser `ProgramStore`** (§4) that instantiates a WIT component in the tab's own wasm engine, and
- a **browser edge host** (§5) backing `state`/`blobs`/`dom` with IndexedDB/Cache/DOM, plus the browser's
  native `WebSocket` standing in for the federation `ws/dial`/`ws/send` primitive (§6).

Everything above the transport — the handshake, routing, what federates, the reducer fold — is byte-for-byte
the federation protocol and the WIT contract already specified. The browser is a new HOST, not a new model.

---

## 1. The model — where the browser sits

```
  ┌──────────────────────── browser tab (a federation NODE) ────────────────────────┐
  │  Layer 3  APP REDUCER      Cadenza reducer component: on-message/on-response →step │  (guest wasm)
  │  Layer 2  BROWSER HOST     ProgramStore(browser) + edge host (state/blobs/dom)     │  (JS/wasm glue)
  │  Layer 1  WIRE CODEC       cadenza-ast value-form frames (focused codec component) │  (cadenza-ast)
  │  Layer 0  WS TRANSPORT     browser-native WebSocket ≙ ws/dial · ws/send · ws/frame │  (browser API)
  └───────────────────────────────────────────┬─────────────────────────────────────┘
                                               │ dials, hello/welcome (§6 / federation §2)
                                               ▼
                                        ┌──────────────┐
                                        │     HUB      │  routes SessionId→SessionId (agents, other nodes)
                                        └──────────────┘
```

The right three layers (0, 1, 3) are IDENTICAL to a native federation node — the browser only substitutes
the transport implementation (native WebSocket for the host `ws/dial` plumbing) and the host bindings.
Layer 2 is the browser-specific work; Layer 1 is a focused reused component (§2).

---

## 2. What ships to the browser — focused components over binary AST (operator's core axis)

The operator's directive: do NOT ship a monolithic `cdz-wasm` bundle (whole compiler + syntax + more).
Ship SMALLER, FOCUSED wasm components that communicate via binary AST — the platform's exchange format
("binary-AST is THE data-exchange format, no exceptions"). This section is the decision.

### 2.1 The browser RUNS, it does not COMPILE (the decision that shrinks everything)

A browser outpost's job at runtime is: **instantiate a pre-compiled reducer, fold events, render values.**
None of that requires compiling Cadenza source. Compilation stays on build nodes (the fleet, a build
outpost), which publish content-addressed reducer components to the blob store; the tab fetches + runs
them. So the tab does NOT ship `rcdzc` (the compiler) or `cadenza-syntax` (the surface parser) — the two
biggest chunks of the current bundle. It ships only a small runtime + the app.

The one exception is a **playground/live-coding app** that compiles Cadenza in the tab. That is an app
that OPTS IN to a heavy `compiler` component (§2.2) as one of its dependencies — the default browser
outpost never carries it. This keeps the compiler out of the common path and off the codegen surface
(§2.4).

### 2.2 Component inventory — what a tab actually needs at runtime

Each is a content-addressed wasm component, composed via the Component Model (jco, §4), talking binary AST
at its seams. Sized smallest-shared-first:

| Component            | Ships when            | Role                                                                 | Seam (binary AST) |
|----------------------|-----------------------|----------------------------------------------------------------------|-------------------|
| **`ast-codec`**      | always (shared, cached)| encode/decode `cadenza-ast` value-form documents (wire frames, dom patches, values) | is the codec |
| **`reducer-runtime`**| always (shared, cached)| the browser `ProgramStore` + edge-host glue that drives a reducer's `on-message`/`step` | binary-AST payloads |
| **`value-render`**   | UI apps               | project a `cadenza-ast` value (the vDOM patch, §3) into a DOM patch the host JS applies | patch value in |
| **app `reducer`**    | per app (hot-shipped) | the application itself — a WIT `guest` component (Layer 3)            | events in / requests out |
| **`compiler`**       | OPT-IN (playground only)| `rcdzc` + `cadenza-syntax` as a heavy component; compiles source→reducer in-tab | source in / component out |

`ast-codec` + `reducer-runtime` (+ `value-render` for UI apps) are the shared, cacheable "browser outpost
runtime" — shipped once per origin, reused across every app. Only the app `reducer` is per-app and hot-
shipped. The `compiler` component is never in the default path.

### 2.3 Component boundaries + their binary-AST seams

- **codec ↔ everything:** every cross-component payload is a `cadenza-ast` value-form document. No JSON,
  no bespoke framing (matches the wire codec, `DESIGN-hub-federation-protocol.md` §1 Layer 1).
- **reducer ↔ host (`reducer-runtime`):** the WIT `guest` interface is typed, but the per-contract
  `payload` and the `dom` patch/event are binary-AST bytes — so the host never parses app schemas.
- **reducer ↔ reducer (federation):** binary-AST frames over the WebSocket (§6).
- **render (`value-render`):** input is a binary-AST vDOM-patch value (§3); output is a DOM mutation the
  thin host JS applies. Keeping the patch a VALUE (not imperative calls) is what preserves determinism +
  replayability.

The composition rule: components are wired by the Component Model (imports/exports), and any data that
crosses a component boundary is binary AST. This is the "focused components communicating via binary AST"
the operator asked for, expressed in the platform's existing exchange format.

### 2.4 Wins — load time + codegen surface

- **Load/instantiate:** ship the small shared runtime once (cached), then only the per-app reducer (small).
  Hot-ship a single reducer component, not the world (§4). A tab no longer pays to download/instantiate the
  whole compiler to run a counter app.
- **Codegen surface (bonus the operator flagged):** the compiler's parse/render path is exactly the
  wasm-codegen surface implicated in the current guide-examples OOB (an LLVM-22 wasm32 miscompile of that
  path). A browser outpost that ships a focused runtime/render component rather than `rcdzc` **does not drag
  that surface into the tab at all** — the miscompile-prone code is simply absent from what runs in the
  browser. This is a correctness argument for the split, not only a size one.

---

## 3. The `dom` interface — the browser's edge, fully specified

Native edge reducers reach the outside world through WASI (`cadenza-platform.md` L262–271: capability-
oriented). The browser's outside world is the DOM and the user, so the browser outpost adds ONE new WIT
interface, `dom`, that is the browser's WASI-edge: capability-scoped and host-supplied like WASI, DOM-
shaped instead of POSIX-shaped. A reducer that does not import `dom` cannot touch the page.

### 3.1 Outbound — `render(patch)` (a `request`/effect in the reducer's `step`)

The reducer emits a `render` request carrying a **vDOM patch as a `cadenza-ast` value-form document**.
Keeping the patch a *value* (not imperative DOM calls) preserves the pure-fold property: the UI is a
deterministic projection of fold state, so it is loggable, replayable, and diff-testable by the platform-
conformance suite. Proposed patch value shape (a `cadenza-ast` tagged union; final schema pinned with
`v-syntax`/`v-platform`):

```
patch    := replace(node) | update(list<edit>)          ; replace whole tree, or apply keyed edits
node      := element(tag, attrs, list<node>) | text(str) | keyed(key, node)
edit      := set-attr(path, name, value) | remove-attr(path, name)
           | insert(path, index, node) | remove(path, index) | replace-node(path, node)
attrs    := list<(name, value)>                          ; value carries a small tagged scalar/handler-ref
path     := list<u32>                                    ; child-index path from root
```

`attrs` may carry a **handler reference** (not a JS closure): `on(event-name, contract)` binds a DOM event
to a `dom-event` contract the reducer will receive (§3.2). The host wires the listener; the reducer stays
pure. This is the Elm architecture expressed in the reducer contract — the reducer is the app's `update`,
the patch is `view`'s diff.

Sibling outbound requests under `dom` (all optional, capability-gated): `title(str)`, `navigate(url)`,
`focus(path)`, `set-timer(id, ms)` (fires a `dom-event` tick — but note determinism caveat in §11).

### 3.2 Inbound — user + agent events arrive as `message`s (`on-message`)

User interactions are delivered to the reducer as ordinary `message`s with a `dom-event` contract, payload
= a `cadenza-ast` value:

```
dom-event := click(handler-ref, target-path)
           | input(handler-ref, target-path, value)      ; text/checkbox/select value
           | submit(handler-ref, form-fields)
           | key(handler-ref, key, modifiers)
           | route-change(url)
           | timer(id)
```

To the reducer a click and an agent command are the SAME `on-message` call — they differ only by
contract/sender (a `dom-event` contract vs an app-defined drive contract, §7). That symmetry is what makes
the app "agent-drivable for free."

### 3.3 Capability scoping

`dom` is added as a WIT interface + a `browser-reducer-world` = `reducer-world` + `dom`. It is capability-
scoped exactly like WASI: instantiating a reducer without granting `dom` yields an app that cannot render
or receive DOM events (useful for headless/logic-only reducers in a tab). Authorization for the DOM
capability follows the platform's §5 resource-scoped capability model.

---

## 4. The browser `ProgramStore` — instantiation, jco, hot-ship

`trait ProgramStore { async fn spawn(&self, program: ProgramHash, ctx: SpawnContext) -> Option<Reducer> }`
(`cdz-platform/src/program.rs` L45) is the instantiation seam. The native impl (`WasmProgramStore`,
wasmtime) is native-only. The browser impl lives in a new crate (proposed `cdz-platform-browser`,
`#[cfg(target_arch = "wasm32")]`, NOT `#[cfg(feature = "host")]`) and is the `reducer-runtime` component.

### 4.1 Instantiation via jco (chosen default)

Browsers do not yet ship the Component Model natively, so a WIT component must be transpiled to something a
browser wasm engine can run. Default: **jco** (the WebAssembly Component Model JS tooling) transpiles a
component to an ES module + one or more core-wasm modules. Flow:
1. `spawn(program_hash, ctx)` looks up the component bytes by `ProgramHash` in the browser blob store (§5).
2. Bytes were transpiled by jco at build time (build-node concern) → an ES module keyed by hash, fetched
   dynamically (`import()`), OR transpiled in-tab by a jco-runtime component (heavier; default is build-time).
3. The transpiled module's imports are satisfied by the edge host (§5) — `state`, `blobs`, `identity`,
   `run`, and `dom` (§3) — supplied as JS/wasm import bindings.
4. Instantiation yields an object exporting `on-message`/`on-response`/`on-notification`; `reducer-runtime`
   wraps it in the `Reducer` shape the fold loop drives.

### 4.2 Hot-ship — re-instantiate on a new hash, no reload

"A reducer evolves by publishing a new module hash, never a host redeploy" (`world.wit` L5–6) maps directly
to the tab: when the platform publishes a new `ProgramHash` for the app, `reducer-runtime` fetches the new
transpiled module, instantiates it, hands the OLD reducer's fold `state` (§5, durable in IndexedDB) to the
NEW instance, swaps the live reducer, and re-renders. No page reload. The `SessionId` (identity) is
preserved across the swap so federation peers see continuity. State-shape migration across incompatible
hashes is an open question (§10).

### 4.3 What the ProgramStore does NOT do

It does not compile. It only instantiates + drives an already-compiled component (§2.1). The `compiler`
component (opt-in, playground) would be spawned as just another component the app depends on — the
ProgramStore treats it identically to any reducer.

---

## 5. The browser edge host — backing the WIT imports with browser APIs

The `reducer-world` imports `state`, `blobs`, `identity`, `run` (`world.wit` L191–211). The browser edge
host backs each with a browser API, mirroring what a native WASI host supplies:

- **`state` → IndexedDB.** The reducer's fold state, durable per-origin, survives reload. The state
  interface's get/set map onto an IndexedDB object store keyed by the reducer's `SessionId`. This is what
  lets a tab resume its fold after a refresh and what hot-ship (§4.2) carries across a swap.
- **`blobs` → Cache API / IndexedDB.** The content-addressed blob store, keyed by `Hash`. Component bytes
  (including hot-shipped reducers) and any app blobs live here; a fetch-miss falls through to a federated
  fetch from the hub (a blob request over §6).
- **`identity` → the tab's `SessionId`.** A genesis `Hash`, minted once per tab (persisted in `state` so
  it is stable across reloads), node-agnostic — this is what makes the tab addressable by agents (§7).
- **`run` → the pure-eval path.** For a reducer that needs to evaluate a pure Cadenza value (not compile
  source), `run` maps onto the existing `rcdzc-wasm` pure-eval entry (a focused capability, not the whole
  compiler). Most UI apps won't import `run`.
- **`dom` → §3**, backed by the thin host JS + `value-render` component.

The host is plumbing only — it moves opaque binary-AST payloads and applies patches; all policy is the
reducer's fold + Cedar (same plumbing/policy split as the federation host).

---

## 6. Federation over the browser WebSocket — the transport

A native node dials the hub via the host `ws/dial` outbound effect and receives `ws/frame`/`ws/disconnect`
inbound events (`DESIGN-hub-federation-protocol.md` §1 Layer 0). In the browser there is no host to add a
primitive to — the browser natively provides `new WebSocket(hubUrl)`, `.send(frame)`, `onmessage`,
`onclose`. The `reducer-runtime` maps these one-to-one onto the reducer-facing seam so the federation
reducer fold is bit-identical native vs in-tab:

| Federation seam (native)        | Browser implementation                              |
|---------------------------------|-----------------------------------------------------|
| `ws/dial(hubUrl)` outbound effect | `new WebSocket(hubUrl)`; mint a conn-id `Hash`      |
| `ws/send(conn-id, frame)` effect  | `socket.send(frame_bytes)`                          |
| `ws/frame(conn-id, bytes)` inbound| `socket.onmessage` → deliver as inbound event       |
| `ws/connect` / `ws/disconnect`    | `socket.onopen` / `socket.onclose`                  |
| wire frame                        | `cadenza-ast` value-form doc via `ast-codec` (§2.2) |

**Asymmetry (a fit, not a limit):** a browser tab can only DIAL OUT (browsers cannot accept inbound ws) —
which matches the star topology exactly: outposts dial the hub, the hub never dials an outpost. The
handshake (`hello`/`welcome`), routing, directory, and "what federates" are unchanged from the federation
doc §2–§3; the browser is a consumer of that protocol. WebSocket reconnect/backoff is a `reducer-runtime`
concern (the fold's federation-session state tracks conn liveness).

---

## 7. The agent-drive path

Because the tab is a federation node with a `SessionId`, an agent (itself a reducer/node) drives it by
emitting a message to that `SessionId` over federation — the hub routes it, the browser host delivers it as
an `on-message` call, the app reducer folds it into new state + a `render` patch:

```
  agent reducer ──Emit{target: tab.SessionId, payload: cadenza-ast command}──▶ hub ──▶ tab reducer
       ▲                                                                                    │
       └────────────────── Emit{target: agent.SessionId, reply payload} ◀───── step.requests┘
```

Bidirectional by construction (the reply is just another `Emit`). An app exposes a **drive contract** — a
set of message shapes an agent may send — which is nothing more than the message variants its `on-message`
handles, the SAME surface a human's clicks map onto (§3.2). So "an agent can drive the app" needs no new
mechanism: the app author defines a drive contract as an ordinary set of `message` shapes, and any authorized
agent emits them. **Authorization** ("which agents may drive this tab") is the platform's existing Cedar
policy on the federation route (`cadenza-platform.md` §5) — a policy question, not new machinery. An app can
expose a purely-agent contract, a purely-human (DOM) contract, or both over the same fold.

---

## 8. Increments (top-to-bottom, the way a vertical lands them — once greenlit)

Each increment is independently gate-able and delivers a running artifact.

- **P0 — local reducer app (no network).** Ship `ast-codec` + `reducer-runtime` + `value-render` (the
  shared runtime) and one demo app `reducer` (counter/todo). The browser `ProgramStore` (jco) instantiates
  the reducer; it folds `dom-event` messages (§3.2) and emits `render` patches (§3.1); `state`→IndexedDB.
  Proves: browser `ProgramStore`, the `dom` interface, focused-component composition, `value-render`.
  *Gate:* the app reducer passes the in-process platform-conformance FIFO drive identically to its
  in-browser run (same events → same fold state), plus a headless-browser smoke (depends on closing the
  browser-driver gap, `DESIGN-browser-compound-property-test-driver.md`).
- **P1 — federate.** The tab dials the hub over native WebSocket (§6), runs the `hello`/`welcome`
  handshake, registers its `SessionId`, exchanges `cadenza-ast` frames via `ast-codec`. Proves: Layer 0/1
  in the browser, the federation reducer fold running unchanged in a tab. *Gate:* a federated `Emit`
  tab→node is observationally identical to an in-process Emit under platform-conformance (the ready-made
  oracle from the federation doc §0).
- **P2 — agent-drive.** An agent node emits messages to the tab's `SessionId`; the app folds them and
  replies (§7). Proves the drive-contract path end-to-end. *Gate:* a scripted agent drives the P0 demo app
  remotely; the resulting fold state matches the same command sequence applied locally.
- **P3 — hot-ship.** Publishing a new `ProgramHash` re-instantiates the app in-place (§4.2), preserving
  fold state from IndexedDB; `SessionId` preserved. Proves "evolve by new hash, never redeploy" in-browser.

Later / out-of-scope-for-v1: server-side rendering / hydration, multi-tab shared session, offline-first
sync conflict resolution, a Component-Model-native browser path when browsers ship it (drops the jco
transpile), the opt-in in-tab `compiler` component (playground apps).

---

## 9. Seams & file anchors

- Extend `design/cadenza-platform.md` §3 (L248–271): add a "The browser as an edge reducer" subsection
  stating the tab is an edge node whose WASI-edge is the `dom` interface + browser WebSocket, and that it
  ships focused components (not the monolith).
- `implementation/design/DESIGN-hub-federation-protocol.md` §1 Layer 0/1, §2 handshake — the browser reuses
  these; note that the browser substitutes native WebSocket for `ws/dial` (§6 table).
- `cdz-platform/wit/world.wit` — add the `dom` interface + a `browser-reducer-world` (reducer-world + dom).
  No change to `interface guest`.
- `cdz-platform/src/program.rs` `ProgramStore::spawn` — the seam the browser `ProgramStore` implements (new
  crate `cdz-platform-browser`, `#[cfg(target_arch = "wasm32")]`).
- `crates/cdz-wasm` / `crates/rcdzc-wasm` — DECOMPOSE per §2: today's monolithic bundle splits into the
  focused components (`ast-codec`, `reducer-runtime`, `value-render`, opt-in `compiler`). The JS↔wasm
  marshaling + `cadenza-ast` codec are the reuse targets.
- `crates/rcdzc/src/backend/wasm/DESIGN-reducer-guest-emission.md` — the app reducer is emitted here,
  unchanged; the browser just instantiates the product.

## 10. Open decisions — chosen default, alternative, and risk (per fork)

1. **In-browser component engine.** *Default:* jco transpile at build time (component → ES module + core
   wasm), loaded by hash. *Alternative:* wait for native browser Component Model (defers the capability);
   or in-tab jco transpile (heavier). *Risk:* jco output size/perf; toolchain dependency. *Chosen because*
   it is shippable today and the native path is a drop-in later.
2. **Compile-in-browser?** *Default:* NO — the browser runs pre-compiled reducers; compilation is a build-
   node concern (§2.1). *Alternative:* an opt-in `compiler` component for playground/live-coding apps.
   *Risk:* a future "notebook in the browser" use-case may want in-tab compile — handled by the opt-in
   component, not the default path. *Chosen because* it removes the largest bundle chunk + the miscompile-
   prone codegen surface (§2.4) from the common path.
3. **Which host does the browser federate with?** *Default:* the federation protocol (ws + cadenza-ast),
   satisfying the `cdz-platform` reducer WIT contract for the in-tab reducer — reconciling the two
   vocabularies (§11). *Alternative:* speak `cdz-platform` primitives (`Request`/`deliver`) directly.
   *Risk:* the federation arc is designed-not-landed, so P1 depends on it. *Escalate* only if the operator
   wants the tab to bypass federation.
4. **Render model.** *Default:* declarative vDOM patch as a `cadenza-ast` value (Elm architecture, §3).
   *Alternative:* a thin JS view layer subscribing to fold state (less deterministic, forks the value
   story). *Risk:* patch-diff perf for large trees — mitigated by keyed edits. *Chosen because* one
   encoding everywhere + replayable UI.
5. **State durability + hot-ship migration.** *Default:* IndexedDB-backed `state`, per-origin, survives
   reload; hot-ship carries state across a compatible hash. *Alternative:* in-memory only (simpler P0, no
   persistence). *Open risk:* state-SHAPE migration across INCOMPATIBLE reducer hashes is unspecified —
   proposal: a reducer declares a state-version; an incompatible bump triggers a reducer-defined migration
   message or a reset. Pin in P3.
6. **Component granularity.** *Default:* the §2.2 inventory (`ast-codec`, `reducer-runtime`, `value-render`,
   app, opt-in `compiler`). *Alternative:* finer or coarser splits. *Risk:* too-fine = composition/overhead
   churn; too-coarse = back toward the monolith. *Chosen* as the smallest set that separates "always-shared
   runtime" from "per-app" from "opt-in heavy."

## 11. Watch-outs

- **`host.rs` is wasmtime + `#[cfg(feature=host)]` — it cannot run in a browser as-is.** The browser
  `ProgramStore` is a distinct impl (§4); do not try to `wasm32`-compile the native host. Biggest fork point.
- **Two vocabularies.** The federation doc speaks `Emit`/`Inbound`/`SessionId`/`ws/*` (the outpost/harness
  host); `cdz-platform` speaks `Request`/`Message`/`deliver`/`ProgramStore` (in-repo WIT kernel). This design
  pins: comms = federation vocabulary, contract = `cdz-platform` WIT (decision 10.3). A vertical must not
  blur them.
- **Browser tabs dial out only.** Star topology is a hard fit, not a limitation (§6).
- **Determinism.** Keep `render` a *value* (patch) and route ALL user input as `message`s, so the tab
  reducer stays a pure fold the platform-conformance oracle can replay. An imperative DOM escape hatch would
  break the replay gate — resist it. NB: `set-timer`/wall-clock introduce nondeterminism — model timers as
  explicit `dom-event` inbound so they enter the fold log like any event (do not read the clock inside the fold).
- **Don't re-introduce the monolith.** The whole point of §2 is that the browser ships focused components;
  a build that bundles `rcdzc`+`cadenza-syntax` into the default browser artifact defeats it. The `compiler`
  component is opt-in only.
- **Missing browser driver.** The headless-browser test path is a known gap
  (`DESIGN-browser-compound-property-test-driver.md`) — P0's gate depends on closing it; coordinate.
- **jco toolchain surface.** jco is an external dependency in the build path; pin it + treat its output as a
  build artifact (content-addressed like everything else).

## 12. Relationship to existing arcs

- **To `DESIGN-hub-federation-protocol.md`:** the browser outpost is a CONSUMER of the federation protocol —
  it adds no wire, no handshake, no topology; it substitutes the transport implementation (native WebSocket)
  and adds a host. If federation lands first, the browser is "federation, minus the native ws host, plus a
  `dom` edge."
- **To `cadenza-platform.md`:** this specializes the "edge reducer" / "everything is a wasm module"
  sections; it does not change the kernel, the session model, or `interface guest`.
- **To `cdz-wasm` / `rcdzc-wasm`:** those prove Cadenza wasm + the `cadenza-ast` codec run in a browser; the
  browser outpost DECOMPOSES that surface (§2) into focused components and builds the reducer-host layer on
  top.
- **To the guide-examples wasm32 OOB (LLVM-22 miscompile of the compiler's parse/render path):** the focused-
  component split keeps that codegen surface out of the tab (§2.4), so the browser outpost neither depends on
  nor is blocked by that bug.
