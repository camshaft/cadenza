# reducer-echo-cdz — a Cadenza-authored reducer guest (the `.cdz`→wasm path)

`reducer.sexp` is a minimal reducer written in Cadenza (s-expression surface), authored by
v-platform. It declares its reducer world INLINE via `(world reducer-world (export guest …))` and
implements `on-message` as an echo (returns `msg.payload`), calling no host imports.

The nix flake compiles it to a wasm component with `mkCadenzaGuest` —
`cdz compile reducer.sexp --target wasm --component-name cadenza:platform/guest` — and registers the
result in `harnessPrograms` under `reducer-echo-cdz`, the same slot the hand-written Rust
`reducer-echo` occupies. This is the operator's "Cadenza guests, not Rust" path: a reducer authored in
Cadenza, compiled by `rcdzc`, spawned and driven by the platform host exactly like the Rust fixture.

`--component-name cadenza:platform/guest` names the fully-qualified WIT interface the guest publishes its
exports under (the interface v-platform's `WasmReducer` binds `on-message` on); without it `rcdzc` has no
component name and declines a record-of-bytes `message` param.

Scope note (2026-08-23): `on-message` here echoes a record-of-bytes to bytes — a real but simplified
shape. The full platform `guest` interface is `on-message: func(message) -> step` and adds
`on-response`/`on-notification`; growing this probe toward the full typed `step` result and the host
imports (`state`/`blobs`) tracks v-rust-backend's world-export + shared-allocator slices.

## `reducer-step.ml` — the faithful `message → step` echo (staged, awaiting typed-export emit)

`reducer-step.ml` (authored by v-platform) is the reducer the platform host can actually *drive*:
`on-message: (message) -> step` — folds a delivered `message` and emits one request echoing the payload
plus `Continue`, matching the real `cadenza:platform/guest` interface (`world.wit`). It is front-end clean
(`cdz check` exit 0).

It is NOT yet wired into `harnessPrograms` because the guest EXPORT does not TYPE yet: compiled today
(with `--component-name cadenza:platform/guest`, or against the external `KIND_WIT_WORLD` artifact) it emits
an untyped heap export `on-message: func(u32) -> u32`, not the WIT-typed `message -> step` the host binds.
A typed export needs an inline `(world …)` declaring the full `message`/`step` member signatures (the path
that typed `reducer.sexp`'s record→bytes export) — the inline-typed-world surface gap (routed to v-syntax)
— OR external-artifact export typing (routed to v-rust-backend). The moment a typed `message -> step`
export emits, this file is wired into `mkCadenzaGuest` + `harnessPrograms` and driven by a spawn/deliver
harness run. Kept here so the authored, verified source is version-controlled, not lost in a fleet note.
