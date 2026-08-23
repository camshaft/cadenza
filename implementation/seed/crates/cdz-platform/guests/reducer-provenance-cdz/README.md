# reducer-provenance-cdz — a Cadenza reducer on the PRIVILEGED event-reducer-world

`reducer.cdz` (authored by v-platform) performs `provenance.program-of(msg.sender.reducer)` in `on-message`
— reading which program the sending reducer runs (a bare `list<u8>` host result, #3121 shared-allocator) —
and stamps it as the echoed request's token. It targets the PRIVILEGED **event-reducer-world** (adds the
graph / deliver / provenance imports over the ordinary reducer world) and uses the natural `[{…}]`/`[]`
list literal (no `List.push` helpers).

Like `reducer-identity-cdz`, it CALLS a host import, so it consumes the external `KIND_WIT_WORLD` artifact
`event-reducer-world.bin` (`mkCadenzaGuest { witWorld = "${worldArtifacts}/event-reducer-world.bin";
witWorldName = "event-reducer-world"; }`) for the import declarations. The component imports
`cadenza:platform/provenance` + `cadenza:runtime/heap@…` and exports the typed `cadenza:platform/guest`.

Registered as `reducer-provenance-cdz`; `harness-runs/reducer-provenance-cdz-echo.ml` spawns it with
`kind = "event"` (the privileged event/system reducer) + delivers a message — validating the platform host
composes a PRIVILEGED import, not only the ordinary `state`/`identity` ones.
