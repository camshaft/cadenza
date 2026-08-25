//! The platform integration-test executable (`design/cadenza-platform.md` §9).
//!
//! Takes a single **Cadenza binary AST** that describes the entire run — the program blobs, the tasks to
//! spawn, and the event registry — decodes it (`cdz_platform::testing::HarnessSpec`), drives it through the
//! platform under the bach simulator over a real [`WasmProgramStore`], and prints the observation log. The
//! description is a language-neutral Cadenza value, not an argv convention: a program blob is opaque bytes,
//! given inline in the AST or by a path this executable reads, so a run is a self-contained value.
//!
//! If the description names a **checker** program, the executable runs it after the main run: it spawns the
//! checker as a reducer, delivers it the whole observation log (a `check` message), and reads the pass/fail
//! `verdict` it emits (§9) — the harness just executes a wasm reducer, knowing nothing of how the checker was
//! authored. The main log always prints to stdout; the exit code carries the verdict, so a caller (the nix
//! `--features host` check) enforces the check by exit code alone.
//!
//! Usage: `cdz-platform-itest <harness.ast>`. Exit 0 on a completed run with no checker or a passing checker;
//! exit 1 on a failing checker (its reasons print to stderr); exit 2 on a usage/IO/decode error; exit 3 if the
//! run exceeds its wall-clock timeout (a guest hung). A run reaches quiescence in ~milliseconds (bach jumps
//! virtual time), so the generous default (120s, override with `CDZ_ITEST_TIMEOUT_SECS`) only ever fires on a
//! genuine hang — a guest that infinite-loops inside a single fold, which bach's virtual-time horizon cannot
//! bound — making it fail cleanly and diagnosably rather than hanging the CI derivation until it is reclaimed.
//!
//! Set `CDZ_ITEST_TRACE` (to any value) to STREAM every observation to stdout the moment it is recorded
//! (flushed per line), so a run that gets stuck in a loop or crashes before the checker still shows its
//! progress up to that point — the final rendered log otherwise only appears once the run completes, which is
//! too late to debug a run that hangs. Streaming goes to stdout (so `cdz-platform-itest … | tee run.log`
//! captures it) and, when on, REPLACES the final rendered log (the stream already emitted every line, in the
//! same order and format), so stdout carries each line exactly once.
//!
//! Behind the `testing` (harness + observation log + AST decoder) and `host` (the wasm program store that
//! instantiates the blobs) features, so the routine light build pulls in neither the harness nor wasmtime.

use cdz_platform::testing::{
    BlobSource, CheckOutcome, Harness, HarnessSpec, ObservationLog, PureRun, RecordingBlobStore,
    RecordingDelivery, RecordingGraph, RecordingKvStore, RecordingProvenance,
    RecordingRejectedSink, RecordingRun, SpawnSpec, check_message, no_verdict_reason, render,
    verdict_in,
};
use cdz_platform::{
    BachRuntime, BlobStore, Bytes, Delivery, HostId, InMemoryBlobStore, InMemoryKvStore,
    InMemoryReducerGraph, KvStore, NoDelivery, NoProvenance, Origin, ProgramHash, Provenance,
    ReducerGraph, ReducerId, RejectedSink, RunSink, Runner, Runtime, WasmProgramStore,
};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

// jemalloc as this executable's global allocator (operator suggestion 2026-08-23): the run drives a whole
// platform + wasmtime + the observation log, which is allocation-heavy, so a better allocator cuts overhead
// and fragmentation. The choice is invisible to the recorded log (seq/time/id are logical, not addresses),
// so it does not affect the run's Bach determinism. Not on msvc (jemalloc does not build there); the binary
// only ships for the host targets (Linux/macOS) the nix `--features host` check builds anyway.
#[cfg(not(target_env = "msvc"))]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let Some(spec_path) = args.next().map(PathBuf::from) else {
        return usage();
    };
    if args.next().is_some() {
        return usage();
    }

    let spec_bytes = match std::fs::read(&spec_path) {
        Ok(bytes) => bytes,
        Err(e) => {
            eprintln!("cannot read {}: {e}", spec_path.display());
            return ExitCode::from(2);
        }
    };
    // Decode, resolving `BlobBytes(<name>)` / `BlobHash(<name>)` references against the run's blobs. A blob
    // supplied by path (how a program is provided) has its bytes materialized by reading the path here — so a
    // `BlobHash` payload (e.g. the program a `run.run` runs) resolves even though the bytes are on disk, not
    // inline. A path that fails to read yields `None`, so the reference is a clean decode error.
    let spec = match HarnessSpec::decode_with(&spec_bytes, |path| {
        std::fs::read(path).ok().map(Bytes::from)
    }) {
        Ok(spec) => spec,
        Err(e) => {
            eprintln!("{}: {e}", spec_path.display());
            return ExitCode::from(2);
        }
    };

    // Bound the run by wall-clock: bach bounds virtual time, but a guest that hangs in a fold would hang the
    // process forever. Arm the watchdog around the run and drop it the moment the run returns.
    let watchdog = arm_watchdog(parse_timeout(std::env::var(TIMEOUT_ENV).ok().as_deref()));
    let result = run(spec);
    drop(watchdog);

    match result {
        Ok(report) => {
            // With live tracing on, every record was already streamed to stdout as it happened, so printing
            // the rendered log would duplicate every line — emit it only when NOT tracing.
            if !tracing_on() {
                print!("{}", report.log);
            }
            match report.outcome {
                // No checker configured, or the checker passed: a successful run.
                None | Some(CheckOutcome::Pass) => ExitCode::SUCCESS,
                // The checker failed (or emitted no verdict): report the reasons and exit nonzero, so a
                // caller (the nix `--features host` check) enforces the verdict by exit code alone.
                Some(CheckOutcome::Fail { reasons }) => {
                    eprintln!("checker FAILED:");
                    for reason in reasons {
                        eprintln!("  - {reason}");
                    }
                    ExitCode::from(1)
                }
            }
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(2)
        }
    }
}

fn usage() -> ExitCode {
    eprintln!(
        "usage: cdz-platform-itest <harness.ast>\n  \
         the argument is a Cadenza binary AST describing the whole run — its program blobs (inline or by \
         path), the tasks to spawn, and the event registry. The run's observation log is printed to stdout."
    );
    ExitCode::from(2)
}

/// An error building or resolving a run — a blob path this executable could not read.
#[derive(Debug)]
struct RunError(String);

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// This node's host id — stamped on the `from` of routed messages and on the `Origin` of every recorded
/// store call, so events and store calls a reducer makes are attributed to the same reducer-on-host.
fn host() -> HostId {
    HostId::of(b"cdz-platform-itest")
}

/// The environment variable that turns on live log streaming. When set (to any value), every observation is
/// written to stdout the moment it is recorded, so a run that gets stuck in a loop or crashes *before* its
/// log is rendered still shows its progress up to the hang/crash — the final rendered log only appears once
/// the run completes, which is too late to debug a run that never gets there.
const TRACE_ENV: &str = "CDZ_ITEST_TRACE";

/// Whether live log streaming is on (the [`TRACE_ENV`] variable is set). When on, the run streams each record
/// to stdout as it happens *and* the final rendered log is suppressed (the stream already emitted it), so
/// stdout carries each line exactly once.
fn tracing_on() -> bool {
    std::env::var_os(TRACE_ENV).is_some()
}

/// The environment variable that overrides the wall-clock timeout, in whole seconds. Unset/empty/zero/
/// unparseable falls back to [`DEFAULT_TIMEOUT`].
const TIMEOUT_ENV: &str = "CDZ_ITEST_TIMEOUT_SECS";

/// The default wall-clock timeout — generous, because a bounded run reaches quiescence in ~milliseconds
/// (bach jumps virtual time). It only guards against a runaway guest hanging the process indefinitely.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);

/// Resolve the wall-clock timeout from the raw [`TIMEOUT_ENV`] value: a positive whole-seconds override, or
/// [`DEFAULT_TIMEOUT`] when it is unset, empty, zero, or unparseable. Pure (the env read is the caller's), so
/// the policy is unit-testable without touching the process environment.
fn parse_timeout(raw: Option<&str>) -> Duration {
    match raw.and_then(|s| s.trim().parse::<u64>().ok()) {
        Some(secs) if secs > 0 => Duration::from_secs(secs),
        _ => DEFAULT_TIMEOUT,
    }
}

/// Arm a wall-clock watchdog: a background thread that force-exits the process (code 3) if the run has not
/// finished within `timeout`. bach bounds *virtual* time (the `run-for` horizon), not wall-clock, so a fold
/// that never yields — a guest stuck in an infinite loop — would otherwise hang the process (and the CI
/// derivation) until it is reclaimed; this bounds it to a clean, diagnosable failure. Returns a guard: hold
/// it for the duration of the run, then drop it — dropping disconnects the channel, so the watchdog thread
/// wakes and exits quietly. If the run instead overruns, the watchdog prints why and exits 3.
fn arm_watchdog(timeout: Duration) -> mpsc::Sender<()> {
    let (done_tx, done_rx) = mpsc::channel::<()>();
    std::thread::spawn(move || {
        if let Err(mpsc::RecvTimeoutError::Timeout) = done_rx.recv_timeout(timeout) {
            eprintln!(
                "cdz-platform-itest: run exceeded {}s without reaching quiescence — aborting (a guest \
                 likely hung in a fold; raise {TIMEOUT_ENV} to change the limit, or set {TRACE_ENV} to \
                 stream the log up to the hang)",
                timeout.as_secs()
            );
            std::process::exit(3);
        }
        // Ok(()) or Disconnected: the run finished (the guard was dropped) — cancel quietly.
    });
    done_tx
}

/// A fresh observation log for a run phase, streaming each record to stdout as it is appended when tracing is
/// on (off by default). Streaming goes to stdout — flushed per line — so `… | tee run.log` captures it and a
/// hung run still shows progress (a piped stdout is block-buffered, so an unflushed line would not appear
/// until the process exits; flushing forces each line out immediately).
fn observation_log() -> ObservationLog {
    let log = ObservationLog::new();
    if tracing_on() {
        log.on_record(|record| {
            use std::io::Write;
            let mut out = std::io::stdout().lock();
            let _ = writeln!(out, "{record}");
            let _ = out.flush();
        })
    } else {
        log
    }
}

/// The blob name of the placeholder system reducer the checker phase routes to. The checker's verdict is
/// read from the emitted-request observation, not from a system reducer handling it, so a non-component
/// placeholder is enough — a routed verdict is recorded and then declined, never crashing the run.
const CHECKER_SYSTEM: &str = "$checker-system";

/// The result of running a description: the rendered observation log of the main run, and — if the run named
/// a checker — its verdict. `outcome` is `None` when no checker was configured.
struct Report {
    log: String,
    outcome: Option<CheckOutcome>,
}

/// Resolve a spec's **unnamed dependency components** ([`HarnessSpec::deps`]) to `(label, bytes)` — the
/// value-heap runtime a Cadenza guest imports and that runtime's NFC dependency, injected into the spec (by
/// nix, for a reproducible run) so the run is self-contained. Each is seeded into the run's content-addressed
/// store so a guest's content-addressed imports resolve (`host::…::bind_dependencies` composes them by hash);
/// without them a Cadenza guest cannot instantiate and silently never folds. The label is inert — the CAS
/// keys by the bytes' content hash, which is the import's `+<hash>` — and no spawn refers to a dep.
fn resolve_deps(deps: &[BlobSource]) -> Result<Vec<(String, Bytes)>, RunError> {
    deps.iter()
        .enumerate()
        .map(|(i, source)| {
            let bytes = match source {
                BlobSource::Inline(bytes) => bytes.clone(),
                BlobSource::Path(path) => load_blob(path)?,
            };
            Ok((format!("cdz-dep:{i}"), bytes))
        })
        .collect()
}

/// Add resolved dependency `components` to a harness as blobs, so a Cadenza guest's runtime (and its
/// dependencies) resolve in the run's CAS. The blob names are inert labels — a component is looked up by its
/// content hash, not by name — and no spawn refers to them.
fn with_deps(mut harness: Harness, components: &[(String, Bytes)]) -> Harness {
    for (label, bytes) in components {
        harness = harness.blob(label.clone(), bytes.clone());
    }
    harness
}

/// Read a program blob from a filesystem path, as the harness's blob loader does.
fn load_blob(path: &str) -> Result<Bytes, RunError> {
    std::fs::read(path)
        .map(Bytes::from)
        .map_err(|e| RunError(format!("cannot read blob {path}: {e}")))
}

/// A [`WasmProgramStore`] over `cas` whose per-reducer key-value and blob stores record into `log` (attributed
/// to each reducer's [`Origin`] on this `host`), so a reducer's store calls join its events in the one log
/// (§7/§8/§9). Built the same way for the main run and the checker run.
fn wasm_store(cas: Arc<dyn BlobStore>, log: ObservationLog, host: HostId) -> WasmProgramStore {
    let kv_log = log.clone();
    let make_kv: Arc<dyn Fn(ReducerId) -> Box<dyn KvStore> + Send + Sync> = Arc::new(move |id| {
        Box::new(RecordingKvStore::new(
            InMemoryKvStore::new(),
            Origin { reducer: id, host },
            kv_log.clone(),
            BachRuntime::now as fn() -> u64,
        ))
    });
    let blob_log = log.clone();
    let make_blobs: Arc<dyn Fn(ReducerId) -> Box<dyn BlobStore> + Send + Sync> =
        Arc::new(move |id| {
            Box::new(RecordingBlobStore::new(
                InMemoryBlobStore::new(),
                Origin { reducer: id, host },
                blob_log.clone(),
                BachRuntime::now as fn() -> u64,
            ))
        });
    // The graph factory records each reducer's node-side graph calls (reads + writes of the routing
    // substrate, §7) via a RecordingGraph decorator over the one shared node-wide graph — so a conformance
    // run asserts how a reducer inspected/changed the graph (e.g. an event reducer's neighbors read for a
    // contract is how it routes). The base graph is genuinely shared (all reducers see one graph); the
    // decorator only observes, attributed per reducer.
    let graph: Arc<dyn ReducerGraph> = Arc::new(InMemoryReducerGraph::new());
    let graph_log = log.clone();
    let make_graph: Arc<dyn Fn(ReducerId) -> Arc<dyn ReducerGraph> + Send + Sync> =
        Arc::new(move |id| {
            Arc::new(RecordingGraph::new(
                graph.clone(),
                Origin { reducer: id, host },
                graph_log.clone(),
                BachRuntime::now as fn() -> u64,
            )) as Arc<dyn ReducerGraph>
        });
    // The delivery factory records each reducer's privileged `deliver` (the §4 routing ACT) into the log via a
    // RecordingDelivery decorator, over a NoDelivery base — the base never lands (the itest has no live
    // routing target), but the ACT itself is what a §4 dispatch run asserts, and it is recorded regardless.
    let delivery_log = log.clone();
    let make_delivery: Arc<dyn Fn(ReducerId) -> Arc<dyn Delivery> + Send + Sync> =
        Arc::new(move |id| {
            Arc::new(RecordingDelivery::new(
                Arc::new(NoDelivery),
                Origin { reducer: id, host },
                delivery_log.clone(),
                BachRuntime::now as fn() -> u64,
            )) as Arc<dyn Delivery>
        });
    // The provenance factory records each reducer's privileged `program-of` read (which program the queried
    // reducer runs) via a RecordingProvenance decorator, over a NoProvenance base — the base answers `None`
    // (the itest wires no real system), but the read ACT + the answer are recorded regardless.
    let provenance_log = log.clone();
    let make_provenance: Arc<dyn Fn(ReducerId) -> Arc<dyn Provenance> + Send + Sync> =
        Arc::new(move |id| {
            Arc::new(RecordingProvenance::new(
                Arc::new(NoProvenance),
                Origin { reducer: id, host },
                provenance_log.clone(),
                BachRuntime::now as fn() -> u64,
            )) as Arc<dyn Provenance>
        });
    // The rejected-sink factory records each reducer's host calls REJECTED at the arg-parse guard (a
    // malformed-arg `graph` op the host early-returns from without reaching the recordable capability, §9) via
    // a RecordingRejectedSink over the one shared log — so a conformance run can assert that a (malformed)
    // host call was still performed + observed, closing the silent-observation hole. Unset by default (the
    // reducer's `rejected` is `None`, dropping it); wiring this factory makes it observed.
    let rejected_log = log.clone();
    let make_rejected: Arc<dyn Fn(ReducerId) -> Arc<dyn RejectedSink> + Send + Sync> =
        Arc::new(move |id| {
            Arc::new(RecordingRejectedSink::new(
                Origin { reducer: id, host },
                rejected_log.clone(),
                BachRuntime::now as fn() -> u64,
            )) as Arc<dyn RejectedSink>
        });
    // The run-sink factory records each reducer's synchronous pure-`run` host calls (§3) — the sub-program +
    // contract + input + outcome — via a RecordingRun over the one shared log, so a conformance run can assert
    // a reducer actually invoked `run` (a `run` leaves no request in the step, so it is otherwise unobservable,
    // §9). Unset by default (the reducer's `run_sink` is `None`, dropping it); wiring this factory makes it observed.
    let run_log = log;
    let make_run: Arc<dyn Fn(ReducerId) -> Arc<dyn RunSink> + Send + Sync> = Arc::new(move |id| {
        Arc::new(RecordingRun::new(
            Origin { reducer: id, host },
            run_log.clone(),
            BachRuntime::now as fn() -> u64,
        )) as Arc<dyn RunSink>
    });
    WasmProgramStore::new(cas, make_blobs, make_kv, make_graph)
        .expect("build the wasm program store (the wasm engine must initialize)")
        .with_delivery(make_delivery)
        .with_provenance(make_provenance)
        .with_rejected(make_rejected)
        .with_run_sink(make_run)
}

/// Drive the run to quiescence under bach over a [`WasmProgramStore`], then — if the description named a
/// `checker` program — run that checker over the completed observation log and read its verdict. The harness
/// just executes the checker as a wasm reducer: it spawns it, delivers it the whole log, and reads the
/// [`verdict`](verdict_in) request it emits (§9). Returns the rendered main log and the checker's outcome.
fn run(spec: HarnessSpec) -> Result<Report, RunError> {
    let host = host();

    // Check the description's cross-references (every named blob is declared, parents precede children,
    // task names are unique, deliveries hit a spawned task) before running — so a malformed AST is a clean
    // error, not a panic deep in name resolution.
    spec.validate().map_err(|e| RunError(e.to_string()))?;

    // A pure run is a distinct, standalone phase (§3): run one program as a pure function of its input, with
    // no system reducer, no spawns, no checker. Return before the spawn/deliver/checker flow, which it does
    // not use.
    if let Some(pure_run) = &spec.pure_run {
        return pure_run_phase(&spec, pure_run, host);
    }

    // Resolve the checker program's bytes before `build` consumes the description's blobs. The checker must
    // be one of the run's registered blobs (it is seeded into the store like any program).
    let checker = match &spec.checker {
        None => None,
        Some(name) => {
            let blob = spec.blobs.iter().find(|b| &b.name == name).ok_or_else(|| {
                RunError(format!(
                    "checker blob '{name}' is not registered with the run"
                ))
            })?;
            let bytes = match &blob.source {
                BlobSource::Inline(bytes) => bytes.clone(),
                BlobSource::Path(path) => load_blob(path)?,
            };
            Some((name.clone(), bytes))
        }
    };

    // The spec's unnamed dependency components (the value-heap runtime + its NFC dep) every Cadenza guest's
    // imports resolve against — resolved once and seeded into each run's CAS below so a Cadenza guest can
    // instantiate. Resolved before `build` consumes the spec, so the checker/pure-run phases can seed them too.
    let deps = resolve_deps(&spec.deps)?;

    // The main run: drive the described scenario to quiescence, recording into one shared log.
    let main_harness = with_deps(spec.build(load_blob)?, &deps);
    let main_log = observation_log();
    let store_log = main_log.clone();
    let main_run = main_harness
        .host(host)
        .log(main_log)
        .run(move |cas| wasm_store(cas, store_log, host));

    // The checker run: if a checker was named, spawn it, deliver it the whole main log, and read its verdict.
    // A named checker that emits no verdict is a failed check — the run declared a check that did not report —
    // and [`no_verdict_reason`] diagnoses *why* from the checker's own observations (it closed without
    // reporting, faulted, emitted on the wrong contract, or never ran), so a CI failure is actionable.
    let outcome = checker.map(|(name, bytes)| {
        let checker_log = observation_log();
        let store_log = checker_log.clone();
        let checker_base = Harness::new(CHECKER_SYSTEM)
            .blob(
                CHECKER_SYSTEM,
                Bytes::from_static(b"cdz-platform-itest:checker-no-system"),
            )
            .blob(name.clone(), bytes);
        let checker_run = with_deps(checker_base, &deps)
            .spawn(SpawnSpec::new("checker", name))
            .deliver("checker", check_message(&main_run.records))
            .host(host)
            .log(checker_log)
            .run(move |cas| wasm_store(cas, store_log, host));
        verdict_in(&checker_run.records)
            .unwrap_or_else(|| CheckOutcome::fail(no_verdict_reason(&checker_run.records)))
    });

    Ok(Report {
        log: render(&main_run.records),
        outcome,
    })
}

/// Perform a [`PureRun`] (§3): run its program as a pure function of the input — the `run` primitive
/// instantiates it with an empty capability set, so every effect it emits is denied (dropped, never routed)
/// and its only output is the fold's result. The run passes iff that output equals the expected bytes. This
/// is standalone: no system reducer, no spawns, no checker; the observation log stays empty (a pure reducer
/// touches no store), so the report's log is empty and the exit code carries the assertion.
fn pure_run_phase(
    spec: &HarnessSpec,
    pure_run: &PureRun,
    host: HostId,
) -> Result<Report, RunError> {
    // Resolve the program's bytes from the run's blobs (`validate` already checked it is declared).
    let blob = spec
        .blobs
        .iter()
        .find(|b| b.name == pure_run.program)
        .ok_or_else(|| {
            RunError(format!(
                "pure-run program '{}' is not registered with the run",
                pure_run.program
            ))
        })?;
    let bytes = match &blob.source {
        BlobSource::Inline(b) => b.clone(),
        BlobSource::Path(p) => load_blob(p)?,
    };
    let program = ProgramHash::of(&bytes);
    let contract = pure_run.contract;
    let input = pure_run.input.clone();
    let expect = pure_run.expect_output.clone();

    // Drive the async `run` under bach (deterministic), writing the result into a shared cell — the same
    // pattern the harness uses to read a run's output after the sim completes. bach jumps virtual time, so a
    // bounded pure fold settles immediately.
    let cell: Arc<Mutex<Option<Result<Bytes, RunError>>>> = Arc::new(Mutex::new(None));
    let cell_in = cell.clone();
    let log = observation_log();
    // The spec's dependency components (value-heap runtime + NFC) the pure program's own imports resolve
    // against — a pure Cadenza guest imports the runtime just like any reducer, so they must be in its CAS too.
    let deps = resolve_deps(&spec.deps)?;
    bach::sim(move || {
        use bach::ext::*;
        let cell = cell_in;
        async move {
            let mut cas = InMemoryBlobStore::new();
            for (_label, component) in &deps {
                cas.put(component.clone()).await;
            }
            cas.put(bytes).await;
            let store = wasm_store(Arc::new(cas), log, host);
            let runner = Runner::new(Arc::new(store));
            let out = runner
                .run(program, contract, input)
                .await
                .map_err(|e| RunError(format!("pure run failed: {e:?}")));
            *cell.lock().expect("pure-run result lock") = Some(out);
        }
        .group("pure-run")
        .primary()
        .spawn();
    });
    let result = cell
        .lock()
        .expect("pure-run result lock")
        .take()
        .ok_or_else(|| RunError("pure run did not complete".to_string()))?;

    let outcome = match result {
        Ok(output) if output.as_ref() == expect.as_ref() => CheckOutcome::Pass,
        Ok(output) => {
            CheckOutcome::fail(format!("pure run produced {output:?}, expected {expect:?}"))
        }
        Err(e) => CheckOutcome::fail(e.to_string()),
    };
    Ok(Report {
        log: String::new(),
        outcome: Some(outcome),
    })
}

#[cfg(test)]
mod tests {
    use super::run;
    use cdz_platform::testing::{
        BlobSource, BlobSpec, CheckOutcome, HarnessSpec, PureRun, RegistrySpec, SpawnSpec,
    };
    use cdz_platform::{Bytes, ContractId};

    #[test]
    fn a_run_records_the_spawn_of_each_blob_even_when_the_bytes_are_not_a_component() {
        // Drive the whole pipeline (build the harness from a spec, seed the blob store, build the
        // WasmProgramStore, spawn under bach, render) on inline opaque bytes that are NOT a valid component:
        // the program store declines to instantiate (no crash), but the run still records the spawn the
        // harness assigned the blob's name — proving the executable wires a decoded HarnessSpec over the wasm
        // program store end-to-end. (A real component's birth is exercised by the nix `--features host`
        // check against a built guest.)
        let spec = HarnessSpec {
            run_for: None,
            blobs: vec![
                BlobSpec {
                    name: "$system".to_string(),
                    source: BlobSource::Inline(Bytes::from_static(b"itest:no-system-reducer")),
                },
                BlobSpec {
                    name: "greeter".to_string(),
                    source: BlobSource::Inline(Bytes::from_static(b"not a wasm component")),
                },
            ],
            spawns: vec![SpawnSpec::new("greeter", "greeter")],
            deliveries: vec![],
            checker: None,
            pure_run: None,
            deps: Vec::new(),
            registry: RegistrySpec {
                default: "$system".to_string(),
                handlers: vec![],
            },
            edges: vec![],
        };
        let report = run(spec).expect("inline blobs never invoke the path loader");
        assert!(
            report.log.contains("spawn \"greeter\""),
            "the run records the spawn of the named blob:\n{}",
            report.log
        );
        assert!(report.outcome.is_none(), "no checker ⇒ no verdict");
    }

    #[test]
    fn a_named_checker_that_never_runs_is_a_failed_check() {
        // A run names a checker, but its bytes are not a valid component, so it never spawns and emits no
        // verdict. The executable treats a declared-but-silent checker as a failure — the two-phase control
        // flow runs the checker phase and enforces "a named check must report". (A real passing checker is
        // exercised by the nix `--features host` check against a compiled checker guest.)
        let spec = HarnessSpec {
            run_for: None,
            blobs: vec![
                BlobSpec {
                    name: "$system".to_string(),
                    source: BlobSource::Inline(Bytes::from_static(b"itest:no-system-reducer")),
                },
                BlobSpec {
                    name: "worker".to_string(),
                    source: BlobSource::Inline(Bytes::from_static(b"not a wasm component")),
                },
                BlobSpec {
                    name: "check".to_string(),
                    source: BlobSource::Inline(Bytes::from_static(b"not a wasm component either")),
                },
            ],
            spawns: vec![SpawnSpec::new("worker", "worker")],
            deliveries: vec![],
            checker: Some("check".to_string()),
            pure_run: None,
            deps: Vec::new(),
            registry: RegistrySpec {
                default: "$system".to_string(),
                handlers: vec![],
            },
            edges: vec![],
        };
        let report = run(spec).expect("inline blobs never invoke the path loader");
        assert!(
            matches!(report.outcome, Some(CheckOutcome::Fail { .. })),
            "a named checker that emits no verdict fails the run: {:?}",
            report.outcome
        );
        // …and the failure DIAGNOSES the cause (it never ran — its bytes are not a component), not the bare
        // "the checker emitted no verdict" — the signal a CI run (nix `--features host`) surfaces.
        let reasons = report
            .outcome
            .as_ref()
            .map(CheckOutcome::reasons)
            .unwrap_or_default();
        assert!(
            reasons.iter().any(|r| r.contains("never ran")),
            "the failure says why the checker produced no verdict: {reasons:?}"
        );
    }

    #[test]
    fn an_unregistered_checker_blob_is_an_error() {
        // Naming a checker blob that the run does not register is a usage error, not a silent no-op.
        let spec = HarnessSpec {
            run_for: None,
            blobs: vec![BlobSpec {
                name: "$system".to_string(),
                source: BlobSource::Inline(Bytes::from_static(b"itest:no-system-reducer")),
            }],
            spawns: vec![],
            deliveries: vec![],
            checker: Some("absent".to_string()),
            pure_run: None,
            deps: Vec::new(),
            registry: RegistrySpec {
                default: "$system".to_string(),
                handlers: vec![],
            },
            edges: vec![],
        };
        assert!(
            run(spec).is_err(),
            "an unregistered checker blob is rejected"
        );
    }

    #[test]
    fn a_bad_cross_reference_is_a_clean_error_not_a_panic() {
        // A spawn names a blob the run does not declare. `run` validates the description up front (before it
        // builds the harness), so a malformed AST is a clean RunError (exit 2), not a panic deep in name
        // resolution — the executable's contract for untrusted input.
        let spec = HarnessSpec {
            run_for: None,
            blobs: vec![BlobSpec {
                name: "$system".to_string(),
                source: BlobSource::Inline(Bytes::from_static(b"itest:no-system-reducer")),
            }],
            spawns: vec![SpawnSpec::new("t", "undeclared")],
            deliveries: vec![],
            checker: None,
            pure_run: None,
            deps: Vec::new(),
            registry: RegistrySpec {
                default: "$system".to_string(),
                handlers: vec![],
            },
            edges: vec![],
        };
        assert!(
            run(spec).is_err(),
            "an undeclared spawn blob is rejected before the run, not panicked on"
        );
    }

    #[test]
    fn parse_timeout_uses_the_override_or_falls_back_to_the_default() {
        use super::{DEFAULT_TIMEOUT, parse_timeout};
        use std::time::Duration;
        // A positive whole-seconds override is honored (trimmed).
        assert_eq!(parse_timeout(Some("30")), Duration::from_secs(30));
        assert_eq!(parse_timeout(Some("  45 ")), Duration::from_secs(45));
        // Unset, empty, zero, and unparseable all fall back to the generous default.
        assert_eq!(parse_timeout(None), DEFAULT_TIMEOUT);
        assert_eq!(parse_timeout(Some("")), DEFAULT_TIMEOUT);
        assert_eq!(parse_timeout(Some("0")), DEFAULT_TIMEOUT);
        assert_eq!(parse_timeout(Some("soon")), DEFAULT_TIMEOUT);
    }

    #[test]
    fn a_pure_run_whose_program_is_not_a_component_fails_with_a_reason() {
        // A pure-run spec runs the standalone Runner phase (no system/spawns/checker). Its program bytes are
        // not a valid wasm component, so the run cannot instantiate it — a failed check with a diagnosed
        // reason, not a panic. (The SUCCESS path, run -> Ok(expect_output), is exercised e2e by the nix
        // `--features host` conformance run against the compiled reducer-emit-then-close-cdz guest.)
        let spec = HarnessSpec {
            run_for: None,
            blobs: vec![
                BlobSpec {
                    name: "$system".to_string(),
                    source: BlobSource::Inline(Bytes::from_static(b"itest:no-system-reducer")),
                },
                BlobSpec {
                    name: "prog".to_string(),
                    source: BlobSource::Inline(Bytes::from_static(b"not a wasm component")),
                },
            ],
            spawns: vec![],
            deliveries: vec![],
            checker: None,
            pure_run: Some(PureRun {
                program: "prog".to_string(),
                contract: ContractId::of(b"cdz-platform.deliver"),
                input: Bytes::from_static(b"X"),
                expect_output: Bytes::from_static(b"X"),
            }),
            deps: Vec::new(),
            registry: RegistrySpec {
                default: "$system".to_string(),
                handlers: vec![],
            },
            edges: vec![],
        };
        let report = run(spec).expect("inline blobs never invoke the path loader");
        assert!(
            matches!(report.outcome, Some(CheckOutcome::Fail { .. })),
            "a pure run of a non-component fails: {:?}",
            report.outcome
        );
        let reasons = report
            .outcome
            .as_ref()
            .map(CheckOutcome::reasons)
            .unwrap_or_default();
        assert!(
            reasons.iter().any(|r| r.contains("pure run")),
            "the failure names the pure run: {reasons:?}"
        );
    }
}
