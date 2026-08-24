# reducer-dispatch-cdz — the first Cadenza event reducer performing `deliver` (§4)

`reducer.cdz` (authored by v-platform) is the first Cadenza **event** reducer that performs the privileged
**routing act** (`design/cadenza-platform.md` §4): on `on-message` it calls `deliver` — injecting an event
into another reducer's log. This minimal §4 form delivers a **notification** of the effect back to its
sender (`deliver-notification(target: list<u8>, event: record{contract, payload})` — v-rust-backend's
shape-d2, #3159: a mixed `list<u8>` target + a record-with-Bytes-fields event). It proves a Cadenza event
reducer composes the privileged `deliver` import end-to-end.

It targets the privileged **`event-reducer-world`** (`KIND_WIT_WORLD` artifact) — the only world that imports
`deliver` (plus `graph`/`provenance`). The component imports `cadenza:platform/deliver` and exports the typed
`cadenza:platform/guest`; `on-response`/`on-notification` are inert. Compiles to a 5087-byte typed component:

```
cdz compile reducer.cdz wit-world:event-reducer-world=event-reducer-world.bin --component-name cadenza:platform/guest --target wasm
```

The nix flake auto-enumerates it (`guests/event-reducer-world/reducer-dispatch-cdz/`). The full dispatch
(`deliver-response` for answer-back, `deliver-message` for the handler chain) follows as v-rb lands the
result-field and nested-record host-arg shapes; the paired conformance run (spawn the event reducer, deliver
a message, assert it delivered a notification to the sender) is authored by v-platform-itest.
