//! The `cdz-run` command surface, as an EMBEDDABLE clap `Args` group + a `run` entry point.
//!
//! Factored out of the standalone `main.rs` so the unified `cdz` binary can MOUNT it as `cdz run …`
//! (the same flatten pattern `cdz` uses for the syntax/compiler CLIs) WITHOUT a second binary on the
//! PATH. The standalone `cdz-run` bin is now a thin shim over [`run`]; `cdz run` calls the same code.
//! `run` takes the already-parsed [`RunArgs`] and returns an `ExitCode`, so both entry points share one
//! implementation and one `--help`.

use std::path::PathBuf;
use std::process::ExitCode;

use crate::{
    HostResponse, Outcome, Peer, RunOpts, required_runtime, run_capturing, run_with_peers,
};

/// The arguments to `cdz run` / `cdz-run` — run a finished Cadenza wasm component and print its result.
#[derive(clap::Args)]
pub struct RunArgs {
    /// The component `.wasm` to run, or `-` to read it from stdin (so it composes in a pipe:
    /// `cdz compile - -o - | cdz run -`).
    pub component: PathBuf,

    /// The export to call. Defaults to the component's sole function export.
    #[arg(long)]
    pub call: Option<String>,

    /// An argument to the export, repeatable; coerced to the export's declared parameter type.
    /// `allow_hyphen_values` so a negative number (`--arg -4`) is taken as the value, not a flag.
    #[arg(long = "arg", value_name = "VALUE", allow_hyphen_values = true)]
    pub args: Vec<String>,

    /// Override the value-heap runtime `.wasm` (escape hatch). Normally the runtime is resolved BY
    /// CONTENT ADDRESS from the store: the exact hash the component records must be present. This
    /// bypasses that lookup — use for local runtime debugging, not conformance.
    #[arg(long)]
    pub runtime: Option<PathBuf>,

    /// The content-addressed store to resolve the runtime from (`<store>/<hash>.wasm`).
    /// [default: <repo>/target/cadenza-store]
    #[arg(long)]
    pub store: Option<PathBuf>,

    /// A recorded HOST-CALL RESPONSE, repeatable, in call order — `op=value` (e.g.
    /// `--host-response ask.ask=10`). A program that delegates an effect to the host consumes these in
    /// order when it performs an operation. The value is coerced to the operation's boundary result type.
    #[arg(long = "host-response", value_name = "OP=VALUE")]
    pub host_responses: Vec<String>,

    /// A PEER Cadenza component to compose across the live boundary (X4b), repeatable —
    /// `<interface>=<path>` (e.g. `--peer cadenza:math/api=math.wasm`). The component being run is the
    /// CONSUMER; each peer's exported interface is bound into the consumer's like-named `(extern …)`
    /// import, all sharing one value-heap runtime instance (component-abi.md §Cross-Component Value
    /// Exchange). Absent (the common case) → an ordinary single-component run.
    #[arg(long = "peer", value_name = "INTERFACE=PATH")]
    pub peers: Vec<String>,
}

/// Run a component per `args`, printing the value to stdout (host calls to stderr) and returning the
/// process exit code. `prog` names the tool in diagnostics (`cdz-run` for the standalone bin, `cdz` for
/// the unified one), so an error message points at the command the user actually typed.
pub fn run(args: &RunArgs, prog: &str) -> ExitCode {
    match real_run(args) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("{prog}: {e:#}");
            ExitCode::from(2)
        }
    }
}

fn real_run(cli: &RunArgs) -> anyhow::Result<ExitCode> {
    // The component bytes: from a file, or from stdin when the path is `-`.
    let component_bytes = if cli.component.as_os_str() == "-" {
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut std::io::stdin(), &mut buf)
            .map_err(|e| anyhow::anyhow!("read component from stdin: {e}"))?;
        buf
    } else {
        std::fs::read(&cli.component)
            .map_err(|e| anyhow::anyhow!("read component {}: {e}", cli.component.display()))?
    };

    // Resolve the value-heap runtime ONLY if the component records one — a scalar/const component
    // imports nothing and needs no runtime, so a missing store is not an error there. When it does,
    // resolve BY CONTENT ADDRESS: the exact hash the component records must be in the store.
    let runtime = match required_runtime(&component_bytes)? {
        Some(req) => Some(resolve_runtime(cli, &req)?),
        None => None,
    };

    // Cache the COMPILED runtime artifact in the store dir (unless the runtime came from a `--runtime`
    // override, which is a debugging path we don't cache). Compiling the runtime is ~75ms and it is
    // byte-identical across heap programs, so caching turns every-run recompiles into one compile +
    // fast deserializes. See `cdz_run::RunOpts::runtime_cache_dir`.
    let runtime_cache_dir = if runtime.is_some() && cli.runtime.is_none() {
        Some(cli.store.clone().unwrap_or_else(default_store))
    } else {
        None
    };

    // Parse each `--host-response op=value` into a `HostResponse`. A missing `=` takes the whole string
    // as the value with an empty op label (the ordered-consume model does not yet match on the op).
    let host_responses = cli
        .host_responses
        .iter()
        .map(|s| match s.split_once('=') {
            Some((op, value)) => HostResponse {
                op: op.to_string(),
                value: value.to_string(),
            },
            None => HostResponse {
                op: String::new(),
                value: s.clone(),
            },
        })
        .collect();

    // Parse each `--peer interface=path` and read the peer component bytes. A peer that itself imports the
    // runtime is composed against the SAME shared instance `run_with_peers` binds (X4b/X5).
    let peers: Vec<Peer> = cli
        .peers
        .iter()
        .map(|s| {
            let (iface, path) = s
                .split_once('=')
                .ok_or_else(|| anyhow::anyhow!("--peer expects `interface=path`, got `{s}`"))?;
            let bytes = std::fs::read(path)
                .map_err(|e| anyhow::anyhow!("read peer component {path}: {e}"))?;
            Ok(Peer {
                bytes,
                interface: iface.to_string(),
            })
        })
        .collect::<anyhow::Result<_>>()?;

    // If any peer needs the runtime but the consumer did not, resolve it too (they share one instance).
    let runtime = match runtime {
        Some(r) => Some(r),
        None if !peers.is_empty() => {
            let mut rt = None;
            for peer in &peers {
                if let Some(req) = required_runtime(&peer.bytes)? {
                    rt = Some(resolve_runtime(cli, &req)?);
                    break;
                }
            }
            rt
        }
        None => None,
    };

    let opts = RunOpts {
        export: cli.call.clone(),
        args: cli.args.clone(),
        runtime,
        runtime_cache_dir,
        host_responses,
    };

    if !peers.is_empty() {
        // Compose the CONSUMER with its peers across the live boundary; the observed host calls are not
        // captured on this path (a cross-component run is not a host-effect run).
        let outcome = run_with_peers(&component_bytes, &peers, &opts)?;
        return match outcome {
            Outcome::Value(text) => {
                println!("{text}");
                Ok(ExitCode::SUCCESS)
            }
            Outcome::Trap(msg) => {
                eprintln!("trap: {msg}");
                Ok(ExitCode::FAILURE)
            }
        };
    }

    let (outcome, observed) = run_capturing(&component_bytes, &opts)?;
    // Emit the OBSERVED host calls to stderr, in call order. On stderr (not stdout) so the value on stdout
    // stays clean; absent for a program that makes no host call. Each observed entry is `<op>` OR
    // `<op>\t<message>` (the latter when the call carried STRING arguments — a `report.fail("…")` /
    // `log.emit("…")`). Split on the FIRST tab so the op stays clean:
    //   - `host-call\t<op>` — ALWAYS emitted (the corpus gate reads these to verify `(host-calls …)`; the
    //     `<op>` field is unpolluted so an argument-carrying call still matches its recorded op).
    //   - `host-arg\t<op>\t<message>` — ALSO emitted when a message rode along, so a consumer that wants
    //     the argument (`cdz test`, whose failure path emits the assertion text) can read it. The gate
    //     ignores an unknown `host-arg` prefix, so this is additive and backward-compatible.
    for entry in &observed {
        let (op, msg) = match entry.split_once('\t') {
            Some((op, msg)) => (op, Some(msg)),
            None => (entry.as_str(), None),
        };
        eprintln!("host-call\t{op}");
        if let Some(msg) = msg {
            eprintln!("host-arg\t{op}\t{msg}");
        }
    }
    match outcome {
        Outcome::Value(text) => {
            println!("{text}");
            Ok(ExitCode::SUCCESS)
        }
        Outcome::Trap(msg) => {
            eprintln!("trap: {msg}");
            Ok(ExitCode::FAILURE)
        }
    }
}

/// Resolve the value-heap runtime bytes the component requires, BY CONTENT ADDRESS. The component
/// records the exact hash it was emitted against (component-abi.md §The Emitted Component Records Its
/// Required Runtime); the host locates `<store>/<hash>.wasm` and REFUSES to run if that exact hash is
/// absent — never substituting a different runtime (§The Host Resolves The Runtime By Content
/// Address). `--runtime <path>` is a debugging escape hatch that bypasses the store lookup.
//= spec/contracts/component-abi.md#the-host-resolves-the-runtime-by-content-address
//# A host MUST resolve a program's runtime import by reading the required runtime content address the component records and locating the runtime component of that content address in a content-addressed store, rather than by assuming a single ambient runtime, so that programs pinned to different runtime versions coexist and each resolves the exact runtime it was emitted against.
//= spec/contracts/component-abi.md#the-host-resolves-the-runtime-by-content-address
//# A host that cannot locate a runtime of the content address a component requires MUST refuse to run the component rather than substitute a different runtime, so that a mismatched runtime is a detected error rather than a silent change in observable behavior.
// Resolving by the component's pinned hash (and verifying the store entry hashes back to it, below) is
// also how a run is bound to the exact runtime the program was emitted against — the reproducible-
// derivation guarantee that execution is deterministic in the (program, runtime content address) pair:
//= spec/contracts/reproducible-derivation.md#derivation-is-a-function-of-source-and-toolchain
//# A program that is run or resumed against the value-heap runtime MUST be run against the runtime whose content address is the one pinned for that program, so that execution is deterministic in the pair (program, runtime content address) and a runtime built from different bytes is a distinct, explicitly-identified execution environment rather than a silent substitution.
fn resolve_runtime(cli: &RunArgs, req: &crate::RuntimeReq) -> anyhow::Result<Vec<u8>> {
    if let Some(path) = &cli.runtime {
        return std::fs::read(path)
            .map_err(|e| anyhow::anyhow!("read --runtime {}: {e}", path.display()));
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
    let repo = manifest
        .ancestors()
        .nth(4)
        .unwrap_or(&manifest)
        .to_path_buf();
    repo.join("target/cadenza-store")
}
