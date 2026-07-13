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
    /// Report every well-formedness fault in FILE (type mismatch, unbound name, …) as
    /// `file:line:col: severity [CODE]: message` — "diagnostics as you type". No export/run needed;
    /// exits non-zero if any error-severity fault is present.
    Check(CheckArgs),
    /// Apply every VERIFIED fix in FILE — each proposed fix that, applied + re-checked, actually clears
    /// its diagnostic — and write the repaired program back (or preview with `--diff`/`--dry-run`).
    /// Turns "here is the fix" into "fixed it": the capstone of `cdz check`'s structured suggestions.
    Fix(FixArgs),
    /// Go-to-definition: the defining occurrence of the name at a source BYTE OFFSET in FILE, as
    /// `file:line:col`.
    Def(DefArgs),
    /// The bindings visible at a source BYTE OFFSET in FILE — "variable scope tracking". Each visible
    /// binding as `file:line:col: name : type` (innermost first).
    Scope(ScopeArgs),
    /// The module's exported interface: each `(export …)` name and its type, as
    /// `file:line:col: name : type`.
    Exports(ExportsArgs),
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
        // The compiler command. `cdz` (unlike bare `rcdzc`) holds the front-end, so it can accept a
        // SOURCE file directly — parsing it in-process to the `ast` artifact, and (for a debug target)
        // the `spans` artifact too — rather than requiring a pre-built binary AST.
        Cmd::Compile(a) => run_compile(a),
        // The span-mapped semantic queries live here (they need both libraries in one process).
        Cmd::Type(a) => run_type(&a),
        Cmd::TypeAt(a) => run_type_at(&a),
        Cmd::Uses(a) => run_uses(&a),
        Cmd::Check(a) => run_check(&a),
        Cmd::Fix(a) => run_fix(&a),
        Cmd::Def(a) => run_def(&a),
        Cmd::Scope(a) => run_scope(&a),
        Cmd::Exports(a) => run_exports(&a),
    }
}

// ── compile (with in-process source parsing + auto-spans for debug) ──────────────────────────────

/// A source-file extension `cdz compile` can parse in-process (vs a pre-built binary AST). `.cdz`/`.ml`
/// read as the ml surface, `.sexp`/`.sexpr` as s-expressions — mirroring `load_program_spanned`.
fn is_source_file(spec: &str) -> bool {
    // Only a bare `path` spec (no `kind:`/`name=`) with a source extension is auto-parsed; an explicit
    // `kind:name=path` (e.g. `ast:m=…`, `spans:m=…`) is passed through as a raw artifact untouched.
    if spec.contains(':') || spec.contains('=') {
        return false;
    }
    [".cdz", ".ml", ".sexp", ".sexpr"]
        .iter()
        .any(|ext| spec.ends_with(ext))
}

/// Expand each input spec, replacing a DIRECTORY with the source files under it (recursively). A spec
/// that is not a bare path (a `kind:name=path` artifact spec) or is `-` (stdin) passes through
/// verbatim — only a bare path that `is_dir()` is walked. The walk collects every file whose extension
/// is a recognized SOURCE surface (`.cdz`/`.ml`/`.sexp`/`.sexpr`), path-sorted for a deterministic
/// package (the same convention `cdz query`/`lint` use over a directory). A directory containing no
/// source files is an error (the user pointed at an empty tree). Mirrors `cadenza_syntax::cli`'s
/// `collect_dir`, kept here because `cdz compile` reads sources into `ast` artifacts itself.
fn expand_input_specs(specs: &[String]) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for spec in specs {
        // Only a BARE path can be a directory. A `kind:name=path` / `name=path` spec or `-` is not.
        let is_bare_path = spec != "-" && !spec.contains(':') && !spec.contains('=');
        let path = std::path::Path::new(spec);
        if is_bare_path && path.is_dir() {
            let before = out.len();
            collect_source_dir(path, &mut out)?;
            if out.len() == before {
                return Err(format!(
                    "{spec}: no source files (.cdz/.ml/.sexp) found in directory"
                ));
            }
        } else {
            out.push(spec.clone());
        }
    }
    Ok(out)
}

/// Recurse `dir`, appending every file with a recognized SOURCE extension to `out`. Sub-directories are
/// walked depth-first; each directory's own entries are path-sorted so the collected package is a
/// deterministic function of the tree (not filesystem enumeration order). Non-source files (README,
/// `.gitignore`, a pre-built `.ast`) and unreadable entries are skipped with a warning — pointing at a
/// dir never tries to parse a non-source file. Recognized extensions match [`is_source_file`].
fn collect_source_dir(dir: &std::path::Path, out: &mut Vec<String>) -> Result<(), String> {
    let entries =
        std::fs::read_dir(dir).map_err(|e| format!("reading dir {}: {e}", dir.display()))?;
    // Collect + sort this level's entries so the walk is deterministic (read_dir order is unspecified).
    let mut paths: Vec<std::path::PathBuf> = Vec::new();
    for entry in entries {
        match entry {
            Ok(e) => paths.push(e.path()),
            Err(e) => eprintln!(
                "{PROG}: skipping unreadable entry in {}: {e}",
                dir.display()
            ),
        }
    }
    paths.sort();
    for path in paths {
        if path.is_dir() {
            collect_source_dir(&path, out)?;
        } else if let Some(s) = path.to_str() {
            // The extension gates inclusion (a `.ast`/`.spans`/README in the tree is skipped) — only a
            // parseable source surface is compiled. `is_source_file` also rejects a `:`/`=` in the
            // name, so a path is only included when it is a plain source file.
            if is_source_file(s) {
                out.push(s.to_string());
            }
        }
    }
    Ok(())
}

/// Run `cdz compile`. Because `cdz` holds the front-end, a SOURCE file input is parsed in-process to
/// the `ast` artifact — and, when a debug target (`wasm-debug`/`dwarf`) is requested, also to the
/// `spans` artifact (projected into rcdzc's wire form), so a user gets DWARF without hand-building a
/// spans artifact. With no source input (the pure artifacts-in path), it delegates to the compiler CLI
/// unchanged — the `rcdzc` bin's behavior is untouched.
fn run_compile(args: compiler_cli::CompileArgs) -> ExitCode {
    // Expand any DIRECTORY input into the source files under it (recursively), so
    // `cdz compile src/ --entry app` compiles a whole package tree without naming each file. A plain
    // file, a `kind:name=path` artifact spec, and `-` (stdin) pass through untouched. Done here at the
    // host boundary — the pure `compile` never sees a path (`compile.rs` §NO I/O).
    let specs = match expand_input_specs(args.input_specs()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{PROG}: {e}");
            return ExitCode::FAILURE;
        }
    };
    // Fast path: no source-file input → the ordinary artifacts-in compile, byte-for-byte as before.
    // (A directory only ever expands to SOURCE files, so if one was given this branch is not taken;
    // `specs` here equals the original args, so `run(args)` sees the same inputs.)
    if !specs.iter().any(|s| is_source_file(s)) {
        return compiler_cli::run(args, PROG);
    }

    // A source-file compile ALWAYS contributes a `spans` input alongside its `ast`, not only when a
    // debug target wants one: a debug target CONSUMES spans to build DWARF, but the CLI's DIAGNOSTIC
    // reporter also uses them to locate an error as `path:line:col` (so `cdz compile foo.cdz` gives the
    // same located errors as `cdz check`, not a raw `(node N)`). Spans are output-neutral for a plain
    // wasm compile (verified byte-identical), so attaching them unconditionally is free. (An explicit
    // `spans:` input still works and takes precedence for its own program.)
    let targets = args.targets();
    let mut inputs: Vec<rcdzc::Artifact> = Vec::new();
    for spec in &specs {
        if is_source_file(spec) {
            // Parse the source in-process, keeping the span table (the whole-program form, as the gate
            // and the semantic queries use).
            let (source, arenas, spantable) = match load_program_spanned(spec) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("{PROG}: {e}");
                    return ExitCode::FAILURE;
                }
            };
            let name = program_name(spec);
            inputs.push(rcdzc::Artifact::new(
                rcdzc::Artifact::KIND_AST,
                name.clone(),
                cadenza_syntax::codec::encode(&arenas),
            ));
            {
                let span_data = span_data_of(spec, &source, &spantable);
                inputs.push(rcdzc::Artifact::new(
                    rcdzc::spans::KIND_SPANS,
                    name,
                    rcdzc::spans::encode(&span_data),
                ));
            }
        } else {
            // A raw artifact spec (`kind:name=path`) — read it from disk, kind/name from the spec.
            match read_artifact_spec(spec) {
                Ok(a) => inputs.push(a),
                Err(e) => {
                    eprintln!("{PROG}: {e}");
                    return ExitCode::FAILURE;
                }
            }
        }
    }
    // A `--entry <NAME>` names the package entry (a multi-file package needs it) — inject the
    // `KIND_ENTRY` artifact, exactly as the artifacts-in `run` path does.
    if let Some(entry) = args.entry() {
        inputs.push(compiler_cli::entry_artifact(entry));
    }
    compiler_cli::run_prepared(inputs, &targets, args.out_path(), PROG)
}

/// The artifact NAME for a source file — its file stem (so `add.cdz` → `add`), matching the compiler
/// CLI's default naming; falls back to `main`.
fn program_name(spec: &str) -> String {
    std::path::Path::new(spec)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("main")
        .to_string()
}

/// Project the front-end `SpanTable` (+ the source text) into rcdzc's `spans::SpanData` wire form — the
/// `(start, len)` byte range per `StructId`, the tree-relative module path, and the source text (for
/// line derivation). This MIRRORS rcdzc's format (copy-don't-depend: the two crates share no code, so
/// the mapping lives here at the driver that holds both). The module path is NORMALIZED
/// (`debug_module_path`) so DWARF carries no absolute build directory (`DESIGN-debug-info-rcdzc.md` §4).
fn span_data_of(
    spec: &str,
    source: &str,
    spantable: &cadenza_syntax::spans::SpanTable,
) -> rcdzc::spans::SpanData {
    let spans: Vec<(u32, u32)> = (0..spantable.len())
        .map(
            |i| match spantable.get(cadenza_syntax::StructId(i as u32)) {
                Some(sp) => (sp.start as u32, (sp.end - sp.start) as u32),
                None => (0, 0),
            },
        )
        .collect();
    rcdzc::spans::SpanData {
        module_path: debug_module_path(spec),
        spans,
        source: source.to_string(),
    }
}

/// Normalize a source spec into the module path DWARF records (`DW_AT_name` on the compile unit + the
/// file-table entry). Debug info MUST be a deterministic function of source + toolchain and carry no
/// provenance (`DESIGN-debug-info-rcdzc.md` §4 — the `DW_AT_name` counterpart of `-ffile-prefix-map`):
/// an ABSOLUTE path leaks the build directory (`/home/alice/proj/add.sexp`), so two machines building
/// the same source would emit different DWARF. So an absolute path is reduced to its file name (a
/// deterministic, build-directory-free stand-in); a relative path is already tree-relative — kept
/// verbatim — with `./` stripped for tidiness. Empty (only when a spec is itself empty) falls back to
/// the file name too. Backslashes are not special here (POSIX host); a Windows path degrades to
/// file-name only, which is still deterministic.
fn debug_module_path(spec: &str) -> String {
    let path = std::path::Path::new(spec);
    if path.is_absolute() {
        // Strip the build directory — keep only the file name (deterministic across machines).
        return path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(spec)
            .to_string();
    }
    // A relative path is already tree-relative; drop a leading `./` for tidiness.
    spec.strip_prefix("./").unwrap_or(spec).to_string()
}

/// Read a raw artifact from a `kind:name=path` (or `name=path`, or `path`) spec — the same spec grammar
/// the compiler CLI parses, so a mixed `cdz compile prog.cdz sidecar:d=drive.bin` works. `-` reads stdin.
fn read_artifact_spec(spec: &str) -> Result<rcdzc::Artifact, String> {
    // Split an optional `kind:` prefix (only when it looks like one), then an optional `name=` prefix.
    let (kind, rest) = match spec.split_once(':') {
        Some((k, r)) if !k.contains('/') && !k.contains('=') => (k.to_string(), r),
        _ => (rcdzc::Artifact::KIND_AST.to_string(), spec),
    };
    let (name, path) = match rest.split_once('=') {
        Some((n, p)) => (n.to_string(), p.to_string()),
        None => (program_name(rest), rest.to_string()),
    };
    let bytes = if path == "-" {
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut std::io::stdin(), &mut buf)
            .map_err(|e| format!("cannot read stdin: {e}"))?;
        buf
    } else {
        std::fs::read(&path).map_err(|e| format!("cannot read {path}: {e}"))?
    };
    Ok(rcdzc::Artifact::new(kind, name, bytes))
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
struct CheckArgs {
    /// The program file to check (`.cdz`/`.ml` → ml, `.sexp`/`.sexpr` → sexpr).
    file: String,
    /// Emit each diagnostic as a machine-readable JSON object (one per line), including its structured
    /// fix — the shape an agent or an editor consumes to apply the repair directly, rather than
    /// text-parsing the human `file:line:col` output. Exit code is unchanged (non-zero iff any error).
    #[arg(long)]
    json: bool,
    /// VERIFY each proposed fix by applying it to the source and re-checking: a heuristic fix that
    /// actually clears its diagnostic (with no parse error and no new same-code error) is UPGRADED to
    /// `verified` in the output — so an agent can apply it blind (`spec/capabilities/diagnostics.md` §A
    /// Confirmed Fix Is Marked Verified). Off by default (it recompiles once per fix); the compiler's own
    /// rule-verified fixes (e.g. the `_`-prefix silence) are always `verified` regardless.
    #[arg(long)]
    verify_fixes: bool,
}

#[derive(clap::Args)]
struct FixArgs {
    /// The program file to repair (`.cdz`/`.ml` → ml, `.sexp`/`.sexpr` → sexpr).
    file: String,
    /// Preview the repaired program on stdout WITHOUT writing the file (a unified diff of the change).
    #[arg(long)]
    diff: bool,
    /// Print the repaired program on stdout WITHOUT writing the file (the full text, not a diff).
    #[arg(long)]
    dry_run: bool,
    /// Also apply HEURISTIC fixes that verify (apply + re-check clears the fault) — by default `fix`
    /// applies only fixes the COMPILER marked verified by a rule. With `--all`, any fix that recompiles
    /// clean is applied (the `check --verify-fixes` bar). A fix that does not verify is NEVER applied.
    #[arg(long)]
    all: bool,
    /// Report the applied fixes as JSON (one object per fix: `code`, `message`, `kind`) instead of the
    /// human "applied N fix(es)" line — so an agent driving `fix` learns exactly WHICH faults were
    /// repaired, not just how many. The file is still written (unless `--diff`/`--dry-run`).
    #[arg(long)]
    json: bool,
}

#[derive(clap::Args)]
struct DefArgs {
    /// The program file (`.cdz`/`.ml` → ml, `.sexp`/`.sexpr` → sexpr).
    file: String,
    /// The source BYTE OFFSET of the reference to jump from (0-based, UTF-8 bytes).
    offset: usize,
}

#[derive(clap::Args)]
struct ScopeArgs {
    /// The program file (`.cdz`/`.ml` → ml, `.sexp`/`.sexpr` → sexpr).
    file: String,
    /// The source BYTE OFFSET whose visible bindings to list (0-based, UTF-8 bytes).
    offset: usize,
}

#[derive(clap::Args)]
struct ExportsArgs {
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
    // ONE line-start index over the source, so each reference's line:col is a binary search, not a
    // from-start newline scan — `cdz uses` over N references was O(N × source_len) = O(N²) (a name with
    // 4000 references = 207ms, 99.9% in `line_col`); with the index it is linear.
    let index = cadenza_syntax::query::driver::LineIndex::new(&source);
    for id in ids {
        match spans.get(cadenza_syntax::StructId(id)) {
            Some(span) => {
                let (line, col) = index.line_col(&source, span.start);
                println!("{}:{line}:{col}", args.file);
            }
            // A referencing occurrence with no recorded span (should not happen for a user node) still
            // reports the raw id rather than dropping it silently.
            None => println!("{}:node {id}", args.file),
        }
    }
    ExitCode::SUCCESS
}

/// Apply a structural fix to `source`, returning the edited text — the STRUCTURAL realization of an
/// [`rcdzc::DiagnosticFix`]. Rather than splice bytes by hand (finding a list's closing paren for an
/// insert, trimming a separator for a delete, substituting a `…` sentinel for a wrap, reshaping the wrap
/// for the surface), it builds the NEW TREE — the parsed program with the target node transformed per the
/// fix — and hands old+new to `cadenza_syntax`'s formatting-preserving structural rewriter
/// (`textedit::rewrite_preserving`), the SAME engine `cdz rewrite` uses. That engine edits only the
/// changed subtree at its span, reprints it in the file's SURFACE (ML pretty-print vs s-expr), and leaves
/// all other bytes — layout, comments — verbatim. So surface-correctness, insert placement, and
/// separator hygiene all come from one shared, tested mechanism instead of four hand-rolled text cases.
///
/// `arenas`/`spans` are the parsed program + its node→span table (from `load_program_spanned`); `target`
/// is the fix's node id; `kind`/`repl` its operation + payload. `surface` is the file's format. `None`
/// when the fix cannot be built structurally (an unparseable payload, a node not found) — the caller then
/// declines the fix rather than corrupting.
fn apply_fix_to_source(
    source: &str,
    arenas: &cadenza_syntax::Arenas,
    spans: &cadenza_syntax::spans::SpanTable,
    kind: &str,
    target: cadenza_syntax::StructId,
    repl: &str,
    surface: cadenza_syntax::convert::Format,
) -> Option<String> {
    let (old, new) = fix_old_new(arenas, kind, target, repl)?;
    let span_of = |t: &cadenza_syntax::query::Tree| -> Option<(usize, usize)> {
        t.origin()
            .and_then(|id| spans.get(id))
            .map(|s| (s.start, s.end))
    };
    let edited =
        cadenza_syntax::query::textedit::rewrite_preserving(source, &old, &new, &span_of, surface);
    Some(edited.output)
}

/// The STRUCTURAL PATCH a fix realizes — the minimal, surface-correct, span-anchored byte edits
/// (`[{start, end, text}]`) turning `source` into the fixed program. The machine-channel (`cdz check
/// --json`) counterpart of [`apply_fix_to_source`]: same new-tree build + same `cadenza_syntax` engine,
/// but returns the primitive edits (via `textedit::edits_preserving`) so an agent applies them directly
/// (`source[start..end] := text`) instead of re-deriving positions from a kind/prefix/suffix. `None` when
/// the fix cannot be built (unparseable payload, node not found).
fn fix_edits(
    source: &str,
    arenas: &cadenza_syntax::Arenas,
    spans: &cadenza_syntax::spans::SpanTable,
    kind: &str,
    target: cadenza_syntax::StructId,
    repl: &str,
    surface: cadenza_syntax::convert::Format,
) -> Option<Vec<cadenza_syntax::query::textedit::Edit>> {
    let (old, new) = fix_old_new(arenas, kind, target, repl)?;
    let span_of = |t: &cadenza_syntax::query::Tree| -> Option<(usize, usize)> {
        t.origin()
            .and_then(|id| spans.get(id))
            .map(|s| (s.start, s.end))
    };
    Some(cadenza_syntax::query::textedit::edits_preserving(
        source, &old, &new, &span_of, surface,
    ))
}

/// Build the `(old, new)` tree pair a fix transforms — the shared core of [`apply_fix_to_source`] and
/// [`fix_edits`]. `old` is the parsed program; `new` is it with the target node transformed per kind (a
/// PURE tree op — no text, no `…` sentinel, no paren-finding). `None` if the payload doesn't parse or the
/// target isn't found.
fn fix_old_new(
    arenas: &cadenza_syntax::Arenas,
    kind: &str,
    target: cadenza_syntax::StructId,
    repl: &str,
) -> Option<(cadenza_syntax::query::Tree, cadenza_syntax::query::Tree)> {
    use cadenza_syntax::query::Tree;
    let old = Tree::of(arenas);
    let new = if kind == "delete" {
        // Delete removes the node from its parent's child list — a structural op the node-transform
        // closure can't express (it returns a replacement node, not "no node"), so its own builder.
        delete_target(&old, target)?
    } else {
        transform_target(&old, target, &mut |node: &Tree| -> Option<Tree> {
            match kind {
                // Replace the node with the parsed payload subtree.
                "replace" => parse_fragment(repl),
                // Wrap: build the ctor form from `repl`'s parse and substitute its `…` hole atom with the
                // ORIGINAL node subtree (spans intact) — `(Some …)` + node → `(Some <node>)`.
                "wrap" => {
                    let ctor = parse_fragment(repl)?;
                    Some(substitute_hole(&ctor, node))
                }
                // Insert: append the parsed arm form(s) as new children at the end of the target LIST.
                "insert" => {
                    let Tree::List(children, origin) = node else {
                        return None; // an insert targets a list (the `(match …)` form)
                    };
                    let mut children = children.clone();
                    for arm in split_top_forms(repl) {
                        children.push(parse_fragment(&arm)?);
                    }
                    Some(Tree::List(children, *origin))
                }
                _ => None,
            }
        })?
    };
    Some((old, new))
}

/// Parse a fix-payload s-expression fragment (`(Some …)`, `(B unit)`, `compute`) into an owned [`Tree`],
/// or `None` if it does not parse. New nodes carry NO provenance (they are synthesized), which the
/// structural rewriter handles — only ORIGINAL nodes need spans.
fn parse_fragment(text: &str) -> Option<cadenza_syntax::query::Tree> {
    cadenza_syntax::sexpr::read(text)
        .ok()
        .map(|a| cadenza_syntax::query::Tree::of(&a))
}

/// Split a space-joined run of top-level s-expression forms (the `insert` payload, e.g. `(Green unit)
/// (Blue unit)`) into its individual forms. Uses the reader's multi-form parse, then renders each back —
/// so each element is a complete, independently-parseable form.
fn split_top_forms(text: &str) -> Vec<String> {
    match cadenza_syntax::sexpr::read_all(text) {
        Ok(a) => {
            let tree = cadenza_syntax::query::Tree::of(&a);
            match &tree {
                // `read_all` wraps multiple forms in a synthetic `(do …)`; unwrap to the forms.
                cadenza_syntax::query::Tree::List(items, _)
                    if matches!(
                        items.first(),
                        Some(cadenza_syntax::query::Tree::Atom(
                            cadenza_syntax::ast::Leaf::Name(n),
                            _,
                        )) if n == "do"
                    ) =>
                {
                    items.iter().skip(1).map(|t| t.to_sexpr()).collect()
                }
                other => vec![other.to_sexpr()],
            }
        }
        Err(_) => vec![text.to_string()],
    }
}

/// Substitute the [`rcdzc::WRAP_HOLE`] atom inside `template` with `fill` — the structural realization of
/// a wrap: `(Some …)` with `…` replaced by the wrapped subtree becomes `(Some <subtree>)`. Recurses; a
/// non-hole node is copied structurally (preserving its provenance so an unchanged child keeps its span).
fn substitute_hole(
    template: &cadenza_syntax::query::Tree,
    fill: &cadenza_syntax::query::Tree,
) -> cadenza_syntax::query::Tree {
    use cadenza_syntax::query::Tree;
    match template {
        Tree::Atom(cadenza_syntax::ast::Leaf::Name(n), _) if n == &rcdzc::WRAP_HOLE.to_string() => {
            fill.clone()
        }
        Tree::Atom(..) => template.clone(),
        Tree::List(items, origin) => Tree::List(
            items.iter().map(|t| substitute_hole(t, fill)).collect(),
            *origin,
        ),
    }
}

/// Rebuild `tree`, applying `f` to the node whose origin is `target` (replacing it with `f`'s result).
/// `None` if the target is not found or `f` declines. Recurses structurally, preserving provenance on the
/// untouched nodes so the rewriter edits only the one changed subtree.
fn transform_target(
    tree: &cadenza_syntax::query::Tree,
    target: cadenza_syntax::StructId,
    f: &mut dyn FnMut(&cadenza_syntax::query::Tree) -> Option<cadenza_syntax::query::Tree>,
) -> Option<cadenza_syntax::query::Tree> {
    use cadenza_syntax::query::Tree;
    if tree.origin() == Some(target) {
        return f(tree);
    }
    match tree {
        Tree::Atom(..) => None,
        Tree::List(items, origin) => {
            let mut hit = false;
            let mut out = Vec::with_capacity(items.len());
            for child in items {
                if !hit && let Some(new_child) = transform_target(child, target, f) {
                    out.push(new_child);
                    hit = true;
                } else {
                    out.push(child.clone());
                }
            }
            hit.then_some(Tree::List(out, *origin))
        }
    }
}

/// Rebuild `tree` with the node whose origin is `target` REMOVED from its parent's child list — the
/// structural realization of a delete (`(host (log) 42)` → `(host () 42)`; the separator hygiene the old
/// text path hand-trimmed is handled by the rewriter's child-alignment). `None` if not found.
fn delete_target(
    tree: &cadenza_syntax::query::Tree,
    target: cadenza_syntax::StructId,
) -> Option<cadenza_syntax::query::Tree> {
    use cadenza_syntax::query::Tree;
    match tree {
        Tree::Atom(..) => None,
        Tree::List(items, origin) => {
            if items.iter().any(|c| c.origin() == Some(target)) {
                let kept: Vec<Tree> = items
                    .iter()
                    .filter(|c| c.origin() != Some(target))
                    .cloned()
                    .collect();
                return Some(Tree::List(kept, *origin));
            }
            let mut hit = false;
            let mut out = Vec::with_capacity(items.len());
            for child in items {
                if !hit && let Some(nc) = delete_target(child, target) {
                    out.push(nc);
                    hit = true;
                } else {
                    out.push(child.clone());
                }
            }
            hit.then_some(Tree::List(out, *origin))
        }
    }
}

/// Whether an error diagnostic with code `code` is ABSENT from `text` when parsed as `is_ml` (else
/// s-expr) — the "did the fix clear the fault?" predicate for `check --verify-fixes`. Re-parses the
/// edited text and re-runs the `Diagnostics` query; a parse failure or an unresolved AST reads as "not
/// cleared" (returns `false`), so a fix is confirmed ONLY when the program parses AND the same-code
/// error is gone. Conservative: it does not require the WHOLE program to be clean (an unrelated
/// pre-existing error may remain), only that THIS diagnostic's code no longer appears.
/// The multiset of diagnostics a source produces, each keyed by `(severity, code, message)` — the
/// baseline a candidate fix is judged against. Returns `None` if the source does not parse/compile at the
/// entry (a broken edit), so a caller treats an unparseable result as "no clean verdict". Tracking
/// SEVERITY (not just errors) lets a WARNING-clearing fix verify too — a redundant-arm DELETE (CDZ0213, a
/// warning) clears its warning without touching the error set, which an error-only baseline could never
/// confirm. Keyed by MESSAGE (see below), not node id, so a renumbering edit does not spuriously regress.
fn program_diagnostic_keys(text: &str, is_ml: bool) -> Option<Vec<(String, String, String)>> {
    let arenas = if is_ml {
        let parsed = cadenza_syntax::parser::read_ml(text);
        if !parsed.errors.is_empty() {
            return None; // the edit broke the parse — no verdict
        }
        parsed.arenas
    } else {
        match cadenza_syntax::sexpr::read(text) {
            Ok(a) => a,
            Err(_) => match cadenza_syntax::sexpr::read_all(text) {
                Ok(a) => a,
                Err(_) => return None,
            },
        }
    };
    let out = run_sidecar(
        &arenas,
        rcdzc::Request::Query(rcdzc::sidecar::Query::Diagnostics),
    );
    let bytes = out.artifact(rcdzc::sidecar::KIND_DIAGNOSTICS)?; // no artifact → failed at entry
    let text_out = String::from_utf8_lossy(bytes);
    // The wire is one fault per line: `severity<TAB>code<TAB>node<TAB>fix-kind<TAB>fix-node<TAB>
    // fix-repl<TAB>fix-verified<TAB>message` (8 cols). Key each fault by `(SEVERITY, code, MESSAGE)` — NOT
    // the node id. A fix that RENUMBERS nodes (a wrap/insert shifts every following node's id) would make
    // an untouched, still-present fault land at a different id and look "new", failing the no-regression
    // check spuriously (`cdz fix --all` would then decline a valid fix when a SECOND independent fault
    // survives). The message text is invariant under renumbering, so it identifies "the same fault"
    // faithfully; two genuinely-distinct faults of one code differ in their message (they name different
    // variants / spots), so they stay distinct keys.
    let mut keys: Vec<(String, String, String)> = text_out
        .lines()
        .filter_map(|line| {
            let cols: Vec<&str> = line.splitn(8, '\t').collect();
            match (cols.first(), cols.get(1), cols.get(7)) {
                (Some(sev), Some(code), Some(msg)) => {
                    Some((sev.to_string(), code.to_string(), msg.to_string()))
                }
                _ => None,
            }
        })
        .collect();
    keys.sort();
    Some(keys)
}

/// Whether applying an edit to a program is a CONFIRMED, machine-applicable fix — the `--verify-fixes` /
/// `cdz fix --all` upgrade rule. Two conditions, both necessary (`spec/capabilities/diagnostics.md` §A
/// Confirmed Fix Is Marked Verified): (1) the edited program clears an error with `code` (the fault is
/// gone), AND (2) the edit introduces NO NEW error — every error the edited program still has was already
/// present in `baseline`. Condition (2) is what keeps an insert-arms fix HEURISTIC: adding `(Blue unit)`
/// clears the CDZ0210 but its `unit` PLACEHOLDER body mismatches the other arms' type (a fresh CDZ0203),
/// so it is not "apply blind" — the author must fill the body. `baseline` is `program_diagnostic_keys` of
/// the ORIGINAL source; `None` means the original itself did not compile (then any edit that yields a clean
/// parse and clears the fault is accepted — there is no baseline to regress against). `severity` is the
/// faulting diagnostic's severity ("error"/"warning"): a WARNING-clearing fix (a redundant-arm DELETE,
/// CDZ0213) verifies the same way an error fix does — condition (1) matches on (severity, code), condition
/// (2) still guards the ERROR set only (a fix may freely change warnings, but must never introduce an error).
fn fix_verifies(
    text: &str,
    is_ml: bool,
    severity: &str,
    code: &str,
    baseline: Option<&[(String, String, String)]>,
) -> bool {
    let Some(after) = program_diagnostic_keys(text, is_ml) else {
        return false; // the edit broke the parse/compile — not a clean fix
    };
    // (1) a fault with THIS (severity, code) must be GONE — the edited program has strictly fewer.
    let count_of = |keys: &[(String, String, String)], sev: &str, c: &str| {
        keys.iter().filter(|(s, k, _)| s == sev && k == c).count()
    };
    let cleared = match baseline {
        Some(b) => count_of(&after, severity, code) < count_of(b, severity, code),
        None => !after.iter().any(|(s, k, _)| s == severity && k == code),
    };
    if !cleared {
        return false;
    }
    // (2) NO NEW ERROR: every ERROR the edited program still has must have been in the baseline (as a
    // multiset, so an edit that turns one error into two of a kind is caught). Warnings are NOT guarded —
    // a fix that clears one warning may leave/introduce others (e.g. deleting a redundant arm could expose
    // a now-unused binding), which an agent handles on the next pass; only a NEW error blocks the fix. A
    // missing baseline waives this — the original did not compile, so there is nothing to regress.
    if let Some(b) = baseline {
        let mut remaining: Vec<&(String, String, String)> =
            b.iter().filter(|(s, _, _)| s == "error").collect();
        for key in after.iter().filter(|(s, _, _)| s == "error") {
            match remaining.iter().position(|k| *k == key) {
                Some(i) => {
                    remaining.swap_remove(i);
                }
                None => return false, // an error not accounted for by the baseline — a NEW error
            }
        }
    }
    true
}

/// `cdz check FILE` — report every well-formedness fault, "diagnostics as you type". Drives the
/// compiler's `Query::Diagnostics` (the fault set, NOT gated on export/emit), maps each fault's node id
/// to `file:line:col` via the span table, and prints `file:line:col: severity [CODE]: message`. Exits
/// non-zero iff any error-severity fault is present (a clean file prints nothing and exits 0) — the
/// CI-gate / editor-lint shape.
fn run_check(args: &CheckArgs) -> ExitCode {
    let (source, arenas, spans) = match load_program_spanned(&args.file) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{PROG}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let out = run_sidecar(
        &arenas,
        rcdzc::Request::Query(rcdzc::sidecar::Query::Diagnostics),
    );
    let Some(bytes) = out.artifact(rcdzc::sidecar::KIND_DIAGNOSTICS) else {
        report_errors(&out);
        return ExitCode::FAILURE;
    };
    let text = String::from_utf8_lossy(bytes);
    let mut any_error = false;
    // The ORIGINAL program's diagnostic set — the baseline `--verify-fixes` judges each candidate fix
    // against (a fix upgrades to verified only if it clears its fault AND introduces no new error).
    // Computed once (a recompile), only when `--verify-fixes` is set, so plain `check`/`--json` pay nothing.
    let baseline_errors: Option<Vec<(String, String, String)>> = if args.verify_fixes {
        program_diagnostic_keys(&source, is_ml_source(&args.file))
    } else {
        None
    };
    // A node id → its 1-based `(line, col)` start plus its `[from, to)` UTF-8 byte range, via the span
    // table this process kept. `None` for an unanchored (`-`) or unmapped node. One line-start index
    // (binary-searched line:col) so a program with MANY diagnostics — each mapped here, some more than
    // once via the source-order sort key — stays linear instead of O(diags × source_len) (500 unbound
    // names = 73ms, growing ~2.8×/doubling in the from-start `line_col` scan).
    let index = cadenza_syntax::query::driver::LineIndex::new(&source);
    let span_of = |node: &str| -> Option<(usize, usize, usize, usize)> {
        let n = node.parse::<u32>().ok()?;
        let span = spans.get(cadenza_syntax::StructId(n))?;
        let (l, c) = index.line_col(&source, span.start);
        Some((l, c, span.start, span.end))
    };
    let loc_label = |node: &str| match span_of(node) {
        Some((l, c, _, _)) => format!("{}:{l}:{c}", args.file),
        None => args.file.clone(),
    };
    // REPORT IN SOURCE ORDER (by node start byte), not the sidecar's fault-collection order — the tree
    // walk that gathers faults does not visit strictly left-to-right, so without this a reader sees an
    // error at column 22 above one at column 21 (e.g. `(match foo (a bar) …)` reports `foo`, then `bar`
    // to its LEFT). A diagnostic whose node has no span (unanchored `-`, or a spanless synthesized node)
    // sorts LAST via the STABLE sort, keeping the sidecar's relative order — so the sequence stays a
    // deterministic function of the source (`diagnostics.md` §Diagnostics Are Emitted In A Deterministic
    // Order), now also legible top-to-bottom. The node id is column 3 (index 2) of each TAB line.
    let line_start = |line: &str| -> Option<usize> {
        let node = line.split('\t').nth(2)?;
        span_of(node).map(|(_, _, from, _)| from)
    };
    let mut lines: Vec<&str> = text.lines().collect();
    lines.sort_by(|a, b| match (line_start(a), line_start(b)) {
        (Some(fa), Some(fb)) => fa.cmp(&fb),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
    // Each line is `severity<TAB>code<TAB>node-id<TAB>fix-kind<TAB>fix-node<TAB>fix-replacement<TAB>
    // fix-verified<TAB>message` — the first seven columns split on the first seven tabs, message is the
    // free-text remainder. `code`/`node-id`/the four fix columns may be `-` (absent).
    for line in lines {
        let mut cols = line.splitn(8, '\t');
        let (severity, code, node, fix_kind, fix_node, fix_repl, fix_verified, message) = match (
            cols.next(),
            cols.next(),
            cols.next(),
            cols.next(),
            cols.next(),
            cols.next(),
            cols.next(),
            cols.next(),
        ) {
            (Some(s), Some(c), Some(n), Some(fk), Some(fn_), Some(fr), Some(fv), Some(m)) => {
                (s, c, n, fk, fn_, fr, fv, m)
            }
            _ => continue, // a malformed line (shouldn't happen) — skip rather than crash
        };
        any_error |= severity == "error";
        // A fix the compiler rendered in s-expr form may not be byte-splice-able into THIS file's
        // surface. `replace`/`delete`/`wrap` render on any surface (a bare name, a deletion, or the
        // `(ctor …)`→`ctor(…)` reshape). `insert` splices ARM/child scaffold whose syntax only exists
        // in-context (a handle/match arm) — an s-expr arm can't be lowered to ML by a fragment print —
        // so on a non-s-expr file we DROP the structured fix (the message still names the arm to add).
        // Treating it as "no fix node" makes both the human `help:` line and the JSON `fix` object omit
        // it uniformly. (`cdz fix` already declines it via the verify re-parse; this stops `check --json`
        // from handing an ML agent an unusable insert.)
        let fix_node = if fix_kind == "insert" && is_ml_source(&args.file) {
            "-"
        } else {
            fix_node
        };

        // `--verify-fixes`: UPGRADE a heuristic fix to verified when applying it actually clears this
        // diagnostic. The compiler marks a fix verified only when a RULE proves it (D3's `_`-prefix);
        // a wrap/coercion/did-you-mean is heuristic there because the compiler does not recompile. Here
        // — where we hold BOTH the source text and the compiler — we CAN: splice the fix in, re-check,
        // and confirm the same-code error is gone (`spec/capabilities/diagnostics.md` §A Confirmed Fix
        // Is Marked Verified). Only heuristic fixes with a real target span are candidates.
        let verified_flag = if fix_verified == "verified" {
            true
        } else if args.verify_fixes && fix_node != "-" && code != "-" {
            let is_ml = is_ml_source(&args.file);
            fix_node
                .parse::<u32>()
                .ok()
                .and_then(|n| {
                    apply_fix_to_source(
                        &source,
                        &arenas,
                        &spans,
                        fix_kind,
                        cadenza_syntax::StructId(n),
                        fix_repl,
                        surface_of(&args.file),
                    )
                })
                .map(|edited| {
                    fix_verifies(&edited, is_ml, severity, code, baseline_errors.as_deref())
                })
                .unwrap_or(false)
        } else {
            false
        };

        if args.json {
            // The machine-readable shape: one JSON object per diagnostic, its structured fix nested. The
            // fix carries a STRUCTURAL PATCH — `edits: [{from, to, text}]` — that an agent applies
            // literally: for each edit (already sorted, non-overlapping), `source[from..to] := text`. The
            // edits come from the SAME structural engine `cdz fix` applies (`cadenza_syntax`'s
            // formatting-preserving rewriter over the fixed tree), so they are minimal and surface-correct
            // — a wrap is two inserts around the preserved node bytes, an insert lands at the right child
            // position, a delete drops the node + its separator — with NO `…` sentinel and no hand-derived
            // positions. `kind` is advisory (what the fix does); `verified` says whether to apply blind.
            use cadenza_syntax::query::json;
            let mut obj = json::Object::new();
            obj.string("severity", severity);
            if code != "-" {
                obj.string("code", code);
            }
            obj.string("message", message);
            if let Some((l, c, from, to)) = span_of(node) {
                obj.raw("line", &l.to_string());
                obj.raw("col", &c.to_string());
                obj.raw("from", &from.to_string());
                obj.raw("to", &to.to_string());
            }
            // Compute the structural patch for the fix (if any). Only when the fix's node parses AND the
            // patch builds — otherwise the diagnostic carries no `fix` (message-only guidance).
            let patch = if fix_node != "-" {
                fix_node.parse::<u32>().ok().and_then(|n| {
                    fix_edits(
                        &source,
                        &arenas,
                        &spans,
                        fix_kind,
                        cadenza_syntax::StructId(n),
                        fix_repl,
                        surface_of(&args.file),
                    )
                })
            } else {
                None
            };
            if let Some(edits) = patch.filter(|e| !e.is_empty()) {
                let mut fix = json::Object::new();
                fix.string("kind", fix_kind); // "replace" | "insert" | "wrap" | "delete"
                fix.raw("verified", if verified_flag { "true" } else { "false" });
                let mut arr = json::Array::new();
                for e in &edits {
                    let mut eo = json::Object::new();
                    eo.raw("from", &e.start.to_string());
                    eo.raw("to", &e.end.to_string());
                    eo.string("text", &e.text);
                    arr.raw(&eo.finish());
                }
                fix.raw("edits", &arr.finish());
                obj.raw("fix", &fix.finish());
            }
            println!("{}", obj.finish());
            continue;
        }

        let code_part = if code == "-" {
            String::new()
        } else {
            format!(" [{code}]")
        };
        println!("{}: {severity}{code_part}: {message}", loc_label(node));
        // A structural fix, if the diagnostic carries one — the rustc-style `help:` line an agent (or an
        // editor's quick-fix) applies directly. `replace` swaps the node's spelling; `insert` appends the
        // rendered form(s) into the node (e.g. the missing match arms). The applicability marker rides
        // along so a consumer branches (`verified` = apply blind, else confirm intent).
        if fix_node != "-" {
            let marker = if verified_flag {
                "" // machine-applicable — no caveat
            } else {
                " (heuristic)"
            };
            let action = match fix_kind {
                "insert" => format!("add `{fix_repl}`"),
                "wrap" => format!("wrap in `{fix_repl}`"),
                "delete" => "remove this element".to_string(),
                _ => format!("replace with `{fix_repl}`"),
            };
            println!("{}: help{marker}: {action}", loc_label(fix_node));
        }
    }
    if any_error {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// `cdz fix FILE` — apply every VERIFIED fix and write the repaired program back. Runs the same
/// `Diagnostics` query as `check`, and for each diagnostic that carries a fix, KEEPS the fix only if it
/// verifies: applying it (structurally, via [`apply_fix_to_source`]) and re-checking clears that
/// diagnostic's code with no new error (`fix_verifies`). By default only fixes the compiler ALREADY
/// marked verified (a rule) plus, with `--all`, any heuristic fix that so verifies — a fix that does not
/// verify is never applied.
///
/// Fixes are applied ONE AT A TIME, RE-LOADING (re-parse + re-diagnose) between each: a structural fix
/// rebuilds the tree and reprints the changed subtree, which shifts node ids, so a second fix must be
/// resolved against the freshly-edited program rather than stale ids. A fixpoint loop (bounded) applies
/// fixes until none remain or nothing changes. The result is written back, or previewed with `--dry-run`
/// (full text) / `--diff` (a unified diff). Exit 0 on success.
fn run_fix(args: &FixArgs) -> ExitCode {
    let (source, arenas, _) = match load_program_spanned(&args.file) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{PROG}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let is_ml = is_ml_source(&args.file);
    let surface = surface_of(&args.file);
    // The ORIGINAL program's diagnostic set — the baseline every `--all` candidate is judged against
    // (apply a heuristic fix only if it clears its fault AND introduces no new error). Fixed to the
    // original so a fix's own downstream warning never reads as a regression across the fixpoint iterations.
    let baseline_errors: Option<Vec<(String, String, String)>> =
        program_diagnostic_keys(&source, is_ml);

    // Apply fixes to a fixpoint, re-loading between each so node ids track the edited text. Each pass
    // applies AT MOST ONE fix (the first applicable one), then re-parses; the loop ends when a pass
    // applies nothing. Bounded so a pathological non-converging fix can't spin forever.
    let mut current = source.clone();
    let mut current_arenas = arenas;
    let mut applied = 0usize;
    let mut considered_any = false;
    // Each applied fix's `(code, kind, message)` — for the `--json` report so an agent learns WHICH
    // faults were repaired (not just the count).
    let mut applied_fixes: Vec<(String, String, String)> = Vec::new();
    for _ in 0..64 {
        let out = run_sidecar(
            &current_arenas,
            rcdzc::Request::Query(rcdzc::sidecar::Query::Diagnostics),
        );
        let Some(bytes) = out.artifact(rcdzc::sidecar::KIND_DIAGNOSTICS) else {
            report_errors(&out);
            return ExitCode::FAILURE;
        };
        let diag_text = String::from_utf8_lossy(bytes).into_owned();
        // Rebuild the span table for the CURRENT text (node ids shift after each structural edit).
        let Some(spans) = reparse_spans(&current, surface) else {
            break;
        };

        // Find the first applicable fix in this pass.
        let mut applied_this_pass = false;
        for line in diag_text.lines() {
            let cols: Vec<&str> = line.splitn(8, '\t').collect();
            if cols.len() < 8 {
                continue;
            }
            let (severity, code, fix_kind, fix_node, fix_repl, fix_verified, message) = (
                cols[0], cols[1], cols[3], cols[4], cols[5], cols[6], cols[7],
            );
            if fix_node == "-" || code == "-" {
                continue;
            }
            considered_any = true;
            let Some(target) = fix_node.parse::<u32>().ok().map(cadenza_syntax::StructId) else {
                continue;
            };
            // Build the edited text structurally.
            let Some(edited) =
                apply_fix_to_source(&current, &current_arenas, &spans, fix_kind, target, fix_repl, surface)
            else {
                continue;
            };
            // Apply a compiler-verified fix always; a heuristic one only under `--all` AND only if it
            // verifies (clears its fault, introduces no new error).
            let apply = fix_verified == "verified"
                || (args.all
                    && fix_verifies(&edited, is_ml, severity, code, baseline_errors.as_deref()));
            if !apply {
                continue;
            }
            // Commit this edit, re-parse for the next pass.
            let Some(next_arenas) = reparse_arenas(&edited, surface) else {
                continue; // a fix that broke the parse — skip it (should not happen post-verify)
            };
            current = edited;
            current_arenas = next_arenas;
            applied += 1;
            applied_fixes.push((code.to_string(), fix_kind.to_string(), message.to_string()));
            applied_this_pass = true;
            break;
        }
        if !applied_this_pass {
            break;
        }
    }

    let repaired = current;

    // The applied-fixes report, as a JSON array of `{code, kind, message}` (empty when nothing applied) —
    // so an agent driving `fix` sees exactly WHICH faults were repaired. Emitted to stdout REGARDLESS of
    // the write mode; `--diff`/`--dry-run` still preview the text below, `--json` just replaces the human
    // "applied N" line. Printed here (before the write) so a write failure doesn't suppress the report.
    let emit_json_report = || {
        use cadenza_syntax::query::json;
        let mut arr = json::Array::new();
        for (code, kind, message) in &applied_fixes {
            let mut o = json::Object::new();
            o.string("code", code);
            o.string("kind", kind);
            o.string("message", message);
            arr.raw(&o.finish());
        }
        println!("{}", arr.finish());
    };

    // `--json` selects the machine report as the OUTPUT SHAPE (over `--diff`'s unified diff / `--dry-run`'s
    // full text / the human "applied N" line). It still respects the WRITE MODE: with `--diff`/`--dry-run`
    // the file is not written (a preview), otherwise the repaired text is written back — the report just
    // says what changed either way.
    if args.json {
        if !args.diff && !args.dry_run && applied > 0
            && let Err(e) = std::fs::write(&args.file, &repaired)
        {
            eprintln!("{PROG}: writing {}: {e}", args.file);
            return ExitCode::FAILURE;
        }
        emit_json_report();
        return ExitCode::SUCCESS;
    }

    if applied == 0 {
        eprintln!(
            "{PROG}: {}: no applicable fixes ({} candidate fix(es) considered)",
            args.file,
            if considered_any { "some" } else { "0" },
        );
        return ExitCode::SUCCESS;
    }

    if args.diff {
        print!(
            "{}",
            cadenza_syntax::query::diff::unified(&source, &repaired, &args.file, &args.file)
        );
        return ExitCode::SUCCESS;
    }
    if args.dry_run {
        print!("{repaired}");
        return ExitCode::SUCCESS;
    }
    if let Err(e) = std::fs::write(&args.file, &repaired) {
        eprintln!("{PROG}: writing {}: {e}", args.file);
        return ExitCode::FAILURE;
    }
    eprintln!("{PROG}: {}: applied {applied} fix(es)", args.file);
    ExitCode::SUCCESS
}

/// `cdz def FILE OFFSET` — go-to-definition. Resolves the offset to the reference node (shared
/// `SpanTable::node_at_offset`), drives the compiler's `ResolveOf { node }` query to the defining
/// occurrence's node id, and maps THAT to a source `file:line:col`. The offset→node and id→location
/// mapping stay at the boundary (span-owning); the compiler answers by node identity.
fn run_def(args: &DefArgs) -> ExitCode {
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
        rcdzc::Request::Query(rcdzc::sidecar::Query::ResolveOf { node: node.0 }),
    );
    let Some(bytes) = out.artifact(rcdzc::sidecar::KIND_RESOLVE) else {
        report_errors(&out);
        return ExitCode::FAILURE;
    };
    let text = String::from_utf8_lossy(bytes);
    // The result is the defining occurrence's node id (empty = not a navigable reference).
    let Some(target) = text.trim().parse::<u32>().ok() else {
        eprintln!(
            "{PROG}: no definition for the token at byte offset {} in {}",
            args.offset, args.file
        );
        return ExitCode::FAILURE;
    };
    match spans.get(cadenza_syntax::StructId(target)) {
        Some(span) => {
            let (line, col) = cadenza_syntax::query::driver::line_col(&source, span.start);
            println!("{}:{line}:{col}", args.file);
            ExitCode::SUCCESS
        }
        // The definition has no source span (a prelude/built-in binding) — nothing to jump to.
        None => {
            eprintln!("{PROG}: the definition is a built-in (no source location)");
            ExitCode::FAILURE
        }
    }
}

/// `cdz scope FILE OFFSET` — variable scope tracking. Resolves the offset to a node, drives the
/// compiler's `ScopeAt { node }` query (every visible binding + its type + binder node id), and prints
/// each as `file:line:col: name : type` (innermost first — nearest enclosing binder). What an editor's
/// autocomplete / scope panel rides on.
fn run_scope(args: &ScopeArgs) -> ExitCode {
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
        rcdzc::Request::Query(rcdzc::sidecar::Query::ScopeAt { node: node.0 }),
    );
    let Some(bytes) = out.artifact(rcdzc::sidecar::KIND_SCOPE) else {
        report_errors(&out);
        return ExitCode::FAILURE;
    };
    let text = String::from_utf8_lossy(bytes);
    if text.trim().is_empty() {
        eprintln!(
            "{PROG}: no bindings in scope at byte offset {}",
            args.offset
        );
        return ExitCode::SUCCESS;
    }
    // Each line is `name<TAB>type<TAB>binder-node-id`; map the binder node to its source location.
    // One line-start index (binary-searched line:col) so many bindings stay linear, not O(bindings×len).
    let index = cadenza_syntax::query::driver::LineIndex::new(&source);
    for line in text.lines() {
        let mut cols = line.splitn(3, '\t');
        let (name, ty, binder) = match (cols.next(), cols.next(), cols.next()) {
            (Some(n), Some(t), Some(b)) => (n, t, b),
            _ => continue,
        };
        let loc = match binder
            .parse::<u32>()
            .ok()
            .and_then(|b| spans.get(cadenza_syntax::StructId(b)))
        {
            Some(span) => {
                let (l, c) = index.line_col(&source, span.start);
                format!("{}:{l}:{c}", args.file)
            }
            None => args.file.clone(),
        };
        println!("{loc}: {name} : {ty}");
    }
    ExitCode::SUCCESS
}

/// `cdz exports FILE` — the module's exported interface. Drives `Query::Exports` (each exported name +
/// its type + the def's name node), and prints `file:line:col: name : type` per export. The
/// module-interface-at-a-glance view.
fn run_exports(args: &ExportsArgs) -> ExitCode {
    let (source, arenas, spans) = match load_program_spanned(&args.file) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{PROG}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let out = run_sidecar(
        &arenas,
        rcdzc::Request::Query(rcdzc::sidecar::Query::Exports),
    );
    let Some(bytes) = out.artifact(rcdzc::sidecar::KIND_EXPORTS) else {
        report_errors(&out);
        return ExitCode::FAILURE;
    };
    let text = String::from_utf8_lossy(bytes);
    if text.trim().is_empty() {
        eprintln!("{PROG}: {} exports nothing", args.file);
        return ExitCode::SUCCESS;
    }
    // Each line is `name<TAB>type<TAB>def-name-node-id` (`-` when the export names no def).
    // One line-start index (binary-searched line:col) so a wide export list stays linear.
    let index = cadenza_syntax::query::driver::LineIndex::new(&source);
    for line in text.lines() {
        let mut cols = line.splitn(3, '\t');
        let (name, ty, node) = match (cols.next(), cols.next(), cols.next()) {
            (Some(n), Some(t), Some(d)) => (n, t, d),
            _ => continue,
        };
        let loc = match node
            .parse::<u32>()
            .ok()
            .and_then(|d| spans.get(cadenza_syntax::StructId(d)))
        {
            Some(span) => {
                let (l, c) = index.line_col(&source, span.start);
                format!("{}:{l}:{c}", args.file)
            }
            None => args.file.clone(),
        };
        println!("{loc}: {name} : {ty}");
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

/// Whether a program file is the ML surface (`.cdz`/`.ml`) vs s-expressions (`.sexp`/`.sexpr`) — the
/// one place the surface is inferred from the extension, shared by `load_program_spanned` and the
/// `check --verify-fixes` re-parse (which must re-read the edited text in the SAME surface).
fn is_ml_source(file: &str) -> bool {
    file.ends_with(".cdz") || file.ends_with(".ml")
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
    let is_ml = is_ml_source(file);
    if is_ml {
        let parsed = cadenza_syntax::parser::read_ml(&source);
        // Render each recovered parse error in the SAME `file:line:col: error: message` shape as the
        // semantic diagnostics — not the raw `ParseError { span: Span { … }, message: "…" }` Debug dump,
        // and not mislabeled a "warning" (a parse error leaves `<error>` placeholder nodes that will
        // cascade into spurious downstream faults, so it is a genuine error the user must fix first). The
        // parser RECOVERS (it never aborts), so several may print; the compile still proceeds over the
        // recovered arena, exactly as before — only the wording changes.
        for e in &parsed.errors {
            let (line, col) = cadenza_syntax::query::driver::line_col(&source, e.span.start);
            eprintln!("{file}:{line}:{col}: error: {}", e.message);
        }
        // CANONICALIZE the ML arena + REMAP its span table to the canonical ids. The ML reader builds
        // nodes in a non-canonical order (it parses an infix operand before the operator head), so a raw
        // `codec::encode` (which canonicalizes — `canon.rs`) re-indexes the arena and the compiler's
        // decoded node ids no longer match the pre-canonical span table. Every downstream query
        // (`check`/`fix` fix byte-ranges, `type-at`, `def`, `scope`) maps a COMPILER node id through this
        // table, so it must be keyed by the CANONICAL ids. Do it once here, at the load boundary, so both
        // surfaces hand the rest of the tool canonical arenas + matching spans. (The s-expr reader already
        // builds canonically, so its path is unchanged.)
        let (arenas, id_map) = cadenza_syntax::canon::canonicalize_with_map(&parsed.arenas);
        let spans = parsed.spans.remap(&id_map, arenas.structure.len());
        Ok((source, arenas, spans))
    } else {
        // Mirror the driver's root convention (`query::driver::load`): a SINGLE top-level form stays
        // BARE (so a lone `(module …)`/`(def …)` is the root the compiler scans), and only MULTIPLE
        // forms wrap in a synthetic `(do …)`. `read_spanned` succeeds iff there's exactly one form
        // (it errors on trailing input); fall back to `read_all_spanned` for several. Using
        // `read_all_spanned` unconditionally would wrap a lone `(module …)` in `(do …)`, and the
        // compiler's top-level scan would then see the module as one opaque item and find no defs.
        let (raw_arenas, raw_spans) = match cadenza_syntax::sexpr::read_spanned(&source) {
            Ok(pair) => pair,
            Err(_) => cadenza_syntax::sexpr::read_all_spanned(&source)
                .map_err(|e| format!("{file}: {}", e.0))?,
        };
        // CANONICALIZE + REMAP the span table to canonical ids — the SAME step the ML branch does, and
        // for the same reason: `run_sidecar`/`compile` feed the arena through `codec::encode`, which
        // canonicalizes (`canon.rs`), so the compiler reports diagnostics against CANONICAL node ids. The
        // s-expr reader builds a LONE form canonically (so a single-def file needed no remap and looked
        // fine), but the MULTI-form fallback wraps the roots in a synthetic `(do …)` whose head is built
        // LAST — canonicalization then reorders the ids, and an un-remapped span table maps every
        // diagnostic (and every `check --fix` byte range) to a NEIGHBOUR's span. That mis-anchored a
        // warning's fix onto the wrong node — e.g. `(def (f x y) x) (export f)`'s unused-`y` fix landed on
        // the whole `(f x y)` param list, so applying it produced `(def _y x)` and DESTROYED the function.
        // Remapping here keys the table by canonical ids on BOTH surfaces (a single form's identity map is
        // a no-op, so the previously-correct lone-form case is unchanged).
        let (arenas, id_map) = cadenza_syntax::canon::canonicalize_with_map(&raw_arenas);
        let spans = raw_spans.remap(&id_map, arenas.structure.len());
        Ok((source, arenas, spans))
    }
}

/// The source surface implied by a file's extension (`.cdz`/`.ml` → ML, else s-expr). The `Format` the
/// structural rewriter reprints a changed node in, and the surface `reparse_*` read.
fn surface_of(file: &str) -> cadenza_syntax::convert::Format {
    if is_ml_source(file) {
        cadenza_syntax::convert::Format::Ml
    } else {
        cadenza_syntax::convert::Format::Sexpr
    }
}

/// Parse `text` in `surface` into a canonicalized arena + its remapped span table — the surface-parse
/// half of [`load_program_spanned`], factored so `cdz fix` can RE-parse its own edited text between fixes
/// (node ids shift after each structural edit). `None` if the text does not parse cleanly (an ML parse
/// error, or an s-expr read failure) — the caller stops applying further fixes.
fn reparse_spans(
    text: &str,
    surface: cadenza_syntax::convert::Format,
) -> Option<cadenza_syntax::spans::SpanTable> {
    match surface {
        cadenza_syntax::convert::Format::Ml => {
            let parsed = cadenza_syntax::parser::read_ml(text);
            if !parsed.errors.is_empty() {
                return None;
            }
            let (arenas, id_map) = cadenza_syntax::canon::canonicalize_with_map(&parsed.arenas);
            Some(parsed.spans.remap(&id_map, arenas.structure.len()))
        }
        _ => match cadenza_syntax::sexpr::read_spanned(text) {
            Ok((_, spans)) => Some(spans),
            Err(_) => cadenza_syntax::sexpr::read_all_spanned(text)
                .ok()
                .map(|(_, spans)| spans),
        },
    }
}

/// Parse `text` in `surface` into the canonical arena (the compiler's input) — the arena half of
/// [`reparse_spans`], for driving the next `Diagnostics` pass over `cdz fix`'s edited text. `None` on a
/// parse failure.
fn reparse_arenas(
    text: &str,
    surface: cadenza_syntax::convert::Format,
) -> Option<cadenza_syntax::Arenas> {
    match surface {
        cadenza_syntax::convert::Format::Ml => {
            let parsed = cadenza_syntax::parser::read_ml(text);
            if !parsed.errors.is_empty() {
                return None;
            }
            Some(cadenza_syntax::canon::canonicalize_with_map(&parsed.arenas).0)
        }
        _ => match cadenza_syntax::sexpr::read_spanned(text) {
            Ok((arenas, _)) => Some(arenas),
            Err(_) => cadenza_syntax::sexpr::read_all(text).ok(),
        },
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
    // One line-start index (binary-searched line:col) so a query with many matches stays linear rather
    // than O(matches × source_len) — the same from-start-scan O(N²) the `uses`/`clones` paths had.
    let index = cadenza_syntax::query::driver::LineIndex::new(&source);
    for m in kept {
        let loc = match m.span {
            Some(s) => {
                let (l, c) = index.line_col(&source, s.start);
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

    #[test]
    fn debug_module_path_strips_the_build_directory() {
        // An ABSOLUTE path leaks the build directory into DWARF (breaking cross-machine determinism,
        // DESIGN-debug-info-rcdzc.md §4) — reduced to its file name.
        assert_eq!(debug_module_path("/home/alice/proj/add.sexp"), "add.sexp");
        assert_eq!(debug_module_path("/tmp/dw_f.sexp"), "dw_f.sexp");
        // A relative path is already tree-relative — kept verbatim (a leading `./` is dropped).
        assert_eq!(debug_module_path("src/app.cdz"), "src/app.cdz");
        assert_eq!(debug_module_path("./app.cdz"), "app.cdz");
        assert_eq!(debug_module_path("app.cdz"), "app.cdz");
    }

    #[test]
    fn debug_module_path_is_deterministic_across_build_dirs() {
        // The same source file compiled from two different absolute locations yields the SAME module
        // path — the property that makes the DWARF byte-reproducible regardless of where it was built.
        assert_eq!(
            debug_module_path("/build/a/prog.sexp"),
            debug_module_path("/elsewhere/b/prog.sexp"),
        );
    }

    /// A throwaway directory unique to `tag`, created empty. The caller populates + removes it.
    fn tmp(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("cdz-expand-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    #[test]
    fn expand_recurses_a_directory_collecting_only_source_files() {
        let dir = tmp("recurse");
        std::fs::create_dir_all(dir.join("lib/math")).unwrap();
        std::fs::write(dir.join("app.sexp"), "(do (def (main) 1) (export main))").unwrap();
        std::fs::write(dir.join("lib/helper.sexp"), "(do (def (h) 2) (export h))").unwrap();
        std::fs::write(dir.join("lib/math/base.cdz"), "def base() = 3").unwrap();
        std::fs::write(dir.join("README.md"), "not source").unwrap();
        std::fs::write(dir.join("lib/notes.txt"), "skip me").unwrap();

        let out = expand_input_specs(&[dir.to_string_lossy().into_owned()]).unwrap();
        // Only the three source files, path-sorted; the README + .txt are skipped.
        let names: Vec<String> = out
            .iter()
            .map(|p| program_name(p)) // file stem, order-preserving
            .collect();
        assert_eq!(names, vec!["app", "helper", "base"], "got {out:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn expand_passes_through_files_and_artifact_specs() {
        // A plain file, a `kind:name=path` artifact spec, and `-` are NOT directories → verbatim.
        let specs = vec![
            "app.sexp".to_string(),
            "spans:m=x.spans".to_string(),
            "-".to_string(),
        ];
        let out = expand_input_specs(&specs).unwrap();
        assert_eq!(out, specs);
    }

    #[test]
    fn expand_errors_on_a_directory_with_no_source_files() {
        let dir = tmp("nosrc");
        std::fs::write(dir.join("README.md"), "no source here").unwrap();
        let err = expand_input_specs(&[dir.to_string_lossy().into_owned()]).unwrap_err();
        assert!(err.contains("no source files"), "got {err}");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A MULTI-form s-expr program's diagnostic (and its fix) must anchor to the RIGHT source bytes. The
    /// multi-form fallback wraps the roots in a synthetic `(do …)` whose head is built last;
    /// `codec::encode` canonicalizes and reorders the ids, so an un-remapped span table maps a diagnostic
    /// to a NEIGHBOUR's span. This regression pins the fix: the unused-`y` warning's fix span must cover
    /// exactly `y` — not the `(f x y)` param list, which once made `cdz fix` rewrite the file to
    /// `(def _y x)` and destroy the function. Exercised through `load_program_spanned` (the load boundary
    /// that does the canonicalize + remap) so it guards both surfaces' span mapping.
    #[test]
    fn a_multi_form_sexpr_diagnostic_fix_anchors_to_the_right_bytes() {
        let dir = tmp("multiform");
        let file = dir.join("p.sexp");
        let src = "(def (f x y) x) (export f)";
        std::fs::write(&file, src).unwrap();
        let (source, arenas, spans) = load_program_spanned(&file.to_string_lossy()).expect("loads");
        let out = run_sidecar(
            &arenas,
            rcdzc::Request::Query(rcdzc::sidecar::Query::Diagnostics),
        );
        let bytes = out
            .artifact(rcdzc::sidecar::KIND_DIAGNOSTICS)
            .expect("a diagnostics artifact");
        let text = String::from_utf8_lossy(bytes);
        // Find the CDZ0306 line; its fix-node (column 5) span must cover exactly the binder `y`.
        let mut checked = false;
        for line in text.lines() {
            let cols: Vec<&str> = line.splitn(8, '\t').collect();
            if cols.get(1) == Some(&"CDZ0306") {
                let fix_node = cols[4];
                assert_ne!(
                    fix_node, "-",
                    "the unused-binding fix carries a target node"
                );
                let n: u32 = fix_node.parse().expect("a node id");
                let span = spans
                    .get(cadenza_syntax::StructId(n))
                    .expect("the fix node has a span");
                assert_eq!(
                    &source[span.start..span.end],
                    "y",
                    "the unused-`y` fix must anchor on `y`, not a neighbour node (the do-wrap span bug)"
                );
                checked = true;
            }
        }
        assert!(checked, "expected a CDZ0306 unused-binding warning: {text}");
        std::fs::remove_dir_all(&dir).ok();
    }
}
