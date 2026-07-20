//! End-to-end tests for `cdz normalize --match-to-let` — the opt-in single-clause-irrefutable-`match`
//! →`let` codemod. Pins the CLI disposition contract (stdout / `--check` exit codes / refutable-safe)
//! AND a store-guarded SEMANTIC-PRESERVATION check: a normalized program compiles + runs to the SAME
//! value as the original. The normalization deliberately changes the AST shape (a match becomes a let),
//! so it is a separate command from `fmt` (which is structure-preserving); these tests are its gate.

use std::process::Command;

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
    if let Ok(dir) = std::env::var("CADENZA_STORE") {
        return std::path::Path::new(&dir).is_dir()
            && std::fs::read_dir(&dir)
                .map(|mut e| e.next().is_some())
                .unwrap_or(false);
    }
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_cdz"))
        .parent()
        .and_then(|d| d.parent())
        .map(|t| t.join("cadenza-store").exists())
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
