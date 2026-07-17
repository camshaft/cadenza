//! End-to-end tests for `cdz tree` — the project DEPENDENCY-TREE view (the `cargo tree` analogue).
//!
//! `cdz tree` resolves a project's `Project.cdz`, prints its `name (dir)`, then recurses into each `def
//! deps` path-dependency (indented beneath its parent), so the whole transitive graph is legible. These
//! drive the built binary over real sibling project dirs and assert: a nested graph renders with tree
//! connectors, a DIAMOND/cycle dep is marked `(*)` and not re-expanded, an unresolvable dep is marked
//! `*unresolved*` (partial tree, not an abort), and a project with no deps prints just its root line.

use std::process::Command;

/// Run `cdz <args…>`, returning (exit_ok, stdout, stderr).
fn run(args: &[&str]) -> (bool, String, String) {
    let exe = env!("CARGO_BIN_EXE_cdz");
    let out = Command::new(exe).args(args).output().expect("spawn cdz");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// A fresh workspace root under a unique temp dir.
fn workspace(tag: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("cdz-tree-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    root
}

/// Write a project dir `root/<name>` with a manifest naming `name`, an entry, and optional `deps`.
fn project(root: &std::path::Path, name: &str, deps: &[&str]) {
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).expect("mkdir project");
    let deps_line = if deps.is_empty() {
        String::new()
    } else {
        let list = deps
            .iter()
            .map(|d| format!("\"{d}\""))
            .collect::<Vec<_>>()
            .join(", ");
        format!("def deps = [{list}]\n")
    };
    std::fs::write(
        dir.join("Project.cdz"),
        format!("def name = \"{name}\"\ndef entry = \"main.sexp\"\n{deps_line}"),
    )
    .unwrap();
    std::fs::write(dir.join("main.sexp"), "(do (def (main) 0) (export main))").unwrap();
}

#[test]
fn tree_prints_a_nested_dependency_graph() {
    // app → mathlib → util, and app → util directly (a DIAMOND): the tree shows mathlib's util expanded,
    // and the second (direct) util marked `(*)` and NOT re-expanded (the cycle/diamond terminator).
    let root = workspace("nested");
    project(&root, "util", &[]);
    project(&root, "mathlib", &["../util"]);
    project(&root, "app", &["../mathlib", "../util"]);
    let (ok, out, err) = run(&["tree", root.join("app").to_str().unwrap()]);
    assert!(ok, "cdz tree should succeed: {out}{err}");
    // Root, then mathlib with util nested under it, then the diamond util marked (*).
    assert!(
        out.starts_with("app ("),
        "root line names the project: {out}"
    );
    assert!(
        out.contains("mathlib (../mathlib)"),
        "mathlib is a dep: {out}"
    );
    assert!(
        out.contains("util (../util)") && out.contains("(*)"),
        "the diamond util is shown once expanded + once marked (*): {out}"
    );
    // The nested util (under mathlib) is indented deeper than the top-level deps.
    assert!(
        out.contains("│   └── util") || out.contains("    └── util"),
        "mathlib's util is nested beneath it: {out}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn tree_marks_an_unresolvable_dependency() {
    // A `def deps` entry with no `Project.cdz` at its path is marked `*unresolved*` — a partial tree, not
    // an abort (still exit 0, since the root project itself is fine).
    let root = workspace("unresolved");
    project(&root, "app", &["../ghost"]);
    let (ok, out, err) = run(&["tree", root.join("app").to_str().unwrap()]);
    assert!(ok, "an unresolvable dep is not fatal: {out}{err}");
    assert!(
        out.contains("../ghost *unresolved*"),
        "the missing dep is marked unresolved: {out}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn tree_of_a_depless_project_is_just_the_root() {
    // A project with no `def deps` prints only its root `name (dir)` line — no connectors.
    let root = workspace("solo");
    project(&root, "solo", &[]);
    let (ok, out, err) = run(&["tree", root.join("solo").to_str().unwrap()]);
    assert!(ok, "cdz tree on a depless project: {out}{err}");
    assert!(out.starts_with("solo ("), "root line: {out}");
    assert!(
        !out.contains("──"),
        "no dependency connectors for a depless project: {out}"
    );
    let _ = std::fs::remove_dir_all(&root);
}
