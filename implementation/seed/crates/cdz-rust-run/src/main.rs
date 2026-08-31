//! `cdz-rust-run` — the RUST-backend corpus exec runner CLI.
//!
//! `cdz-rust-run --grade <test-run.ast> [--module <emitted.rs>] [--async] [--compile-status N]
//!  [--compile-diag <path>] [--cdz-rt-dir <d>] [--cdz-num-dir <d>] [--cadenza-ast-dir <d>]
//!  [--workdir <d>]`
//!
//! Grades a Cadenza program's emitted Rust against a shredded `test-run.ast`, reproducing the `xtask gate`
//! comparison for every outcome kind — the Rust-target analogue of `cdz-run --grade` (wasm). RUN outcomes
//! (`expect-output`/`expect-trap`) assemble a driver around `--module` and compile+run it with `rustc`
//! (linking the pre-built runtime rlibs the caller points at); COMPILE outcomes (`expect-error`/
//! `expect-declines`) + `warns` are graded from `--compile-status`/`--compile-diag` (no `--module`). The
//! nix per-case rust exec layer shells out to this bin, one case per invocation, so a compiler change with
//! byte-identical emit re-runs nothing. Exit `0` if all pass (or `Todo`), `1` on the first `Fail`.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;

use cdz_rust_run::run::RlibDirs;

/// Grade a Cadenza program's emitted Rust against a shredded corpus `test-run.ast`.
#[derive(Parser)]
#[command(
    name = "cdz-rust-run",
    about = "Grade emitted Rust against a corpus test-run.ast: compile, run, verdict."
)]
struct Cli {
    /// The shredded `test-run.ast` (from `cdz corpus records --out-dir`) to grade against — REQUIRED.
    #[arg(long, value_name = "TEST_RUN_AST")]
    grade: PathBuf,

    /// The emitted `--target rust[-async]` module source. OPTIONAL: absent means the compile was REFUSED
    /// (a `--compile-status` != 0 error/declines case), graded from the diagnostic with no run.
    #[arg(long, value_name = "PATH")]
    module: Option<PathBuf>,

    /// The emitted module is an async (`--target rust-async`) one — link `cdz_rt` and read the async
    /// signature markers.
    #[arg(long)]
    r#async: bool,

    /// The exit status of the case's compile (`0` = compiled → `--module` present; non-zero = the compiler
    /// refused → an error/declines outcome). Defaults to `0`.
    #[arg(long, value_name = "N", default_value_t = 0)]
    compile_status: i32,

    /// The compiler's captured stderr (the diagnostic), for grading `expect-error`/`expect-declines` (code
    /// + message) and `warns` (presence). Empty/absent → no diagnostic text.
    #[arg(long, value_name = "PATH")]
    compile_diag: Option<PathBuf>,

    /// The compiler's STRUCTURED diagnostics wire (`KIND_DIAGNOSTICS` / `cdz check --json`) for grading a
    /// case's DIAGNOSTIC-QUALITY facets (`(fix …)`/`(no-fix)`/`(count N)`). Absent → quality grading OFF.
    #[arg(long, value_name = "PATH")]
    diagnostics: Option<PathBuf>,

    /// The directory holding `libcdz_rt.rlib` (linked only in `--async` mode).
    #[arg(long, value_name = "DIR")]
    cdz_rt_dir: Option<PathBuf>,

    /// The directory holding `libcdz_num.rlib` (a BigInt program's `cdz_num::Big`).
    #[arg(long, value_name = "DIR")]
    cdz_num_dir: Option<PathBuf>,

    /// The directory holding `libcadenza_ast.rlib` (+ its `deps/` for `num_bigint`) — the native value codec.
    #[arg(long, value_name = "DIR")]
    cadenza_ast_dir: Option<PathBuf>,

    /// Where to write + compile the per-trial driver (each trial in its own subdir). Defaults to a
    /// process-unique dir under the system temp dir.
    #[arg(long, value_name = "DIR")]
    workdir: Option<PathBuf>,

    /// The committed rust baseline (`spec/semantics/.gate-baseline-rust`), a `<verdict>\t<description>`
    /// snapshot. When given, a REGRESSION — a case the baseline recorded as `pass` that no longer passes —
    /// fails the grade (exit 1), the per-case analogue of `xtask gate --check --target rust` (gap #7).
    #[arg(long, value_name = "PATH")]
    baseline: Option<PathBuf>,

    /// CLASSIFY mode (`--emit-verdict PATH`, the nix `.#corpus-verdicts-rust[-async]` harvest / `gate --save`
    /// replacement): write this case's current verdict (`<tag>\t<description>`, tag ∈ pass/todo/fail from
    /// `Grade::verdict` — the coarse vocab `.gate-baseline-rust*` records) to `<PATH>` and ALWAYS exit 0.
    /// Independent of `--baseline` and takes precedence over it (a save/harvest run classifies the CURRENT
    /// state; it never regression-fails). The rust/rust-async analogue of `cdz-run --emit-verdict`.
    #[arg(long = "emit-verdict", value_name = "PATH")]
    emit_verdict: Option<PathBuf>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match real_main(&cli) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("cdz-rust-run: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn real_main(cli: &Cli) -> anyhow::Result<ExitCode> {
    let test_run_ast = std::fs::read(&cli.grade)
        .map_err(|e| anyhow::anyhow!("read test-run.ast {}: {e}", cli.grade.display()))?;
    let module = match &cli.module {
        Some(p) => Some(
            std::fs::read_to_string(p)
                .map_err(|e| anyhow::anyhow!("read module {}: {e}", p.display()))?,
        ),
        None => None,
    };
    let compile_diag = match &cli.compile_diag {
        Some(p) => std::fs::read_to_string(p).unwrap_or_default(),
        None => String::new(),
    };
    let diag_wire: Option<Vec<u8>> = cli
        .diagnostics
        .as_ref()
        .map(|p| std::fs::read(p).unwrap_or_default());
    let baseline = match &cli.baseline {
        Some(p) => Some(
            std::fs::read_to_string(p)
                .map_err(|e| anyhow::anyhow!("read baseline {}: {e}", p.display()))?,
        ),
        None => None,
    };
    let rlibs = RlibDirs {
        cdz_rt: cli.cdz_rt_dir.clone(),
        cdz_num: cli.cdz_num_dir.clone(),
        cadenza_ast: cli.cadenza_ast_dir.clone(),
    };
    let workdir = match &cli.workdir {
        Some(d) => d.clone(),
        None => std::env::temp_dir().join(format!("cdz-rust-run-{}", std::process::id())),
    };
    std::fs::create_dir_all(&workdir)?;

    cdz_rust_run::grade::grade(
        module.as_deref(),
        &test_run_ast,
        &rlibs,
        cli.r#async,
        cli.compile_status,
        &compile_diag,
        diag_wire.as_deref(),
        &workdir,
        baseline.as_deref(),
        cli.emit_verdict.as_deref(),
    )
}
