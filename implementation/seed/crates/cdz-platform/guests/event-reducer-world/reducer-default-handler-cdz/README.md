# reducer-default-handler-cdz — the default event handler as a Cadenza event reducer

`reducer.cdz` (authored by v-platform) is the **default event handler** (`design/cadenza-platform.md` §4;
`DESIGN-default-event-handler-guest.md`): the privileged event program the `EventRegistry` resolves a contract
to when no override is installed. On `on-message` — an effect the kernel routes here, stamped with the emitting
reducer as `sender` — it reads that emitter's transform chain for the effect's contract from the reducer graph
and routes through it:

- `graph.neighbors(msg.sender.reducer, msg.contract, Dir.Outgoing)` — the NODE is the **emitting reducer** (the
  reducer that spawned the event); the edge KIND is the **contract-id** itself (the graph keys a contract's
  chain under `for_contract(contract)`, whose bytes are the contract-id). The chain returns in weight-then-id
  (routing) order.
- **chain non-empty** → **forward** the effect to the first transform (`deliver-message`, full envelope
  intact); the transform re-routes onward as it emits.
- **chain empty** → **handle by default**: with no configured transform, decline with a rejection `response`
  (`answer = Err(missing-handler)`) back to the caller. Routing itself never auto-faults — the default handler
  decides.

The operator's model: the `EventRegistry` maps a contract → the **privileged** event program (this guest); the
graph holds, per emitting reducer, the chain of **non-privileged** transform reducers. They coexist — the
registry picks the program, the graph supplies the per-owner transform chain — and dispatch does not migrate to
a graph read.

Targets the privileged **event-reducer-world** (`KIND_WIT_WORLD` artifact) — the world importing `graph` +
`deliver`. Consumes `event-reducer-world.bin`
(`mkCadenzaGuest { witWorld = "${worldArtifacts}/event-reducer-world.bin"; witWorldName = "event-reducer-world"; }`).
The component imports `cadenza:platform/graph` + `cadenza:platform/deliver` (the first guest performing **two**
host interfaces — v-rust-backend's multi-interface emit #3232, multi-record-param #3233, deliver-response
err-arm #3228) and exports the typed `cadenza:platform/guest`. `on-response` / `on-notification` are inert.

The conformance run (install this as the registry default, emit an ordinary effect, assert it is routed —
forwarded when a transform chain exists, rejected otherwise, observed via the recorded `Routed` act) is
v-platform-itest's.
