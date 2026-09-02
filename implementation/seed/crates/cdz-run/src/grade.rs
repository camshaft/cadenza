//! Grade a compiled WASM component against a shredded `test-run.ast` — the exec phase of the corpus nix
//! caching pipeline for the wasm backend (`design/DESIGN-corpus-nix-per-case-caching.md`).
//!
//! The GRADE compare (decode the test-run, compare an outcome to each trial's expectation, the host-call
//! sequence + warns checks, the verdict) is BACKEND-INDEPENDENT and lives in `cdz-corpus-grade`, shared
//! with the rust backend's `cdz-rust-run`. This module supplies only the wasm-specific piece: run each
//! runnable trial with `cdz-run`'s own machinery (`run_capturing`) and hand the outcome to the shared
//! `grade_run` orchestrator.

use std::process::ExitCode;

use anyhow::{Context, Result};
use cdz_corpus_grade::{
    GTrial, Grade, Outcome as GradeOutcome, Verdict, check_live_objects_scalar, decode_test_run,
    exec_exit, expect_is_scalar_return, grade_run,
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
    // The STRUCTURED diagnostics wire (`KIND_DIAGNOSTICS`) for the compiled program, when the compile phase
    // captured it — feeds `grade_diag_quality` so a case's `(fix …)`/`(count …)` facets are asserted. `None`
    // when uncaptured (diagnostic-QUALITY grading OFF; the code+message checks still run from `compile_diag`).
    diag_wire: Option<&[u8]>,
    // The `cdz check --diagnostics-wire` capture for the SAME case (C1 check-vs-compile parity, #7143):
    // forwarded to `grade_run`, which reds if `cdz check` misses a coded fault `cdz compile` rejects. `None`
    // (today's callers) = parity OFF; the upstream per-case capture (v-nix) flips it on. Purely additive.
    check_diag: Option<&[u8]>,
    baseline: Option<&str>,
    // CLASSIFY mode (`--emit-verdict PATH`, gate-delete `--save` replacement): when set, write this case's
    // current verdict (`<tag>\t<description>`) to PATH and return success WITHOUT the baseline regression
    // check — the per-case half of the nix `.#corpus-verdicts` harvest. Takes precedence over `baseline`.
    emit_verdict: Option<&std::path::Path>,
    // Cross-component PROVIDER peers (`--peer <iface>=<wasm>`) the CONSUMER imports. A `(peer …)` corpus
    // case MUST be graded with its peers COMPOSED — the consumer's imported interface is bound by
    // forwarding the peer's exported funcs over the shared runtime instance (`run_with_peers`). Empty for a
    // plain single-component case (the common path). Without this, a peer case's imported interface falls
    // through to an unbound host-call and grades "no recorded response" (the corpus-29 nix reds).
    peers: &[Peer],
    // LEAK-CEILING tolerance (the `--tolerate-fewer-live-objects` flag): on a KNOWN-LEAK case, a live-cell
    // count <= the pinned ceiling PASSES (the path reclaimed more — strictly safer). Opted into ONLY by the
    // corpus-cadenza cadenza-hop exec (the direct wasm exec leaves it false → exact `== N` drift guard). See
    // [`leak_ceiling_clamp`]. `false` for the normal single-path grade.
    tolerate_fewer_live_objects: bool,
    // PRECOMPILED (seq-250 AOT corpus-exec): the `component_bytes` and `runtime` handed in are serialized
    // `.cwasm` artifacts to `Component::deserialize`, not `.wasm` to JIT — set on every `RunOpts` below so
    // the cranelift-free exec runs them. `false` = the JIT path, unchanged.
    precompiled: bool,
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
    // Parallel to `per_trial_live` (one entry per trial, same order): does the trial RETURN a heap-free
    // scalar? Fed to `check_live_objects_scalar` so a later heap-RETURN trial's 0-check is skipped (#7527).
    let mut per_trial_scalar: Vec<bool> = Vec::new();
    let result = grade_run(
        &test_run,
        compile_status,
        compile_diag,
        diag_wire,
        check_diag,
        |trial: &GTrial| {
            // Record the scalar-vs-heap-return classification FIRST (before any early-return below), so it
            // stays index-aligned with `per_trial_live` even when a trial short-circuits (bad artifact).
            per_trial_scalar.push(expect_is_scalar_return(&trial.expect));
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
                precompiled,
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
                match run_with_live_objects(
                    component_bytes,
                    &opts,
                    second_call,
                    drop_handle,
                    call_member,
                ) {
                    Ok(triple) => triple,
                    // An emitted component that will not LOAD (wasmtime "invalid component: …", from
                    // `run_with_live_objects`'s `load_guest`) is a MISCOMPILE / bad artifact — grade it a
                    // FAIL, do NOT let the error crash the harvest. This makes the grade harvest ROBUST to a
                    // miscompile (it records the case fail faithfully + is tracked, flipping to pass when the
                    // emit is fixed) instead of one un-loadable component wedging the whole re-baseline —
                    // the same class of hardening as "a trap grades fail, not a crash". Scoped to the load
                    // failure: a runtime/host-infra error (a genuine harness fault) still propagates, so it
                    // is never silently masqueraded as a case fail.
                    Err(e) if format!("{e}").contains("invalid component") => {
                        per_trial_live.push(None);
                        return Ok(GradeOutcome::BadArtifact(format!("{e}")));
                    }
                    Err(e) => return Err(e),
                }
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
        },
    )?;

    // Heap-balance assertion (corpus-infra OPT-OUT default): a HEAP-importing case must end at its expected
    // live-cell count after EACH call — the default is 0 (no leak / no double-free) for a case with no
    // `(live-objects …)` clause, or N for `(live-objects N)` / `(live-objects known-leak N)`. A no-heap
    // trial contributes `None` and is skipped (nothing to balance, never a false fail), as is a refused/
    // unrun case (`per_trial_live` empty). Every heap trial is checked — not just call[0] — so a multi-call
    // leak on a later call is caught (`check_live_objects`, tested in `cdz-corpus-grade`). The count is
    // meaningful only on the DEBUG-COUNTERS runtime the exec passes via `--runtime` (→ `runtime` here); the
    // shipped runtime reports 0 vacuously.
    let mut result = result;
    // seq-15 PURE-BINARY leak semantics (operator ruling): a KNOWN-LEAK case is NOT count-checked — its leak
    // magnitude does not matter, so the balance assertion is SKIPPED. Instead surface a non-blocking TIGHTEN
    // CANDIDATE advisory when it now measures fully clean (its reclaim fix landed → the `(live-objects
    // known-leak)` marker can be dropped). A CLEAN case (no known-leak marker) asserts its EXACT residual on
    // EVERY heap trial: > expected = a clean→leak regression (the signal the corpus MUST catch), < expected =
    // an over-free/UAF risk. The `tolerate_fewer_live_objects` leak-ceiling path is vestigial under binary
    // (a known-leak case never reaches a ceiling check); retained only for signature stability.
    let _ = tolerate_fewer_live_objects;
    if test_run.live_objects_known_leak {
        if cdz_corpus_grade::known_leak_now_clean(&per_trial_live) {
            eprintln!(
                "TIGHTEN CANDIDATE: {} — a (live-objects known-leak) case now measures 0 live cells on every \
                 heap trial; its reclaim fix has landed, drop the known-leak marker",
                test_run.description
            );
        }
    } else if let Some(msg) = check_live_objects_scalar(
        &per_trial_live,
        test_run.live_objects,
        test_run.live_objects_per_call.as_deref(),
        &per_trial_scalar,
    ) {
        result.grade = std::mem::replace(&mut result.grade, Grade::Pass).worse(Grade::Fail(msg));
    }

    // The exit reproduces `xtask gate --check` when a baseline is supplied (fail ONLY on a pass→not-pass
    // regression; a baseline-todo/absent case that is now todo/fail is not a --check failure), else fails on
    // any outright Fail (the miscompile check).
    // CLASSIFY mode: emit `<tag>\t<description>` (tag from `Grade::verdict` — the coarse pass/todo/fail
    // vocab `.gate-baseline` records) + exit 0, skipping the baseline regression check. Precedence over
    // `--baseline`: a save/harvest run classifies the CURRENT state, it never regression-fails.
    if let Some(path) = emit_verdict {
        let tag = match result.grade.verdict() {
            Verdict::Pass => "pass",
            Verdict::Todo => "todo",
            Verdict::Fail => "fail",
        };
        std::fs::write(path, format!("{tag}\t{}\n", test_run.description))
            .with_context(|| format!("writing verdict to {}", path.display()))?;
        return Ok(ExitCode::SUCCESS);
    }
    // WASM: the `.gate-baseline` is the FULL-corpus harvest (no legitimately-absent cases), so #3984 stays
    // strict — an absent-or-todo case that fails reds. Not membership-only.
    Ok(exec_exit(&result, &test_run.description, baseline, false))
}
