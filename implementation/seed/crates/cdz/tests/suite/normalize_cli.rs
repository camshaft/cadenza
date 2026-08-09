//! End-to-end tests for `cdz normalize --match-to-let` — the opt-in single-clause-irrefutable-`match`
//! →`let` codemod. Pins the CLI disposition contract (stdout / `--check` exit codes / refutable-safe)
//! AND a store-guarded SEMANTIC-PRESERVATION check: a normalized program compiles + runs to the SAME
//! value as the original. The normalization deliberately changes the AST shape (a match becomes a let),
//! so it is a separate command from `fmt` (which is structure-preserving); these tests are its gate.

use std::process::Command;

#[path = "../common/mod.rs"]
mod common;
use common::write_stdin_tolerating_broken_pipe;

/// Run `cdz normalize <args…>`, returning (exit_ok, stdout, stderr).
fn normalize(args: &[&str]) -> (bool, String, String) {
    let exe = env!("CARGO_BIN_EXE_cdz");
    let mut a = vec!["normalize"];
    a.extend_from_slice(args);
    let out = Command::new(exe).args(&a).output().expect("spawn cdz");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Compile FILE and run it, returning the rendered result line (or the error text). Mirrors the
/// store-resolution the runtime uses.
fn compile_and_run(path: &str, wasm: &str) -> String {
    let exe = env!("CARGO_BIN_EXE_cdz");
    let c = Command::new(exe)
        .args(["compile", path, "-o", wasm])
        .output()
        .expect("spawn cdz compile");
    if !c.status.success() {
        return format!("COMPILE-FAIL: {}", String::from_utf8_lossy(&c.stderr));
    }
    let r = Command::new(exe)
        .args(["run", wasm])
        .output()
        .expect("spawn cdz run");
    format!(
        "{}{}",
        String::from_utf8_lossy(&r.stdout).trim(),
        String::from_utf8_lossy(&r.stderr).trim()
    )
}

/// Whether the value-heap runtime STORE is present (CI's bare `test` job is storeless — no
/// `cargo xtask build` — so the compile+run value check must SKIP there). Same resolution as the
/// runtime resolver; mirrors `run_ml_cli`/`run_emitted_cli`'s guard.
fn store_present() -> bool {
    // HASH-AWARE: the store must hold the CURRENT runtime — `<store>/<REQUIRED_RUNTIME_HASH>.wasm` is a
    // file — NOT just be a non-empty dir. A NON-EMPTY-only check passed on a STALE rust-cached store (an
    // OLDER runtime hash after a hash bump) → the runtime-driving test ran → the resolver missed the
    // current hash → "no runtime of content address … refusing to run" red. Checking the exact `<hash>.wasm`
    // makes a stale/empty store read as absent so the test SKIPS (as in the storeless CI job, where xtask
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

/// Write `src` to a unique temp `.cdz` file; return (dir, path).
fn temp_src(tag: &str, src: &str) -> (std::path::PathBuf, String) {
    let dir = std::env::temp_dir().join(format!("cdz-normalize-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("m.cdz");
    std::fs::write(&path, src).unwrap();
    (dir, path.to_str().unwrap().to_string())
}

#[test]
fn stdout_lowers_an_irrefutable_single_clause_match_to_a_let() {
    let (dir, path) = temp_src("stdout", "def f(p) = match p with | (a, b) => a + b\n");
    let (ok, out, err) = normalize(&["--match-to-let", &path, "--stdout"]);
    assert!(ok, "exit ok; stderr={err}");
    assert!(
        out.contains("let (a, b) = p in"),
        "expected a let; got:\n{out}"
    );
    assert!(
        !out.contains("match"),
        "the match should be gone; got:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn refutable_match_is_left_unchanged_and_check_passes() {
    let src = "def f(p) = match p with | Some(x) => x | None => 0\n";
    let (dir, path) = temp_src("refutable", src);
    // --stdout leaves it as a match (still multi-clause + a ctor pattern).
    let (ok, out, _) = normalize(&["--match-to-let", &path, "--stdout"]);
    assert!(ok);
    assert!(
        out.contains("match p with"),
        "refutable match must stay a match:\n{out}"
    );
    // --check: nothing to normalize → exit 0.
    let (check_ok, _, _) = normalize(&["--match-to-let", &path, "--check"]);
    assert!(check_ok, "--check on an already-normal file must exit 0");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_exits_nonzero_when_a_file_would_be_normalized() {
    let (dir, path) = temp_src("check", "def f(p) = match p with | x => x\n");
    let (ok, out, _) = normalize(&["--match-to-let", &path, "--check"]);
    assert!(!ok, "--check must exit non-zero when a file would change");
    assert!(
        out.contains("would normalize"),
        "names the file; got:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Run `cdz normalize <args…>` with `stdin` piped in, returning (exit_ok, stdout, stderr). For the
/// stdin (`-`) disposition path, which reads real `std::io::stdin()` (not in-process unit-testable from
/// cadenza-syntax) — the end-to-end complement of that crate's `emits_to_stdout` unit test.
fn normalize_stdin(stdin: &str, args: &[&str]) -> (bool, String, String) {
    let exe = env!("CARGO_BIN_EXE_cdz");
    let mut a = vec!["normalize"];
    a.extend_from_slice(args);
    let mut child = Command::new(exe)
        .args(&a)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn cdz normalize (stdin)");
    write_stdin_tolerating_broken_pipe(child.stdin.take().unwrap(), stdin.as_bytes());
    let out = child.wait_with_output().expect("wait cdz");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn stdin_check_reds_when_input_would_normalize_and_is_clean_otherwise() {
    // The stdin disposition bug (found probing `cdz`, fixed by v-syntax in cadenza-syntax cli.rs PR #1127
    // 0c7a0e134): `normalize - --check` used to hit the stdout-emit branch UNCONDITIONALLY (there's no file
    // to edit), so it printed the transformed program and exited 0 — SILENTLY IGNORING --check even when the
    // input would normalize. A CI pipe (`… | cdz normalize - --match-to-let --check`) then got a FALSE PASS.
    // The fix honors --check on stdin. Pin it end-to-end over the REAL stdin pipe (not unit-testable from the
    // syntax crate): a would-normalize input REDS (exit non-zero) + names `<stdin>`, an already-normal input
    // is clean (exit 0). No store — a pure text disposition.
    // Would-normalize: a single-clause irrefutable match → --check must RED, NOT silently print + exit 0.
    let (ok, out, err) = normalize_stdin(
        "def f(x: Int64) -> Int64 = match x with | y => y + 1\n",
        &["-", "--from", "ml", "--match-to-let", "--check"],
    );
    assert!(
        !ok,
        "normalize - --check must RED when stdin would normalize (not a silent exit-0):\nstdout:\n{out}\nstderr:\n{err}"
    );
    assert!(
        out.contains("would normalize") && out.contains("<stdin>"),
        "the --check report names the stdin source as would-normalize:\nstdout:\n{out}\nstderr:\n{err}"
    );
    // The lowered `let` must NOT be emitted under --check (it inspects, never writes/prints the transform).
    assert!(
        !out.contains("let y ="),
        "--check must not print the transformed program (that's the --stdout mode):\nstdout:\n{out}"
    );

    // Already-normal (no single-clause match to lower): --check is clean → exit 0.
    let (ok2, _out2, err2) = normalize_stdin(
        "def f(x: Int64) -> Int64 = x + 1\n",
        &["-", "--from", "ml", "--match-to-let", "--check"],
    );
    assert!(
        ok2,
        "normalize - --check exits 0 when stdin is already normal:\nstderr:\n{err2}"
    );
}

#[test]
fn single_variant_sum_ctor_lowers_but_multi_variant_stays() {
    // Type-aware: a ctor pattern on a SINGLE-variant sum is irrefutable → lowers; a multi-variant
    // one stays a match (would-fall-through → refutable).
    let (dir, path) = temp_src(
        "single",
        "type Wrapper = | Wrap(Int64)\ndef f(w) = match w with | Wrap(x) => x\n",
    );
    let (ok, out, _) = normalize(&["--match-to-let", &path, "--stdout"]);
    assert!(ok);
    assert!(
        out.contains("let Wrap(x) = w in"),
        "single-variant ctor lowers:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&dir);

    let (dir2, path2) = temp_src(
        "multi",
        "type Opt = | Some(Int64) | None\ndef f(o) = match o with | Some(x) => x\n",
    );
    let (ok2, out2, _) = normalize(&["--match-to-let", &path2, "--stdout"]);
    assert!(ok2);
    assert!(
        out2.contains("match o with"),
        "multi-variant ctor stays a match:\n{out2}"
    );
    let _ = std::fs::remove_dir_all(&dir2);
}

#[test]
fn requires_a_normalization_flag() {
    let (dir, path) = temp_src("noflag", "def f(p) = match p with | x => x\n");
    let (ok, _, err) = normalize(&[&path, "--stdout"]);
    assert!(!ok, "no `--match-to-let` must error");
    assert!(err.contains("normalization is required"), "err:\n{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn normalization_preserves_runtime_semantics() {
    // The crux: the lowered `let` must compute the SAME value as the original `match`. Store-guarded
    // (CI's bare `test` job is storeless — the compile+run value check only runs where a store exists).
    if !store_present() {
        return;
    }
    let src = "def main() -> Int64 = match (3, 4) with | (a, b) => a + b\nexport { main }\n";
    let (dir, path) = temp_src("sem", src);
    let before = compile_and_run(&path, dir.join("before.wasm").to_str().unwrap());
    // Normalize in place, then compile + run the result.
    let (ok, _, err) = normalize(&["--match-to-let", &path]);
    assert!(ok, "in-place normalize; stderr={err}");
    let normalized = std::fs::read_to_string(&path).unwrap();
    assert!(
        normalized.contains("let (a, b) = "),
        "the match lowered:\n{normalized}"
    );
    let after = compile_and_run(&path, dir.join("after.wasm").to_str().unwrap());
    assert_eq!(before, after, "match→let must preserve the runtime value");
    assert_eq!(before, "7", "sanity: (3,4) → 3+4 = 7 (before={before})");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn normalization_preserves_semantics_when_the_pattern_shadows_a_scrutinee_name() {
    // The subtle correctness case: the pattern binds a name the SCRUTINEE also uses. A `match e with
    // | x => body` evaluates `e` (seeing the OUTER `x`) before rebinding `x`; the lowered
    // `let x = e in body` must be NON-RECURSIVE so `e` also sees the outer `x` — not the being-bound
    // one. `let x=10 in match x+5 with | x => x` = 15; the lowered `let x=10 in let x=x+5 in x` must
    // ALSO be 15 (the inner `x+5` reads the outer 10). Pins that the emitted let-binding shape is
    // non-recursive — a subtle property a future change could silently break.
    if !store_present() {
        return;
    }
    let src =
        "def main() -> Int64 =\n  let x = 10 in\n  match x + 5 with | x => x\nexport { main }\n";
    let (dir, path) = temp_src("shadow", src);
    let before = compile_and_run(&path, dir.join("before.wasm").to_str().unwrap());
    let (ok, _, err) = normalize(&["--match-to-let", &path]);
    assert!(ok, "in-place normalize; stderr={err}");
    let after = compile_and_run(&path, dir.join("after.wasm").to_str().unwrap());
    assert_eq!(before, after, "shadowing match→let must preserve the value");
    assert_eq!(
        before, "15",
        "outer x=10, scrutinee x+5=15, rebind x=15 (before={before})"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
