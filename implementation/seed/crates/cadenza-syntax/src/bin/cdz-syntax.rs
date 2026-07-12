//! `cdz-syntax` — convert AND structurally query/rewrite a Cadenza program.
//!
//! Surfaces: `binary`, `sexpr`, `ml`, plus two output-only views of the binary AST — `debug` (an
//! indented tree) and `flat` (the arenas dumped literally: leaf pool + structure vector + root).
//!
//! ```text
//! cdz-syntax convert [--from FMT] [--to FMT] [--width N] [FILE]
//! cdz-syntax query   PATTERN          [FILE] [--from FMT] [--count]
//! cdz-syntax rewrite PATTERN TEMPLATE [FILE] [--from FMT] [--to FMT] [--width N] [--fixpoint]
//! ```
//!
//! `--from`/`--to` are inferred from the FILE extension when omitted (`.cdz`/`.ml` → ml,
//! `.sexp`/`.sexpr` → sexpr, `.bin`/`.cdzb` → binary); `--to` defaults to `sexpr`. With no FILE (or
//! `-`), input is read from stdin (then `--from` is required). Output goes to stdout.
//!
//! `query`/`rewrite` are the structural-editing codemod prototype (see the `query` module and
//! `implementation/DESIGN-query-engine.md`). A PATTERN/TEMPLATE is s-expression text with `,x`
//! (bind one node) and `,@xs` (bind a run of siblings) metavariables — the same shape as the code it
//! matches. A `rewrite` re-parses its result and rejects it if it does not round-trip through the ML
//! surface (a validated transaction — never a half-applied edit). This bin is the only place in the
//! crate that touches the filesystem/stdio.

use cadenza_syntax::convert::{self, Format, Options};
use cadenza_syntax::query::{self, Pattern, Query, Rule, RuleSet, Strategy, Template};
use clap::{Args, Parser, Subcommand, ValueEnum};
use std::io::{Read, Write};
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "cdz-syntax",
    about = "Convert a Cadenza program between its surfaces."
)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Convert a program between surfaces (reads FILE or stdin, writes stdout).
    Convert(ConvertArgs),
    /// Structurally search a program for a PATTERN, printing each match (with its span) or a count.
    Query(QueryArgs),
    /// Structurally rewrite a program: replace every PATTERN match with TEMPLATE, validated.
    Rewrite(RewriteArgs),
}

#[derive(Args)]
struct QueryArgs {
    /// The s-expression pattern. Metavariables: `,x` (bind a node), `,@xs` (bind a run), `,_`
    /// (wildcard), `,(x GUARD…)` (guarded, e.g. `,(x is-literal)` / `,(x (head-is +))`).
    pattern: String,

    /// Input file; omit or use `-` to read stdin.
    file: Option<String>,

    /// Input format. Inferred from FILE's extension when omitted; required when reading stdin.
    #[arg(short, long, value_enum)]
    from: Option<Fmt>,

    /// Print only the number of matches, not the matches themselves.
    #[arg(short, long)]
    count: bool,

    /// Keep only matches that occur INSIDE some ancestor matching this pattern (repeatable).
    #[arg(long)]
    inside: Vec<String>,

    /// Keep only matches that CONTAIN some descendant matching this pattern (repeatable).
    #[arg(long)]
    has: Vec<String>,

    /// Drop matches that occur inside an ancestor matching this pattern (repeatable).
    #[arg(long = "not-inside")]
    not_inside: Vec<String>,

    /// Drop matches that contain a descendant matching this pattern (repeatable).
    #[arg(long = "not-has")]
    not_has: Vec<String>,
}

#[derive(Args)]
struct RewriteArgs {
    /// The s-expression pattern to match (with `,x` / `,@xs` / guards). Omit with `--rules`.
    pattern: Option<String>,

    /// The s-expression replacement template; its metavariables are filled from the match.
    template: Option<String>,

    /// Input file; omit or use `-` to read stdin.
    file: Option<String>,

    /// Input format. Inferred from FILE's extension when omitted; required when reading stdin.
    #[arg(short, long, value_enum)]
    from: Option<Fmt>,

    /// Output format. Inferred from FILE's extension when omitted, else defaults to the input format.
    #[arg(short, long, value_enum)]
    to: Option<Fmt>,

    /// Target line width for `ml` output.
    #[arg(short, long, default_value_t = Options::default().width)]
    width: usize,

    /// Re-apply until the tree stops changing (bounded). Off by default (one pass).
    #[arg(long)]
    fixpoint: bool,

    /// A file of `(rule PATTERN TEMPLATE)` forms applied together (first match wins). Replaces the
    /// positional PATTERN/TEMPLATE — a peephole simplifier in one pass.
    #[arg(long)]
    rules: Option<String>,

    /// Traverse top-down (match outermost first) instead of the default bottom-up.
    #[arg(long = "top-down")]
    top_down: bool,
}

#[derive(Args)]
struct ConvertArgs {
    /// Input format. Inferred from FILE's extension when omitted; required when reading stdin.
    #[arg(short, long, value_enum)]
    from: Option<Fmt>,

    /// Output format. Inferred from FILE's extension when omitted, else defaults to `sexpr`.
    #[arg(short, long, value_enum)]
    to: Option<Fmt>,

    /// Target line width for `ml` output.
    #[arg(short, long, default_value_t = Options::default().width)]
    width: usize,

    /// Input file; omit or use `-` to read stdin.
    file: Option<String>,
}

/// The surface formats, as a clap `ValueEnum`. Mirrors [`Format`]; kept in the bin so the library
/// takes no CLI dependency.
#[derive(Clone, Copy, ValueEnum)]
enum Fmt {
    Binary,
    Sexpr,
    Ml,
    Debug,
    Flat,
}

impl From<Fmt> for Format {
    fn from(f: Fmt) -> Format {
        match f {
            Fmt::Binary => Format::Binary,
            Fmt::Sexpr => Format::Sexpr,
            Fmt::Ml => Format::Ml,
            Fmt::Debug => Format::Debug,
            Fmt::Flat => Format::Flat,
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Cmd::Convert(args) => run_convert(&args),
        Cmd::Query(args) => run_query(&args),
        Cmd::Rewrite(args) => run_rewrite(&args),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("cdz-syntax: {msg}");
            ExitCode::FAILURE
        }
    }
}

/// Read a program, convert it, write it.
fn run_convert(args: &ConvertArgs) -> Result<(), String> {
    let from = resolve_from(args.from, args.file.as_deref())?;
    let to = resolve_to(args.to, args.file.as_deref());

    let input = read_input(args.file.as_deref())?;
    let opts = Options { width: args.width };
    let output = convert::convert_with(&input, from, to, opts).map_err(|e| e.to_string())?;

    std::io::stdout()
        .write_all(&output)
        .map_err(|e| format!("writing stdout: {e}"))?;
    // A trailing newline after text output so the terminal prompt lands on its own line; binary
    // output is emitted exactly.
    if to != Format::Binary {
        let _ = std::io::stdout().write_all(b"\n");
    }
    Ok(())
}

/// Compile a list of pattern strings into patterns (for the relational flags).
fn compile_patterns(srcs: &[String]) -> Result<Vec<Pattern>, String> {
    srcs.iter()
        .map(|s| Pattern::compile(s).map_err(|e| e.to_string()))
        .collect()
}

/// Search the target for a pattern (with optional structural context), printing matches (or a count).
fn run_query(args: &QueryArgs) -> Result<(), String> {
    let from = resolve_from(args.from, args.file.as_deref())?;
    let input = read_input(args.file.as_deref())?;
    let pattern = Pattern::compile(&args.pattern).map_err(|e| e.to_string())?;
    let relquery = Query {
        inside: compile_patterns(&args.inside)?,
        has: compile_patterns(&args.has)?,
        not_inside: compile_patterns(&args.not_inside)?,
        not_has: compile_patterns(&args.not_has)?,
    };

    let (target, errors) = query::driver::load(&input, from)?;
    report_input_errors(&errors);

    if args.count {
        println!("{}", query::count_with(&pattern, &relquery, &target.tree));
    } else {
        let report = query::driver::report_matches(&pattern, &relquery, &target);
        // Print the report as-is (each match already ends in a newline); empty means no matches.
        print!("{report}");
    }
    Ok(())
}

/// Rewrite the target: apply the rule (or rule set) under the chosen strategy, validated, then project.
fn run_rewrite(args: &RewriteArgs) -> Result<(), String> {
    let from = resolve_from(args.from, args.file.as_deref())?;
    // Default the output format to the input format (a rewrite usually stays on the same surface),
    // overridable with --to; binary output is unsupported for rewrites (the driver rejects it).
    let to = args
        .to
        .map(Format::from)
        .or_else(|| args.file.as_deref().and_then(Format::from_extension))
        .unwrap_or(from);

    // The rule set comes from either --rules FILE, or the positional PATTERN + TEMPLATE.
    let rules = match &args.rules {
        Some(path) => {
            let text = std::fs::read_to_string(path).map_err(|e| format!("reading {path}: {e}"))?;
            RuleSet::compile(&text).map_err(|e| e.to_string())?
        }
        None => {
            let pattern = args
                .pattern
                .as_deref()
                .ok_or("a PATTERN (or --rules FILE) is required")?;
            let template = args
                .template
                .as_deref()
                .ok_or("a TEMPLATE is required (or use --rules FILE)")?;
            let p = Pattern::compile(pattern).map_err(|e| e.to_string())?;
            let t = Template::compile(template).map_err(|e| e.to_string())?;
            RuleSet::new(vec![Rule::new(p, t)])
        }
    };
    let strategy = if args.top_down {
        Strategy::TopDown
    } else {
        Strategy::BottomUp
    };

    let input = read_input(args.file.as_deref())?;
    let (target, errors) = query::driver::load(&input, from)?;
    report_input_errors(&errors);

    let outcome =
        query::driver::apply_rewrite(&rules, strategy, &target, to, args.width, args.fixpoint)?;
    // Report the site count to stderr (so stdout is exactly the rewritten program, pipeable).
    eprintln!("cdz-syntax: rewrote {} site(s)", outcome.count);
    print!("{}", outcome.output);
    if !outcome.output.ends_with('\n') {
        println!();
    }
    Ok(())
}

/// Note any recoverable parse errors in the input on stderr — the query still runs over the
/// recovered tree (the parser never bails), but the user should know the input wasn't clean.
fn report_input_errors(errors: &[String]) {
    for e in errors {
        eprintln!("cdz-syntax: input parse warning: {e}");
    }
}

/// Resolve the input format: explicit `--from`, else inferred from the file extension. Reading stdin
/// (no file, or `-`) with no `--from` and no inferable extension is an error.
fn resolve_from(from: Option<Fmt>, file: Option<&str>) -> Result<Format, String> {
    if let Some(f) = from {
        return Ok(f.into());
    }
    match file {
        Some(path) if path != "-" => Format::from_extension(path).ok_or_else(|| {
            format!("cannot infer input format from `{path}`; pass --from (binary|sexpr|ml)")
        }),
        _ => Err("reading stdin requires --from (binary|sexpr|ml)".to_string()),
    }
}

/// Resolve the output format: explicit `--to`, else inferred from the file extension, else `sexpr`.
fn resolve_to(to: Option<Fmt>, file: Option<&str>) -> Format {
    if let Some(t) = to {
        return t.into();
    }
    file.filter(|p| *p != "-")
        .and_then(Format::from_extension)
        .unwrap_or(Format::Sexpr)
}

/// Read the whole input from `file` (or stdin when `None` or `"-"`).
fn read_input(file: Option<&str>) -> Result<Vec<u8>, String> {
    match file {
        None | Some("-") => {
            let mut buf = Vec::new();
            std::io::stdin()
                .read_to_end(&mut buf)
                .map_err(|e| format!("reading stdin: {e}"))?;
            Ok(buf)
        }
        Some(path) => std::fs::read(path).map_err(|e| format!("reading {path}: {e}")),
    }
}
