//! End-to-end tests for `cdz run-emitted [FILE]` — the W4 emit-equals-interpret runner: compile a program
//! through compiler-ml's wasm-EMIT backend (`emit-src-bytes`) to a core module, run it via wasmtime, print
//! the i64. Contract mirrors `cdz run-ml` (value/declined, exit 0) so the W4 differential compares
//! run-emitted's value against run-ml's value case-by-case.

use std::io::Write;
use std::process::{Command, Stdio};

/// Run `cdz run-emitted` with `program` on stdin; return (exit_ok, stdout_trimmed).
fn run_emitted(program: &str) -> (bool, String) {
    let exe = env!("CARGO_BIN_EXE_cdz");
    let mut child = Command::new(exe)
        .arg("run-emitted")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn cdz run-emitted");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(program.as_bytes())
        .unwrap();
    let out = child.wait_with_output().expect("wait");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
    )
}

/// Same for `cdz run-ml` — the interpreter ORACLE the W4 differential compares against.
fn run_ml(program: &str) -> String {
    let exe = env!("CARGO_BIN_EXE_cdz");
    let mut child = Command::new(exe)
        .arg("run-ml")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn cdz run-ml");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(program.as_bytes())
        .unwrap();
    let out = child.wait_with_output().expect("wait");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

// NOTE: the two tests below drive the FULL compiler-ml front-end (sread→lower→emit) per call — tens of
// seconds each in the debug `cargo test` profile (a whole-pipeline compile, like run-ml's tests). They are
// `#[ignore]`d so `cargo test --workspace` (CI) stays fast; run them explicitly with `--ignored`. The
// verdict CONTRACT is still guarded cheaply by the two active tests (out-of-subset decline + read-error),
// and the emit≡interpret invariant is the W4 gate step (v-compiler-ml wires run-emitted-vs-run-ml into
// `xtask check`) — so the value/agreement path is covered without paying the compile cost in every suite.
#[test]
#[ignore = "drives the full compiler-ml pipeline per call (tens of seconds); run with --ignored / covered by the W4 gate"]
fn run_emitted_runs_the_emitted_module_to_its_i64() {
    // The emit backend compiles a const/arith `main` to a core wasm module; run-emitted runs it and prints
    // the i64. Exit 0, a `value <n>` verdict.
    let (ok, out) = run_emitted("(do (def (main) (+ (* 10 4) 2)) (export main))");
    assert!(ok, "a run outcome exits 0: {out}");
    assert_eq!(out, "value 42", "the emitted module runs to 42: {out:?}");
}

#[test]
#[ignore = "drives the full compiler-ml pipeline 8× (tens of seconds each); run with --ignored / this IS the W4 gate invariant"]
fn run_emitted_agrees_with_run_ml_the_interpret_oracle() {
    // The W4 invariant: emit ≡ interpret. For each in-subset case, run-emitted's verdict must EQUAL
    // run-ml's (the eval-db oracle). Includes the trap==declined case (div0): the emitted module traps,
    // run-emitted maps it to `declined`, matching run-ml's `declined`.
    for prog in [
        "(do (def (main) 42) (export main))",
        "(do (def (main) (if (< 3 5) 7 9)) (export main))",
        "(do (def (main) (let ((x 21)) (+ x x))) (export main))",
        "(do (def (main) (/ 10 0)) (export main))", // div0: both declined (trap==decline)
    ] {
        let emitted = run_emitted(prog).1;
        let ml = run_ml(prog);
        assert_eq!(
            emitted, ml,
            "emit≡interpret must agree for `{prog}`: emitted={emitted:?} ml={ml:?}"
        );
    }
}

/// Whether the value-heap runtime STORE is present beside the `cdz` binary. CI's bare `test` job runs
/// `cargo test --workspace` with NO `cargo xtask build`, so there is no store — and `cdz run-emitted`
/// resolves the runtime (to run the emitted module) BEFORE it can report a verdict, so even an
/// out-of-subset program traps "no runtime in store" + exits non-zero storeless. Skip when absent; the
/// store-having `gate` + `@test suites` jobs exercise it fully. Mirrors `test_manifest_cli`'s guard.
fn store_present() -> bool {
    // HASH-AWARE: the store must hold the CURRENT runtime — `<store>/<REQUIRED_RUNTIME_HASH>.wasm` is a
    // file — NOT just be a non-empty dir. Resolve the store dir the SAME way the runtime resolver does
    // (`CADENZA_STORE` first, else `<target>/cadenza-store`). A NON-EMPTY-only check passed on a STALE
    // rust-cached store (an OLDER runtime hash after a hash bump with no fresh `cargo xtask build`) → the
    // runtime-driving test ran → the resolver missed the current hash → "no runtime of content address …
    // refusing to run" red (this red the REQUIRED `test (macos-latest)` job trunk-wide). Checking the exact
    // `<hash>.wasm` makes a stale/empty store read as absent so the test SKIPS — as it does in the storeless
    // CI job (where xtask sets `CADENZA_STORE=<empty temp dir>`).
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

#[test]
fn run_emitted_declines_out_of_subset() {
    // A program outside the emit subset (a string) → emit-src-bytes returns None → `declined`, exit 0.
    // run-emitted resolves the runtime before reporting, so skip storeless (the storeless CI test job).
    if !store_present() {
        eprintln!(
            "skipping: no cadenza-store (storeless test job) — run-emitted resolves the runtime"
        );
        return;
    }
    let (ok, out) = run_emitted("(do (def (main) \"hello\") (export main))");
    assert!(ok, "a decline is a verdict, not a shell failure: {out}");
    assert_eq!(out, "declined", "out-of-subset → declined: {out:?}");
}

#[test]
fn run_emitted_out_of_range_literal_declines_not_harness_error() {
    // An out-of-range Int64 literal (2^63) makes the emit DRIVER trap (checked-arith overflow embedding
    // it). That is the program being inexpressible → `declined` (exit 0), AGREEING with run-ml — NOT a
    // non-zero harness error (which would grade harness-broken / NotYet). Breaker + v-compiler-ml W4.
    // Like the sibling decline test: run-emitted resolves the runtime before reporting (the driver is the
    // full ML compiler), so skip storeless (the storeless `cargo test --workspace` CI job) — the
    // store-having gate exercises it. Without this guard the test reds that job (it landed unguarded).
    if !store_present() {
        eprintln!(
            "skipping: no cadenza-store (storeless test job) — run-emitted resolves the runtime"
        );
        return;
    }
    let (ok, out) = run_emitted("(do (def (main) (- 0 9223372036854775808)) (export main))");
    assert!(
        ok,
        "an out-of-range literal is a decline verdict (exit 0), not a harness error: {out}"
    );
    assert_eq!(
        out, "declined",
        "out-of-range literal → declined (matches run-ml): {out:?}"
    );
}

#[test]
fn run_emitted_an_explicit_dash_reads_stdin_not_a_file_named_dash() {
    // REGRESSION: `run-emitted -` (the stdin MARKER the other pipe commands use) used to fall through to
    // `read_to_string("-")`, leaking the raw `cannot read -: No such file or directory (os error 2)` errno.
    // Now `-` reads stdin like a missing arg. run-emitted resolves the runtime for an in-subset program, so
    // store-guard the verdict assert (storeless CI); the errno-leak assertion holds regardless.
    let exe = env!("CARGO_BIN_EXE_cdz");
    let mut child = Command::new(exe)
        .args(["run-emitted", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn cdz run-emitted -");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"(do (def (main) 42) (export main))")
        .unwrap();
    let out = child.wait_with_output().expect("wait");
    let stderr = String::from_utf8_lossy(&out.stderr);
    // The core fix: `-` is the stdin marker, NOT a file to open — no errno leak, whether or not a store is
    // present (the read happens before any runtime resolution).
    assert!(
        !stderr.contains("cannot read -"),
        "must NOT leak the `cannot read -` errno — `-` is the stdin marker: {stderr}"
    );
    if store_present() {
        assert!(out.status.success(), "run-emitted - exits 0 (a verdict)");
        assert_eq!(
            String::from_utf8_lossy(&out.stdout).trim(),
            "value 42",
            "run-emitted - reads the program from stdin"
        );
    }
}

#[test]
fn run_emitted_on_an_unreadable_file_exits_non_zero() {
    // The one non-zero exit: a harness READ failure (no verdict).
    let exe = env!("CARGO_BIN_EXE_cdz");
    let dir = std::env::temp_dir().join(format!("cdz-runemit-missing-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let missing = dir.join("nope.sexp");
    let out = Command::new(exe)
        .arg("run-emitted")
        .arg(&missing)
        .output()
        .expect("spawn");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        !out.status.success(),
        "an unreadable input is a non-zero harness exit"
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("cannot read"),
        "names the read failure"
    );
}
