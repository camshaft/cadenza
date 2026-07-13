//! The `cdz-syntax` command surface — convert AND structurally query/rewrite a Cadenza program —
//! factored into the library so BOTH the standalone `cdz-syntax` bin and the unified `cdz` bin drive
//! ONE implementation (no duplicated arg-parsing / dispatch). The thin bins call [`parse_and_run`]
//! (whole-program) or embed [`Cmd`] as a subcommand group and call [`run`] (one command).
//!
//! Surfaces: `binary`, `sexpr`, `ml`, plus two output-only views of the binary AST — `debug` (an
//! indented tree) and `flat` (the arenas dumped literally: leaf pool + structure vector + root).
//!
//! ```text
//! convert [--from FMT] [--to FMT] [--width N] [FILE]
//! query   PATTERN          [FILE|DIR…] [--from FMT] [--count] [--json]
//! rewrite PATTERN TEMPLATE [FILE|DIR…] [--from FMT] [--to FMT] [--diff|--write|--json]
//! diff    FILE-A FILE-B    [--from FMT] [--json]
//! lint    [FILE|DIR…]      --rules FILE | --rule '(lint …)' [--from FMT] [--json]
//! clones  [FILE|DIR…]      [--min-size N] [--near] [--from FMT] [--json]
//! ```
//!
//! `--from`/`--to` are inferred from the FILE extension when omitted (`.cdz`/`.ml` → ml,
//! `.sexp`/`.sexpr` → sexpr, `.bin`/`.cdzb` → binary); `--to` defaults to `sexpr`. With no FILE (or
//! `-`), input is read from stdin (then `--from` is required). Output goes to stdout.
//!
//! `query`/`rewrite`/`diff`/`lint`/`clones` are the structural-editing codemod tool — Rung 2 of
//! `implementation/DESIGN-query-engine.md` (a built-in transform set run by a Rust driver), which
//! stands in for the eventual self-hosted sidecar (Rung 3; see the `query` module). A PATTERN/TEMPLATE
//! is s-expression text with `,x` (bind one node) and `,@xs` (bind a run of siblings) metavariables —
//! the same shape as the code it matches. A `rewrite` re-parses its result and rejects it if it does
//! not round-trip through the ML surface (a validated transaction — never a half-applied edit). This
//! module is the only place in the crate that touches the filesystem/stdio.

use crate::convert::{self, Format, Options};
use crate::query::clones;
use crate::query::lint::LintSet;
use crate::query::{self, Pattern, Query, Rule, RuleSet, Strategy, Template};
use clap::{Args, Parser, Subcommand, ValueEnum};
use std::io::{Read, Write};
use std::process::ExitCode;

/// The whole `cdz-syntax` CLI — the standalone bin's entry. The `cdz` bin does NOT use this; it embeds
/// [`Cmd`] as a subcommand group instead.
#[derive(Parser)]
#[command(
    name = "cdz-syntax",
    about = "Convert a Cadenza program between its surfaces."
)]
pub struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

/// The syntax subcommands — embeddable in another bin's subcommand tree (the `cdz` bin flattens these
/// alongside the compiler's).
#[derive(Subcommand)]
pub enum Cmd {
    /// Convert a program between surfaces (reads FILE or stdin, writes stdout).
    Convert(ConvertArgs),
    /// Structurally search a program for a PATTERN, printing each match (with its span) or a count.
    Query(QueryArgs),
    /// Structurally rewrite a program: replace every PATTERN match with TEMPLATE, validated.
    Rewrite(RewriteArgs),
    /// Structurally diff two programs: report which SUBTREES changed (not text lines).
    Diff(DiffArgs),
    /// Flag structural anti-patterns from a lint-rule set; exits non-zero on any `error` diagnostic.
    Lint(LintArgs),
    /// Find duplicated subtrees (clones) within/across programs — copy-paste to factor out.
    Clones(ClonesArgs),
}

#[derive(Args)]
pub struct ClonesArgs {
    /// Input files or directories (recursed by extension). Omit (or use `-`) to read stdin. Clones
    /// may span files.
    files: Vec<String>,

    /// Minimum subtree size (node count) to consider a clone — filters out trivial duplication.
    #[arg(long, default_value_t = 3)]
    min_size: usize,

    /// Find NEAR-clones (same shape, differing leaves) instead of exact clones. Reports the inferred
    /// `,mK`-metavariable pattern that matches every site — feedable into `rewrite`.
    #[arg(long)]
    near: bool,

    /// Input format. Inferred from each FILE's extension when omitted; required when reading stdin.
    #[arg(short, long, value_enum)]
    from: Option<Fmt>,

    /// Emit classes as JSON (exact: `[{exemplar, size, sites}]`; near: `[{pattern, size, holes, sites}]`).
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
pub struct LintArgs {
    /// Input files or directories (recursed by extension). Omit (or use `-`) to read stdin.
    files: Vec<String>,

    /// A file of `(lint PATTERN "message" [severity])` forms.
    #[arg(long)]
    rules: Option<String>,

    /// An inline lint rule, e.g. `--rule '(lint (deprecated ,@_) "avoid" error)'` (repeatable).
    /// Combined with any `--rules` file.
    #[arg(long)]
    rule: Vec<String>,

    /// Input format. Inferred from each FILE's extension when omitted; required when reading stdin.
    #[arg(short, long, value_enum)]
    from: Option<Fmt>,

    /// Emit diagnostics as JSON (`[{file?, line, col, severity, message, matched}]`).
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
pub struct DiffArgs {
    /// The "before" program.
    file_a: String,

    /// The "after" program.
    file_b: String,

    /// Input format for both files. Inferred from each extension when omitted.
    #[arg(short, long, value_enum)]
    from: Option<Fmt>,

    /// Emit changes as JSON (`[{path, kind, old?, new?}]`) for machine consumption.
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
pub struct QueryArgs {
    /// The s-expression pattern. Metavariables: `,x` (bind a node), `,@xs` (bind a run), `,_`
    /// (wildcard), `,(x GUARD…)` (guarded, e.g. `,(x is-literal)` / `,(x (head-is +))`).
    pub pattern: String,

    /// Input files or directories (recursed by extension). Omit (or use `-`) to read stdin.
    pub files: Vec<String>,

    /// Input format. Inferred from each FILE's extension when omitted; required when reading stdin.
    #[arg(short, long, value_enum)]
    pub from: Option<Fmt>,

    /// Print only the number of matches, not the matches themselves.
    #[arg(short, long)]
    pub count: bool,

    /// Emit matches as JSON (`[{file?, span, matched, bindings}]`) for machine consumption.
    #[arg(long)]
    pub json: bool,

    /// Keep only matches that occur INSIDE some ancestor matching this pattern (repeatable).
    #[arg(long)]
    pub inside: Vec<String>,

    /// Keep only matches that CONTAIN some descendant matching this pattern (repeatable).
    #[arg(long)]
    pub has: Vec<String>,

    /// Drop matches that occur inside an ancestor matching this pattern (repeatable).
    #[arg(long = "not-inside")]
    pub not_inside: Vec<String>,

    /// Drop matches that contain a descendant matching this pattern (repeatable).
    #[arg(long = "not-has")]
    pub not_has: Vec<String>,

    /// A SEMANTIC filter: keep only matches whose binding has the asked-for type, e.g.
    /// `--where 'type-of(x) = Int64'` (or `!=`). Only the unified `cdz` binary honors this (it needs
    /// the compiler); the pure `cdz-syntax` front-end ignores it. See `cdz`'s combined-query path.
    #[arg(long = "where")]
    pub where_: Option<String>,
}

#[derive(Args)]
pub struct RewriteArgs {
    /// The s-expression pattern to match (with `,x` / `,@xs` / guards). Omit with `--rules`.
    pattern: Option<String>,

    /// The s-expression replacement template; its metavariables are filled from the match.
    template: Option<String>,

    /// Input files or directories (recursed by extension). Omit (or use `-`) to read stdin.
    files: Vec<String>,

    /// Input format. Inferred from each FILE's extension when omitted; required when reading stdin.
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

    /// Reprint the WHOLE program through the pretty-printer instead of the default
    /// formatting-preserving (span-splicing) edit. Preserving mode keeps every unchanged byte —
    /// whitespace, newlines, comments, hand-alignment — exactly as it was, editing only the changed
    /// subtrees at their spans; it applies when the output surface matches the input (no `--to`
    /// conversion) and the input carries spans (ml/sexpr). Use `--reprint` to force a canonical
    /// reflow (e.g. to normalize layout). A cross-surface `--to` always reprints.
    #[arg(long)]
    reprint: bool,

    /// Show a unified diff of each change instead of the rewritten program. Preview mode.
    #[arg(long)]
    diff: bool,

    /// Apply the rewrite in place, overwriting each input FILE (only when it changes and validates).
    /// Requires FILE inputs (never stdin). Mutually exclusive with `--diff`/`--json`.
    #[arg(long)]
    write: bool,

    /// Emit the rewrite result as JSON (`{file?, count, rewritten}`) for machine consumption.
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
pub struct ConvertArgs {
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

/// The surface formats, as a clap `ValueEnum`. Mirrors [`Format`]. `pub` because it appears in the
/// (now-`pub`) `QueryArgs::from` field that the `cdz` bin reads.
#[derive(Clone, Copy, ValueEnum)]
pub enum Fmt {
    Binary,
    Sexpr,
    Ml,
    Markdown,
    Debug,
    Flat,
}

impl From<Fmt> for Format {
    fn from(f: Fmt) -> Format {
        match f {
            Fmt::Binary => Format::Binary,
            Fmt::Sexpr => Format::Sexpr,
            Fmt::Ml => Format::Ml,
            Fmt::Markdown => Format::Markdown,
            Fmt::Debug => Format::Debug,
            Fmt::Flat => Format::Flat,
        }
    }
}

/// Parse the whole `cdz-syntax` CLI from `std::env::args` and run it — the standalone bin's `main`.
pub fn parse_and_run() -> ExitCode {
    run(Cli::parse().command, "cdz-syntax")
}

/// Run one syntax command, reporting tool-level errors under `prog` (the invoking binary's name, so
/// `cdz` and `cdz-syntax` each prefix their own diagnostics). Returns the process exit code.
pub fn run(command: Cmd, prog: &str) -> ExitCode {
    // `lint` has a third outcome: it can run cleanly yet find `error` diagnostics, which must exit
    // non-zero (the CI gate) WITHOUT printing a tool-level error. So it returns `Ok(had_error)`.
    if let Cmd::Lint(args) = &command {
        return match run_lint(args) {
            Ok(false) => ExitCode::SUCCESS,
            Ok(true) => ExitCode::FAILURE, // error-severity diagnostics found
            Err(msg) => {
                eprintln!("{prog}: {msg}");
                ExitCode::FAILURE
            }
        };
    }
    let result = match command {
        Cmd::Convert(args) => run_convert(&args),
        Cmd::Query(args) => run_query(&args),
        Cmd::Rewrite(args) => run_rewrite(&args),
        Cmd::Diff(args) => run_diff(&args),
        Cmd::Clones(args) => run_clones(&args),
        Cmd::Lint(_) => unreachable!("handled above"),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("{prog}: {msg}");
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

/// One resolved target: an optional path (`None` = stdin) and the format to read it as.
struct TargetSpec {
    path: Option<String>,
    format: Format,
}

/// Expand the `files` argument list into concrete targets. An empty list (or a lone `-`) means
/// stdin (then `--from` supplies the format). A directory is recursed, collecting every file whose
/// extension maps to a surface (`.cdz`/`.ml`/`.sexp`/`.sexpr`/`.bin`/`.cdzb`); `--from` overrides the
/// per-file inference. Results are path-sorted for deterministic output.
fn collect_targets(files: &[String], from: Option<Fmt>) -> Result<Vec<TargetSpec>, String> {
    // stdin case
    if files.is_empty() || (files.len() == 1 && files[0] == "-") {
        return Ok(vec![TargetSpec {
            path: None,
            format: resolve_from(from, files.first().map(String::as_str))?,
        }]);
    }
    let mut out = Vec::new();
    for f in files {
        let p = std::path::Path::new(f);
        if p.is_dir() {
            let before = out.len();
            collect_dir(p, from, &mut out)?;
            if out.len() == before {
                eprintln!("cdz: {f}: no source files (.cdz/.ml/.sexp/.bin) found");
            }
        } else {
            // An explicitly-named file honors --from (or its extension); the user asked for it.
            let format = resolve_from(from, Some(f))?;
            out.push(TargetSpec {
                path: Some(f.clone()),
                format,
            });
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

/// Load one target, choosing between HARD-error and skip-with-warning. In a multi-target run
/// (`resilient == true`) a read/parse failure on one file warns to stderr and returns `None` (skip),
/// so one broken file can't abort a whole directory sweep; for a single target it propagates as an
/// error. Recoverable parse warnings (the recovering ML parser) are always reported, never fatal.
fn load_target(
    spec: &TargetSpec,
    resilient: bool,
) -> Result<Option<(query::driver::Target, String)>, String> {
    let load = || -> Result<(query::driver::Target, String), String> {
        let input = read_input(spec.path.as_deref())?;
        let src = String::from_utf8_lossy(&input).into_owned();
        let (target, errors) =
            query::driver::load(&input, spec.format).map_err(|e| with_path(&spec.path, &e))?;
        report_input_errors(spec.path.as_deref(), &errors);
        Ok((target, src))
    };
    match load() {
        Ok(v) => Ok(Some(v)),
        Err(e) if resilient => {
            eprintln!("cdz: skipping {e}");
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

/// Recurse `dir`, adding every file with a RECOGNIZED CODE surface extension (`.cdz`/`.ml`/`.sexp`/
/// `.sexpr`/`.bin`/`.cdzb`). A directory walk always filters by extension — non-source files (README,
/// `.gitignore`, …) are skipped, and markdown (`.md`) is skipped too: a `.md` is a literate document,
/// not code to sweep. `--from` overrides only the FORMAT the matched files are read as
/// (e.g. treat every `.cdz` as sexpr), NOT which files are included — so pointing at a dir can never
/// try to parse a README. (An explicitly-NAMED file always honors `--from`, since the user asked for
/// it; that path is in `collect_targets`, not here.) Unreadable entries warn and are skipped.
fn collect_dir(
    dir: &std::path::Path,
    from: Option<Fmt>,
    out: &mut Vec<TargetSpec>,
) -> Result<(), String> {
    let entries =
        std::fs::read_dir(dir).map_err(|e| format!("reading dir {}: {e}", dir.display()))?;
    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                eprintln!("cdz: skipping unreadable entry in {}: {e}", dir.display());
                continue;
            }
        };
        let path = entry.path();
        if path.is_dir() {
            collect_dir(&path, from, out)?;
        } else {
            let path_str = path.to_string_lossy().into_owned();
            // Only recognized CODE surfaces. `--from` picks the format; the extension gates inclusion.
            // Markdown is excluded from a directory SWEEP on purpose: a `.md` is a literate DOCUMENT
            // (READMEs, docs), not code to query/rewrite in bulk, so pointing a codemod at a tree must
            // not slurp in every README. An explicitly-NAMED `.md` still works — that path is in
            // `collect_targets`, which honors any recognized extension.
            if let Some(inferred) = Format::from_extension(&path_str)
                && inferred != Format::Markdown
            {
                out.push(TargetSpec {
                    path: Some(path_str),
                    format: from.map(Format::from).unwrap_or(inferred),
                });
            }
        }
    }
    Ok(())
}

/// Search the targets for a pattern (with optional structural context), printing matches, a count,
/// or JSON. Runs over one-or-more files (or stdin), reporting per file when more than one.
fn run_query(args: &QueryArgs) -> Result<(), String> {
    let pattern = Pattern::compile(&args.pattern).map_err(|e| e.to_string())?;
    let relquery = Query {
        inside: compile_patterns(&args.inside)?,
        has: compile_patterns(&args.has)?,
        not_inside: compile_patterns(&args.not_inside)?,
        not_has: compile_patterns(&args.not_has)?,
    };
    let targets = collect_targets(&args.files, args.from)?;
    let multi = targets.len() > 1;
    let mut total = 0usize;
    let mut json_objs: Vec<String> = Vec::new();

    for spec in &targets {
        let Some((target, _src)) = load_target(spec, multi)? else {
            continue;
        };

        if args.json {
            json_objs.push(query::driver::matches_json(
                &pattern,
                &relquery,
                &target,
                spec.path.as_deref(),
            ));
            continue;
        }
        if args.count {
            let n = query::count_with(&pattern, &relquery, &target.tree);
            total += n;
            if multi {
                println!("{}: {n}", label(&spec.path));
            }
            continue;
        }
        let report = query::driver::report_matches(&pattern, &relquery, &target);
        if multi && !report.is_empty() {
            println!("=== {} ===", label(&spec.path));
        }
        print!("{report}");
    }

    if args.json {
        // Concatenate the per-file arrays into one flat array of match objects.
        let inner: Vec<String> = json_objs
            .iter()
            .map(|a| a.trim_start_matches('[').trim_end_matches(']').to_string())
            .filter(|s| !s.is_empty())
            .collect();
        println!("[{}]", inner.join(","));
    } else if args.count && multi {
        println!("total: {total}");
    } else if args.count && !multi {
        println!("{total}");
    }
    Ok(())
}

/// Structurally diff two programs: report which subtrees changed (not text lines).
fn run_diff(args: &DiffArgs) -> Result<(), String> {
    let from_a = resolve_from(args.from, Some(&args.file_a))?;
    let from_b = resolve_from(args.from, Some(&args.file_b))?;
    let input_a = read_input(Some(&args.file_a))?;
    let input_b = read_input(Some(&args.file_b))?;
    let (a, errs_a) =
        query::driver::load(&input_a, from_a).map_err(|e| format!("{}: {e}", args.file_a))?;
    let (b, errs_b) =
        query::driver::load(&input_b, from_b).map_err(|e| format!("{}: {e}", args.file_b))?;
    report_input_errors(Some(&args.file_a), &errs_a);
    report_input_errors(Some(&args.file_b), &errs_b);

    if args.json {
        println!("{}", query::driver::changes_json(&a.tree, &b.tree));
    } else {
        let report = query::driver::changes_report(&a.tree, &b.tree);
        if report.is_empty() {
            eprintln!("cdz: no structural changes");
        } else {
            print!("{report}");
        }
    }
    Ok(())
}

/// Find duplicated subtrees across the targets (clones may span files), printing clone classes or
/// JSON. A loaded target's tree/spans/source-text is held for the duration so detection can borrow it.
fn run_clones(args: &ClonesArgs) -> Result<(), String> {
    let targets = collect_targets(&args.files, args.from)?;

    // Load every target, keeping (label, Target, source-text) alive so `clones::Source` can borrow.
    struct Loaded {
        label: String,
        target: query::driver::Target,
        src: String,
    }
    let multi = targets.len() > 1;
    let mut loaded: Vec<Loaded> = Vec::new();
    for spec in &targets {
        let Some((target, src)) = load_target(spec, multi)? else {
            continue;
        };
        loaded.push(Loaded {
            label: label(&spec.path),
            target,
            src,
        });
    }

    let sources: Vec<clones::Source> = loaded
        .iter()
        .map(|l| clones::Source {
            tree: &l.target.tree,
            spans: l.target.spans.as_ref(),
            file: Some(l.label.clone()),
        })
        .collect();
    // label → source text, for line:col rendering.
    let src_map: std::collections::HashMap<String, String> = loaded
        .iter()
        .map(|l| (l.label.clone(), l.src.clone()))
        .collect();

    if args.near {
        // Near-clones: same shape, differing leaves — report the inferred `,mK` pattern per class.
        let classes = clones::find_near_clones(&sources, args.min_size);
        if args.json {
            println!("{}", query::driver::near_clones_json(&classes, &src_map));
        } else {
            let report = query::driver::near_clones_report(&classes, &src_map);
            if report.is_empty() {
                eprintln!("cdz: no near-clones (min-size {})", args.min_size);
            } else {
                print!("{report}");
            }
        }
    } else {
        let classes = clones::find_clones_multi(&sources, args.min_size);
        if args.json {
            println!("{}", query::driver::clones_json(&classes, &src_map));
        } else {
            let report = query::driver::clones_report(&classes, &src_map);
            if report.is_empty() {
                eprintln!("cdz: no clones (min-size {})", args.min_size);
            } else {
                print!("{report}");
            }
        }
    }
    Ok(())
}

/// Lint the targets against a rule set, printing diagnostics (or JSON). Returns whether any
/// `error`-severity diagnostic fired (the caller maps `true` → non-zero exit, the CI gate).
fn run_lint(args: &LintArgs) -> Result<bool, String> {
    // Assemble the rule set from --rules FILE and/or inline --rule forms.
    let mut set = LintSet::default();
    if let Some(path) = &args.rules {
        let text = std::fs::read_to_string(path).map_err(|e| format!("reading {path}: {e}"))?;
        set.rules
            .extend(LintSet::compile(&text).map_err(|e| e.to_string())?.rules);
    }
    for r in &args.rule {
        set.rules
            .extend(LintSet::compile(r).map_err(|e| e.to_string())?.rules);
    }
    if set.rules.is_empty() {
        return Err("no lint rules — pass --rules FILE and/or --rule '(lint …)'".into());
    }

    let targets = collect_targets(&args.files, args.from)?;
    let multi = targets.len() > 1;
    let mut any_error = false;
    let mut json_objs: Vec<String> = Vec::new();

    for spec in &targets {
        let Some((target, src)) = load_target(spec, multi)? else {
            continue;
        };
        let lbl = label(&spec.path);

        if args.json {
            let (j, had_error) =
                query::driver::lint_json(&set, &target, &src, spec.path.as_deref());
            // `j` is a per-file array; collect its elements for one flat array at the end.
            let inner = j.trim_start_matches('[').trim_end_matches(']');
            if !inner.is_empty() {
                json_objs.push(inner.to_string());
            }
            any_error |= had_error;
        } else {
            let (report, had_error) = query::driver::lint_report(&set, &target, &src, &lbl);
            print!("{report}");
            any_error |= had_error;
        }
    }

    if args.json {
        println!("[{}]", json_objs.join(","));
    }
    Ok(any_error)
}

/// Rewrite the targets: apply the rule (or rule set) under the chosen strategy, validated, then
/// project — printing the result, a unified diff (`--diff`), JSON (`--json`), or writing in place
/// (`--write`). Runs over one-or-more files (or stdin).
fn run_rewrite(args: &RewriteArgs) -> Result<(), String> {
    if args.write && (args.diff || args.json) {
        return Err("--write is mutually exclusive with --diff / --json".into());
    }
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
    let targets = collect_targets(&args.files, args.from)?;
    if args.write && targets.iter().any(|t| t.path.is_none()) {
        return Err("--write needs FILE input(s), not stdin".into());
    }
    let multi = targets.len() > 1;
    let mut json_objs: Vec<String> = Vec::new();

    for spec in &targets {
        // Output format: --to, else the file extension, else the input format.
        let to = args
            .to
            .map(Format::from)
            .or_else(|| spec.path.as_deref().and_then(Format::from_extension))
            .unwrap_or(spec.format);

        let Some((target, src)) = load_target(spec, multi)? else {
            continue;
        };

        // Formatting-preserving (span-splicing) mode is the DEFAULT: it applies when the output
        // surface matches the input (no cross-surface `--to`) and the input carried spans. It edits
        // only changed subtrees at their spans, leaving all other bytes — layout/comments — verbatim.
        // `--reprint` forces the canonical whole-tree reflow; a cross-surface conversion always does.
        let preserving = !args.reprint && to == spec.format && target.spans.is_some();

        let (outcome, preserved) = if preserving {
            match query::driver::apply_rewrite_preserving(
                &rules,
                strategy,
                &target,
                &src,
                spec.format,
                args.fixpoint,
            ) {
                Ok(o) => (o, true),
                // If the span-splice can't be validated (a rewrite whose edit doesn't re-parse to
                // the intended tree — e.g. a shape the minimal-edit aligner can't place), fall back
                // to the whole-tree reprint rather than fail, warning that layout will reflow.
                Err(e) => {
                    eprintln!(
                        "cdz: {}: {e}; falling back to a full reprint (layout will reflow)",
                        label(&spec.path)
                    );
                    match reprint_outcome(&rules, strategy, &target, to, args.width, args.fixpoint)
                    {
                        Ok(o) => (o, false),
                        Err(e) if multi => {
                            eprintln!("cdz: skipping {}", with_path(&spec.path, &e));
                            continue;
                        }
                        Err(e) => return Err(with_path(&spec.path, &e)),
                    }
                }
            }
        } else {
            match reprint_outcome(&rules, strategy, &target, to, args.width, args.fixpoint) {
                Ok(o) => (o, false),
                // A rewrite that fails its validated-transaction check on one file of many warns and
                // skips (the other files still get rewritten); a single target is a hard error.
                Err(e) if multi => {
                    eprintln!("cdz: skipping {}", with_path(&spec.path, &e));
                    continue;
                }
                Err(e) => return Err(with_path(&spec.path, &e)),
            }
        };

        if args.json {
            json_objs.push(query::driver::rewrite_json(
                spec.path.as_deref(),
                outcome.count,
                &outcome.output,
            ));
            continue;
        }

        if args.diff {
            // The "before" side: the ORIGINAL SOURCE when preserving (so the diff shows only the
            // real edit, no reformatting noise), else the tree reprojected the same way as the result.
            let before = if preserved {
                src.clone()
            } else {
                query::driver::project_target(&target, to, args.width)?
            };
            let d = query::diff::unified(
                &before,
                &outcome.output,
                &format!("a/{}", label(&spec.path)),
                &format!("b/{}", label(&spec.path)),
            );
            if d.is_empty() {
                eprintln!("cdz: {}: no change", label(&spec.path));
            } else {
                print!("{d}");
            }
            continue;
        }

        if args.write {
            let path = spec.path.as_deref().expect("write requires a path");
            if outcome.count == 0 {
                eprintln!("cdz: {path}: no change");
            } else {
                let content = ensure_trailing_newline(&outcome.output);
                std::fs::write(path, content).map_err(|e| format!("writing {path}: {e}"))?;
                eprintln!("cdz: {path}: rewrote {} site(s)", outcome.count);
            }
            continue;
        }

        // Default: print the rewritten program to stdout, count to stderr.
        if multi {
            eprintln!(
                "cdz: {}: rewrote {} site(s)",
                label(&spec.path),
                outcome.count
            );
            println!("=== {} ===", label(&spec.path));
        } else {
            eprintln!("cdz: rewrote {} site(s)", outcome.count);
        }
        print!("{}", outcome.output);
        if !outcome.output.ends_with('\n') {
            println!();
        }
    }

    if args.json {
        println!("[{}]", json_objs.join(","));
    }
    Ok(())
}

/// The whole-tree reprint rewrite (the pre-ask-89 behavior): apply the rules and project the whole
/// program through the printer. Used for cross-surface `--to`, `--reprint`, and as the fallback when
/// a formatting-preserving splice can't be validated.
fn reprint_outcome(
    rules: &RuleSet,
    strategy: Strategy,
    target: &query::driver::Target,
    to: Format,
    width: usize,
    fixpoint: bool,
) -> Result<query::driver::RewriteOutcome, String> {
    query::driver::apply_rewrite(rules, strategy, target, to, width, fixpoint)
}

/// A display label for a target path (`(stdin)` when reading stdin).
fn label(path: &Option<String>) -> String {
    path.clone().unwrap_or_else(|| "(stdin)".to_string())
}

/// Prefix an error with the target path (or `(stdin)`), so a multi-file run points at the culprit.
fn with_path(path: &Option<String>, msg: &str) -> String {
    format!("{}: {msg}", label(path))
}

/// Ensure the text ends in exactly one newline (for writing a file back).
fn ensure_trailing_newline(s: &str) -> String {
    if s.ends_with('\n') {
        s.to_string()
    } else {
        format!("{s}\n")
    }
}

/// Note any recoverable parse errors in the input on stderr — the query still runs over the
/// recovered tree (the parser never bails), but the user should know the input wasn't clean.
fn report_input_errors(path: Option<&str>, errors: &[String]) {
    let where_ = path.unwrap_or("(stdin)");
    for e in errors {
        eprintln!("cdz: {where_}: input parse warning: {e}");
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
