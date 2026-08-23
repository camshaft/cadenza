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
