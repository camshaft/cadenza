//! Compile an assembled Rust driver with `rustc` and RUN it, capturing the outcome — the impure half of
//! the rust exec runner (the driver source is built purely by [`crate::driver`]). Ported from xtask's
//! `run_program_rust` compile+run tail. `rustc` links the pre-built runtime rlibs (`cdz_rt` for the async
//! `CdzEnv`, `cdz_num` for `cdz_num::Big`, `cadenza_ast` + `num_bigint` for the native value codec) that
//! the caller (the nix per-case rust exec layer) provides — passed as `-L dependency=<dir> --extern
//! <crate>=<rlib>`. Grading the [`Outcome`] against a `test-run.ast` is a later increment.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// What a compiled-and-run driver produced.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// The driver ran and printed a value (its canonical text) — plus the observed host-call op sequence
    /// (`host-call\t<op>` stderr lines, in call order; empty for a non-host program).
    Value(String, Vec<String>),
    /// The driver TRAPPED (a Cadenza trap = a Rust panic → non-zero exit); the trap reason.
    Trap(String),
    /// The emitted `.rs` did not compile (the miscompile class this catches), or the binary would not
    /// launch — the reason.
    BadArtifact(String),
}

/// The pre-built runtime rlib directories to link, each optional (a scalar/const program needs none). Each
/// is the DIRECTORY holding the rlib (`<dir>/lib<crate>.rlib`); `cadenza_ast`'s transitive deps live in
/// `<dir>/deps`.
#[derive(Debug, Default, Clone)]
pub struct RlibDirs {
    pub cdz_rt: Option<PathBuf>,
    pub cdz_num: Option<PathBuf>,
    pub cadenza_ast: Option<PathBuf>,
}

/// The per-run wall-clock cap (default 120s; `CDZ_RUN_TIMEOUT_SECS=<n>` overrides, `0` is ignored) — an
/// infinite-loop emitted program TRAPS(hang) at the deadline instead of hanging the caller forever.
pub fn run_timeout() -> Duration {
    let secs = std::env::var("CDZ_RUN_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|&s| s > 0)
        .unwrap_or(120);
    Duration::from_secs(secs)
}

/// The first line of a byte stream (lossy UTF-8), trimmed of the trailing newline.
pub fn first_line(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .lines()
        .next()
        .unwrap_or("")
        .to_string()
}

/// The observed host-call ops from a run's stderr — each `host-call\t<op>` line's op, in call order.
pub fn observed_host_calls(stderr: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(stderr)
        .lines()
        .filter_map(|l| l.strip_prefix("host-call\t").map(str::to_string))
        .collect()
}

/// The trap REASON from a Rust process's panic stderr. Rust prints `thread '…' panicked at <loc>:` then the
/// panic MESSAGE on the NEXT line (`panic!("unreachable")` → `unreachable`); return that message line (the
/// header carries no reason), falling back to the first line for a non-panic non-zero exit.
pub fn rust_panic_message(bytes: &[u8]) -> String {
    let s = String::from_utf8_lossy(bytes);
    let mut lines = s.lines();
    while let Some(line) = lines.next() {
        if line.contains("panicked at")
            && let Some(msg) = lines.next()
        {
            return msg.trim().to_string();
        }
    }
    first_line(bytes)
}

/// Wait for `child`, draining its piped stdout/stderr on threads (a hung child may still have emitted
/// partial output, and an undrained full pipe would block it), and KILL it at `timeout`. `Ok(None)` = the
/// deadline passed (a hang). Ported from xtask's `wait_with_timeout`.
fn wait_with_timeout(
    mut child: std::process::Child,
    timeout: Duration,
) -> std::io::Result<Option<std::process::Output>> {
    use std::io::Read;
    let mut out_pipe = child.stdout.take();
    let mut err_pipe = child.stderr.take();
    let out_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(p) = out_pipe.as_mut() {
            let _ = p.read_to_end(&mut buf);
        }
        buf
    });
    let err_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(p) = err_pipe.as_mut() {
            let _ = p.read_to_end(&mut buf);
        }
        buf
    });
    let deadline = std::time::Instant::now() + timeout;
    let status = loop {
        match child.try_wait()? {
            Some(status) => break Some(status),
            None => {
                if std::time::Instant::now() >= deadline {
                    break None;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    };
    match status {
        Some(status) => {
            let stdout = out_thread.join().unwrap_or_default();
            let stderr = err_thread.join().unwrap_or_default();
            Ok(Some(std::process::Output {
                status,
                stdout,
                stderr,
            }))
        }
        None => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = out_thread.join();
            let _ = err_thread.join();
            Ok(None)
        }
    }
}

/// Compile `driver_src` with `rustc` (linking `rlibs`) into `workdir/prog`, run it, and capture the
/// [`Outcome`]. Compiled at `-O0` (verdict-equivalent to `-O2` for a correct program, and the gate tests
/// the BACKEND's emit not rustc's optimizer — cutting peak rustc memory). `async_mode` links `cdz_rt`.
/// A compile failure or launch failure → `BadArtifact`; a non-zero run → `Trap` (the panic reason); a clean
/// run → `Value(stdout, observed host-calls)`; a deadline overrun → `Trap("timeout (hang)")`.
pub fn compile_and_run(
    driver_src: &str,
    workdir: &Path,
    rlibs: &RlibDirs,
    async_mode: bool,
) -> Outcome {
    let src = workdir.join("prog.rs");
    let bin = workdir.join("prog");
    if std::fs::write(&src, driver_src).is_err() {
        return Outcome::BadArtifact("could not write emitted Rust to a temp file".to_string());
    }
    let mut cmd = Command::new("rustc");
    cmd.args(["--edition", "2021"])
        .arg(&src)
        .arg("-o")
        .arg(&bin);
    if async_mode && let Some(dir) = rlibs.cdz_rt.as_deref() {
        cmd.arg("-L")
            .arg(format!("dependency={}", dir.display()))
            .arg("--extern")
            .arg(format!("cdz_rt={}", dir.join("libcdz_rt.rlib").display()));
    }
    if let Some(dir) = rlibs.cdz_num.as_deref() {
        cmd.arg("-L")
            .arg(format!("dependency={}", dir.display()))
            .arg("--extern")
            .arg(format!("cdz_num={}", dir.join("libcdz_num.rlib").display()));
    }
    if let Some(dir) = rlibs.cadenza_ast.as_deref() {
        cmd.arg("-L")
            .arg(format!("dependency={}", dir.display()))
            .arg("-L")
            .arg(format!("dependency={}", dir.join("deps").display()))
            .arg("--extern")
            .arg(format!(
                "cadenza_ast={}",
                dir.join("libcadenza_ast.rlib").display()
            ));
        // The native value-encode emit constructs `num_bigint::BigInt`, so `num_bigint` must be a NAMABLE
        // extern (hash-named in `<dir>/deps`). Glob for it and `--extern num_bigint=` it (harmless when
        // unreferenced). `num_bigint` is a shared dep, so it is always built.
        if let Ok(entries) = std::fs::read_dir(dir.join("deps"))
            && let Some(rlib) = entries.filter_map(|e| e.ok()).map(|e| e.path()).find(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("libnum_bigint-") && n.ends_with(".rlib"))
            })
        {
            cmd.arg("--extern")
                .arg(format!("num_bigint={}", rlib.display()));
        }
        // Same rationale for `unicode_normalization` — the native `Core::NfcNormalize` emit (String.concat /
        // from-bytes NFC canonicalization, FINDING #23 rust parity) calls `unicode_normalization::…::nfc(…)`, so
        // it must be a NAMABLE extern. It is a `std`-feature dep of `cadenza_ast` (like `num_bigint`), hash-named
        // in `<dir>/deps`; glob for it and `--extern unicode_normalization=` it (harmless when unreferenced).
        if let Ok(entries) = std::fs::read_dir(dir.join("deps"))
            && let Some(rlib) = entries.filter_map(|e| e.ok()).map(|e| e.path()).find(|p| {
                p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                    n.starts_with("libunicode_normalization-") && n.ends_with(".rlib")
                })
            })
        {
            cmd.arg("--extern")
                .arg(format!("unicode_normalization={}", rlib.display()));
        }
    }
    let compiled = match cmd.output() {
        Ok(o) => o,
        Err(e) => return Outcome::BadArtifact(format!("rustc failed to launch: {e}")),
    };
    if !compiled.status.success() {
        return Outcome::BadArtifact(first_line(&compiled.stderr));
    }
    // Run it, retrying a few times on a transient launch error (a freshly-linked binary can briefly report
    // "text file busy" while the writer handle closes — a race, not a defect; a genuinely unrunnable binary
    // fails every attempt).
    let mut last_err = None;
    let mut got = None;
    for attempt in 0..8 {
        match Command::new(&bin)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
        {
            Ok(child) => match wait_with_timeout(child, run_timeout()) {
                Ok(Some(o)) => {
                    got = Some(o);
                    break;
                }
                Ok(None) => return Outcome::Trap("timeout (hang)".to_string()),
                Err(e) => {
                    last_err = Some(e);
                    std::thread::sleep(Duration::from_millis(2 * (attempt + 1)));
                }
            },
            Err(e) => {
                last_err = Some(e);
                std::thread::sleep(Duration::from_millis(2 * (attempt + 1)));
            }
        }
    }
    let run = match got {
        Some(o) => o,
        None => {
            return Outcome::BadArtifact(format!(
                "compiled prog failed to launch: {}",
                last_err.map(|e| e.to_string()).unwrap_or_default()
            ));
        }
    };
    if run.status.success() {
        // Route the captured stdout through the value-doc marker interpreter: a `CDZDOC:<hex>` marker (the
        // flag-gated value-doc path) decodes to the canonical surface via `render_binary`; any other output
        // (the default string render) passes through byte-identical (a string render never starts with the
        // marker). A corrupt marker → `BadArtifact` (never a silent mis-render).
        let raw = String::from_utf8_lossy(&run.stdout).trim().to_string();
        match crate::value_doc::interpret_run_stdout(&raw) {
            Ok(value) => Outcome::Value(value, observed_host_calls(&run.stderr)),
            Err(e) => Outcome::BadArtifact(format!("value-doc marker decode failed: {e}")),
        }
    } else {
        Outcome::Trap(rust_panic_message(&run.stderr))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panic_message_takes_the_line_after_the_header() {
        let s = b"thread 'main' panicked at src/prog.rs:1:1:\nunreachable\nnote: run with ...\n";
        assert_eq!(rust_panic_message(s), "unreachable");
        // Non-panic non-zero exit → first line.
        assert_eq!(
            rust_panic_message(b"some other error\n"),
            "some other error"
        );
    }

    #[test]
    fn observed_host_calls_parses_the_op_lines() {
        let s = b"host-call\tio.log\nhost-arg\tio.log\ttag\nhost-call\task.ask\n";
        assert_eq!(
            observed_host_calls(s),
            vec!["io.log".to_string(), "ask.ask".to_string()]
        );
    }

    // INTEGRATION: shells the ambient `rustc` (present in the dev/nix toolchain). A no-rlib scalar driver
    // (built by `crate::driver`) compiles + runs + prints its value; a diverging one traps.
    #[test]
    fn compiles_and_runs_a_scalar_driver() {
        let dir = std::env::temp_dir().join(format!("cdz-rr-run-{}-scalar", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let driver = crate::driver::build_driver_source(
            "pub fn main() -> i64 { 42 }",
            "main",
            &[],
            &[],
            &[],
            false,
        );
        match compile_and_run(&driver, &dir, &RlibDirs::default(), false) {
            Outcome::Value(v, _) => assert_eq!(v, "42"),
            other => panic!("expected Value(42), got {other:?}"),
        }
    }

    #[test]
    fn a_panicking_driver_is_a_trap() {
        let dir = std::env::temp_dir().join(format!("cdz-rr-run-{}-trap", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // A diverging export (`-> !`) — the driver just calls it and it panics.
        let m = "// cdz-return[boom]: !\npub fn boom() -> ! { panic!(\"unreachable\") }";
        let driver = crate::driver::build_driver_source(m, "boom", &[], &[], &[], false);
        match compile_and_run(&driver, &dir, &RlibDirs::default(), false) {
            Outcome::Trap(reason) => {
                assert!(reason.contains("unreachable"), "trap reason: {reason}")
            }
            other => panic!("expected Trap, got {other:?}"),
        }
    }
}
