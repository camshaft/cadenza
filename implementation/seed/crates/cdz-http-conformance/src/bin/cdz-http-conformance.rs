//! `cdz-http-conformance` — the conformance-harness driver binary (the `cdz-platform-itest` analogue). The
//! nix harness rig invokes it once per scenario:
//!
//!   cdz-http-conformance <run-spec.ml-binary-AST>
//!
//! with the environment set up by the rig:
//!   - `CDZ_CAS_HTTP_BIN` / `CDZ_HTTP_CONTROL_MOCK_BIN` / `CDZ_HTTP_GATEWAY_BIN` — the three SUT binaries.
//!   - `CDZ_HARNESS_PROGRAMS_DIR` — a directory holding one compiled `<program>.wasm` per program the rig
//!     built from `programs/` (the run-spec references programs by that name).
//!
//! It reads + parses the run-spec (binary-AST, `cdz rewrite`-resolved by the rig), builds the program
//! manifest from the programs dir, runs the scenario end to end, and reports the verdict as its exit code:
//! `0` = the run passed, `1` = a setup error or an assertion failure (the reason on stderr). All the logic
//! lives in the library ([`run_scenario`]); this is the thin argv/env → library adapter.

use cdz_http_conformance::programs::ProgramManifest;
use cdz_http_conformance::run::HarnessBins;
use cdz_http_conformance::scenario::run_scenario;
use cdz_http_conformance::spec::parse_run_spec;
use std::process::ExitCode;

/// The env var naming the directory of compiled `<program>.wasm` files the rig built.
const PROGRAMS_DIR_VAR: &str = "CDZ_HARNESS_PROGRAMS_DIR";

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("cdz-http-conformance: {e}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), String> {
    // `--parse-only <spec>`: decode the compiled run-spec + print its summary, then stop — no SUTs spawned.
    // This is how the nix rig proves a `cdz convert`-encoded run-spec round-trips through the parser (that the
    // ML surface + the parser agree on the value shape) without needing the SUT binaries.
    let mut args = std::env::args().skip(1);
    let mut parse_only = false;
    let mut spec_path = None;
    for arg in args.by_ref() {
        match arg.as_str() {
            "--parse-only" => parse_only = true,
            other => spec_path = Some(other.to_string()),
        }
    }
    let spec_path = spec_path.ok_or("usage: cdz-http-conformance [--parse-only] <run-spec.bin>")?;

    let spec_bytes =
        std::fs::read(&spec_path).map_err(|e| format!("reading run-spec {spec_path}: {e}"))?;
    let spec = parse_run_spec(&spec_bytes)
        .ok_or_else(|| format!("run-spec {spec_path} is not a valid binary-AST run value"))?;

    if parse_only {
        eprintln!(
            "cdz-http-conformance: parsed {spec_path}: {}",
            spec.summary()
        );
        return Ok(());
    }

    let bins = HarnessBins::resolve_from_env()?;

    let programs_dir = std::env::var_os(PROGRAMS_DIR_VAR).ok_or_else(|| {
        format!("{PROGRAMS_DIR_VAR} is not set (the nix rig provides the programs dir)")
    })?;
    let manifest = ProgramManifest::under_dir(
        programs_dir,
        spec.config.programs.iter().map(|p| p.program.clone()),
    );

    run_scenario(&bins, &spec, &manifest).await
}
