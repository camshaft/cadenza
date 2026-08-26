//! Grade a compiled WASM component against a shredded `test-run.ast` — the exec phase of the corpus nix
//! caching pipeline for the wasm backend (`design/DESIGN-corpus-nix-per-case-caching.md`).
//!
//! The GRADE compare (decode the test-run, compare an outcome to each trial's expectation, the host-call
//! sequence + warns checks, the verdict) is BACKEND-INDEPENDENT and lives in `cdz-corpus-grade`, shared
//! with the rust backend's `cdz-rust-run`. This module supplies only the wasm-specific piece: run each
//! runnable trial with `cdz-run`'s own machinery (`run_capturing`) and hand the outcome to the shared
//! `grade_run` orchestrator.

use std::process::ExitCode;

use anyhow::Result;
use cdz_corpus_grade::{
    GTrial, Outcome as GradeOutcome, decode_test_run, grade_run, print_verdict,
};

use crate::{HostResponse, Outcome, RunOpts, run_capturing};

/// Grade `component_bytes` (the emitted wasm; `None` when the compile was refused) against `test_run_ast`.
/// `component_name` (a `(wit-world …)` case's `(component-name …)`) qualifies a trial's call as
/// `<iface>#<export>`. Compile outcomes (error/declines) + warns are graded from
/// `compile_status`/`compile_diag` by the shared grader — no wasm run. Returns the process exit code (`0`
/// pass/todo, `1` on the first fail).
pub fn grade(
    component_bytes: Option<&[u8]>,
    test_run_ast: &[u8],
    runtime: Option<Vec<u8>>,
    runtime_cache_dir: Option<std::path::PathBuf>,
    component_name: Option<&str>,
    compile_status: i32,
    compile_diag: &str,
) -> Result<ExitCode> {
    let test_run = decode_test_run(test_run_ast)?;
    // The recorded host-response tape, shared across every trial's run.
    let host_responses: Vec<HostResponse> = test_run
        .host_responses
        .iter()
        .map(|(op, value)| HostResponse {
            op: op.clone(),
            value: value.clone(),
        })
        .collect();

    // The wasm-specific trial runner: build the call (qualified for a world-imposed export), run the
    // component, and map cdz-run's `Outcome` → the shared grade `Outcome` (carrying the observed host-calls).
    let result = grade_run(&test_run, compile_status, compile_diag, |trial: &GTrial| {
        let export = match (&trial.call, component_name) {
            (Some(c), Some(iface)) => Some(format!("{iface}#{}", c.export)),
            (Some(c), None) => Some(c.export.clone()),
            (None, _) => None, // invoke the sole export
        };
        let args = trial
            .call
            .as_ref()
            .map(|c| c.args.clone())
            .unwrap_or_default();
        let component_bytes = component_bytes.ok_or_else(|| {
            anyhow::anyhow!(
                "grade: an output/trap case compiled (status 0) but no component was supplied"
            )
        })?;
        let opts = RunOpts {
            export,
            args,
            runtime: runtime.clone(),
            runtime_cache_dir: runtime_cache_dir.clone(),
            host_responses: host_responses.clone(),
        };
        let (outcome, observed) = run_capturing(component_bytes, &opts)?;
        Ok(match outcome {
            Outcome::Value(v) => GradeOutcome::Value(v, observed),
            Outcome::Trap(t) => GradeOutcome::Trap(t),
        })
    })?;

    Ok(print_verdict(&result, &test_run.description))
}
