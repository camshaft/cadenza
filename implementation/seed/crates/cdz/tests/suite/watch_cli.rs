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

/// Poll until `needle` occurs at least `want` times in the capture file (up to `timeout`); return
/// whether that count was reached. (For asserting a per-run signal fired N times, not just once.)
fn wait_for_count(cap: &std::path::Path, needle: &str, want: usize, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if count(cap, needle) >= want {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    count(cap, needle) >= want
}

/// Wait until `needle`'s count STOPS CHANGING for a short quiet window, then return the settled count
/// (capped by `max`). A poll-until-stable replacement for a blanket `sleep` when waiting for an
/// uncertain-length event burst to finish (e.g. a macOS spurious startup FS event that may fire a
/// second run): on a platform where nothing further arrives (Linux), the count is stable on the first
/// window and this returns in ~`quiet`, not the full cap — so the common case pays no fixed latency,
/// while a real trailing event still gets absorbed up to `max`. (Copilot PR #523.)
fn settle_count(cap: &std::path::Path, needle: &str, max: Duration) -> usize {
    let quiet = Duration::from_millis(300); // the count must hold steady this long to be "settled"
    let start = Instant::now();
    let mut last = count(cap, needle);
    let mut stable_since = Instant::now();
    while start.elapsed() < max {
        std::thread::sleep(Duration::from_millis(50));
        let now = count(cap, needle);
        if now != last {
            last = now;
            stable_since = Instant::now(); // changed → restart the quiet window
        } else if stable_since.elapsed() >= quiet {
            break; // held steady for the quiet window → settled
        }
    }
    last
}

/// Poll until `path` exists (up to `timeout`); return whether it appeared. Used to detect a build's
/// artifact by its FILE (stabler than grepping the build's stdout for a substring that could change).
fn wait_for_path(path: &std::path::Path, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if path.exists() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    path.exists()
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
    std::thread::sleep(Duration::from_millis(800)); // let the initial run settle before editing
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
fn watch_runs_the_command_exactly_once_on_startup() {
    // REGRESSION (Copilot PR #506): the mid-run-edit rework accidentally left TWO identical initial-run
    // `rerun()` blocks, so `cdz watch` ran the command TWICE on startup. Pin a SINGLE initial run: a
    // scaffold with an unused private def emits exactly one CDZ0306 "unused" line per check run — so
    // before any edit, that diagnostic must appear EXACTLY ONCE.
    let (root, proj) = scaffold("once");
    // Add an unused private def so `check` emits a stable, countable diagnostic each run.
    std::fs::write(
        proj.join("main.cdz"),
        "def helper() -> Int64 = 0\ndef main() -> Int64 = 0\n\nexport { main }\n",
    )
    .expect("seed source with an unused def");
    let (mut child, cap) = spawn_watch(&proj, &[], "once");
    assert!(
        wait_for(&cap, "watching", Duration::from_secs(20)),
        "startup banner: {}",
        read_cap(&cap)
    );
    // Wait for the initial run's diagnostic, then a little longer to catch a spurious second run.
    assert!(
        wait_for(&cap, "CDZ0306", Duration::from_secs(20)),
        "the initial check runs (emits the unused-def diagnostic): {}",
        read_cap(&cap)
    );
    std::thread::sleep(Duration::from_secs(2));
    let initial_runs = count(&cap, "CDZ0306");
    let reruns_before_edit = count(&cap, "change detected");

    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_file(&cap);
    // The #506 regression is a DUPLICATE startup `rerun()` block: it ran the command twice with NEITHER
    // run logging "change detected" (2 CDZ0306, 0 change-detected). Assert the invariant that catches
    // that precisely: exactly ONE startup run is not attributable to a detected change. On macOS,
    // FSEvents delivers a spurious startup event for the pre-existing source (Linux inotify does not),
    // which the watch loop handles via its normal change path — logging "change detected" AND emitting a
    // second CDZ0306 (2 CDZ0306, 1 change-detected). Counting raw CDZ0306 == 1 wrongly failed there; the
    // subtraction tolerates that platform event while still failing the double-`rerun()` bug (2 - 0 = 2).
    // (Draining the startup FSEvents event in the watcher itself is filed to v-cdz-tooling separately.)
    // Each change-detected re-run also runs check → one CDZ0306 each, so initial_runs >= reruns (no
    // underflow) and a correct startup yields initial_runs - reruns_before_edit == 1.
    let startup_runs_not_from_change = initial_runs - reruns_before_edit;
    assert_eq!(
        startup_runs_not_from_change, 1,
        "exactly one startup run not caused by a detected change (guards the #506 double-`rerun()` \
         regression; tolerates a macOS FSEvents spurious startup event): {initial_runs} CDZ0306, \
         {reruns_before_edit} change-detected"
    );
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
    std::thread::sleep(Duration::from_millis(800));
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
fn watch_does_not_drop_edits_made_across_successive_saves() {
    // REGRESSION (Copilot PR #502): the post-run drain used to DISCARD every event that arrived during a
    // run as "already reflected" — but an edit made mid-run is NOT reflected in the just-finished run, so
    // that save was silently lost until some later event. Now a mid-run source edit flags one more
    // re-run. This is hard to time deterministically (check is fast), so exercise the same guarantee
    // observably: make SEVERAL successive edits and assert the watch keeps re-running for each wave (the
    // final edit's result must eventually be reflected — not stuck on a dropped save). Each edit toggles
    // between clean and a distinct error so the LAST state is unambiguous in the output.
    let (root, proj) = scaffold("nodrop");
    let (mut child, cap) = spawn_watch(&proj, &[], "nodrop");
    assert!(
        wait_for(&cap, "watching", Duration::from_secs(20)),
        "startup banner: {}",
        read_cap(&cap)
    );

    // A first edit (clean) → at least one re-run fires.
    std::thread::sleep(Duration::from_millis(800));
    std::fs::write(
        proj.join("main.cdz"),
        "def main() -> Int64 = 1\n\nexport { main }\n",
    )
    .expect("edit 1");
    assert!(
        wait_for(&cap, "change detected", Duration::from_secs(20)),
        "the first save re-runs: {}",
        read_cap(&cap)
    );

    // A later edit introducing a UNIQUELY-named unbound reference — its diagnostic must eventually show
    // up, proving the watch is still live and reflected the LATEST source state (not stuck on a drop).
    std::thread::sleep(Duration::from_millis(800));
    std::fs::write(
        proj.join("main.cdz"),
        "def main() -> Int64 = zzz_unique_marker\n\nexport { main }\n",
    )
    .expect("edit 2");
    let reflected_latest = wait_for(&cap, "zzz_unique_marker", Duration::from_secs(20));
    let captured = read_cap(&cap);

    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_file(&cap);
    assert!(
        reflected_latest,
        "the watch reflects the LATEST edit (no dropped save): {captured}"
    );
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
    // Wait for the initial build to write its component (detect the FILE, not a stdout substring — the
    // artifact's existence is the stable signal), then some more — a self-trigger loop would keep
    // emitting "change detected".
    assert!(
        wait_for_path(&proj.join("main.wasm"), Duration::from_secs(20)),
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

#[test]
fn watch_run_executes_the_entry_on_the_initial_pass() {
    // `--exec run` builds the entry and RUNS it (the live "run on save" loop). The scaffold's `main()`
    // returns 0, so the initial run prints `0` — and, like build, the `.wasm` it writes must NOT
    // self-trigger (the positive source filter covers the run path too, since it shares the build's
    // artifact writes). Assert the banner names `run`, the value prints, and no self-trigger fires.
    let (root, proj) = scaffold("run");
    let (mut child, cap) = spawn_watch(&proj, &["--exec", "run"], "run");
    assert!(
        wait_for(&cap, "watching", Duration::from_secs(20)),
        "startup banner: {}",
        read_cap(&cap)
    );
    assert!(
        read_cap(&cap).contains("cdz run"),
        "the banner names the run command: {}",
        read_cap(&cap)
    );
    // The initial build+run prints the scalar entry's value (`0`). Build+run is slower than check, so
    // allow a generous window.
    let ran = wait_for(&cap, "\n0", Duration::from_secs(30));
    std::thread::sleep(Duration::from_secs(2));
    let self_triggers = count(&cap, "change detected");

    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_file(&cap);
    assert!(ran, "the initial `run` pass prints the entry's value");
    assert_eq!(
        self_triggers, 0,
        "the run's own build artifacts must NOT re-trigger the watch"
    );
}

#[test]
fn watch_reruns_when_a_path_dependency_source_changes() {
    // The multi-project edit loop: `cdz watch` on a project with `def deps = ["../mathlib"]` watches the
    // DEP's directory too, so editing the dep's source re-triggers the run — not just editing the consumer.
    // (Before this, only the consumer's dir was watched, so a dep edit was silently missed.) Uses the
    // default `--exec check` — deterministic + fast; the WATCH TRIGGER (what's under test) fires the same
    // for any exec, and check doesn't depend on the run succeeding (so no timing flakiness).
    let root = std::env::temp_dir().join(format!("cdz-watch-dep-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("mathlib")).unwrap();
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
    std::fs::create_dir_all(root.join("app")).unwrap();
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

    let (mut child, cap) = spawn_watch(&root.join("app"), &[], "dep");
    assert!(
        wait_for(&cap, "watching", Duration::from_secs(20)),
        "startup banner: {}",
        read_cap(&cap)
    );

    // Edit the DEPENDENCY's source (not the consumer's) — the watch must still re-run.
    std::thread::sleep(Duration::from_millis(800));
    std::fs::write(
        root.join("mathlib/lib.sexp"),
        "(do (def (add (: x Int64) (: y Int64)) (+ (+ x y) 100)) (export add))",
    )
    .expect("edit the dependency source");
    let rer = wait_for(&cap, "change detected", Duration::from_secs(30));

    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_file(&cap);
    assert!(rer, "editing a path-dependency's source re-runs the watch");
}

#[test]
fn watch_run_passes_call_and_arg_to_an_arg_taking_entry() {
    // FOLLOW-UP: `cdz watch --exec run` used to pass NO call/arg to the entry, so an arg-TAKING `main`
    // errored ("argument count mismatch") on every run. Now `--call`/`--arg` thread through to the run
    // (like `cdz run --call/--arg`). Watch a project whose `main(x) = x + 1` under `--exec run --call
    // main --arg 7`; the initial pass must print `8` (not an arity error), and a source edit changing the
    // body to `x + 100` must re-run and print `107` — proving the arg is re-applied on every re-run.
    let (root, proj) = scaffold("runarg");
    std::fs::write(
        proj.join("main.cdz"),
        "def main(x: Int64) -> Int64 = x + 1\n\nexport { main }\n",
    )
    .expect("seed an arg-taking entry");
    let (mut child, cap) = spawn_watch(
        &proj,
        &["--exec", "run", "--call", "main", "--arg", "7"],
        "runarg",
    );
    assert!(
        wait_for(&cap, "watching", Duration::from_secs(20)),
        "startup banner: {}",
        read_cap(&cap)
    );
    // The initial run applies main(7) = 8 — the arg reached the entry (no arity error).
    let printed_8 = wait_for(&cap, "\n8", Duration::from_secs(30));
    let after_initial = read_cap(&cap);

    // Edit the body → the re-run must apply the SAME arg to the new code: main(7) = 7 + 100 = 107.
    std::thread::sleep(Duration::from_millis(800));
    std::fs::write(
        proj.join("main.cdz"),
        "def main(x: Int64) -> Int64 = x + 100\n\nexport { main }\n",
    )
    .expect("rewrite the entry body");
    let printed_107 = wait_for(&cap, "\n107", Duration::from_secs(30));
    let captured = read_cap(&cap);

    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_file(&cap);
    assert!(
        printed_8,
        "the initial run passes --arg to the entry: main(7) = 8 (no arity error): {after_initial}"
    );
    assert!(
        printed_107,
        "the re-run re-applies --arg to the edited entry: main(7) = 107: {captured}"
    );
}

#[test]
fn watch_test_passes_filter_to_the_rerun() {
    // FOLLOW-UP (symmetric with the run --call/--arg passthrough): `cdz watch --exec test` used to
    // hardcode filter/tag/trials/seed, so you couldn't watch ONE test. Now `--filter` (and `--tag`/
    // `--trials`/`--seed`) thread through to each re-run (like `cdz test --filter`). Watch a project with
    // two `@test`s (`alpha_ok`, `beta_ok`) under `--exec test --filter alpha`: a re-run must report
    // `PASS alpha_ok` but NOT run `beta_ok` — proving the filter reached the test run, not the whole suite.
    let root = std::env::temp_dir().join(format!("cdz-watch-tfilter-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let proj = root.join("app");
    std::fs::create_dir_all(&proj).unwrap();
    std::fs::write(
        proj.join("Project.cdz"),
        "def name = \"app\"\ndef entry = \"main.cdz\"\ndef tests = [\"main.cdz\"]\n",
    )
    .unwrap();
    std::fs::write(
        proj.join("main.cdz"),
        "def main() -> Int64 = 0\n\
         @test def alpha_ok() = if 1 == 1 then unit else trap(\"a\")\n\
         @test def beta_ok() = if 2 == 2 then unit else trap(\"b\")\n\n\
         export { main }\n",
    )
    .unwrap();

    let (mut child, cap) = spawn_watch(&proj, &["--exec", "test", "--filter", "alpha"], "tfilter");
    assert!(
        wait_for(&cap, "watching", Duration::from_secs(20)),
        "startup banner: {}",
        read_cap(&cap)
    );
    // The initial test run must report the filtered test — proving --filter reached the run.
    let saw_alpha = wait_for(&cap, "PASS alpha_ok", Duration::from_secs(30));
    std::thread::sleep(Duration::from_secs(2)); // let any second (unfiltered) test surface if the filter were dropped
    let ran_beta = count(&cap, "beta_ok");
    let captured = read_cap(&cap);

    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_file(&cap);
    assert!(
        saw_alpha,
        "the --filter reaches the test re-run (PASS alpha_ok): {captured}"
    );
    assert_eq!(
        ran_beta, 0,
        "the filtered-out test must NOT run (proving --filter threaded through, not the whole suite): {captured}"
    );
}

#[test]
fn watch_clear_emits_a_clear_screen_before_each_run() {
    // `--clear` (like `cargo watch -c`) clears the terminal before EACH run so output starts fresh. The
    // clear is the ANSI erase-display sequence `\x1b[2J`; our capture file records it verbatim (harmless
    // when stdout isn't a tty). The invariant `--clear` guarantees is PER-RUN: every run (the initial one
    // and every re-run) emits exactly one clear.
    //
    // We assert this as a DELTA, not an absolute startup count (Copilot PR #521): on macOS, FSEvents
    // delivers a spurious startup filesystem event that the watch loop handles via its normal change path,
    // firing a SECOND startup run (and thus a second clear) that Linux inotify does not — so a fixed
    // `assert_eq!(startup_clears, 1)` FLAKES there even though `--clear` works. The sibling
    // `watch_runs_the_command_exactly_once_on_startup` test documents the same platform event. So: record
    // whatever the startup clear count settles at (≥1 — the feature IS on), then assert a source edit
    // INCREASES it (the re-run clears again) — a per-run delta that holds on every platform.
    let (root, proj) = scaffold("clear");
    let (mut child, cap) = spawn_watch(&proj, &["--clear"], "clear");
    assert!(
        wait_for(&cap, "watching", Duration::from_secs(20)),
        "startup banner: {}",
        read_cap(&cap)
    );
    // The initial run cleared (at least once — proves `--clear` is active; without it the capture would
    // contain zero `\x1b[2J`, as the other watch tests' captures do).
    assert!(
        wait_for(&cap, "\x1b[2J", Duration::from_secs(20)),
        "the initial run clears the screen: {:?}",
        read_cap(&cap)
    );
    // Let the startup clear count SETTLE before taking the baseline — a poll-until-stable, not a blanket
    // sleep: on Linux (no spurious event) it returns in ~one quiet window; on macOS it absorbs the extra
    // startup FS event's clear up to the cap. Either way the baseline is the settled count (Copilot #523:
    // the earlier fixed 2s sleep added that latency to every run, even where the event never occurs).
    let startup_clears = settle_count(&cap, "\x1b[2J", Duration::from_secs(2));
    assert!(
        startup_clears >= 1,
        "the startup run clears at least once: {:?}",
        read_cap(&cap)
    );

    // A source edit → the re-run clears AGAIN: the count must strictly increase (the per-run guarantee),
    // regardless of whether startup fired one clear (Linux) or two (a macOS spurious startup event).
    std::thread::sleep(Duration::from_millis(800));
    std::fs::write(
        proj.join("main.cdz"),
        "def main() -> Int64 = 1\n\nexport { main }\n",
    )
    .expect("edit source");
    let cleared_again =
        wait_for_count(&cap, "\x1b[2J", startup_clears + 1, Duration::from_secs(20));
    let captured = read_cap(&cap);

    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_file(&cap);
    assert!(
        cleared_again,
        "the re-run clears the screen again (clears increased past the startup count {startup_clears}): \
         {captured:?}"
    );
}
