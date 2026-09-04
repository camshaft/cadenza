//! Grade a Cadenza program's emitted RUST against a shredded `test-run.ast` — the exec phase of the
//! corpus nix caching pipeline for the rust backend (`design/DESIGN-corpus-nix-per-case-caching.md`).
//!
//! The GRADE compare (decode the test-run, compare an outcome to each trial's expectation, the host-call
//! sequence + warns checks, the verdict) is BACKEND-INDEPENDENT and lives in `cdz-corpus-grade`, shared
//! with the wasm backend's `cdz-run`. This module supplies only the rust-specific piece: for each runnable
//! trial, assemble the driver source around the emitted module ([`crate::driver::build_driver_source`]),
//! compile+run it with `rustc` ([`crate::run::compile_and_run`]), and hand the outcome to the shared
//! `grade_run` orchestrator.

use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context, Result};
use cdz_corpus_grade::{
    GTrial, Grade, GradeResult, Outcome as GradeOutcome, Verdict, decode_test_run, exec_exit,
    grade_run,
};

use crate::driver::build_driver_source;
use crate::run::{Outcome as RunOutcome, RlibDirs, compile_and_run};
use crate::sig::sole_export_name;

/// Whether a decoded case is driven by a resource protocol the standalone-`.rs` rust exec path cannot
/// perform: a `(then …)` borrowed-handle TWO-CALL (`second_call`), an explicit resource `(drop)`
/// (`drop_handle`), or a `(call-method …)` value-resource member reach (`method`). All three live ONLY in
/// the wasm `cdz-run` closure/escape driver; the rust path would run only the FIRST call and silently
/// produce the single-call value (a dishonest miscompile). Such a case DECLINES → `Todo`, mirroring the
/// in-process `xtask gate` guard so the nix coarse-rust gate and the in-process gate agree.
fn declines_resource_drive(test_run: &cdz_corpus_grade::TestRun) -> bool {
    test_run.trials.iter().any(|t| {
        t.call
            .as_ref()
            .is_some_and(|c| c.second_call.is_some() || c.drop_handle || c.method.is_some())
    })
}

/// Grade `module` (the emitted `--target rust[-async]` source; `None` when the compile was refused) against
/// `test_run_ast`, printing the verdict and returning the process exit code (`0` pass/todo, `1` on the
/// first fail). A thin wrapper over [`grade_to_result`] + the shared `print_verdict`.
#[allow(clippy::too_many_arguments)] // the corpus exec's full rust grade surface (module + rlibs + metadata + baseline)
pub fn grade(
    module: Option<&str>,
    test_run_ast: &[u8],
    rlibs: &RlibDirs,
    async_mode: bool,
    compile_status: i32,
    compile_diag: &str,
    diag_wire: Option<&[u8]>,
    workdir: &Path,
    baseline: Option<&str>,
    // CLASSIFY mode (`--emit-verdict PATH`, the nix `.#corpus-verdicts-rust[-async]` harvest / `gate --save`
    // replacement): when set, write this case's current verdict (`<tag>\t<description>`) to PATH and return
    // success WITHOUT the baseline regression check — the rust analogue of `cdz-run --emit-verdict`. Takes
    // precedence over `baseline`.
    emit_verdict: Option<&Path>,
    // The case's imposed WIT-WORLD (`wit-world.ast`), present ONLY for a `(wit-world …)` case. When
    // `Some`, this RUST target DECLINES the case → `Todo`: an imposed external world runs ONLY on the wasm
    // backend (the standalone `.rs` emit has no external-world ingest; the corpus header prescribes rust/ML
    // → todo). Skips emit/compile/run entirely — otherwise the rust pipeline compiles the bare program
    // (ignoring the world) and E0425s on the world-declared export = a dishonest FAIL. Keeps declined≠error
    // (the fuzzer differential) + honestly characterizes the rust column (imposed-WIT-world = rust todo-by-
    // design). The compiler cannot self-decline this — the world is a corpus sibling clause it never sees.
    wit_world: Option<&Path>,
    // The case's PEER provider artifact (`peer-*.ast`), present ONLY for a `(peer …)`-clause case (a
    // cross-component-peer program). PRESENCE-ONLY: when `Some`, this RUST target DECLINES the case → `Todo`,
    // skipping emit/compile/run — the standalone `.rs` emit has no component model / peer boundary, so a
    // `host (A B) …` bound to a peer interface has no provider to compose against; the rust pipeline would
    // otherwise emit an unbound-peer-op panic-stub that TRAPS at runtime = a dishonest FAIL. The exact twin of
    // `wit_world` (both are wasm-only corpus sibling clauses the compiler never sees, so it cannot
    // self-decline). Keeps declined≠error (the fuzzer differential) + honestly characterizes the rust column.
    peer: Option<&Path>,
) -> Result<ExitCode> {
    let test_run = decode_test_run(test_run_ast)?;
    // A `(then …)` two-call, a `(drop)`, or a `(call-method …)` value-resource case is driven ONLY through
    // the WASM harness (`cdz-run`'s closure/escape driver: `--call-twice` / `--drop-handle` / `--call-member`).
    // The standalone-`.rs` rust exec path has NO such resource drive — it runs only the FIRST call — so a
    // compound-result `(then)` case would SILENTLY produce the single-call value (e.g. `#tuple(5 105)` where the
    // repeatable double-call expects `#tuple(#tuple(5 105) #tuple(5 105))`), a DISHONEST miscompile the nix
    // coarse-rust gate catches as a todo→fail. DECLINE it → `Todo`, the EXACT mirror of the in-process
    // `xtask gate` guard (`xtask/src/main.rs`, non-wasm two-call/drop/method → Declined): without this the two
    // paths DIVERGE (in-process declines-todo, nix emits-and-mis-runs → fail). The compiler cannot self-decline
    // — `call`/`then`/`drop`/`method` are corpus sibling clauses it never sees, same as `wit_world`/`peer`.
    let resource_driven = declines_resource_drive(&test_run);
    let result = if wit_world.is_some() {
        GradeResult {
            grade: Grade::Todo(
                "imposed WIT-world: the rust backend has no external-world ingest (wasm-only) — \
                 declines by design"
                    .to_string(),
            ),
            ran_a_trial: false,
        }
    } else if peer.is_some() {
        GradeResult {
            grade: Grade::Todo(
                "cross-component peer: the rust backend emits a standalone .rs with no component model / \
                 peer boundary — declines by design"
                    .to_string(),
            ),
            ran_a_trial: false,
        }
    } else if resource_driven {
        GradeResult {
            grade: Grade::Todo(
                "(then)/drop/method resource drive: the standalone .rs rust exec path has no borrowed-handle \
                 two-call / resource-drop / value-resource-member drive (wasm-only) — declines by design"
                    .to_string(),
            ),
            ran_a_trial: false,
        }
    } else {
        grade_to_result(
            &test_run,
            module,
            rlibs,
            async_mode,
            compile_status,
            compile_diag,
            diag_wire,
            workdir,
        )?
    };
    // The exit reproduces `xtask gate --check --target rust` when a baseline is supplied (fail ONLY on a
    // pass→not-pass regression; a baseline-todo/absent case that is now todo/fail — e.g. an imposed-WIT-world
    // reducer the rust backend declines → todo, that this pipeline compiles-without-the-world → fail — is NOT
    // a --check failure), else fails on any outright Fail (the miscompile check).
    //
    // CLASSIFY mode (`--emit-verdict`): emit `<tag>\t<description>` (tag from `Grade::verdict` — the coarse
    // pass/todo/fail vocab `.gate-baseline-rust*` records) + exit 0, skipping the baseline regression check.
    // Precedence over `baseline`: a save/harvest run classifies the CURRENT state, it never regression-fails.
    // Mirrors `cdz-run --emit-verdict` (the wasm harvest); the nix `.#corpus-verdicts-rust[-async]`
    // derivations call it per case + aggregate the lines into the rust/rust-async `.gate-baseline`s.
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
    // RUST is MEMBERSHIP-ONLY: `.gate-baseline-rust` is a CURATED SUBSET (~8962 of ~10819 — rust stays
    // incremental, no-value-heap), so a case ABSENT from it is intentionally not-covered-on-rust, NOT a
    // gate-hole. `membership_only=true` exempts an absent-and-now-failing case from the #3984 red (grade
    // IFF title ∈ baseline); a baselined `todo` that now fails STILL reds (it IS covered). This is what
    // makes the coarse-rust gate viable — the unfiltered #3984 red on an absent miscompile (v-nix's C
    // re-measure: the "oversize constant if-branch" case) is exactly the false-red this closes.
    Ok(exec_exit(&result, &test_run.description, baseline, true))
}

/// Grade a decoded `test_run` against `module`, returning the [`GradeResult`] (no printing) — the testable
/// core. RUN outcomes (`expect-output`/`expect-trap`) assemble a driver around `module` and compile+run it
/// under `workdir` (each trial in its own subdir), linking `rlibs`; COMPILE outcomes (`expect-error`/
/// `expect-declines`) + `warns` are graded from `compile_status`/`compile_diag` by the shared grader — no
/// run. `async_mode` links `cdz_rt` and reads the async signature markers.
#[allow(clippy::too_many_arguments)] // the corpus exec's full rust grade surface (module + rlibs + metadata + diag)
pub fn grade_to_result(
    test_run: &cdz_corpus_grade::TestRun,
    module: Option<&str>,
    rlibs: &RlibDirs,
    async_mode: bool,
    compile_status: i32,
    compile_diag: &str,
    diag_wire: Option<&[u8]>,
    workdir: &Path,
) -> Result<GradeResult> {
    let host_responses = test_run.host_responses.clone();
    let host_calls = test_run.host_calls.clone();

    // The rust-specific trial runner: pick the export (a `(call …)` names it; otherwise the sole emitted
    // export), assemble the driver around the emitted module, compile+run it, and map `run::Outcome` → the
    // shared grade `Outcome` (carrying the observed host-calls). Only invoked for a COMPILED value/trap
    // trial (the shared grader grades a refused compile / a compile-outcome case without running).
    let mut trial_no = 0usize;
    grade_run(
        test_run,
        compile_status,
        compile_diag,
        diag_wire,
        // check_diag: the C1 check-vs-compile parity leg's `cdz check` wire. This rust-exec path captures no
        // check wire, so parity is OFF here (`None`) — matching grade_run's documented default. (Caller-update
        // for the grade_run signature that added this param; the cdz-rust-run caller was missed.)
        None,
        |trial: &GTrial| {
            let module = module.ok_or_else(|| {
            anyhow::anyhow!(
                "grade: an output/trap case compiled (status 0) but no emitted Rust was supplied"
            )
        })?;
            let (export, args) = match &trial.call {
                Some(c) => (c.export.clone(), c.args.clone()),
                None => {
                    let name = sole_export_name(module, async_mode).ok_or_else(|| {
                        anyhow::anyhow!(
                            "grade: no `(call …)` and no exported fn in the emitted Rust"
                        )
                    })?;
                    (name, Vec::new())
                }
            };
            let driver = build_driver_source(
                module,
                &export,
                &args,
                &host_responses,
                &host_calls,
                async_mode,
            );
            let dir = workdir.join(format!("trial-{trial_no}"));
            trial_no += 1;
            std::fs::create_dir_all(&dir)?;
            Ok(match compile_and_run(&driver, &dir, rlibs, async_mode) {
                RunOutcome::Value(v, observed) => GradeOutcome::Value(v, observed),
                RunOutcome::Trap(t) => GradeOutcome::Trap(t),
                RunOutcome::BadArtifact(e) => GradeOutcome::BadArtifact(e),
            })
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use cdz_corpus_grade::{GCall, GExpect, Grade, TestRun};

    /// A `(test-run …)` with a single trial + no host tape/warns — the common scalar case shape.
    fn one_trial(call: Option<GCall>, expect: GExpect) -> TestRun {
        TestRun {
            description: "test".into(),
            trials: vec![GTrial {
                call,
                expect,
                diag: None,
                exact_code: false,
            }],
            host_responses: vec![],
            host_calls: vec![],
            warns: vec![],
            live_objects: None,
            live_objects_known_leak: false,
            live_objects_per_call: None,
            no_other_errors: false,
            no_diagnostic: vec![],
            diagnostic_quality: false,
            diagnostic_quality_opt_out: false,
        }
    }

    fn workdir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("cdz-rr-grade-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    // INTEGRATION: shells the ambient `rustc`. A nullary scalar export graded against its expected value
    // passes (the sole-export name is recovered from the emitted module, no `(call …)` needed).
    #[test]
    fn a_matching_scalar_output_passes() {
        let tr = one_trial(None, GExpect::Output("(: 42 Int64)".into()));
        let res = grade_to_result(
            &tr,
            Some("pub fn main() -> i64 { 42 }"),
            &RlibDirs::default(),
            false,
            0,
            "",
            None,
            &workdir("pass"),
        )
        .unwrap();
        assert_eq!(res.grade, Grade::Pass);
        assert!(res.ran_a_trial);
    }

    // A wrong emitted value is a Fail (miscompile) — the grade actually compiled + ran the module.
    #[test]
    fn a_wrong_scalar_output_fails() {
        let tr = one_trial(
            Some(GCall {
                export: "main".into(),
                args: vec![],
                second_call: None,
                drop_handle: false,
                method: None,
            }),
            GExpect::Output("(: 42 Int64)".into()),
        );
        let res = grade_to_result(
            &tr,
            Some("pub fn main() -> i64 { 41 }"),
            &RlibDirs::default(),
            false,
            0,
            "",
            None,
            &workdir("fail"),
        )
        .unwrap();
        assert!(matches!(res.grade, Grade::Fail(_)), "got {:?}", res.grade);
    }

    // A RUNTIME ABORT (signal-killed child — the class a STACK OVERFLOW falls in: SIGSEGV/SIGABRT, no exit
    // code, no `panicked at` in stderr) must be graded as a TRAP, NOT swallowed as a pass. compile_and_run
    // keys the outcome on `run.status.success()` (false for a signal death) → `Outcome::Trap`, so an
    // expect-VALUE case sees Trap≠Value → Fail. Pins that the GRADE path never reports success on an aborted
    // run (breaker's cdz-run-rust stack-overflow report — the run-rust *verdict command* prints `trap …` +
    // exit 0 by protocol design, but the GRADE path here is exit-code-independent and cannot be fooled).
    #[test]
    fn a_runtime_abort_is_graded_a_trap_not_swallowed_as_a_pass() {
        let tr = one_trial(
            Some(GCall {
                export: "main".into(),
                args: vec![],
                second_call: None,
                drop_handle: false,
                method: None,
            }),
            GExpect::Output("(: 42 Int64)".into()),
        );
        let res = grade_to_result(
            &tr,
            // Compiles cleanly (status 0), but the RUN aborts by signal before returning — the value is
            // never produced. The signal kill gives `status.success() == false` with no exit code, exactly
            // a stack overflow's shape.
            Some("pub fn main() -> i64 { std::process::abort() }"),
            &RlibDirs::default(),
            false,
            0,
            "",
            None,
            &workdir("abort-trap"),
        )
        .unwrap();
        assert!(
            matches!(res.grade, Grade::Fail(_)),
            "an aborted run must grade a trap-vs-expected-value Fail, never a swallowed Pass — got {:?}",
            res.grade
        );
        assert!(
            res.ran_a_trial,
            "the trial did run (compiled + launched, then aborted)"
        );
    }

    // A REFUSED compile (status != 0, no module) with an `expect-error <CODE>` grades Pass from the
    // diagnostic alone — no run (and no rustc shell). (The former `(declines)` marker was removed: a
    // rejection must now be coded `(error CDZxxxx)`.)
    #[test]
    fn a_coded_error_case_is_graded_from_the_diagnostic() {
        let tr = one_trial(None, GExpect::Error("CDZ0999".into(), vec![], vec![]));
        let res = grade_to_result(
            &tr,
            None,
            &RlibDirs::default(),
            false,
            1,
            "cdz: error [CDZ0999] (node 1): not yet supported",
            None,
            &workdir("coded-error"),
        )
        .unwrap();
        assert_eq!(res.grade, Grade::Pass);
        assert!(!res.ran_a_trial, "a coded-error case runs no trial");
    }

    // The resource-drive DECLINE predicate: a `(then)` two-call / `(drop)` / `(call-method)` case must
    // decline on the standalone-.rs rust path (the wasm-only resource protocols), so the nix coarse-rust
    // gate matches the in-process xtask decline instead of mis-running the first call. Pins the exact
    // divergence v-nix's (C) re-verify caught (21-host-closures compound-result double-call: single-call
    // #tuple(5 105) vs expected #tuple(#tuple(5 105) #tuple(5 105))).
    fn call_with(
        second_call: Option<Vec<String>>,
        drop_handle: bool,
        method: Option<String>,
    ) -> GCall {
        GCall {
            export: "pair".into(),
            args: vec!["100".into()],
            second_call,
            drop_handle,
            method,
        }
    }

    #[test]
    fn a_then_two_call_case_declines_the_resource_drive() {
        let tr = one_trial(
            Some(call_with(Some(vec!["5".into()]), false, None)),
            GExpect::Output("(: (tuple (tuple 5 105) (tuple 5 105)) …)".into()),
        );
        assert!(
            declines_resource_drive(&tr),
            "a (then) borrowed-handle two-call must decline on the rust exec path (wasm-only drive)"
        );
    }

    #[test]
    fn a_drop_handle_case_declines_the_resource_drive() {
        let tr = one_trial(
            Some(call_with(None, true, None)),
            GExpect::Output("(: unit Unit)".into()),
        );
        assert!(
            declines_resource_drive(&tr),
            "an explicit (drop) must decline on the rust exec path"
        );
    }

    #[test]
    fn a_call_method_case_declines_the_resource_drive() {
        let tr = one_trial(
            Some(call_with(None, false, Some("area".into()))),
            GExpect::Output("(: 42 Int64)".into()),
        );
        assert!(
            declines_resource_drive(&tr),
            "a (call-method) value-resource member reach must decline on the rust exec path"
        );
    }

    #[test]
    fn a_plain_single_call_does_not_decline() {
        // The negative control: an ORDINARY `(call …)` (no then/drop/method) runs on the rust path as usual.
        let tr = one_trial(
            Some(call_with(None, false, None)),
            GExpect::Output("(: 42 Int64)".into()),
        );
        assert!(
            !declines_resource_drive(&tr),
            "a plain single-call case must NOT decline — it runs on the rust exec path"
        );
    }

    #[test]
    fn a_no_call_case_does_not_decline() {
        let tr = one_trial(None, GExpect::Output("(: 42 Int64)".into()));
        assert!(
            !declines_resource_drive(&tr),
            "a no-(call) scalar case must not decline"
        );
    }
}
