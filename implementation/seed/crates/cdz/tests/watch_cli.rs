//! End-to-end tests for `cdz watch` — the `cargo watch` analogue: watch a project's source files and
//! re-run a command (`check`/`test`/`build`) on every change.
//!
//! `watch` is a long-running loop, so these drive the REAL process: scaffold a project, spawn `cdz
//! watch`, poll its captured output until the "watching …" banner appears, then TOUCH a source file and
//! assert a re-run fires — and, crucially, that a `watch --exec build` does NOT self-trigger on the
//! `.wasm`/`link-map.txt` artifacts it writes into the watched directory (the positive source-file
//! filter). The process is killed at the end of each test. Timings are generous to stay non-flaky under
//! a loaded CI; the assertions are on observable output, not exact latency.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Scaffold a fresh `cdz new` project under a unique temp dir; return (root, project_dir).
fn scaffold(tag: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let root = std::env::temp_dir().join(format!("cdz-watch-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("mkdir");
    let exe = env!("CARGO_BIN_EXE_cdz");
    let ok = Command::new(exe)
        .args(["new", "app"])
        .current_dir(&root)
        .status()
        .expect("spawn cdz new")
        .success();
    assert!(ok, "cdz new should scaffold a project");
    (root.clone(), root.join("app"))
}

/// Spawn `cdz watch <extra…>` in `cwd`, its stdout+stderr redirected to a file we can poll. Returns the
/// child and the capture-file path. (A pipe would need a reader thread to avoid blocking on a full buffer;
/// a file is simpler + lets the test poll the accumulated output.)
fn spawn_watch(
    cwd: &std::path::Path,
    extra: &[&str],
    tag: &str,
) -> (std::process::Child, std::path::PathBuf) {
    let exe = env!("CARGO_BIN_EXE_cdz");
    let cap = std::env::temp_dir().join(format!("cdz-watch-cap-{tag}-{}.txt", std::process::id()));
    let f = std::fs::File::create(&cap).expect("create capture file");
    let f2 = f.try_clone().expect("clone capture handle");
    let mut args = vec!["watch"];
    args.extend_from_slice(extra);
    let child = Command::new(exe)
        .args(&args)
        .current_dir(cwd)
        .stdout(Stdio::from(f))
        .stderr(Stdio::from(f2))
        .spawn()
        .expect("spawn cdz watch");
    (child, cap)
}

/// The current contents of the capture file.
fn read_cap(cap: &std::path::Path) -> String {
    let mut s = String::new();
    if let Ok(mut f) = std::fs::File::open(cap) {
        let _ = f.read_to_string(&mut s);
    }
    s
}

/// Poll the capture file until `needle` appears (up to `timeout`); return whether it showed up.
fn wait_for(cap: &std::path::Path, needle: &str, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if read_cap(cap).contains(needle) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    read_cap(cap).contains(needle)
}

/// Count occurrences of `needle` in the capture file.
fn count(cap: &std::path::Path, needle: &str) -> usize {
    read_cap(cap).matches(needle).count()
}

#[test]
fn watch_reruns_check_when_a_source_file_changes() {
    let (root, proj) = scaffold("check");
    let (mut child, cap) = spawn_watch(&proj, &[], "check");

    // The watcher announces itself + names the default command it re-runs.
    assert!(
        wait_for(&cap, "watching", Duration::from_secs(20)),
        "watch prints a startup banner: {}",
        read_cap(&cap)
    );
    assert!(
        read_cap(&cap).contains("cdz check"),
        "the default watch command is check: {}",
        read_cap(&cap)
    );

    // Touch a source file → a re-run must fire. (Rewrite the entry so the mtime + content both change.)
    std::thread::sleep(Duration::from_millis(300)); // let the initial run settle before editing
    std::fs::write(
        proj.join("main.cdz"),
        "def main() -> Int64 = 0\n\nexport { main }\n",
    )
    .expect("rewrite entry");
    let rer = wait_for(&cap, "change detected", Duration::from_secs(20));

    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_file(&cap);
    assert!(rer, "a source-file change re-runs the command");
}

#[test]
fn watch_reports_diagnostics_from_the_rerun() {
    let (root, proj) = scaffold("diag");
    let (mut child, cap) = spawn_watch(&proj, &[], "diag");
    assert!(
        wait_for(&cap, "watching", Duration::from_secs(20)),
        "startup banner: {}",
        read_cap(&cap)
    );

    // Introduce a syntax error → the re-run's `cdz check` must surface a diagnostic.
    std::thread::sleep(Duration::from_millis(300));
    std::fs::write(proj.join("main.cdz"), "def broken( = 1\n").expect("write broken source");
    let saw_err = wait_for(&cap, "error:", Duration::from_secs(20));
    let captured = read_cap(&cap);

    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_file(&cap);
    assert!(saw_err, "the re-run reports the new diagnostic: {captured}");
}

#[test]
fn watch_build_does_not_self_trigger_on_its_own_artifacts() {
    // The self-trigger guard: `watch --exec build` writes `main.wasm`/`link-map.txt` INTO the watched
    // dir. Those are NOT source files, so the positive filter must ignore them — otherwise the build
    // would re-trigger itself in an infinite loop. Assert NO "change detected" fires from the artifact
    // writes alone (we never touch a source file here).
    let (root, proj) = scaffold("build");
    let (mut child, cap) = spawn_watch(&proj, &["--exec", "build"], "build");
    assert!(
        wait_for(&cap, "watching", Duration::from_secs(20)),
        "startup banner: {}",
        read_cap(&cap)
    );
    // Wait for the initial build to write its artifacts, then some more — a self-trigger loop would keep
    // emitting "change detected".
    assert!(
        wait_for(&cap, "main.wasm", Duration::from_secs(20)),
        "the initial build writes its component: {}",
        read_cap(&cap)
    );
    std::thread::sleep(Duration::from_secs(2));
    let self_triggers = count(&cap, "change detected");

    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_file(&cap);
    assert_eq!(
        self_triggers, 0,
        "the build's own artifact writes must NOT re-trigger the watch (no self-trigger loop)"
    );
}
