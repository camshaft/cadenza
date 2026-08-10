//! The `cdz-corpus` command surface, as an EMBEDDABLE clap `Args` group + a `run` entry point.
//!
//! Factored out of the standalone `bin/cdz-corpus.rs` so the unified `cdz` binary can MOUNT it as
//! `cdz corpus <records|migrate|check> …` (the same flatten pattern `cdz` uses for the syntax/compiler
//! CLIs) WITHOUT a second binary on the PATH. The standalone `cdz-corpus` bin is now a thin shim over
//! [`run`]; xtask (which shells out to the standalone bin) is unaffected. Both entry points share one
//! implementation and one `--help`. `run` takes the already-parsed [`CorpusArgs`] and returns an
//! `ExitCode`, threading a `prog` name so a diagnostic points at the command the user actually typed.

use std::io::Write;
use std::path::Path;
use std::process::ExitCode;

/// The arguments to `cdz corpus` / `cdz-corpus` — read and migrate the executable-semantics corpus.
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

/// Run a corpus command per `args`, returning the process exit code. `prog` names the tool in
/// diagnostics (`cdz-corpus` for the standalone bin, `cdz` for the unified one).
pub fn run(args: &CorpusArgs, prog: &str) -> ExitCode {
    let result = match &args.command {
        CorpusCmd::Records { files } => run_records(files),
        CorpusCmd::Migrate { write, files } => run_migrate(*write, files, prog),
        CorpusCmd::Check { files } => run_check(files),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("{prog}: {msg}");
            ExitCode::FAILURE
        }
    }
}

/// `records FILE…`: read each corpus file, normalize its cases, and emit the flat record stream to
/// stdout (records from all files concatenated, in file then case order). A `.md` file is read as a
/// migrated markdown corpus; any other extension is read as the s-expression corpus. Both paths emit
/// the identical record stream (see [`crate::read_markdown`]).
fn run_records(files: &[String]) -> Result<(), String> {
    let mut out = String::new();
    for path in files {
        let text = std::fs::read_to_string(path).map_err(|e| format!("reading {path}: {e}"))?;
        let is_markdown = Path::new(path).extension().is_some_and(|x| x == "md");
        // PLATFORM genre (operator seq358/seq359): a `(platform-case …)` file emits the platform record
        // stream, not the compiler-case one. Detected by genre so no new flag — a spec/platform/* file
        // just works and a semantics file is unaffected. The two genres are disjoint (a file is one or
        // the other). Both surfaces route: a .sexp by its leading head; a .md by the reconstructed
        // sexpr's head (its `platform-case` marker fence), so a migrated platform twin emits the SAME
        // platform stream as its .sexp source (without this, a .md would fall to read_markdown→read,
        // which only sees `(case …)`, silently dropping the platform case).
        if is_markdown {
            let reconstructed =
                crate::markdown::to_sexpr(&text).map_err(|e| format!("{path}: {e}"))?;
            if crate::is_platform_genre(&reconstructed) {
                let records =
                    crate::read_platform(&reconstructed).map_err(|e| format!("{path}: {e}"))?;
                out.push_str(&crate::render_platform(&records));
            } else {
                let records = crate::read_markdown(&text).map_err(|e| format!("{path}: {e}"))?;
                out.push_str(&crate::render(&records));
            }
            continue;
        }
        if crate::is_platform_genre(&text) {
            let records = crate::read_platform(&text).map_err(|e| format!("{path}: {e}"))?;
            out.push_str(&crate::render_platform(&records));
            continue;
        }
        let records = crate::read(&text).map_err(|e| format!("{path}: {e}"))?;
        out.push_str(&crate::render(&records));
    }
    std::io::stdout()
        .write_all(out.as_bytes())
        .map_err(|e| format!("writing stdout: {e}"))?;
    Ok(())
}

/// `migrate [--write] FILE…`: project each `.sexp` corpus file to markdown. With `--write`, the
/// output goes to `<stem>.md` beside the input; otherwise it prints to stdout. The document is
/// titled with the file's stem (e.g. `01-literals`).
fn run_migrate(write: bool, files: &[String], prog: &str) -> Result<(), String> {
    for path in files {
        let text = std::fs::read_to_string(path).map_err(|e| format!("reading {path}: {e}"))?;
        let title = Path::new(path).file_stem().and_then(|s| s.to_str());
        let md =
            crate::markdown::migrate_titled(&text, title).map_err(|e| format!("{path}: {e}"))?;
        if write {
            let out_path = Path::new(path).with_extension("md");
            std::fs::write(&out_path, md)
                .map_err(|e| format!("writing {}: {e}", out_path.display()))?;
            eprintln!("{prog}: wrote {}", out_path.display());
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
        match crate::markdown::check(&text) {
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
