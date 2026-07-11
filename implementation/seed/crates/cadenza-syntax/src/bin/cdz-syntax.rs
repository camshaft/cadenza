//! `cdz-syntax` — convert a Cadenza program between its surfaces, and read the corpus.
//!
//! Surfaces: `binary`, `sexpr`, `ml`, plus two output-only views of the binary AST — `debug` (an
//! indented tree) and `flat` (the arenas dumped literally: leaf pool + structure vector + root).
//!
//! ```text
//! cdz-syntax convert [--from FMT] [--to FMT] [--width N] [FILE]
//! cdz-syntax corpus FILE…            # parse corpus cases → one normalized record per case
//! ```
//!
//! `convert`'s `--from`/`--to` are inferred from the FILE extension when omitted (`.cdz`/`.ml` → ml,
//! `.sexp`/`.sexpr` → sexpr, `.bin`/`.cdzb` → binary); `--to` defaults to `sexpr`. With no FILE (or
//! `-`), input is read from stdin (then `--from` is required). Output goes to stdout. This bin is
//! the only place in the crate that touches the filesystem/stdio; conversion is the pure
//! `cadenza_syntax::convert` module and corpus reading the `cadenza_syntax::corpus` module.

use cadenza_syntax::convert::{self, Format, Options};
use cadenza_syntax::corpus;
use clap::{Args, Parser, Subcommand, ValueEnum};
use std::io::{Read, Write};
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "cdz-syntax",
    about = "Convert a Cadenza program between its surfaces."
)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Convert a program between surfaces (reads FILE or stdin, writes stdout).
    Convert(ConvertArgs),
    /// Parse corpus files and emit one normalized record per case to stdout.
    Corpus {
        /// Corpus `.sexp` files to read.
        #[arg(required = true)]
        files: Vec<String>,
    },
}

#[derive(Args)]
struct ConvertArgs {
    /// Input format. Inferred from FILE's extension when omitted; required when reading stdin.
    #[arg(short, long, value_enum)]
    from: Option<Fmt>,

    /// Output format. Inferred from FILE's extension when omitted, else defaults to `sexpr`.
    #[arg(short, long, value_enum)]
    to: Option<Fmt>,

    /// Target line width for `ml` output.
    #[arg(short, long, default_value_t = Options::default().width)]
    width: usize,

    /// Input file; omit or use `-` to read stdin.
    file: Option<String>,
}

/// The surface formats, as a clap `ValueEnum`. Mirrors [`Format`]; kept in the bin so the library
/// takes no CLI dependency.
#[derive(Clone, Copy, ValueEnum)]
enum Fmt {
    Binary,
    Sexpr,
    Ml,
    Debug,
    Flat,
}

impl From<Fmt> for Format {
    fn from(f: Fmt) -> Format {
        match f {
            Fmt::Binary => Format::Binary,
            Fmt::Sexpr => Format::Sexpr,
            Fmt::Ml => Format::Ml,
            Fmt::Debug => Format::Debug,
            Fmt::Flat => Format::Flat,
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Cmd::Convert(args) => run_convert(&args),
        Cmd::Corpus { files } => run_corpus(&files),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("cdz-syntax: {msg}");
            ExitCode::FAILURE
        }
    }
}

/// Read a program, convert it, write it.
fn run_convert(args: &ConvertArgs) -> Result<(), String> {
    let from = resolve_from(args.from, args.file.as_deref())?;
    let to = resolve_to(args.to, args.file.as_deref());

    let input = read_input(args.file.as_deref())?;
    let opts = Options { width: args.width };
    let output = convert::convert_with(&input, from, to, opts).map_err(|e| e.to_string())?;

    std::io::stdout()
        .write_all(&output)
        .map_err(|e| format!("writing stdout: {e}"))?;
    // A trailing newline after text output so the terminal prompt lands on its own line; binary
    // output is emitted exactly.
    if to != Format::Binary {
        let _ = std::io::stdout().write_all(b"\n");
    }
    Ok(())
}

/// Resolve the input format: explicit `--from`, else inferred from the file extension. Reading stdin
/// (no file, or `-`) with no `--from` and no inferable extension is an error.
fn resolve_from(from: Option<Fmt>, file: Option<&str>) -> Result<Format, String> {
    if let Some(f) = from {
        return Ok(f.into());
    }
    match file {
        Some(path) if path != "-" => Format::from_extension(path).ok_or_else(|| {
            format!("cannot infer input format from `{path}`; pass --from (binary|sexpr|ml)")
        }),
        _ => Err("reading stdin requires --from (binary|sexpr|ml)".to_string()),
    }
}

/// Resolve the output format: explicit `--to`, else inferred from the file extension, else `sexpr`.
fn resolve_to(to: Option<Fmt>, file: Option<&str>) -> Format {
    if let Some(t) = to {
        return t.into();
    }
    file.filter(|p| *p != "-")
        .and_then(Format::from_extension)
        .unwrap_or(Format::Sexpr)
}

/// `corpus FILE…`: read each corpus file, normalize its cases, and emit the flat record stream to
/// stdout (records from all files concatenated, in file then case order).
fn run_corpus(files: &[String]) -> Result<(), String> {
    let mut out = String::new();
    for path in files {
        let text = std::fs::read_to_string(path).map_err(|e| format!("reading {path}: {e}"))?;
        let records = corpus::read(&text).map_err(|e| format!("{path}: {e}"))?;
        out.push_str(&corpus::render(&records));
    }
    std::io::stdout()
        .write_all(out.as_bytes())
        .map_err(|e| format!("writing stdout: {e}"))?;
    Ok(())
}

/// Read the whole input from `file` (or stdin when `None` or `"-"`).
fn read_input(file: Option<&str>) -> Result<Vec<u8>, String> {
    match file {
        None | Some("-") => {
            let mut buf = Vec::new();
            std::io::stdin()
                .read_to_end(&mut buf)
                .map_err(|e| format!("reading stdin: {e}"))?;
            Ok(buf)
        }
        Some(path) => std::fs::read(path).map_err(|e| format!("reading {path}: {e}")),
    }
}
