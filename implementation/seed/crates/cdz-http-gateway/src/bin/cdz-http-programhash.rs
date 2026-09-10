//! `cdz-http-programhash` — compute a compiled component's `ProgramHash` for the deploy-baking step.
//!
//! The operator-mandated root router (`guests/root-router-baked`) bakes its route table + the REAL handler
//! `ProgramHash`es into the compiled program. A guest cannot compute a content hash, so the DEPLOY tooling
//! computes each handler's hash from its compiled `.wasm` and bakes it into the router source (a Cadenza
//! `b"…"` byte literal — `\xNN` escapes compile to raw bytes) before `cdz compile`. This is that hashing
//! primitive: there is no `cdz` subcommand for it, and computing it needs only `ProgramHash::of` (the same
//! call `MockControlServer` uses to key the route table), so it lives here as a tiny standalone tool with no
//! `host`/wasmtime dependency.
//!
//! Usage:
//!   cdz-http-programhash [--escaped | --base62] <component.wasm>
//!
//! - `--base62` (default): the canonical base62 `Hash` string (what the CAS + `Hash` Display use).
//! - `--escaped`: the 33 raw hash bytes rendered as `\xNN\xNN…`, ready to drop between the quotes of a
//!   Cadenza `b"…"` byte literal so the deploy step can `sed`-substitute it into the router source.

use cdz_platform::ProgramHash;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (escaped, path) = match args.as_slice() {
        [flag, path] if flag == "--escaped" => (true, path.clone()),
        [flag, path] if flag == "--base62" => (false, path.clone()),
        [path] if !path.starts_with("--") => (false, path.clone()),
        _ => {
            eprintln!("usage: cdz-http-programhash [--escaped | --base62] <component.wasm>");
            return ExitCode::from(2);
        }
    };

    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("cdz-http-programhash: cannot read {path}: {e}");
            return ExitCode::FAILURE;
        }
    };

    let hash = ProgramHash::of(&bytes);
    if escaped {
        // The 33 tagged-hash bytes as a `\xNN`-escaped body (no surrounding `b"…"`).
        let mut out = String::with_capacity(hash.hash().as_bytes().len() * 4);
        for b in hash.hash().as_bytes() {
            out.push_str(&format!("\\x{b:02x}"));
        }
        println!("{out}");
    } else {
        // The canonical base62 `Hash` string.
        println!("{}", hash.hash());
    }
    ExitCode::SUCCESS
}
