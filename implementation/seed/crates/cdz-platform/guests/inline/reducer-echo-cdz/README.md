# reducer-echo-cdz — a Cadenza-authored reducer guest, driven end-to-end

`reducer.cdz` (authored by v-platform) is a faithful, host-drivable reducer written in Cadenza. An inline
`world reducer-world` declares the typed `cadenza:platform/guest` export over the real `world.wit`
`message`/`response`/`notification` → `step` shapes, so `rcdzc` emits the WIT-typed boundary the platform
host binds:

```
on-message:      func(msg:  message)      -> step
on-response:     func(resp: response)     -> step
on-notification: func(note: notification) -> step
```

`on-message` echoes the delivered payload back as one request on the same contract and `Continue`s (the
event round-trips, session stays alive); `on-response`/`on-notification` are inert (`Continue`, no
requests) — the fixture drives the message path. It calls no host imports.

The nix flake compiles it with `mkCadenzaGuest` — `cdz compile reducer.cdz --target wasm --component-name
cadenza:platform/guest` — canonicalizes + content-addresses it, and registers it in `harnessPrograms` under
`reducer-echo-cdz`, the same slot the hand-written Rust `reducer-echo` occupies. This is the operator's
"Cadenza guests, not Rust" path: a reducer authored in Cadenza, compiled by `rcdzc`, spawned and driven by
the platform host exactly like the Rust fixture.

`--component-name cadenza:platform/guest` names the fully-qualified WIT interface the guest publishes its
exports under (the interface v-platform's `WasmReducer` binds). The inline `world` is what *types* the
export (`message`→`step`); a bare `--component-name` without the world, or the external `KIND_WIT_WORLD`
artifact, emits only an untyped heap export today — so the inline world is load-bearing.

## Driven end-to-end

`harness-runs/reducer-echo-cdz-echo.ml` spawns this guest and delivers one message; the harness run
`checks.<sys>.harness-reducer-echo-cdz-echo` passes (exit 0), i.e. the platform host composed the guest
(runtime + nfc), bound the typed guest interface, and folded `on-message` to quiescence. That is the first
end-to-end drive of a Cadenza-authored reducer through the platform. A checker asserting the echoed request
in the observation log is a follow-up (v-platform owns the checker).
