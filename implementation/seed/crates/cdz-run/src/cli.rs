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
/// `Clone` so a caller (e.g. `cdz run <project>`, which builds first) can re-target `component` at a
/// freshly-built component while passing every other flag through unchanged.
///
/// TIMEOUT: a run is capped at a wall-clock deadline (default 30s) so a runaway/infinite loop TRAPS
/// instead of spinning forever; set `CDZ_RUN_TIMEOUT_SECS=<n>` to change it, or `=0` to disable the cap
/// (e.g. under a debugger). A normal program finishes in milliseconds and never hits this.
#[derive(clap::Args, Clone)]
pub struct RunArgs {
    /// The component `.wasm` to run, or `-` to read it from stdin (so it composes in a pipe:
    /// `cdz compile - -o - | cdz run -`). OMITTED — under the `cdz` front-end — means "the project in the
    /// current directory": `cdz` searches up for the nearest `Project.cdz` and builds+runs its entry (the
    /// `cargo run` analogue). The standalone `cdz-run` binary has no compiler, so it still REQUIRES a
    /// component argument (a bare `cdz-run` errors); the optionality is honored only on the `cdz run` path.
    pub component: Option<PathBuf>,

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

    /// PROJECT mode only (`cdz run` on a `Project.cdz`/directory/omitted): build the entry at the RELEASE
    /// tier (`O2`) before running, the `cargo run --release` analogue. Ignored when running a pre-built
    /// `.wasm` (there is nothing to build). Shorthand for `--opt-level O2`; `--opt-level` wins if both are
    /// given, and a manifest `def opt-level` wins over `--release` (same precedence as `cdz build`).
    #[arg(long)]
    pub release: bool,

    /// PROJECT mode only: the optimization LEVEL (`O0`..`O3`) to build the entry at before running,
    /// overriding both `--release` and any `Project.cdz` `opt-level`. Ignored when running a pre-built
    /// `.wasm`. Omitted → the manifest's `opt-level`, else `--release`'s `O2`, else the default `O1`.
    #[arg(long, value_name = "LEVEL")]
    pub opt_level: Option<String>,
}

/// Run a component per `args`, printing the value to stdout (host calls to stderr) and returning the
/// process exit code. `prog` names the tool in diagnostics (`cdz-run` for the standalone bin, `cdz` for
/// the unified one), so an error message points at the command the user actually typed.
///
/// Exit-code contract (consistent with the rest of the `cdz` toolchain): an OPERATIONAL failure — a
/// missing/unreadable component, an unresolvable runtime, an invalid component, or a run-time trap — is
/// `1`. A CLI-USAGE error (an unknown flag, a missing required argument) is `2`, but clap emits THAT
/// before `run` is ever called, so `run`'s own error path is always an operational `1`. This distinction
/// lets a script tell "you invoked it wrong" (2) from "it ran and failed" (1) — previously an operational
/// error here returned `2`, colliding with the usage signal (and inconsistent with a trap, which is `1`).
pub fn run(args: &RunArgs, prog: &str) -> ExitCode {
    match real_run(args, prog) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("{prog}: {e:#}");
            ExitCode::FAILURE // operational failure → 1 (usage errors are clap's own 2, before this)
        }
    }
}

fn real_run(cli: &RunArgs, prog: &str) -> anyhow::Result<ExitCode> {
    // The component is required on this path: a `.wasm`/stdin arg to run directly. A None `component`
    // reaches here only via the standalone `cdz-run` (which has no compiler to build a project from) — the
    // `cdz run` front-end intercepts the project cases (`Project.cdz` / a directory / omitted) BEFORE
    // delegating here. So an absent component is a clear usage error naming what to pass.
    let Some(component) = cli.component.as_ref() else {
        anyhow::bail!(
            "no component to run — pass a `.wasm` (or `-` for stdin). To build+run a project, \
             use the `cdz run` front-end (`cdz run [dir]`), which has the compiler"
        );
    };
    // The component bytes: from a file, or from stdin when the path is `-`.
    let component_bytes = if component.as_os_str() == "-" {
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut std::io::stdin(), &mut buf)
            .map_err(|e| anyhow::anyhow!("read component from stdin: {e}"))?;
        buf
    } else {
        std::fs::read(component)
            .map_err(|e| anyhow::anyhow!("read component {}: {e}", component.display()))?
    };

    // Resolve the value-heap runtime ONLY if the component records one — a scalar/const component
    // imports nothing and needs no runtime, so a missing store is not an error there. When it does,
    // resolve BY CONTENT ADDRESS: the exact hash the component records must be in the store.
    let runtime = match required_runtime(&component_bytes)? {
        Some(req) => Some(resolve_runtime(cli, &req)?),
        None => None,
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
            // Both halves must be non-empty. An empty PATH (`--peer iface=`) otherwise falls through to
            // `fs::read("")` → a confusing blank-filename "No such file" error; an empty INTERFACE
            // (`--peer =path`) makes a peer with no interface name that fails opaquely later. Name the
            // real problem at the CLI edge.
            if iface.is_empty() {
                return Err(anyhow::anyhow!(
                    "--peer `{s}` has an empty interface name — expected `interface=path` \
                     (e.g. `cadenza:math/api=math.wasm`)"
                ));
            }
            if path.is_empty() {
                return Err(anyhow::anyhow!(
                    "--peer `{s}` has an empty path — expected `interface=path` \
                     (e.g. `cadenza:math/api=math.wasm`)"
                ));
            }
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

    // Compute the runtime cache/NFC-store dir AFTER the peer-runtime resolution above — so a runtime induced
    // by a PEER (consumer needs none, but a `--peer` does) is store-scoped for NFC too, not just a consumer's.
    // Computing it earlier (before this block) missed the peer case: `runtime` was still `None` at that point,
    // so an explicit `--store` did not scope NFC for the peer-induced runtime (PR #1633 review follow-on).
    let runtime_cache_dir =
        resolve_runtime_cache_dir(runtime.is_some(), cli.runtime.is_some(), cli.store.clone());

    // FINDING#23: the runtime imports `cadenza:nfc/normalize`, but the host now SELF-RESOLVES that NFC
    // component from the store inside `compose_nfc_into_runtime_linker` (via `runtime_cache_dir`/`CDZ_STORE`/
    // the default store + `runtime.toml`) — no `nfc` field to thread here anymore.
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
                eprintln!("{prog}: trap: {msg}");
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
            eprintln!("{prog}: trap: {msg}");
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

/// BLAKE3 of `bytes`, lowercase hex — the CANONICAL content-address of a Cadenza component store.
///
/// # Store-address contract (the shared seam every store reader/writer must honor)
///
/// This is the one address function for the seed/nix component store. Producers (`xtask`'s store
/// writer, `cdz`'s `--store` output) and readers (this crate's `resolve_nfc_from_store` +
/// runtime-dep resolver, the kernel's `component_store` per concierge ruling (A)) all address blobs
/// with THIS function. A reader that content-verifies with a different primitive (e.g. the kernel's
/// internal BLAKE3 `Hash::of`, which is for events/KV/blobs — NOT the on-disk store) will mismatch
/// every fetch. ("readers" above: `resolve_nfc_from_store` + the runtime-dep resolver in this crate's
/// `lib.rs`, and the kernel's `component_store` in the `cdz-kernel` crate — cross-module, so named in
/// plain code font, not intra-doc links.)
///
/// - **Address:** BLAKE3 of the component bytes, lowercase hex (64 chars).
/// - **Store layout:** `<base62hash>.wasm` per component + a `runtime.toml` manifest at the store root.
/// - **`runtime.toml`:** maps the runtime's BARE inter-runtime imports by NAME→hash —
///   `runtime = "<hash>"`, `debug_runtime = "<hash>"`, `nfc = "<hash>"`. This is how a bare
///   `cadenza:nfc/normalize` import is resolved (name→hash→`<hash>.wasm`, then content-verified here),
///   as distinct from a program's OWN runtime dep which carries the hash IN the import name
///   (`cadenza:runtime/heap@0.0.0+<hash>`) and resolves directly.
/// - **`REQUIRED_RUNTIME_HASH`** (`rcdzc::backend::wasm::runtime_abi`) IS this content address of the
///   built runtime bytes, pinned into the generated ABI by `xtask` codegen and keyed by every program's
///   runtime import. It is regenerated by `xtask codegen` whenever this address function changes.
/// - **UNIFIED HASH:** content addressing is unified tree-wide on the platform's [`cdz_contract::Hash`]
///   (a `HashTag::Blob`-tagged blake3 digest, rendered base62 per design §8) — so a `+<hash>` dep import,
///   a blob-store key, a `CDZ_STORE` `<hash>.wasm` name, and `REQUIRED_RUNTIME_HASH` are ONE interchangeable
///   address space. It delegates to `cdz-contract` so this producer is byte-identical to the store's own
///   `put()` (`blob_store.rs` returns `Hash::of(HashTag::Blob, bytes)`) and to the compiler's emitted
///   `+<suffix>`; base62 is the sole text form because the suffix rides a component-import semver
///   build-metadata field, whose grammar rejects base64url's `_` (design §8).
pub fn content_address(bytes: &[u8]) -> String {
    cdz_contract::Hash::of(cdz_contract::HashTag::Blob, bytes).to_string()
}

#[cfg(test)]
mod content_address_tests {
    use super::content_address;

    /// The load-bearing invariant of the base62 flip: a content address MUST be legal as a WebAssembly
    /// component-import semver build-metadata suffix (`cadenza:runtime/heap@0.0.0+<addr>`), whose grammar
    /// admits only `[0-9A-Za-z-]`. base62 (`0-9A-Za-z`) satisfies it; base64url (`_`) or any `+`/`/`/`=`
    /// encoding would NOT — that is why the fleet is on base62 and never hex-or-base64url here. This pins
    /// the property AT the `content_address` boundary, so a future re-encoding that reintroduces a
    /// suffix-illegal character fails here, not silently at wasm-tools compose time.
    #[test]
    fn content_address_is_a_legal_component_import_suffix() {
        for input in [
            b"".as_slice(),
            b"cadenza",
            b"the value-heap runtime bytes",
            &[0xFFu8; 64],
        ] {
            let addr = content_address(input);
            // 45 fixed base62 chars (the tagged 33-byte Hash's text width).
            assert_eq!(
                addr.len(),
                45,
                "content address is a fixed 45-char base62 string"
            );
            // Every character legal in a semver build-metadata suffix, and specifically NONE of the
            // separators a non-base62 encoding would introduce.
            assert!(
                addr.bytes().all(|b| b.is_ascii_alphanumeric()),
                "content address {addr} must be base62 (0-9A-Za-z) to ride a component-import +suffix"
            );
            assert!(
                !addr.contains(['_', '+', '/', '=', '-']),
                "content address {addr} must carry no base64url/base64/hyphen separator"
            );
        }
    }

    /// Content addressing is a deterministic, collision-distinguishing function of the bytes — the whole
    /// premise of a content-addressed store (same bytes ⇒ same name; different bytes ⇒ different name).
    #[test]
    fn content_address_is_deterministic_and_distinguishing() {
        assert_eq!(content_address(b"cadenza"), content_address(b"cadenza"));
        assert_ne!(content_address(b"cadenza"), content_address(b"cadenzb"));
    }
}

/// The default content-addressed store: the `CDZ_STORE` env var if set, else `<repo>/target/cadenza-store`
/// resolved from this crate's manifest location (crate lives at `<repo>/implementation/seed/crates/cdz-run`).
/// The `--store` flag still wins over this (callers `unwrap_or_else(default_store)`), so precedence is
/// flag > `CDZ_STORE` > compiled default — matching `resolve_nfc_from_store`'s NFC-component resolution so a
/// single env var repoints the WHOLE store (value-heap runtime + NFC) at a Nix-provided path (R4).
fn default_store() -> PathBuf {
    store_from_env_or(std::env::var_os("CDZ_STORE"), compiled_default_store)
}

/// The compiled fallback store path (`<repo>/target/cadenza-store`) — used only when `CDZ_STORE` is unset.
fn compiled_default_store() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // <repo>/implementation/seed/crates/cdz-run → up 4 → <repo>
    let repo = manifest
        .ancestors()
        .nth(4)
        .unwrap_or(&manifest)
        .to_path_buf();
    repo.join("target/cadenza-store")
}

/// Choose the store dir threaded into `RunOpts::runtime_cache_dir`, which serves TWO purposes: caching the
/// COMPILED runtime artifact (`<hash>.cwasm`) AND self-resolving the runtime's NFC dependency
/// (`resolve_nfc_from_store` reads `runtime.toml` + `<store>/<hash>.wasm` off it). `has_runtime` = a runtime
/// will be composed; `explicit_runtime` = `--runtime <path>` was given (a debug override, bypassing the
/// store's runtime lookup); `store` = the `--store <dir>` value if pinned. Rules:
///   • no runtime to compose → `None` (nothing to cache or NFC-resolve).
///   • store-resolved runtime (no `--runtime`) → the pinned `--store` else the default store.
///   • an EXPLICIT `--store` even WITH `--runtime` → that store, so an explicit store scopes NFC resolution
///     too (else `--store D --runtime P` would silently resolve NFC from `CDZ_STORE`/default, IGNORING the D
///     the user pinned — PR #1623 review). Caching the cwasm there is harmless.
///   • a bare `--runtime` with NO `--store` → `None` (no pinned store: don't cache a debug-override runtime;
///     NFC falls back to `CDZ_STORE`/default).
fn resolve_runtime_cache_dir(
    has_runtime: bool,
    explicit_runtime: bool,
    store: Option<PathBuf>,
) -> Option<PathBuf> {
    if !has_runtime {
        None
    } else if !explicit_runtime {
        Some(store.unwrap_or_else(default_store))
    } else {
        // `--runtime <path>` given: honor an explicit `--store` (scopes NFC), else `None`.
        store
    }
}

/// Pure precedence: `CDZ_STORE` (as an already-read `OsString`) wins over the compiled fallback. Split out
/// so the env-var precedence is unit-testable without mutating the process-global environment.
fn store_from_env_or(
    env: Option<std::ffi::OsString>,
    fallback: impl FnOnce() -> PathBuf,
) -> PathBuf {
    match env {
        Some(dir) => PathBuf::from(dir),
        None => fallback(),
    }
}

#[cfg(test)]
mod cache_dir_tests {
    use super::*;

    #[test]
    fn no_runtime_needs_no_store() {
        // A scalar/const component composes no runtime → no cache/NFC dir at all.
        assert_eq!(
            resolve_runtime_cache_dir(false, false, Some(PathBuf::from("/s"))),
            None
        );
    }

    #[test]
    fn store_resolved_runtime_uses_the_pinned_store_else_default() {
        // No `--runtime`: the pinned `--store` is the cache + NFC dir.
        assert_eq!(
            resolve_runtime_cache_dir(true, false, Some(PathBuf::from("/pinned"))),
            Some(PathBuf::from("/pinned"))
        );
        // No `--store` either → the compiled default store.
        assert_eq!(
            resolve_runtime_cache_dir(true, false, None),
            Some(default_store())
        );
    }

    #[test]
    fn explicit_store_scopes_nfc_even_with_an_explicit_runtime() {
        // The PR #1623 review fix: `--store D --runtime P` must resolve NFC from D, NOT CDZ_STORE/default.
        // So the cache/NFC dir is the pinned store even when `--runtime` overrides the runtime bytes.
        assert_eq!(
            resolve_runtime_cache_dir(true, true, Some(PathBuf::from("/pinned"))),
            Some(PathBuf::from("/pinned")),
            "an explicit --store must scope NFC resolution even with --runtime"
        );
    }

    #[test]
    fn a_bare_runtime_override_with_no_store_pins_nothing() {
        // `--runtime P` with NO `--store`: don't cache a debug-override runtime; NFC falls back to
        // CDZ_STORE/default (handled downstream in resolve_nfc_from_store), so no dir is pinned here.
        assert_eq!(resolve_runtime_cache_dir(true, true, None), None);
    }
}

#[cfg(test)]
mod store_tests {
    use super::*;

    #[test]
    fn cdz_store_env_wins_over_compiled_default() {
        let picked = store_from_env_or(Some("/nix/store/abc-cadenza-store".into()), || {
            PathBuf::from("/should/not/be/used")
        });
        assert_eq!(picked, PathBuf::from("/nix/store/abc-cadenza-store"));
    }

    #[test]
    fn compiled_default_used_when_env_unset() {
        let picked = store_from_env_or(None, || PathBuf::from("/repo/target/cadenza-store"));
        assert_eq!(picked, PathBuf::from("/repo/target/cadenza-store"));
    }

    #[test]
    fn empty_env_value_is_still_honored_not_treated_as_unset() {
        // An explicitly-set-but-empty CDZ_STORE is a caller choice (var_os returns Some("")), distinct from
        // unset (None). We honor it verbatim rather than silently falling back — the flag layer above can
        // still override, and an empty path fails loudly at store-open rather than masking a misconfig.
        let picked = store_from_env_or(Some("".into()), || PathBuf::from("/fallback"));
        assert_eq!(picked, PathBuf::from(""));
    }
}
