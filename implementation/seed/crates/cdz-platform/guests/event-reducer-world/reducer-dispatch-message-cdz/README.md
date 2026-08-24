# reducer-dispatch-message-cdz — a Cadenza event reducer performing FULL handler-chain routing

`reducer.cdz` (authored by v-platform) is the Cadenza event reducer that performs the complete §4 routing
act: on `on-message` it calls `deliver` with the whole message envelope —
`deliver-message(target: list<u8>, event: message{contract, sender{reducer, host}, payload, token})` — the
NESTED-record shape (v-rust-backend's shape-d3, #3170). Where `reducer-dispatch-cdz` (deliver-notification,
#3159/#3164) proved the flat `record{contract, payload}`, this proves the nested `sender` sub-record composes,
so a message routes to the next handler with its full provenance intact.

The minimal self-consistent form re-delivers the incoming message back to its sender's reducer, rebuilding the
envelope field-for-field — so a conformance run can assert every field (contract, sender.reducer, sender.host,
payload, token) crosses through unchanged.

Targets the privileged **event-reducer-world** (`KIND_WIT_WORLD` artifact) — the only world that imports
`deliver`. Consumes `event-reducer-world.bin`
(`mkCadenzaGuest { witWorld = "${worldArtifacts}/event-reducer-world.bin"; witWorldName = "event-reducer-world"; }`).
The component imports `cadenza:platform/deliver` and exports the typed `cadenza:platform/guest`.
`on-response` / `on-notification` are inert.

The conformance run (spawn as `kind = "event"`, deliver a message, assert the routed message carries every
field) is v-platform-itest's.
