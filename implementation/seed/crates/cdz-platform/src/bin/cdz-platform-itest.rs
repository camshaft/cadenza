//! The platform integration-test executable (`design/cadenza-platform.md` §9).
//!
//! Takes a single **Cadenza binary AST** that describes the entire run — the program blobs, the tasks to
//! spawn, and the system reducer — decodes it (`cdz_platform::testing::HarnessSpec`), drives it through the
//! platform under the bach simulator over a real [`WasmProgramStore`], and prints the observation log. The
//! description is a language-neutral Cadenza value, not an argv convention: a program blob is opaque bytes,
//! given inline in the AST or by a path this executable reads, so a run is a self-contained value a checker
//! can also produce. That rendered log is what a checker asserts over (a checker component is a later slice;
//! for now a caller — e.g. the nix `--features host` check — asserts on the printed log).
//!
//! Usage: `cdz-platform-itest <harness.ast>`. Exit 0 on a completed run; exit 2 on a usage/IO/decode error.
//!
//! Behind the `testing` (harness + observation log + AST decoder) and `host` (the wasm program store that
//! instantiates the blobs) features, so the routine light build pulls in neither the harness nor wasmtime.

use cdz_platform::testing::{
    HarnessSpec, ObservationLog, RecordingBlobStore, RecordingKvStore, render,
};
use cdz_platform::{
    BachRuntime, BlobStore, Bytes, HostId, InMemoryBlobStore, InMemoryKvStore,
    InMemoryReducerGraph, KvStore, Origin, ReducerId, Runtime, WasmProgramStore,
};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

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
        Ok(log) => {
            print!("{log}");
            ExitCode::SUCCESS
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

/// Resolve the description's blobs (reading any `path`-sourced blob from disk), seed a content-addressed
/// store, drive the run to quiescence under bach over a [`WasmProgramStore`], and return the rendered log.
fn run(spec: HarnessSpec) -> Result<String, RunError> {
    let harness = spec.build(|path| {
        std::fs::read(path)
            .map(Bytes::from)
            .map_err(|e| RunError(format!("cannot read blob {path}: {e}")))
    })?;
    // One observation log, shared by the run's event tap AND by each reducer's key-value and blob stores, so
    // a reducer's KV and blob calls are recorded in the one ordered log alongside the events it folds (§7/§8/
    // §9) — the whole of the observation log, not just its event half.
    let log = ObservationLog::new();
    let host = host();
    let store_log = log.clone();
    let run = harness.host(host).log(log).run(move |cas| {
        // Each reducer gets a fresh in-memory key-value and content store, each wrapped in a recording
        // decorator over the shared log and attributed to that reducer's `Origin`.
        let kv_log = store_log.clone();
        let make_kv: Arc<dyn Fn(ReducerId) -> Box<dyn KvStore> + Send + Sync> =
            Arc::new(move |id| {
                Box::new(RecordingKvStore::new(
                    InMemoryKvStore::new(),
                    Origin { reducer: id, host },
                    kv_log.clone(),
                    BachRuntime::now as fn() -> u64,
                ))
            });
        let blob_log = store_log.clone();
        let make_blobs: Arc<dyn Fn(ReducerId) -> Box<dyn BlobStore> + Send + Sync> =
            Arc::new(move |id| {
                Box::new(RecordingBlobStore::new(
                    InMemoryBlobStore::new(),
                    Origin { reducer: id, host },
                    blob_log.clone(),
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
    });
    Ok(render(&run.records))
}

#[cfg(test)]
mod tests {
    use super::run;
    use cdz_platform::Bytes;
    use cdz_platform::testing::{BlobSource, BlobSpec, HarnessSpec};

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
            spawns: vec![cdz_platform::testing::SpawnSpec::new("greeter", "greeter")],
            deliveries: vec![],
        };
        let log = run(spec).expect("inline blobs never invoke the path loader");
        assert!(
            log.contains("spawn \"greeter\""),
            "the run records the spawn of the named blob:\n{log}"
        );
    }
}
