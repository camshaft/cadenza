# reducer-echo — a reducer guest component fixture

A minimal wasm **reducer component** that targets the platform's `reducer-world`
(`../../wit/world.wit`). The host driver's end-to-end tests (`cdz-platform`, behind
the `host` feature) instantiate it and drive an event through it, proving the WIT
ABI, the host imports, and the event↔WIT conversion layer all round-trip through a
genuine component rather than a mock.

Behaviour (see `src/lib.rs`): `on-message` echoes the event back as one request on
the same contract with the same payload, and sets that request's token to the
reducer's own id read from the `identity` host import. `on-response` /
`on-notification` are inert (`continue`, no requests).

## Building it

Standalone crate (its own `[workspace]`) so the native seed `cargo build` / gate
ignores it, exactly like `cdz-nfc` and `cdz-runtime`. Build the component with:

```sh
cargo component build --release --target wasm32-unknown-unknown
# → target/wasm32-unknown-unknown/release/cdz_platform_reducer_echo.wasm
```

The built `.wasm` is **not** checked in (operator ruling: no one-off committed
fixtures — the guest must be part of the reproducible builds). **v-nix** owns
wiring `cargo component` into the wasm CI job so this component is built
reproducibly, alongside the integration executable; the host's end-to-end driver
test loads the nix-built artifact rather than a checked-in file. This is a test
fixture, not a shipped hash-pinned artifact, so it needs no build-std
byte-determinism.
