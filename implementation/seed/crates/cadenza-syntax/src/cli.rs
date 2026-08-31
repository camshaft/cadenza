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
//! fmt     [FILE|DIR…]      [--from FMT] [--width N] [--check|--diff|--stdout]
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
//! `implementation/design/DESIGN-query-engine.md` (a built-in transform set run by a Rust driver), which
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
///
/// `Query`/`Rewrite` are the documented structural interface an agent uses to read and rewrite a
/// program's canonical representation directly — matching/editing the arena tree by s-expression
/// pattern, never patching source text:
//
//= spec/capabilities/agent-authoring.md#a-structural-interface-exists
//# The language MUST expose a documented interface to read a program's canonical representation without textual patching.
//
//= spec/capabilities/agent-authoring.md#a-structural-interface-exists
//# The language MUST expose a documented interface to rewrite a program's canonical representation without textual patching.
#[derive(Subcommand)]
pub enum Cmd {
    /// Convert a program between surfaces (reads FILE or stdin, writes stdout).
    Convert(ConvertArgs),
    /// Format program file(s) in place: reprint each canonically in its OWN surface.
    Fmt(FmtArgs),
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
    /// Extract a program's public doc surface into a derived `doc-module` doc-AST (cadenza-docs I1):
    /// per exported `def`/`type`/`effect`, a `doc-item` with its name, printed signature, and `///`
    /// prose. Reads FILE or stdin; writes the doc-AST to stdout (canonical binary by default, or a
    /// surface via `--to` for inspection). The output IS `cdzast` — a queryable structured doc index.
    Doc(DocArgs),
    /// Apply a canonicalizing NORMALIZATION codemod (opt-in, distinct from `fmt`). Currently the
    /// single-clause-irrefutable-`match`→`let` rewrite (`--match-to-let`), an idiom cleanup that
    /// `fmt` deliberately does NOT do (it would change the AST shape). Reads FILE(s) or stdin; writes
    /// in place, or to stdout / `--check` / `--diff` like `fmt`.
    Normalize(NormalizeArgs),
}

#[derive(Args)]
pub struct NormalizeArgs {
    /// Files or directories to normalize (directories are recursed by extension). Omit (or use `-`)
    /// to read stdin and write the normalized program to stdout.
    files: Vec<String>,

    /// Apply the single-clause-irrefutable-`match`→`let` rewrite: `match v with | (a,b) => body`
    /// becomes `let (a, b) = v in body`. ONLY an irrefutable, unguarded, single-clause `match` is
    /// rewritten (a refutable/multi-clause/guarded one is left alone — rewriting it would erase a
    /// trap). At least one normalization flag is required (this is the only one today).
    #[arg(long = "match-to-let")]
    match_to_let: bool,

    /// Input surface. Inferred from each FILE's extension when omitted; required when reading stdin.
    /// Same-surface (a `.cdz` reprints as ML): normalization edits the tree, never changes surface.
    #[arg(short, long, value_enum)]
    from: Option<Fmt>,

    /// Target line width for the pretty-printer.
    #[arg(short, long, default_value_t = Options::default().width)]
    width: usize,

    /// Don't write anything; exit non-zero if any file WOULD be changed by the normalization, listing
    /// which (the CI/`--check` shape).
    #[arg(long)]
    check: bool,

    /// Show a unified diff of what the normalization WOULD change, without writing. Preview mode.
    #[arg(long)]
    diff: bool,

    /// Write the normalized program to stdout instead of editing the file in place. (The implicit
    /// mode when input is stdin.) Mutually exclusive with `--check`/`--diff`.
    #[arg(long)]
    stdout: bool,
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

    /// Suppress a named lint (or a whole group, e.g. `idiomatic`). Repeatable. Overrides the rule's
    /// default level; a CLI level wins over any module `(allow/warn/deny …)` directive.
    #[arg(long = "allow", value_name = "NAME")]
    allow: Vec<String>,

    /// Report a named lint (or group) as a warning. Repeatable. See `--allow` for precedence.
    #[arg(long = "warn", value_name = "NAME")]
    warn: Vec<String>,

    /// Promote a named lint (or group) to an error (fails the run). Repeatable. See `--allow`.
    #[arg(long = "deny", value_name = "NAME")]
    deny: Vec<String>,

    /// Apply each lint's `Verified` fix in place (an equivalence-preserving codemod), rewriting each
    /// input FILE. Only `Verified` fixes apply (add `--heuristic` to opt in Heuristic ones); a lint
    /// set to `--allow` (or an in-source `@allow`) applies no fix. Requires FILE inputs (never stdin);
    /// the edit is formatting-preserving. Mutually exclusive with `--json`.
    #[arg(long)]
    fix: bool,

    /// With `--fix`, also apply `Heuristic` fixes (offered-not-auto by default). Ignored without `--fix`.
    #[arg(long)]
    heuristic: bool,

    /// Target line width for a reprinted `--fix` result (used only when the formatting-preserving
    /// splice must fall back to a whole-tree reprint).
    #[arg(short, long, default_value_t = Options::default().width)]
    width: usize,
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

    /// Warn (on stderr) when the TEMPLATE introduces a binder (`let`/`fn`/`def`) whose name occurs FREE
    /// inside a matched metavariable's subtree — a silent variable CAPTURE: splicing that subtree under
    /// the new binder re-scopes its free occurrences to the template's binder, changing the program's
    /// meaning even though the rewrite is a faithful structural replace. Purely a diagnostic (semantics
    /// unchanged, no α-renaming — binding is the compiler's domain, not this syntax layer's); the fix is
    /// the template author's (rename the binder, or match a fresh name). Off by default.
    #[arg(long = "warn-capture")]
    warn_capture: bool,
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

    /// Render `--to sexpr` STRUCTURALLY: comment nodes as ordinary `(comment "text" form)` lists rather
    /// than `;` line-comments (the `render_sexpr` parse-tree form the `spec/syntax/` golden corpus uses).
    /// No effect on a non-`sexpr` target.
    #[arg(long)]
    structural: bool,

    /// Input file; omit or use `-` to read stdin.
    file: Option<String>,
}

#[derive(Args)]
pub struct DocArgs {
    /// Input format of the PROGRAM. Inferred from FILE's extension when omitted; required for stdin.
    #[arg(short, long, value_enum)]
    from: Option<Fmt>,

    /// Output format for the doc-AST. Defaults to `binary` (canonical `cdzast\x00\x01` — the doc index
    /// is a binary AST); use `sexpr`/`ml` to inspect it, or `json`/etc. as any surface of the arena.
    #[arg(short, long, value_enum)]
    to: Option<Fmt>,

    /// The module name recorded in the emitted `(doc-module "…")`. Defaults to the input file's stem
    /// (or `module` for stdin).
    #[arg(short, long)]
    module: Option<String>,

    /// Target line width for a text output surface.
    #[arg(short, long, default_value_t = Options::default().width)]
    width: usize,

    /// Input file; omit or use `-` to read stdin.
    file: Option<String>,
}

#[derive(Args)]
pub struct FmtArgs {
    /// Files or directories to format (directories are recursed by extension). Omit (or use `-`) to
    /// read stdin and write the formatted program to stdout.
    files: Vec<String>,

    /// Input surface. Inferred from each FILE's extension when omitted; required when reading stdin.
    /// Formatting is same-surface (a `.cdz` reprints as ML, a `.sexp` as s-expr) — `fmt` never
    /// changes a file's surface; use `convert` for that.
    #[arg(short, long, value_enum)]
    from: Option<Fmt>,

    /// Target line width for the pretty-printer.
    #[arg(short, long, default_value_t = Options::default().width)]
    width: usize,

    /// Don't write anything; exit non-zero if any file is not already canonically formatted, listing
    /// which. The CI shape (the `cargo fmt --check` analogue) — pairs naturally with a directory input.
    #[arg(long)]
    check: bool,

    /// Show a unified diff of what formatting WOULD change, without writing. Preview mode.
    #[arg(long)]
    diff: bool,

    /// Write the formatted program to stdout instead of editing the file in place. (The implicit mode
    /// when input is stdin.) Mutually exclusive with `--check`/`--diff`.
    #[arg(long)]
    stdout: bool,
}

impl FmtArgs {
    /// Build a `FmtArgs` over an EXPLICIT, already-resolved file list, preserving the mode flags of a
    /// parsed `FmtArgs`. The `files`/`from`/`width`/mode fields are private (clap-derived), so a caller
    /// that resolves its own target set — the `cdz` bin expanding a `Project.cdz` manifest into its
    /// concrete entry+modules+tests files for `cdz fmt` with no argument — cannot construct one directly.
    /// This lets that caller keep the project-manifest knowledge on its side and hand `fmt` the resolved
    /// list, while formatting stays surface-only here (v-cdz-tooling coordination: the one lifecycle
    /// command that lacked no-arg/project resolution). `self`'s `from`/`width`/`check`/`diff`/`stdout` are
    /// carried over unchanged; only the target list is replaced.
    pub fn with_files(self, files: Vec<String>) -> FmtArgs {
        FmtArgs { files, ..self }
    }

    /// The parsed positional targets, exactly as the user supplied them (before directory recursion or
    /// stdin resolution). The read side of [`with_files`](Self::with_files): a caller that owns project
    /// resolution — the `cdz` bin deciding whether `cdz fmt` should enter project-mode — inspects these
    /// to classify the invocation before rebuilding the args. Empty means "no positionals" (stdin, or a
    /// project sweep); a lone `-` is the explicit stdin marker; a single directory or `Project.cdz` is a
    /// project target. `fmt` itself never needs this (it resolves internally via `collect_targets`); it
    /// exists purely so the classify logic can stay on the caller's side. The field stays private.
    pub fn files(&self) -> &[String] {
        &self.files
    }
}

/// The surface formats, as a clap `ValueEnum`. Mirrors [`Format`]. `pub` because it appears in the
/// (now-`pub`) `QueryArgs::from` field that the `cdz` bin reads.
#[derive(Clone, Copy, ValueEnum)]
pub enum Fmt {
    Binary,
    Sexpr,
    Ml,
    Markdown,
    Json,
    Toml,
    Cedar,
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
            Fmt::Json => Format::Json,
            Fmt::Toml => Format::Toml,
            Fmt::Cedar => Format::Cedar,
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
    // `fmt --check` has the same third outcome as `lint`: it runs cleanly yet must exit non-zero when
    // some file is not formatted (the CI gate), without printing a tool-level error. So it returns
    // `Ok(all_formatted)` and maps a `false` to a non-zero exit here.
    if let Cmd::Fmt(args) = &command {
        return match run_fmt(args) {
            Ok(true) => ExitCode::SUCCESS,
            Ok(false) => ExitCode::FAILURE, // `--check`: some file was not formatted
            Err(msg) => {
                eprintln!("{prog}: {msg}");
                ExitCode::FAILURE
            }
        };
    }
    // `normalize --check` has the same third outcome as `fmt --check`: a clean run that must exit
    // non-zero when some file WOULD be normalized (the CI gate). Returns `Ok(all_normalized)`.
    if let Cmd::Normalize(args) = &command {
        return match run_normalize(args) {
            Ok(true) => ExitCode::SUCCESS,
            Ok(false) => ExitCode::FAILURE, // `--check`: some file would be normalized
            Err(msg) => {
                eprintln!("{prog}: {msg}");
                ExitCode::FAILURE
            }
        };
    }
    let result = match command {
        Cmd::Convert(args) => run_convert(&args),
        Cmd::Doc(args) => run_doc(&args),
        Cmd::Query(args) => run_query(&args),
        Cmd::Rewrite(args) => run_rewrite(&args),
        Cmd::Diff(args) => run_diff(&args),
        Cmd::Clones(args) => run_clones(&args),
        Cmd::Lint(_) | Cmd::Fmt(_) | Cmd::Normalize(_) => unreachable!("handled above"),
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
    let opts = Options {
        width: args.width,
        structural: args.structural,
        ..Options::default()
    };
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

/// `cdz doc` (cadenza-docs I1): read a PROGRAM, project its public doc surface into a `doc-module`
/// doc-AST, and emit that doc-AST. The projection is `doc_item::project` (a compiler query over the
/// program); the result is ordinary `cdzast`, so it emits through the SAME surface writer as `convert`
/// — canonical binary `\x00\x01` by default (the doc index is a binary AST), or a text surface via
/// `--to` for inspection. The doc-module name is `--module`, else the input file's stem, else `module`.
fn run_doc(args: &DocArgs) -> Result<(), String> {
    let from = resolve_from(args.from, args.file.as_deref())?;
    // Default output = canonical binary (the doc-AST is a binary index); `--to` overrides for inspection.
    let to = args.to.map(Format::from).unwrap_or(Format::Binary);
    let module_name = args
        .module
        .clone()
        .unwrap_or_else(|| module_stem(args.file.as_deref()));

    let input = read_input(args.file.as_deref())?;
    let output = doc_bytes(&input, from, to, &module_name, args.width)?;

    std::io::stdout()
        .write_all(&output)
        .map_err(|e| format!("writing stdout: {e}"))?;
    if to != Format::Binary {
        let _ = std::io::stdout().write_all(b"\n");
    }
    Ok(())
}

/// The byte-producing core of `cdz doc` (factored out of [`run_doc`] so it's testable without stdout):
/// read `input` as a program in `from`, project its public doc surface into a `doc-module` doc-AST, and
/// write that doc-AST in `to`. The projection ([`crate::doc_item::project`]) is a compiler query over
/// the program; the result is ordinary `cdzast`, emitted through the shared surface writer.
fn doc_bytes(
    input: &[u8],
    from: Format,
    to: Format,
    module_name: &str,
    width: usize,
) -> Result<Vec<u8>, String> {
    let program = convert::read(input, from).map_err(|e| e.to_string())?;
    let doc_ast = crate::doc_item::project(&program, module_name);
    let opts = Options {
        width,
        ..Options::default()
    };
    convert::write_with(&doc_ast, to, opts).map_err(|e| e.to_string())
}

/// The default `doc-module` name for a program: the input file's stem (`lib.cdz` → `lib`), or `module`
/// for stdin / a path with no usable stem.
fn module_stem(file: Option<&str>) -> String {
    file.filter(|p| *p != "-")
        .and_then(|p| std::path::Path::new(p).file_stem())
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "module".to_string())
}

/// Format the targets: read each, reprint it CANONICALLY in its OWN surface (a same-surface round-trip
/// through the printer — `.cdz` as ML, `.sexp` as s-expr), and edit it in place when the bytes change.
/// `fmt` never changes a file's surface (that is `convert`'s job) and never rewrites structure (that is
/// `rewrite`'s) — it only normalizes layout, so an already-canonical file is left byte-identical.
///
/// The write modes mirror the codemod tool: default is IN-PLACE (only files that change are rewritten,
/// reported to stderr); `--stdout` prints the result instead (the implicit mode for stdin, which has no
/// file to edit); `--diff` previews a unified diff; `--check` writes nothing and reports whether every
/// file was already formatted. Returns `Ok(all_formatted)` — always `true` outside `--check`; under
/// `--check`, `false` when some file needed formatting (the caller maps that to a non-zero exit, the CI
/// gate). A file that does not parse is a hard error and is NEVER written (the reader rejects a recovered
/// tree, so a broken file can't be silently "reformatted" into a patched-up shape) — in a multi-file run
/// it warns and skips, so one unparseable file can't abort a whole directory sweep.
/// The comment lexis of a surface — which marker starts a line comment, so the write-path guard counts
/// the RIGHT thing per file. The ML/text surfaces use `//` (+ `///` doc); the s-expr surface uses a
/// single `;`. A `;` in ML text is the SEQUENCE operator, NOT a comment — so the guard MUST NOT count
/// `;` as a comment on an ML file (and must NOT count `//` on a `.sexp` file). Derived from [`Format`].
#[derive(Clone, Copy, PartialEq)]
enum CommentLexis {
    /// `//` line comment, `///` doc comment (ML, and the doc/markdown text surfaces).
    SlashSlash,
    /// `;` line comment (s-expr). No doc-comment distinction.
    Semicolon,
    /// A surface with no line-comment lexis this guard models (binary/json/toml/cedar/…) — count nothing.
    None,
}

impl From<Format> for CommentLexis {
    fn from(f: Format) -> Self {
        match f {
            // ML surfaces (and doc/markdown, which carry `//`/`///` in embedded code) use slash comments.
            Format::Ml | Format::Markdown => CommentLexis::SlashSlash,
            Format::Sexpr => CommentLexis::Semicolon,
            _ => CommentLexis::None,
        }
    }
}

/// The number of DOC and plain-COMMENT markers in `text`, per the surface's [`CommentLexis`], counting a
/// marker ANYWHERE on a line (leading OR trailing — `x = 1 // note` / `(f x) ; note` both count),
/// scanning raw text and SKIPPING a marker inside a `"…"` string or `#\…` char literal (so a `//` in a
/// URL, or a `;` inside a `";"` string, is not miscounted). String state is tracked ACROSS lines, so a
/// marker on a CONTINUATION line of a MULTI-LINE string literal is skipped too — e.g. a `;` inside a
/// multi-line `(doc "…; …")` s-expr string, or a `//` inside a multi-line ML string, is not miscounted as
/// a comment. (Without this, re-wrapping such a doc string to a different width shifts which continuation
/// lines carry a `;`, so the count drifts and the guard falsely refuses — the exact false-positive that
/// blocked `cdz fmt` on every heavily-doc-commented `spec/semantics/*.sexp`.) At most one marker per line
/// (a comment runs to end-of-line). For `SlashSlash`, `///` counts ONLY as a doc; `Semicolon` has no doc
/// distinction (all s-expr `;` are plain comments). Returned as `(doc, comment)`.
///
/// Used by the WRITE paths of `fmt`/`normalize` to detect a reprint that DROPS a comment marker — the
/// exact signature of a reader comment-attachment gap (a comment the reader loses is invisible to any
/// ARENA-level check, because it never became a node; only this raw-text count catches it). The guard
/// then refuses the write, turning silent comment-loss into a visible fail-safe no-op. (Fixes a gap where
/// a `.sexp` file's `;` comments were counted as 0 under the ML-only `//` scan, so the guard never fired
/// and the s-expr printer silently dropped them — v-lsp/v-cdz-tooling report.)
fn comment_counts(text: &str, lexis: CommentLexis) -> (usize, usize) {
    if lexis == CommentLexis::None {
        return (0, 0);
    }
    let mut docs = 0;
    let mut comments = 0;
    // `in_str` is carried ACROSS lines so a multi-line string literal's continuation lines are scanned as
    // string content (a marker inside them is skipped), not as fresh code. A comment marker is only ever
    // reached while `!in_str`, and we `break` at it, so `in_str` is never corrupted by the break.
    let mut in_str = false;
    for line in text.lines() {
        let bytes = line.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let c = bytes[i];
            if in_str {
                if c == b'\\' {
                    i += 2;
                    continue;
                }
                if c == b'"' {
                    in_str = false;
                }
                i += 1;
                continue;
            }
            if c == b'"' {
                in_str = true;
                i += 1;
                continue;
            }
            if c == b'#' && bytes.get(i + 1) == Some(&b'\\') {
                // A char literal `#\x` — skip the `#\` and the escaped char so its content never scans.
                i += 3.min(bytes.len() - i);
                continue;
            }
            match lexis {
                CommentLexis::SlashSlash if c == b'/' && bytes.get(i + 1) == Some(&b'/') => {
                    if bytes.get(i + 2) == Some(&b'/') {
                        docs += 1;
                    } else {
                        comments += 1;
                    }
                    break; // rest of line is comment text
                }
                CommentLexis::Semicolon if c == b';' => {
                    comments += 1;
                    break; // rest of line is comment text
                }
                _ => {}
            }
            i += 1;
        }
    }
    (docs, comments)
}

/// True if reprinting `input` as `output` (both in surface `lexis`) would DROP a doc or plain comment —
/// the fail-safe signal for `fmt`/`normalize`'s write paths. Only a NET decrease in either count trips it
/// (a transform that adds, or leaves counts equal, is fine); an increase never trips (so `normalize`'s
/// match→let, which only adds a `let`, is unaffected). Both args are the final text about to be written.
fn would_drop_comments(input: &[u8], output: &[u8], lexis: CommentLexis) -> Option<String> {
    let (in_docs, in_comments) = comment_counts(&String::from_utf8_lossy(input), lexis);
    let (out_docs, out_comments) = comment_counts(&String::from_utf8_lossy(output), lexis);
    if out_docs < in_docs || out_comments < in_comments {
        let mut lost = Vec::new();
        if out_docs < in_docs {
            lost.push(format!("{} doc-comment(s) (`///`)", in_docs - out_docs));
        }
        if out_comments < in_comments {
            let marker = if lexis == CommentLexis::Semicolon {
                "`;`"
            } else {
                "`//`"
            };
            lost.push(format!(
                "{} comment(s) ({marker})",
                in_comments - out_comments
            ));
        }
        Some(lost.join(" + "))
    } else {
        None
    }
}

/// Whether a target's formatted output should be written straight to stdout (the disposition shared by
/// `run_fmt` and `run_normalize`), given whether it came from stdin and the mode flags.
///
/// This is the exact locus of a fixed bug: `--stdout` is an unconditional "emit the formatted program"
/// mode, and reading stdin has no file to edit so it ALSO emits to stdout by default — BUT only when
/// NOT inspecting. `--check`/`--diff` are no-write INSPECTION modes, and piping into `--check` is a
/// natural CI shape (`cat f | cdz fmt - --check` = "is this already formatted?"). The old code gated the
/// stdin emit on `spec.path.is_none()` ALONE, evaluated BEFORE the check/diff branches, so `fmt - --check`
/// printed the formatted text and exited 0 even on UNFORMATTED stdin — a silent false-pass defeating the
/// CI contract. So a stdin `--check`/`--diff` must FALL THROUGH to the inspection branches (labelled
/// `<stdin>`), i.e. NOT emit here. `--stdout` is rejected up front as mutually exclusive with
/// `--check`/`--diff`, so it never co-occurs with them; it always emits.
fn emits_to_stdout(from_stdin: bool, stdout: bool, check: bool, diff: bool) -> bool {
    stdout || (from_stdin && !check && !diff)
}

/// Format one surface for `cdz fmt`: read + re-print canonically in the SAME surface. For a MULTI-FORM
/// s-expr file (a corpus of top-level `(case …)` forms), the single-form `convert::read` inside
/// `convert_with` fails with "trailing input"; fall back to reading it MULTI-form (`sexpr::read_all` → a
/// synthetic `(do …)`) and print via `print_pretty_program` (seq-256 `(do)`-unwrap, flush-left top level)
/// so a multi-form .sexp fmt-normalizes + round-trips. Scoped to the fmt path — `convert::read` stays
/// single-form for its other callers. A single-form .sexp is unaffected (it takes the `convert_with`
/// path); a genuinely malformed .sexp still errors (both paths reject it, so a broken file is never
/// silently rewritten).
fn fmt_surface(
    input: &[u8],
    format: Format,
    opts: Options,
) -> Result<Vec<u8>, convert::ConvertError> {
    match convert::convert_with(input, format, format, opts) {
        Ok(b) => Ok(b),
        Err(e) => {
            if format == Format::Sexpr
                && let Ok(text) = std::str::from_utf8(input)
                && let Ok(arenas) = crate::sexpr::read_all(text)
            {
                return Ok(crate::sexpr::print_pretty_program(&arenas, opts.width).into_bytes());
            }
            Err(e)
        }
    }
}

fn run_fmt(args: &FmtArgs) -> Result<bool, String> {
    // `--check`/`--diff` inspect without writing; `--stdout` writes elsewhere. They are exclusive: a
    // single run has one output disposition, so an ambiguous combination is rejected up front rather
    // than silently letting one win.
    let modes = [args.check, args.diff, args.stdout];
    if modes.iter().filter(|m| **m).count() > 1 {
        return Err("--check, --diff, and --stdout are mutually exclusive".into());
    }

    let targets = collect_targets(&args.files, args.from)?;
    let multi = targets.len() > 1;
    let opts = Options {
        width: args.width,
        ..Options::default()
    };
    let mut all_formatted = true;

    for spec in &targets {
        // Read the raw bytes ourselves (not via `load_target`) so we can compare the formatted output
        // against the EXACT original bytes — the "is it already canonical?" test — and, on a parse
        // failure, decline the file rather than format a recovered tree.
        let input = match read_input(spec.path.as_deref()) {
            Ok(b) => b,
            Err(e) if multi => {
                eprintln!("cdz: skipping {}", with_path(&spec.path, &e));
                continue;
            }
            Err(e) => return Err(with_path(&spec.path, &e)),
        };
        // Format = read the surface and re-print it canonically in the SAME surface. `convert::read`
        // (inside `convert_with`) rejects a program that only parses with recovered errors, so a broken
        // file errors here instead of being rewritten to a patched-up form.
        let formatted = match fmt_surface(&input, spec.format, opts) {
            Ok(mut b) => {
                // The printer emits no trailing newline; keep a file newline-terminated (and stable
                // under re-formatting) by appending one, matching `rewrite --write`'s convention.
                if b.last() != Some(&b'\n') {
                    b.push(b'\n');
                }
                b
            }
            Err(e) if multi => {
                eprintln!("cdz: skipping {}", with_path(&spec.path, &e.to_string()));
                continue;
            }
            Err(e) => return Err(with_path(&spec.path, &e.to_string())),
        };

        // Emit to stdout — the EXPLICIT `--stdout` mode, and the IMPLICIT mode when reading stdin (see
        // `emits_to_stdout`, which also carries the `fmt - --check` false-pass fix). Always prints the
        // formatted text, even when already canonical: `--stdout`/implicit-stdin is a "give me the
        // formatted program" request (like `convert`), not a conditional edit.
        if emits_to_stdout(spec.path.is_none(), args.stdout, args.check, args.diff) {
            std::io::stdout()
                .write_all(&formatted)
                .map_err(|e| format!("writing stdout: {e}"))?;
            continue;
        }
        // The label for messages/diffs: a real path, or `<stdin>` for a piped `--check`/`--diff`.
        let path = spec.path.as_deref().unwrap_or("<stdin>");

        if formatted == input {
            // Already canonical — nothing to do in the write/diff/check modes (no diff, no write, no
            // `--check` failure).
            continue;
        }

        if args.check {
            // The input WOULD change — report it and remember to fail (but keep scanning the rest, so
            // one `--check` run lists every unformatted file; a piped stdin reports as `<stdin>`).
            all_formatted = false;
            println!("not formatted: {path}");
            continue;
        }
        if args.diff {
            // Compare the original bytes against the formatted result (both lossy-decoded — a formatter
            // only touches text surfaces, so this is faithful).
            let before = String::from_utf8_lossy(&input);
            let after = String::from_utf8_lossy(&formatted);
            print!(
                "{}",
                query::diff::unified(&before, &after, &format!("a/{path}"), &format!("b/{path}"))
            );
            continue;
        }
        // COMMENT-SAFETY GUARD: refuse to write a reprint that would DROP a `///`/`//` line. `fmt` is
        // meant to preserve every comment; a drop means a reader doc/comment-attachment gap ate one, and
        // silently overwriting the file would DESTROY it. Fail-safe: skip this file with a clear message
        // + remember to exit non-zero, so a comment-loss becomes a visible no-op instead of data loss.
        // (Guards the WRITE path only — `--stdout`/`--diff`/`--check` don't overwrite the source.)
        if let Some(lost) = would_drop_comments(&input, &formatted, spec.format.into()) {
            all_formatted = false;
            eprintln!(
                "cdz: refusing to format {path}: would drop {lost} (a reader comment-attachment gap)"
            );
            continue;
        }
        // Default: edit in place. (`--stdout`/stdin were handled before the already-canonical check.)
        std::fs::write(path, &formatted).map_err(|e| format!("writing {path}: {e}"))?;
        eprintln!("cdz: formatted {path}");
    }

    Ok(all_formatted)
}

/// Apply a canonicalizing normalization codemod to each target. Mirrors [`run_fmt`]'s disposition
/// modes (stdin→stdout, `--stdout`, `--check`, `--diff`, in-place write), but instead of reprinting
/// the SAME tree it transforms the arena first (the whole point of a normalization). Today the sole
/// normalization is `--match-to-let`. Because the codemod deliberately CHANGES the AST shape (a match
/// becomes a let), the output is a canonical reprint of the transformed tree — this is NOT
/// formatting-preserving and is NOT `fmt` (which is why it is a separate, opt-in command). Returns
/// `Ok(all_unchanged)` so `--check` can map "some file would change" to a non-zero exit.
fn run_normalize(args: &NormalizeArgs) -> Result<bool, String> {
    let modes = [args.check, args.diff, args.stdout];
    if modes.iter().filter(|m| **m).count() > 1 {
        return Err("--check, --diff, and --stdout are mutually exclusive".into());
    }
    // At least one normalization must be requested — an empty `normalize` would silently no-op every
    // file, reading like "already normalized" when nothing was even attempted.
    if !args.match_to_let {
        return Err("a normalization is required (currently only `--match-to-let`)".into());
    }

    let targets = collect_targets(&args.files, args.from)?;
    let multi = targets.len() > 1;
    let opts = Options {
        width: args.width,
        ..Options::default()
    };
    let mut all_unchanged = true;

    for spec in &targets {
        let input = match read_input(spec.path.as_deref()) {
            Ok(b) => b,
            Err(e) if multi => {
                eprintln!("cdz: skipping {}", with_path(&spec.path, &e));
                continue;
            }
            Err(e) => return Err(with_path(&spec.path, &e)),
        };
        // Parse to an arena (rejecting a recovered/broken parse, like `fmt`), apply the codemod on the
        // owned `Tree`, and reprint canonically in the SAME surface.
        let arenas = match convert::read(&input, spec.format) {
            Ok(a) => a,
            Err(e) if multi => {
                eprintln!("cdz: skipping {}", with_path(&spec.path, &e.to_string()));
                continue;
            }
            Err(e) => return Err(with_path(&spec.path, &e.to_string())),
        };
        let tree = crate::query::Tree::of(&arenas);
        let (rewritten, count) = crate::match_to_let::rewrite(&tree);
        let mut normalized = match convert::write_with(&rewritten.to_arena(), spec.format, opts) {
            Ok(b) => b,
            Err(e) => return Err(with_path(&spec.path, &e.to_string())),
        };
        if normalized.last() != Some(&b'\n') {
            normalized.push(b'\n');
        }

        // stdout — the explicit `--stdout` mode and the implicit mode for stdin input. Shares
        // `run_fmt`'s `emits_to_stdout` decision, so `normalize - --check` piped from CI exits non-zero
        // on a would-normalize input instead of silently printing + exiting 0 (the same false-pass fix).
        if emits_to_stdout(spec.path.is_none(), args.stdout, args.check, args.diff) {
            std::io::stdout()
                .write_all(&normalized)
                .map_err(|e| format!("writing stdout: {e}"))?;
            continue;
        }
        // The label for messages/diffs: a real path, or `<stdin>` for a piped `--check`/`--diff`.
        let path = spec.path.as_deref().unwrap_or("<stdin>");

        // Nothing rewritten → the file is already normalized. (Guard on the rewrite COUNT, not a byte
        // compare: a canonical reprint of an untransformed tree can still differ from the original
        // bytes — that reflow is `fmt`'s job, not `normalize`'s. Only a real match→let counts.)
        if count == 0 {
            continue;
        }

        if args.check {
            // Only `--check` maps a would-change file to a non-zero exit (the CI gate). Actually
            // writing/diffing a changed file is SUCCESS — the change is the point.
            all_unchanged = false;
            println!("would normalize: {path} ({count} match→let)");
            continue;
        }
        if args.diff {
            let before = String::from_utf8_lossy(&input);
            let after = String::from_utf8_lossy(&normalized);
            print!(
                "{}",
                query::diff::unified(&before, &after, &format!("a/{path}"), &format!("b/{path}"))
            );
            continue;
        }
        // COMMENT-SAFETY GUARD (same as run_fmt): a normalization must never DROP a comment. match→let
        // only ADDS a `let` (never removes a `///`/`//`), so this won't false-trip today; it fail-safes
        // any future normalization — or a latent reader gap in the reprint — into a visible no-op rather
        // than silent comment-loss. Skip + exit non-zero (via `all_unchanged=false`) instead of writing.
        if let Some(lost) = would_drop_comments(&input, &normalized, spec.format.into()) {
            all_unchanged = false;
            eprintln!(
                "cdz: refusing to normalize {path}: would drop {lost} (a reader comment-attachment gap)"
            );
            continue;
        }
        std::fs::write(path, &normalized).map_err(|e| format!("writing {path}: {e}"))?;
        eprintln!("cdz: normalized {path} ({count} match→let)");
    }

    Ok(all_unchanged)
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
            // Markdown, JSON, TOML, and Cedar are excluded from a directory SWEEP on purpose: a `.md`
            // is a literate DOCUMENT (READMEs, docs) and `.json`/`.toml`/`.cedar` are DATA (configs,
            // fixtures, manifests like `Cargo.toml`, authorization policies — and JSONC that isn't even
            // strict JSON), not code to query/rewrite in bulk, so pointing a codemod at a tree must not
            // slurp them in. An explicitly-NAMED `.md`/`.json`/`.toml`/`.cedar` still works — that path
            // is in `collect_targets`, which honors any recognized extension.
            if let Some(inferred) = Format::from_extension(&path_str)
                && inferred != Format::Markdown
                && inferred != Format::Json
                && inferred != Format::Toml
                && inferred != Format::Cedar
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
    if args.fix && args.json {
        return Err(
            "--fix is mutually exclusive with --json (a fix rewrites files, not JSON)".into(),
        );
    }
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
    // With NO explicit rules, `cdz lint FILE` runs the BUILT-IN `idiomatic` catalog (the Tier-A pack) —
    // so the command is useful out of the box, not an error. Explicit --rules/--rule still fully replace
    // it (they populate `set` above); a level flag/directive then tunes whichever set is active.
    if set.rules.is_empty() {
        set = LintSet::builtin();
    }

    // Build the CLI level overrides (allow/warn/deny NAME). Later flags win on the same key; a CLI
    // level overrides a rule's default. Applied in a stable order (allow, warn, deny) so that if the
    // SAME name is passed to two of them the last-listed flag group wins — deterministic, not arg-order
    // dependent. This is the CLI layer; it is overlaid ON TOP of each target's `@`-attribute lint
    // directives (CLI wins), matching the §3 order CLI > in-source attribute > rule-default.
    let mut cli_levels = crate::query::lint::LintLevels::new();
    for n in &args.allow {
        cli_levels.set(n.clone(), crate::query::lint::LintLevel::Allow);
    }
    for n in &args.warn {
        cli_levels.set(n.clone(), crate::query::lint::LintLevel::Warn);
    }
    for n in &args.deny {
        cli_levels.set(n.clone(), crate::query::lint::LintLevel::Deny);
    }

    let targets = collect_targets(&args.files, args.from)?;
    if args.fix && targets.iter().any(|t| t.path.is_none()) {
        return Err("--fix needs FILE input(s), not stdin".into());
    }
    let multi = targets.len() > 1;
    let mut any_error = false;
    let mut json_objs: Vec<String> = Vec::new();

    for spec in &targets {
        let Some((target, src)) = load_target(spec, multi)? else {
            continue;
        };
        let lbl = label(&spec.path);

        // Resolve this target's effective levels: its own `@allow/@warn/@deny(NAME)` attribute
        // directives FIRST, then the CLI flags overlaid on top (CLI wins). Recomputed per target so
        // each file honors its OWN in-source attributes.
        let mut levels = crate::query::lint::LintLevels::from_attributes(&target.tree);
        levels.overlay(&cli_levels);

        if args.fix {
            // Apply each firing lint's Verified fix (Heuristic too under --heuristic) as a validated,
            // formatting-preserving codemod, then write the file back only when it changed. Reporting
            // is a separate action from fixing (DESIGN-cadenza-lint §5): --fix rewrites, it does not
            // also print the warnings it resolved.
            let outcome = query::driver::lint_fix_with_levels(
                &set,
                &target,
                &src,
                spec.format,
                &levels,
                args.heuristic,
                args.width,
            )
            .map_err(|e| with_path(&spec.path, &e))?;
            let path = spec.path.as_deref().expect("--fix requires a path");
            if outcome.count == 0 {
                eprintln!("cdz: {path}: no fixable lints");
            } else {
                let content = ensure_trailing_newline(&outcome.output);
                std::fs::write(path, content).map_err(|e| format!("writing {path}: {e}"))?;
                eprintln!("cdz: {path}: fixed {} lint(s)", outcome.count);
            }
            continue;
        }

        if args.json {
            let (j, had_error) = query::driver::lint_json_with_levels(
                &set,
                &target,
                &src,
                spec.path.as_deref(),
                &levels,
            );
            // `j` is a per-file array; collect its elements for one flat array at the end.
            let inner = j.trim_start_matches('[').trim_end_matches(']');
            if !inner.is_empty() {
                json_objs.push(inner.to_string());
            }
            any_error |= had_error;
        } else {
            let (report, had_error) =
                query::driver::lint_report_with_levels(&set, &target, &src, &lbl, &levels);
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
    // A template metavariable the pattern never binds can NEVER be filled, so every site would fail to
    // instantiate and the rewrite would silently report "rewrote 0 site(s)" — reading like the pattern
    // just did not match, hiding the real (static) cause. Reject it UP FRONT, naming the stray metavar,
    // so a typo'd/leftover template metavar is caught before the search. Checked per rule so a `--rules`
    // file's bad rule is named too.
    for (i, rule) in rules.rules.iter().enumerate() {
        let unbound = rule.unbound_template_metavars();
        if let Some(first) = unbound.first() {
            let quoted: Vec<String> = unbound.iter().map(|m| format!("`,{m}`")).collect();
            let where_ = if args.rules.is_some() {
                format!(" (rule {})", i + 1)
            } else {
                String::new()
            };
            let plural = if unbound.len() == 1 {
                "metavariable"
            } else {
                "metavariables"
            };
            return Err(format!(
                "the template{where_} uses {plural} {} that the pattern never binds — a template \
                 metavariable must be bound by its pattern, or it can never be filled (nearest fix: \
                 bind `,{first}` in the pattern, or remove it from the template)",
                quoted.join(", ")
            ));
        }
    }
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

        // `--warn-capture`: before rewriting, scan each rule's matches for a template binder that would
        // capture a free name inside a matched metavariable's subtree, and warn. Diagnostic only — the
        // rewrite proceeds unchanged (the structural-replace contract has no α-renaming).
        if args.warn_capture {
            for (ri, rule) in rules.rules.iter().enumerate() {
                for m in query::search(&rule.pattern, &target.tree, None) {
                    for risk in rule.template.capture_risks(&m.bindings) {
                        let where_rule = if rules.rules.len() > 1 {
                            format!(" (rule {})", ri + 1)
                        } else {
                            String::new()
                        };
                        // Name the metavar with its real sigil — `,@e` for a splice, `,e` for a single.
                        let sigil = if risk.is_splice { ",@" } else { "," };
                        eprintln!(
                            "cdz: {}: warning{where_rule}: template binder `{}` may capture the free \
                             `{}` inside the matched `{sigil}{}` — the spliced code's `{}` will resolve \
                             to the template's binder, not the outer one (rename the binder or match a \
                             fresh name; --warn-capture is advisory, the rewrite is unchanged)",
                            label(&spec.path),
                            risk.binder,
                            risk.binder,
                            risk.metavar,
                            risk.binder,
                        );
                    }
                }
            }
        }

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
            // No inferable extension. Distinguish the two reasons so the message is actionable: a path
            // that DOES NOT EXIST is a missing-file typo (say so — the `--from` advice would send the
            // user chasing a format when the real fix is the path), whereas a file that EXISTS but has
            // an unknown/absent extension genuinely needs `--from`.
            if !std::path::Path::new(path).exists() {
                format!("no such file `{path}`")
            } else {
                format!("cannot infer input format from `{path}`; pass --from (binary|sexpr|ml)")
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_from_distinguishes_a_missing_file_from_an_unknown_extension() {
        // No `--from` + no inferable extension: a path that DOES NOT EXIST is a missing-file typo (the
        // `--from` advice would misdirect), while an EXISTING extensionless file genuinely needs `--from`.
        let missing = resolve_from(None, Some("/tmp/cdz-nope-extensionless")).unwrap_err();
        assert!(
            missing.contains("no such file"),
            "a nonexistent extensionless path is a missing-file error; got {missing}"
        );
        // An EXISTING extensionless file → the format-inference error (needs --from).
        let dir = std::env::temp_dir().join("cdz_resolve_from_test");
        std::fs::create_dir_all(&dir).unwrap();
        let existing = dir.join("noext");
        std::fs::write(&existing, b"(module m)").unwrap();
        let unknown = resolve_from(None, Some(&existing.to_string_lossy())).unwrap_err();
        assert!(
            unknown.contains("cannot infer input format"),
            "an existing extensionless file needs --from; got {unknown}"
        );
        // A path WITH a known extension resolves regardless of existence (the read error, if any, comes
        // later) — so a nonexistent `.sexp` still infers sexpr here.
        assert!(resolve_from(None, Some("/tmp/cdz-nope.sexp")).is_ok());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn comment_counts_finds_leading_and_trailing_markers_skipping_strings() {
        use CommentLexis::{Semicolon, SlashSlash};
        // ML surface (`//`/`///`): leading doc + leading comment + trailing comment; a `//` in a string
        // is NOT counted.
        assert_eq!(
            comment_counts("/// doc\ndef f() = 1 // trailing\n// lead", SlashSlash),
            (1, 2)
        );
        // A `///` counts ONLY as a doc, never also as a comment.
        assert_eq!(comment_counts("/// just a doc", SlashSlash), (1, 0));
        // `//` inside a string literal is skipped (a URL).
        assert_eq!(
            comment_counts("def u() = \"http://x.com\"", SlashSlash),
            (0, 0)
        );
        // A trailing comment AFTER a string still counts; the string's `//` does not.
        assert_eq!(
            comment_counts("let u = \"a//b\" // real", SlashSlash),
            (0, 1)
        );
        // A `//` on a CONTINUATION line of a MULTI-LINE ML string is skipped too (string state carried
        // across lines) — only the real trailing `// real` counts.
        assert_eq!(
            comment_counts("let u = \"a\n b//c\nd\" // real", SlashSlash),
            (0, 1),
            "// inside a multi-line string continuation is not a comment"
        );
        // Only the FIRST marker per line counts (a `//` runs to end-of-line).
        assert_eq!(comment_counts("x // a // b", SlashSlash), (0, 1));
        // No markers.
        assert_eq!(comment_counts("def f() = 1 + 2", SlashSlash), (0, 0));

        // S-EXPR surface (`;`): a leading + trailing `;` comment; a `;` in a string is NOT counted; and
        // — crucially — `//` is NOT a comment in s-expr (it's ordinary), while a `;` IS. No doc marker.
        assert_eq!(
            comment_counts("; header\n(def (f) 1) ; trailing", Semicolon),
            (0, 2)
        );
        assert_eq!(
            comment_counts("(f \"a;b\")", Semicolon),
            (0, 0),
            "; in a string"
        );
        // A `;` on a CONTINUATION line of a MULTI-LINE string is NOT a comment — string state is carried
        // across lines. (The false-positive that refused every doc-commented `spec/semantics/*.sexp`: a
        // multi-line `(doc "…; …")` whose continuation line's `;` was miscounted as a comment.) Only the
        // real leading `; c` counts.
        assert_eq!(
            comment_counts(
                "; c\n(doc \"one; still string\n two; also string\")",
                Semicolon
            ),
            (0, 1),
            "; inside a multi-line string continuation is not a comment"
        );
        // The closing `"` re-opens code, so a genuine trailing `;` AFTER a multi-line string still counts.
        assert_eq!(
            comment_counts("(doc \"a\nb\") ; real", Semicolon),
            (0, 1),
            "trailing ; after a multi-line string still counts"
        );
        // A `;` INSIDE a `#\…` char literal (`#\;` — the semicolon character, a real Cadenza s-expr datum)
        // is NOT a comment; miscounting it would falsely refuse (or, if it flipped in_str, mask a real
        // drop). The `#\` skip consumes the char, so the following real `; c` is what counts.
        assert_eq!(
            comment_counts("(f #\\;) ; c", Semicolon),
            (0, 1),
            "; in a #\\; char literal is not a comment; the trailing ; is"
        );
        // A `#\"` char literal is the QUOTE character — it must NOT open a string (else a following `;`
        // would be swallowed as string content). The `#\` skip consumes it, so the trailing `; c` counts.
        assert_eq!(
            comment_counts("(f #\\\") ; c", Semicolon),
            (0, 1),
            "#\\\" is a char literal, not a string opener"
        );
        assert_eq!(
            comment_counts("(f x) // not a sexpr comment", Semicolon),
            (0, 0)
        );
        // And the ML lexis does NOT count a `;` (it's the ML sequence operator, not a comment).
        assert_eq!(
            comment_counts("a; b", SlashSlash),
            (0, 0),
            "; is ML seq, not a comment"
        );
        // A None-lexis surface counts nothing.
        assert_eq!(
            comment_counts("; anything // here", CommentLexis::None),
            (0, 0)
        );
    }

    #[test]
    fn would_drop_comments_trips_only_on_a_net_decrease() {
        use CommentLexis::{Semicolon, SlashSlash};
        // A dropped trailing comment (1 → 0) trips.
        assert!(would_drop_comments(b"x = 1 // note\n", b"x = 1\n", SlashSlash).is_some());
        // A dropped doc trips.
        assert!(
            would_drop_comments(b"/// d\ndef f() = 1\n", b"def f() = 1\n", SlashSlash).is_some()
        );
        // Equal counts do NOT trip (a faithful reprint).
        assert!(would_drop_comments(b"// c\nx = 1\n", b"// c\nx = 1\n", SlashSlash).is_none());
        // An INCREASE does not trip (e.g. a comment that reattaches to two lines — never a loss).
        assert!(would_drop_comments(b"x = 1\n", b"// added\nx = 1\n", SlashSlash).is_none());
        // S-EXPR: a dropped `;` comment trips (the v-lsp bug — an ML-only `//` count would miss it).
        assert!(
            would_drop_comments(b"(m ; keep\n (def (f) 1))", b"(m (def (f) 1))", Semicolon)
                .is_some(),
            "a dropped s-expr `;` must trip the guard"
        );
        // But under ML lexis the same `;`-drop does NOT trip (a `;` isn't an ML comment) — so the guard
        // is surface-correct, not blanket.
        assert!(would_drop_comments(b"(m ; c\n x)", b"(m x)", SlashSlash).is_none());
    }

    #[test]
    fn comment_counts_is_total_on_arbitrary_input() {
        // The guard scans RAW bytes (possibly a malformed/partial file) — it must never panic, at any
        // input: unterminated strings, lone `#`/`\`/`/` at EOL, non-ASCII, embedded NULs. A tiny
        // deterministic PRNG (SplitMix64, the crate house style) fuzzes over a comment-relevant alphabet.
        struct R(u64);
        impl R {
            fn next(&mut self) -> u64 {
                self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
                let mut z = self.0;
                z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
                z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
                z ^ (z >> 31)
            }
        }
        // Bytes that exercise every branch of the scanner: quotes, escapes, slashes, `#\`, newlines,
        // a multi-byte char, and ordinary text.
        let alphabet = ['/', '"', '\\', '#', '\n', ' ', 'a', '1', 'é', '\0'];
        let mut r = R(0xC0FFEE);
        for _ in 0..4000 {
            let len = (r.next() % 40) as usize;
            let s: String = (0..len)
                .map(|_| alphabet[(r.next() as usize) % alphabet.len()])
                .collect();
            // The ONLY assertion is that it returns (does not panic); the counts themselves are exercised
            // by the hand-written cases above. Totality is the property under test — for every lexis.
            let _ = comment_counts(&s, CommentLexis::SlashSlash);
            let _ = comment_counts(&s, CommentLexis::Semicolon);
            let _ = comment_counts(&s, CommentLexis::None);
        }
    }

    #[test]
    fn guard_never_false_trips_on_a_comment_preserving_reprint() {
        // SOUNDNESS of the guard's no-false-trip property: for a program whose comments all survive an
        // fmt reprint (leading `///`/`//` on their own lines — the positions the reader DOES preserve),
        // `would_drop_comments(src, fmt(src))` must be `None` (no refusal). If this regressed, the guard
        // would start refusing legitimate formats. (The genuinely-lost trailing-inline case is tested to
        // TRIP in the integration tests; here we pin that the safe case does NOT trip.)
        for src in [
            "/// a doc\ndef f() = 1",
            "// a comment\ndef g() = 2",
            "/// doc\ndef h() =\n  // body note\n  3",
            "def i() = 4\n// between\ndef j() = 5",
        ] {
            let printed = match convert::convert_with(
                src.as_bytes(),
                Format::Ml,
                Format::Ml,
                Options::default(),
            ) {
                Ok(mut b) => {
                    if b.last() != Some(&b'\n') {
                        b.push(b'\n');
                    }
                    b
                }
                Err(e) => panic!("reprint {src:?}: {e}"),
            };
            assert!(
                would_drop_comments(src.as_bytes(), &printed, CommentLexis::SlashSlash).is_none(),
                "guard must NOT trip on a comment-preserving reprint of {src:?} → {}",
                String::from_utf8_lossy(&printed)
            );
        }
    }

    /// A fresh temp dir unique to `tag` (the caller populates + removes it).
    fn fmt_tmp(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("cdz-fmt-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    /// Build `FmtArgs` for a single file with the given write mode (the clap defaults filled in).
    fn fmt_args(files: Vec<String>, check: bool, diff: bool, stdout: bool) -> FmtArgs {
        FmtArgs {
            files,
            from: None,
            width: Options::default().width,
            check,
            diff,
            stdout,
        }
    }

    #[test]
    fn fmt_surface_normalizes_a_multi_form_sexp_file() {
        // A spec/semantics/*.sexp corpus file has MULTIPLE top-level forms; the single-form
        // `convert::read` inside `convert_with` fails with "trailing input" on the 2nd form. `fmt_surface`
        // falls back to the multi-form read (`sexpr::read_all` -> synthetic `(do …)`) + `print_pretty_program`
        // (flush-left top level, no `(do)` wrapper), so a multi-form .sexp fmt-normalizes + round-trips.
        let multi = b"(def (a) 1)\n(def (b) 2)\n";
        let out = fmt_surface(multi, Format::Sexpr, Options::default())
            .expect("multi-form .sexp formats");
        let out_s = String::from_utf8(out).unwrap();
        // Flush-left top-level forms, no `(do)` wrapper, blank-line-separated (the seq-256 program layout).
        assert_eq!(out_s, "(def (a) 1)\n\n(def (b) 2)");
        // Idempotent (the fmt file path re-adds the trailing newline before re-reading).
        let out2 = fmt_surface(
            format!("{out_s}\n").as_bytes(),
            Format::Sexpr,
            Options::default(),
        )
        .expect("re-format is idempotent");
        assert_eq!(String::from_utf8(out2).unwrap(), out_s);
        // A single-form .sexp is UNAFFECTED — it takes the `convert_with` path unchanged.
        let single = fmt_surface(b"(def (a) 1)\n", Format::Sexpr, Options::default())
            .expect("single-form .sexp");
        assert_eq!(String::from_utf8(single).unwrap(), "(def (a) 1)");
        // A `;` comment in a multi-form file (the corpus's section headers) is PRESERVED, not dropped —
        // the fmt path must not silently lose comments (comment-preservation seq-285 covers the surface).
        let commented = fmt_surface(
            b"; header\n(def (a) 1)\n(def (b) 2)\n",
            Format::Sexpr,
            Options::default(),
        )
        .expect("commented multi-form .sexp");
        let commented_s = String::from_utf8(commented).unwrap();
        assert!(
            commented_s.contains("; header"),
            "the ; comment must survive fmt, got: {commented_s:?}"
        );

        // The false-positive that refused EVERY doc-commented `spec/semantics/*.sexp`: a multi-line
        // `(doc "…; …")` string whose CONTINUATION line carries a `;`. The reader preserves the real
        // `; header` comment, so the guard must NOT refuse. And (seq-282 multi-line comment preservation)
        // the pretty printer keeps a multi-line string MULTI-LINE (its `\n` is emitted as a REAL newline,
        // not the `\n` escape), byte-exact — a `;` on a continuation line is string CONTENT, never a
        // comment. Pin: the real comment survives, the multi-line string stays multi-line byte-exact, and
        // `would_drop_comments(src, fmt(src))` is `None`.
        let src =
            b"; header\n(case \"x\"\n  (doc \"one; still string\n        two; also string\"))\n";
        let formatted = {
            let mut b = fmt_surface(src, Format::Sexpr, Options::default())
                .expect("multi-line doc-string .sexp formats");
            b.push(b'\n');
            b
        };
        let formatted_s = String::from_utf8_lossy(&formatted);
        assert!(
            formatted_s.contains("; header"),
            "the ; comment must survive fmt over a multi-line doc string, got: {formatted_s:?}"
        );
        assert!(
            formatted_s.contains("one; still string\n        two; also string"),
            "the multi-line doc string stays multi-line byte-exact (real newline, ; is content), got: {formatted_s:?}"
        );
        assert!(
            !formatted_s.contains("one; still string\\n"),
            "the multi-line string must NOT be collapsed to a `\\n` escape (seq-282), got: {formatted_s:?}"
        );
        assert!(
            would_drop_comments(src, &formatted, CommentLexis::Semicolon).is_none(),
            "the guard must NOT falsely refuse a multi-line doc-string .sexp reprint: {formatted_s:?}"
        );
    }

    #[test]
    fn emits_to_stdout_honors_check_and_diff_on_stdin() {
        // The disposition predicate shared by `run_fmt`/`run_normalize`, pinning the `fmt - --check`
        // false-pass fix (v-cdz-tooling report): a stdin `--check`/`--diff` must NOT emit to stdout —
        // it must fall through to the inspection branches so an unformatted pipe exits non-zero, instead
        // of the old behavior that printed + returned success. Table over (from_stdin, stdout, check, diff):

        // --stdout ALWAYS emits (it's the explicit "give me the formatted program" mode). It's rejected
        // up front as mutually exclusive with check/diff, so those are false alongside it here.
        assert!(
            emits_to_stdout(false, true, false, false),
            "--stdout on a file emits"
        );
        assert!(
            emits_to_stdout(true, true, false, false),
            "--stdout on stdin emits"
        );

        // Plain stdin (no mode flag) emits to stdout — the implicit "no file to edit" disposition.
        assert!(
            emits_to_stdout(true, false, false, false),
            "bare stdin emits to stdout"
        );

        // THE FIX: stdin + --check / stdin + --diff must NOT emit — they inspect (the regression case
        // that used to print + exit 0 on unformatted input).
        assert!(
            !emits_to_stdout(true, false, true, false),
            "stdin + --check does NOT emit (inspects)"
        );
        assert!(
            !emits_to_stdout(true, false, false, true),
            "stdin + --diff does NOT emit (inspects)"
        );

        // A FILE never auto-emits (it edits in place / checks / diffs against the path), regardless of
        // check/diff — only explicit --stdout redirects a file to stdout.
        assert!(
            !emits_to_stdout(false, false, false, false),
            "a file edits in place, no stdout"
        );
        assert!(
            !emits_to_stdout(false, false, true, false),
            "a file + --check inspects, no stdout"
        );
        assert!(
            !emits_to_stdout(false, false, false, true),
            "a file + --diff inspects, no stdout"
        );
    }

    #[test]
    fn with_files_replaces_the_target_list_and_preserves_mode_flags() {
        // `FmtArgs::with_files` is the API `cdz` uses to hand a resolved project file set to `fmt` while
        // keeping the parsed flags (the v-cdz-tooling no-arg/project-fmt coordination). Only the file list
        // changes; from/width/check/diff/stdout carry over.
        let base = FmtArgs {
            files: vec!["-".to_string()], // e.g. the no-arg stdin default cdz would override
            from: Some(Fmt::Ml),
            width: 100,
            check: true,
            diff: false,
            stdout: false,
        };
        let resolved = base.with_files(vec!["a.cdz".to_string(), "b.cdz".to_string()]);
        assert_eq!(
            resolved.files,
            vec!["a.cdz".to_string(), "b.cdz".to_string()]
        );
        assert!(matches!(resolved.from, Some(Fmt::Ml)), "from preserved");
        assert_eq!(resolved.width, 100, "width preserved");
        assert!(resolved.check, "check preserved");
        assert!(!resolved.diff && !resolved.stdout, "diff/stdout preserved");
    }

    #[test]
    fn files_getter_reads_the_parsed_positionals_verbatim_the_read_side_of_with_files() {
        // `FmtArgs::files()` is the read side `cdz` uses to CLASSIFY an invocation before deciding
        // project-mode (empty = stdin/project, lone `-` = explicit stdin, single dir/Project.cdz =
        // project target). It returns the positionals exactly as parsed — no recursion, no stdin
        // resolution — and round-trips with `with_files` (write then read yields the same list).
        let base = FmtArgs {
            files: vec![],
            from: None,
            width: 80,
            check: false,
            diff: false,
            stdout: false,
        };
        // No positionals — the "stdin or enter project-mode" case cdz keys off.
        assert!(
            base.files().is_empty(),
            "empty positionals read back as empty"
        );
        // A lone `-` stays verbatim (the explicit stdin marker, NOT a project sweep).
        let stdin = base.with_files(vec!["-".to_string()]);
        assert_eq!(
            stdin.files(),
            ["-".to_string()],
            "lone `-` preserved verbatim"
        );
        // A single directory-looking arg is handed back unmodified — cdz, not fmt, classifies it.
        let dir = FmtArgs {
            files: vec![],
            from: None,
            width: 80,
            check: false,
            diff: false,
            stdout: false,
        }
        .with_files(vec!["src".to_string()]);
        assert_eq!(
            dir.files(),
            ["src".to_string()],
            "a lone dir arg is read verbatim"
        );
        // Round-trip: with_files(write) then files(read) yields the same list.
        let list = vec!["a.cdz".to_string(), "b.sexp".to_string()];
        let round = dir.with_files(list.clone());
        assert_eq!(
            round.files(),
            list.as_slice(),
            "with_files → files() round-trips"
        );
    }

    #[test]
    fn fmt_in_place_canonicalizes_and_is_idempotent() {
        let dir = fmt_tmp("inplace");
        let file = dir.join("m.sexp");
        // A non-canonical program (extra whitespace + blank lines) — `fmt` must reflow it.
        std::fs::write(&file, "(module   m\n\n  (def (main)   1) (export main))").unwrap();
        let path = file.to_string_lossy().into_owned();

        // Default mode edits in place and reports success (Ok(true) — not a `--check` verdict).
        assert!(run_fmt(&fmt_args(vec![path.clone()], false, false, false)).unwrap());
        let after = std::fs::read_to_string(&file).unwrap();
        assert_eq!(
            after, "(module m\n  (def (main) 1)\n\n  (export main))\n",
            "in-place fmt canonicalizes to the printer's form (a top-level module lays out VERTICALLY — \
             each definition on its own line, blank-separated — the readable canonical style) + a trailing newline"
        );
        // Idempotent: a second run leaves the bytes byte-identical.
        assert!(run_fmt(&fmt_args(vec![path], false, false, false)).unwrap());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), after);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn fmt_check_reports_but_never_writes() {
        let dir = fmt_tmp("check");
        let file = dir.join("m.sexp");
        let original = "(module   m (def (main) 1) (export main))"; // non-canonical, no trailing \n
        std::fs::write(&file, original).unwrap();
        let path = file.to_string_lossy().into_owned();

        // `--check` on an unformatted file returns Ok(false) (→ non-zero exit) and does NOT touch it.
        assert!(!run_fmt(&fmt_args(vec![path.clone()], true, false, false)).unwrap());
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            original,
            "--check must not modify the file"
        );

        // Once canonical, `--check` returns Ok(true).
        run_fmt(&fmt_args(vec![path.clone()], false, false, false)).unwrap();
        assert!(run_fmt(&fmt_args(vec![path], true, false, false)).unwrap());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn fmt_declines_a_file_that_does_not_parse_and_never_writes_it() {
        // The safety guarantee: a program that only parses with recovered errors is REJECTED, and the
        // file is left exactly as it was — `fmt` never rewrites a broken file into a patched-up tree.
        let dir = fmt_tmp("broken");
        let file = dir.join("bad.sexp");
        let broken = "(module m (def (main) 1"; // unbalanced — a hard parse error
        std::fs::write(&file, broken).unwrap();
        let path = file.to_string_lossy().into_owned();

        // A single-file run surfaces the parse error as a hard `Err`.
        assert!(run_fmt(&fmt_args(vec![path], false, false, false)).is_err());
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            broken,
            "a file that fails to parse must be left untouched"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn fmt_write_modes_are_mutually_exclusive() {
        let dir = fmt_tmp("exclusive");
        let file = dir.join("m.sexp");
        std::fs::write(&file, "(module m (def (main) 1) (export main))").unwrap();
        let path = file.to_string_lossy().into_owned();
        // Any two of --check/--diff/--stdout together is rejected before any file work.
        assert!(run_fmt(&fmt_args(vec![path.clone()], true, true, false)).is_err());
        assert!(run_fmt(&fmt_args(vec![path.clone()], true, false, true)).is_err());
        assert!(run_fmt(&fmt_args(vec![path], false, true, true)).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn collect_targets_recurses_a_dir_by_code_extension_and_excludes_docs_and_data() {
        // The directory-sweep path (`cdz fmt <dir>` / a codemod over a tree) recurses subdirs and
        // includes ONLY code surfaces (.cdz/.ml/.sexp/.sexpr/.bin/.cdzb), excluding markdown/json/toml/
        // cedar (documents + data, not code to bulk-format) and unrelated files. Load-bearing now that
        // `cdz` will feed project-resolved trees to fmt (FmtArgs::with_files). Build a tree + assert the
        // collected set.
        let dir = fmt_tmp("collect");
        let sub = dir.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        // Code surfaces (included) — at the top level and one level down.
        std::fs::write(dir.join("a.cdz"), "def f() = 1\n").unwrap();
        std::fs::write(dir.join("b.sexp"), "(def (g) 2)\n").unwrap();
        std::fs::write(sub.join("c.ml"), "def h() = 3\n").unwrap();
        // Documents + data + noise (all EXCLUDED from a sweep).
        std::fs::write(dir.join("README.md"), "# doc\n").unwrap();
        std::fs::write(dir.join("data.json"), "{}\n").unwrap();
        std::fs::write(dir.join("Config.toml"), "x = 1\n").unwrap();
        std::fs::write(
            dir.join("policy.cedar"),
            "permit(principal,action,resource);\n",
        )
        .unwrap();
        std::fs::write(dir.join(".gitignore"), "*.tmp\n").unwrap();

        let specs = collect_targets(&[dir.to_string_lossy().into_owned()], None).unwrap();
        // Exactly the three code files, each inferring its own surface; sorted by path.
        let names: Vec<String> = specs
            .iter()
            .map(|s| {
                std::path::Path::new(s.path.as_ref().unwrap())
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(
            names,
            vec!["a.cdz", "b.sexp", "c.ml"],
            "code files only, recursed, sorted"
        );
        // Surfaces inferred from each extension (a dir sweep never forces one format).
        let by_name = |n: &str| {
            specs
                .iter()
                .find(|s| s.path.as_ref().unwrap().ends_with(n))
                .unwrap()
                .format
        };
        assert!(matches!(by_name("a.cdz"), Format::Ml));
        assert!(matches!(by_name("b.sexp"), Format::Sexpr));
        assert!(matches!(by_name("c.ml"), Format::Ml));
        // An EXPLICITLY-named doc/data file still works (collect_targets honors any recognized ext) —
        // only the SWEEP excludes them.
        let md = dir.join("README.md").to_string_lossy().into_owned();
        let explicit = collect_targets(&[md], None).unwrap();
        assert_eq!(explicit.len(), 1, "an explicitly-named .md is honored");
        assert!(matches!(explicit[0].format, Format::Markdown));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn resolve_to_prefers_flag_then_extension_then_sexpr() {
        // Explicit `--to` always wins (even when the file extension says otherwise).
        assert!(matches!(
            resolve_to(Some(Fmt::Binary), Some("out.ml")),
            Format::Binary
        ));
        // No `--to`: infer from the output file's extension.
        assert!(matches!(resolve_to(None, Some("out.ml")), Format::Ml));
        assert!(matches!(resolve_to(None, Some("out.sexp")), Format::Sexpr));
        assert!(matches!(resolve_to(None, Some("out.json")), Format::Json));
        // No `--to`, and the destination is stdin/`-` or an unknown/absent extension: default to sexpr.
        // (Unlike `resolve_from`, output resolution never errors — it always has the sexpr fallback.)
        assert!(matches!(resolve_to(None, Some("-")), Format::Sexpr));
        assert!(matches!(resolve_to(None, None), Format::Sexpr));
        assert!(matches!(
            resolve_to(None, Some("out.unknownext")),
            Format::Sexpr
        ));
    }

    #[test]
    fn extension_inference_is_case_insensitive() {
        // `Format::from_extension` lowercases the extension, so `.ML`/`.SEXP` infer like their lowercase
        // forms — used by both resolve_from and resolve_to.
        assert!(matches!(resolve_to(None, Some("OUT.ML")), Format::Ml));
        assert!(matches!(resolve_to(None, Some("Out.Sexp")), Format::Sexpr));
        assert!(resolve_from(None, Some("/tmp/cdz-nope.SEXP")).is_ok());
    }

    #[test]
    fn fmt_to_format_is_a_total_one_to_one_mapping() {
        // Every surface `Fmt` maps to its like-named `Format` — a closed correspondence, so a new
        // surface flag can't silently route to the wrong backend. (The match is exhaustive, so a new
        // `Fmt` variant is a compile error in `From<Fmt>` until mapped; this pins the pairing.)
        let pairs = [
            (Fmt::Binary, Format::Binary),
            (Fmt::Sexpr, Format::Sexpr),
            (Fmt::Ml, Format::Ml),
            (Fmt::Markdown, Format::Markdown),
            (Fmt::Json, Format::Json),
            (Fmt::Toml, Format::Toml),
            (Fmt::Cedar, Format::Cedar),
            (Fmt::Debug, Format::Debug),
            (Fmt::Flat, Format::Flat),
        ];
        for (f, expected) in pairs {
            assert_eq!(
                std::mem::discriminant(&Format::from(f)),
                std::mem::discriminant(&expected),
                "Fmt maps to its like-named Format"
            );
        }
    }

    #[test]
    fn ensure_trailing_newline_is_idempotent_and_adds_exactly_one() {
        // Adds a newline when missing; leaves an already-terminated string alone (no double newline);
        // idempotent. A file writer relies on "exactly one trailing newline".
        assert_eq!(ensure_trailing_newline("abc"), "abc\n");
        assert_eq!(ensure_trailing_newline("abc\n"), "abc\n");
        // Already-terminated input is returned unchanged — a trailing blank line is NOT collapsed here
        // (that is the printer's job); this only guarantees the string ENDS in a newline.
        assert_eq!(ensure_trailing_newline("abc\n\n"), "abc\n\n");
        assert_eq!(ensure_trailing_newline(""), "\n");
        // Idempotence: applying twice equals applying once.
        let once = ensure_trailing_newline("x");
        assert_eq!(ensure_trailing_newline(&once), once);
    }

    #[test]
    fn label_and_with_path_name_the_target_or_stdin() {
        // `label` names the file, or `(stdin)` when there is none; `with_path` prefixes a message with it
        // so a multi-file run points at the culprit.
        assert_eq!(label(&Some("a/b.sexp".to_string())), "a/b.sexp");
        assert_eq!(label(&None), "(stdin)");
        assert_eq!(with_path(&Some("f.ml".to_string()), "boom"), "f.ml: boom");
        assert_eq!(with_path(&None, "boom"), "(stdin): boom");
    }

    #[test]
    fn doc_bytes_projects_a_program_to_a_binary_doc_module() {
        // `cdz doc`'s core: an ML program → its public doc surface as a canonical-binary doc-module.
        // The bytes are `cdzast\x00\x01` and decode to a (doc-module …) carrying the exported item.
        let src = b"/// Doubles n.\ndef double(n) = n\nexport { double }";
        let out = doc_bytes(src, Format::Ml, Format::Binary, "mymod", 100).expect("doc_bytes");
        assert_eq!(&out[..8], b"cdzast\x00\x01", "doc-AST is canonical binary");
        let arenas = crate::codec::decode(&out).expect("doc-AST decodes");
        let mod_args = arenas
            .as_form(arenas.root, "doc-module")
            .expect("root is a doc-module");
        assert_eq!(arenas.as_str(mod_args[0]), Some("mymod"), "module name");
        // some doc-item names `double`
        let has_double = mod_args.iter().any(|&c| {
            arenas.as_form(c, "doc-item").is_some_and(|item| {
                item.iter().any(|&f| {
                    arenas.as_form(f, "name").and_then(|a| arenas.as_str(a[0])) == Some("double")
                })
            })
        });
        assert!(has_double, "the exported `double` is a doc-item");
    }

    #[test]
    fn doc_bytes_can_emit_a_text_surface_for_inspection() {
        // `--to sexpr` renders the doc-module as readable s-expr (not binary) — the inspection path.
        let src = b"def f(x) = x\nexport { f }";
        let out = doc_bytes(src, Format::Ml, Format::Sexpr, "m", 100).expect("doc_bytes sexpr");
        let text = String::from_utf8(out).expect("sexpr is utf-8");
        assert!(text.contains("doc-module"), "renders the doc-module head");
        assert!(text.contains("doc-item"), "renders a doc-item");
        assert!(text.contains("\"f\""), "names the exported item");
    }

    #[test]
    fn module_stem_defaults_from_file_or_falls_back() {
        assert_eq!(module_stem(Some("src/lib.cdz")), "lib");
        assert_eq!(module_stem(Some("a/b/thing.sexp")), "thing");
        assert_eq!(module_stem(Some("-")), "module", "stdin falls back");
        assert_eq!(module_stem(None), "module", "no file falls back");
    }
}
