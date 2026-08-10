//! The `cdz-corpus` command surface, as an EMBEDDABLE clap `Args` group + a `run` entry point.
//!
//! Factored out of the standalone `bin/cdz-corpus.rs` so the unified `cdz` binary can MOUNT it as
//! `cdz corpus records …` (the same flatten pattern `cdz` uses for the syntax/compiler
//! CLIs) WITHOUT a second binary on the PATH. The standalone `cdz-corpus` bin is now a thin shim over
//! [`run`]; xtask (which shells out to the standalone bin) is unaffected. Both entry points share one
//! implementation and one `--help`. `run` takes the already-parsed [`CorpusArgs`] and returns an
//! `ExitCode`, threading a `prog` name so a diagnostic points at the command the user actually typed.

use std::io::Write;
use std::process::ExitCode;

/// The arguments to `cdz corpus` / `cdz-corpus` — read the executable-semantics corpus.
#[derive(clap::Args)]
pub struct CorpusArgs {
    #[command(subcommand)]
    command: CorpusCmd,
}

#[derive(clap::Subcommand)]
enum CorpusCmd {
    /// Parse corpus files and emit one normalized record per case to stdout.
    Records {
        /// Corpus `.sexp` files to read.
        #[arg(required = true)]
        files: Vec<String>,
    },
}

/// Run a corpus command per `args`, returning the process exit code. `prog` names the tool in
/// diagnostics (`cdz-corpus` for the standalone bin, `cdz` for the unified one).
pub fn run(args: &CorpusArgs, prog: &str) -> ExitCode {
    let result = match &args.command {
        CorpusCmd::Records { files } => run_records(files),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("{prog}: {msg}");
            ExitCode::FAILURE
        }
    }
}

/// `records FILE…`: read each corpus `.sexp` file, normalize its cases, and emit the flat record stream
/// to stdout (records from all files concatenated, in file then case order). A `(platform-case …)` file
/// emits the platform record stream; any other file emits the compiler-case stream. The genre is
/// auto-detected by the leading form's head — the two genres are disjoint (a file is one or the other).
fn run_records(files: &[String]) -> Result<(), String> {
    let mut out = String::new();
    for path in files {
        let text = std::fs::read_to_string(path).map_err(|e| format!("reading {path}: {e}"))?;
        if crate::is_platform_genre(&text) {
            let records = crate::read_platform(&text).map_err(|e| format!("{path}: {e}"))?;
            out.push_str(&crate::render_platform(&records));
        } else {
            let records = crate::read(&text).map_err(|e| format!("{path}: {e}"))?;
            out.push_str(&crate::render(&records));
        }
    }
    std::io::stdout()
        .write_all(out.as_bytes())
        .map_err(|e| format!("writing stdout: {e}"))?;
    Ok(())
}
