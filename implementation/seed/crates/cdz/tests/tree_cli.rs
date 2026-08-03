//! End-to-end tests for `cdz tree` — the project DEPENDENCY-TREE view (the `cargo tree` analogue).
//!
//! `cdz tree` resolves a project's `Project.cdz`, prints its `name (dir)`, then recurses into each `def
//! deps` path-dependency (indented beneath its parent), so the whole transitive graph is legible. These
//! drive the built binary over real sibling project dirs and assert: a nested graph renders with tree
//! connectors, a DIAMOND/cycle dep is marked `(*)` and not re-expanded, an unresolvable dep is marked
//! `*unresolved*` (partial tree, not an abort), and a project with no deps prints just its root line.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Run `cdz <args…>` under a WALL-CLOCK DEADLINE, returning (exit_ok, stdout, stderr). The cycle tests
/// assert `cdz tree` TERMINATES on a dependency cycle (a→b→a); a bare `Command::output()` blocks with no
/// timeout, so a regression that dropped the visited-set guard would spin `cdz tree` forever and HANG the
/// whole test binary / CI job, not just the one test. This spawns the child, polls `try_wait()` against a
/// deadline, and on timeout KILLS it and PANICS — so a non-terminating regression fails FAST with a clear
/// message instead of hanging indefinitely. The bound is generous (`cdz tree` on these tiny fixtures is
/// milliseconds) so it never flakes on a loaded host, only firing on a genuine infinite loop.
///
/// `try_wait()` REAPS the child when it returns `Some(status)`, so we capture that `ExitStatus` and read
/// the piped stdout/stderr handles DIRECTLY — never a second `wait_with_output()`, which would double-wait
/// on an already-reaped child and can fail (`No child processes`). `cdz tree`'s output is a handful of
/// lines (far under the pipe buffer), so reading it after exit can't deadlock.
fn run(args: &[&str]) -> (bool, String, String) {
    let exe = env!("CARGO_BIN_EXE_cdz");
    let mut child = Command::new(exe)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn cdz");
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        match child.try_wait().expect("try_wait on cdz") {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                panic!(
                    "`cdz {}` did not terminate within 30s — likely a `cdz tree` regression looping on a \
                     dependency cycle (the visited-set terminator broke); killed the child",
                    args.join(" ")
                );
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    };
    // Read the piped handles directly — the child is already reaped, so `wait_with_output()` would be a
    // double-wait. Read into bytes then `from_utf8_lossy` (NOT `read_to_string`, which PANICS on a single
    // non-UTF8 byte) so non-UTF8 diagnostic output is captured LOSSILY — the same robustness the prior
    // `wait_with_output()` + `from_utf8_lossy` had. Both handles were piped at spawn, so `take()` yields them.
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    child
        .stdout
        .take()
        .expect("piped stdout")
        .read_to_end(&mut stdout)
        .expect("read stdout");
    child
        .stderr
        .take()
        .expect("piped stderr")
        .read_to_end(&mut stderr)
        .expect("read stderr");
    (
        status.success(),
        String::from_utf8_lossy(&stdout).into_owned(),
        String::from_utf8_lossy(&stderr).into_owned(),
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

#[test]
fn tree_terminates_on_a_dependency_cycle() {
    // A true CYCLE (a → b → a), not just a diamond — the diamond above is a DAG that would terminate even
    // without the guard (it would merely re-expand), but a cycle would recurse FOREVER without the
    // visited-set. Pin that the walk terminates (exit 0, in bounded time) and marks the back-edge to `a`
    // with `(*)` instead of re-expanding it. `run`'s wall-clock deadline makes this fail FAST (kill +
    // panic) if a regression dropped/mis-keyed the visited-set, rather than hanging the CI job.
    let root = workspace("cycle");
    project(&root, "a", &["../b"]);
    project(&root, "b", &["../a"]);
    let (ok, out, err) = run(&["tree", root.join("a").to_str().unwrap()]);
    assert!(
        ok,
        "a cyclic graph still succeeds (does not loop): {out}{err}"
    );
    assert!(
        out.starts_with("a ("),
        "root line names the entry project: {out}"
    );
    assert!(out.contains("b (../b)"), "b is a's dep: {out}");
    // The back-edge to `a` (under b) is marked `(*)` and not re-expanded — proof the cycle terminated at
    // the already-visited root rather than descending into a's deps again.
    assert!(
        out.contains("a (../a) (*)"),
        "the back-edge to the root is marked (*) and not re-expanded: {out}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn tree_json_terminates_on_a_dependency_cycle() {
    // The `--json` counterpart of the text cycle pin: a → b → a must emit a FINITE object, with the
    // back-edge to `a` marked `repeated: true` and carrying no nested `deps` (not re-expanded). Parsed,
    // so a malformed / non-terminating object fails loudly rather than substring-matching.
    let root = workspace("cycle-json");
    project(&root, "a", &["../b"]);
    project(&root, "b", &["../a"]);
    let (ok, out, err) = run(&["tree", root.join("a").to_str().unwrap(), "--json"]);
    assert!(ok, "a cyclic graph emits JSON without looping: {out}{err}");
    let v: serde_json::Value =
        serde_json::from_str(out.trim()).unwrap_or_else(|e| panic!("valid JSON ({e}): {out}"));
    assert_eq!(v["name"], "a", "root name: {out}");
    // b is the sole dep, expanded once.
    let b = &v["deps"][0];
    assert_eq!(b["name"], "b", "b is the dep: {out}");
    // Under b, the back-edge to a is marked repeated and not re-expanded.
    let back = &b["deps"][0];
    assert_eq!(back["path"], "../a", "the back-edge names a's path: {out}");
    assert_eq!(
        back["repeated"], true,
        "the back-edge is marked repeated: {out}"
    );
    assert!(
        back["deps"].is_null(),
        "the repeated back-edge is not re-expanded: {out}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn tree_terminates_on_a_self_dependency() {
    // The degenerate cycle: a project that lists ITSELF (`def deps = ["."]`) as a dependency. The
    // visited-set inserts the root's canonical dir before walking, so the self-edge resolves to an
    // already-visited dir and is marked `(*)` rather than recursing into the same project forever. (`cdz
    // add` refuses a self-dep, but a hand-edited manifest can still contain one, so `cdz tree` must stay
    // total on it.) `run`'s deadline fails fast if the self-edge ever loops.
    let root = workspace("self");
    project(&root, "a", &["."]);
    let (ok, out, err) = run(&["tree", root.join("a").to_str().unwrap()]);
    assert!(ok, "a self-dependency does not loop: {out}{err}");
    assert!(out.starts_with("a ("), "root line names the project: {out}");
    assert!(
        out.contains("a (.) (*)"),
        "the self-edge is marked (*) and not re-expanded: {out}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn tree_collapses_path_aliases_to_the_same_dir_in_the_visited_set() {
    // The visited-set is keyed by the CANONICAL dir, not the raw manifest spelling — so two deps that name
    // the SAME directory via different path spellings (`../lib` and `../lib/.`) are recognized as one node:
    // the first expands, the second is a diamond marked `(*)`. A visited-set keyed by the raw string would
    // MISS this (treat them as two distinct deps and expand both), so this pins the canonicalization that
    // makes cycle/diamond detection correct under path aliasing.
    let root = workspace("alias");
    project(&root, "lib", &[]);
    project(&root, "app", &["../lib", "../lib/."]);
    let (ok, out, err) = run(&["tree", root.join("app").to_str().unwrap()]);
    assert!(
        ok,
        "cdz tree with aliased dep paths should succeed: {out}{err}"
    );
    // Both spellings appear (echoed as the raw manifest refs), but exactly ONE is expanded and the other
    // marked `(*)` — proof the canonical-dir key collapsed the alias.
    assert!(
        out.contains("lib (../lib)"),
        "the first spelling is shown: {out}"
    );
    assert!(
        out.contains("lib (../lib/.) (*)"),
        "the aliased second spelling is recognized as the same dir and marked (*): {out}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn tree_json_emits_the_nested_graph() {
    // `--json` emits ONE nested `{name, path, deps: [...]}` object — the shape a tool consumes without
    // parsing the box-drawing connectors. Same graph as the text test: app → mathlib → util, app → util
    // (a diamond → `repeated: true`, not re-expanded), + an unresolvable dep (`unresolved: true`). Parsed,
    // not substring-checked, so a malformed object fails loudly.
    let root = workspace("json");
    project(&root, "util", &[]);
    project(&root, "mathlib", &["../util"]);
    project(&root, "app", &["../mathlib", "../util", "../ghost"]);
    let (ok, out, err) = run(&["tree", root.join("app").to_str().unwrap(), "--json"]);
    assert!(ok, "cdz tree --json should succeed: {out}{err}");
    let v: serde_json::Value =
        serde_json::from_str(out.trim()).unwrap_or_else(|e| panic!("valid JSON ({e}): {out}"));
    assert_eq!(v["name"], "app", "root name: {out}");
    let deps = v["deps"].as_array().expect("deps is an array");
    assert_eq!(deps.len(), 3, "three deps: {out}");
    // mathlib expanded, with util nested under it.
    let mathlib = deps
        .iter()
        .find(|d| d["name"] == "mathlib")
        .expect("mathlib dep");
    assert_eq!(
        mathlib["deps"][0]["name"], "util",
        "mathlib's util is nested: {out}"
    );
    // The direct util is the diamond — marked repeated, NOT re-expanded (no nested deps).
    let util = deps
        .iter()
        .find(|d| d["path"] == "../util" && d["repeated"] == true)
        .expect("the diamond util is marked repeated");
    assert!(
        util["deps"].is_null(),
        "a repeated node is not re-expanded: {out}"
    );
    // The missing dep is marked unresolved (and carries no name).
    let ghost = deps
        .iter()
        .find(|d| d["path"] == "../ghost")
        .expect("ghost dep");
    assert_eq!(
        ghost["unresolved"], true,
        "the missing dep is unresolved: {out}"
    );
    let _ = std::fs::remove_dir_all(&root);
}
