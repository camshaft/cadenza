//! The `cdz-http-gateway` binary — the **dumb gateway, boot-from-control** entry point
//! (`DESIGN-http-outpost-drive-contract.md` §3/§4).
//!
//!   cdz-http-gateway --listen-addr <http host:port> --control-addr <control-ws host:port>
//!
//! The gateway is told ONLY where to serve HTTP and where the control server is. On start it dials a
//! single persistent bidirectional WebSocket to `ws://<control-addr>/`, applies the `ControlConfig`
//! control ships on connect (CAS URL + credential + root-router `ProgramHash`), resolves programs from the
//! CAS by hash, and serves HTTP on `--listen-addr` by driving the root-router program. A route change =
//! control pushes a new `ControlConfig` (new root-router hash) → live swap, no restart. Behind the `host`
//! feature (it instantiates wasm reducers via the wasmtime-backed store).
//!
//! CLI contract (agreed with `v-gateway-conformance`, whose harness launches this binary):
//!  - `--listen-addr <host:port>` (env `CDZ_GATEWAY_LISTEN_ADDR`): the HTTP serve address. The harness
//!    passes `127.0.0.1:0` for an ephemeral port, so once bound the gateway PRINTS
//!    `gateway: listen=<bound-addr> control=<control-addr>` to stderr for the driver to read the real port.
//!  - `--control-addr <host:port>` (env `CDZ_GATEWAY_CONTROL_ADDR`): the control server's ws address to dial.
//!
//! Boot wiring is built up over the rewrite's slices; this entry point parses + validates its inputs and
//! hands off to the boot path as each slice lands.

use std::process::ExitCode;

struct Args {
    listen_addr: String,
    control_addr: String,
}

/// Parse `--listen-addr`/`--control-addr` (in any order), falling back to the env vars. Returns `None`
/// (→ usage) if either address is missing.
fn parse_args() -> Option<Args> {
    let mut listen_addr = std::env::var("CDZ_GATEWAY_LISTEN_ADDR").ok();
    let mut control_addr = std::env::var("CDZ_GATEWAY_CONTROL_ADDR").ok();

    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        match flag.as_str() {
            "--listen-addr" => listen_addr = it.next(),
            "--control-addr" => control_addr = it.next(),
            _ => return None,
        }
    }

    Some(Args {
        listen_addr: listen_addr?,
        control_addr: control_addr?,
    })
}

fn main() -> ExitCode {
    let Some(args) = parse_args() else {
        eprintln!(
            "usage: cdz-http-gateway --listen-addr <http host:port> --control-addr <control-ws host:port>\n\
             (env: CDZ_GATEWAY_LISTEN_ADDR / CDZ_GATEWAY_CONTROL_ADDR)"
        );
        return ExitCode::from(2);
    };

    // Boot-from-control (dial the control link, apply ControlConfig, serve by driving the root router,
    // printing the bound listen addr) is wired in the boot-from-control slice of the rewrite. Until then
    // the entry point validates its inputs and reports the gap rather than pretending to serve.
    eprintln!(
        "cdz-http-gateway: boot-from-control not yet wired (listen={:?} control={:?}); \
         see DESIGN-http-outpost-drive-contract.md",
        args.listen_addr, args.control_addr
    );
    ExitCode::FAILURE
}
