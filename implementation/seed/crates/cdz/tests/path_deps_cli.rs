//! End-to-end tests for `cdz` PATH DEPENDENCIES — a `Project.cdz` `def deps = ["../sibling", …]` that
//! `cdz run` builds and PEER-BINDS across the component boundary (the cargo-analogue path-dep, Increment
//! 1; ecosystem-design registry/lockfile deferred).
//!
//! The mechanism reuses v-peer-linking's cross-component binding: for each dep, `cdz run` resolves the
//! sibling's `Project.cdz`, compiles its entry to a component published under `cadenza:<dep-name>/api`,
//! and hands it to the runner as a `--peer` — so `run_with_peers` composes consumer + deps in one
//! wasmtime store. The consumer's SOURCE binds the dep's interface by that exact name. These drive the
//! built `cdz` binary over real sibling project dirs. Scalar programs (no value-heap) so no runtime store
//! is needed — the compose is hermetic.

use std::process::Command;

/// Run `cdz <args…>` from `cwd`, returning (exit_ok, stdout, stderr).
fn run_in(cwd: &std::path::Path, args: &[&str]) -> (bool, String, String) {
    let exe = env!("CARGO_BIN_EXE_cdz");
    let out = Command::new(exe)
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("spawn cdz");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// A workspace root with a `mathlib` dep project (exports `add`, published as `cadenza:mathlib/api`) and
/// an `app` consumer project that declares `def deps = ["../mathlib"]` and binds that interface. Returns
/// (root, app_dir). The consumer's `main(x)` computes `Math.add(x, 10)` via the peer-bound dep.
fn workspace(tag: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let root = std::env::temp_dir().join(format!("cdz-pathdep-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("mathlib")).expect("mkdir mathlib");
    std::fs::create_dir_all(root.join("app")).expect("mkdir app");

    std::fs::write(
        root.join("mathlib/Project.cdz"),
        "def name = \"mathlib\"\ndef entry = \"lib.sexp\"\n",
    )
    .unwrap();
    std::fs::write(
        root.join("mathlib/lib.sexp"),
        "(do (def (add (: x Int64) (: y Int64)) (+ x y)) (export add))",
    )
    .unwrap();

    std::fs::write(
        root.join("app/Project.cdz"),
        "def name = \"app\"\ndef entry = \"main.sexp\"\ndef deps = [\"../mathlib\"]\n",
    )
    .unwrap();
    std::fs::write(
        root.join("app/main.sexp"),
        "(do (effect Math (op add (-> Int64 Int64 Int64))) (bind Math \"cadenza:mathlib/api\") \
         (def (main (: x Int64)) (host (Math) (Math.add x 10))) (export main))",
    )
    .unwrap();

    let app = root.join("app");
    (root, app)
}

#[test]
fn cdz_run_builds_and_binds_a_path_dependency() {
    // The headline: `cdz run` on a project with `def deps = ["../mathlib"]` builds the dep, peer-binds its
    // `cadenza:mathlib/api` export, and the consumer calls into it. `main(5)` = Math.add(5, 10) = 15.
    let (root, app) = workspace("run");
    let (ok, out, err) = run_in(&app, &["run", "--call", "main", "--arg", "5"]);
    assert!(ok, "cdz run with a path-dep should succeed: {out}{err}");
    assert_eq!(
        out.trim(),
        "15",
        "the consumer calls the dep's add across the component boundary: {out}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_path_dependency_run_leaves_no_temp_components_behind() {
    // The consumer's built component AND each dep's built component are temp artifacts; both must be
    // cleaned up after the run (like a plain `cdz run <project>`), leaving no `.cdz-run-*` files.
    let (root, app) = workspace("clean");
    let (ok, _o, err) = run_in(&app, &["run", "--call", "main", "--arg", "1"]);
    assert!(ok, "run failed: {err}");
    for proj in ["app", "mathlib"] {
        let leftovers: Vec<_> = std::fs::read_dir(root.join(proj))
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with(".cdz-run-"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "no temp component left in {proj}: {leftovers:?}"
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_missing_path_dependency_fails_with_a_clear_error() {
    // A `def deps` entry pointing at a directory with no `Project.cdz` is a clear, non-zero error naming
    // the dependency — not a silent skip or an opaque downstream failure.
    let (root, app) = workspace("missing");
    std::fs::write(
        app.join("Project.cdz"),
        "def name = \"app\"\ndef entry = \"main.sexp\"\ndef deps = [\"../nonexistent\"]\n",
    )
    .unwrap();
    let (ok, _o, err) = run_in(&app, &["run", "--call", "main", "--arg", "5"]);
    assert!(!ok, "a missing path-dep must fail (non-zero exit)");
    assert!(
        err.contains("dependency `../nonexistent`"),
        "the error names the unresolvable dependency: {err}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn metadata_reports_the_declared_path_dependencies() {
    // `cdz metadata` should surface the manifest's `deps` so a tool/editor sees the project graph. (The
    // manifest parser reads `def deps`; metadata emits the raw field.)
    let (root, app) = workspace("meta");
    let (ok, out, err) = run_in(&app, &["metadata"]);
    assert!(ok, "cdz metadata failed: {out}{err}");
    assert!(
        out.contains("../mathlib"),
        "metadata reports the declared path-dep: {out}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn cdz_run_binds_multiple_path_dependencies() {
    // `def deps` is a LIST — a consumer can depend on several projects at once, each peer-bound under its
    // own `cadenza:<dep>/api`. Build two deps (`inclib` exports `inc`, `neglib` exports `neg`) and a
    // consumer that binds BOTH; `main(5)` = neg(inc(5)) = neg(6) = -6, proving both compose in one run.
    let root = std::env::temp_dir().join(format!("cdz-pathdep-multi-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for (name, op, body) in [("inclib", "inc", "(+ x 1)"), ("neglib", "neg", "(- 0 x)")] {
        std::fs::create_dir_all(root.join(name)).unwrap();
        std::fs::write(
            root.join(name).join("Project.cdz"),
            format!("def name = \"{name}\"\ndef entry = \"lib.sexp\"\n"),
        )
        .unwrap();
        std::fs::write(
            root.join(name).join("lib.sexp"),
            format!("(do (def ({op} (: x Int64)) {body}) (export {op}))"),
        )
        .unwrap();
    }
    std::fs::create_dir_all(root.join("app")).unwrap();
    std::fs::write(
        root.join("app/Project.cdz"),
        "def name = \"app\"\ndef entry = \"main.sexp\"\ndef deps = [\"../inclib\", \"../neglib\"]\n",
    )
    .unwrap();
    // Consumer binds BOTH deps' interfaces and composes them: neg(inc(x)).
    std::fs::write(
        root.join("app/main.sexp"),
        "(do (effect Inc (op inc (-> Int64 Int64))) (bind Inc \"cadenza:inclib/api\") \
         (effect Neg (op neg (-> Int64 Int64))) (bind Neg \"cadenza:neglib/api\") \
         (def (main (: x Int64)) (host (Inc Neg) (Neg.neg (Inc.inc x)))) (export main))",
    )
    .unwrap();
    let (ok, out, err) = run_in(&root.join("app"), &["run", "--call", "main", "--arg", "5"]);
    assert!(ok, "cdz run with two path-deps should succeed: {out}{err}");
    assert_eq!(
        out.trim(),
        "-6",
        "both deps compose: neg(inc(5)) = neg(6) = -6: {out}"
    );
    let _ = std::fs::remove_dir_all(&root);
}
