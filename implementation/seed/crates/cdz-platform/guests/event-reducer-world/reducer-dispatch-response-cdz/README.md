# reducer-dispatch-response-cdz — a Cadenza event reducer performing ANSWER-BACK routing

`reducer.cdz` (authored by v-platform) is the Cadenza event reducer that folds a reply back to a caller: on
`on-message` it performs `deliver-response` — injecting a `response` event into the caller's log. The `answer`
is `result<payload, error>` (the prelude `Result`): a successful reply carries the output value in `Ok`, a
runtime failure an `Error` variant in `Err`. This completes the §4 dispatch trilogy on the guest side
(notification → message → response).

The minimal self-consistent form answers the sender's reducer, echoing the incoming message's `contract` and
correlation `token`, with `answer = Ok(msg.payload)` — a successful reply that hands the payload straight back
— so a conformance run can assert the delivered response correlates by token and carries the value. The
`response` record shape is `{contract, token, answer: result<payload, error>}` (`wit/world.wit`); the op is
`deliver-response(target: reducer-id, event: response) -> bool`.

Targets the privileged **event-reducer-world** (`KIND_WIT_WORLD` artifact) — the only world that imports
`deliver`. Consumes `event-reducer-world.bin`
(`mkCadenzaGuest { witWorld = "${worldArtifacts}/event-reducer-world.bin"; witWorldName = "event-reducer-world"; }`).
The component imports `cadenza:platform/deliver` and exports the typed `cadenza:platform/guest`.
`on-response` / `on-notification` are inert.

The `answer` field exercises a host-op argument carrying `result<payload, error>`: v-rust-backend's
deliver-response frontend (#3171) resolves it against the guest's named `Error` sum (the world's anonymous
`error` variant maps to the guest's `type Error`); the result-field CODEGEN lands separately. Until #3171 is
on main, compiling declines only at that op's resolution (`CDZ0201`) — the guest is otherwise sound. The
conformance run (spawn as `kind = "event"`, deliver a message, assert the routed response's token + answer) is
v-platform-itest's.
