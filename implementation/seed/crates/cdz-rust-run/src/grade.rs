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

use anyhow::Result;
use cdz_corpus_grade::{
    GTrial, GradeResult, Outcome as GradeOutcome, decode_test_run, exec_exit, grade_run,
};

use crate::driver::build_driver_source;
use crate::run::{Outcome as RunOutcome, RlibDirs, compile_and_run};
use crate::sig::sole_export_name;

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
) -> Result<ExitCode> {
    let test_run = decode_test_run(test_run_ast)?;
    let result = grade_to_result(
        &test_run,
        module,
        rlibs,
        async_mode,
        compile_status,
        compile_diag,
        diag_wire,
        workdir,
    )?;
    // The exit reproduces `xtask gate --check --target rust` when a baseline is supplied (fail ONLY on a
    // pass→not-pass regression; a baseline-todo/absent case that is now todo/fail — e.g. an imposed-WIT-world
    // reducer the rust backend declines → todo, that this pipeline compiles-without-the-world → fail — is NOT
    // a --check failure), else fails on any outright Fail (the miscompile check).
    Ok(exec_exit(&result, &test_run.description, baseline))
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
            }],
            host_responses: vec![],
            host_calls: vec![],
            warns: vec![],
            live_objects: None,
            live_objects_known_leak: false,
            live_objects_per_call: None,
            no_other_errors: false,
            no_diagnostic: vec![],
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

    // A REFUSED compile (status != 0, no module) with an `expect-declines` grades Pass from the diagnostic
    // alone — no run (and no rustc shell).
    #[test]
    fn a_declined_case_is_graded_from_the_diagnostic() {
        let tr = one_trial(None, GExpect::Declines(None, vec![], vec![]));
        let res = grade_to_result(
            &tr,
            None,
            &RlibDirs::default(),
            false,
            1,
            "cdz: error [CDZ0999] (node 1): not yet supported",
            None,
            &workdir("declines"),
        )
        .unwrap();
        assert_eq!(res.grade, Grade::Pass);
        assert!(!res.ran_a_trial, "a declines case runs no trial");
    }
}
