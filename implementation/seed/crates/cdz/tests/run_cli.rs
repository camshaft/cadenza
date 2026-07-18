//! End-to-end tests for `cdz run` — the wasm-component runner FOLDED into the unified `cdz` binary
//! (from the `cdz-run` lib). The point of the fold is the operator's #1 requirement: ONE binary on the
//! PATH, so `cdz compile` and `cdz run` are the same executable. These drive the built `cdz` binary:
//! compile a tiny program to a component, then run it via `cdz run` and check the printed value + exit
//! code. The standalone `cdz-run` bin is now a thin shim over the same code; this pins the mounted path.

use std::process::Command;

/// Run `cdz <args…>`, returning (exit_ok, stdout, stderr).
fn run(args: &[&str]) -> (bool, String, String) {
    let exe = env!("CARGO_BIN_EXE_cdz");
    let out = Command::new(exe).args(args).output().expect("spawn cdz");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

/// Run `cdz <args…>`, returning the numeric EXIT CODE (for pinning the operational-vs-usage code
/// distinction). `None` if the process was killed by a signal (no code) — never expected here.
fn run_code(args: &[&str]) -> Option<i32> {
    let exe = env!("CARGO_BIN_EXE_cdz");
    Command::new(exe)
        .args(args)
        .output()
        .expect("spawn cdz")
        .status
        .code()
}

/// Run `cdz <args…>` with one extra environment variable set, returning (exit_ok, stdout, stderr). The
/// env var rides on the SUBPROCESS only — so a test exercising `CDZ_RUN_TIMEOUT_SECS` can't race the
/// shared process env of the in-process cdz-run lib tests.
fn run_with_env(key: &str, val: &str, args: &[&str]) -> (bool, String, String) {
    let exe = env!("CARGO_BIN_EXE_cdz");
    let out = Command::new(exe)
        .args(args)
        .env(key, val)
        .output()
        .expect("spawn cdz");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

/// A unique temp dir for one test.
fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("cdz-run-cli-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    dir
}

/// Compile `src` (s-expr) to a component under a fresh dir and return the `.wasm` path.
fn compile_component(tag: &str, name: &str, src: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let dir = temp_dir(tag);
    let srcpath = dir.join(format!("{name}.sexp"));
    std::fs::write(&srcpath, src).unwrap();
    let (ok, _out, err) = run(&[
        "compile",
        srcpath.to_str().unwrap(),
        "-o",
        dir.to_str().unwrap(),
    ]);
    assert!(ok, "compile failed: {err}");
    let wasm = dir.join(format!("{name}.wasm"));
    assert!(wasm.is_file(), "no component produced: {err}");
    (dir, wasm)
}

#[test]
fn cdz_run_unknown_call_lists_the_available_function_exports() {
    // A `--call` naming a nonexistent export (a typo / misremembered name) must LIST what IS callable,
    // not just say "no function `addd`" and leave the user to guess — the rustc/cargo bar: name the
    // alternatives. A two-export component (`add`, `sub`) with a typo'd `--call addd` names both.
    let (dir, wasm) = compile_component(
        "twoexports",
        "add",
        "(module m (def (add (: a Int64) (: b Int64)) (+ a b)) \
         (def (sub (: a Int64) (: b Int64)) (- a b)) (export add) (export sub))",
    );
    let (ok, _out, err) = run(&["run", wasm.to_str().unwrap(), "--call", "addd"]);
    assert!(!ok, "an unknown --call must fail");
    assert!(
        err.contains("no function `addd`")
            && err.contains("add")
            && err.contains("sub")
            && err.contains("function exports are"),
        "the error lists the available function exports (add, sub): {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cdz_run_ambiguous_default_lists_the_function_exports_to_choose_from() {
    // With MULTIPLE function exports and NO `--call`, there is no sole export to default to — the error
    // must name the choices so the user knows which `--call` to pass, not just say "no single export".
    let (dir, wasm) = compile_component(
        "ambig",
        "add",
        "(module m (def (add (: a Int64) (: b Int64)) (+ a b)) \
         (def (sub (: a Int64) (: b Int64)) (- a b)) (export add) (export sub))",
    );
    let (ok, _out, err) = run(&["run", wasm.to_str().unwrap()]);
    assert!(!ok, "an ambiguous default (no --call, >1 export) must fail");
    assert!(
        err.contains("no single function export") && err.contains("add") && err.contains("sub"),
        "the error lists the exports to choose a --call from: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cdz_run_invokes_an_export_and_prints_the_value() {
    // The headline of the one-binary fold: `cdz compile` then `cdz run` are the SAME binary.
    let (dir, wasm) = compile_component(
        "add",
        "add",
        "(module m (def (add (: a Int64) (: b Int64)) (+ a b)) (export add))",
    );
    let (ok, out, err) = run(&[
        "run",
        wasm.to_str().unwrap(),
        "--call",
        "add",
        "--arg",
        "2",
        "--arg",
        "40",
    ]);
    assert!(ok, "cdz run failed: {err}");
    assert_eq!(out.trim(), "42", "printed value: {out}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cdz_run_defaults_to_the_sole_function_export() {
    // With one function export, `--call` is optional — `cdz run` picks it.
    let (dir, wasm) = compile_component(
        "double",
        "double",
        "(module m (def (double (: n Int64)) (* n 2)) (export double))",
    );
    let (ok, out, err) = run(&["run", wasm.to_str().unwrap(), "--arg", "21"]);
    assert!(ok, "cdz run failed: {err}");
    assert_eq!(out.trim(), "42", "printed value: {out}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cdz_run_reads_a_component_from_stdin() {
    // `-` reads the component from stdin, so `cdz compile … -o - | cdz run -` composes in a pipe.
    let (dir, wasm) = compile_component(
        "id",
        "id",
        "(module m (def (id (: n Int64)) n) (export id))",
    );
    let bytes = std::fs::read(&wasm).unwrap();
    let exe = env!("CARGO_BIN_EXE_cdz");
    let mut child = Command::new(exe)
        .args(["run", "-", "--call", "id", "--arg", "7"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn cdz");
    use std::io::Write;
    child.stdin.take().unwrap().write_all(&bytes).unwrap();
    let out = child.wait_with_output().expect("wait");
    assert!(
        out.status.success(),
        "stdin run failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "7");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cdz_run_on_a_missing_file_errors_with_the_cdz_prog_name() {
    // A read error names the tool the user actually typed (`cdz`), not `cdz-run` — the diagnostic prog
    // name threads through the mounted entry point. Exit code is non-zero.
    let (ok, _out, err) = run(&["run", "/no/such/component.wasm", "--call", "x"]);
    assert!(!ok, "a missing component should fail");
    assert!(
        err.contains("cdz:") && err.to_lowercase().contains("read"),
        "error names `cdz` and mentions the read failure: {err}"
    );
}

#[test]
fn cdz_run_exit_codes_distinguish_operational_from_usage_errors() {
    // The exit-code contract: an OPERATIONAL failure (a missing/unreadable component) is `1` — the same
    // code a run-time trap returns and the same code every other `cdz` subcommand uses for a real failure.
    // A CLI-USAGE error (an unknown flag) is `2` (clap's convention). This lets a script tell "you invoked
    // it wrong" (2) from "it ran and failed" (1). Regression: an operational error here previously returned
    // `2`, colliding with the usage signal (and inconsistent with a trap's `1`).
    assert_eq!(
        run_code(&["run", "/no/such/component.wasm"]),
        Some(1),
        "a missing component is an OPERATIONAL error → exit 1 (not the usage code 2)"
    );
    assert_eq!(
        run_code(&["run", "--definitely-not-a-flag"]),
        Some(2),
        "an unknown flag is a USAGE error → clap's exit 2"
    );
}

#[test]
fn cdz_run_on_a_trap_prefixes_the_message_with_the_prog_name() {
    // A RUNTIME trap (here: integer divide by zero on a runtime divisor) prints `cdz: trap: …` on
    // stderr with a FAILURE exit — the `{prog}:` prefix on the trap line is consistent with every other
    // `cdz` stderr message. (Both trap sites in cdz_run::cli emit `{prog}: trap:`, not a bare `trap:`.)
    let (dir, wasm) = compile_component(
        "trap",
        "boom",
        "(module m (def (boom (: n Int64) (: d Int64)) (/ n d)) (export boom))",
    );
    let (ok, _out, err) = run(&[
        "run",
        wasm.to_str().unwrap(),
        "--call",
        "boom",
        "--arg",
        "5",
        "--arg",
        "0",
    ]);
    assert!(!ok, "a divide-by-zero trap should fail");
    assert!(
        err.contains("cdz: trap:"),
        "trap line carries the `cdz:` prog prefix: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ── `cdz run <project>` — the cargo-run analogue: build the manifest entry, then run it ─────────────

/// A scalar (runtime-free) project: `Project.cdz` naming `app.cdz` with an `add` export. Scalar so the
/// run needs NO value-heap runtime store — the test is hermetic. Returns the project dir.
fn scalar_project(tag: &str) -> std::path::PathBuf {
    let dir = temp_dir(tag);
    std::fs::write(
        dir.join("Project.cdz"),
        "def name = \"demo\"\ndef entry = \"app.cdz\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("app.cdz"),
        "def add(a: Int64, b: Int64) -> Int64 = a + b\nexport { add }\n",
    )
    .unwrap();
    dir
}

#[test]
fn cdz_run_a_project_directory_builds_then_runs() {
    // `cdz run <dir>` builds the manifest's entry and runs it — the `cargo run` analogue — printing the
    // export's value with a success exit.
    let dir = scalar_project("proj-dir");
    let (ok, out, err) = run(&[
        "run",
        dir.to_str().unwrap(),
        "--call",
        "add",
        "--arg",
        "2",
        "--arg",
        "40",
    ]);
    assert!(ok, "cdz run <project dir> failed: {err}");
    assert_eq!(out.trim(), "42", "printed the built export's value: {out}");
    // The temp build artifact is cleaned up (no `.cdz-run-*.wasm` left behind).
    let leftovers: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with(".cdz-run-"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "the temp build component is cleaned up after the run"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cdz_run_a_manifest_path_builds_then_runs() {
    // `cdz run path/to/Project.cdz` also works (the manifest-file form of the project target).
    let dir = scalar_project("proj-manifest");
    let manifest = dir.join("Project.cdz");
    let (ok, out, err) = run(&[
        "run",
        manifest.to_str().unwrap(),
        "--call",
        "add",
        "--arg",
        "20",
        "--arg",
        "22",
    ]);
    assert!(ok, "cdz run <Project.cdz> failed: {err}");
    assert_eq!(out.trim(), "42", "printed value: {out}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cdz_run_a_directory_without_a_manifest_errors() {
    // `cdz run <dir>` where the directory has NO `Project.cdz` is a clear project-resolution error (not a
    // confusing component-read failure).
    let dir = temp_dir("proj-nomani");
    let (ok, _out, err) = run(&["run", dir.to_str().unwrap()]);
    assert!(!ok, "a dir with no manifest must fail");
    assert!(
        err.contains("cdz:") && err.contains("Project.cdz"),
        "error names the missing manifest: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cdz_run_a_project_stdout_is_only_the_value_and_stderr_is_quiet() {
    // `cdz run <project>` stdout is JUST the value (composes in a pipe), AND stderr is QUIET on success —
    // the in-memory build emits no `cdz: wrote <temp>.wasm` notice, so a project run doesn't leak its
    // internal temp-artifact path (the `cargo run` convention: don't announce where the binary went).
    let dir = scalar_project("proj-stdout");
    let exe = env!("CARGO_BIN_EXE_cdz");
    let out = Command::new(exe)
        .args([
            "run",
            dir.to_str().unwrap(),
            "--call",
            "add",
            "--arg",
            "1",
            "--arg",
            "2",
        ])
        .output()
        .expect("spawn cdz");
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "3",
        "stdout is only the value"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.trim().is_empty() && !stderr.contains("wrote"),
        "stderr is quiet on a successful project run (no build/temp-path notice): [{stderr}]"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Run `cdz <args…>` FROM `cwd` (for the no-arg upward-search cases), returning (exit_ok, stdout, stderr).
fn run_from(cwd: &std::path::Path, args: &[&str]) -> (bool, String, String) {
    let exe = env!("CARGO_BIN_EXE_cdz");
    let out = Command::new(exe)
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("spawn cdz");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

#[test]
fn cdz_run_no_arg_builds_and_runs_the_current_project() {
    // A bare `cdz run` (no component) in a project directory builds+runs the manifest entry — the `cargo
    // run` analogue, completing the no-arg parity `build`/`test`/`check` already have.
    let dir = scalar_project("noarg");
    let (ok, out, err) = run_from(&dir, &["run", "--call", "add", "--arg", "2", "--arg", "40"]);
    assert!(ok, "bare `cdz run` in a project dir failed: {err}");
    assert_eq!(out.trim(), "42", "printed the built export's value: {out}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cdz_run_no_arg_searches_upward_from_a_subdirectory() {
    // Like `cargo run`, a bare `cdz run` searches UP for the nearest `Project.cdz`, so it works from any
    // subdirectory of the project.
    let dir = scalar_project("noarg-up");
    let deep = dir.join("sub").join("deep");
    std::fs::create_dir_all(&deep).unwrap();
    let (ok, out, err) = run_from(
        &deep,
        &["run", "--call", "add", "--arg", "20", "--arg", "22"],
    );
    assert!(ok, "upward-search `cdz run` failed: {err}");
    assert_eq!(out.trim(), "42", "ran the project found upward: {out}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cdz_run_no_arg_with_no_project_errors() {
    // A bare `cdz run` where no `Project.cdz` exists in the cwd or any ancestor is a clear error (not a
    // confusing missing-component read failure).
    let dir = temp_dir("noarg-none");
    let (ok, _out, err) = run_from(&dir, &["run"]);
    assert!(!ok, "bare `cdz run` with no project must fail");
    assert!(
        err.contains("Project.cdz"),
        "error names the missing manifest: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cdz_run_release_builds_the_project_and_runs() {
    // `cdz run --release` builds the entry at the O2 tier before running (the `cargo run --release`
    // analogue). The observable is the same value + success; the tier is exercised by the build path.
    let dir = scalar_project("release");
    let (ok, out, err) = run(&[
        "run",
        dir.to_str().unwrap(),
        "--release",
        "--call",
        "add",
        "--arg",
        "2",
        "--arg",
        "40",
    ]);
    assert!(ok, "cdz run --release failed: {err}");
    assert_eq!(out.trim(), "42", "release build runs correctly: {out}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cdz_run_opt_level_builds_the_project_and_runs() {
    // `cdz run --opt-level O0` builds+runs at the named tier — the explicit-level form.
    let dir = scalar_project("optlevel");
    let (ok, out, err) = run(&[
        "run",
        dir.to_str().unwrap(),
        "--opt-level",
        "O0",
        "--call",
        "add",
        "--arg",
        "20",
        "--arg",
        "22",
    ]);
    assert!(ok, "cdz run --opt-level O0 failed: {err}");
    assert_eq!(out.trim(), "42", "O0 build runs correctly: {out}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cdz_run_a_bad_opt_level_errors_naming_the_choices() {
    // A malformed `--opt-level` on a project run is a clear error naming the valid set (shared with
    // `cdz build`'s precedence), not a silent fallback.
    let dir = scalar_project("badopt");
    let (ok, _out, err) = run(&[
        "run",
        dir.to_str().unwrap(),
        "--opt-level",
        "O9",
        "--call",
        "add",
    ]);
    assert!(!ok, "a bad --opt-level must fail");
    assert!(
        err.contains("O0, O1, O2, O3"),
        "error names the valid levels: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cdz_run_arg_type_error_names_the_cadenza_type_not_the_component_model_spelling() {
    // A `--arg` that doesn't parse as the export's declared parameter type must name the CADENZA type the
    // user WROTE (`Int64`), not wasmtime's component-model debug spelling (`S64`). The user annotated
    // `(: n Int64)`; an error that says "as S64" leaks an internal ABI name they never typed and can't map
    // back to their source. Regression: the coercion error printed `{t:?}` (the component spelling).
    let (dir, wasm) = compile_component(
        "argtype",
        "inc",
        "(module m (def (inc (: n Int64)) (+ n 1)) (export inc))",
    );
    let (ok, _out, err) = run(&[
        "run",
        wasm.to_str().unwrap(),
        "--call",
        "inc",
        "--arg",
        "hello",
    ]);
    assert!(!ok, "a non-numeric arg to an Int64 param must fail");
    assert!(
        err.contains("as Int64"),
        "the error names the Cadenza type the user wrote: {err}"
    );
    assert!(
        !err.contains("S64"),
        "must NOT leak the component-model spelling `S64`: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cdz_run_host_response_errors_surface_the_actionable_cause_not_an_opaque_trap() {
    // A program that performs a host effect but gets a bad/absent `--host-response` must surface the
    // ACTIONABLE cause, NOT a bare wasmtime `error while executing at wasm backtrace:` wrapper. Two cases,
    // both pinning behaviors that exist today (a prior breaker fix + the arg-coercion type-name reuse) so a
    // future refactor of the host-func error path can't silently regress them back to an opaque trap:
    //   - NO response supplied  → names the op + the call number + how many responses were given;
    //   - a non-coercible value → names the Cadenza type (`Int64`), via the shared coercion path.
    // `main` performs `Ask.ask : () -> Int64` once, so the runner needs one Int64 host-response.
    let (dir, wasm) = compile_component(
        "hostresp",
        "main",
        "(module m (effect Ask (op ask (-> Int64))) (def (main) (host (Ask) (Ask.ask))) (export main))",
    );
    let w = wasm.to_str().unwrap();

    // Missing response: the run fails (non-zero) and the cause names the op, not just the wasm wrapper.
    let (ok, _o, err) = run(&["run", w, "--call", "main"]);
    assert!(!ok, "a performed host op with no response must fail");
    assert!(
        err.contains("host call `ask.ask` has no recorded response"),
        "the actionable cause (which op, no response) is surfaced, not buried under the wasm wrapper: {err}"
    );

    // Non-coercible response value: the cause names the Cadenza type (`Int64`), not the component spelling.
    let (ok2, _o2, err2) = run(&[
        "run",
        w,
        "--call",
        "main",
        "--host-response",
        "ask.ask=hello",
    ]);
    assert!(!ok2, "a non-coercible host-response value must fail");
    assert!(
        err2.contains("cannot parse `hello` as Int64"),
        "the coercion failure names the Cadenza type: {err2}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cdz_run_arg_coercion_respects_narrow_width_bounds_no_silent_wrap() {
    // A narrow-width param must ACCEPT its in-range values verbatim and REJECT an out-of-range one — never
    // silently WRAP it (a `256` taken as `0` for a `UInt8` would be a silent miscompile of the caller's
    // intent). Pins the boundary behavior EVEN THOUGH it currently holds, so a future `coerce_one` change
    // (e.g. a wrapping `as` cast instead of a checked parse) can't quietly flip it. `UInt8` identity export:
    // 255 (max) passes through as 255; 256 (max+1) and -1 (below 0) are rejected, not wrapped to 0/255.
    let (dir, wasm) = compile_component(
        "u8bounds",
        "idu8",
        "(module m (def (idu8 (: n UInt8)) n) (export idu8))",
    );
    let w = wasm.to_str().unwrap();

    let (ok, out, err) = run(&["run", w, "--call", "idu8", "--arg", "255"]);
    assert!(ok, "the in-range max 255 must run: {err}");
    assert_eq!(
        out.trim(),
        "255",
        "255 passes through unchanged (no wrap): {out}"
    );

    let (ok, _o, err) = run(&["run", w, "--call", "idu8", "--arg", "256"]);
    assert!(
        !ok && err.contains("as UInt8"),
        "256 is out of UInt8 range → rejected (NOT silently wrapped to 0): {err}"
    );

    let (ok, _o, err) = run(&["run", w, "--call", "idu8", "--arg", "-1"]);
    assert!(
        !ok && err.contains("as UInt8"),
        "-1 is below UInt8 range → rejected (NOT wrapped to 255): {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cdz_run_with_timeout_secs_zero_is_unbounded_not_an_instant_trap() {
    // `CDZ_RUN_TIMEOUT_SECS=0` is the documented UNBOUNDED escape hatch (debugger / a legitimately long
    // run). Regression (breaker): with the shared engine's `epoch_interruption(true)`, a store's DEFAULT
    // epoch deadline is 0, so the old `secs==0 → skip set_epoch_deadline` path left the deadline at 0 and
    // EVERY program trapped instantly with `interrupt` — even a trivial `main`. A trivial component must
    // RUN (print its value, exit 0) under `SECS=0`, not trap. Subprocess env → no race with other tests.
    let (dir, wasm) = compile_component(
        "unbounded",
        "seven",
        "(module m (def (seven) 7) (export seven))",
    );
    let (ok, out, err) = run_with_env(
        "CDZ_RUN_TIMEOUT_SECS",
        "0",
        &["run", wasm.to_str().unwrap(), "--call", "seven"],
    );
    assert!(ok, "SECS=0 must be UNBOUNDED, not an instant trap: {err}");
    assert_eq!(
        out.trim(),
        "7",
        "the program runs and prints its value: {out}"
    );
    assert!(
        !err.contains("interrupt"),
        "must NOT trap with an epoch `interrupt` under the unbounded setting: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cdz_run_with_an_absurdly_large_timeout_does_not_overflow_into_an_instant_trap() {
    // A giant `CDZ_RUN_TIMEOUT_SECS` (a user reaching for "effectively unbounded") must behave as a huge
    // timeout, not overflow. Regression: `new_store` computed `secs * 1000` — for a large `secs` that
    // overflows the u64 millis product (a PANIC in debug; in release it WRAPS to a tiny value → a huge
    // timeout inverts into a near-instant trap, the same class of bug as SECS=0). `saturating_mul` clamps
    // it to a giant tick count instead. Pick a `secs` whose `*1000` would wrap to a SMALL number
    // (`u64::MAX/1000 + 1`), which pre-fix produced a tiny deadline: a trivial program must still RUN.
    let (dir, wasm) = compile_component(
        "bigtimeout",
        "seven",
        "(module m (def (seven) 7) (export seven))",
    );
    let secs = (u64::MAX / 1000 + 1).to_string();
    let (ok, out, err) = run_with_env(
        "CDZ_RUN_TIMEOUT_SECS",
        &secs,
        &["run", wasm.to_str().unwrap(), "--call", "seven"],
    );
    assert!(
        ok,
        "a huge timeout must not overflow into a trap/panic: {err}"
    );
    assert_eq!(
        out.trim(),
        "7",
        "the program runs under a huge timeout: {out}"
    );
    assert!(
        !err.contains("interrupt") && !err.contains("overflow"),
        "no epoch interrupt or overflow panic under a giant timeout: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cdz_run_on_a_source_file_gives_an_actionable_diagnostic_not_a_wasm_parse_error() {
    // `cdz run foo.sexp` (a SOURCE file passed by mistake) must NOT surface the cryptic
    // "invalid component: failed to parse WebAssembly module" — it should point at the real paths
    // (compile-then-run, or run the project). Exit non-zero (a usage mistake).
    let dir = temp_dir("run-source");
    let src = dir.join("prog.sexp");
    std::fs::write(&src, "(do (def (main) 1) (export main))").unwrap();
    let (ok, _out, err) = run(&["run", src.to_str().unwrap()]);
    assert!(!ok, "running a source file is a usage error (non-zero)");
    assert!(
        err.contains("is a SOURCE file") && err.contains("cdz compile"),
        "the diagnostic names the source-file mistake + the compile-first fix: {err}"
    );
    assert!(
        !err.contains("failed to parse WebAssembly"),
        "must NOT leak the cryptic wasm-parse error: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
