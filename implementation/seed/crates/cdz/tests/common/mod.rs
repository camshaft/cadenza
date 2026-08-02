//! Shared helpers for the `cdz` CLI integration tests. Lives in `tests/common/` (a SUBDIR, not
//! `tests/common.rs`) so Cargo does NOT compile it as its own test binary — each test file pulls it in with
//! `mod common;` and calls the shared fns, so a change is single-sourced. Anything unused by a given test
//! file is `#[allow(dead_code)]` (each file only uses part of the surface, and dead-code is per-crate).

#![allow(dead_code)]

use std::io::Write;
use std::process::ChildStdin;

/// Write `bytes` to a child `cdz`'s stdin, TOLERATING a `BrokenPipe`.
///
/// `cdz` may report an error and EXIT before it finishes reading stdin (e.g. a rejected arg like a missing
/// `--from` or a bad pattern), closing the read end of the pipe. Our `write_all` then races that close and,
/// on a slower runner, returns `BrokenPipe` — benign here, since these tests assert on `cdz`'s exit status +
/// stderr (checked after `wait`), not that every byte was consumed. So swallow ONLY `BrokenPipe` and panic
/// on any other write error. Single-sourced so the tolerated kind can't drift across the CLI test files.
pub fn write_stdin_tolerating_broken_pipe(stdin: ChildStdin, bytes: &[u8]) {
    let mut stdin = stdin;
    if let Err(e) = stdin.write_all(bytes) {
        assert_eq!(
            e.kind(),
            std::io::ErrorKind::BrokenPipe,
            "unexpected stdin write error (not the benign BrokenPipe race): {e}"
        );
    }
}
