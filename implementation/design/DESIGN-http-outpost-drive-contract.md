# HTTP-outpost — the gateway ↔ looping-program DRIVE CONTRACT + current architecture

Status: DESIGN (v-http-outpost). The AUTHORITATIVE spec for how the *dumb* gateway drives control-server-
provided programs as **looping reducers that emit effects/timers and exchange messages with the control
server**, and the home for the operator's requirements + refinements. Companion to `DESIGN-http-outpost.md`
(original) and `DESIGN-http-outpost-integration-harness.md` (testing); SUPERSEDES the original doc's one-shot
handler + in-gateway routing model.

## Operator requirements & refinements (2026-09-10) — the current intent, captured

These direct the architecture and supersede earlier shapes. Each is clarified in the section noted.

1. **Drive loop mirrors `cdz-platform/src/system.rs`, NOT a serial request/response loop** (§1). The gateway
   must NOT block its event loop awaiting each effect's response one-at-a-time; it drives a reducer like
   system.rs's launch loop — a per-session mailbox, fire-and-forget effects, responses folded back in
   arrival order (correlated by `continuation_token`), many effects in flight, messages handled in any
   order. Refactor system.rs to expose/reuse the loop shape if that's cleaner than a parallel copy.
2. **One PERSISTENT, BIDIRECTIONAL control-link connection** (§3). Not a one-shot fetch. A single ws
   multiplexes: config/root-router pushes DOWN, a handler's `control.send` UP (as a provenance-stamped
   envelope), and control's RESPONSE back DOWN routed to the originating session. Request/response to the
   control server rides this same connection.
3. **Canonical platform hashes** (§5). The contract-ids (and schema ids) must be computed the SAME WAY as
   other platform contracts — the Cadenza program reflects + hashes the types (the descriptor), and Rust
   codegen pulls in the ACTUAL hash. Replace the v0 padded 33-byte ASCII markers.
4. **Routing baked into a compiled program, shipped by hash** (§4). The root router bakes its table + the
   real handler `ProgramHash`es into its source, COMPILED ahead of time (cheap in Cadenza), shipped by hash;
   the gateway fetches it and drives it with a BARE http-request, holding nothing. A route change = recompile
   + push a new program hash (the live cell swaps the HASH). Not a table blob on the wire, not runtime-
   templated by control.
5. **HTTP response body = `Inline | CasRef(hash)`** (§6). A handler may answer with an inline body or a CAS
   reference; the gateway resolves-by-hash + pipes it. Buffered v0 (fetch-whole-then-pipe); streaming later.
6. **Tests are SCRIPTED nix integration tests, NOT rust `#[test]`s** — nix builds the binaries, a script
   drives control + gateway + CAS and makes observations. See `DESIGN-http-outpost-integration-harness.md`.
   Retire superseded rust `#[test]`s once harness scenarios cover them.
7. **Crate cleanup** (§8). Retire dead pre-redirect cruft — `boot.rs`'s directory-based deployment, the
   superseded RouteQuery guest + router-dynamic/DynamicRouter/Router/RouterReducer/HandlerRunner — as the new
   path lands.

## 0. The key finding — no platform gap in the reducer surface
The reducer world (`cdz-platform/src/reducer.rs`) already exposes the surface a looping, effect-emitting,
timer-setting, message-exchanging program needs. What was WRONG was the gateway-side DRIVER (serial), not the
platform. The driver must be re-shaped to system.rs's model (§1); the effect vocabulary (§2) is outpost-defined.

| Need | Existing surface |
|---|---|
| loop across turns | `Outcome::Continue` keeps the reducer alive; `Break{schema,reason}` ends it |
| emit effects | `on_message`/`on_response`/`on_notification` all return `(Vec<Request>, Outcome)` |
| an effect | `Request { id: ContractId, payload: Bytes, continuation_token: Bytes, deadline: Option<Duration> }` |
| a timer | a `Request` with `deadline: Some(d)` |
| fold an effect's answer | `on_response(Response { id, continuation_token, payload: Result<Bytes,Error> })` |
| receive an unsolicited push | `on_notification(Notification { id, payload })` |
| provenance | `Message.from: Origin { reducer, host }` (unforgeable envelope metadata) |

## 1. The drive loop — the `system.rs` mailbox/event-loop model (NOT serial)
`cdz-platform/src/system.rs` drives each reducer as its own async task over a per-reducer **mailbox** (an
mpsc channel), NOT a serial await-each-effect loop. The gateway's driver must have the SAME shape (refactor
system.rs to share it where feasible rather than keep a divergent copy):

```
spawn(program from CAS by hash); create a mailbox (Sender kept by the gateway, Receiver drained by the task)
task:  while let Some(event) = mailbox.recv().await:          # Message | Response | Notification, ANY order
         (requests, outcome) = reducer.fold(event)            # on_message / on_response / on_notification
         for req in requests: carry_out(req)                  # §2 — FIRE-AND-FORGET: never await the answer
         if outcome == Break{schema, reason}: finish; stop
       # connection closed → deliver on_notification(closed), stop
```

- **Per-session mailbox, one task per driven reducer.** Inbound events — a socket frame, a control push, a
  timer firing, a carried-out effect's response — are all just `Delivered` events pushed into the mailbox and
  folded in ARRIVAL order. (system.rs `Shared::launch` recv-fold loop; `fold` dispatches by variant.)
- **Effects are FIRE-AND-FORGET.** `carry_out(req)` dispatches the effect and returns immediately; when the
  effect produces an answer it is `send`-injected back into the SAME mailbox as a `Response`, correlated by
  the request's `continuation_token`. Multiple effects can be in flight; the loop never blocks awaiting one.
  This is the key departure from the old serial `drive_loop` (which `await`ed each `resolve` — rejected).
- **No pending-requests map in the driver.** Correlation is entirely the `continuation_token` echoed on each
  `Response`; the reducer matches it. (Same as system.rs — no driver-side outstanding-effect table.)
- **Timers** are effects, not a loop primitive: a `Request` with `deadline: Some(d)` arms a detached
  sleep-then-`send` that injects the fire/`Timeout` back into the mailbox by `continuation_token` (system.rs
  `FireAfter`/`timers.wrap`; cancelled on reducer exit).
- **Reuse vs. copy:** system.rs's `launch` is coupled to the full runtime (graph/registry/runner/kind checks).
  The reusable core is the recv-fold-fire-and-forget shape + the mailbox; the refactor extracts that (a trait
  for "dispatch this effect; later inject a Response back") and drops the platform coupling, so the gateway
  drives ONE reducer over ONE connection with the same loop shape.

## 2. The effect-contract vocabulary (outpost-defined; the gateway routes by `req.id`, payload OPAQUE)
The gateway is a pure router of effects: it switches on the contract-id and never inspects the payload beyond
the envelope a given effect needs. (Contract-ids are the canonical descriptor hashes of §5, not markers.)

- **`http.dispatch`** — "spawn this subprogram and hand it this input" (subprogram `ProgramHash` + input).
  The gateway fetches it from CAS, spawns + drives it (§1), and its terminal `Break` reason folds back as the
  emitter's `on_response`. How the ROOT ROUTER dispatches to a handler.
- **`http.response`** — "answer the current HTTP request with this http-response" (body per §6). Written to
  the socket.
- **`ws.send`** — "push this frame to the connection" (the ws-send effect).
- **`http.deny`** — "reject this request / ws-upgrade" (status + reason). The gateway floors/closes.
- **`control.send`** — "send this OPAQUE payload to the control server." Wrapped with provenance (§3) and
  forwarded UP the persistent link; control's response comes back DOWN and folds via `on_response`.
- a **timer** — any `Request` with `deadline: Some(d)` (no dedicated contract).

## 3. The persistent, bidirectional control link (ONE ws, multiplexed)
The gateway holds ONE persistent ws to the control server for its whole life (redirect + operator). It is a
BIDIRECTIONAL multiplexed bus — the gateway is a pure opaque router (provenance/addressing only, never
payload):

- **DOWN — config/routing:** the initial `ControlConfig { cas_url, cas_credential, root_router }` on connect,
  then each pushed update (a NEW `root_router` program hash → live-swap, applied by the next request; reuses
  the #8619 cell, now holding a hash). Delivered via a persistent link (config-down half:
  `control_link::run_control_config_link`).
- **UP — handler → control request:** a `control.send` effect → `ControlUp { provenance:{program, session},
  payload }`, written to the link's WRITE half (the `GatewayResolver`'s `ControlSink` is wired here).
- **DOWN — control → handler response/push:** `ControlDown { addressing:{session}, payload }`, routed by
  `session` to the originating reducer's mailbox and folded as `on_response` (to a `control.send`) or
  `on_notification` (an unsolicited push). Correlated by `continuation_token`/`session`.

All three are multiplexed on the one connection; the read loop DEMUXES `ControlConfig` vs `ControlDown`. The
envelope is binary-AST (self-describing); the gateway never decodes `payload`.

## 4. The root router — routing baked into a compiled program, shipped by hash
Operator A/B decision: the routing table + the REAL handler `ProgramHash`es are BAKED INTO the router source,
COMPILED ahead of time by deploy tooling, and SHIPPED BY HASH. The gateway fetches the program from CAS by
the control-supplied `root_router` hash and drives it with a BARE http-request (no `RouteQuery` envelope, no
table on the wire); it holds nothing. A route change = recompile the router with the new table + push the new
program hash (the live cell swaps the HASH). Deploy tooling bakes real hashes via `cdz-http-programhash`
(#8652) into `b"\xNN…"` byte literals before `cdz compile`. (Not runtime-templated by control; not a gateway-
held table.)

## 5. Canonical platform hashes (descriptor-derived contract-ids)
The effect/schema contract-ids (§2) and the terminal schemas (`http.response`/`deny`) must be the canonical
ids other platform contracts use — the Cadenza program reflects + hashes the contract's types (the
descriptor), and Rust codegen exposes the ACTUAL hash for the gateway to match. Replaces the interim v0
padded 33-byte ASCII markers (`b"cdz.http.dispatch……"` etc.). This is the "schema-id descriptor-read fix":
expose the userspace-contract descriptor ids to Rust the way kernel contracts are, and assert each routed id
== its canonical derivation (a drift gate).

## 6. HTTP response body — `Inline | CasRef(hash)`
An `http-response`'s body is `Inline(Bytes) | CasRef(Hash)`. On `CasRef`, the gateway fetches the blob from
the CAS by hash and pipes it as the body. Buffered v0 (fetch-whole-then-pipe via the `BlobStore`); a
streaming CAS GET is a later refinement when a large body makes buffering hurt.

## 7. Testing — scripted nix integration harness (see the companion doc)
Behavior is proven by a SCRIPTED nix integration harness (nix builds the binaries; a script boots control +
gateway + CAS, drives real HTTP + control frames, asserts observable outcomes), NOT rust `#[test]`s. Full
design in `DESIGN-http-outpost-integration-harness.md`. Rust `#[test]`s for gateway/drive-loop behavior are
retired as harness scenarios cover them.

## 8. Re-architecture sequence + cleanup
1. **Drive loop → system.rs event-loop model** (§1): a mailbox-driven, fire-and-forget session driver;
   refactor system.rs to share the loop shape. Re-home dispatch/timers/`control.send` onto it.
2. **Canonical descriptor hashes** (§5): replace the markers + add the drift gate.
3. **The persistent bidirectional control-link bus** (§3): config-down + ControlUp-up + ControlDown-to-session.
4. **Gateway boot-from-control**: dial control → `ControlConfig` → build the dumb edge over the CAS +
   root-router hash → serve; a root-router-hash push swaps live.
5. **The scripted nix harness** (§7) + scenarios (route-to-handler, live-swap, deny/413/504, control.send
   round-trip over the persistent link, content-addressed body).
6. **Cleanup** (§7 req 7): retire `boot.rs`'s directory deploy + the superseded RouteQuery guest / router-
   dynamic / DynamicRouter / Router / RouterReducer / HandlerRunner + their rust `#[test]`s.

Landed so far: contracts/codec/edge spine, `HttpEdge::dumb` bare-request path, the baked-table root-router
guest (#8649) + its runtime e2e (#8650), the `cdz-http-programhash` deploy tool (#8652). WIP (uncommitted):
the control-server binary + config-link — to be realigned to §1/§3 before landing.

## 9. Retrospective — what we wish we'd known at the start (to avoid building the wrong thing)
This vertical built several things that were later thrown out. The wrong turns, and the foundational
assumption that would have prevented each — read this BEFORE building the next outpost-shaped system:

1. **Drive the loop like the platform already does — study `system.rs` FIRST, don't invent a driver.** We
   wrote a bespoke SERIAL drive loop (`resolve(effect).await` one at a time). The platform's `system.rs`
   already drives reducers the right way: a per-reducer mailbox, fire-and-forget effects, responses folded in
   arrival order by `continuation_token`. **Lesson:** when the platform already runs the exact abstraction you
   need (a looping reducer), find and REUSE/refactor its driver before writing your own; a "simpler" serial
   version is a different, wrong execution model, not a simplification.
2. **The control link is ONE persistent, bidirectional, multiplexed connection — design it full-duplex up
   front.** We modeled it as a one-shot config fetch, then a one-directional table/config reader. It is
   actually the gateway's single long-lived bus: config/routing DOWN, handler requests UP, control responses
   DOWN, routed by session. **Lesson:** a "control plane" link is almost always a persistent bidirectional
   bus; assume that, not request/response or one-way push.
3. **Ship a compiled PROGRAM, don't pass or template config.** We built routing three wrong ways — a baked
   table returned as a decision (`router`), a table passed in the message (`router-dynamic`/`RouteQuery`), and
   an in-gateway live table (`DynamicRouter`) — before the answer: bake the table + real handler hashes into
   the router SOURCE, compile it, ship it BY HASH; a route change recompiles + repushes the hash. **Lesson:**
   in a system where compilation is cheap and content-addressed, prefer "compile a program and ship its hash"
   over "carry config data at runtime." Ask early: is this config, or should it be a program?
4. **Use the platform's canonical contract-id scheme from day one.** We stubbed contract/schema ids as padded
   33-byte ASCII markers ("v0"), which then had to be threaded consistently across guest + Rust + wire and
   still aren't the real ids. **Lesson:** derive ids the way the rest of the platform does (reflect + hash the
   descriptor; codegen the real hash) from the start — an ad-hoc id scheme is debt that touches every layer.
5. **Build the black-box integration harness FIRST; express behavior there, not in rust `#[test]`s.** We
   accreted a large pile of rust `#[test]`s (timers, deny, dispatch-chain budget, provenance, drive-loop
   semantics) — language-locked unit tests of internals — that are now superseded by a scripted nix harness.
   **Lesson:** for a multi-binary system (gateway + control + CAS + Cadenza guests), stand up the scripted
   integration harness early and drive behavior through the wire; reserve unit tests for pure leaf logic.
6. **(Meta) Pin the foundational architecture with the operator BEFORE incremental building.** The core
   decisions — the drive/execution model, the control-plane protocol shape, the config-vs-program boundary,
   the id scheme, and the test strategy — were settled reactively, after many slices had committed to wrong
   assumptions. **Lesson:** for a greenfield system with an evolving spec, write these foundational choices
   into the design doc and confirm them with the operator up front; a day of design alignment would have
   saved multiple rebuild cycles. Keep the design doc the living source of truth (this doc) as they evolve.
