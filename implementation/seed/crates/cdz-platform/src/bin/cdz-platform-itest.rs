//! The platform integration-test executable (`design/cadenza-platform.md` §9).
//!
//! Takes a single **Cadenza binary AST** that describes the entire run — the program blobs, the tasks to
//! spawn, and the system reducer — decodes it (`cdz_platform::testing::HarnessSpec`), drives it through the
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
//! exit 1 on a failing checker (its reasons print to stderr); exit 2 on a usage/IO/decode error.
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
    BlobSource, CheckOutcome, Harness, HarnessSpec, ObservationLog, RecordingBlobStore,
    RecordingKvStore, SpawnSpec, check_message, render, verdict_in,
};
use cdz_platform::{
    BachRuntime, BlobStore, Bytes, HostId, InMemoryBlobStore, InMemoryKvStore,
    InMemoryReducerGraph, KvStore, Origin, ReducerId, Runtime, WasmProgramStore,
};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

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
    let spec = match HarnessSpec::decode(&spec_bytes) {
        Ok(spec) => spec,
        Err(e) => {
            eprintln!("{}: {e}", spec_path.display());
            return ExitCode::from(2);
        }
    };

    match run(spec) {
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
         path), the tasks to spawn, and the system reducer. The run's observation log is printed to stdout."
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
    let make_blobs: Arc<dyn Fn(ReducerId) -> Box<dyn BlobStore> + Send + Sync> =
        Arc::new(move |id| {
            Box::new(RecordingBlobStore::new(
                InMemoryBlobStore::new(),
                Origin { reducer: id, host },
                log.clone(),
                BachRuntime::now as fn() -> u64,
            ))
        });
    WasmProgramStore::new(
        cas,
        make_blobs,
        make_kv,
        Arc::new(InMemoryReducerGraph::new()),
    )
    .expect("build the wasm program store (the wasm engine must initialize)")
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

    // The main run: drive the described scenario to quiescence, recording into one shared log.
    let main_harness = spec.build(load_blob)?;
    let main_log = observation_log();
    let store_log = main_log.clone();
    let main_run = main_harness
        .host(host)
        .log(main_log)
        .run(move |cas| wasm_store(cas, store_log, host));

    // The checker run: if a checker was named, spawn it, deliver it the whole main log, and read its verdict.
    // A named checker that emits no verdict (e.g. its bytes are not a valid component, so it never runs) is a
    // failed check — the run declared a check that did not report.
    let outcome = checker.map(|(name, bytes)| {
        let checker_log = observation_log();
        let store_log = checker_log.clone();
        let checker_run = Harness::new(CHECKER_SYSTEM)
            .blob(
                CHECKER_SYSTEM,
                Bytes::from_static(b"cdz-platform-itest:checker-no-system"),
            )
            .blob(name.clone(), bytes)
            .spawn(SpawnSpec::new("checker", name))
            .deliver("checker", check_message(&main_run.records))
            .host(host)
            .log(checker_log)
            .run(move |cas| wasm_store(cas, store_log, host));
        verdict_in(&checker_run.records)
            .unwrap_or_else(|| CheckOutcome::fail("the checker emitted no verdict"))
    });

    Ok(Report {
        log: render(&main_run.records),
        outcome,
    })
}

#[cfg(test)]
mod tests {
    use super::run;
    use cdz_platform::Bytes;
    use cdz_platform::testing::{BlobSource, BlobSpec, CheckOutcome, HarnessSpec, SpawnSpec};

    #[test]
    fn a_run_records_the_spawn_of_each_blob_even_when_the_bytes_are_not_a_component() {
        // Drive the whole pipeline (build the harness from a spec, seed the blob store, build the
        // WasmProgramStore, spawn under bach, render) on inline opaque bytes that are NOT a valid component:
        // the program store declines to instantiate (no crash), but the run still records the spawn the
        // harness assigned the blob's name — proving the executable wires a decoded HarnessSpec over the wasm
        // program store end-to-end. (A real component's birth is exercised by the nix `--features host`
        // check against a built guest.)
        let spec = HarnessSpec {
            system: "$system".to_string(),
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
            system: "$system".to_string(),
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
        };
        let report = run(spec).expect("inline blobs never invoke the path loader");
        assert!(
            matches!(report.outcome, Some(CheckOutcome::Fail { .. })),
            "a named checker that emits no verdict fails the run: {:?}",
            report.outcome
        );
    }

    #[test]
    fn an_unregistered_checker_blob_is_an_error() {
        // Naming a checker blob that the run does not register is a usage error, not a silent no-op.
        let spec = HarnessSpec {
            system: "$system".to_string(),
            run_for: None,
            blobs: vec![BlobSpec {
                name: "$system".to_string(),
                source: BlobSource::Inline(Bytes::from_static(b"itest:no-system-reducer")),
            }],
            spawns: vec![],
            deliveries: vec![],
            checker: Some("absent".to_string()),
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
            system: "$system".to_string(),
            run_for: None,
            blobs: vec![BlobSpec {
                name: "$system".to_string(),
                source: BlobSource::Inline(Bytes::from_static(b"itest:no-system-reducer")),
            }],
            spawns: vec![SpawnSpec::new("t", "undeclared")],
            deliveries: vec![],
            checker: None,
        };
        assert!(
            run(spec).is_err(),
            "an undeclared spawn blob is rejected before the run, not panicked on"
        );
    }
}
