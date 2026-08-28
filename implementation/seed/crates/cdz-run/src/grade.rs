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
    GTrial, Grade, Outcome as GradeOutcome, check_live_objects, decode_test_run, exec_exit,
    grade_run,
};

use crate::{
    HostResponse, Outcome, Peer, RunOpts, run_with_live_objects, run_with_peers_live_objects,
};

/// Grade `component_bytes` (the emitted wasm; `None` when the compile was refused) against `test_run_ast`.
/// `component_name` (a `(wit-world …)` case's `(component-name …)`) qualifies a trial's call as
/// `<iface>#<export>`. Compile outcomes (error/declines) + warns are graded from
/// `compile_status`/`compile_diag` by the shared grader — no wasm run. Returns the process exit code (`0`
/// pass/todo, `1` on the first fail).
#[allow(clippy::too_many_arguments)] // the corpus exec's full grade surface (artifact + metadata + baseline)
pub fn grade(
    component_bytes: Option<&[u8]>,
    test_run_ast: &[u8],
    runtime: Option<Vec<u8>>,
    runtime_cache_dir: Option<std::path::PathBuf>,
    component_name: Option<&str>,
    compile_status: i32,
    compile_diag: &str,
    baseline: Option<&str>,
    // Cross-component PROVIDER peers (`--peer <iface>=<wasm>`) the CONSUMER imports. A `(peer …)` corpus
    // case MUST be graded with its peers COMPOSED — the consumer's imported interface is bound by
    // forwarding the peer's exported funcs over the shared runtime instance (`run_with_peers`). Empty for a
    // plain single-component case (the common path). Without this, a peer case's imported interface falls
    // through to an unbound host-call and grades "no recorded response" (the corpus-29 nix reds).
    peers: &[Peer],
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
    // component, and map cdz-run's `Outcome` → the shared grade `Outcome` (carrying the observed
    // host-calls). It ALSO reads the value-heap runtime's live-cell count when the component imports the
    // runtime (a HEAP case) — `run_with_live_objects` returns `Some(n)` then, `None` for a no-heap
    // scalar/const program. The count of EVERY runnable trial is collected (in trial order) for the opt-out
    // balance assertion below: a multi-call case must balance on EACH call, not just call[0]. Checking only
    // the first call silently false-greened leaks that appear (or scale) on calls 2+ — the systemic gate
    // hole this harness owns. A no-heap trial contributes `None` and is skipped when balancing.
    let mut per_trial_live: Vec<Option<u32>> = Vec::new();
    let result = grade_run(&test_run, compile_status, compile_diag, |trial: &GTrial| {
        let export = match (&trial.call, component_name) {
            // A `(call-method …)` case has no export (empty) — leave it None so the run routes to the
            // value-resource escape driver, which the named member reach keys off.
            (Some(c), _) if c.method.is_some() => None,
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
        // A `(then …)` two-call continuation and a `(drop)` clause on the trial's call drive the closure
        // resource the same way the direct gate does (`--call-twice`/`--then-arg`, `--drop-handle`), so a
        // `(then)`/`(drop)` case grades identically on the nix grade path — not first-call-only / undropped.
        let second_call: Option<&[String]> =
            trial.call.as_ref().and_then(|c| c.second_call.as_deref());
        let drop_handle = trial.call.as_ref().map(|c| c.drop_handle).unwrap_or(false);
        // A `(call-method <member>)` case reaches a named value-resource member on the grade path too (no
        // export → routes to the escape driver, which reaches `<member>` instead of `encode`).
        let call_member: Option<&str> = trial.call.as_ref().and_then(|c| c.method.as_deref());
        // A `(peer …)` case composes its providers (the consumer's imported interface is bound by
        // forwarding the peer's exported funcs over the shared runtime); a plain case runs the consumer
        // alone. Both read the shared runtime's live-cell count for the heap-balance assertion.
        let (outcome, observed, live) = if peers.is_empty() {
            run_with_live_objects(
                component_bytes,
                &opts,
                second_call,
                drop_handle,
                call_member,
            )?
        } else {
            // A COMPOSE-TIME REJECT (arity/type/missing-op) is the peer case's OUTCOME, not a harness
            // error: the corpus models it as `(trap "signature mismatch")` / `(trap "type mismatch")` /
            // `(trap "does not export op")` (authoring rule), and the shared grader classifies those
            // reasons to CDZ0705/CDZ0706. So map the compose `Err` to a graded `Trap` (with the reject
            // message as the reason) rather than propagating it as a hard grade error — otherwise a
            // reject case exits non-zero instead of grading against its expected trap. A successful
            // compose returns its outcome normally; no live-count on a reject (no run happened).
            match run_with_peers_live_objects(
                component_bytes,
                peers,
                &opts,
                second_call,
                drop_handle,
                call_member,
            ) {
                Ok(triple) => triple,
                Err(e) => (Outcome::Trap(format!("{e}")), Vec::new(), None),
            }
        };
        per_trial_live.push(live);
        Ok(match outcome {
            Outcome::Value(v) => GradeOutcome::Value(v, observed),
            Outcome::Trap(t) => GradeOutcome::Trap(t),
        })
    })?;

    // Heap-balance assertion (corpus-infra OPT-OUT default): a HEAP-importing case must end at its expected
    // live-cell count after EACH call — the default is 0 (no leak / no double-free) for a case with no
    // `(live-objects …)` clause, or N for `(live-objects N)` / `(live-objects known-leak N)`. A no-heap
    // trial contributes `None` and is skipped (nothing to balance, never a false fail), as is a refused/
    // unrun case (`per_trial_live` empty). Every heap trial is checked — not just call[0] — so a multi-call
    // leak on a later call is caught (`check_live_objects`, tested in `cdz-corpus-grade`). The count is
    // meaningful only on the DEBUG-COUNTERS runtime the exec passes via `--runtime` (→ `runtime` here); the
    // shipped runtime reports 0 vacuously.
    let mut result = result;
    if let Some(msg) = check_live_objects(&per_trial_live, test_run.live_objects) {
        result.grade = std::mem::replace(&mut result.grade, Grade::Pass).worse(Grade::Fail(msg));
    }

    // The exit reproduces `xtask gate --check` when a baseline is supplied (fail ONLY on a pass→not-pass
    // regression; a baseline-todo/absent case that is now todo/fail is not a --check failure), else fails on
    // any outright Fail (the miscompile check).
    Ok(exec_exit(&result, &test_run.description, baseline))
}
