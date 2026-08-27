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
    HostResponse, Outcome, Peer, RunOpts, required_runtime, run_capturing, run_with_live_objects,
    run_with_peers,
};

/// How `cdz run` encodes the run RESULT on stdout. `Sexp` (the historical default) pretty-prints the value
/// form as a human-readable s-expression; `BinaryAst` writes the RAW canonical binary value form — the
/// universal `cadenza-ast` codec bytes — so a downstream tool decodes it with `cadenza_ast::codec::decode`
/// (a dependency it already has) and navigates the value STRUCTURALLY, rather than parsing rendered text.
#[derive(clap::ValueEnum, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum OutputFormat {
    /// Pretty-print the value form as an s-expression (the historical human render).
    #[default]
    Sexp,
    /// Emit the raw canonical binary AST bytes of the escaped value form (machine exchange).
    #[value(name = "binary-ast")]
    BinaryAst,
}

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

    /// Two-call-on-one-handle mode for a CLOSURE export (a `borrow<t>` closure keeps its handle live
    /// across calls, so it is repeatable — an `own<t>` closure would trap on the second call). When set,
    /// the run makes the closure handle ONCE (`--arg`s split as usual: make's params then the FIRST
    /// call's args), then calls it a SECOND time with `--then-arg`s, and renders both results as a tuple
    /// `(: (tuple <r1> <r2>) (Tuple T T))`. The corpus `(then …)` clause drives this.
    #[arg(long = "call-twice")]
    pub call_twice: bool,

    /// A SECOND-call argument (repeatable), used only with `--call-twice`. `allow_hyphen_values` so a
    /// negative value is taken as the argument, not a flag. A bare `--call-twice` with no `--then-arg`
    /// drives a nullary second call.
    #[arg(long = "then-arg", value_name = "VALUE", allow_hyphen_values = true)]
    pub then_args: Vec<String>,

    /// After the closure call(s), RESOURCE-DROP the minted handle before reading the result / heap balance.
    /// `call` BORROWS the handle, so without this the closure cell stays live until store teardown (a
    /// `--report-live-objects` run then reports the leak); dropping fires the resource's `t-dtor`, reclaiming
    /// the cell, so a leak-release case reports 0. The corpus `(drop)` clause drives this. Closure/escape
    /// runs only.
    #[arg(long = "drop-handle")]
    pub drop_handle: bool,

    /// Invoke a NAMED member on the value-resource the program produces, instead of the default `encode`.
    /// A runtime value crossing as a resource in the `cadenza:run/run` instance exposes compiler-emitted
    /// members (e.g. a `Bytes` value's `len`/`is-empty`/`to-bytes` besides `encode`); this reaches the named
    /// one and renders its result (a scalar/bool directly, a value-form list<u8> decoded). `--call-twice`/
    /// `--then-arg` repeat it on the same handle (a borrow method is repeatable); `--drop-handle` reclaims
    /// after. The corpus `(call-method <member> …)` clause drives this.
    #[arg(long = "call-member", value_name = "MEMBER")]
    pub call_member: Option<String>,

    /// Output ENCODING for the run RESULT. `sexp` (default) pretty-prints the value form as an s-expression;
    /// `binary-ast` writes the RAW canonical binary AST bytes of the escaped value form to stdout instead —
    /// the universal `cadenza-ast` exchange format — so a downstream tool decodes it with
    /// `cadenza_ast::codec::decode` and navigates the value structurally rather than parsing rendered text
    /// (e.g. `cdz-contract` reading a contract's `descriptor()` record to project its `id`/`name`).
    /// `binary-ast` requires a value that escapes as a value-form document (a compound result crossing via
    /// the `cadenza:run/run` `encode` escape) and emits that `encode` document (independent of
    /// `--call-member`, which selects an alternate resource member for the s-expression render path).
    #[arg(long = "format", value_name = "FMT", default_value = "sexp")]
    pub format: OutputFormat,

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

    /// GRADE mode (the corpus nix pipeline's exec phase, `design/DESIGN-corpus-nix-per-case-caching.md`):
    /// grade the case against a shredded `test-run.ast` (built by `cdz corpus records --out-dir`),
    /// reproducing the `xtask gate` comparison for EVERY outcome kind. RUN outcomes (`expect-output`/
    /// `expect-trap`) run the wasm `COMPONENT` (which is then OPTIONAL — absent when the compile was
    /// refused); COMPILE outcomes (`expect-error`/`expect-declines`) + `warns` are graded from the captured
    /// compiler result (`--compile-status`/`--compile-diag`). `--call`/`--arg`/`--host-response` come from
    /// the artifact. Exit `0` if all pass (or `Todo`), `1` on the first `Fail`.
    #[arg(long, value_name = "TEST_RUN_AST")]
    pub grade: Option<PathBuf>,

    /// GRADE mode: the exit status of the case's compile (`0` = compiled → `COMPONENT` present; non-zero =
    /// the compiler refused → an error/declines outcome). Defaults to `0`.
    #[arg(long, value_name = "N", default_value_t = 0)]
    pub compile_status: i32,

    /// GRADE mode: the compiler's captured stderr (the diagnostic), for grading `expect-error`/
    /// `expect-declines` (code + message) and `warns` (presence). Empty/absent → no diagnostic text.
    #[arg(long, value_name = "PATH")]
    pub compile_diag: Option<PathBuf>,

    /// The interface a `(wit-world …)` case's guest exports under (its `(component-name …)`), used ONLY
    /// with `--grade` to qualify a trial's call as `<iface>#<export>` — the same qualification the gate
    /// applies for a world-imposed export. Absent → the export is called by its bare name.
    #[arg(long, value_name = "INTERFACE")]
    pub component_name: Option<String>,

    /// After running, ALSO read the value-heap runtime's live-cell count (`live-objects`) and print it as
    /// a `live-objects\t<N>` line on stdout (after the result). The corpus `(live-objects N)` clause drives
    /// this to assert heap balance (no leak / no double-free). Requires the component to import the runtime,
    /// and the resolved runtime MUST be the debug-counters build (the shipped one always reports 0).
    #[arg(long)]
    pub report_live_objects: bool,

    /// GRADE mode: the committed per-backend baseline (`spec/semantics/.gate-baseline`), a
    /// `<verdict>\t<description>` snapshot. When given, a REGRESSION — a case the baseline recorded as
    /// `pass` that no longer passes — fails the grade (exit 1), the per-case analogue of `xtask gate
    /// --check` (gap #7). Absent → no regression check (a plain pass/todo/fail grade).
    #[arg(long, value_name = "PATH")]
    pub baseline: Option<PathBuf>,
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
    // GRADE mode: grade a case against a shredded `test-run.ast` (the corpus nix pipeline's exec phase).
    // The component is OPTIONAL here — a case whose compile was REFUSED (error/declines) has no wasm; it is
    // graded purely from `--compile-status`/`--compile-diag`. When present, the wasm is read + its runtime
    // resolved so the RUN outcomes (output/trap) can execute. Takes over from the single-call path below.
    if let Some(test_run_path) = &cli.grade {
        let test_run_ast = std::fs::read(test_run_path)
            .map_err(|e| anyhow::anyhow!("read test-run.ast {}: {e}", test_run_path.display()))?;
        let compile_diag = match &cli.compile_diag {
            Some(p) => std::fs::read_to_string(p).unwrap_or_default(),
            None => String::new(),
        };
        let baseline = match &cli.baseline {
            Some(p) => Some(
                std::fs::read_to_string(p)
                    .map_err(|e| anyhow::anyhow!("read baseline {}: {e}", p.display()))?,
            ),
            None => None,
        };
        // A `(peer …)` case ships providers via `--peer <iface>=<wasm>`; the grade path MUST compose them
        // (the consumer's imported interface is bound by forwarding the peer's exports over the shared
        // runtime). Parse them HERE — grade mode returns before the direct-run path's peer parse, so
        // without this the `--peer` args are silently dropped and the import falls to an unbound host-call
        // (the corpus-29 nix reds). Empty for a plain case.
        let peers = parse_peer_args(&cli.peers)?;
        let (bytes, runtime, runtime_cache_dir) = match &cli.component {
            Some(component) => {
                let bytes = read_component_bytes(component)?;
                // Resolve the value-heap runtime the composition needs. The CONSUMER's import first; but a
                // SCALAR consumer that composes a HEAP peer (e.g. the A→B→C chain, where the runtime is
                // imported by a middle/leaf provider, not the top consumer) imports none itself — so fall
                // back to the FIRST peer that requires one (they all pin the same shared instance). Without
                // this fallback the `--runtime` override (the grade's DEBUG-COUNTERS runtime) is not applied
                // to the peer, so it resolves the SHIPPED runtime by hash and the heap-balance count is
                // vacuous — the exact same consumer/peer runtime-sharing the direct-run path resolves.
                let runtime = match required_runtime(&bytes)? {
                    Some(req) => Some(resolve_runtime(cli, &req)?),
                    None => {
                        let mut rt = None;
                        for peer in &peers {
                            if let Some(req) = required_runtime(&peer.bytes)? {
                                rt = Some(resolve_runtime(cli, &req)?);
                                break;
                            }
                        }
                        rt
                    }
                };
                let rcd = resolve_runtime_cache_dir(
                    runtime.is_some(),
                    cli.runtime.is_some(),
                    cli.store.clone(),
                );
                (Some(bytes), runtime, rcd)
            }
            None => (None, None, None),
        };
        return crate::grade::grade(
            bytes.as_deref(),
            &test_run_ast,
            runtime,
            runtime_cache_dir,
            cli.component_name.as_deref(),
            cli.compile_status,
            &compile_diag,
            baseline.as_deref(),
            &peers,
        );
    }

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
    let component_bytes = read_component_bytes(component)?;

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
    let peers: Vec<Peer> = parse_peer_args(&cli.peers)?;

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

    // The runtime imports `cadenza:nfc/normalize@0.0.0+<hash>` (self-describing — the NFC dependency's
    // content address is stamped inline into the import), and the host resolves that NFC component from the
    // store BY THAT INLINE HASH inside `compose_nfc_into_runtime_linker` (via `runtime_cache_dir`/`CDZ_STORE`/
    // the default store — a pure CAS lookup, NO `runtime.toml` / mapping file). No `nfc` field to thread here.
    let opts = RunOpts {
        export: cli.call.clone(),
        args: cli.args.clone(),
        runtime,
        runtime_cache_dir,
        host_responses,
    };
    // A `--call-twice` request (the corpus `(then …)` two-call-on-one-handle drive): the second call's
    // args, threaded alongside `opts` into the run path down to the closure-escape dispatch. `None` for an
    // ordinary one-call run. Not a `RunOpts` field — that struct's field-adds are a known livelock (203
    // exhaustive literals); it rides as a parameter on the run functions the gate path uses instead.
    let second_call: Option<&[String]> = cli.call_twice.then_some(cli.then_args.as_slice());
    // Whether to resource-drop the closure handle after the call(s) (the `(drop)` clause) — threaded like
    // `second_call` on the run path down to the closure/escape driver.
    let drop_handle = cli.drop_handle;
    // The named value-resource member to reach (the `(call-method)` clause), threaded like `drop_handle`.
    let call_member: Option<&str> = cli.call_member.as_deref();

    // `--format binary-ast`: emit the RAW canonical binary value form of the escaped result to stdout (the
    // universal cadenza-ast exchange format) instead of the rendered s-expression, so a downstream tool
    // (e.g. `cdz-contract` reading a contract's `descriptor()`) decodes it with `cadenza_ast::codec::decode`
    // and navigates the value structurally — no fragile text parse. Emits the `encode` escape document (the
    // whole value form); the caller projects the field(s) it wants after decoding. A value that does not
    // escape as a value-form document (a bare scalar) errors in `capture_escaped_value_doc`, naming the cause.
    if cli.format == OutputFormat::BinaryAst {
        let doc = crate::capture_escaped_value_doc(&component_bytes, &cli.args, &opts)?;
        use std::io::Write;
        std::io::stdout()
            .write_all(&doc)
            .map_err(|e| anyhow::anyhow!("writing binary-ast result to stdout: {e}"))?;
        return Ok(ExitCode::SUCCESS);
    }

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

    if cli.report_live_objects {
        // Run + read the heap's live-cell count, printing the value then a `live-objects\t<N>` line on
        // stdout (the corpus opt-out gate reads the tab line to assert heap balance). The line is emitted
        // ONLY when the component imports the value-heap runtime (a HEAP case); a scalar/const program has
        // no heap to balance, so it runs normally and emits NO line — the gate then SKIPS the balance
        // check for it (never a false fail). Observed host calls are captured + emitted exactly as the
        // normal path, so a heap case that also delegates host effects still has its `(host-calls …)`
        // verified. Requires the DEBUG-COUNTERS runtime for the count to be meaningful (the shipped one
        // always reports 0).
        //
        // DIAGNOSTIC (to STDERR — stdout carries only the value + the optional `live-objects` tab line the
        // gate parses): name the runtime that will ACTUALLY run, by the content address of its bytes, plus
        // how it was resolved. A `live-objects 0` from the SHIPPED release runtime is otherwise
        // indistinguishable from a genuine leak-free run; printing the loaded hash makes a vacuous run
        // self-evident (if the hash is the release build, the count is meaningless — pass
        // `--runtime <debug-counters>.wasm`).
        if let Some(rt) = &opts.runtime {
            let src = if cli.runtime.is_some() {
                "--runtime override"
            } else {
                "store-resolved"
            };
            eprintln!(
                "{prog}: live-objects run on value-heap runtime {} ({src})",
                content_address(rt)
            );
        }
        let (outcome, observed, live) = run_with_live_objects(
            &component_bytes,
            &opts,
            second_call,
            drop_handle,
            call_member,
        )?;
        emit_observed_host_calls(&observed);
        return match outcome {
            Outcome::Value(text) => {
                println!("{text}");
                if let Some(live) = live {
                    println!("live-objects\t{live}");
                }
                Ok(ExitCode::SUCCESS)
            }
            Outcome::Trap(msg) => {
                eprintln!("{prog}: trap: {msg}");
                if let Some(live) = live {
                    println!("live-objects\t{live}");
                }
                Ok(ExitCode::FAILURE)
            }
        };
    }

    let (outcome, observed) = run_capturing(
        &component_bytes,
        &opts,
        second_call,
        drop_handle,
        call_member,
    )?;
    emit_observed_host_calls(&observed);
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

/// Emit the OBSERVED host calls to stderr, in call order. On stderr (not stdout) so the value on stdout
/// stays clean; absent for a program that makes no host call. Each observed entry is `<op>` OR
/// `<op>\t<message>` (the latter when the call carried STRING arguments — a `report.fail("…")` /
/// `log.emit("…")`). Split on the FIRST tab so the op stays clean:
///   - `host-call\t<op>` — ALWAYS emitted (the corpus gate reads these to verify `(host-calls …)`; the
///     `<op>` field is unpolluted so an argument-carrying call still matches its recorded op).
///   - `host-arg\t<op>\t<message>` — ALSO emitted when a message rode along, so a consumer that wants the
///     argument (`cdz test`, whose failure path emits the assertion text) can read it. The gate ignores an
///     unknown `host-arg` prefix, so this is additive and backward-compatible.
fn emit_observed_host_calls(observed: &[String]) {
    for entry in observed {
        let (op, msg) = match entry.split_once('\t') {
            Some((op, msg)) => (op, Some(msg)),
            None => (entry.as_str(), None),
        };
        eprintln!("host-call\t{op}");
        if let Some(msg) = msg {
            eprintln!("host-arg\t{op}\t{msg}");
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
/// Read a component's bytes — from a file, or from stdin when the path is `-` (so the bin composes in a
/// pipe). Shared by the single-run path and grade mode.
/// Parse each `--peer <interface>=<path>` into a [`Peer`] (interface name + the peer component's bytes).
/// Shared by the direct-run path AND the `--grade` path — a `(peer …)` corpus case graded via the nix exec
/// MUST compose its peers, so grade mode parses them here too (before its early return) rather than letting
/// the `--peer` args be silently dropped. Both halves must be non-empty (a blank interface / path fails
/// opaquely deeper), named at the CLI edge.
fn parse_peer_args(peers: &[String]) -> anyhow::Result<Vec<Peer>> {
    peers
        .iter()
        .map(|s| {
            let (iface, path) = s
                .split_once('=')
                .ok_or_else(|| anyhow::anyhow!("--peer expects `interface=path`, got `{s}`"))?;
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
        .collect()
}

fn read_component_bytes(component: &std::path::Path) -> anyhow::Result<Vec<u8>> {
    if component.as_os_str() == "-" {
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut std::io::stdin(), &mut buf)
            .map_err(|e| anyhow::anyhow!("read component from stdin: {e}"))?;
        Ok(buf)
    } else {
        std::fs::read(component)
            .map_err(|e| anyhow::anyhow!("read component {}: {e}", component.display()))
    }
}

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
/// writer, `cdz`'s `--store` output) and readers (this crate's `resolve_nfc_by_hash` + the
/// runtime-dep resolver) all address blobs with THIS function. A reader that content-verifies with a
/// different primitive will mismatch every fetch.
///
/// - **Address:** `Hash::of(HashTag::Blob, component_bytes)` rendered base62 (45 chars, `0-9A-Za-z`).
/// - **Store layout:** a pure CAS — `<base62hash>.wasm` per component. A component's dependencies are
///   resolved from the inline `+<hash>` in its own import names (self-describing — operator directive
///   2026-08-23: no mapping file passed to any executable). A `runtime.toml` may sit at the store root as
///   an INFORMATIONAL listing of the heap builds, but no executable reads it to resolve anything.
/// - **NFC dependency:** the value-heap runtime imports `cadenza:nfc/normalize@0.0.0+<hash>` (the NFC
///   component's content address stamped inline at build time); it is resolved by that hash →
///   `<store>/<hash>.wasm`, then content-verified here — exactly like a program's OWN runtime dep, which
///   carries its hash in the import name the same way (`cadenza:runtime/heap@0.0.0+<hash>`). Every dep at
///   every level is self-describing.
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
/// flag > `CDZ_STORE` > compiled default — matching `resolve_nfc_by_hash`'s NFC-component resolution so a
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
/// (`resolve_nfc_by_hash` reads `<store>/<hash>.wasm` by the heap import's inline nfc hash). `has_runtime` = a runtime
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
        // CDZ_STORE/default (handled downstream in resolve_nfc_by_hash), so no dir is pinned here.
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
