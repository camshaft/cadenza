//! Delegated compilation — reach the compiler by SPAWNING the standalone `cdz-compile` process
//! instead of linking `rcdzc` in-process. Gated behind the `delegate-compile` cargo feature
//! (`design/DESIGN-cdz-delegate-compile.md`): a compiler-only change then need not rebuild `cdz`, so
//! nix turns the feature on for the packaged toolchain and `cdz` + the compiler cache independently.
//!
//! Why this is behavior-identical: `cdz-compile` IS `rcdzc::cli::run` under its own name (a thin shim
//! over the same `CompileArgs` + `run` → `run_prepared`). So delegating is not a reimplementation — we
//! hand `cdz-compile` the SAME named artifacts and flags the in-process path would, and it executes the
//! exact host-boundary compile+report+write. In particular the diagnostics reporter runs THERE, over
//! the same `spans` artifacts we pass through, so a source-file compile's located `path:line:col`
//! errors come out byte-identical to the in-process path. Child stdio is inherited, so a `-o -` stdout
//! pipe and stderr diagnostics flow straight to the caller; the child's exit status is forwarded.
//!
//! This module is compiled ONLY under the feature; the in-process path (`compiler_cli::run` /
//! `run_prepared`) stays the default, so a dev/interactive `cdz` never pays the subprocess cost.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use rcdzc::{Artifact, OptLevel, Target};

/// Locate the `cdz-compile` binary to delegate to. Resolution order, most-specific first:
///  1. `$CDZ_COMPILE_BIN` — an explicit path (how the nix package injects the content-addressed
///     compiler bin, so the two derivations stay independently cached with no `$PATH`/CWD ambiguity).
///  2. beside THIS `cdz` — `current_exe().parent()/cdz-compile[.exe]`, the co-built location (the same
///     convention the `cdz smith`/`cdz cad` passthroughs use via [`crate::locate_sibling_bin`]).
///  3. bare `cdz-compile` — resolved on `$PATH` by `Command::new` (a system install).
fn locate() -> PathBuf {
    if let Some(p) = std::env::var_os("CDZ_COMPILE_BIN") {
        return PathBuf::from(p);
    }
    crate::locate_sibling_bin("cdz-compile")
        .unwrap_or_else(|| PathBuf::from(crate::bin_name("cdz-compile")))
}

/// The `--target` value string for a backend [`Target`] — matched to `rcdzc::cli`'s `TargetArg` clap
/// spelling (kebab-case), so the delegated flag parses identically to the in-process one.
fn target_cli(t: Target) -> &'static str {
    match t {
        Target::Wasm => "wasm",
        Target::WasmDebug => "wasm-debug",
        Target::Dwarf => "dwarf",
        Target::Rust => "rust",
        Target::RustAsync => "rust-async",
    }
}

/// The `--opt-level` value string for an [`OptLevel`] — matched to `rcdzc::cli`'s `OptLevelArg` spelling.
fn opt_cli(o: OptLevel) -> &'static str {
    match o {
        OptLevel::O0 => "o0",
        OptLevel::O1 => "o1",
        OptLevel::O2 => "o2",
        OptLevel::O3 => "o3",
    }
}

/// Spawn `cdz-compile` with `input_specs` (each a `path` / `name=path` / `kind:name=path` the compiler
/// CLI parses), the resolved `targets` (empty ⇒ let `cdz-compile` apply its own `[Wasm]` default),
/// optional `-o out`, and `--opt-level`. Optional `--entry`/`--component-name` flags for the
/// artifacts-in path (the source path bakes these into the artifact stream instead). Inherits stdio and
/// forwards the child's exit status; a spawn failure prints an actionable, `prog`-labelled error.
#[allow(clippy::too_many_arguments)]
fn spawn(
    input_specs: &[String],
    targets: &[Target],
    out: Option<&Path>,
    entry: Option<&str>,
    component_name: Option<&str>,
    opt_level: OptLevel,
    prog: &str,
) -> ExitCode {
    let program = locate();
    let mut cmd = Command::new(&program);
    cmd.args(input_specs);
    for t in targets {
        cmd.arg("--target").arg(target_cli(*t));
    }
    if let Some(o) = out {
        cmd.arg("-o").arg(o);
    }
    if let Some(e) = entry {
        cmd.arg("--entry").arg(e);
    }
    if let Some(c) = component_name {
        cmd.arg("--component-name").arg(c);
    }
    cmd.arg("--opt-level").arg(opt_cli(opt_level));

    match cmd.status() {
        Ok(status) if status.success() => ExitCode::SUCCESS,
        Ok(status) => crate::exit_code_from_child(status.code()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!(
                "{prog}: cdz-compile not found (looked at $CDZ_COMPILE_BIN, beside `cdz`, then \
                 $PATH) — the `delegate-compile` feature spawns the compiler instead of linking it; \
                 build it with `cargo build -p rcdzc --bin cdz-compile` and re-run"
            );
            ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!(
                "{prog}: could not run cdz-compile ({}): {e}",
                program.display()
            );
            ExitCode::FAILURE
        }
    }
}

/// Delegate a PURE ARTIFACTS-IN `cdz compile` (no source-file input): forward the raw input specs +
/// `--entry`/`--component-name` flags + targets/`-o`/`--opt-level` verbatim to `cdz-compile`. The
/// specs already name on-disk artifacts (or `-` for stdin, whose stream the inherited stdin carries),
/// so no temp files are needed — this is the same `run` the standalone `rcdzc`/`cdz-compile` bin does.
pub fn delegate_args(args: &rcdzc::cli::CompileArgs, prog: &str) -> ExitCode {
    spawn(
        args.input_specs(),
        &args.targets(),
        args.out_path().as_deref(),
        args.entry(),
        args.component_name(),
        args.opt_level(),
        prog,
    )
}

/// Delegate a compile of ALREADY-PREPARED input artifacts (the source-file path, where `cdz` has
/// parsed each source in-process into `ast` + `spans` artifacts and appended any `entry`/
/// `component-name` artifacts). Each artifact is materialized to a temp file and handed to
/// `cdz-compile` as a `kind:name=path` spec — including the `entry`/`component-name` artifacts, which
/// `cdz-compile` reads from the artifact stream exactly as if `--entry`/`--component-name` were passed
/// (so no flags are needed here). The temp directory is removed on every exit path.
pub fn delegate_from_artifacts(
    inputs: &[Artifact],
    targets: &[Target],
    out: Option<&Path>,
    opt_level: OptLevel,
    prog: &str,
) -> ExitCode {
    // A per-process unique scratch dir (pid + a monotonic counter, so concurrent `cdz` compiles in the
    // same process don't collide — no wall-clock/rng needed).
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "cdz-delegate-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("{prog}: cannot create temp dir {}: {e}", dir.display());
        return ExitCode::FAILURE;
    }
    let _guard = crate::RemoveOnDrop::dir(dir.clone());

    let mut specs: Vec<String> = Vec::with_capacity(inputs.len());
    for (i, a) in inputs.iter().enumerate() {
        // A stable, separator-free temp file name per artifact — the `kind:name=` spec carries the
        // artifact's identity, so the file stem need only be unique (the index).
        let path = dir.join(i.to_string());
        if let Err(e) = std::fs::write(&path, &a.bytes) {
            eprintln!("{prog}: cannot write temp artifact {}: {e}", path.display());
            return ExitCode::FAILURE;
        }
        specs.push(format!("{}:{}={}", a.kind, a.name, path.display()));
    }

    spawn(&specs, targets, out, None, None, opt_level, prog)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_strings_match_the_cli_spelling() {
        // These MUST equal `rcdzc::cli::TargetArg`'s clap value names, else the delegated `--target`
        // flag fails to parse. Pin every variant so a new backend can't silently drift the mapping.
        assert_eq!(target_cli(Target::Wasm), "wasm");
        assert_eq!(target_cli(Target::WasmDebug), "wasm-debug");
        assert_eq!(target_cli(Target::Dwarf), "dwarf");
        assert_eq!(target_cli(Target::Rust), "rust");
        assert_eq!(target_cli(Target::RustAsync), "rust-async");
    }

    #[test]
    fn opt_level_strings_match_the_cli_spelling() {
        assert_eq!(opt_cli(OptLevel::O0), "o0");
        assert_eq!(opt_cli(OptLevel::O1), "o1");
        assert_eq!(opt_cli(OptLevel::O2), "o2");
        assert_eq!(opt_cli(OptLevel::O3), "o3");
    }

    #[test]
    fn cdz_compile_bin_env_override_wins() {
        // $CDZ_COMPILE_BIN takes precedence over the sibling/$PATH resolution — the nix-injection path.
        // SAFETY: single-threaded test; the var is set and read within this test only.
        unsafe { std::env::set_var("CDZ_COMPILE_BIN", "/opt/custom/cdz-compile") };
        assert_eq!(locate(), PathBuf::from("/opt/custom/cdz-compile"));
        unsafe { std::env::remove_var("CDZ_COMPILE_BIN") };
    }
}
