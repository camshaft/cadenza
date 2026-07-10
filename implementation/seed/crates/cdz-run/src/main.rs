//! `cdz-run` — the generic wasm-component runner CLI.
//!
//! `cdz-run <component.wasm> [--call <export>] [--arg <v> …] [--store <dir>]`
//!
//! Instantiates the component; if it records a required value-heap runtime, resolves that runtime BY
//! CONTENT ADDRESS from the store (the exact hash the component records — refusing if absent);
//! invokes the export (the sole function export by default); and prints the rendered result to
//! stdout. A trap or any error goes to stderr with a non-zero exit — clean to diff in tests.

use std::path::PathBuf;
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

    /// Override the value-heap runtime `.wasm` (escape hatch). Normally the runtime is resolved BY
    /// CONTENT ADDRESS from the store: the exact hash the component records must be present. This
    /// bypasses that lookup — use for local runtime debugging, not conformance.
    #[arg(long)]
    runtime: Option<PathBuf>,

    /// The content-addressed store to resolve the runtime from (`<store>/<hash>.wasm`).
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

    // Resolve the value-heap runtime ONLY if the component records one — a scalar/const component
    // imports nothing and needs no runtime, so a missing store is not an error there. When it does,
    // resolve BY CONTENT ADDRESS: the exact hash the component records must be in the store.
    let runtime = match cdz_run::required_runtime(&component_bytes)? {
        Some(req) => Some(resolve_runtime(cli, &req)?),
        None => None,
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

/// Resolve the value-heap runtime bytes the component requires, BY CONTENT ADDRESS. The component
/// records the exact hash it was emitted against (component-abi.md §The Emitted Component Records Its
/// Required Runtime); the host locates `<store>/<hash>.wasm` and REFUSES to run if that exact hash is
/// absent — never substituting a different runtime (§The Host Resolves The Runtime By Content
/// Address). `--runtime <path>` is a debugging escape hatch that bypasses the store lookup.
fn resolve_runtime(cli: &Cli, req: &cdz_run::RuntimeReq) -> anyhow::Result<Vec<u8>> {
    if let Some(path) = &cli.runtime {
        return std::fs::read(path).map_err(|e| anyhow::anyhow!("read --runtime {}: {e}", path.display()));
    }

    if req.hash.is_empty() {
        return Err(anyhow::anyhow!(
            "component imports the value-heap runtime but records no content address to resolve it by \
             (an unpinned runtime import); pass --runtime <path> explicitly"
        ));
    }

    let store = cli.store.clone().unwrap_or_else(default_store);
    let path = store.join(format!("{}.wasm", req.hash));
    if !path.exists() {
        return Err(anyhow::anyhow!(
            "no runtime of content address {} in the store at {} — refusing to run rather than \
             substitute a different runtime (build the required runtime with `cargo xtask build`)",
            req.hash,
            store.display()
        ));
    }
    let bytes = std::fs::read(&path)
        .map_err(|e| anyhow::anyhow!("read stored runtime {}: {e}", path.display()))?;

    // Verify the stored bytes actually hash to the required address — a store entry misnamed or
    // corrupted would otherwise be a silent substitution, exactly what content addressing prevents.
    let actual = content_address(&bytes);
    if actual != req.hash {
        return Err(anyhow::anyhow!(
            "store entry {} has content address {actual}, not the required {} — refusing",
            path.display(),
            req.hash
        ));
    }
    Ok(bytes)
}

/// SHA-256 of `bytes`, lowercase hex — the store's content-address function (matches xtask).
fn content_address(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    let mut s = String::with_capacity(64);
    for b in digest {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// The default content-addressed store: `<repo>/target/cadenza-store`, resolved from this crate's
/// manifest location (crate lives at `<repo>/implementation/seed/crates/cdz-run`).
fn default_store() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // <repo>/implementation/seed/crates/cdz-run → up 4 → <repo>
    let repo = manifest.ancestors().nth(4).unwrap_or(&manifest).to_path_buf();
    repo.join("target/cadenza-store")
}
