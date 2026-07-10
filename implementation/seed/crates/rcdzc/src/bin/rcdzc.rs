//! `rcdzc` — the command-line front over the pure compiler.
//!
//! Artifacts-in, artifacts-out: takes one or more NAMED input artifacts and a list of backend
//! targets (default `wasm`), runs the pure [`rcdzc::compile`], and writes each produced artifact to a
//! file. Diagnostics go to stderr; a nonzero exit means at least one error diagnostic.
//!
//! This bin is the HOST boundary — it owns the filesystem and argument parsing, the concerns the pure
//! core deliberately excludes so that core ports to the Cadenza self-host. It is intentionally thin:
//! all compilation logic lives behind `rcdzc::compile`. (The eventual query-program-driven entry will
//! replace the fixed pipeline here with a program the caller supplies; the pure core is unchanged.)
//!
//! Usage:
//!   rcdzc <input.ast>… [--target wasm]… [-o OUT]
//!   rcdzc kind:name=path.ast --target wasm -o build/
//!   rcdzc main.ast -o out/hello.wasm       # single output → an exact file path
//!
//! An input is `path`, `name=path`, or `kind:name=path`; kind defaults to `ast`, name to the file
//! stem. `-o` is a DIRECTORY into which each artifact is written as `<name>.<ext-for-kind>` — EXCEPT
//! when exactly one artifact is produced and `-o` does not name an existing directory, in which case
//! `-o` is the exact output FILE path. With no `-o`, artifacts are written to the current directory.

use clap::Parser;
use rcdzc::{compile, Artifact, Severity, Target};
use std::path::PathBuf;
use std::process::ExitCode;

/// Compile Cadenza binary-AST artifacts to one or more backend targets.
#[derive(Parser)]
#[command(name = "rcdzc", about = "The reference Cadenza → component compiler (artifacts in, artifacts out).")]
struct Cli {
    /// Input artifacts: `path`, `name=path`, or `kind:name=path` (kind defaults to `ast`).
    #[arg(required = true, value_name = "INPUT")]
    inputs: Vec<String>,

    /// Backend target(s) to emit; repeatable. Defaults to `wasm` when none is given.
    #[arg(long, short, value_name = "TARGET")]
    target: Vec<TargetArg>,

    /// Where to write output. A directory holding `<name>.<ext>` per artifact; or, when a single
    /// artifact is produced and this is not an existing directory, the exact output file path.
    /// Defaults to the current directory.
    #[arg(long, short, value_name = "OUT")]
    out: Option<PathBuf>,
}

/// A backend target, as a clap-parsed value (its own enum so clap validates the spelling and `--help`
/// lists the choices — extending to a second target is one variant here).
#[derive(Clone, Copy, clap::ValueEnum)]
enum TargetArg {
    /// A WebAssembly component.
    Wasm,
}

impl From<TargetArg> for Target {
    fn from(t: TargetArg) -> Target {
        match t {
            TargetArg::Wasm => Target::Wasm,
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    // Read each named input artifact from disk.
    let mut inputs: Vec<Artifact> = Vec::new();
    for spec in &cli.inputs {
        let parsed = parse_input_spec(spec);
        let bytes = match std::fs::read(&parsed.path) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("rcdzc: cannot read {}: {e}", parsed.path.display());
                return ExitCode::FAILURE;
            }
        };
        inputs.push(Artifact::new(parsed.kind, parsed.name, bytes));
    }

    // Compile to the requested targets (default: wasm).
    let targets: Vec<Target> = if cli.target.is_empty() {
        vec![Target::Wasm]
    } else {
        cli.target.iter().map(|&t| t.into()).collect()
    };
    let out = compile(&inputs, &targets);

    // Report diagnostics (stderr).
    for d in &out.diagnostics {
        let sev = match d.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        match &d.code {
            Some(code) => eprintln!("rcdzc: {sev} [{code}]: {}", d.message),
            None => eprintln!("rcdzc: {sev}: {}", d.message),
        }
    }

    // Decide whether `-o` names an exact output FILE (single artifact, not an existing directory) or
    // a DIRECTORY to write each `<name>.<ext>` into.
    let single_file_out: Option<&PathBuf> = match (&cli.out, out.artifacts.as_slice()) {
        (Some(p), [_one]) if !p.is_dir() => Some(p),
        _ => None,
    };

    // Write each produced artifact.
    for art in &out.artifacts {
        let path = match single_file_out {
            // Single artifact, `-o FILE`: write bytes to that exact path.
            Some(file) => file.clone(),
            // Otherwise: `<outdir>/<name>.<ext>`, outdir defaulting to the current directory.
            None => {
                let dir = cli.out.clone().unwrap_or_else(|| PathBuf::from("."));
                dir.join(format!("{}.{}", art.name, ext_for_kind(&art.kind)))
            }
        };
        if let Err(e) = std::fs::write(&path, &art.bytes) {
            eprintln!("rcdzc: cannot write {}: {e}", path.display());
            return ExitCode::FAILURE;
        }
        eprintln!("rcdzc: wrote {} ({} bytes)", path.display(), art.bytes.len());
    }

    if out.has_error() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// A parsed input spec: the artifact kind, its logical name, and the file to read it from.
struct InputSpec {
    kind: String,
    name: String,
    path: PathBuf,
}

/// Parse an input spec: `path`, `name=path`, or `kind:name=path`. Kind defaults to `ast`; name
/// defaults to the file stem.
fn parse_input_spec(spec: &str) -> InputSpec {
    // Split an optional `kind:` prefix — only when it looks like one (no path separator or `=` before
    // the colon), so a Windows-y or `name=path` spec is not mistaken for a kind.
    let (kind, rest) = match spec.split_once(':') {
        Some((k, r)) if !k.contains('/') && !k.contains('=') => (k.to_string(), r),
        _ => (Artifact::KIND_AST.to_string(), spec),
    };
    // Split an optional `name=` prefix.
    let (name, path) = match rest.split_once('=') {
        Some((n, p)) => (n.to_string(), PathBuf::from(p)),
        None => {
            let path = PathBuf::from(rest);
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("input").to_string();
            (stem, path)
        }
    };
    InputSpec { kind, name, path }
}

/// The file extension a produced artifact of the given kind is written with.
fn ext_for_kind(kind: &str) -> &str {
    match kind {
        "component" => "wasm",
        other => other,
    }
}
