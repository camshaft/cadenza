//! `cdz-calc` — the native Cadenza calculator REPL.
//!
//! `cdz-calc` (interactive) or `cdz-calc --once "<expr>"` (compute one line and exit — the hook a
//! launcher / script shells out to). You type an expression in the same language you write programs in;
//! it compiles + runs and prints the value. Variables accumulate (`x = 5`, then `x * x` → `25`), and
//! `ans` recalls the last result. ML surface by default; `--sexpr` for s-expressions.
//!
//! This bin is a thin shim: the command surface lives in `cdz_calc::cli` (an embeddable clap `Args`
//! group + a `run` entry), so the unified `cdz` binary can mount the SAME code as `cdz calc …` without
//! a second binary on the PATH. Both entry points share one implementation and one `--help`; the
//! evaluation is the existing pipeline (front-end → `rcdzc` → `cdz-run`), wired in the `cdz_calc` library.

use std::process::ExitCode;

use clap::Parser;

/// The Cadenza calculator: a REPL over the real language, exact by construction.
#[derive(Parser)]
#[command(
    name = "cdz-calc",
    about = "A Cadenza calculator REPL: type an expression, get its value; assign variables and recall them."
)]
struct Cli {
    #[command(flatten)]
    calc: cdz_calc::cli::CalcArgs,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    cdz_calc::cli::run(&cli.calc, "cdz-calc")
}
