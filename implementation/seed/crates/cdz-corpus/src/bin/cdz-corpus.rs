//! `cdz-corpus` — read the executable-semantics corpus and migrate it to markdown.
//!
//! ```text
//! cdz-corpus records FILE…             # parse corpus cases → one normalized record per case
//! cdz-corpus migrate [--write] FILE…   # sexpr corpus → literate markdown (stdout, or `.md` beside)
//! cdz-corpus check FILE…               # verify a migration preserves the record stream
//! ```
//!
//! `records` emits the flat record stream the xtask gate consumes. `migrate` projects a `.sexp`
//! corpus file into the literate markdown format (one tagged `cdz` fence per DSL clause; see the
//! [`cdz_corpus::markdown`] module docs). `check` proves a migration is behaviour-preserving — the
//! reconstructed corpus's record stream is byte-identical to the original's. This bin is the only
//! place that touches the filesystem/stdio; the logic is the pure `cdz_corpus` library.

use clap::{Parser, Subcommand};
use std::io::Write;
use std::path::Path;
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "cdz-corpus",
    about = "Read and migrate the executable-semantics corpus."
)]
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
    /// Migrate corpus `.sexp` files to literate markdown.
    Migrate {
        /// Write `<file>.md` beside each input instead of printing to stdout.
        #[arg(long)]
        write: bool,
        /// Corpus `.sexp` files to migrate.
        #[arg(required = true)]
        files: Vec<String>,
    },
    /// Verify a migration preserves the record stream (byte-identical) for each file.
    Check {
        /// Corpus `.sexp` files to check.
        #[arg(required = true)]
        files: Vec<String>,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Cmd::Records { files } => run_records(&files),
        Cmd::Migrate { write, files } => run_migrate(write, &files),
        Cmd::Check { files } => run_check(&files),
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

/// `migrate [--write] FILE…`: project each `.sexp` corpus file to markdown. With `--write`, the
/// output goes to `<stem>.md` beside the input; otherwise it prints to stdout.
fn run_migrate(write: bool, files: &[String]) -> Result<(), String> {
    for path in files {
        let text = std::fs::read_to_string(path).map_err(|e| format!("reading {path}: {e}"))?;
        let md = cdz_corpus::markdown::migrate(&text).map_err(|e| format!("{path}: {e}"))?;
        if write {
            let out_path = Path::new(path).with_extension("md");
            std::fs::write(&out_path, md)
                .map_err(|e| format!("writing {}: {e}", out_path.display()))?;
            eprintln!("cdz-corpus: wrote {}", out_path.display());
        } else {
            std::io::stdout()
                .write_all(md.as_bytes())
                .map_err(|e| format!("writing stdout: {e}"))?;
        }
    }
    Ok(())
}

/// `check FILE…`: verify each migration is behaviour-preserving. Reports per-file OK/FAIL and exits
/// non-zero if any file's record stream changed.
fn run_check(files: &[String]) -> Result<(), String> {
    let mut failures = 0;
    for path in files {
        let text = std::fs::read_to_string(path).map_err(|e| format!("reading {path}: {e}"))?;
        match cdz_corpus::markdown::check(&text) {
            Ok(()) => println!("  ok    {path}"),
            Err(e) => {
                failures += 1;
                println!("  FAIL  {path}");
                for line in e.lines() {
                    println!("        {line}");
                }
            }
        }
    }
    if failures == 0 {
        println!(
            "\ncheck: all {} file(s) preserve the record stream",
            files.len()
        );
        Ok(())
    } else {
        Err(format!("{failures} file(s) changed the record stream"))
    }
}
