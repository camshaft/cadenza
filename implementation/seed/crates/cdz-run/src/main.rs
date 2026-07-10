//! `cdz-run` — the generic wasm-component runner CLI.
//!
//! `cdz-run <component.wasm> [--call <export>] [--arg <v> …] [--runtime <wasm>]`
//!
//! Instantiates the component, composes the value-heap runtime when the component imports it,
//! invokes the export (the sole function export by default), and prints the rendered result to
//! stdout. A trap or any error goes to stderr with a non-zero exit — clean to diff in tests.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;

/// Run a finished Cadenza wasm component and print its result.
#[derive(Parser)]
#[command(name = "cdz-run", about = "Run a wasm component: link, call an export, print the result.")]
struct Cli {
    /// The component `.wasm` to run.
    component: PathBuf,

    /// The export to call. Defaults to the component's sole function export.
    #[arg(long)]
    call: Option<String>,

    /// An argument to the export, repeatable; coerced to the export's declared parameter type.
    /// `allow_hyphen_values` so a negative number (`--arg -4`) is taken as the value, not a flag.
    #[arg(long = "arg", value_name = "VALUE", allow_hyphen_values = true)]
    args: Vec<String>,

    /// The value-heap runtime `.wasm` to compose. Required only if the component imports
    /// `cadenza:runtime/heap`; if omitted, resolved from the content-addressed store.
    #[arg(long)]
    runtime: Option<PathBuf>,

    /// The content-addressed store to resolve the runtime from when `--runtime` is not given.
    /// [default: <repo>/target/cadenza-store]
    #[arg(long)]
    store: Option<PathBuf>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match real_main(&cli) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("cdz-run: {e:#}");
            ExitCode::from(2)
        }
    }
}

fn real_main(cli: &Cli) -> anyhow::Result<ExitCode> {
    let component_bytes = std::fs::read(&cli.component)
        .map_err(|e| anyhow::anyhow!("read component {}: {e}", cli.component.display()))?;

    // Resolve the value-heap runtime ONLY if the component needs it — a scalar/const component
    // imports nothing and needs no runtime, so a missing store is not an error there.
    let runtime = if cdz_run::needs_runtime(&component_bytes)? {
        Some(resolve_runtime(cli)?)
    } else {
        None
    };

    let opts = cdz_run::RunOpts {
        export: cli.call.clone(),
        args: cli.args.clone(),
        runtime,
    };

    match cdz_run::run(&component_bytes, &opts)? {
        cdz_run::Outcome::Value(text) => {
            println!("{text}");
            Ok(ExitCode::SUCCESS)
        }
        cdz_run::Outcome::Trap(msg) => {
            eprintln!("cdz-run: trap: {msg}");
            Ok(ExitCode::FAILURE)
        }
    }
}

/// Resolve the value-heap runtime bytes: `--runtime <path>` if given, else the runtime recorded by
/// the content-addressed store (`<store>/runtime.toml` → `<store>/<hash>.wasm`).
fn resolve_runtime(cli: &Cli) -> anyhow::Result<Vec<u8>> {
    if let Some(path) = &cli.runtime {
        return std::fs::read(path).map_err(|e| anyhow::anyhow!("read --runtime {}: {e}", path.display()));
    }
    let store = cli.store.clone().unwrap_or_else(default_store);
    let path = runtime_from_store(&store).ok_or_else(|| {
        anyhow::anyhow!(
            "component imports the value-heap runtime, but no --runtime was given and no runtime was \
             found in the store at {} (build it with `cargo xtask build`)",
            store.display()
        )
    })?;
    std::fs::read(&path).map_err(|e| anyhow::anyhow!("read stored runtime {}: {e}", path.display()))
}

/// The default content-addressed store: `<repo>/target/cadenza-store`, resolved from this crate's
/// manifest location (crate lives at `<repo>/implementation/seed/crates/cdz-run`).
fn default_store() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // <repo>/implementation/seed/crates/cdz-run → up 4 → <repo>
    let repo = manifest
        .ancestors()
        .nth(4)
        .unwrap_or(&manifest)
        .to_path_buf();
    repo.join("target/cadenza-store")
}

/// The runtime `.wasm` a store records: prefer the hash in `runtime.toml`, else the sole `.wasm`.
fn runtime_from_store(store: &Path) -> Option<PathBuf> {
    // `runtime.toml` records `runtime = "<hash>"`; the file is `<store>/<hash>.wasm`.
    if let Ok(toml) = std::fs::read_to_string(store.join("runtime.toml")) {
        if let Some(hash) = toml
            .lines()
            .find_map(|l| l.trim().strip_prefix("runtime = ").map(str::trim))
            .map(|v| v.trim_matches('"').to_string())
        {
            let candidate = store.join(format!("{hash}.wasm"));
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    // Fallback: the sole `.wasm` in the store, if there is exactly one.
    let mut wasm = None;
    for entry in std::fs::read_dir(store).ok()?.flatten() {
        let p = entry.path();
        if p.extension().is_some_and(|e| e == "wasm") {
            if wasm.is_some() {
                return None; // ambiguous
            }
            wasm = Some(p);
        }
    }
    wasm
}
