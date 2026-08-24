//! `cargo xtask world-artifact` — a thin shell-out to the `cdz-world-artifact` utility CLI, which parses
//! `cdz-platform/wit/world.wit` and emits each reducer world's `KIND_WIT_WORLD` binary-AST artifact.
//!
//! The WIT→artifact logic lives in the isolated `cdz-world-artifact` crate (its `lib` + `main`), NOT here:
//! xtask SHELLS OUT to the built binary rather than linking it (operator directive 2026-08-24 — decompose
//! xtask into small single-purpose utility programs, same pattern as `cdz-component-rewrite`). This keeps
//! `cargo xtask world-artifact` as a convenience entry point for local dev while the real work — and its
//! own tests, over a self-contained WIT fixture — sits behind a clean crate boundary.

use crate::Paths;
use std::path::PathBuf;

/// Build the `cdz-world-artifact` utility, then shell out to it to write `<world>.bin` per reducer world.
/// `out` defaults to `<repo>/target/wit-worlds`; `wit` to `crates/cdz-platform/wit/world.wit`; `world`, when
/// given, restricts emission to that one world (else the utility emits every world the document declares).
/// Exits the process non-zero (after printing) on any build/parse/write error — a build step, not a lib call.
pub fn run(paths: &Paths, out: Option<PathBuf>, wit: Option<PathBuf>, world: Option<String>) {
    let wit_path = wit.unwrap_or_else(|| paths.seed.join("crates/cdz-platform/wit/world.wit"));
    let out_dir = out.unwrap_or_else(|| paths.repo.join("target/wit-worlds"));

    // Build the utility CLI (idempotent: cargo no-ops when warm). SHELL OUT to it, never link it.
    let status = std::process::Command::new("cargo")
        .args([
            "build",
            "--release",
            "-p",
            "cdz-world-artifact",
            "--bin",
            "cdz-world-artifact",
        ])
        .current_dir(&paths.repo)
        .status();
    match status {
        Ok(s) if s.success() => {}
        other => {
            eprintln!(
                "xtask world-artifact: failed to build cdz-world-artifact ({other:?}) — needed to emit the world artifacts"
            );
            std::process::exit(1);
        }
    }

    let bin = paths.repo.join("target/release/cdz-world-artifact");
    let mut cmd = std::process::Command::new(&bin);
    cmd.arg(&wit_path).arg(&out_dir);
    if let Some(w) = world {
        cmd.arg(w);
    }
    match cmd.status() {
        Ok(s) if s.success() => {}
        other => {
            eprintln!(
                "xtask world-artifact: cdz-world-artifact failed ({other:?}) emitting from {}",
                wit_path.display()
            );
            std::process::exit(1);
        }
    }
}
