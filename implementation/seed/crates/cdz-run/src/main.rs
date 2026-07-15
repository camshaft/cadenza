//! `cdz-run` — the generic wasm-component runner CLI.
//!
//! `cdz-run <component.wasm> [--call <export>] [--arg <v> …] [--store <dir>]`
//!
//! Instantiates the component; if it records a required value-heap runtime, resolves that runtime BY
//! CONTENT ADDRESS from the store (the exact hash the component records — refusing if absent);
//! invokes the export (the sole function export by default); and prints the rendered result to
//! stdout. A trap or any error goes to stderr with a non-zero exit — clean to diff in tests.
//!
//! This bin is a thin shim: the command surface lives in `cdz_run::cli` (an embeddable clap `Args`
//! group + a `run` entry), so the unified `cdz` binary can mount the SAME code as `cdz run …` without
//! putting a second binary on the PATH. Both entry points share one implementation and one `--help`.

use std::process::ExitCode;

use clap::Parser;

/// Run a finished Cadenza wasm component and print its result.
#[derive(Parser)]
#[command(
    name = "cdz-run",
    about = "Run a wasm component: link, call an export, print the result."
)]
struct Cli {
    #[command(flatten)]
    run: cdz_run::cli::RunArgs,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    cdz_run::cli::run(&cli.run, "cdz-run")
}
