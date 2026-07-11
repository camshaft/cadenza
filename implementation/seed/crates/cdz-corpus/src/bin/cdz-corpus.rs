//! `cdz-corpus` — read the executable-semantics corpus.
//!
//! ```text
//! cdz-corpus records FILE…            # parse corpus cases → one normalized record per case
//! ```
//!
//! `records` parses each `spec/semantics/*.sexp` file, normalizes every case's `input` to the
//! runnable export shape, and emits the flat record stream (see [`cdz_corpus`] module docs) to
//! stdout — the interface the xtask gate consumes. This bin is the only place that touches the
//! filesystem/stdio; parsing + normalization is the pure `cdz_corpus` library.
//!
//! The migration tooling (`.sexp` → markdown) will land here as further subcommands.

use clap::{Parser, Subcommand};
use std::io::Write;
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "cdz-corpus", about = "Read the executable-semantics corpus.")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Parse corpus files and emit one normalized record per case to stdout.
    Records {
        /// Corpus `.sexp` files to read.
        #[arg(required = true)]
        files: Vec<String>,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Cmd::Records { files } => run_records(&files),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("cdz-corpus: {msg}");
            ExitCode::FAILURE
        }
    }
}

/// `records FILE…`: read each corpus file, normalize its cases, and emit the flat record stream to
/// stdout (records from all files concatenated, in file then case order).
fn run_records(files: &[String]) -> Result<(), String> {
    let mut out = String::new();
    for path in files {
        let text = std::fs::read_to_string(path).map_err(|e| format!("reading {path}: {e}"))?;
        let records = cdz_corpus::read(&text).map_err(|e| format!("{path}: {e}"))?;
        out.push_str(&cdz_corpus::render(&records));
    }
    std::io::stdout()
        .write_all(out.as_bytes())
        .map_err(|e| format!("writing stdout: {e}"))?;
    Ok(())
}
