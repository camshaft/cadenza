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
//! source `file:line:col`.
//!
//! The same in-process co-location powers the COMBINED query `cdz query PATTERN --where 'type-of(x) =
//! T'`: the structural matcher (cadenza-syntax) finds shape matches and each match's binding carries
//! its `StructId`; the compiler (rcdzc, via a batch of `Query::TypeAt`) types those nodes; the filter
//! keeps only matches whose binding has the asked-for type. Shape ∧ meaning in one command — the thing
//! neither library can do alone, unblocked because they share the byte-identical `StructId` space.
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
        // A `--where` clause makes this a COMBINED structural+semantic query — `cdz` runs it (it needs
        // the compiler). Without `--where` it is the pure structural query, delegated unchanged.
        Cmd::Query(a) if a.where_.is_some() => run_query_where(&a),
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
    run_sidecar_many(arenas, &[request])
}

/// Drive a BATCH of sidecar requests over one program in a single compile. A request list is ordered
/// and the `Db`'s columns are shared/warm across the batch, so N `TypeAt` queries (one per match
/// binding, for `--where`) cost one `Db::load` + shared inference, not N separate compiles.
fn run_sidecar_many(
    arenas: &cadenza_syntax::Arenas,
    requests: &[rcdzc::Request],
) -> rcdzc::CompileOutput {
    let ast = cadenza_syntax::codec::encode(arenas);
    let sidecar = rcdzc::sidecar::encode(requests);
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

// ── the combined structural + semantic query (`cdz query … --where …`) ───────────────────────────

/// A `--where` predicate: keep a match iff the type of its binding VAR relates to TYPE by OP. Minimal
/// on purpose (the "don't invent syntax first" discipline) — one relation, `type-of(var) = type` or
/// `!= type` — enough for the motivating "match `(f ,x)` only where `x : Int64`" case, extensible later.
struct WherePredicate {
    /// The metavariable whose binding is typed (the `x` in `type-of(x)`).
    var: String,
    /// The expected rendered type (`Ty::render_name` form, e.g. `Int64`, `(-> Int64 Int64)`).
    ty: String,
    /// `true` for `=` (keep matches whose type equals `ty`), `false` for `!=`.
    equal: bool,
}

/// Parse `type-of(VAR) = TYPE` / `type-of(VAR) != TYPE`. Whitespace-insensitive around the tokens;
/// TYPE is taken verbatim (trimmed) so a compound type like `(-> Int64 Int64)` works. Returns a
/// message on a shape it doesn't recognize.
fn parse_where(src: &str) -> Result<WherePredicate, String> {
    let s = src.trim();
    let rest = s.strip_prefix("type-of(").ok_or_else(|| {
        format!("unsupported --where predicate `{src}` (expected `type-of(VAR) = TYPE` or `!=`)")
    })?;
    let (var, after) = rest
        .split_once(')')
        .ok_or_else(|| format!("--where: missing `)` after the variable in `{src}`"))?;
    let var = var.trim().trim_start_matches(',').trim().to_string();
    if var.is_empty() {
        return Err(format!("--where: empty variable in `{src}`"));
    }
    let after = after.trim();
    // `!=` before `=` so the longer operator wins.
    let (equal, ty) = if let Some(t) = after.strip_prefix("!=") {
        (false, t)
    } else if let Some(t) = after.strip_prefix('=') {
        (true, t)
    } else {
        return Err(format!(
            "--where: expected `=` or `!=` after `type-of({var})` in `{src}`"
        ));
    };
    let ty = ty.trim().to_string();
    if ty.is_empty() {
        return Err(format!("--where: empty type in `{src}`"));
    }
    Ok(WherePredicate { var, ty, equal })
}

/// `cdz query PATTERN --where 'type-of(x) = T'` — the combined query. Runs the structural search
/// (cadenza-syntax), then for each match reads the type of the `--where` variable's binding node from
/// the COMPILER (a batch of `Query::TypeAt`), keeping only matches whose binding's type relates to the
/// asked-for type. Shape ∧ meaning in one command. Prints the surviving matches like `cdz query`.
fn run_query_where(args: &syntax_cli::QueryArgs) -> ExitCode {
    use cadenza_syntax::query::{self, Pattern};

    let pred = match parse_where(args.where_.as_deref().unwrap_or("")) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{PROG}: {e}");
            return ExitCode::FAILURE;
        }
    };

    // The combined query is single-file for now (a compiler query is per-compilation-unit); a dir
    // sweep is a later fan-out. Require exactly one FILE input.
    let file = match args.files.as_slice() {
        [f] if f != "-" => f.clone(),
        _ => {
            eprintln!(
                "{PROG}: `query --where` needs exactly one FILE input (semantic query is per unit)"
            );
            return ExitCode::FAILURE;
        }
    };

    let (source, arenas, spans) = match load_program_spanned(&file) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{PROG}: {e}");
            return ExitCode::FAILURE;
        }
    };

    // Compile the structural pattern + any relational context (--inside/--has/…), then search.
    let pattern = match Pattern::compile(&args.pattern) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{PROG}: pattern: {e}");
            return ExitCode::FAILURE;
        }
    };
    let relq = match build_relational_query(args) {
        Ok(q) => q,
        Err(e) => {
            eprintln!("{PROG}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let tree = query::Tree::of(&arenas);
    let matches = query::search_with(&pattern, &relq, &tree, Some(&spans));

    // The node id of each match's `--where` binding (a match with no such binding, or whose binding is
    // not a single node, can't be typed — it's dropped). Dedup so each distinct node is typed once.
    let mut typed_nodes: Vec<u32> = Vec::new();
    let binding_node: Vec<Option<u32>> = matches
        .iter()
        .map(|m| {
            let id = m
                .bindings
                .get(&pred.var)
                .and_then(|t| t.origin())
                .map(|s| s.0);
            if let Some(n) = id
                && !typed_nodes.contains(&n)
            {
                typed_nodes.push(n);
            }
            id
        })
        .collect();

    if typed_nodes.is_empty() {
        // No match binds `var` to a typeable node — nothing can satisfy the predicate.
        if !args.count {
            // (silent: no matches)
        } else {
            println!("0");
        }
        return ExitCode::SUCCESS;
    }

    // ONE compile, a batch of TypeAt requests — the type column is shared/warm across the batch.
    let requests: Vec<rcdzc::Request> = typed_nodes
        .iter()
        .map(|&n| rcdzc::Request::Query(rcdzc::sidecar::Query::TypeAt { node: n }))
        .collect();
    let out = run_sidecar_many(&arenas, &requests);
    // node id → rendered type, read from the `type-at` artifacts (each names its node id).
    let mut node_ty: std::collections::HashMap<u32, String> = std::collections::HashMap::new();
    for art in &out.artifacts {
        if art.kind == rcdzc::sidecar::KIND_TYPE_AT
            && let Ok(n) = art.name.parse::<u32>()
        {
            node_ty.insert(n, String::from_utf8_lossy(&art.bytes).into_owned());
        }
    }

    // Keep matches whose binding's type relates to `pred.ty` by the operator.
    let kept: Vec<&query::Match> = matches
        .iter()
        .zip(&binding_node)
        .filter_map(|(m, node)| {
            let ty = node.and_then(|n| node_ty.get(&n))?;
            let hit = (ty == &pred.ty) == pred.equal;
            hit.then_some(m)
        })
        .collect();

    if args.count {
        println!("{}", kept.len());
        return ExitCode::SUCCESS;
    }
    for m in kept {
        let loc = match m.span {
            Some(s) => {
                let (l, c) = cadenza_syntax::query::driver::line_col(&source, s.start);
                format!("{file}:{l}:{c}")
            }
            None => file.clone(),
        };
        println!("{loc}: {}", m.node.to_sexpr());
        for (name, nodes) in m.bindings.iter() {
            let rendered = match nodes {
                [one] => one.to_sexpr(),
                many => format!(
                    "[{}]",
                    many.iter()
                        .map(|t| t.to_sexpr())
                        .collect::<Vec<_>>()
                        .join(" ")
                ),
            };
            println!("  ${name} = {rendered}");
        }
    }
    ExitCode::SUCCESS
}

/// Build the relational-context `Query` from the repeatable `--inside`/`--has`/`--not-inside`/
/// `--not-has` patterns (the same constraints the pure structural query supports).
fn build_relational_query(
    args: &syntax_cli::QueryArgs,
) -> Result<cadenza_syntax::query::Query, String> {
    use cadenza_syntax::query::{Pattern, Query};
    let compile = |srcs: &[String]| -> Result<Vec<Pattern>, String> {
        srcs.iter()
            .map(|s| Pattern::compile(s).map_err(|e| format!("relational pattern `{s}`: {e}")))
            .collect()
    };
    let mut q = Query::new();
    for p in compile(&args.inside)? {
        q = q.inside(p);
    }
    for p in compile(&args.has)? {
        q = q.has(p);
    }
    for p in compile(&args.not_inside)? {
        q = q.not_inside(p);
    }
    for p in compile(&args.not_has)? {
        q = q.not_has(p);
    }
    Ok(q)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_where_accepts_eq_and_neq() {
        let p = parse_where("type-of(x) = Int64").unwrap();
        assert_eq!(
            (p.var.as_str(), p.ty.as_str(), p.equal),
            ("x", "Int64", true)
        );

        let p = parse_where("type-of(x) != Bool").unwrap();
        assert_eq!(
            (p.var.as_str(), p.ty.as_str(), p.equal),
            ("x", "Bool", false)
        );
    }

    #[test]
    fn parse_where_is_whitespace_and_comma_insensitive() {
        // A leading `,` on the var (as one might copy from a pattern) and loose spacing are tolerated.
        let p = parse_where("  type-of( ,elem )  =  (List Int64) ").unwrap();
        assert_eq!(p.var, "elem");
        assert_eq!(p.ty, "(List Int64)"); // a compound type is taken verbatim
        assert!(p.equal);
    }

    #[test]
    fn parse_where_rejects_unknown_shapes() {
        for bad in [
            "x is Int64",
            "type-of(x)",
            "type-of() = Int64",
            "type-of(x) = ",
        ] {
            assert!(parse_where(bad).is_err(), "should reject `{bad}`");
        }
    }
}
