//! `cdz` — the unified Cadenza command-line tool.
//!
//! ONE binary over the sub-libraries: `cadenza-syntax` (front-end — convert + the structural codemod)
//! and `rcdzc` (the compiler — compile/emit + the sidecar query engine). The syntax and compiler
//! command surfaces each live in their library's `cli` module (one implementation, shared with the
//! standalone `cdz-syntax`/`rcdzc` bins); this bin FLATTENS both into one subcommand tree and adds the
//! two commands that only a single process holding BOTH libraries can offer:
//!
//!   cdz type NAME  FILE     — the solved type of a definition (a compiler query), rendered.
//!   cdz uses NAME  FILE     — every source location that references a definition/type, as
//!                             `file:line:col`.
//!
//! Why those two are here and not in `cdz-syntax`: `type`/`uses` need the COMPILER (`rcdzc::type_of`,
//! resolution) AND the front-end's `SpanTable` in ONE process. The cross-process CLI throws the span
//! table away between `cdz-syntax` and `rcdzc`, so the compiler could only ever report node IDS; here
//! we parse keeping the spans, drive the compiler's sidecar query, and map the result ids back to
//! source `file:line:col`. Because both libraries share the byte-identical `StructId` space, this also
//! unblocks combining a structural selector with a semantic query (a later increment).
//!
//! `cdz-run` stays a SEPARATE bin — it pulls in wasmtime + the runtime store, a different concern.

use clap::{Parser, Subcommand};
use std::process::ExitCode;

use cadenza_syntax::cli as syntax_cli;
use rcdzc::cli as compiler_cli;

/// The unified tool. The name reported in tool-level diagnostics is `cdz`.
const PROG: &str = "cdz";

#[derive(Parser)]
#[command(
    name = "cdz",
    about = "The Cadenza toolchain: convert, query, compile, and inspect a program — one tool.",
    long_about = "cdz unifies the front-end (convert + structural codemod) and the compiler \
                  (compile/emit + semantic queries) over one program. `type` and `uses` are \
                  span-mapped compiler queries only a single process holding both can answer."
)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    // ── front-end (cadenza-syntax) ──────────────────────────────────────────────────────────────
    /// Convert a program between surfaces (binary/sexpr/ml + the debug/flat views).
    Convert(syntax_cli::ConvertArgs),
    /// Structurally search a program for a PATTERN (the codemod query).
    Query(syntax_cli::QueryArgs),
    /// Structurally rewrite a program: replace every PATTERN match with TEMPLATE, validated.
    Rewrite(syntax_cli::RewriteArgs),
    /// Structurally diff two programs: report which SUBTREES changed.
    Diff(syntax_cli::DiffArgs),
    /// Flag structural anti-patterns from a lint-rule set.
    Lint(syntax_cli::LintArgs),
    /// Find duplicated subtrees (clones) within/across programs.
    Clones(syntax_cli::ClonesArgs),

    // ── compiler (rcdzc) ────────────────────────────────────────────────────────────────────────
    /// Compile binary-AST artifacts to one or more backend targets (wasm/rust). The `rcdzc` surface.
    Compile(compiler_cli::CompileArgs),

    // ── semantic queries — the in-process win (both libraries + spans) ──────────────────────────
    /// The solved type of a definition NAME in FILE, rendered (a compiler query over the type column).
    Type(TypeArgs),
    /// The inferred type of the node at a source BYTE OFFSET in FILE — a "type at cursor" (hover).
    TypeAt(TypeAtArgs),
    /// Every source location that references the definition/type NAME in FILE, as `file:line:col`.
    Uses(UsesArgs),
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        // Front-end commands defer to the syntax CLI, reconstructing its command enum (its arg structs
        // are re-exported, so `cdz convert …` and `cdz-syntax convert …` run the SAME code).
        Cmd::Convert(a) => syntax_cli::run(syntax_cli::Cmd::Convert(a), PROG),
        Cmd::Query(a) => syntax_cli::run(syntax_cli::Cmd::Query(a), PROG),
        Cmd::Rewrite(a) => syntax_cli::run(syntax_cli::Cmd::Rewrite(a), PROG),
        Cmd::Diff(a) => syntax_cli::run(syntax_cli::Cmd::Diff(a), PROG),
        Cmd::Lint(a) => syntax_cli::run(syntax_cli::Cmd::Lint(a), PROG),
        Cmd::Clones(a) => syntax_cli::run(syntax_cli::Cmd::Clones(a), PROG),
        // The compiler command defers to the rcdzc CLI.
        Cmd::Compile(a) => compiler_cli::run(a, PROG),
        // The span-mapped semantic queries live here (they need both libraries in one process).
        Cmd::Type(a) => run_type(&a),
        Cmd::TypeAt(a) => run_type_at(&a),
        Cmd::Uses(a) => run_uses(&a),
    }
}

// ── the semantic queries ─────────────────────────────────────────────────────────────────────────

#[derive(clap::Args)]
struct TypeArgs {
    /// The definition name to type.
    name: String,
    /// The program file (`.cdz`/`.ml` → ml, `.sexp`/`.sexpr` → sexpr).
    file: String,
}

#[derive(clap::Args)]
struct UsesArgs {
    /// The definition or type name to find references to.
    name: String,
    /// The program file (`.cdz`/`.ml` → ml, `.sexp`/`.sexpr` → sexpr).
    file: String,
}

#[derive(clap::Args)]
struct TypeAtArgs {
    /// The program file (`.cdz`/`.ml` → ml, `.sexp`/`.sexpr` → sexpr).
    file: String,
    /// The source BYTE OFFSET to type — the cursor position (0-based, UTF-8 bytes).
    offset: usize,
}

/// `cdz type NAME FILE` — parse in-process, drive the compiler's `TypeOf` sidecar query, print the
/// rendered type. A query is a pure, total fact read: it answers even for a program that would not
/// compile (`DESIGN-sidecar-api.md`).
fn run_type(args: &TypeArgs) -> ExitCode {
    let (source, arenas) = match load_program(&args.file) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{PROG}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let _ = source; // type output carries no span
    let out = run_sidecar(
        &arenas,
        rcdzc::Request::Query(rcdzc::sidecar::Query::TypeOf {
            name: args.name.clone(),
        }),
    );
    match out.artifact(rcdzc::sidecar::KIND_TYPE_INFO) {
        Some(bytes) => {
            println!("{}", String::from_utf8_lossy(bytes));
            ExitCode::SUCCESS
        }
        None => {
            report_errors(&out);
            ExitCode::FAILURE
        }
    }
}

/// `cdz type-at FILE OFFSET` — the "type at cursor" query. Resolves the source byte offset to the
/// INNERMOST node id (via the span table this process kept — `SpanTable::node_at_offset`, the SAME
/// resolution the browser IDE uses), drives the compiler's `TypeAt { node }` query, and prints the
/// rendered type with the node's source `line:col-line:col` range. The offset→node split keeps the
/// compiler span-free while the type is a node-identity query (`DESIGN-sidecar-api.md`).
fn run_type_at(args: &TypeAtArgs) -> ExitCode {
    let (source, arenas, spans) = match load_program_spanned(&args.file) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{PROG}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let Some(node) = spans.node_at_offset(args.offset) else {
        eprintln!(
            "{PROG}: no node at byte offset {} in {}",
            args.offset, args.file
        );
        return ExitCode::FAILURE;
    };
    let out = run_sidecar(
        &arenas,
        rcdzc::Request::Query(rcdzc::sidecar::Query::TypeAt { node: node.0 }),
    );
    let Some(bytes) = out.artifact(rcdzc::sidecar::KIND_TYPE_AT) else {
        report_errors(&out);
        return ExitCode::FAILURE;
    };
    let ty = String::from_utf8_lossy(bytes);
    // Show the node's source range so the caller can highlight exactly the sub-expression typed.
    match spans.get(node) {
        Some(span) => {
            let (l0, c0) = cadenza_syntax::query::driver::line_col(&source, span.start);
            let (l1, c1) = cadenza_syntax::query::driver::line_col(&source, span.end);
            println!("{ty} @ {}:{l0}:{c0}-{l1}:{c1}", args.file);
        }
        None => println!("{ty}"),
    }
    ExitCode::SUCCESS
}

/// `cdz uses NAME FILE` — drive the compiler's `UsesOf` query (node ids), then MAP each id to a source
/// `file:line:col` via the SpanTable this process kept. This is the payoff of holding both libraries in
/// one process: the cross-process CLI could only print node ids.
fn run_uses(args: &UsesArgs) -> ExitCode {
    let (source, arenas, spans) = match load_program_spanned(&args.file) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{PROG}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let out = run_sidecar(
        &arenas,
        rcdzc::Request::Query(rcdzc::sidecar::Query::UsesOf {
            name: args.name.clone(),
        }),
    );
    let Some(bytes) = out.artifact(rcdzc::sidecar::KIND_USES) else {
        report_errors(&out);
        return ExitCode::FAILURE;
    };
    let text = String::from_utf8_lossy(bytes);
    let ids: Vec<u32> = text.lines().filter_map(|l| l.trim().parse().ok()).collect();
    if ids.is_empty() {
        eprintln!("{PROG}: no references to `{}` in {}", args.name, args.file);
        return ExitCode::SUCCESS;
    }
    for id in ids {
        match spans.get(cadenza_syntax::StructId(id)) {
            Some(span) => {
                let (line, col) = cadenza_syntax::query::driver::line_col(&source, span.start);
                println!("{}:{line}:{col}", args.file);
            }
            // A referencing occurrence with no recorded span (should not happen for a user node) still
            // reports the raw id rather than dropping it silently.
            None => println!("{}:node {id}", args.file),
        }
    }
    ExitCode::SUCCESS
}

// ── shared plumbing ────────────────────────────────────────────────────────────────────────────────

/// Compile `arenas` under a single sidecar request, on the compiler's stack-guarded worker thread.
fn run_sidecar(arenas: &cadenza_syntax::Arenas, request: rcdzc::Request) -> rcdzc::CompileOutput {
    let ast = cadenza_syntax::codec::encode(arenas);
    let sidecar = rcdzc::sidecar::encode(&[request]);
    let inputs = vec![
        rcdzc::Artifact::new(rcdzc::Artifact::KIND_AST, "main", ast),
        rcdzc::Artifact::new(rcdzc::sidecar::KIND_SIDECAR, "drive", sidecar),
    ];
    // No emit target: a query-only run (`DESIGN-sidecar-api.md` query-only mode). The stack guard keeps
    // pathologically deep input a decline, not a crash.
    rcdzc::run_with_compiler_stack(|| rcdzc::compile(&inputs, &[]))
}

/// Report a compile output's error diagnostics to stderr (used when a query produced no artifact —
/// which for a TOTAL query means the AST itself failed to decode/compile at the entry).
fn report_errors(out: &rcdzc::CompileOutput) {
    for d in &out.diagnostics {
        if d.severity == rcdzc::Severity::Error {
            match &d.code {
                Some(code) => eprintln!("{PROG}: error [{code}]: {}", d.message),
                None => eprintln!("{PROG}: error: {}", d.message),
            }
        }
    }
}

/// Read + parse a program file into its arenas (no spans). Format inferred from the extension.
fn load_program(file: &str) -> Result<(String, cadenza_syntax::Arenas), String> {
    let (source, arenas, _) = load_program_spanned(file)?;
    Ok((source, arenas))
}

/// Read + parse a program file into its arenas AND span table. Format inferred from the extension
/// (`.cdz`/`.ml` → ml, `.sexp`/`.sexpr` → sexpr). The parse is the WHOLE-program form (`read_all_*`),
/// matching how the gate normalizes a corpus program to an export shape.
fn load_program_spanned(
    file: &str,
) -> Result<
    (
        String,
        cadenza_syntax::Arenas,
        cadenza_syntax::spans::SpanTable,
    ),
    String,
> {
    let source = std::fs::read_to_string(file).map_err(|e| format!("reading {file}: {e}"))?;
    let is_ml = file.ends_with(".cdz") || file.ends_with(".ml");
    if is_ml {
        let parsed = cadenza_syntax::parser::read_ml(&source);
        for e in &parsed.errors {
            eprintln!("{PROG}: {file}: parse warning: {e:?}");
        }
        Ok((source, parsed.arenas, parsed.spans))
    } else {
        // Mirror the driver's root convention (`query::driver::load`): a SINGLE top-level form stays
        // BARE (so a lone `(module …)`/`(def …)` is the root the compiler scans), and only MULTIPLE
        // forms wrap in a synthetic `(do …)`. `read_spanned` succeeds iff there's exactly one form
        // (it errors on trailing input); fall back to `read_all_spanned` for several. Using
        // `read_all_spanned` unconditionally would wrap a lone `(module …)` in `(do …)`, and the
        // compiler's top-level scan would then see the module as one opaque item and find no defs.
        let (arenas, spans) = match cadenza_syntax::sexpr::read_spanned(&source) {
            Ok(pair) => pair,
            Err(_) => cadenza_syntax::sexpr::read_all_spanned(&source)
                .map_err(|e| format!("{file}: {}", e.0))?,
        };
        Ok((source, arenas, spans))
    }
}
