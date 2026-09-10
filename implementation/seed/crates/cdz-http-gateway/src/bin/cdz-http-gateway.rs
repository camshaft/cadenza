//! The `cdz-http-gateway` deployable binary (`DESIGN-http-outpost.md` §0.1 "deploy once"): serve a
//! deployment directory (a `route-table.bin` frame + the `*.wasm` handler/runtime components) over HTTP.
//!
//!   cdz-http-gateway <listen-addr> <component-dir>
//!
//! e.g. `cdz-http-gateway 127.0.0.1:8080 ./deploy` — the dir holds `route-table.bin`, the value-heap
//! `runtime.wasm` + `nfc.wasm`, and each handler `.wasm`. Behind the `host` feature (it drives wasmtime).

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let [_, addr, dir] = args.as_slice() else {
        eprintln!("usage: cdz-http-gateway <listen-addr> <component-dir>");
        std::process::exit(2);
    };
    let addr = addr.parse().unwrap_or_else(|_| {
        eprintln!("cdz-http-gateway: invalid listen address {addr:?} (e.g. 127.0.0.1:8080)");
        std::process::exit(2);
    });
    cdz_http_gateway::boot::serve(addr, std::path::Path::new(dir)).await
}
