//! `cdz-calc` — the native Cadenza calculator REPL.
//!
//! `cdz-calc` (interactive) or `cdz-calc --once "<expr>"` (compute one line and exit — the hook a
//! launcher / script shells out to). You type an expression in the same language you write programs in;
//! it compiles + runs and prints the value. Variables accumulate (`x = 5`, then `x * x` → `25`), and
//! `ans` recalls the last result. ML surface by default; `--sexpr` for s-expressions.
//!
//! The evaluation is the existing pipeline (front-end → `rcdzc` → `cdz-run`), wired together in the
//! `cdz_calc` library; this bin is just the input loop + rendering.

use std::io::Write;
use std::process::ExitCode;

use cadenza_syntax::convert::Format;
use cdz_calc::{Calculator, Eval};
use clap::Parser;

/// The Cadenza calculator: a REPL over the real language, exact by construction.
#[derive(Parser)]
#[command(
    name = "cdz-calc",
    about = "A Cadenza calculator REPL: type an expression, get its value; assign variables and recall them."
)]
struct Cli {
    /// Evaluate ONE expression, print its value, and exit — the one-shot mode a launcher/script uses
    /// (`cdz-calc --once "1/3 + 1/3 + 1/3"`). Without it, `cdz-calc` starts the interactive loop.
    #[arg(long, value_name = "EXPR")]
    once: Option<String>,

    /// Read + render in the s-expression surface instead of the default ML surface.
    #[arg(long)]
    sexpr: bool,

    /// Turn OFF exact mode: a bare numeric literal keeps its ordinary Int64/Float default, so `1 / 3` is
    /// integer division (0). By DEFAULT exact mode is ON (forced rationals) — `1 / 3` is `1/3`.
    #[arg(long = "no-exact")]
    no_exact: bool,

    /// Print results as the BARE value (`1/3`, `1500 meter`, `42`) — stripping the `: Type` suffix, and
    /// showing a whole rational `5/1` as `5`. The launcher-facing shape (Raycast/Alfred, `--once --plain`).
    #[arg(long)]
    plain: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let surface = if cli.sexpr { Format::Sexpr } else { Format::Ml };
    let exact = !cli.no_exact;

    match &cli.once {
        Some(expr) => run_once(expr, surface, exact, cli.plain),
        None => run_interactive(surface, exact, cli.plain),
    }
}

/// One-shot: evaluate `expr` against an empty binding set, print the value to stdout (a trap/error to
/// stderr), and exit — success only when the expression produced a value. The shape a Raycast/Alfred
/// launcher consumes: clean value on stdout, non-zero + message on stderr otherwise.
fn run_once(expr: &str, surface: Format, exact: bool, plain: bool) -> ExitCode {
    let mut calc = Calculator::new_with_exact(surface, exact).with_plain(plain);
    match calc.eval(expr) {
        Eval::Value(v) | Eval::Bound { value: v, .. } => {
            println!("{v}");
            ExitCode::SUCCESS
        }
        Eval::Trap(msg) => {
            eprintln!("trap: {msg}");
            ExitCode::FAILURE
        }
        Eval::Error(msg) => {
            eprintln!("error: {msg}");
            ExitCode::FAILURE
        }
    }
}

/// The interactive loop: read a line, evaluate it, print the result, repeat. Reads plain lines from
/// stdin (EOF / an empty `:q` line quits). A blank line is skipped. `:help` prints a short reminder.
fn run_interactive(surface: Format, exact: bool, plain: bool) -> ExitCode {
    let mut calc = Calculator::new_with_exact(surface, exact).with_plain(plain);
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();

    println!(
        "Cadenza calculator ({} surface{}). Type an expression; `name = expr` binds a variable; `ans` \
         recalls the last result. `:q` or Ctrl-D to quit, `:help` for help.",
        surface.name(),
        if exact { ", exact" } else { "" }
    );

    let mut buf = String::new();
    loop {
        // Prompt (flush so it shows before the read blocks).
        let _ = write!(out, "› ");
        let _ = out.flush();
        buf.clear();
        match stdin.read_line(&mut buf) {
            Ok(0) => {
                println!(); // newline after the dangling prompt on EOF
                break;
            }
            Ok(_) => {}
            Err(e) => {
                eprintln!("cdz-calc: read error: {e}");
                return ExitCode::FAILURE;
            }
        }
        let line = buf.trim();
        if line.is_empty() {
            continue;
        }
        match line {
            ":q" | ":quit" | ":exit" => break,
            ":help" => {
                print_help(surface);
                continue;
            }
            ":vars" => {
                let names = calc.names();
                if names.is_empty() {
                    println!("(no variables bound)");
                } else {
                    println!("bound: {}", names.join(", "));
                }
                continue;
            }
            _ => {}
        }
        print_eval(&calc.eval(line));
    }
    ExitCode::SUCCESS
}

/// Print one evaluation outcome to stdout, with a small role marker.
fn print_eval(result: &Eval) {
    match result {
        Eval::Value(v) => println!("= {v}"),
        Eval::Bound { name, value } => println!("{name} = {value}"),
        Eval::Trap(msg) => println!("trap: {msg}"),
        Eval::Error(msg) => println!("error: {msg}"),
    }
}

fn print_help(surface: Format) {
    let (bind, expr, eq) = match surface {
        Format::Sexpr => ("x = (+ 2 3)", "(* x x)", "(Rational.of 1 3)"),
        _ => ("x = 2 + 3", "x * x", "1/3 + 1/3 + 1/3"),
    };
    println!(
        "Type an expression to evaluate it. Examples:\n  \
         {expr}            an expression using a bound variable\n  \
         {bind}         bind a variable (recall it later, or `ans` for the last result)\n  \
         {eq}   exact arithmetic\n\
         Commands: :q quit · :help this · :vars list bound variables"
    );
}
