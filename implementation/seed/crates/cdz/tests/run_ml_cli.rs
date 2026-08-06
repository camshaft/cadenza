//! End-to-end tests for `cdz run-ml [FILE]` — the CADENZA-IN-CADENZA (ML) compiler conformance seam the
//! `cargo xtask gate` `cadenza-ml` target invokes.
//!
//! These pin the EXIT-CODE CONTRACT the subcommand's doc promises (and that the gate wiring relies on):
//! any RUN OUTCOME — a value, a `declined` (not-yet-supported), or an `error <msg>` — exits 0 (a verdict,
//! not a shell failure), so the gate reads stdout for the per-case outcome; the ONE non-zero exit is a
//! HARNESS failure that produced no verdict (a file/stdin READ error). (`run-ml` is currently a
//! declines-all stub; the ML front-end lands behind it — so the value/error verdicts aren't exercised
//! here yet, but the decline + read-error exits, which the gate contract hinges on, are.)

use std::io::Write;
use std::process::{Command, Stdio};

/// Run `cdz run-ml <args…>`, returning (exit_ok, stdout, stderr).
fn run(args: &[&str]) -> (bool, String, String) {
    let exe = env!("CARGO_BIN_EXE_cdz");
    let mut a = vec!["run-ml"];
    a.extend_from_slice(args);
    let out = Command::new(exe).args(&a).output().expect("spawn cdz");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Whether the value-heap runtime STORE is present beside the `cdz` binary. CI's bare `test` job runs
/// `cargo test --workspace` with NO `cargo xtask build`, so there is no store — and `cdz run-ml`
/// resolves the runtime (to run the module to its value) BEFORE it can report the `value <n>` verdict, so
/// storeless it `declined`s (or errors) instead. Skip the VALUE assertion when absent; the store-having
/// `gate` + `@test suites` jobs exercise it fully. Mirrors `run_emitted_cli`/`test_manifest_cli`'s guard.
fn store_present() -> bool {
    // HASH-AWARE: the store must hold the CURRENT runtime — `<store>/<REQUIRED_RUNTIME_HASH>.wasm` is a
    // file — NOT just be a non-empty dir. A NON-EMPTY-only check passed on a STALE rust-cached store (an
    // OLDER runtime hash after a hash bump) → the runtime-driving value-check ran → the resolver missed the
    // current hash → "no runtime of content address … refusing to run" red. Checking the exact `<hash>.wasm`
    // makes a stale/empty store read as absent so the check SKIPS (as in the storeless CI job, where xtask
    // sets `CADENZA_STORE=<empty temp dir>`).
    let required = rcdzc::backend::wasm::runtime_abi::REQUIRED_RUNTIME_HASH;
    let store_dir = std::env::var_os("CADENZA_STORE")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::path::PathBuf::from(env!("CARGO_BIN_EXE_cdz"))
                .parent()
                .and_then(|d| d.parent())
                .map(|t| t.join("cadenza-store"))
        });
    store_dir
        .map(|d| d.join(format!("{required}.wasm")).is_file())
        .unwrap_or(false)
}

/// Write `src` to a unique temp `.sexp` file; return (dir, path).
fn temp_src(tag: &str, src: &str) -> (std::path::PathBuf, String) {
    let dir = std::env::temp_dir().join(format!("cdz-runml-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("m.sexp");
    std::fs::write(&path, src).unwrap();
    (dir, path.to_str().unwrap().to_string())
}

#[test]
fn run_ml_exits_zero_for_a_run_outcome() {
    // A well-formed program yields a VERDICT to stdout and exits 0 — a decline is a verdict, not a shell
    // failure (the gate reads stdout, not the exit code, for the per-case outcome). Currently the stub
    // declines every program, so the verdict is `declined`; the invariant under test is the ZERO exit.
    let (dir, path) = temp_src("verdict", "(do (def (main) 0) (export main))");
    let (ok, out, err) = run(&[&path]);
    assert!(
        ok,
        "a run outcome exits 0 (a verdict is not a shell failure): {err}{out}"
    );
    assert!(
        out.contains("declined") || out.starts_with("value ") || out.starts_with("error "),
        "stdout carries one verdict line: {out:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn run_ml_from_stdin_also_exits_zero_with_a_verdict() {
    // No FILE arg → read the program from stdin (the same path the gate can drive); still a verdict + exit 0.
    let exe = env!("CARGO_BIN_EXE_cdz");
    let mut child = Command::new(exe)
        .args(["run-ml"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn cdz run-ml");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"(do (def (main) 0) (export main))\n")
        .unwrap();
    let out = child.wait_with_output().expect("wait");
    assert!(out.status.success(), "stdin run-ml exits 0: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("declined") || stdout.starts_with("value ") || stdout.starts_with("error "),
        "stdin run-ml prints a verdict: {stdout:?}"
    );
}

#[test]
fn run_ml_an_explicit_dash_reads_stdin_not_a_file_named_dash() {
    // REGRESSION: `run-ml -` (the stdin MARKER `cdz fmt -`/`convert -`/`compile -`/`run -` use) used to fall
    // through to `read_to_string("-")`, leaking the raw `cannot read -: No such file or directory (os error
    // 2)` errno — the exact errno-leak the query commands' `-` guard prevents. A script piping with a `-`
    // (the shell convention everywhere else) hit it. Now `-` reads stdin like a missing arg: a verdict + exit
    // 0, and specifically NOT the "cannot read -" errno leak.
    let exe = env!("CARGO_BIN_EXE_cdz");
    let mut child = Command::new(exe)
        .args(["run-ml", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn cdz run-ml -");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"(do (def (main) 0) (export main))\n")
        .unwrap();
    let out = child.wait_with_output().expect("wait");
    assert!(
        out.status.success(),
        "run-ml - exits 0 (a verdict): {out:?}"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("declined") || stdout.starts_with("value ") || stdout.starts_with("error "),
        "run-ml - prints a verdict from stdin: {stdout:?}"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("cannot read -"),
        "must NOT leak the `cannot read -` errno — `-` is the stdin marker: {stderr}"
    );
}

#[test]
fn run_ml_leaves_no_driver_file_behind() {
    // `cdz run-ml` writes a pid-stamped `zz-run-ml-driver-<pid>.cdz` INTO the compiler-ml src tree (its
    // `import "sread-eval"` resolves entry-dir-relative, so it can't live in a temp dir). An RAII guard
    // must remove it on every in-process exit path — else a screenful of stray drivers accumulates in the
    // shared worktree's `git status` (the fleet-pollution issue this pins).
    //
    // Assert on OUR child's OWN pid-stamped driver, not a directory COUNT: the sibling run-ml tests run
    // concurrently against the same shared src dir, so their in-flight drivers make a count-delta racy
    // (observed before=0/after=2). Spawn the child ourselves, capture its pid, let it fully exit, then
    // assert exactly `zz-run-ml-driver-<child-pid>.cdz` is absent — a leak of THIS run is unambiguous and
    // independent of any concurrent invocation.
    let mut root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let src_dir = loop {
        let cand = root.join("implementation/compiler-ml/src");
        if cand.is_dir() {
            break Some(cand);
        }
        if !root.pop() {
            break None;
        }
    };
    let Some(src_dir) = src_dir else {
        // The compiler-ml src tree isn't present in this checkout (shouldn't happen in-repo) — nothing to
        // pin; don't fail the harness over a layout we can't see.
        return;
    };
    let exe = env!("CARGO_BIN_EXE_cdz");
    let (dir, path) = temp_src("cleanup", "(do (def (main) 0) (export main))");
    // Spawn so we can read the child's pid — the driver is named after the CHILD `cdz`'s process id.
    let child = Command::new(exe)
        .args(["run-ml", &path])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn cdz run-ml");
    let child_pid = child.id();
    let out = child.wait_with_output().expect("wait run-ml");
    let _ = std::fs::remove_dir_all(&dir);
    // The child has fully exited, so its RAII guard has run — its own driver must be gone.
    let leaked = src_dir.join(format!("zz-run-ml-driver-{child_pid}.cdz"));
    assert!(
        !leaked.exists(),
        "run-ml leaked its driver {} (child exited status {:?}) — the RAII cleanup guard regressed",
        leaked.display(),
        out.status
    );
}

#[test]
fn run_ml_runs_a_multi_def_nullary_main_module() {
    // The `looks_in_ml_subset` shape gate must ADMIT a multi-definition module with a NULLARY `main` (a
    // helper def + a main that calls it) — the ML reader resolves calls to sibling defs, so this runs
    // end-to-end. Before the widening, the gate fast-declined any module with more than one `(def`, so a
    // program the ML compiler ACTUALLY RUNS was reported `declined` — an under-count of real conformance.
    // Pin the value verdict: `(do (def (g) 7) (def (main) (+ (g) 5)) (export main))` → 12.
    let (dir, path) = temp_src(
        "multidef",
        "(do (def (g) 7) (def (main) (+ (g) 5)) (export main))",
    );
    let (ok, out, err) = run(&[&path]);
    assert!(ok, "a multi-def module exits 0 with a verdict: {err}{out}");
    // The VALUE verdict needs the runtime store (run-ml resolves + runs it). CI's storeless
    // `cargo test --workspace` has no store, so skip the value-check there; the store-having gate +
    // @test suites jobs pin `value 12`. Storeless, the exit-0 verdict contract above still holds.
    if store_present() {
        assert!(
            out.trim() == "value 12",
            "a multi-def nullary-main module runs (not fast-declined): got {out:?}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn run_ml_declines_an_unbound_type_annotation_via_the_real_pipeline() {
    // A type annotation with an UNBOUND type name `(: x foo)` must DECLINE — matching the reference's
    // CDZ0101. This USED to be fast-declined by an over-broad `(: ` substring gate in `looks_in_ml_subset`
    // (a coverage-not-yet stopgap, back when the ML front-end read but DISCARDED annotation types). That
    // gate is now DROPPED (annotation enforcement landed), so this reaches the real pipeline — and the
    // reader still declines an unrecognized type name (the bodyId -1 sentinel). Same `declined` verdict,
    // now for the RIGHT reason (a real rejection, not a shape-gate skip).
    let (dir, path) = temp_src(
        "unbound-type",
        "(do (def (f (: x foo)) x) (def (main) (f 7)) (export main))",
    );
    let (ok, out, err) = run(&[&path]);
    assert!(ok, "an unbound-type-annotated module exits 0: {err}{out}");
    assert!(
        out.trim() == "declined",
        "an unbound type name `(: x foo)` declines (matches reference CDZ0101): got {out:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn run_ml_runs_a_narrow_annotation_in_the_main_wrapper() {
    // REGRESSION GUARD (the `looks_in_ml_subset` over-broad `(: ` fix): a valid narrow-int annotation inside
    // the `(do (def (main) …) (export main))` wrapper must now RUN, not fast-decline. The `(: ` exclusion
    // formerly fired ONLY in the nullary-main-module branch, so a BARE `(: 100 Int8)` already ran while the
    // SAME annotation in the wrapper wrongly fast-declined (a spurious wrapper-vs-bare asymmetry — NOT a
    // scale-emergent emit bug, as first misdiagnosed). With enforcement landed, both forms run and are
    // checked: `(: 100 Int8)` → 100. (An overflow like `(: 200 Int8)` still declines via the real pipeline,
    // matching run-emitted + the reference — covered by the compiler-ml @tests + the W4 differential.)
    let (dir, path) = temp_src(
        "narrow-ann-wrapper",
        "(do (def (main) (: 100 Int8)) (export main))",
    );
    let (ok, out, err) = run(&[&path]);
    assert!(ok, "a narrow-annotation module exits 0: {err}{out}");
    if store_present() {
        assert!(
            out.trim() == "value 100",
            "a narrow annotation `(: 100 Int8)` in the main wrapper RUNS to 100 (no longer fast-declined): got {out:?}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn run_ml_on_an_unreadable_file_exits_non_zero() {
    // The ONE non-zero exit: a HARNESS failure that produced no verdict (a file READ error). A script /
    // the gate distinguishes "the ML compiler judged this program" (exit 0 + a verdict) from "the input
    // couldn't be read at all" (non-zero + no verdict).
    let dir = std::env::temp_dir().join(format!("cdz-runml-missing-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let missing = dir.join("nope.sexp");
    let (ok, out, err) = run(&[missing.to_str().unwrap()]);
    assert!(
        !ok,
        "an unreadable input is a non-zero (harness) exit, not a verdict: {out}"
    );
    assert!(
        err.contains("cannot read"),
        "the read failure is named on stderr: {err}"
    );
    assert!(
        !out.contains("declined") && !out.starts_with("value "),
        "no verdict line is printed for a read error: {out:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
