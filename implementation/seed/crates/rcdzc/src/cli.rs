//! The `rcdzc` compile command surface, factored into the library so BOTH the standalone `rcdzc` bin
//! and the unified `cdz` bin drive ONE implementation. The thin bins call [`parse_and_run`], or embed
//! [`CompileArgs`] as a subcommand and call [`run`].
//!
//! Artifacts-in, artifacts-out: takes one or more NAMED input artifacts and a list of backend
//! targets (default `wasm`), runs the pure [`crate::compile`], and writes each produced artifact to a
//! file. Diagnostics go to stderr; a nonzero exit means at least one error diagnostic.
//!
//! This is the HOST boundary — it owns the filesystem and argument parsing, the concerns the pure
//! core deliberately excludes so that core ports to the Cadenza self-host. It is intentionally thin:
//! all compilation logic lives behind `crate::compile`.
//!
//! Usage:
//!   rcdzc <input.ast>… [--target wasm]… [-o OUT]
//!   rcdzc kind:name=path.ast --target wasm -o build/
//!   rcdzc main.ast -o out/hello.wasm       # single output → an exact file path
//!   rcdzc - -o -                           # stdin → stdout: composes in a pipe (single artifact)
//!
//! An input is `path`, `name=path`, or `kind:name=path`; kind defaults to `ast`, name to the file
//! stem. `-o` is a DIRECTORY into which each artifact is written as `<name>.<ext-for-kind>` — EXCEPT
//! when exactly one artifact is produced and `-o` does not name an existing directory, in which case
//! `-o` is the exact output FILE path. With no `-o`, artifacts are written to the current directory.

use crate::{Artifact, Severity, Target, compile};
use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;

/// Compile Cadenza binary-AST artifacts to one or more backend targets. `#[command(...)]` here names
/// the standalone `rcdzc` bin; embedded as a `cdz compile` subcommand the outer bin supplies the name.
#[derive(Parser)]
#[command(
    name = "rcdzc",
    about = "The reference Cadenza → component compiler (artifacts in, artifacts out)."
)]
pub struct CompileArgs {
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
    /// A WebAssembly component carrying EMBEDDED DWARF debug info (Mode E) — steps through Cadenza
    /// source and inspects scalar arguments in GDB/LLDB/Chrome. Needs a `spans` input (supply it as
    /// `spans:NAME=path.spans`); without it the compile declines. Debug sections are inert + strippable.
    WasmDebug,
    /// A DETACHED DWARF sidecar (Mode S) — a `<name>.dwarf` file carrying only the debug sections, for a
    /// debugger to load alongside a lean (undecorated) component. Also needs a `spans` input.
    Dwarf,
    /// Rust source (a `.rs` module linking into a Rust codebase, no FFI).
    Rust,
    /// Rust source in ASYNC, GAS-METERED form: every fn is `async` and threads `env: &mut impl CdzEnv`,
    /// awaiting `env.consume(1)` at entry so the host meters fuel and can yield cooperatively.
    RustAsync,
}

impl From<TargetArg> for Target {
    fn from(t: TargetArg) -> Target {
        match t {
            TargetArg::Wasm => Target::Wasm,
            TargetArg::WasmDebug => Target::WasmDebug,
            TargetArg::Dwarf => Target::Dwarf,
            TargetArg::Rust => Target::Rust,
            TargetArg::RustAsync => Target::RustAsync,
        }
    }
}

impl CompileArgs {
    /// The raw input specs (`path` / `name=path` / `kind:name=path`) — read by a wrapping driver (the
    /// `cdz` bin) that may pre-process SOURCE-file inputs into artifacts before compiling.
    pub fn input_specs(&self) -> &[String] {
        &self.inputs
    }

    /// The explicit backend targets requested on the command line (empty when none was given — the
    /// caller applies the default, see [`run_prepared`]).
    pub fn targets(&self) -> Vec<Target> {
        self.target.iter().map(|&t| t.into()).collect()
    }

    /// Where output is written (`-o`), if given.
    pub fn out_path(&self) -> Option<PathBuf> {
        self.out.clone()
    }
}

/// Parse the whole `rcdzc` CLI from `std::env::args` and run it — the standalone bin's `main`.
pub fn parse_and_run() -> ExitCode {
    run(CompileArgs::parse(), "rcdzc")
}

/// Run one compile command, reporting tool-level errors under `prog` (the invoking binary's name).
/// This is the host boundary: filesystem + args + the trace sink.
pub fn run(cli: CompileArgs, prog: &str) -> ExitCode {
    // Install the trace sink at the HOST boundary (the lib core only EMITS events). Output goes to
    // stderr, filtered by `CDZ_LOG` (e.g. `CDZ_LOG=rcdzc::infer=trace` to watch only inference, or
    // `CDZ_LOG=rcdzc=trace` for every decision). With no `CDZ_LOG` set, nothing is installed and the
    // `trace!` sites compile to no-ops at runtime — a normal run pays nothing. The subscriber living
    // only in the binary is the right split: the lib is instrumentation-only, the bin decides the sink.
    //
    // A TOOL-PRIVATE env var (not the shared `RUST_LOG`): the pipeline runs as `cargo xtask` driving
    // `cdz-syntax | rcdzc | cdz-run`, and `RUST_LOG` would fan out to cargo, wasmtime, and every other
    // `tracing`/`env_logger` consumer in those processes. `CDZ_LOG` is read only by this subscriber, so
    // `CDZ_LOG=rcdzc=trace` shows the compiler's decisions and nothing else's noise.
    if std::env::var("CDZ_LOG").is_ok() {
        use tracing_subscriber::{EnvFilter, fmt};
        let _ = fmt()
            .with_env_filter(EnvFilter::from_env("CDZ_LOG"))
            .with_writer(std::io::stderr)
            // Show each event's source file:line — the trace sites map decisions straight back to
            // the code that made them, which is the whole point of a debugging trace. `tracing`
            // captures the call-site location in the event metadata (no cost when no subscriber is
            // installed), so this is a pure formatting choice here at the host boundary.
            .with_file(true)
            .with_line_number(true)
            // Drop the timestamp and level: every event here is a `TRACE` and the wall-clock time is
            // noise for reading a compile's decision flow — the file:line + target + message is what
            // matters. (The target still prefixes each line, so the module is clear.)
            .without_time()
            .with_level(false)
            .try_init();
    }

    // Read each named input artifact — from disk, or from stdin when the path is `-` (so the bin
    // composes in a pipe: `… | rcdzc - -o -`). A `-` input takes the kind/name from its spec, both
    // defaulting to `ast`/`main` since a piped artifact has no file stem to name it after.
    let mut inputs: Vec<Artifact> = Vec::new();
    for spec in &cli.inputs {
        let parsed = parse_input_spec(spec);
        let bytes = if parsed.path.as_os_str() == "-" {
            let mut buf = Vec::new();
            if let Err(e) = std::io::Read::read_to_end(&mut std::io::stdin(), &mut buf) {
                eprintln!("{prog}: cannot read stdin: {e}");
                return ExitCode::FAILURE;
            }
            buf
        } else {
            match std::fs::read(&parsed.path) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("{prog}: cannot read {}: {e}", parsed.path.display());
                    return ExitCode::FAILURE;
                }
            }
        };
        inputs.push(Artifact::new(parsed.kind, parsed.name, bytes));
    }

    run_prepared(inputs, &cli.targets(), cli.out, prog)
}

/// Compile a set of ALREADY-BUILT input artifacts to the requested targets and write the outputs — the
/// host boundary's compile+report+write tail, exposed so a wrapping driver (the `cdz` bin) can
/// pre-build artifacts from SOURCE files (parsing them in-process with its front-end, injecting the
/// `ast` + `spans` artifacts) and reuse the identical output-writing behavior. `targets` is the
/// explicit `--target` list (empty ⇒ apply the default here). `out` is the `-o` destination.
pub fn run_prepared(
    inputs: Vec<Artifact>,
    targets: &[Target],
    out: Option<PathBuf>,
    prog: &str,
) -> ExitCode {
    // Apply the target default here (so both `run` and an external driver get the same rule): explicit
    // targets win; else `[Wasm]` UNLESS a `sidecar` input drives the run (then its Emit requests name
    // the targets, and a default `wasm` would force an unwanted component for a query-only sidecar).
    let has_sidecar = inputs
        .iter()
        .any(|a| a.kind == crate::sidecar::KIND_SIDECAR);
    let targets: Vec<Target> = if !targets.is_empty() {
        targets.to_vec()
    } else if has_sidecar {
        Vec::new()
    } else {
        vec![Target::Wasm]
    };
    // Run the compile on a worker thread with a stack sized to reach the recursive-descent depth
    // guard, so pathologically deep input DECLINES (the guard trips) rather than overflowing the
    // native stack and aborting — the `decline-don't-crash` contract, made independent of whatever
    // stack the ambient thread happens to have. See `rcdzc::host`.
    let out_dest = out;
    let cli_out = &out_dest;
    let out = crate::run_with_compiler_stack(|| compile(&inputs, &targets));

    // Report diagnostics (stderr).
    for d in &out.diagnostics {
        let sev = match d.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        // The node index (if any) rides along so a caller holding the source's span table can map the
        // diagnostic to a text region — the compiler reports node IDENTITY, never a source position.
        let at = match d.node {
            Some(n) => format!(" (node {n})"),
            None => String::new(),
        };
        match &d.code {
            Some(code) => eprintln!("{prog}: {sev} [{code}]{at}: {}", d.message),
            None => eprintln!("{prog}: {sev}{at}: {}", d.message),
        }
    }

    // `-o -`: write the single produced artifact's bytes to stdout (so the bin composes in a pipe:
    // `… | rcdzc - -o - | cdz-run`). Only meaningful for a single artifact — a multi-artifact build
    // has no one stream to write, so that is an error rather than an ambiguous concatenation.
    if cli_out.as_deref().map(|p| p.as_os_str()) == Some(std::ffi::OsStr::new("-")) {
        match out.artifacts.as_slice() {
            [art] => {
                if let Err(e) = std::io::Write::write_all(&mut std::io::stdout(), &art.bytes) {
                    eprintln!("{prog}: cannot write stdout: {e}");
                    return ExitCode::FAILURE;
                }
            }
            [] => {} // no artifact (errors already reported); fall through to the exit status.
            many => {
                eprintln!(
                    "cdz: `-o -` writes ONE artifact to stdout, but {} were produced",
                    many.len()
                );
                return ExitCode::FAILURE;
            }
        }
        return if out.has_error() {
            ExitCode::FAILURE
        } else {
            ExitCode::SUCCESS
        };
    }

    // Decide whether `-o` names an exact output FILE (single artifact, not an existing directory) or
    // a DIRECTORY to write each `<name>.<ext>` into.
    let single_file_out: Option<&PathBuf> = match (cli_out, out.artifacts.as_slice()) {
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
                let dir = cli_out.clone().unwrap_or_else(|| PathBuf::from("."));
                dir.join(format!("{}.{}", art.name, ext_for_kind(&art.kind)))
            }
        };
        if let Err(e) = std::fs::write(&path, &art.bytes) {
            eprintln!("{prog}: cannot write {}: {e}", path.display());
            return ExitCode::FAILURE;
        }
        eprintln!("cdz: wrote {} ({} bytes)", path.display(), art.bytes.len());
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
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("input")
                .to_string();
            (stem, path)
        }
    };
    InputSpec { kind, name, path }
}

/// The file extension a produced artifact of the given kind is written with.
fn ext_for_kind(kind: &str) -> &str {
    match kind {
        "component" => "wasm",
        "rust" => "rs",
        // A detached DWARF sidecar (Mode S) is a bare `.wasm`-format core module of debug sections;
        // written with a `.dwarf` extension so it is distinct from the runnable `<name>.wasm`.
        "dwarf" => "dwarf",
        // Sidecar QUERY results are UTF-8 text (a rendered type, a newline-separated node-id list) —
        // written with a `.txt` extension. A `sidecar` INPUT is read generically as `kind:name=path`,
        // so no case is needed for it here (this maps only produced OUTPUT kinds).
        "type-info" | "uses" => "txt",
        other => other,
    }
}
