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
//! `cdz run` is MOUNTED here too (from the `cdz-run` lib) — one binary on the PATH is the operator's
//! headline requirement, so the wasmtime + runtime-store weight rides along rather than living in a
//! separate `cdz-run` bin the user must also install. The standalone `cdz-run` bin remains as a thin
//! shim over the same `cdz_run::cli` code (so existing call sites keep working); both share one impl.

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process::ExitCode;

use cadenza_syntax::cli as syntax_cli;
use rcdzc::cli as compiler_cli;

// The LSP server (`cdz lsp`) — its own module so `main.rs` gains only the `Cmd::Lsp` arm + dispatch,
// keeping the server implementation (owned by the v-lsp vertical) out of the shared command file.
mod lsp;

// The structural fix-application engine, shared by `cdz fix` / `cdz check --json` / `cdz lsp` codeAction.
mod fix;
use fix::{FileTree, OriginPaths, apply_fix_to_source, fix_edits};

// The import-closure loader, shared by `cdz check`/… and `cdz lsp` (cross-file analysis).
mod closure;
use closure::{declared_import_paths, load as load_import_closure_with};

/// The unified tool. The name reported in tool-level diagnostics is `cdz`.
const PROG: &str = "cdz";

#[derive(Parser)]
#[command(
    name = "cdz",
    // A real toolchain reports its version — `cdz --version`/`-V` prints the crate version, so a bug
    // report or a script can pin which build it is talking to. (Pulled from `CARGO_PKG_VERSION`.)
    version,
    about = "The Cadenza toolchain: scaffold, build, run, test, and inspect a project — one tool.",
    long_about = "cdz is the Cadenza toolchain in one binary — the cargo-analogue project workflow \
                  plus the front-end and compiler.\n\n\
                  PROJECT (over a Project.cdz manifest; each finds the nearest one upward when given no \
                  argument): `new`/`init` scaffold a project, `build` compiles it, `run` builds + runs it, \
                  `test` runs its @test suite, `check` reports diagnostics, `metadata` prints it as JSON, \
                  `clean` removes build artifacts.\n\n\
                  PROGRAM: `compile`/`run` a single file, `convert` between surfaces, `fmt`/`query`/\
                  `rewrite`/`lint` the structure, and the span-mapped compiler queries (`type`, `uses`, \
                  `check`, `def`, …) only a single process holding both the front-end and compiler can \
                  answer. Also `calc` (the exact calculator REPL) and `completions` (shell completions)."
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
    /// Format program file(s) in place: reprint each canonically in its OWN surface (`--check`/`--diff`).
    /// With NO file argument (or a `Project.cdz` / a directory holding one), formats the whole PROJECT —
    /// the manifest's `entry`+`modules`+`tests` — like `cdz build`/`test`/`check`. A lone `-` reads stdin.
    Fmt(syntax_cli::FmtArgs),
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

    // ── run (cdz-run) ───────────────────────────────────────────────────────────────────────────
    /// Run a finished wasm component: link it (resolving its value-heap runtime by content address from
    /// the store), call an export (the sole function export by default), and print the rendered result.
    /// A trap or error goes to stderr with a non-zero exit. Folded in from the `cdz-run` bin so a single
    /// `cdz` on the PATH both compiles and runs (`cdz compile foo.cdz -o - | cdz run -`).
    Run(cdz_run::cli::RunArgs),

    // ── corpus (cdz-corpus) ─────────────────────────────────────────────────────────────────────
    /// Read + migrate the executable-semantics corpus (`records`/`migrate`/`check`) — the maintenance
    /// tool for `spec/semantics/*.sexp`. Folded in from the `cdz-corpus` bin so it needn't be a separate
    /// binary on the PATH. `cdz corpus records FILE…` emits the flat record stream the gate consumes;
    /// `migrate` projects a `.sexp` corpus to literate markdown; `check` proves a migration is
    /// behaviour-preserving.
    Corpus(cdz_corpus::cli::CorpusArgs),

    // ── calc (cdz-calc) ─────────────────────────────────────────────────────────────────────────
    /// The calculator REPL over the real language, exact by construction. `cdz calc` starts the
    /// interactive loop; `cdz calc --once "<expr>"` computes one line and exits (the launcher/script
    /// hook). Variables accumulate and `ans` recalls the last result; `--plain` prints the bare value,
    /// `--sexpr` reads the s-expression surface, `--no-exact` turns off forced rationals. Folded in from
    /// the `cdz-calc` bin so a single `cdz` on the PATH also gives the calculator.
    Calc(cdz_calc::cli::CalcArgs),

    // ── project build ───────────────────────────────────────────────────────────────────────────
    /// Build a PROJECT from its `Project.cdz` manifest (the `cargo build` analogue): compile the
    /// manifest's `entry` file (plus its `modules`) into one wasm component, with NO per-run flags —
    /// the manifest tells `cdz` what to compile. `cdz build` with no argument searches up for the
    /// nearest `Project.cdz` (like `cargo build` finding `Cargo.toml`); `cdz build <dir>` or `cdz build
    /// path/to/Project.cdz` builds that project. For a single loose file, use `cdz compile <file>`.
    Build(BuildArgs),

    // ── project metadata ──────────────────────────────────────────────────────────────────────────
    /// Print the resolved project manifest as JSON (the `cargo metadata` analogue) — a machine-readable
    /// description of the project for editors, build tools, and scripts. Resolves the same `Project.cdz`
    /// as `cdz build`/`cdz test` and emits one object: the manifest's raw fields (`name`, `entry`,
    /// `opt_level`, and the `modules`/`tests`/`exclude` PATTERNS) plus their RESOLVED glob-expanded file
    /// sets (`entry_file`, `module_files`, `test_files`), so a consumer sees both intent and concrete
    /// files without re-implementing glob resolution.
    Metadata(MetadataArgs),

    // ── project clean ─────────────────────────────────────────────────────────────────────────────
    /// Remove the build artifacts a `cdz build`/`cdz run` produces (the `cargo clean` analogue): the
    /// project's `<entry>.wasm`/`.rs`/`.dwarf`, `link-map.txt`, and any leftover `cdz run` temp component,
    /// in the manifest directory. PRECISE — derived from the entry name, so a source file or an unrelated
    /// `.wasm` is never touched. `--dry-run` lists what would be removed. `cdz clean` with no argument
    /// searches up for the nearest `Project.cdz`.
    Clean(CleanArgs),

    // ── project scaffold ────────────────────────────────────────────────────────────────────────
    /// Scaffold a new PROJECT (the `cargo new` analogue): create `<name>/` with a `Project.cdz`
    /// manifest naming the entry, and a minimal buildable entry file. `cdz new my-app` then `cd my-app
    /// && cdz build` compiles it. Refuses to overwrite a non-empty directory. `--sexpr` scaffolds the
    /// s-expression surface instead of ML.
    New(NewArgs),

    // ── project init (adopt an existing directory) ──────────────────────────────────────────────
    /// Scaffold a project INTO an existing directory (the `cargo init` analogue): write a `Project.cdz`
    /// manifest + a minimal buildable entry into the directory (default: the current one), WITHOUT
    /// creating a new subdirectory. Complements `cdz new <name>` — use `init` to adopt a directory you're
    /// already in (`cdz init` then `cdz build`). Refuses only if a `Project.cdz` already exists (never
    /// overwrites a manifest); other files are left untouched. `--sexpr` scaffolds the s-expr surface.
    Init(InitArgs),

    // ── shell completions ───────────────────────────────────────────────────────────────────────
    /// Print a shell COMPLETION script for `cdz` to stdout — `cdz completions <shell>` for bash, zsh,
    /// fish, elvish, or powershell. Generated from the actual command tree, so it always matches the
    /// current subcommands + flags. Install by sourcing/placing per your shell (e.g.
    /// `cdz completions bash > /etc/bash_completion.d/cdz`, or `cdz completions zsh > _cdz` on `$fpath`).
    Completions(CompletionsArgs),

    // ── toolchain health ────────────────────────────────────────────────────────────────────────
    /// Diagnose the `cdz` TOOLCHAIN environment (a `cargo`-doctor-style preflight): the `cdz` version +
    /// path, whether the sibling `cdz-run` binary is present (needed to run/`test` compiled components),
    /// and whether the value-heap runtime store holds the runtime `cdz` compiles against (needed to run a
    /// program that builds heap values). Exits non-zero if a component that would break `cdz run`/`test`
    /// is missing — so CI/setup scripts can gate on `cdz doctor`. `--store <DIR>` checks a specific store.
    Doctor(DoctorArgs),

    // ── unit testing ─────────────────────────────────────────────────────────────────────────────
    /// Compile a SEPARATE test component from a FILE's `@test`-marked NULLARY definitions and run each,
    /// reporting pass/fail. Each `@test def` crosses the boundary as a nullary entry the runner invokes;
    /// a test that RETURNS (unit) PASSES, one that TRAPS FAILS (an assertion emits its message via a host
    /// effect, then traps). The report/host effect is compiled in ONLY here — a normal `cdz compile` build
    /// never carries it. Shells out to the sibling `cdz-run` binary to execute each test (this bin holds
    /// no wasm engine). Exits non-zero if any test fails.
    Test(TestArgs),

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
    /// The document OUTLINE of FILE: every top-level declaration (value/function/type/effect/module)
    /// classified by kind, as `file:line:col: kind name` — the LSP `documentSymbol` analogue. The
    /// superset companion of `cdz exports` (which lists only the exported subset): `symbols` lists EVERY
    /// declaration, private ones included, so an editor can render a symbol tree / breadcrumb.
    Symbols(SymbolsArgs),
    /// SEMANTIC SYNTAX HIGHLIGHTING for FILE: every token CLASSIFIED by the role it plays (type vs
    /// constructor vs local vs call vs unbound), as `file:line:col: kind` — the LSP `semanticTokens`
    /// analogue, coloured by MEANING (the compiler's columns) rather than by spelling.
    Highlight(HighlightArgs),
    /// The documentation of a definition NAME in FILE — its `(doc "…")` text, or a built-in's
    /// documentation (a prelude module's `(meta doc)` channel, or a grammar keyword's help) when the
    /// name is not a user definition. The doc companion of `cdz type`.
    Doc(DocArgs),
    /// The documentation of the definition at a source BYTE OFFSET in FILE — a "documentation at cursor"
    /// hover. Resolves the offset to a node, then to the definition it is or references, and prints that
    /// definition's `(doc "…")` text. The doc companion of `cdz type-at`/`cdz def`.
    DocAt(DocAtOffsetArgs),
    /// Every CONCRETE INSTANTIATION of a generic / ad-hoc-polymorphic definition NAME in FILE — the
    /// monomorphized functions one source definition becomes. Reports each specialization's concrete
    /// arguments (a recursive generic at each element type, a type-valued-parameter def at each type, and
    /// a `const` dictionary parameter at each concrete dictionary — the ad-hoc-polymorphism case).
    Instantiations(InstantiationsArgs),

    // ── editor integration ────────────────────────────────────────────────────────────────────────
    /// Run a Language Server (LSP) over stdio — the persistent editor face of the in-process query
    /// engine. An editor launches `cdz lsp` and speaks the Language Server Protocol; the server holds
    /// each open document in memory and republishes its diagnostics on every edit ("diagnostics as you
    /// type"), reusing the SAME compiler queries the one-shot subcommands drive. No arguments — it
    /// communicates only over stdin/stdout.
    Lsp,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        // Front-end commands defer to the syntax CLI, reconstructing its command enum (its arg structs
        // are re-exported, so `cdz convert …` and `cdz-syntax convert …` run the SAME code).
        Cmd::Convert(a) => syntax_cli::run(syntax_cli::Cmd::Convert(a), PROG),
        Cmd::Fmt(a) => run_fmt(a),
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
        // `cdz run` — mounted from the `cdz-run` lib; the same code the standalone `cdz-run` bin runs.
        // When the `component` arg is a PROJECT (a `Project.cdz` or a directory holding one), `cdz`
        // BUILDS the manifest's entry first (the `cargo run` analogue), then runs the produced component;
        // otherwise it runs the given `.wasm`/stdin component directly.
        Cmd::Run(a) if run_target_is_project(a.component.as_deref()) => run_project(&a),
        Cmd::Run(a) => cdz_run::cli::run(&a, PROG),
        // `cdz corpus` — mounted from the `cdz-corpus` lib; the same code the standalone bin runs.
        Cmd::Corpus(a) => cdz_corpus::cli::run(&a, PROG),
        // `cdz calc` — mounted from the `cdz-calc` lib; the same code the standalone `cdz-calc` bin runs.
        Cmd::Calc(a) => cdz_calc::cli::run(&a, PROG),
        Cmd::Build(a) => run_build(&a),
        Cmd::Metadata(a) => run_metadata(&a),
        Cmd::Clean(a) => run_clean(&a),
        Cmd::New(a) => run_new(&a),
        Cmd::Init(a) => run_init(&a),
        Cmd::Completions(a) => run_completions(&a),
        Cmd::Doctor(a) => run_doctor(&a),
        Cmd::Test(a) => run_test(&a),
        // The span-mapped semantic queries live here (they need both libraries in one process).
        Cmd::Type(a) => run_type(&a),
        Cmd::TypeAt(a) => run_type_at(&a),
        Cmd::Uses(a) => run_uses(&a),
        Cmd::Check(a) => run_check(&a),
        Cmd::Fix(a) => run_fix(&a),
        Cmd::Def(a) => run_def(&a),
        Cmd::Scope(a) => run_scope(&a),
        Cmd::Exports(a) => run_exports(&a),
        Cmd::Symbols(a) => run_symbols(&a),
        Cmd::Highlight(a) => run_highlight(&a),
        Cmd::Doc(a) => run_doc(&a),
        Cmd::DocAt(a) => run_doc_at(&a),
        Cmd::Instantiations(a) => run_instantiations(&a),
        Cmd::Lsp => run_lsp(),
    }
}

/// `cdz lsp` — run the stdio Language Server to completion. Returns FAILURE only on a transport-level
/// error (a broken stream); a clean client shutdown is SUCCESS. The server itself never fails on a bad
/// buffer — a query is total (an un-analyzable document yields empty diagnostics, never a crash).
fn run_lsp() -> ExitCode {
    match lsp::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{PROG} lsp: {e}");
            ExitCode::FAILURE
        }
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
    let mut specs = match expand_input_specs(args.input_specs()) {
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

    // FOLLOW A SINGLE FILE'S IMPORT CLOSURE — the same resolution `cdz check`/`cdz test` do. A lone
    // source file that `(import …)`s a sibling was compiled ALONE, so the compiler hit the import as an
    // unmodeled module form AND the bare imported name fell back to a BUILT-IN (e.g. `Ast` → the
    // metaprog Ast), yielding a misleading cascade (a match over the imported sum reported the built-in's
    // variants "not covered"). So `cdz compile app.cdz` FLIPPED a program's meaning vs `cdz check app.cdz`
    // (v-compiler-ml stress finding). When the SOLE input is a source file with NO explicit `--entry` and
    // it declares imports, expand it to its transitive import closure (entry = that file) and compile the
    // package — resolving the import exactly as check does. A file with no imports, a `--entry` already
    // given, or multiple inputs is untouched (byte-identical to before).
    let mut entry_from_closure: Option<String> = None;
    if args.entry().is_none() {
        let sources: Vec<&String> = specs.iter().filter(|s| is_source_file(s)).collect();
        if let [only] = sources.as_slice() {
            match load_import_closure_with(only, &|_| None) {
                Ok(files) if !declared_import_paths(&files[0].arenas).is_empty() => {
                    // The entry names the package boundary; the closure files (entry first) become the
                    // compile inputs, replacing the lone spec.
                    entry_from_closure = Some(files[0].name.clone());
                    specs = files.into_iter().map(|f| f.path).collect();
                }
                // No imports (or the closure loaded only the entry with none declared) → leave `specs`
                // as-is: the single-file path below, unchanged. A load error here is non-fatal — the
                // per-spec parse below reports it with the same message.
                Ok(_) | Err(_) => {}
            }
        }
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
    // `KIND_ENTRY` artifact, exactly as the artifacts-in `run` path does. When no `--entry` was given but
    // a single file's import closure was followed (above), that file is the entry.
    if let Some(entry) = args.entry() {
        inputs.push(compiler_cli::entry_artifact(entry));
    } else if let Some(entry) = &entry_from_closure {
        inputs.push(compiler_cli::entry_artifact(entry));
    }
    // A `--component-name <INTERFACE>` names the interface a cross-component PROVIDER publishes its exports
    // under — inject it as a `KIND_COMPONENT_NAME` artifact (X4b), same as the artifacts-in `run` path.
    if let Some(iface) = args.component_name() {
        inputs.push(compiler_cli::component_name_artifact(iface));
    }
    // Thread the requested `--opt-level` (default `O1`) through to the compile — `cdz compile
    // --opt-level O2 foo.cdz` selects the release pass tier, same as the artifacts-in `rcdzc` path.
    compiler_cli::run_prepared(inputs, &targets, args.out_path(), args.opt_level(), PROG)
}

/// Compile a set of SOURCE-file `specs` (already directory-expanded) into a wasm component, with the
/// package `entry` named and output written to `out` — the in-process source-compile core `cdz build`
/// drives (a manifest-resolved package) and the shared spine `run_compile` uses for a source input.
/// Each spec is parsed keeping spans (so a diagnostic locates as `path:line:col`), the `entry` becomes a
/// `KIND_ENTRY` artifact, `targets` names the backend(s) to emit (empty ⇒ the `[Wasm]` default), and
/// `opt_level` selects the optimization tier. Returns the process exit code.
fn compile_source_specs(
    specs: &[String],
    entry: Option<&str>,
    out: Option<PathBuf>,
    targets: &[rcdzc::Target],
    opt_level: rcdzc::OptLevel,
) -> ExitCode {
    let mut inputs: Vec<rcdzc::Artifact> = Vec::new();
    for spec in specs {
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
        let span_data = span_data_of(spec, &source, &spantable);
        inputs.push(rcdzc::Artifact::new(
            rcdzc::spans::KIND_SPANS,
            name,
            rcdzc::spans::encode(&span_data),
        ));
    }
    if let Some(entry) = entry {
        inputs.push(compiler_cli::entry_artifact(entry));
    }
    // `run_prepared` applies the `[Wasm]` default when `targets` is empty, matching a bare `cdz compile`.
    // `opt_level` is the resolved build tier.
    compiler_cli::run_prepared(inputs, targets, out, opt_level, PROG)
}

/// `cdz build [DIR]` — the manifest-driven compile (the `cargo build` analogue). Resolves the project's
/// `Project.cdz` (the `DIR` arg naming the manifest or a directory holding one; OMITTED → search up from
/// the cwd, like `cdz test`), then compiles the manifest's `entry` file together with its `modules` into
/// one wasm component — so a project builds with NO per-run flags, its manifest telling `cdz` what to
/// compile. The `entry`/`modules` may be globs, expanded (path-sorted, `exclude`-filtered) relative to
/// the manifest dir — the same resolution `cdz test` gives `tests`. `entry` is required (there is no
/// component without a boundary file); `modules` are the libraries it may `(import …)`.
fn run_build(args: &BuildArgs) -> ExitCode {
    let project = match resolve_project_specs(args.dir.as_deref(), "cdz build") {
        Ok(p) => p,
        Err(code) => return code,
    };
    // Resolve the optimization tier by PRECEDENCE (v-core-opt design §7): an explicit `--opt-level` wins;
    // else the manifest's `def opt-level`; else `--release` (`O2`); else the default (`O1`). A bad string
    // (flag or manifest) is a clear error naming the valid set rather than a silent fallback.
    let opt_level =
        match resolve_build_opt_level(args, project.m.opt_level.as_deref(), &project.mpath) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("{PROG}: {e}");
                return ExitCode::FAILURE;
            }
        };
    let targets = [rcdzc::Target::from(args.target)];
    compile_source_specs(
        &project.specs,
        Some(&project.entry_name),
        args.out.clone(),
        &targets,
        opt_level,
    )
}

/// Compile a project's `specs` (entry-first) into the wasm COMPONENT BYTES in-memory — the quiet build
/// `cdz run <project>` uses. Unlike [`compile_source_specs`] (which writes the artifact to a file and
/// prints `cdz: wrote <path>`), this returns the component bytes with NO file + NO notice, so a project
/// run doesn't leak its internal temp-artifact path to stderr (`cargo run` doesn't announce where it put
/// the binary). Diagnostics are still reported on failure. `Err` = a load/compile failure (already
/// printed); `Ok(None)` = compiled but produced no `component` artifact (e.g. a diagnostic-only run).
fn compile_project_component_bytes(
    specs: &[String],
    entry: &str,
    opt_level: rcdzc::OptLevel,
) -> Result<Option<Vec<u8>>, ()> {
    let mut inputs: Vec<rcdzc::Artifact> = Vec::new();
    for spec in specs {
        let (source, arenas, spantable) = match load_program_spanned(spec) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("{PROG}: {e}");
                return Err(());
            }
        };
        let name = program_name(spec);
        inputs.push(rcdzc::Artifact::new(
            rcdzc::Artifact::KIND_AST,
            name.clone(),
            cadenza_syntax::codec::encode(&arenas),
        ));
        let span_data = span_data_of(spec, &source, &spantable);
        inputs.push(rcdzc::Artifact::new(
            rcdzc::spans::KIND_SPANS,
            name,
            rcdzc::spans::encode(&span_data),
        ));
    }
    inputs.push(compiler_cli::entry_artifact(entry));
    // Compile on the compiler-stack worker (deep-recursion guard), same as `check_one`/`run_prepared`.
    let out = rcdzc::run_with_compiler_stack(|| {
        rcdzc::compile_with_opt(&inputs, &[rcdzc::Target::Wasm], opt_level)
    });
    if out.has_error() {
        report_errors(&out);
        return Err(());
    }
    // The produced WebAssembly component is the artifact of `kind == "component"`.
    Ok(out
        .artifacts
        .into_iter()
        .find(|a| a.kind == "component")
        .map(|a| a.bytes))
}

/// Is `cdz run`'s `component` argument a PROJECT (rather than a pre-built component)? True when it is
/// OMITTED (a bare `cdz run` → build+run the nearest `Project.cdz` upward, like `cargo run`), or names a
/// `Project.cdz` (the file itself) or a DIRECTORY — the forms `cdz build`/`cdz test` treat as a project.
/// A `.wasm` path, or `-` (stdin), is NOT a project → the direct run path.
fn run_target_is_project(component: Option<&std::path::Path>) -> bool {
    match component {
        None => true, // bare `cdz run` — the current-directory project
        Some(p) => p.is_dir() || p.file_name().and_then(|n| n.to_str()) == Some(MANIFEST_NAME),
    }
}

/// `cdz run <project>` — BUILD the project's manifest entry, then RUN the produced component (the `cargo
/// run` analogue). Resolves the same `Project.cdz` as `cdz build` (via [`resolve_project_specs`]),
/// compiles the entry (+ modules) to component bytes IN-MEMORY (quiet — no `cdz: wrote …` notice, so a
/// project run doesn't leak its internal temp path), writes them to a temp `.wasm` in the manifest dir for
/// the runner, then delegates to the same `cdz-run` code path the direct `cdz run <file>` uses — passing
/// through `--call`/`--arg`/`--store`/`--host-response`/`--peer` unchanged. The temp is removed after.
fn run_project(args: &cdz_run::cli::RunArgs) -> ExitCode {
    // The project target: the given `Project.cdz`/directory, or `None` (a bare `cdz run`) → an upward
    // search from the cwd, exactly as `resolve_project_specs` handles a `None` argument.
    let target = args
        .component
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned());
    let project = match resolve_project_specs(target.as_deref(), "cdz run") {
        Ok(p) => p,
        Err(code) => return code,
    };
    // The build tier by the SAME precedence `cdz build` uses (`--opt-level` > manifest `opt-level` >
    // `--release`'s O2 > the default O1) — so `cdz run --release` (or a manifest `def opt-level`) runs the
    // optimized build, matching `cargo run --release`. The dev default (no flags, no manifest level) is O1.
    let opt_level = match resolve_opt_level_precedence(
        args.opt_level.as_deref(),
        args.release,
        project.m.opt_level.as_deref(),
        &project.mpath,
    ) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("{PROG}: {e}");
            return ExitCode::FAILURE;
        }
    };
    // Build the component IN-MEMORY (quiet — no `cdz: wrote <path>` notice; `cargo run` doesn't announce
    // where it put the binary, and the temp path is an internal detail). On a build failure the diagnostics
    // are already reported.
    let bytes = match compile_project_component_bytes(
        &project.specs,
        &project.entry_name,
        opt_level,
    ) {
        Ok(Some(b)) => b,
        Ok(None) => {
            eprintln!(
                "{PROG}: the project built no runnable component (entry `{}` produced no component output)",
                project.entry_name
            );
            return ExitCode::FAILURE;
        }
        Err(()) => return ExitCode::FAILURE,
    };
    // The runner needs a component to instantiate; write the bytes to a temp file beside the manifest
    // (pid-stamped so concurrent runs don't collide), run it, then remove it. The write itself is silent.
    let out_dir = project
        .mpath
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let out_wasm = out_dir.join(format!(
        ".cdz-run-{}-{}.wasm",
        project.entry_name,
        std::process::id()
    ));
    if let Err(e) = std::fs::write(&out_wasm, &bytes) {
        eprintln!("{PROG}: writing {}: {e}", out_wasm.display());
        return ExitCode::FAILURE;
    }
    // Run the freshly-built component through the SAME `cdz-run` path as a direct `cdz run <file>`: clone
    // the parsed args, but point `component` at the built wasm (the other flags pass through unchanged).
    let mut run_args = args.clone();
    run_args.component = Some(out_wasm.clone());
    let code = cdz_run::cli::run(&run_args, PROG);
    let _ = std::fs::remove_file(&out_wasm); // best-effort cleanup of the temp artifact
    code
}

/// A project resolved from its `Project.cdz`: the manifest (`m`) + its path (`mpath`), plus the compile
/// inputs — the entry package NAME and the full `specs` list (entry file first, then modules, deduped).
struct ProjectSpecs {
    mpath: PathBuf,
    m: Manifest,
    entry_name: String,
    specs: Vec<String>,
}

/// `cdz fmt` — format program file(s), with a PROJECT mode the bare `cadenza-syntax` `fmt` lacks. Every
/// other lifecycle command (`build`/`test`/`check`/`metadata`/`clean`) acts on the whole `Project.cdz`
/// when given a directory / a manifest / no argument; `cdz fmt` now does too, so "format my project" works
/// without listing files. PROJECT mode triggers when the args name a project — a lone `Project.cdz`, a
/// DIRECTORY that holds one, or NO argument with a `Project.cdz` found upward — in which case the
/// manifest's own source set (`entry` + `modules` + `tests`, glob-expanded + `exclude`-filtered, deduped)
/// is formatted. Otherwise (explicit files, a lone `-`/stdin, or a directory with no manifest) it passes
/// through UNCHANGED to the syntax CLI — so `cdz fmt a.cdz b.cdz`, `… | cdz fmt -`, and `cdz fmt <dir>`
/// (recursing a manifest-less tree) keep their existing behavior. All the mode flags (`--check`/`--diff`/
/// `--stdout`/`--from`/`--width`) carry over via `FmtArgs::with_files`.
fn run_fmt(args: syntax_cli::FmtArgs) -> ExitCode {
    // Classify the parsed positionals (read via the `files()` getter; the field is private). PROJECT mode
    // needs a project target: no args (→ upward search), or a single arg that is a `Project.cdz` or a
    // directory containing one. A lone `-` (stdin) or any explicit file list is NOT a project → pass through.
    let files = args.files();
    let is_stdin = files.len() == 1 && files[0] == "-";
    let project_target: Option<Option<&str>> = if is_stdin {
        None // explicit stdin — never project mode
    } else if files.is_empty() {
        // No args: project mode ONLY if a `Project.cdz` exists upward (else keep the historical stdin read).
        find_manifest_upward().map(|_| None)
    } else if files.len() == 1 {
        // A single arg: project mode iff it names a `Project.cdz` or a dir holding one. A plain file or a
        // manifest-less dir is NOT a project (falls through to the syntax CLI's own file/dir handling).
        let p = std::path::Path::new(&files[0]);
        let is_manifest =
            p.file_name().and_then(|n| n.to_str()) == Some(MANIFEST_NAME) && p.is_file();
        let dir_has_manifest = p.is_dir() && p.join(MANIFEST_NAME).is_file();
        if is_manifest || dir_has_manifest {
            Some(Some(files[0].as_str()))
        } else {
            None
        }
    } else {
        None // multiple explicit files — pass through
    };
    let Some(target_arg) = project_target else {
        // Not a project target — delegate unchanged (explicit files / stdin / manifest-less dir).
        return syntax_cli::run(syntax_cli::Cmd::Fmt(args), PROG);
    };
    // PROJECT mode: resolve the manifest + format its full declared source set (entry + modules + tests).
    let (dir, mpath, m) = match resolve_project_manifest(target_arg, "cdz fmt") {
        Ok(v) => v,
        Err(code) => return code,
    };
    // Every source the manifest declares: entry, library modules, and tests — glob-expanded, exclude-
    // filtered, deduped. (Unlike build/check, fmt formats the TESTS too — they're project source.)
    let mut pats: Vec<String> = Vec::new();
    if let Some(entry) = &m.entry {
        pats.push(entry.clone());
    }
    pats.extend(m.modules.iter().cloned());
    pats.extend(m.tests.iter().cloned());
    let mut resolved = expand_manifest_globs(&dir, &pats, &m.exclude);
    let mut seen = std::collections::HashSet::new();
    resolved.retain(|s| seen.insert(s.clone()));
    if resolved.is_empty() {
        eprintln!(
            "{PROG}: {}: the manifest declares no `entry`/`modules`/`tests` source to format",
            mpath.display()
        );
        return ExitCode::FAILURE;
    }
    // Hand the resolved file list to fmt, preserving every mode flag, and delegate to the same code path.
    syntax_cli::run(syntax_cli::Cmd::Fmt(args.with_files(resolved)), PROG)
}

/// Resolve a project's `Project.cdz` to `(dir, manifest-path, manifest)` — the DIR-resolution shared by
/// every project command. `target_arg` is a manifest path, a directory holding one, or `None` → an upward
/// search from the cwd (like `cargo` finding `Cargo.toml`); `cmd` names the invoking command in the "not a
/// project" hint. Does NOT require an `entry` — a command that only needs the manifest directory (e.g.
/// `cdz clean`, which cleans `link-map.txt`/temps regardless of whether an entry is declared) uses this
/// directly, while `resolve_project_specs` layers the entry requirement on top. Prints the diagnostic and
/// returns `Err(ExitCode::FAILURE)` on any resolution failure.
fn resolve_project_manifest(
    target_arg: Option<&str>,
    cmd: &str,
) -> Result<(PathBuf, PathBuf, Manifest), ExitCode> {
    let target: String = match target_arg {
        Some(d) => d.to_string(),
        None => match find_manifest_upward() {
            Some(p) => p.to_string_lossy().into_owned(),
            None => {
                eprintln!(
                    "{PROG}: no `{MANIFEST_NAME}` found in the current directory or any ancestor \
                     (name a project dir/manifest, or add a `{MANIFEST_NAME}`)"
                );
                return Err(ExitCode::FAILURE);
            }
        },
    };
    let path = std::path::Path::new(&target);
    let is_manifest_arg = path.file_name().and_then(|n| n.to_str()) == Some(MANIFEST_NAME);
    // Naming a `Project.cdz` that DOESN'T EXIST is a clear error, mirroring `cdz check`/`cdz test`:
    // without this, the arg resolves to its parent dir and `load_manifest` reports the confusing "no
    // `Project.cdz` in <parent>" (naming the dir, not the file the user typed). Fail with "no such file".
    if is_manifest_arg && !path.is_file() {
        eprintln!("{PROG}: {target}: no such file");
        return Err(ExitCode::FAILURE);
    }
    let dir = if is_manifest_arg {
        match path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
            _ => std::path::Path::new(".").to_path_buf(),
        }
    } else if path.is_dir() {
        path.to_path_buf()
    } else {
        eprintln!(
            "{PROG}: `{target}` is not a `{MANIFEST_NAME}` or a directory holding one \
             (`{cmd}` acts on a project; use `cdz compile <file>` for a single file)"
        );
        return Err(ExitCode::FAILURE);
    };
    match load_manifest(&dir) {
        Ok(Some((mpath, m))) => Ok((dir, mpath, m)),
        Ok(None) => {
            eprintln!("{PROG}: no `{MANIFEST_NAME}` in {}", dir.display());
            Err(ExitCode::FAILURE)
        }
        Err(e) => {
            eprintln!("{PROG}: {e}");
            Err(ExitCode::FAILURE)
        }
    }
}

/// Resolve a project's `Project.cdz` and its compile inputs — the shared front half of `cdz build` and
/// `cdz run <project>`. `target_arg` is the command's DIR argument (a manifest path, a directory holding
/// one, or `None` → an upward search from the cwd, like `cargo build` finding `Cargo.toml`). `cmd` names
/// the invoking command in the "not a project" hint. On any resolution failure this PRINTS the diagnostic
/// and returns `Err(ExitCode::FAILURE)`, so a caller just propagates it. On success returns the manifest
/// and the deduped `specs` (entry file first, then `modules`, glob-expanded + `exclude`-filtered), with
/// `entry_name` = the resolved entry file's stem (NOT the possibly-glob pattern — a `*` name would fail
/// package linking). The entry must resolve to EXACTLY ONE file (a component has one boundary).
fn resolve_project_specs(target_arg: Option<&str>, cmd: &str) -> Result<ProjectSpecs, ExitCode> {
    // Resolve the manifest DIR + load it (shared with `cdz clean`, which needs no entry).
    let (dir, mpath, m) = resolve_project_manifest(target_arg, cmd)?;
    // `entry` names the component boundary file — required to build (no entry, no component).
    let Some(entry_spec) = m.entry.clone() else {
        eprintln!(
            "{PROG}: {}: the manifest declares no `entry` (add `def entry = \"<file>\"` naming the \
             component's boundary file)",
            mpath.display()
        );
        return Err(ExitCode::FAILURE);
    };
    // Resolve the entry to its FILE, glob-expanded (path-sorted, exclude-filtered) relative to the dir —
    // the same resolution `cdz test` uses for `tests`. The entry names the component's single boundary,
    // so it must resolve to EXACTLY ONE file: zero → no such file; more than one (a multi-match glob like
    // `src/*.cdz`) → ambiguous, since a component has one entry. Reject both clearly rather than compile a
    // wrong/invalid entry.
    let entry_files = expand_manifest_globs(&dir, std::slice::from_ref(&entry_spec), &m.exclude);
    let entry_file = match entry_files.as_slice() {
        [] => {
            eprintln!(
                "{PROG}: {}: `entry` (`{entry_spec}`) matched no file",
                mpath.display()
            );
            return Err(ExitCode::FAILURE);
        }
        [one] => one.clone(),
        many => {
            eprintln!(
                "{PROG}: {}: `entry` (`{entry_spec}`) matched {} files — an entry names the ONE \
                 component boundary; name a single file (put libraries in `modules`). Matched: {}",
                mpath.display(),
                many.len(),
                many.iter()
                    .map(|s| program_name(s))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            return Err(ExitCode::FAILURE);
        }
    };
    // The package entry NAME is the RESOLVED entry file's stem (`app.cdz` → `app`), NOT the (possibly
    // glob) `entry_spec` — deriving it from the pattern would pass an invalid name like `*` to the
    // compiler and fail package linking. The entry file leads the spec list; the modules follow.
    let entry_name = program_name(&entry_file);
    let mut specs = vec![entry_file];
    specs.extend(expand_manifest_globs(&dir, &m.modules, &m.exclude));
    // Dedup (a module glob may also match the entry) while preserving order (entry stays first).
    let mut seen = std::collections::HashSet::new();
    specs.retain(|s| seen.insert(s.clone()));
    Ok(ProjectSpecs {
        mpath,
        m,
        entry_name,
        specs,
    })
}

/// Resolve `cdz build`'s optimization tier by precedence (v-core-opt design §7 — the canonical mapping):
/// `--opt-level <LEVEL>` (explicit) > the manifest's `def opt-level` > `--release` (`O2`) > the default
/// (`O1`). A malformed level string — from the flag or the manifest — is an `Err` naming the valid set,
/// so a typo is a clear failure rather than a silent default. `mpath` names the manifest in a manifest
/// parse error.
fn resolve_build_opt_level(
    args: &BuildArgs,
    manifest_opt_level: Option<&str>,
    mpath: &std::path::Path,
) -> Result<rcdzc::OptLevel, String> {
    resolve_opt_level_precedence(
        args.opt_level.as_deref(),
        args.release,
        manifest_opt_level,
        mpath,
    )
}

/// The shared optimization-tier PRECEDENCE, so `cdz build` and `cdz run` agree exactly (v-core-opt design
/// §7): an explicit `--opt-level <LEVEL>` wins; else the manifest's `def opt-level`; else `--release`
/// (`O2`); else the default (`O1`). A malformed level string — from the flag or the manifest — is an
/// `Err` naming the source, so a typo is a clear failure rather than a silent default. `mpath` names the
/// manifest in a manifest parse error.
fn resolve_opt_level_precedence(
    flag_opt_level: Option<&str>,
    release: bool,
    manifest_opt_level: Option<&str>,
    mpath: &std::path::Path,
) -> Result<rcdzc::OptLevel, String> {
    use std::str::FromStr;
    if let Some(s) = flag_opt_level {
        return rcdzc::OptLevel::from_str(s).map_err(|e| format!("--opt-level `{s}`: {e}"));
    }
    if let Some(s) = manifest_opt_level {
        return rcdzc::OptLevel::from_str(s)
            .map_err(|e| format!("{}: `opt-level` `{s}`: {e}", mpath.display()));
    }
    if release {
        return Ok(rcdzc::OptLevel::O2);
    }
    Ok(rcdzc::OptLevel::default())
}

// ── project metadata ─────────────────────────────────────────────────────────────────────────────

#[derive(clap::Args)]
struct MetadataArgs {
    /// The project to describe: a `Project.cdz` manifest, or a DIRECTORY holding one. OMITTED → search up
    /// from the current directory for the nearest `Project.cdz` (like `cdz build`/`cdz test`).
    dir: Option<String>,
}

/// `cdz metadata [DIR]` — print the resolved project manifest as JSON (the `cargo metadata` analogue): a
/// machine-readable description of what the project IS, for editors, build tools, and scripts. Resolves
/// the same `Project.cdz` that `cdz build`/`cdz test` do, then emits one JSON object: the manifest's raw
/// fields (`name`, `entry`, `opt_level`, the `modules`/`tests`/`exclude` PATTERNS) PLUS their RESOLVED,
/// glob-expanded, `exclude`-filtered file sets (`entry_file`, `module_files`, `test_files`) — so a
/// consumer sees both the declared intent and the concrete files, without re-implementing glob
/// resolution. Paths are the manifest-relative form the tool uses.
fn run_metadata(args: &MetadataArgs) -> ExitCode {
    // Resolve the manifest dir, mirroring `cdz build`: an explicit `Project.cdz`, a directory holding one,
    // or (no arg) an upward search from the cwd. A named-but-missing manifest is a clear "no such file".
    let target: String = match &args.dir {
        Some(d) => d.clone(),
        None => match find_manifest_upward() {
            Some(p) => p.to_string_lossy().into_owned(),
            None => {
                eprintln!(
                    "{PROG}: no `{MANIFEST_NAME}` found in the current directory or any ancestor \
                     (name a project dir/manifest, or add a `{MANIFEST_NAME}`)"
                );
                return ExitCode::FAILURE;
            }
        },
    };
    let path = std::path::Path::new(&target);
    let is_manifest_arg = path.file_name().and_then(|n| n.to_str()) == Some(MANIFEST_NAME);
    if is_manifest_arg && !path.is_file() {
        eprintln!("{PROG}: {target}: no such file");
        return ExitCode::FAILURE;
    }
    let dir = if is_manifest_arg {
        match path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
            _ => std::path::Path::new(".").to_path_buf(),
        }
    } else if path.is_dir() {
        path.to_path_buf()
    } else {
        eprintln!(
            "{PROG}: `{target}` is not a `{MANIFEST_NAME}` or a directory holding one \
             (`cdz metadata` describes a project)"
        );
        return ExitCode::FAILURE;
    };
    let (mpath, m) = match load_manifest(&dir) {
        Ok(Some(v)) => v,
        Ok(None) => {
            eprintln!("{PROG}: no `{MANIFEST_NAME}` in {}", dir.display());
            return ExitCode::FAILURE;
        }
        Err(e) => {
            eprintln!("{PROG}: {e}");
            return ExitCode::FAILURE;
        }
    };
    use cadenza_syntax::query::json;
    // A `[patterns] → resolved files` pair: the declared globs and the concrete files they expand to.
    let str_array = |items: &[String]| -> String {
        let mut arr = json::Array::new();
        for it in items {
            arr.string(it);
        }
        arr.finish()
    };
    let mut obj = json::Object::new();
    obj.string("manifest", &mpath.to_string_lossy());
    match &m.name {
        Some(n) => obj.string("name", n),
        None => obj.raw("name", "null"),
    }
    // The RESOLVED file fields (`entry_file`/`module_files`/`test_files`) report files that actually exist
    // on disk — a consumer can `stat` them. `expand_manifest_globs` passes a NON-glob literal through
    // VERBATIM (so `cdz build` can fail with a clear "reading X: No such file" when a declared `entry`/
    // `module` is missing), which is right for the compile path but would make metadata claim a
    // non-existent file is present. So filter the resolved sets to existing files here — matching how a
    // GLOB pattern already reports only matches (a missing literal now reads like a zero-match glob:
    // `entry_file: null`, or an absent module simply omitted). The PATTERN fields are echoed as declared.
    let existing = |files: Vec<String>| -> Vec<String> {
        files
            .into_iter()
            .filter(|f| std::path::Path::new(f).is_file())
            .collect()
    };
    // The entry: its declared pattern, the single EXISTING file it resolves to (a component has ONE
    // boundary; a glob matching ≠1 file — or a literal that doesn't exist — yields `null`, matching how
    // `cdz build` treats it as an error at build time), and that file's SURFACE (`ml` for `.cdz`/`.ml`,
    // `sexpr` for `.sexp`/`.sexpr`) — so a consumer knows which parser the project's boundary uses. `null`
    // when the entry is absent, missing on disk, or doesn't resolve to exactly one file.
    match &m.entry {
        Some(e) => {
            obj.string("entry", e);
            let resolved = existing(expand_manifest_globs(
                &dir,
                std::slice::from_ref(e),
                &m.exclude,
            ));
            match resolved.as_slice() {
                [one] => {
                    obj.string("entry_file", one);
                    obj.string("surface", if is_ml_source(one) { "ml" } else { "sexpr" });
                }
                _ => {
                    obj.raw("entry_file", "null");
                    obj.raw("surface", "null");
                }
            }
        }
        None => {
            obj.raw("entry", "null");
            obj.raw("entry_file", "null");
            obj.raw("surface", "null");
        }
    }
    match &m.opt_level {
        Some(o) => obj.string("opt_level", o),
        None => obj.raw("opt_level", "null"),
    }
    // The pattern lists PLUS their resolved, glob-expanded, exclude-filtered, existence-checked file sets.
    obj.raw("modules", &str_array(&m.modules));
    obj.raw(
        "module_files",
        &str_array(&existing(expand_manifest_globs(
            &dir, &m.modules, &m.exclude,
        ))),
    );
    obj.raw("tests", &str_array(&m.tests));
    obj.raw(
        "test_files",
        &str_array(&existing(expand_manifest_globs(&dir, &m.tests, &m.exclude))),
    );
    obj.raw("exclude", &str_array(&m.exclude));
    // The build ARTIFACTS currently present in the manifest directory — EXACTLY the set `cdz clean` would
    // remove, via the SAME `project_artifact_files` helper `cdz clean` uses, so the two never diverge. That
    // set is this project's own emitted outputs BY NAME (the entry's export-derived `<output>.{wasm,rs,
    // dwarf}`) + `link-map.txt` + `.cdz-run-*` temps — NOT a blanket extension sweep, so a user-authored
    // `helper.rs` / checked-in `asset.wasm` is never misreported as an artifact (the read-only twin of the
    // `cdz clean` data-loss fix). A tool can answer "is this project built?" / "what would `cdz clean`
    // remove?" without a build. Reported as bare file names (the sweep is the manifest's own dir), sorted.
    // Empty `[]` for an un-built project (or if the dir can't be read). The entry's single resolved file
    // names the outputs; a `None`/multi-glob entry contributes only the unambiguous `link-map`/temps.
    let entry_file_for_artifacts = m.entry.as_ref().and_then(|e| {
        match expand_manifest_globs(&dir, std::slice::from_ref(e), &m.exclude).as_slice() {
            [one] => Some(one.clone()),
            _ => None,
        }
    });
    let artifacts: Vec<String> = project_artifact_files(entry_file_for_artifacts.as_deref(), &dir)
        .unwrap_or_default()
        .iter()
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect();
    obj.raw("artifacts", &str_array(&artifacts));
    println!("{}", obj.finish());
    ExitCode::SUCCESS
}

// ── project clean ────────────────────────────────────────────────────────────────────────────────

#[derive(clap::Args)]
struct CleanArgs {
    /// The project to clean: a `Project.cdz` manifest, or a DIRECTORY holding one. OMITTED → search up
    /// from the current directory for the nearest `Project.cdz` (like `cdz build`/`cdz test`).
    dir: Option<String>,
    /// List what WOULD be removed without deleting anything (the preview / CI shape).
    #[arg(long)]
    dry_run: bool,
}

/// The exact output NAME(s) a build of `entry_file` emits (`<name>.wasm`/`.rs`/`.dwarf`) — the entry's
/// EXPORT names, which the compiler names the component after (NOT the entry file stem). Read from the
/// entry via the Exports query, so `cdz clean`'s removal set is precisely what a build writes. `entry_file`
/// is `None` (or unreadable/unparseable/exports-nothing) → EMPTY: the caller then considers only the
/// unambiguous `link-map.txt` + `.cdz-run-*` temps, never guessing a name (so a user file is never at
/// risk). Best-effort: this drives an ordinary sidecar query, no build.
fn clean_output_stems(entry_file: Option<&str>) -> Vec<String> {
    let Some(entry) = entry_file else {
        return Vec::new();
    };
    let Ok((_source, arenas)) = load_program(entry) else {
        return Vec::new();
    };
    let out = run_sidecar(
        &arenas,
        rcdzc::Request::Query(rcdzc::sidecar::Query::Exports),
    );
    let Some(bytes) = out.artifact(rcdzc::sidecar::KIND_EXPORTS) else {
        return Vec::new();
    };
    // Each line is `name<TAB>type<TAB>def-node`. The output component is named after the export; collect
    // EVERY export name (deduped) so a multi-export entry's output — whichever the compiler picks — is
    // covered. A blank/`-` name is skipped.
    let text = String::from_utf8_lossy(bytes);
    let mut stems: Vec<String> = Vec::new();
    for line in text.lines() {
        if let Some(name) = line.split('\t').next()
            && !name.is_empty()
            && name != "-"
            && !stems.contains(&name.to_string())
        {
            stems.push(name.to_string());
        }
    }
    stems
}

/// The build-artifact files THIS project has on disk in `dir` — the "what a build emitted / what `cdz
/// clean` removes" set. PRECISE, never a blanket extension sweep: a file qualifies iff its name is
/// `<output>.wasm`/`.rs`/`.dwarf` for one of the entry's EXPORT-derived output names, or `link-map.txt`,
/// or a `.cdz-run-*.wasm` temp — so a user's hand-authored `helper.rs` / checked-in `asset.wasm` is NEVER
/// included. Returns the sorted absolute paths. Propagates a `read_dir` error (the caller surfaces it)
/// rather than masking it as an empty set.
fn project_artifact_files(
    entry_file: Option<&str>,
    dir: &std::path::Path,
) -> std::io::Result<Vec<std::path::PathBuf>> {
    // The exact output name(s) a build emits — the entry's export names (see `clean_output_stems`), each as
    // `<name>.{wasm,rs,dwarf}` — plus the fixed `link-map.txt`.
    let mut exact: std::collections::HashSet<String> = std::collections::HashSet::new();
    for stem in clean_output_stems(entry_file) {
        for ext in ["wasm", "rs", "dwarf"] {
            exact.insert(format!("{stem}.{ext}"));
        }
    }
    exact.insert("link-map.txt".to_string());
    let mut out: Vec<std::path::PathBuf> = Vec::new();
    for e in std::fs::read_dir(dir)?.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        // A `.cdz-run-*.wasm` temp (our prefix) OR an exact-named output. Never an extension match alone.
        let is_temp = name.starts_with(".cdz-run-") && name.ends_with(".wasm");
        if (exact.contains(&name) || is_temp) && e.path().is_file() {
            out.push(e.path());
        }
    }
    out.sort();
    Ok(out)
}

/// `cdz clean [DIR]` — remove the build artifacts a `cdz build`/`cdz run` of this project produces (the
/// `cargo clean` analogue).
///
/// PRECISE — never a blanket extension sweep: it removes only the files THIS project's build emits, in the
/// manifest directory — `<output>.wasm`/`.rs`/`.dwarf` (where `<output>` is each name the compiler emits,
/// i.e. the entry's EXPORT names — a component is named after its export, not the entry file stem — read
/// via the Exports query), `link-map.txt`, and any `.cdz-run-*.wasm` temp (its `cdz-run-` prefix is ours).
/// A USER-authored file is NEVER deleted by extension: a hand-written `helper.rs`, a checked-in
/// `asset.wasm`, or an unrelated `.dwarf` in the project directory SURVIVES (an earlier version keyed on
/// the extension alone and could silently delete a user's `.rs`/`.wasm` — a data-loss bug this fixes).
/// Only the manifest's own directory is swept. `--dry-run` lists the targets without deleting; a missing
/// artifact is silently fine. A `read_dir` failure is SURFACED (not masked as "nothing to clean").
fn run_clean(args: &CleanArgs) -> ExitCode {
    // Resolve just the manifest DIR — `cdz clean` does NOT require an `entry` (it removes `link-map.txt` +
    // `.cdz-run-*` temps regardless, and derives the primary-output name from the entry's exports only IF
    // an entry is declared). So an entry-less / still-being-authored manifest can still be cleaned.
    let (dir, _mpath, m) = match resolve_project_manifest(args.dir.as_deref(), "cdz clean") {
        Ok(v) => v,
        Err(code) => return code,
    };
    // The entry file the outputs are named after, if the manifest declares one that resolves to a single
    // file — else `None` (clean then handles only the unambiguous `link-map`/temps). Mirrors the entry
    // resolution `cdz build`/`metadata` use, but never ERRORS on a missing/multi-glob entry here.
    let entry_file = m.entry.as_ref().and_then(|e| {
        match expand_manifest_globs(&dir, std::slice::from_ref(e), &m.exclude).as_slice() {
            [one] => Some(one.clone()),
            _ => None,
        }
    });
    // The precise removal set (this project's own emitted outputs by exact name + our temps), NOT a blanket
    // extension sweep — so a user's hand-authored `.rs`/`.wasm` is never deleted. A `read_dir` failure is a
    // real error — SURFACE it (don't mask it as "nothing to clean").
    let targets = match project_artifact_files(entry_file.as_deref(), &dir) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{PROG}: reading {}: {e}", dir.display());
            return ExitCode::FAILURE;
        }
    };
    // Remove each that exists; a missing artifact is not an error (nothing to clean). `--dry-run` lists
    // without deleting. Report what was (or would be) removed so the action is visible.
    let mut removed = 0usize;
    let mut had_error = false;
    for t in &targets {
        if !t.exists() {
            continue;
        }
        if args.dry_run {
            println!("would remove {}", t.display());
            removed += 1;
            continue;
        }
        match std::fs::remove_file(t) {
            Ok(()) => {
                println!("removed {}", t.display());
                removed += 1;
            }
            Err(e) => {
                eprintln!("{PROG}: removing {}: {e}", t.display());
                had_error = true;
            }
        }
    }
    if removed == 0 && !had_error {
        println!(
            "nothing to clean ({} has no build artifacts)",
            dir.display()
        );
    }
    if had_error {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

// ── project scaffold ─────────────────────────────────────────────────────────────────────────────

#[derive(clap::Args)]
struct NewArgs {
    /// The project directory to create (also the project name). Created fresh; refuses to clobber a
    /// non-empty existing directory.
    name: String,
    /// Scaffold the s-expression surface (`.sexp`) instead of the default ML (`.cdz`).
    #[arg(long)]
    sexpr: bool,
}

#[derive(clap::Args)]
struct InitArgs {
    /// The existing directory to initialize as a project (default: the current directory). A missing
    /// named directory is created; unlike `cdz new`, an existing non-empty directory is ADOPTED.
    dir: Option<String>,
    /// Scaffold the s-expression surface (`.sexp`) instead of the default ML (`.cdz`).
    #[arg(long)]
    sexpr: bool,
}

/// `cdz new <name>` — scaffold a new project (the `cargo new` analogue): a `<name>/` directory with a
/// `Project.cdz` manifest naming the entry, and a minimal BUILDABLE entry file, so `cd <name> && cdz
/// build` works immediately. Refuses to overwrite a non-empty directory (never clobbers existing work).
fn run_new(args: &NewArgs) -> ExitCode {
    let dir = std::path::Path::new(&args.name);
    // `cdz new` NAMES A FRESH SUBDIRECTORY to create — reject a name that instead points at an existing
    // in-place directory (empty ``, `.`, `..`, or any path whose final component is `.`/`..`). Those made
    // `new` silently scaffold into the CURRENT dir (empty name → a bogus "created project `app` in " with
    // an empty path; `.` → files written beside the cwd) — which is `cdz init`'s job. Point the user there.
    let final_is_curdir_or_parent = matches!(
        dir.components().next_back(),
        None | Some(std::path::Component::CurDir | std::path::Component::ParentDir)
    );
    if args.name.is_empty() || final_is_curdir_or_parent {
        eprintln!(
            "{PROG}: `cdz new` needs a NEW project directory name (got `{}`). To scaffold into an \
             EXISTING directory (e.g. the current one), use `cdz init`.",
            args.name
        );
        return ExitCode::FAILURE;
    }
    // Refuse to clobber existing work. Distinguish the cases so the error is accurate (a `read_dir`
    // failure was previously ALL reported as "not empty", including when the target is a FILE):
    //   - the target is a FILE → can't scaffold a project there;
    //   - the target is a non-empty DIRECTORY → refuse (never overwrite);
    //   - a missing target, or an empty directory → fine (the common case).
    if dir.is_file() {
        eprintln!(
            "{PROG}: `{}` already exists as a file — `cdz new` scaffolds a project DIRECTORY",
            dir.display()
        );
        return ExitCode::FAILURE;
    }
    if dir.is_dir() {
        // A read failure on an existing dir is a real error (permissions) — surface it, don't guess.
        let non_empty = match std::fs::read_dir(dir) {
            Ok(mut entries) => entries.next().is_some(),
            Err(e) => {
                eprintln!("{PROG}: reading {}: {e}", dir.display());
                return ExitCode::FAILURE;
            }
        };
        if non_empty {
            eprintln!(
                "{PROG}: `{}` already exists and is not empty — refusing to overwrite",
                dir.display()
            );
            return ExitCode::FAILURE;
        }
    }
    if let Err(e) = std::fs::create_dir_all(dir) {
        eprintln!("{PROG}: creating {}: {e}", dir.display());
        return ExitCode::FAILURE;
    }
    scaffold_project(dir, args.sexpr, true)
}

/// `cdz init [dir]` — scaffold a project INTO an EXISTING directory (the `cargo init` analogue): write a
/// `Project.cdz` + a minimal buildable entry into `dir` (default: the current directory), WITHOUT creating
/// a new subdirectory. Complements `cdz new <name>` (which makes a fresh `<name>/`). Refuses only when the
/// directory ALREADY holds a `Project.cdz` (never overwrite an existing manifest); other files are fine —
/// `cdz init` adopts an existing directory. `--sexpr` scaffolds the s-expression surface.
fn run_init(args: &InitArgs) -> ExitCode {
    let dir = std::path::Path::new(args.dir.as_deref().unwrap_or("."));
    // A FILE where the directory should be can't hold a project.
    if dir.is_file() {
        eprintln!(
            "{PROG}: `{}` is a file, not a directory — `cdz init` scaffolds into a directory",
            dir.display()
        );
        return ExitCode::FAILURE;
    }
    // Unlike `cdz new`, `init` ADOPTS an existing (even non-empty) directory — the only refusal is an
    // existing `Project.cdz`, which we must never overwrite (it would clobber the user's manifest).
    if dir.join(MANIFEST_NAME).is_file() {
        eprintln!(
            "{PROG}: `{}` already exists — this directory is already a project",
            dir.join(MANIFEST_NAME).display()
        );
        return ExitCode::FAILURE;
    }
    // Create the directory if it's a named-but-missing target (e.g. `cdz init sub/dir`); the common
    // `cdz init` (current dir) already exists, so this is a no-op there.
    if let Err(e) = std::fs::create_dir_all(dir) {
        eprintln!("{PROG}: creating {}: {e}", dir.display());
        return ExitCode::FAILURE;
    }
    scaffold_project(dir, args.sexpr, false)
}

/// Write a `Project.cdz` manifest + a minimal buildable entry into `dir` (assumed to exist). Shared by
/// `cdz new` (fresh dir) and `cdz init` (existing dir). `sexpr` picks the surface; `created` selects the
/// success wording ("created project" for `new`, "initialized project" for `init`). The project name is
/// the directory's final component. Never overwrites: callers guard the pre-conditions (an empty/new dir
/// for `new`, no existing manifest for `init`).
fn scaffold_project(dir: &std::path::Path, sexpr: bool, created: bool) -> ExitCode {
    // The project name = the directory's final component (so `cdz new path/to/app` names it `app`). A
    // relative `.` (the `cdz init` default) has no final component — fall back to the canonicalized dir's
    // name, else a generic default, so the manifest always gets a real name.
    let proj_name = dir
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string())
        .or_else(|| {
            std::fs::canonicalize(dir)
                .ok()
                .and_then(|c| c.file_name().and_then(|n| n.to_str()).map(String::from))
        })
        .unwrap_or_else(|| "app".to_string());
    let (ext, entry_src) = if sexpr {
        ("sexp", "(do (def (main) 0) (export main))\n".to_string())
    } else {
        (
            "cdz",
            "def main() -> Int64 = 0\nexport { main }\n".to_string(),
        )
    };
    let entry_file = format!("main.{ext}");
    // ESCAPE the project name for the `"…"` string literal — the dir name is user-controlled, so a name
    // with a `"`, `\`, or control char would otherwise inject into (and malform) the generated
    // Project.cdz. `entry_file` is always `main.cdz`/`main.sexp` (no escaping needed), but escape it too
    // for uniformity. Uses the canonical `cadenza_syntax` escaper so the manifest re-parses exactly.
    let manifest_src = format!(
        "def name = \"{}\"\ndef entry = \"{}\"\n",
        cadenza_syntax::literal::escape_string(&proj_name),
        cadenza_syntax::literal::escape_string(&entry_file),
    );
    // Write the manifest + entry. A write failure (permissions, a race) is a clean tool error.
    for (rel, contents) in [
        (MANIFEST_NAME, manifest_src),
        (entry_file.as_str(), entry_src),
    ] {
        if let Err(e) = std::fs::write(dir.join(rel), contents) {
            eprintln!("{PROG}: writing {}: {e}", dir.join(rel).display());
            return ExitCode::FAILURE;
        }
    }
    // Scaffold a `.gitignore` covering the build OUTPUTS a `cdz build`/`cdz run` writes beside the manifest
    // — the exact set `cdz clean` removes — so a fresh project doesn't git-track its artifacts (the
    // `cargo new`→`/target` convention). Only WRITE it if absent: `cdz init` adopts an existing directory,
    // which may already have a `.gitignore` the user maintains — never clobber it (a missing manifest was
    // the sole `init` refusal; the same non-destructive spirit applies here). A write failure is non-fatal:
    // the project is already scaffolded + buildable, so warn and continue rather than fail the command.
    let gitignore = dir.join(".gitignore");
    if !gitignore.exists() {
        // Ignore the EXACT outputs a build of the scaffolded entry emits — the entry exports `main`, so a
        // build writes `main.{wasm,rs,dwarf}` — plus `link-map.txt` and the `.cdz-run-*.wasm` run temp.
        // NOT the broad `*.wasm`/`*.rs`/`*.dwarf` globs a first version used: `*.rs` would git-ignore a
        // user's hand-written Rust helper, and `*.wasm` a checked-in asset — the same over-broad
        // extension assumption that made `cdz clean` a data-loss risk (both narrowed to exact names). The
        // entry stem is fixed at scaffold time (`main`), so these names are stable for the fresh project;
        // if the author renames the export/adds targets, they edit `.gitignore` like any project file.
        let stem = entry_file
            .rsplit_once('.')
            .map(|(s, _)| s)
            .unwrap_or("main");
        let body = format!(
            "# Cadenza build artifacts (see `cdz clean`)\n\
             {stem}.wasm\n\
             {stem}.rs\n\
             {stem}.dwarf\n\
             link-map.txt\n\
             .cdz-run-*.wasm\n"
        );
        if let Err(e) = std::fs::write(&gitignore, &body) {
            eprintln!(
                "{PROG}: warning: could not write {}: {e}",
                gitignore.display()
            );
        }
    }
    let verb = if created { "created" } else { "initialized" };
    println!(
        "{verb} project `{proj_name}` in {} ({MANIFEST_NAME} + {entry_file})\n  next: cd {} && cdz build",
        dir.display(),
        dir.display()
    );
    ExitCode::SUCCESS
}

// ── shell completions ────────────────────────────────────────────────────────────────────────────

#[derive(clap::Args)]
struct CompletionsArgs {
    /// The shell to generate a completion script for.
    #[arg(value_enum)]
    shell: clap_complete::Shell,
}

/// `cdz completions <shell>` — print a shell completion script for `cdz` to stdout, generated from the
/// clap command tree (so it can never drift from the real subcommands/flags). The user redirects it to
/// their shell's completion location. Codegen only; always succeeds.
fn run_completions(args: &CompletionsArgs) -> ExitCode {
    use clap::CommandFactory;
    let mut cmd = Cli::command();
    let name = cmd.get_name().to_string();
    clap_complete::generate(args.shell, &mut cmd, name, &mut std::io::stdout());
    ExitCode::SUCCESS
}

// ── cdz doctor ─────────────────────────────────────────────────────────────────────────────────

#[derive(clap::Args)]
struct DoctorArgs {
    /// The value-heap runtime store to check (`<store>/<hash>.wasm`). Defaults to the store beside the
    /// `cdz` binary (`<target>/cadenza-store`) — the same default `cdz run`/`cdz test` resolve.
    #[arg(long)]
    store: Option<PathBuf>,
    /// Emit the health report as a machine-readable JSON object instead of human lines — for CI/setup
    /// scripts (the `cdz metadata`/`cdz check --json` shape). The exit code is unchanged (non-zero iff a
    /// run/test-breaking component is missing).
    #[arg(long)]
    json: bool,
}

/// `cdz doctor` — a preflight health check of the `cdz` TOOLCHAIN environment (the `cargo`-doctor
/// analogue), so a broken setup surfaces before a `cdz run`/`cdz test` fails mid-operation. It reports
/// three things and exits non-zero if a component that would break run/test is missing. First, the `cdz`
/// version + executable path (what a bug report should cite). Second, the sibling `cdz-run` binary — this
/// bin holds no wasm engine, so `cdz run`/`cdz test` shell out to it; if it's absent, those fail. Third,
/// the value-heap runtime store: present, and holding the runtime `cdz` compiles against
/// (`REQUIRED_RUNTIME_HASH`) — without it, running a program that builds heap values can't resolve its
/// runtime by content address (a scalar/const program still runs without the store, and the note says so).
/// A missing `cdz-run` or runtime is an ERROR (rc≠0) so a setup/CI script can gate on `cdz doctor`.
fn run_doctor(args: &DoctorArgs) -> ExitCode {
    // Compute the three checks into structured `(status, detail)` values FIRST, so the human and `--json`
    // outputs are the same facts rendered two ways (they can't drift). `status` is "ok" for a healthy
    // component or a distinct problem label ("missing"/"stale") a consumer can branch on.
    let exe = std::env::current_exe()
        .ok()
        .map(|p| p.display().to_string());

    // cdz-run runner.
    let cdz_run = locate_cdz_run().map(|p| p.display().to_string());

    // Runtime store.
    let store = args.store.clone().unwrap_or_else(default_store);
    let required = rcdzc::backend::wasm::runtime_abi::REQUIRED_RUNTIME_HASH;
    let store_status = if !store.is_dir() {
        "missing"
    } else if store.join(format!("{required}.wasm")).is_file() {
        "ok"
    } else {
        "stale"
    };
    // A missing `cdz-run` OR a not-ok store is a run/test-breaking problem.
    let ok = cdz_run.is_some() && store_status == "ok";

    if args.json {
        use cadenza_syntax::query::json;
        let mut obj = json::Object::new();
        obj.string("version", env!("CARGO_PKG_VERSION"));
        match &exe {
            Some(p) => obj.string("path", p),
            None => obj.raw("path", "null"),
        }
        let mut cr = json::Object::new();
        cr.raw("ok", if cdz_run.is_some() { "true" } else { "false" });
        match &cdz_run {
            Some(p) => cr.string("path", p),
            None => cr.raw("path", "null"),
        }
        obj.raw("cdz_run", &cr.finish());
        let mut st = json::Object::new();
        st.string("status", store_status); // "ok" | "missing" | "stale"
        st.string("path", &store.to_string_lossy());
        st.string("required_runtime", required);
        obj.raw("runtime_store", &st.finish());
        obj.raw("ok", if ok { "true" } else { "false" });
        println!("{}", obj.finish());
        return if ok {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        };
    }

    // Human report: the same facts as lines.
    println!("cdz {}", env!("CARGO_PKG_VERSION"));
    println!("  path: {}", exe.as_deref().unwrap_or("<unknown>"));
    match &cdz_run {
        Some(p) => println!("  cdz-run: ok ({p})"),
        None => println!(
            "  cdz-run: MISSING — build it (`cargo build --bin cdz-run`) beside `cdz`; \
             `cdz run`/`cdz test` need it"
        ),
    }
    let short = &required[..12.min(required.len())];
    match store_status {
        "ok" => println!(
            "  runtime store: ok ({}) — has the required runtime {short}",
            store.display()
        ),
        "missing" => println!(
            "  runtime store: MISSING ({}) — build it (`cargo xtask build`); required to run a program \
             that builds heap values (a scalar/const program still runs without it)",
            store.display()
        ),
        _ => println!(
            "  runtime store: STALE ({}) — present but missing the required runtime {short}.wasm; rebuild \
             (`cargo xtask build`)",
            store.display()
        ),
    }

    if ok {
        println!("doctor: ok");
        ExitCode::SUCCESS
    } else {
        eprintln!("{PROG}: doctor found problem(s) — see above");
        ExitCode::FAILURE
    }
}

// ── cdz test ─────────────────────────────────────────────────────────────────────────────────────

/// `cdz test FILE` — compile a SEPARATE test component from the file's `@test` NULLARY definitions and
/// run each, reporting pass/fail. The flow, all in this one process for the compile half:
///  1. Parse the source (`load_program`), encode the `ast` artifact.
///  2. Enumerate the `@test` definitions' SOURCE names from a `Db` (`db.test_defs`) — the tests to run,
///     in declaration order; filtered by `--filter` if given.
///  3. Compile with an `EmitTests` sidecar request → the wasm component whose exports ARE the tests
///     (`layout::compute_tests`). A test that TRAPS on failure crosses as a nullary no-result entry.
///  4. Shell out to the sibling `cdz-run` binary once per test (this bin holds no wasm engine, by design):
///     `cdz-run <component> --call <kebab-name>`. Exit 0 = the test returned (PASS); exit ≠ 0 = it trapped
///     (FAIL). A failure's message rides `cdz-run`'s `host-arg` stderr line (the assertion text the test
///     emitted via a host effect before trapping).
///
/// Exits non-zero if ANY test fails (or if a file's compile declines / no `@test` is present) — the CI
/// shape. FILE may be a DIRECTORY: every source file under it (recursively, `.cdz`/`.ml`/`.sexp`) is run
/// and the pass/fail totals are aggregated, so `cdz test <dir>` runs a whole package's suite in one call.
fn run_test(args: &TestArgs) -> ExitCode {
    // Resolve WHICH files to run. Cases:
    //  - NO arg → search UP from the current directory for the nearest `Project.cdz` (like `cargo test`
    //    finding `Cargo.toml`) and run its suite;
    //  - a `Project.cdz` (or a directory holding one): run the manifest's `tests` list — the project
    //    TELLS us its suite (the Cadenza-authored manifest, no per-run flags);
    //  - a directory with NO manifest: run every source file's `@test`s (path-sorted walk);
    //  - a single file: the one-file case.
    let target: String = match &args.file {
        Some(f) => f.clone(),
        None => match find_manifest_upward() {
            Some(p) => p.to_string_lossy().into_owned(),
            None => {
                eprintln!(
                    "{PROG}: no `{MANIFEST_NAME}` found in the current directory or any ancestor \
                     (name a file/dir to test, or add a `{MANIFEST_NAME}`)"
                );
                return ExitCode::FAILURE;
            }
        },
    };
    let path = std::path::Path::new(&target);
    let is_manifest_arg = path.file_name().and_then(|n| n.to_str()) == Some(MANIFEST_NAME);
    let manifest_dir: Option<std::path::PathBuf> = if is_manifest_arg {
        path.parent().map(|p| {
            if p.as_os_str().is_empty() {
                std::path::Path::new(".").to_path_buf()
            } else {
                p.to_path_buf()
            }
        })
    } else if path.is_dir() {
        Some(path.to_path_buf())
    } else {
        None
    };
    let files: Vec<String> = if let Some(dir) = &manifest_dir {
        match load_manifest(dir) {
            Err(e) => {
                eprintln!("{PROG}: {e}");
                return ExitCode::FAILURE;
            }
            // A manifest is present: run its declared `tests`, resolved relative to the manifest's dir.
            // A `tests` entry may be a literal file OR a GLOB (`*.cdz`, `tests/*.cdz`, `**/x.cdz`),
            // expanded against the dir (path-sorted, deduped) — so a project can say `tests = ["*.cdz"]`.
            Ok(Some((mpath, m))) => {
                if m.tests.is_empty() {
                    eprintln!(
                        "{PROG}: {}: the manifest declares no `tests` (add `def tests = [\"…\"]`)",
                        mpath.display()
                    );
                    return ExitCode::SUCCESS;
                }
                let expanded = expand_manifest_globs(dir, &m.tests, &m.exclude);
                if expanded.is_empty() {
                    eprintln!(
                        "{PROG}: {}: the manifest's `tests` matched no files",
                        mpath.display()
                    );
                    return ExitCode::SUCCESS;
                }
                expanded
            }
            // No manifest in the directory: fall back to walking every source file (path-sorted).
            Ok(None) if is_manifest_arg => {
                eprintln!("{PROG}: {target}: no such file");
                return ExitCode::FAILURE;
            }
            Ok(None) => {
                let mut out = Vec::new();
                if let Err(e) = collect_source_dir(dir, &mut out) {
                    eprintln!("{PROG}: {e}");
                    return ExitCode::FAILURE;
                }
                if out.is_empty() {
                    eprintln!(
                        "{PROG}: {target}: no source files (.cdz/.ml/.sexp) found in directory"
                    );
                    return ExitCode::SUCCESS; // an empty tree is vacuously green
                }
                out
            }
        }
    } else {
        vec![target.clone()]
    };

    // Locate the sibling `cdz-run` binary + the runtime store ONCE (shared across files).
    let Some(cdz_run) = locate_cdz_run() else {
        eprintln!(
            "{PROG}: cannot find the `cdz-run` binary beside this executable — build it \
             (`cargo build --bin cdz-run`) so `cdz test` can run the compiled tests"
        );
        return ExitCode::FAILURE;
    };
    let store = args.store.clone().unwrap_or_else(default_store);
    let multi = files.len() > 1;

    let mut total_pass = 0usize;
    let mut total_fail = 0usize;
    let mut any_error = false; // a file whose compile DECLINED (distinct from a test that failed)
    for (i, file) in files.iter().enumerate() {
        // In multi-file mode, head each file's block with its path so the output stays legible.
        if multi {
            if i > 0 {
                println!();
            }
            println!("── {file} ──");
        }
        match run_test_file(
            file,
            args.filter.as_deref(),
            args.tag.as_deref(),
            &cdz_run,
            &store,
            args.trials,
            args.seed,
        ) {
            Ok((p, f)) => {
                total_pass += p;
                total_fail += f;
            }
            Err(()) => any_error = true, // the compile declined; errors already printed to stderr
        }
    }

    // A combined total across a package (a single file already printed its own "N passed, M failed").
    if multi {
        println!(
            "\n═══ TOTAL: {total_pass} passed, {total_fail} failed (across {} files) ═══",
            files.len()
        );
    }
    if total_fail == 0 && !any_error {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Run one file's `@test` definitions, printing `PASS`/`FAIL` per test and a per-file `N passed, M
/// failed` summary. Returns `(passed, failed)` on success, or `Err(())` when the test compile DECLINED
/// (its errors are printed to stderr) — distinct from a clean run where some tests failed. A file with no
/// matching `@test` prints nothing and returns `(0, 0)` (vacuously green), so a directory of mixed
/// modules — some without tests — aggregates cleanly.
fn run_test_file(
    file: &str,
    filter: Option<&str>,
    tag: Option<&str>,
    cdz_run: &std::path::Path,
    store: &std::path::Path,
    trials: u64,
    seed: u64,
) -> Result<(usize, usize), ()> {
    // Follow the entry file's IMPORT CLOSURE so a test in a module that imports a sibling (e.g. a pass
    // that reuses another module's type) resolves + runs — `cdz test FILE` sees the SAME linked program
    // `cdz check FILE` does. A file that imports nothing loads as a lone file, byte-identical to a
    // standalone single-file test compile; only a file carrying an `(import …)` pulls its siblings in.
    let closure = match load_import_closure_with(file, &|_| None) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("{PROG}: {e}");
            return Err(());
        }
    };
    let is_package = !declared_import_paths(&closure[0].arenas).is_empty();

    // Encode each closure file's `ast` ONCE — the per-file artifacts feed BOTH the `Db` that enumerates
    // the ENTRY file's `@test` names and the package emit compile below. The front-end (`cadenza_syntax`)
    // and compiler (`rcdzc`) have DISTINCT arena types; the canonical binary form is the bridge.
    let ast_arts: Vec<rcdzc::Artifact> = closure
        .iter()
        .map(|f| {
            rcdzc::Artifact::new(
                rcdzc::Artifact::KIND_AST,
                f.name.clone(),
                cadenza_syntax::codec::encode(&f.arenas),
            )
        })
        .collect();

    // Build the compiler `Db` used to enumerate test names + solve property-test param types. A single
    // file decodes directly (`Db::load`, byte-identical to before); a PACKAGE links every closure file
    // into one arena and loads it WITH its linkage (`Db::load_linked`), so a cross-file name resolves. On
    // a package, `linkage` also maps a test def back to its file so we run ONLY the ENTRY file's own
    // tests — an imported library's tests run when THAT file is itself the entry (a directory run visits
    // each), never double-counted through an importer.
    let (mut db, entry_filter) = if is_package {
        let mut rcdzc_files = Vec::with_capacity(ast_arts.len());
        for art in &ast_arts {
            let Some(a) = rcdzc::codec::decode(&art.bytes) else {
                eprintln!("{PROG}: {file}: could not decode `{}`'s AST", art.name);
                return Err(());
            };
            rcdzc_files.push((art.name.clone(), a));
        }
        let program = match rcdzc::link::link(&rcdzc_files, &closure[0].name) {
            Ok(p) => p,
            Err(r) => {
                eprintln!("{PROG}: {file}: {}", r.message);
                return Err(());
            }
        };
        let linkage = program.linkage();
        let entry_ix = program.entry;
        let db = rcdzc::db::Db::load_linked(program.arenas, Some(linkage.clone()));
        (db, Some((linkage, entry_ix)))
    } else {
        let Some(rcdzc_arenas) = rcdzc::codec::decode(&ast_arts[0].bytes) else {
            eprintln!("{PROG}: {file}: could not decode the program's AST");
            return Err(());
        };
        (rcdzc::db::Db::load(rcdzc_arenas), None)
    };
    // Each test's name PLUS the generators for its parameters (empty = a plain nullary test, run once;
    // non-empty = a PROPERTY test, run `trials` times with generated inputs). A param whose type is not a
    // generatable scalar makes `param_generators` return `None` — reported per test, not aborting the run.
    let mut tests: Vec<(String, Option<Vec<GenKind>>, bool)> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for i in db.test_defs() {
        // In a PACKAGE, `test_defs()` sees every linked file's `@test`s — keep only the ENTRY file's own,
        // so an imported library's tests aren't run through its importer (they run when that library is
        // itself the entry). A def's file is the file whose id-range holds its signature node.
        if let Some((linkage, entry_ix)) = &entry_filter {
            match linkage.file_of(db.defs[i].sig_occ) {
                Some(fi) if fi == *entry_ix => {}
                _ => continue,
            }
        }
        let name = db.defs[i].name.clone();
        if filter.is_some_and(|needle| !name.contains(needle)) {
            continue;
        }
        // `--tag <t>`: keep only a test whose def carries the `@tag("t")` string tag. AND-composed with
        // `--filter` (both are additive selectors; an absent one imposes no constraint). A test with no
        // `@tag` is skipped under `--tag`, and every test runs when `--tag` is absent.
        if tag.is_some_and(|want| !db.tags_of(i).iter().any(|t| t == want)) {
            continue;
        }
        if !seen.insert(name.clone()) {
            continue;
        }
        // `@exhaustive`: the test is driven over its ENTIRE finite input domain, not by random sampling.
        // Captured per test (before `db` is re-borrowed by the next `param_generators`).
        let exhaustive = db.is_exhaustive(i);
        let gens = param_generators(&mut db, i);
        tests.push((name, gens, exhaustive));
    }
    if tests.is_empty() {
        // No matching `@test` here. A file with no tests (e.g. a pure library module in a package dir, or
        // a `--filter` that selects nothing) is vacuously green — return (0, 0) and print nothing, so a
        // directory run aggregates without a spurious error line per test-free file.
        return Ok((0, 0));
    }

    // Compile the test component: every closure file's `ast` + an `EmitTests` sidecar request. `EmitTests`
    // lays the boundary out from the `@test` defs (`layout::compute_tests`), not the `(export …)` clauses.
    // For a package, the `entry` marker names which file's imports drive linking; a single file needs none
    // (identical to before). `compute_tests` still exports ALL linked `@test`s — the entry-file filter above
    // decides which we RUN, but a library test kept in the component is harmless (unreached, uncalled).
    let mut inputs = ast_arts;
    inputs.push(rcdzc::Artifact::new(
        rcdzc::sidecar::KIND_SIDECAR,
        "drive",
        rcdzc::sidecar::encode(&[rcdzc::Request::EmitTests]),
    ));
    if is_package {
        inputs.push(compiler_cli::entry_artifact(&closure[0].name));
    }
    let out = rcdzc::run_with_compiler_stack(|| rcdzc::compile(&inputs, &[]));
    let Some(component) = out.artifact("component") else {
        // The test compile declined — report its errors (a parameterized `@test`, an ill-typed test body,
        // etc.). `report_errors` prints each coded/uncoded error to stderr.
        report_errors(&out);
        return Err(());
    };
    let component = component.to_vec();

    // Write the component to a temp file the runner reads. (cdz-run also reads `-` from stdin, but a
    // per-test re-invocation reuses one file rather than re-piping the bytes each call.) Keyed by pid +
    // a path-derived tag (non-alphanumerics → `_`, so a nested `dir/mod.cdz` yields a FLAT temp name, not
    // a path with missing parent dirs) so files in a directory run never clash on one path.
    let tag: String = file
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let tmp = std::env::temp_dir().join(format!("cdz-test-{}-{tag}.wasm", std::process::id()));
    if let Err(e) = std::fs::write(&tmp, &component) {
        eprintln!(
            "{PROG}: writing the test component to {}: {e}",
            tmp.display()
        );
        return Err(());
    }

    // Run each test through `cdz-run`, in declaration order. A NULLARY test runs ONCE — PASS = exit 0
    // (returned), FAIL = nonzero (trapped). A PROPERTY test (parameters) runs `trials` times with generated
    // inputs; it PASSES only if every trial returns, and FAILS on the first trapping trial — reported with
    // the failing inputs (shrunk toward a minimal counterexample) + the seed to replay.
    let mut passed = 0usize;
    let mut failed = 0usize;
    for (name, gens, exhaustive) in &tests {
        let kebab = cadenza_syntax::extern_name::kebab_extern_name(name);
        let run_one = |arg_vals: &[String]| -> TrialOutcome {
            run_one_trial(cdz_run, &tmp, &kebab, store, arg_vals)
        };
        match gens {
            // A parameter whose type is not a generatable scalar — cannot property-test it. Report + fail.
            None => {
                failed += 1;
                println!(
                    "FAIL {name}: cannot generate inputs — a parameter's type is not a scalar this \
                     runner generates (Int/Bool/Float/Char); annotate it with a scalar type"
                );
            }
            // An `@exhaustive` test whose (original) parameter was COMPOUND: the compiler synthesized a
            // gen-driven wrapper that builds the value from the runner's random int POOL, which offers no
            // way to ENUMERATE a domain (it samples). So exhaustive checking is not (yet) supported for a
            // compound-parameter test — regardless of whether that domain is unbounded (a `List`) or
            // finite (a small user-sum enum). Decline cleanly, rather than sampling under an `@exhaustive`
            // label (which would falsely imply a proof) or aborting the file at the compound export
            // boundary. (Exhaustive enumeration works for a BOUNDED SCALAR signature — the boundary-arg
            // route above — where the domain is enumerated directly, not drawn from the pool.)
            Some(gens) if gens.is_empty() && *exhaustive => {
                failed += 1;
                println!(
                    "FAIL {name}: @exhaustive is not supported for a compound parameter (a \
                     collection/tuple/record/sum) — its generator samples the random pool and cannot \
                     enumerate a domain; use a sampled `@test` for it, or make the signature bounded \
                     SCALAR parameters (Bool / a narrow integer) for exhaustive checking"
                );
            }
            // Nullary SOURCE signature — but this splits at runtime into two cases by whether the body
            // performs `Test.gen`: a GENERATOR-DRIVEN property test (a nullary wrapper that pulls random
            // ints from the runner to build its own inputs — the compound/int-stream route) vs a plain
            // unit test (pulls no generated int). Decide it by RUNNING once under a seeded int pool and
            // counting the `Test.gen` calls the guest made.
            Some(gens) if gens.is_empty() => {
                match run_gen_driven(cdz_run, &tmp, &kebab, store, trials, seed) {
                    // The test consumed NO generated int → a plain unit test; report its single run.
                    GenDrivenOutcome::Plain(TrialOutcome::Pass) => {
                        passed += 1;
                        println!("PASS {name}");
                    }
                    GenDrivenOutcome::Plain(TrialOutcome::Fail(msg)) => {
                        failed += 1;
                        match msg {
                            Some(m) => println!("FAIL {name}: {m}"),
                            None => println!("FAIL {name}"),
                        }
                    }
                    // A generator-driven property test that passed every trial.
                    GenDrivenOutcome::Property(None) => {
                        passed += 1;
                        println!("PASS {name} ({trials} trials)");
                    }
                    // A generator-driven property test with a counterexample (the shrunk failing int pool).
                    GenDrivenOutcome::Property(Some(fail)) => {
                        failed += 1;
                        let msg = fail.message.map(|m| format!(": {m}")).unwrap_or_default();
                        let pool = fail
                            .inputs
                            .iter()
                            .map(|s| s.as_str())
                            .collect::<Vec<_>>()
                            .join(", ");
                        println!(
                            "FAIL {name}{msg}\n  counterexample: generated ints [{pool}]  (seed {seed}; \
                             replay with `--seed {seed}`)"
                        );
                    }
                }
            }
            // An `@exhaustive` PROPERTY test: drive the ENTIRE finite input domain (every combination of
            // the scalar parameters) rather than random sampling — a pass is a PROOF over the domain. Only
            // a BOUNDED domain can be enumerated; an unbounded parameter (a 32/64-bit int or a float, whose
            // domain is astronomically/infinitely large) makes `exhaustive_domain` return `None` → report
            // (the property must narrow its types, e.g. to `Bool`/`UInt8`, to be exhaustively provable).
            Some(gens) if *exhaustive => match exhaustive_domain(gens) {
                // An unbounded domain (a wide integer / float) is DECLINED with a diagnostic, never
                // silently sampled — so an exhaustive result is never reported for a domain not fully
                // covered.
                //= spec/capabilities/property-based-testing.md#an-unbounded-domain-declines-exhaustive-checking
                //# A property requested to be checked exhaustively over an unbounded input domain MUST be declined with a diagnostic rather than silently sampled, so that an exhaustive result is never reported for a domain that was not fully covered.
                None => {
                    failed += 1;
                    println!(
                        "FAIL {name}: @exhaustive needs a BOUNDED input domain — a parameter's type \
                         (a wide integer or a float) has too large a domain to enumerate; narrow it \
                         (e.g. Bool or UInt8)"
                    );
                }
                Some(domain) => {
                    let total = domain.len();
                    match domain
                        .into_iter()
                        .find(|inputs| matches!(run_one(inputs), TrialOutcome::Fail(_)))
                    {
                        // No failing case in the WHOLE enumerated domain → a proof over the domain, not a
                        // sample.
                        //= spec/capabilities/property-based-testing.md#exhaustive-coverage-is-a-proof-over-a-bounded-domain
                        //# When a property is checked by enumerating its entire bounded finite domain, a run that finds no failing input MUST be treated as a proof of the property over the domain rather than as a sample.
                        None => {
                            passed += 1;
                            println!("PASS {name} (exhaustive, {total} cases)");
                        }
                        Some(inputs) => {
                            failed += 1;
                            // Re-run the failing case to recover its reported message.
                            let msg = match run_one(&inputs) {
                                TrialOutcome::Fail(Some(m)) => format!(": {m}"),
                                _ => String::new(),
                            };
                            let args_str = inputs.join(", ");
                            println!(
                                "FAIL {name}{msg}\n  counterexample: {name}({args_str})  (exhaustive \
                                 — the domain contains a failing case)"
                            );
                        }
                    }
                }
            },
            // A sampled PROPERTY test: run `trials` trials with generated inputs.
            Some(gens) => match run_property(gens, trials, seed, &run_one) {
                None => {
                    passed += 1;
                    println!("PASS {name} ({trials} trials)");
                }
                Some(PropertyFailure { inputs, message }) => {
                    failed += 1;
                    let args_str = inputs.join(", ");
                    let msg = message.map(|m| format!(": {m}")).unwrap_or_default();
                    // A reported property failure records BOTH the input that produced it (the shrunk
                    // counterexample args) AND the seed to replay — so the failing run is reproducible.
                    //= spec/capabilities/property-based-testing.md#generation-is-seeded-and-reproducible
                    //# A reported property failure MUST record the seed and the input that produced it.
                    println!(
                        "FAIL {name}{msg}\n  counterexample: {name}({args_str})  (seed {seed}; replay \
                         with `--seed {seed}`)"
                    );
                }
            },
        }
    }
    let _ = std::fs::remove_file(&tmp);

    println!("\n{passed} passed, {failed} failed");
    Ok((passed, failed))
}

/// The scalar KIND of a property-test parameter — what the runner generates a value of and renders to a
/// `cdz-run --arg` string. Restricted to the scalars `cdz-run`'s `coerce_one` parses from `--arg` text:
/// the boundary-crossable scalars this first property-testing increment supports (a compound param —
/// tuple/sum/list — is a later increment via a guest-side `Gen` effect).
#[derive(Clone, Copy)]
enum GenKind {
    /// A fixed-width integer: `(signed, width)`. Generated as a random value in the width's range.
    Int {
        signed: bool,
        width: u32,
    },
    Bool,
    /// A float (32 or 64). Generated as a random finite decimal — the text parses as either width, so the
    /// width need not be tracked here (`cdz-run`'s `coerce_one` parses `f32`/`f64` from the same decimal).
    Float,
    Char,
}

/// The generators for definition `def`'s parameters, or `None` if ANY parameter's solved type is not a
/// generatable scalar (so the test cannot be property-run). An EMPTY vec means a nullary def (run once).
/// Each param's type is solved with `infer::type_of` on its binder (seeing through a `(: n T)` annotation,
/// the shape a boundary parameter needs) — the same type `layout::export_params` crossed it as.
fn param_generators(db: &mut rcdzc::db::Db, def: usize) -> Option<Vec<GenKind>> {
    let params = db.defs[def].params.clone();
    let mut kinds = Vec::with_capacity(params.len());
    for p in params {
        // See through a `(: name T)` binder to the name occurrence `type_of` types (bare param → itself).
        let binder = db
            .ast
            .as_form(p, ":")
            .and_then(|t| t.first().copied())
            .unwrap_or(p);
        let ty = rcdzc::infer::type_of(db, binder);
        let kind = match ty {
            rcdzc::ty::Ty::Int(it) => GenKind::Int {
                signed: it.ground_signed(),
                width: it.ground_width(),
            },
            rcdzc::ty::Ty::Bool => GenKind::Bool,
            rcdzc::ty::Ty::Float(_) => GenKind::Float,
            rcdzc::ty::Ty::Char => GenKind::Char,
            _ => return None, // a non-scalar (or unresolved) param — cannot generate it here
        };
        kinds.push(kind);
    }
    Some(kinds)
}

/// The outcome of one trial: PASS (the export returned) or FAIL (it trapped) with the failure message the
/// test reported (via its `Test`/report host effect), if any.
enum TrialOutcome {
    Pass,
    Fail(Option<String>),
}

/// A property-test failure: the (rendered) inputs that reproduced it, and the reported message.
struct PropertyFailure {
    inputs: Vec<String>,
    message: Option<String>,
}

/// Invoke `cdz-run` once on the test component, calling `kebab` with `arg_vals` (rendered `--arg` text).
/// PASS = exit 0; FAIL carries `cdz-run`'s `host-arg` failure message if the test reported one.
fn run_one_trial(
    cdz_run: &std::path::Path,
    component: &std::path::Path,
    kebab: &str,
    store: &std::path::Path,
    arg_vals: &[String],
) -> TrialOutcome {
    run_one_trial_with_pool(cdz_run, component, kebab, store, arg_vals, &[]).0
}

/// The well-known GENERATOR effect operation a property test performs to pull one random `Int64` from the
/// runner's driver: `Test.gen : Unit -> Int64` (the "well-known `Test` effect extends" convention — the
/// same `Test` effect that carries `fail`). `cdz test` answers a `Test.gen` performance with the next int
/// from a seeded pool, so a generator built on this ONE op — bolero's Driver model, one int source that
/// type-directed generation decodes — needs no per-shape host coordination.
const GEN_OP_LABEL: &str = "test.gen";

/// Invoke `cdz-run` once, ALSO supplying a seeded int `pool` as `--host-response Test.gen=<n>` responses
/// (consumed IN ORDER by each `Test.gen` performance — a result-bearing op; a unit op like `Test.fail`
/// consumes none). Returns the trial outcome AND how many `Test.gen` calls the guest actually made (parsed
/// from `cdz-run`'s `host-call` stderr lines) — the signal that distinguishes a PROPERTY test (pulls ≥1
/// generated int) from a plain unit test (pulls none). An unconsumed pool response is harmless (ignored).
fn run_one_trial_with_pool(
    cdz_run: &std::path::Path,
    component: &std::path::Path,
    kebab: &str,
    store: &std::path::Path,
    arg_vals: &[String],
    pool: &[i64],
) -> (TrialOutcome, usize) {
    let mut cmd = std::process::Command::new(cdz_run);
    cmd.arg(component)
        .arg("--call")
        .arg(kebab)
        .arg("--store")
        .arg(store);
    for v in arg_vals {
        cmd.arg("--arg").arg(v);
    }
    for n in pool {
        cmd.arg("--host-response").arg(format!("Test.gen={n}"));
    }
    match cmd.output() {
        Ok(o) => {
            let gens = count_gen_calls(&o.stderr);
            let outcome = if o.status.success() {
                TrialOutcome::Pass
            } else {
                TrialOutcome::Fail(test_failure_message(&o.stderr))
            };
            (outcome, gens)
        }
        Err(e) => (
            TrialOutcome::Fail(Some(format!("could not run `cdz-run`: {e}"))),
            0,
        ),
    }
}

/// How many `Test.gen` performances the guest made, from `cdz-run`'s `host-call\t<op>` stderr lines — the
/// count of generated ints a trial consumed. `> 0` ⇒ the test is a PROPERTY test driven by the int pool.
fn count_gen_calls(stderr: &[u8]) -> usize {
    String::from_utf8_lossy(stderr)
        .lines()
        .filter(|l| l.strip_prefix("host-call\t") == Some(GEN_OP_LABEL))
        .count()
}

/// Run a PROPERTY test `trials` times with generated inputs, returning `None` if every trial passed or the
/// first counterexample (SHRUNK toward a minimal failing input). Generation is seeded (`seed`) so a run is
/// reproducible; each trial advances the seed deterministically (`seed + trial`), so the failing trial's
/// inputs re-generate identically on replay. On the first failing trial, `shrink` searches for a smaller
/// still-failing input before reporting.
fn run_property(
    gens: &[GenKind],
    trials: u64,
    seed: u64,
    run_one: &dyn Fn(&[String]) -> TrialOutcome,
) -> Option<PropertyFailure> {
    for trial in 0..trials {
        let inputs = generate_inputs(gens, seed.wrapping_add(trial));
        if let TrialOutcome::Fail(message) = run_one(&inputs) {
            let (inputs, message) = shrink(gens, &inputs, message, run_one);
            return Some(PropertyFailure { inputs, message });
        }
    }
    None
}

/// What a nullary-signature test turned out to be at runtime: a PLAIN unit test (consumed no generated
/// int — its single-run outcome), or a generator-driven PROPERTY test (`None` = every trial passed, or
/// the shrunk failing int pool).
enum GenDrivenOutcome {
    Plain(TrialOutcome),
    Property(Option<PropertyFailure>),
}

/// The number of random ints a property test's generator is offered per trial — the driver POOL size. A
/// generator pulls as many as its shape needs (a scalar 1, an `(Int64, Bool)` 2, a small list a few); a
/// pool larger than any reasonable shape means the guest never runs dry, and unconsumed responses are
/// ignored. (When compound generators land and can pull unboundedly, this becomes a per-trial budget.)
const GEN_POOL_SIZE: usize = 64;

/// Run a nullary-signature test, deciding PLAIN vs generator-driven PROPERTY by whether it pulls any
/// `Test.gen` int. The FIRST run uses a seeded pool (`seed`); if the guest consumed ZERO generated ints
/// it is a plain unit test — return its outcome directly (one run, today's semantics, unaffected by the
/// unconsumed pool). If it consumed ≥1, it is a property test: run `trials` trials each with a FRESH
/// seeded pool (`seed + trial`, reproducible), failing on the first trapping trial with the SHRUNK pool.
fn run_gen_driven(
    cdz_run: &std::path::Path,
    component: &std::path::Path,
    kebab: &str,
    store: &std::path::Path,
    trials: u64,
    seed: u64,
) -> GenDrivenOutcome {
    let run_pool = |pool: &[i64]| -> (TrialOutcome, usize) {
        run_one_trial_with_pool(cdz_run, component, kebab, store, &[], pool)
    };
    // First trial (trial 0) doubles as the PLAIN-vs-property probe.
    let pool0 = gen_pool(seed, GEN_POOL_SIZE);
    let (outcome0, gens0) = run_pool(&pool0);
    if gens0 == 0 {
        // No generated int consumed → a plain unit test. Its outcome is the single run.
        return GenDrivenOutcome::Plain(outcome0);
    }
    // A property test. Trial 0's result counts; if it already failed, shrink + report.
    if let TrialOutcome::Fail(message) = outcome0 {
        return GenDrivenOutcome::Property(Some(shrink_pool(&pool0, gens0, message, &run_pool)));
    }
    // Remaining trials, each a fresh seeded pool.
    for trial in 1..trials {
        let pool = gen_pool(seed.wrapping_add(trial), GEN_POOL_SIZE);
        let (outcome, gens) = run_pool(&pool);
        if let TrialOutcome::Fail(message) = outcome {
            return GenDrivenOutcome::Property(Some(shrink_pool(&pool, gens, message, &run_pool)));
        }
    }
    GenDrivenOutcome::Property(None)
}

/// A seeded pool of `size` random `Int64`s — the driver stream a property test's generator pulls from.
/// Reproducible from `seed` (bolero's `driver::Rng` over a seeded `Xoshiro256PlusPlus`), so a reported
/// failing seed replays the exact pool.
fn gen_pool(seed: u64, size: usize) -> Vec<i64> {
    use bolero_generator::driver::{self, Rng};
    use bolero_generator::{ValueGenerator, produce};
    let rng = rand_from_seed(seed);
    let mut d = Rng::new(rng, &driver::Options::default());
    (0..size)
        .map(|_| produce::<i64>().generate(&mut d).unwrap_or(0))
        .collect()
}

/// SHRINK a failing int pool toward a minimal counterexample: reduce the CONSUMED prefix (`gens` ints —
/// the ones the generator actually pulled; trailing pool entries never affected the run) toward 0, one
/// position at a time by halving, keeping any reduction that STILL fails. Reports the consumed prefix
/// (rendered) — the ints that reproduce the failure. Greedy + bounded, like the scalar `shrink`.
///
/// This IS the harness's shrinking search: on a failing property it searches for a SMALLER input that
/// still fails (halving each consumed position, keeping only reductions that still `Fail`); it TERMINATES
/// (each position halves toward 0, `while n != 0`, and a non-failing candidate breaks that position — no
/// unbounded search); and it REPORTS the minimal `best` prefix it converged to as the counterexample.
//= spec/capabilities/property-based-testing.md#shrinking-converges-to-a-minimal-failing-input
//# When a property fails, the harness MUST search for a smaller input that still fails.
//= spec/capabilities/property-based-testing.md#shrinking-converges-to-a-minimal-failing-input
//# The shrinking search MUST terminate rather than search unboundedly.
//= spec/capabilities/property-based-testing.md#shrinking-converges-to-a-minimal-failing-input
//# The shrinking search MUST report a minimal failing input.
fn shrink_pool(
    pool: &[i64],
    gens: usize,
    message: Option<String>,
    run_pool: &dyn Fn(&[i64]) -> (TrialOutcome, usize),
) -> PropertyFailure {
    // Only the CONSUMED prefix matters — the generator pulled `gens` ints; the rest of the pool is inert.
    let mut best: Vec<i64> = pool.iter().take(gens).copied().collect();
    let mut best_msg = message;
    for i in 0..best.len() {
        let mut n = best[i];
        while n != 0 {
            n /= 2;
            let mut trial = best.clone();
            trial[i] = n;
            // Re-run with the candidate prefix (the runner pads with the untouched trailing pool via the
            // original size is unnecessary — the consumed prefix is what the generator reads in order).
            let (outcome, _) = run_pool(&trial);
            if matches!(outcome, TrialOutcome::Fail(_)) {
                best[i] = n;
                if let (TrialOutcome::Fail(m), _) = run_pool(&best) {
                    best_msg = m;
                }
            } else {
                break; // this position can't shrink further while still failing
            }
        }
    }
    PropertyFailure {
        inputs: best.iter().map(|n| n.to_string()).collect(),
        message: best_msg,
    }
}

/// Generate one `--arg` string per generator, from a driver seeded at `seed` — bolero's `driver::Rng`
/// (a seeded, reproducible driver) feeding each type's `ValueGenerator`. The rendered forms are exactly
/// what `cdz-run`'s `coerce_one` parses (`5`, `-3`, `true`, `1.5`, a single char).
///
/// The generation is a pure function of `seed`: the same seed re-produces the same inputs on every run, so
/// a property run is reproducible from its recorded seed (`run_property` seeds trial `t` at `seed + t`, and
/// `--seed` replays the exact pool). This is what lets a reported failure be replayed deterministically.
//= spec/capabilities/property-based-testing.md#generation-is-seeded-and-reproducible
//# A property run MUST be reproducible from its recorded seed, producing the same inputs on every conforming run.
fn generate_inputs(gens: &[GenKind], seed: u64) -> Vec<String> {
    use bolero_generator::driver::{self, Rng};
    use bolero_generator::{ValueGenerator, produce};
    let rng = rand_from_seed(seed);
    let mut d = Rng::new(rng, &driver::Options::default());
    gens.iter()
        .map(|g| match g {
            GenKind::Bool => produce::<bool>()
                .generate(&mut d)
                .unwrap_or(false)
                .to_string(),
            GenKind::Char => produce::<char>()
                .generate(&mut d)
                .unwrap_or('a')
                .to_string(),
            GenKind::Float => {
                let v = produce::<f64>().generate(&mut d).unwrap_or(0.0);
                // Render a finite decimal `coerce_one` (`parse::<f64>`) accepts; a non-finite generated
                // value falls back to 0 (NaN/inf have no re-parseable decimal here).
                if v.is_finite() { v } else { 0.0 }.to_string()
            }
            GenKind::Int { signed, width } => {
                let raw = produce::<i64>().generate(&mut d).unwrap_or(0);
                render_int(raw, *signed, *width)
            }
        })
        .collect()
}

/// The maximum number of cases `@exhaustive` will enumerate. A domain larger than this is treated as
/// unbounded (`exhaustive_domain` returns `None`) — enumerating millions of cases would be a denial of
/// service, not a proof. `Bool`×`Bool` = 4, a `UInt8` = 256, `UInt8`×`Bool` = 512 all fit comfortably;
/// a 16-bit int (65 536) fits; a 32/64-bit int or a float does not (narrow the type to prove exhaustively).
const MAX_EXHAUSTIVE_CASES: usize = 100_000;

/// The COMPLETE input domain of a property whose parameters are all bounded scalars — every combination of
/// each parameter's full value set, as rendered `--arg` strings (the Cartesian product). `None` if the
/// domain is unbounded/too large (any `Float`, or an integer width whose range times the running product
/// would exceed [`MAX_EXHAUSTIVE_CASES`]) — such a property cannot be exhaustively proven and must narrow
/// its types. An empty `gens` (a nullary signature) yields one case (the empty argument list), though the
/// exhaustive path is only taken for a parameterized boundary-arg test.
fn exhaustive_domain(gens: &[GenKind]) -> Option<Vec<Vec<String>>> {
    // Build the per-parameter value sets (each the full rendered domain of that scalar), bailing if any is
    // unbounded, while tracking the running product so we stop before building an enormous set.
    let mut per_param: Vec<Vec<String>> = Vec::with_capacity(gens.len());
    let mut product: usize = 1;
    for g in gens {
        let values = scalar_domain(g)?;
        product = product.checked_mul(values.len())?;
        if product > MAX_EXHAUSTIVE_CASES {
            return None;
        }
        per_param.push(values);
    }
    // Cartesian product of the per-parameter value sets, in row-major order (last parameter varies
    // fastest), seeded with one empty tuple.
    let mut domain: Vec<Vec<String>> = vec![Vec::new()];
    for values in &per_param {
        let mut next = Vec::with_capacity(domain.len() * values.len());
        for prefix in &domain {
            for v in values {
                let mut row = prefix.clone();
                row.push(v.clone());
                next.push(row);
            }
        }
        domain = next;
    }
    Some(domain)
}

/// The full rendered value domain of ONE bounded scalar generator, or `None` if it is unbounded/too large.
/// `Bool` = {false, true}; `Char` is bounded but astronomically large (all Unicode scalars) so it is not
/// enumerated here; an integer is enumerable only for narrow widths (≤16 bits) whose range fits within
/// [`MAX_EXHAUSTIVE_CASES`]; a `Float` is never enumerable. Each value is rendered exactly as
/// `generate_inputs` renders it (so `cdz-run`'s `coerce_one` accepts it).
fn scalar_domain(g: &GenKind) -> Option<Vec<String>> {
    match g {
        GenKind::Bool => Some(vec!["false".to_string(), "true".to_string()]),
        // A `Char`'s domain is every Unicode scalar (~1.1M) — far past the cap; a float is infinite. Not
        // exhaustively enumerable (narrow to a bounded integer/Bool instead).
        GenKind::Char | GenKind::Float => None,
        GenKind::Int { signed, width } => {
            // Only widths whose FULL range fits the cap are enumerable (8/16-bit); 32/64-bit are unbounded
            // for this purpose. Enumerate the whole range, rendered via `render_int` (same as sampling).
            let range: Vec<i64> = match (signed, width) {
                (false, 8) => (0..=u8::MAX as i64).collect(),
                (true, 8) => (i8::MIN as i64..=i8::MAX as i64).collect(),
                (false, 16) => (0..=u16::MAX as i64).collect(),
                (true, 16) => (i16::MIN as i64..=i16::MAX as i64).collect(),
                _ => return None, // 32/64-bit (or a deferred width) — too large to enumerate
            };
            Some(
                range
                    .into_iter()
                    .map(|v| render_int(v, *signed, *width))
                    .collect(),
            )
        }
    }
}

/// Render a generated `i64` as the decimal text for an integer parameter of the given signedness/width,
/// truncated into that width's range so `cdz-run`'s `parse::<iN/uN>` accepts it (a wider raw value would
/// fail to parse as the narrower type). The `as` truncation into range keeps the full value spread.
fn render_int(raw: i64, signed: bool, width: u32) -> String {
    match (signed, width) {
        (true, 8) => (raw as i8).to_string(),
        (false, 8) => (raw as u8).to_string(),
        (true, 16) => (raw as i16).to_string(),
        (false, 16) => (raw as u16).to_string(),
        (true, 32) => (raw as i32).to_string(),
        (false, 32) => (raw as u32).to_string(),
        (false, 64) => (raw as u64).to_string(),
        // signed 64 (and any deferred/other width defaults to i64 at the boundary).
        _ => raw.to_string(),
    }
}

/// A reproducible RNG from a `u64` seed — `Xoshiro256PlusPlus` (bolero's own generator rng), whose
/// `seed_from_u64` SplitMix64-expands the seed to the full state, so `cdz test --seed N` is deterministic
/// without depending on OS entropy.
fn rand_from_seed(seed: u64) -> rand_xoshiro::Xoshiro256PlusPlus {
    use rand_core::SeedableRng;
    use rand_xoshiro::Xoshiro256PlusPlus;
    Xoshiro256PlusPlus::seed_from_u64(seed)
}

/// SHRINK a failing property input toward a minimal counterexample: for each argument position, try
/// replacing it with progressively "smaller" values (an integer toward 0 by halving, a bool toward
/// `false`, a float toward 0, a char toward `a`) and keep any replacement that STILL fails. Greedy +
/// bounded — one left-to-right pass per position, each position bisected — so it terminates quickly and
/// reports a smaller, more legible witness than the raw random input. Returns the shrunk inputs + the
/// (possibly updated) failure message from the last failing run.
fn shrink(
    gens: &[GenKind],
    inputs: &[String],
    message: Option<String>,
    run_one: &dyn Fn(&[String]) -> TrialOutcome,
) -> (Vec<String>, Option<String>) {
    let mut best = inputs.to_vec();
    let mut best_msg = message;
    for (i, g) in gens.iter().enumerate() {
        for candidate in shrink_candidates(g, &best[i]) {
            let mut trial = best.clone();
            trial[i] = candidate;
            if let TrialOutcome::Fail(m) = run_one(&trial) {
                best = trial;
                best_msg = m;
            }
        }
    }
    (best, best_msg)
}

/// The ordered shrink candidates for one argument (largest-reduction first), by kind: an integer halves
/// toward 0 (then 0); a bool toward `false`; a float toward 0; a char toward `a`. Each is a value that,
/// if it still fails, is a smaller witness than the current one.
fn shrink_candidates(g: &GenKind, current: &str) -> Vec<String> {
    match g {
        GenKind::Int { .. } => {
            let Ok(mut n) = current.parse::<i64>() else {
                return Vec::new();
            };
            let mut out = Vec::new();
            // Halve toward 0 (a geometric descent), ending at 0 — a bounded sequence.
            while n != 0 {
                n /= 2;
                out.push(n.to_string());
            }
            out
        }
        GenKind::Bool => {
            if current == "true" {
                vec!["false".to_string()]
            } else {
                Vec::new()
            }
        }
        GenKind::Float => {
            if current != "0" {
                vec!["0".to_string()]
            } else {
                Vec::new()
            }
        }
        GenKind::Char => {
            if current != "a" {
                vec!["a".to_string()]
            } else {
                Vec::new()
            }
        }
    }
}

/// Locate the sibling `cdz-run` binary — it lives beside THIS binary in `target/<profile>/` (both are
/// built together). `current_exe().parent()/cdz-run` is the robust, install-location-independent path
/// (the same `current_exe`-relative convention xtask uses to re-invoke its own tools). `None` if absent.
fn locate_cdz_run() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|p| {
            p.parent().map(|dir| {
                dir.join(if cfg!(windows) {
                    "cdz-run.exe"
                } else {
                    "cdz-run"
                })
            })
        })
        .filter(|p| p.exists())
}

/// The failure MESSAGE a trapped test emitted — the FIRST `host-arg\t<op>\t<message>` line in `cdz-run`'s
/// stderr (the assertion text a test performs via its report host effect before trapping). `None` when no
/// message rode along (a test that trapped with no host report — just the bare trap). The `host-arg`
/// protocol is `cdz-run`'s additive channel for a host call's STRING argument (`main.rs`), distinct from
/// the gate's `host-call` line, so reading it here never collides with the observed-op sequence.
fn test_failure_message(stderr: &[u8]) -> Option<String> {
    String::from_utf8_lossy(stderr)
        .lines()
        .find_map(|l| l.strip_prefix("host-arg\t").map(str::to_string))
        // `host-arg` is `<op>\t<message>`; take the message (everything after the op's tab).
        .and_then(|entry| entry.split_once('\t').map(|(_op, msg)| msg.to_string()))
}

/// The default content-addressed runtime store — `<repo>/target/cadenza-store`, resolved relative to this
/// binary (`target/<profile>/cdz` → up two → `target` → `cadenza-store`). Mirrors `cdz-run`'s own default
/// so `cdz test` and a direct `cdz-run` agree on where the value-heap runtime lives.
fn default_store() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| {
            p.parent()
                .and_then(|d| d.parent())
                .map(|t| t.join("cadenza-store"))
        })
        .unwrap_or_else(|| PathBuf::from("target/cadenza-store"))
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

// ── project build ─────────────────────────────────────────────────────────────────────────────────

#[derive(clap::Args)]
struct BuildArgs {
    /// The project to build: a `Project.cdz` manifest, or a DIRECTORY holding one. OMITTED → search up
    /// from the current directory for the nearest `Project.cdz` (like `cargo build` finding
    /// `Cargo.toml`). The manifest's `entry` (+ `modules`) are compiled together into one component.
    dir: Option<String>,
    /// Where to write the built component. A directory holding `<entry>.wasm`; or, when this is not an
    /// existing directory, the exact output file path. Defaults to the current directory.
    #[arg(long, short)]
    out: Option<PathBuf>,
    /// Build the RELEASE tier (`O2` — whole-function passes: inlining, global CSE, LICM). Shorthand for
    /// `--opt-level O2`; `--opt-level` wins if both are given. Without it, the dev tier (`O1`).
    #[arg(long)]
    release: bool,
    /// The optimization LEVEL (`O0`..`O3`), overriding both `--release` and any `Project.cdz` `opt-level`.
    /// Omitted → the manifest's `opt-level`, else `--release`'s `O2`, else the default `O1`.
    #[arg(long, value_name = "LEVEL")]
    opt_level: Option<String>,
    /// The backend target to emit. `wasm` (the default) → a WebAssembly component; `rust` → a `.rs`
    /// module. Same targets as `cdz compile`, chosen here at the project level.
    #[arg(long, value_enum, default_value_t = BuildTargetArg::Wasm)]
    target: BuildTargetArg,
}

/// The backend target for `cdz build` — a small clap `ValueEnum` mapping to `rcdzc::Target` (the two a
/// project build picks between; `cdz compile` still offers the debug/dwarf/async targets for a
/// finer-grained single-file build). Its own enum so `--help` lists `wasm`/`rust` and clap validates it.
#[derive(Clone, Copy, clap::ValueEnum)]
enum BuildTargetArg {
    /// A WebAssembly component (the default).
    Wasm,
    /// A Rust source module (`.rs`).
    Rust,
}

impl From<BuildTargetArg> for rcdzc::Target {
    fn from(t: BuildTargetArg) -> rcdzc::Target {
        match t {
            BuildTargetArg::Wasm => rcdzc::Target::Wasm,
            BuildTargetArg::Rust => rcdzc::Target::Rust,
        }
    }
}

// ── unit testing ───────────────────────────────────────────────────────────────────────────────────

#[derive(clap::Args)]
struct TestArgs {
    /// What to test: a FILE (its `@test` defs), a DIRECTORY (its `Project.cdz` suite, else every source
    /// file), or a `Project.cdz`. OMITTED → search up from the current directory for the nearest
    /// `Project.cdz` and run its suite (like `cargo test` finding `Cargo.toml`).
    file: Option<String>,
    /// Run only tests whose name CONTAINS this substring (a filter). Absent = run every `@test`.
    #[arg(long)]
    filter: Option<String>,
    /// Run only tests whose def carries this `@tag("…")` string tag. Absent = no tag constraint. Composes
    /// with `--filter` by AND (a test runs iff it matches BOTH the name substring and carries this tag).
    #[arg(long)]
    tag: Option<String>,
    /// The content-addressed runtime store `cdz-run` resolves the value-heap runtime from, if a test
    /// builds heap values. Defaults to `<repo>/target/cadenza-store` (built by `cargo xtask build`).
    #[arg(long)]
    store: Option<PathBuf>,
    /// Trials per PROPERTY test — a `@test` that takes parameters is run this many times with generated
    /// inputs (a nullary `@test` runs once regardless). Default 100.
    #[arg(long, default_value_t = 100)]
    trials: u64,
    /// The random SEED for property-input generation — the run is reproducible from it (a reported
    /// failure prints the seed to replay with `--seed`). Default 0 (deterministic run-to-run).
    #[arg(long, default_value_t = 0)]
    seed: u64,
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
    /// What to check: a FILE (its program + import closure), a DIRECTORY (every source file under it,
    /// recursively), or a `Project.cdz` (its `modules`+`entry`). OMITTED → search up from the current
    /// directory for the nearest `Project.cdz` and check its project (like `cdz test`/`cdz build`). A
    /// single file with no manifest around it just checks that file.
    file: Option<String>,
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
struct SymbolsArgs {
    /// The program file (`.cdz`/`.ml` → ml, `.sexp`/`.sexpr` → sexpr).
    file: String,
}

#[derive(clap::Args)]
struct InstantiationsArgs {
    /// The generic / ad-hoc-polymorphic definition name whose concrete instantiations to enumerate.
    name: String,
    /// The program file (`.cdz`/`.ml` → ml, `.sexp`/`.sexpr` → sexpr).
    file: String,
}

#[derive(clap::Args)]
struct HighlightArgs {
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

#[derive(clap::Args)]
struct DocArgs {
    /// The definition (or built-in) name to document.
    name: String,
    /// The program file (`.cdz`/`.ml` → ml, `.sexp`/`.sexpr` → sexpr).
    file: String,
}

#[derive(clap::Args)]
struct DocAtOffsetArgs {
    /// The program file (`.cdz`/`.ml` → ml, `.sexp`/`.sexpr` → sexpr).
    file: String,
    /// The source BYTE OFFSET whose documentation to show — the cursor position (0-based, UTF-8 bytes).
    offset: usize,
}

/// Does a total by-NAME query's rendered result (`type`/`doc`/`instantiations`) mean "the name resolves
/// to NOTHING" (a typo) rather than a real answer? The `TypeOf`/`DocOf`/… sidecar queries are TOTAL — they
/// return a defined line even for an unknown name: `no such definition `<name>`` optionally followed by a
/// hint (` — did you mean `Y`?` OR ` — closest matches: …`). A `cdz` command maps THIS verdict to a
/// non-zero exit so a script can tell a typo from a real (if empty/undocumented) answer.
///
/// Match by RECONSTRUCTING the exact sentinel for the QUERIED `name` and comparing the WHOLE trimmed text
/// (`== sentinel`, or `sentinel + " — "` + any hint) — NOT a loose `contains`/`starts_with` on arbitrary
/// rendered prose, which would misclassify a legitimate result that merely began with that phrase (the
/// pr467 brittleness fix, generalized across the by-name queries).
fn is_no_such_definition(rendered: &str, name: &str) -> bool {
    let sentinel = format!("no such definition `{name}`");
    let trimmed = rendered.trim();
    trimmed == sentinel || trimmed.starts_with(&format!("{sentinel} — "))
}

/// `cdz type NAME FILE` — parse in-process, drive the compiler's `TypeOf` sidecar query, print the
/// rendered type. A query is a pure, total fact read: it answers even for a program that would not
/// compile (`DESIGN-sidecar-api.md`). An UNRESOLVABLE name (a typo) exits non-zero — the answer is still
/// printed, but it's a "no such definition" verdict, not a type.
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
            let text = String::from_utf8_lossy(bytes);
            println!("{text}");
            // A total query answers even for an unknown name with a "no such definition `X`" verdict —
            // map that to a FAILURE (a typo isn't a type), while a real type stays SUCCESS.
            if is_no_such_definition(&text, &args.name) {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
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

/// `cdz doc NAME FILE` — drive the compiler's `DocOf` sidecar query, print the documentation. Answers
/// from a user definition's `(doc "…")` text, else a built-in's `(meta doc)` channel / a grammar keyword's
/// help. A pure, total fact read (like `cdz type`): it answers even for a program that would not compile,
/// and a name that documents nothing prints a defined "no documentation" line (exit 0).
fn run_doc(args: &DocArgs) -> ExitCode {
    let (source, arenas) = match load_program(&args.file) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{PROG}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let _ = source; // doc output carries no span
    let out = run_sidecar(
        &arenas,
        rcdzc::Request::Query(rcdzc::sidecar::Query::DocOf {
            name: args.name.clone(),
        }),
    );
    match out.artifact(rcdzc::sidecar::KIND_DOC) {
        Some(bytes) => {
            let text = String::from_utf8_lossy(bytes);
            println!("{text}");
            // The `DocOf` query is TOTAL — it returns a doc artifact for THREE outcomes: the doc text, a
            // "no documentation for `X`" line (a REAL definition that carries no doc), and a "no such
            // definition `X`" line (the name resolves to NOTHING — a typo). The first two are a SUCCESS
            // (`X` exists; asking for its doc is a legitimate answer), but an unresolvable name is a
            // FAILURE — a caller/script should tell "you misspelled the name" from "this exists but is
            // undocumented". `is_no_such_definition` matches the exact sentinel for the queried name (not a
            // loose prefix on the doc prose — the pr467 brittleness fix, shared with `cdz type`).
            if is_no_such_definition(&text, &args.name) {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
        None => {
            report_errors(&out);
            ExitCode::FAILURE
        }
    }
}

/// `cdz doc-at FILE OFFSET` — the "documentation at cursor" query. Resolves the source byte offset to the
/// innermost node id (via the span table this process kept), drives the compiler's `DocAt { node }` query,
/// and prints the documentation of the definition that node is or references. The offset→node split keeps
/// the compiler span-free, exactly as `type-at`/`def` do. An empty result (a node that documents nothing)
/// prints a "no documentation" line.
fn run_doc_at(args: &DocAtOffsetArgs) -> ExitCode {
    let (source, arenas, spans) = match load_program_spanned(&args.file) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{PROG}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let _ = source; // doc output carries no span
    let Some(node) = spans.node_at_offset(args.offset) else {
        eprintln!(
            "{PROG}: no node at byte offset {} in {}",
            args.offset, args.file
        );
        return ExitCode::FAILURE;
    };
    let out = run_sidecar(
        &arenas,
        rcdzc::Request::Query(rcdzc::sidecar::Query::DocAt { node: node.0 }),
    );
    let Some(bytes) = out.artifact(rcdzc::sidecar::KIND_DOC) else {
        report_errors(&out);
        return ExitCode::FAILURE;
    };
    let doc = String::from_utf8_lossy(bytes);
    // A total query: an empty answer means the node documents nothing — say so rather than print a blank.
    if doc.trim().is_empty() {
        println!("no documentation at byte offset {}", args.offset);
    } else {
        println!("{doc}");
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

/// `cdz check [TARGET]` — report every well-formedness fault, "diagnostics as you type". Drives the
/// compiler's `Query::Diagnostics` (the fault set, NOT gated on export/emit), maps each fault's node id
/// to `file:line:col` via the span table, and prints `file:line:col: severity [CODE]: message`. Exits
/// non-zero iff any error-severity fault is present (a clean check prints nothing and exits 0) — the
/// CI-gate / editor-lint shape.
///
/// TARGET resolves like `cdz test`/`cdz build`: a FILE checks that file (+ its import closure); a
/// DIRECTORY or `Project.cdz` (or no arg → the nearest `Project.cdz` upward) checks the whole PROJECT —
/// every source file (a manifest's `modules`+`entry`, else every source file under the dir), aggregating
/// diagnostics and failing if ANY file has an error. So `cdz check` with no arg lints a whole project in
/// one call, matching how `cdz test`/`cdz build` treat a project.
fn run_check(args: &CheckArgs) -> ExitCode {
    // The set of files to check. A single FILE → just it (check_one follows its closure). A DIRECTORY or
    // `Project.cdz` (or no arg → upward search) → the project's source files, resolved the same way
    // `cdz test` resolves its suite (a manifest's `modules`+`entry` globbed, else a source-file walk).
    let files: Vec<String> = match resolve_check_targets(args.file.as_deref()) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("{PROG}: {e}");
            return ExitCode::FAILURE;
        }
    };
    // Check each; a project fails if ANY file has an error-severity fault (OR the per-file results). Every
    // file is checked (not short-circuited) so the user sees ALL diagnostics in one run. But a module
    // that is BOTH a manifest target AND imported by another target would be checked twice — once
    // standalone, once via the importer's closure — DOUBLE-reporting its diagnostics. So skip a target
    // whose file was already pulled into an EARLIER target's import closure: `check_one(entry)` links +
    // checks the entry's whole closure (its imported modules included), so a later standalone check of
    // one of those modules is redundant. Dedup by canonical path (so `./util.cdz` and `util.cdz` match).
    let canon = |p: &str| {
        std::fs::canonicalize(p)
            .map(|c| c.to_string_lossy().into_owned())
            .unwrap_or_else(|_| p.to_string())
    };
    let mut covered: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut any_error = false;
    for f in &files {
        // Canonicalize ONCE per target (a filesystem `canonicalize` + allocation) — reused for both the
        // already-covered test and the insert below, rather than recomputing it each place.
        let canon_f = canon(f);
        if covered.contains(&canon_f) {
            continue; // already checked via an earlier target's closure — don't double-report
        }
        // `check_one` follows + parses f's whole import closure to check it; it hands the closure's file
        // paths back so we mark f AND every file it pulled in as covered WITHOUT reloading + reparsing the
        // same closure here (the redundant second load this used to do). A later target that is one of
        // those imported modules is then skipped. On a load error the returned closure is empty, so f
        // itself is still covered below (check_one already reported the error).
        let (had_error, closure_paths) = check_one(f, args.json, args.verify_fixes);
        any_error |= had_error;
        covered.insert(canon_f);
        for path in &closure_paths {
            covered.insert(canon(path));
        }
    }
    if any_error {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// Resolve `cdz check`'s TARGET to the list of files to check. A single FILE → `[file]` (check_one
/// follows its closure). A `Project.cdz`/DIRECTORY (or `None` → the nearest `Project.cdz` upward) → the
/// project's source files: a manifest's `modules`+`entry` (glob-expanded, `exclude`-filtered) if a
/// manifest is present, else every source file under the directory (path-sorted). Mirrors `cdz test`'s
/// resolution so the three project commands agree on "what is the project".
fn resolve_check_targets(target: Option<&str>) -> Result<Vec<String>, String> {
    // No arg → the nearest Project.cdz upward (a project-wide check, like `cdz test`/`cdz build`).
    let target: String = match target {
        Some(t) => t.to_string(),
        None => match find_manifest_upward() {
            Some(p) => p.to_string_lossy().into_owned(),
            None => {
                return Err(format!(
                    "no `{MANIFEST_NAME}` found in the current directory or any ancestor \
                     (name a file/dir to check, or add a `{MANIFEST_NAME}`)"
                ));
            }
        },
    };
    let path = std::path::Path::new(&target);
    let is_manifest_arg = path.file_name().and_then(|n| n.to_str()) == Some(MANIFEST_NAME);
    // Naming a `Project.cdz` that DOESN'T EXIST is an error, not a silent fallback: without this, the arg
    // resolves to its parent dir and `load_manifest` returns `Ok(None)` (no manifest there), so we'd
    // quietly dir-walk instead of the manifest the user explicitly named. Fail with a clear "no such file".
    if is_manifest_arg && !path.is_file() {
        return Err(format!("no such file `{target}`"));
    }
    // A `Project.cdz` (arg or a dir holding one): check the manifest's declared files. Else a plain dir:
    // walk every source file. Else a single file: just it.
    let dir: Option<std::path::PathBuf> = if is_manifest_arg {
        Some(match path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
            _ => std::path::Path::new(".").to_path_buf(),
        })
    } else if path.is_dir() {
        Some(path.to_path_buf())
    } else {
        None
    };
    let Some(dir) = dir else {
        // A single file — check it alone (its import closure is followed by check_one).
        return Ok(vec![target]);
    };
    // A directory: prefer its manifest's file set; else walk every source file under it.
    match load_manifest(&dir)? {
        Some((mpath, m)) => {
            // The manifest's checkable files: the `entry` FIRST, then its library `modules`. Entry-first
            // so the dedup in `run_check` works: `check_one(entry)` links + checks the entry's whole
            // import closure (the modules it uses), marking them covered — so a module reached that way is
            // then skipped rather than re-checked standalone (which would double-report its diagnostics).
            let mut pats = Vec::new();
            if let Some(entry) = &m.entry {
                pats.push(entry.clone());
            }
            pats.extend(m.modules.iter().cloned());
            if pats.is_empty() {
                return Err(format!(
                    "{}: the manifest declares no `entry`/`modules` to check",
                    mpath.display()
                ));
            }
            let files = expand_manifest_globs(&dir, &pats, &m.exclude);
            if files.is_empty() {
                return Err(format!(
                    "{}: the manifest's `entry`/`modules` matched no files",
                    mpath.display()
                ));
            }
            Ok(files)
        }
        // No manifest — walk every source file under the directory (the same collection `cdz compile
        // <dir>` and `cdz test <dir>` use).
        None => {
            let mut out = Vec::new();
            collect_source_dir(&dir, &mut out)?;
            if out.is_empty() {
                return Err(format!(
                    "{}: no source files (.cdz/.ml/.sexp) to check",
                    dir.display()
                ));
            }
            Ok(out)
        }
    }
}

/// Check ONE file (following its import closure) — the per-file core of `cdz check`. Returns
/// `(had_error, closure_paths)`: `had_error` is `true` if any error-severity fault (including a parse
/// error) was reported (so a project-wide [`run_check`] can OR the results across many files), and
/// `closure_paths` is the on-disk path of EVERY file this check pulled into the import closure — so the
/// caller can mark them covered WITHOUT reloading + reparsing the closure (which `check_one` already
/// did). On a load failure the closure is empty (the caller still covers `file` itself).
/// `json`/`verify_fixes` are the `cdz check` flags.
fn check_one(file: &str, json: bool, verify_fixes: bool) -> (bool, Vec<String>) {
    // Follow the entry file's IMPORT CLOSURE so a cross-file reference (an imported type or definition)
    // resolves and checks — `cdz check FILE` then sees the SAME linked program the package compile does.
    // A file that imports nothing loads as a lone file, byte-identical to a standalone check; only a file
    // carrying an `(import …)` pulls its transitively-imported siblings in. A diagnostic that lands in an
    // imported library is reported at THAT library's own `path:line:col` via the `link-map` demux below.
    let files = match load_import_closure_with(file, &|_| None) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("{PROG}: {e}");
            return (true, Vec::new()); // load failure = an error; nothing loaded to cover
        }
    };
    // A file that did NOT fully parse — an unclosed `(`, an arm-less `match`, a `then` with no `else`.
    // The ML reader RECOVERS (prints each syntax error to stderr, then hands back a truncated arena of
    // `<error>` placeholder nodes), so the check proceeds; but the recovered program is NOT what the
    // author wrote. Two consequences to fix:
    //  1. EXIT: a clean truncation (`(1 + 2` → the `(…` is just dropped) can leave the recovered arena
    //     with NO semantic fault, so `check` reported SUCCESS on a file that does not parse — silently
    //     breaking its "exits non-zero if any error-severity fault is present" contract (an editor/CI
    //     then treats a broken file as clean). A parse error IS an error-severity fault; force FAILURE.
    //  2. CASCADE: an `<error>` placeholder in expression position reduces to a bare NAME `<error>`,
    //     which the checker reports as `unbound name `<error>`` (CDZ0101) — a spurious fault referencing
    //     a token the user never wrote, layered on top of the real parse error. `<error>` is UNLEXABLE on
    //     the ML surface (`<` starts no identifier), so an `<error>`-named diagnostic there is ALWAYS the
    //     placeholder, never a real name — drop it (the parse error already said what to fix).
    let any_parse_error = files.iter().any(|f| f.parse_errors > 0);
    // Route through the package/link path whenever the ENTRY declares an `(import …)` — even if it is the
    // only file loaded (an import naming no sibling). Then `link()` reports the precise "unknown package
    // file" diagnostic (CDZ0201) instead of the generic "imports are not modeled here" a bare single-file
    // compile falls back to. A file with NO imports takes the single-file path, byte-identical to before.
    let is_package = !declared_import_paths(&files[0].arenas).is_empty();
    let out = if is_package {
        // Splice all closure files into one program and run Diagnostics over the WHOLE package (so a name
        // defined in an imported file resolves). `link()` needs the entry named — it is `files[0]`.
        let mut inputs: Vec<rcdzc::Artifact> = files
            .iter()
            .map(|f| {
                rcdzc::Artifact::new(
                    rcdzc::Artifact::KIND_AST,
                    f.name.clone(),
                    cadenza_syntax::codec::encode(&f.arenas),
                )
            })
            .collect();
        inputs.push(rcdzc::Artifact::new(
            rcdzc::sidecar::KIND_SIDECAR,
            "drive",
            rcdzc::sidecar::encode(&[rcdzc::Request::Query(rcdzc::sidecar::Query::Diagnostics)]),
        ));
        inputs.push(compiler_cli::entry_artifact(&files[0].name));
        rcdzc::run_with_compiler_stack(|| rcdzc::compile(&inputs, &[]))
    } else {
        run_sidecar(
            &files[0].arenas,
            rcdzc::Request::Query(rcdzc::sidecar::Query::Diagnostics),
        )
    };
    let Some(bytes) = out.artifact(rcdzc::sidecar::KIND_DIAGNOSTICS) else {
        report_errors(&out);
        // The diagnostics query itself failed = an error. The closure DID load, so still hand its paths
        // back for the caller's coverage set (the files were checked as far as this point).
        let closure_paths = files.into_iter().map(|f| f.path).collect();
        return (true, closure_paths);
    };
    let text = String::from_utf8_lossy(bytes);
    let mut any_error = false;
    // The package demux table (`link-map`) — absent for a single file, so every node belongs to the
    // entry with its local id == the global id.
    let link_map = out
        .artifact(rcdzc::link::KIND_LINK_MAP)
        .map(rcdzc::link::decode_link_map)
        .unwrap_or_default();
    // One line-start index per file (binary-searched line:col), parallel to `files`.
    let indices: Vec<_> = files
        .iter()
        .map(|f| cadenza_syntax::query::driver::LineIndex::new(&f.source))
        .collect();
    // Demux a node id to `(file index, that file's LOCAL id)`. Single file: `(0, id)`. Package: the file
    // whose `[base, base+count)` global range holds the id, minus the base (`link::FileSpan`). A node in
    // no file (the synthesized `(do …)` root, or a prelude/β-copy node) demuxes to `None`.
    let file_of_node = |node: &str| -> Option<(usize, u32)> {
        let n = node.parse::<u32>().ok()?;
        if link_map.is_empty() {
            return Some((0, n));
        }
        let fs = link_map
            .iter()
            .find(|fs| n >= fs.struct_base && n < fs.struct_base + fs.struct_count)?;
        let file_ix = files.iter().position(|f| f.name == fs.path)?;
        Some((file_ix, n - fs.struct_base))
    };
    // The ORIGINAL program's diagnostic set — the baseline `--verify-fixes` judges each candidate fix
    // against (a fix upgrades to verified only if it clears its fault AND introduces no new error).
    // Computed once (a recompile), only when `--verify-fixes` is set AND the check is a single file — a
    // PACKAGE fix would need re-linking the whole package to verify (a follow-up), so a package fix stays
    // heuristic unless the compiler already proved it. Single file: byte-identical to before.
    let baseline_errors: Option<Vec<(String, String, String)>> = if verify_fixes && !is_package {
        program_diagnostic_keys(&files[0].source, is_ml_source(&files[0].path))
    } else {
        None
    };
    // A node id → its 1-based `(line, col)` start plus its `[from, to)` UTF-8 byte range, in ITS file's
    // span table. `None` for an unanchored (`-`) or unmapped node. Per-file line indices keep this linear
    // even when a program has MANY diagnostics (each mapped here, some twice via the source-order sort).
    let span_of = |node: &str| -> Option<(usize, usize, usize, usize)> {
        let (fi, local) = file_of_node(node)?;
        let span = files[fi].spans.get(cadenza_syntax::StructId(local))?;
        let (l, c) = indices[fi].line_col(&files[fi].source, span.start);
        Some((l, c, span.start, span.end))
    };
    let loc_label = |node: &str| match file_of_node(node) {
        Some((fi, _)) => match span_of(node) {
            Some((l, c, _, _)) => format!("{}:{l}:{c}", files[fi].path),
            None => files[fi].path.clone(),
        },
        None => file.to_string(),
    };
    // Fix helpers that demux the fix's TARGET node to its file, then apply against that file's own
    // source / arenas / spans / surface (a fix may land in an imported library, not the entry).
    let fix_is_ml = |fix_node: &str| -> bool {
        file_of_node(fix_node)
            .map(|(fi, _)| is_ml_source(&files[fi].path))
            .unwrap_or(false)
    };
    // The parsed `Tree` (`Tree::of`) of each file, built ONCE and shared across every fix that targets it.
    // Every fix rebuilds `new = old.transform(target)` from the SAME `old` = the file's whole tree; building
    // `old` per fix (`Tree::of` deep-copies the whole arena) made a file with N fixable diagnostics
    // O(N × tree) = O(N²). Cache it lazily per file (`Rc`, so the borrow is cheap to hand out) — the tree
    // materializes at most once per file regardless of how many fixes reference it.
    // Alongside the tree, cache its `origin → path` INDEX (`OriginPaths`) — built once per file so each
    // fix locates its target node in O(depth), not by an O(program) scan (which, per fix over N fixes, was
    // O(N²): `find_by_origin`+`Tree::origin` were ~82% of a wide-fixable-warnings check). Both the tree and
    // its index live for the whole `check` run, shared across every fix targeting the file.
    let tree_cache: std::cell::RefCell<Vec<Option<FileTree>>> =
        std::cell::RefCell::new(vec![None; files.len()]);
    let file_tree = |fi: usize| -> FileTree {
        if let Some(pair) = &tree_cache.borrow()[fi] {
            return pair.clone();
        }
        let t = std::rc::Rc::new(cadenza_syntax::query::Tree::of(&files[fi].arenas));
        let idx = std::rc::Rc::new(OriginPaths::of(&t));
        let pair = (t, idx);
        tree_cache.borrow_mut()[fi] = Some(pair.clone());
        pair
    };
    let do_fix_edits = |kind: &str,
                        fix_node: &str,
                        repl: &str|
     -> Option<Vec<cadenza_syntax::query::textedit::Edit>> {
        let (fi, local) = file_of_node(fix_node)?;
        let (tree, origins) = file_tree(fi);
        fix_edits(
            &files[fi].source,
            &tree,
            &origins,
            &files[fi].spans,
            kind,
            cadenza_syntax::StructId(local),
            repl,
            surface_of(&files[fi].path),
        )
    };
    let do_fix_apply = |kind: &str, fix_node: &str, repl: &str| -> Option<String> {
        let (fi, local) = file_of_node(fix_node)?;
        let (tree, _origins) = file_tree(fi);
        apply_fix_to_source(
            &files[fi].source,
            &tree,
            &files[fi].spans,
            kind,
            cadenza_syntax::StructId(local),
            repl,
            surface_of(&files[fi].path),
        )
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
        // Suppress the parse-recovery `<error>`-placeholder cascade — ANY fault whose subject is the
        // synthetic `<error>` node the ML reader left where a production failed. A failed production leaves
        // the placeholder in whatever position it was parsing, so it surfaces in DIFFERENT downstream checks
        // depending on where: an expression position → `unbound name `<error>`` (CDZ0101); a REPEATED
        // parameter position (a garbled param list recovers several `<error>` binders) → "parameter
        // `<error>` is bound more than once" (CDZ0102); an unused binder → the CDZ0306 warning. All name the
        // synthetic placeholder the user never wrote, layered on the real parse error. `<error>` is UNLEXABLE
        // on the ML surface (`<` starts no identifier), so a diagnostic naming it is ALWAYS the placeholder,
        // never a real name — drop it regardless of code. Gated on the file actually having had a parse
        // error, so a legitimate `<error>` symbol in a parse-clean s-expr file is still reported.
        if any_parse_error && message.contains("`<error>`") {
            continue;
        }
        any_error |= severity == "error";
        // A fix the compiler rendered in s-expr form may not be byte-splice-able into THIS file's
        // surface. `replace`/`delete`/`wrap` render on any surface (a bare name, a deletion, or the
        // `(ctor …)`→`ctor(…)` reshape). `insert` splices ARM/child scaffold whose syntax only exists
        // in-context (a handle/match arm) — an s-expr arm can't be lowered to ML by a fragment print —
        // so on a non-s-expr file we DROP the structured fix (the message still names the arm to add).
        // Treating it as "no fix node" makes both the human `help:` line and the JSON `fix` object omit
        // it uniformly. (`cdz fix` already declines it via the verify re-parse; this stops `check --json`
        // from handing an ML agent an unusable insert.)
        let fix_node = if fix_kind == "insert" && fix_is_ml(fix_node) {
            "-"
        } else {
            fix_node
        };

        // `--verify-fixes`: UPGRADE a heuristic fix to verified when applying it actually clears this
        // diagnostic. The compiler marks a fix verified only when a RULE proves it (D3's `_`-prefix);
        // a wrap/coercion/did-you-mean is heuristic there because the compiler does not recompile. Here
        // — where we hold BOTH the source text and the compiler — we CAN: splice the fix in, re-check,
        // and confirm the same-code error is gone (`spec/capabilities/diagnostics.md` §A Confirmed Fix
        // Is Marked Verified). Only heuristic fixes with a real target span are candidates. A PACKAGE
        // check leaves `baseline_errors` `None` (see above), so a package fix stays heuristic — verifying
        // it against a single re-parsed file would miss the cross-file context.
        let verified_flag = if fix_verified == "verified" {
            true
        } else if verify_fixes && !is_package && fix_node != "-" {
            let is_ml = is_ml_source(&files[0].path);
            do_fix_apply(fix_kind, fix_node, fix_repl)
                .map(|edited| {
                    fix_verifies(&edited, is_ml, severity, code, baseline_errors.as_deref())
                })
                .unwrap_or(false)
        } else {
            false
        };

        // Compute the structural patch for the fix ONCE — shared by BOTH output shapes. A fix carries a
        // `help:`/`fix` only when its node parses AND the patch actually builds (non-empty edits);
        // otherwise it is message-only guidance. This is the SINGLE source of truth for "does this
        // diagnostic have an applicable fix", so the human `help:` line and the JSON `fix` object AGREE:
        // previously the text path advertised a `help:` whenever `fix_node != "-"` (a raw column flag),
        // while JSON emitted `fix` only when `do_fix_edits` succeeded — so a fix whose `fix_repl` does not
        // parse (a malformed wrap payload) printed a phantom `help:` line the JSON/`cdz fix` path silently
        // dropped, misleading a human/agent reading the text output. Gating both on the built patch closes
        // that. The edits are relative to the fix's OWN file (which may be an imported library, not entry).
        let patch = if fix_node != "-" {
            do_fix_edits(fix_kind, fix_node, fix_repl).filter(|e| !e.is_empty())
        } else {
            None
        };

        if json {
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
            // In a PACKAGE check a diagnostic may belong to an imported file, so name it (the `from`/`to`
            // byte offsets below are relative to THAT file). A single-file check omits it — byte-identical
            // to before (the file is implicit, it is `args.file`).
            if is_package && let Some((fi, _)) = file_of_node(node) {
                obj.string("file", &files[fi].path);
            }
            if let Some((l, c, from, to)) = span_of(node) {
                obj.raw("line", &l.to_string());
                obj.raw("col", &c.to_string());
                obj.raw("from", &from.to_string());
                obj.raw("to", &to.to_string());
            }
            if let Some(edits) = patch {
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
        // A structural fix, if the diagnostic carries an APPLICABLE one — the rustc-style `help:` line an
        // agent (or an editor's quick-fix) applies directly. `replace` swaps the node's spelling; `insert`
        // appends the rendered form(s) into the node (e.g. the missing match arms). The applicability
        // marker rides along so a consumer branches (`verified` = apply blind, else confirm intent). Gated
        // on the SAME built `patch` the JSON path uses, so text and JSON never disagree on whether a fix
        // exists — a fix whose payload does not build is message-only on both.
        if patch.is_some() && fix_node != "-" {
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
    // A parse error is an error-severity fault in its own right (its recovered arena is not the author's
    // program), even when the truncated arena carries no downstream semantic fault. `check`'s contract is
    // "exits non-zero if any error-severity fault is present", and the parse error was already printed to
    // stderr from the load boundary — so fail here too, closing the "prints an error but exits 0" gap.
    // Hand the closure's file paths back so the project driver can mark them covered without reloading.
    let closure_paths = files.into_iter().map(|f| f.path).collect();
    (any_error || any_parse_error, closure_paths)
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
            // A fix is a candidate when it has a real TARGET node. A CODED fault is the common case; a
            // code-less DECLINE that nonetheless carries a targeted fix (a top-level keyword typo —
            // `(exprot …)` declines "unbound name … did you mean `export`?" with a replace fix) is ALSO
            // applicable, so gate on the fix node, not the code. `fix_verifies` (below, keyed on
            // severity+code+the cleared count) still proves the edit clears the fault before `--all`
            // applies it, so admitting a code-less fix cannot apply an unverified edit.
            if fix_node == "-" {
                continue;
            }
            considered_any = true;
            let Some(target) = fix_node.parse::<u32>().ok().map(cadenza_syntax::StructId) else {
                continue;
            };
            // Build the edited text structurally. This apply loop re-parses `current` each iteration (the
            // tree CHANGES as fixes accumulate), so build its `Tree` here — no cross-fix caching applies.
            let current_tree = cadenza_syntax::query::Tree::of(&current_arenas);
            let Some(edited) = apply_fix_to_source(
                &current,
                &current_tree,
                &spans,
                fix_kind,
                target,
                fix_repl,
                surface,
            ) else {
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
        if !args.diff
            && !args.dry_run
            && applied > 0
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

/// `cdz symbols FILE` — the document OUTLINE: every top-level declaration classified by kind, as
/// `file:line:col: kind name`. Rides the `Symbols` sidecar query, then maps each declaration's NAME node
/// to a source location through the span table. The superset companion of `cdz exports` — it lists EVERY
/// declaration (private ones too), not just the exported subset, so an editor can render a symbol tree.
fn run_symbols(args: &SymbolsArgs) -> ExitCode {
    let (source, arenas, spans) = match load_program_spanned(&args.file) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{PROG}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let out = run_sidecar(
        &arenas,
        rcdzc::Request::Query(rcdzc::sidecar::Query::Symbols),
    );
    let Some(bytes) = out.artifact(rcdzc::sidecar::KIND_SYMBOLS) else {
        report_errors(&out);
        return ExitCode::FAILURE;
    };
    let text = String::from_utf8_lossy(bytes);
    if text.trim().is_empty() {
        eprintln!("{PROG}: {} declares nothing", args.file);
        return ExitCode::SUCCESS;
    }
    // Each line is `name<TAB>kind<TAB>name-node-id`. One line-start index (binary-searched line:col) so a
    // wide declaration list stays linear (the same swap `exports`/`highlight` carry).
    let index = cadenza_syntax::query::driver::LineIndex::new(&source);
    for line in text.lines() {
        let mut cols = line.splitn(3, '\t');
        let (name, kind, node) = match (cols.next(), cols.next(), cols.next()) {
            (Some(n), Some(k), Some(d)) => (n, k, d),
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
        println!("{loc}: {kind} {name}");
    }
    ExitCode::SUCCESS
}

/// `cdz instantiations NAME FILE` — the DISPOSITION of definition NAME plus, if it is specialized, every
/// concrete monomorphization. Drives `Query::Instantiations`, which forces monomorphization over the whole
/// program then reads the disposition + instantiation records. Prints a disposition line
/// `file:line:col: NAME — DISPOSITION (gloss)` where DISPOSITION is `specialized` / `inlined` / `emitted`
/// / `unreferenced` (a `+`-joined set when more than one applies), then — for a specialized def — one
/// indented line per instance `file:line:col:   NAME[args…] → spec-name`. The location is the SOURCE
/// definition's name occurrence; an instance's args are its concrete per-parameter instantiation (a
/// runtime param `name: TYPE`, an erased compile-time param `const name = VALUE` — e.g. the concrete
/// dictionary an ad-hoc-polymorphic call baked in). An unknown name reports "no such definition".
fn run_instantiations(args: &InstantiationsArgs) -> ExitCode {
    let (source, arenas, spans) = match load_program_spanned(&args.file) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{PROG}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let out = run_sidecar(
        &arenas,
        rcdzc::Request::Query(rcdzc::sidecar::Query::Instantiations {
            name: args.name.clone(),
        }),
    );
    let Some(bytes) = out.artifact(rcdzc::sidecar::KIND_INSTANTIATIONS) else {
        report_errors(&out);
        return ExitCode::FAILURE;
    };
    let text = String::from_utf8_lossy(bytes);
    if text.trim().is_empty() {
        // Empty ONLY for an UNKNOWN name — a known def always emits a `disp` line. (A near-typo gets no
        // suggestion here; `cdz type NAME` is the name-oriented query that offers "did you mean?".) An
        // unresolvable name is a FAILURE (rc≠0) — consistent with `cdz type`/`cdz doc`, so a script can
        // tell a typo from a real result rather than reading a success exit on a "no such definition".
        eprintln!(
            "{PROG}: no such definition `{}` in {}",
            args.name, args.file
        );
        return ExitCode::FAILURE;
    }
    // Two line kinds, each TAB-tagged:
    //   `disp<TAB>node<TAB>disposition`               — the def's fate (specialized/inlined/emitted/…)
    //   `inst<TAB>spec<TAB>node<TAB>arg;arg;…`         — one per specialization (only when specialized)
    // Map the def's name node to a location; render each instance's `;`-joined args as `NAME[a, b, …]`.
    let index = cadenza_syntax::query::driver::LineIndex::new(&source);
    let loc_of = |node: &str| match node
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
    for line in text.lines() {
        let mut cols = line.split('\t');
        match cols.next() {
            Some("disp") => {
                let (node, disp) = match (cols.next(), cols.next()) {
                    (Some(n), Some(d)) => (n, d),
                    _ => continue,
                };
                // Present the disposition set readably, and gloss what it MEANS (why there is / isn't a
                // function to point at) so the status is self-explanatory. A `transformed→copy` tag or a
                // `+`-joined combination carries its own words, so those get no extra gloss.
                let gloss = match disp {
                    "inlined" => " (β-reduced into each call site; no standalone function emitted)",
                    "specialized" => {
                        " (monomorphized — one function per instantiation, listed below)"
                    }
                    "emitted" => " (emitted as a standalone function and called)",
                    "unreferenced" => " (never called, inlined, specialized, or exported)",
                    d if d.starts_with("transformed→") => {
                        " (its recursion was rewritten into the named accumulator loop)"
                    }
                    _ => "", // a `+`-joined combination — the words already say it
                };
                println!("{}: {} — {disp}{gloss}", loc_of(node), args.name);
            }
            Some("inst") => {
                let (spec, node, arglist) = match (cols.next(), cols.next(), cols.next()) {
                    (Some(s), Some(n), Some(a)) => (s, n, a),
                    _ => continue,
                };
                let pretty = arglist
                    .split(';')
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join(", ");
                println!("{}:   {}[{pretty}] → {spec}", loc_of(node), args.name);
            }
            _ => continue,
        }
    }
    ExitCode::SUCCESS
}

/// `cdz highlight FILE` — semantic syntax highlighting: every classified token as `file:line:col: kind`.
/// Rides the `Highlight` sidecar query (the same one the browser IDE's `semantic_tokens` calls), then
/// maps each node id to a source location through the span table. A token whose node has no span is
/// skipped (should not happen for a user leaf).
fn run_highlight(args: &HighlightArgs) -> ExitCode {
    let (source, arenas, spans) = match load_program_spanned(&args.file) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{PROG}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let out = run_sidecar(
        &arenas,
        rcdzc::Request::Query(rcdzc::sidecar::Query::Highlight),
    );
    let Some(bytes) = out.artifact(rcdzc::sidecar::KIND_HIGHLIGHT) else {
        report_errors(&out);
        return ExitCode::FAILURE;
    };
    let text = String::from_utf8_lossy(bytes);
    // ONE line-start index over the source, so each token's line:col is a binary search, not a from-start
    // newline scan — `highlight` emits a token for EVERY node (a whole-file classify), and the per-token
    // from-start `line_col` made it O(tokens × source_len) = O(N²) (a 6400-def file = 5.1s, 99.7% in
    // `line_col`). With the index it is linear. (The fixes-8-11 pattern — the same swap `uses`/`scope`
    // already carry.)
    let index = cadenza_syntax::query::driver::LineIndex::new(&source);
    // Each line is `node-id<TAB>kind`. Map the node to a `file:line:col`, skipping a span-less node.
    for line in text.lines() {
        let mut cols = line.splitn(2, '\t');
        let (node, kind) = match (cols.next(), cols.next()) {
            (Some(n), Some(k)) => (n, k),
            _ => continue,
        };
        if let Some(span) = node
            .parse::<u32>()
            .ok()
            .and_then(|d| spans.get(cadenza_syntax::StructId(d)))
        {
            let (l, c) = index.line_col(&source, span.start);
            println!("{}:{l}:{c}: {kind}", args.file);
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

/// Whether a program file is the ML surface (`.cdz`/`.ml`) vs s-expressions (`.sexp`/`.sexpr`) — the
/// one place the surface is inferred from the extension, shared by `load_program_spanned` and the
/// `check --verify-fixes` re-parse (which must re-read the edited text in the SAME surface).
fn is_ml_source(file: &str) -> bool {
    file.ends_with(".cdz") || file.ends_with(".ml")
}

// ── Project.cdz — the project manifest, written in Cadenza itself ──────────────────────────────────

/// A project manifest read from a `Project.cdz` — the Cadenza-authored description of a project's
/// layout, so `cdz` knows how to build / run / test it WITHOUT per-command flags. The manifest is
/// ordinary Cadenza: a set of well-known TOP-LEVEL DEFS binding constant values (a string, or a list of
/// strings), read straight from the arena — no compile, no new grammar (a def "is a really good way to
/// do this"). Every field is optional; a missing def leaves it `None`/empty.
///
/// Recognized defs (all relative to the manifest's own directory). A file list (`modules`/`tests`/
/// `exclude`) entry may be a literal name OR a GLOB (`*.cdz`, `sub/*.cdz`, `**/x.cdz`).
/// - `def name = "…"`            — the project name (display only).
/// - `def entry = "main.cdz"`     — the entry module `cdz build`/`run` compiles as the component root.
/// - `def modules = ["a.cdz", …]` — the library modules the package links (the entry's importables).
/// - `def tests = ["*.cdz", …]`   — the modules whose `@test` defs `cdz test` runs.
/// - `def exclude = ["x.cdz", …]` — files REMOVED from `modules`/`tests` after glob expansion (skip a
///   demo/fixture a wildcard would otherwise sweep up).
#[derive(Default, Debug)]
struct Manifest {
    name: Option<String>,
    entry: Option<String>,
    modules: Vec<String>,
    tests: Vec<String>,
    exclude: Vec<String>,
    /// The project's default optimization level for `cdz build` (`def opt-level = "O2"`), as the raw
    /// string — parsed via `rcdzc::OptLevel::FromStr` at use. A `--opt-level`/`--release` flag overrides
    /// it. `None` = no manifest default (the build falls back to `--release`'s `O2` or the default `O1`).
    opt_level: Option<String>,
}

/// The file name of a project manifest (looked up in a directory).
const MANIFEST_NAME: &str = "Project.cdz";

/// Read a value def's payload from a manifest arena at `value_id`: a bare STRING literal → one string;
/// a `["…", …]` list literal (arena head `"list"`) → each string element. Anything else → empty (a
/// non-string/list value is ignored rather than erroring — the manifest is advisory).
fn manifest_strings(
    arenas: &cadenza_syntax::Arenas,
    value_id: cadenza_syntax::StructId,
) -> Vec<String> {
    if let Some(s) = arenas.as_str(value_id) {
        return vec![s.to_string()];
    }
    // A list literal is the compound-ctor form `("list" elem…)` — a STRING head, so `as_ctor_form`.
    if let Some(elems) = arenas.as_ctor_form(value_id, "list") {
        return elems
            .iter()
            .filter_map(|&e| arenas.as_str(e))
            .map(str::to_string)
            .collect();
    }
    Vec::new()
}

/// Whether `pat` is a GLOB (contains a wildcard metacharacter) rather than a literal file name. A
/// literal entry in a manifest list is used verbatim; a glob is expanded against the manifest dir.
fn is_glob(pat: &str) -> bool {
    pat.contains('*') || pat.contains('?')
}

/// Match a single PATH SEGMENT (no `/`) against a glob segment supporting `*` (any run, incl. empty)
/// and `?` (exactly one char). A backtracking matcher over char slices — segments are short (file
/// names), so the worst-case backtracking is irrelevant. `**` is handled by the caller (it spans
/// segments), never reaching here.
fn glob_segment_matches(pat: &[char], name: &[char]) -> bool {
    match pat.split_first() {
        None => name.is_empty(),
        Some((&'*', rest)) => {
            // `*` matches zero or more chars: try consuming 0, 1, … of `name`.
            (0..=name.len()).any(|i| glob_segment_matches(rest, &name[i..]))
        }
        Some((&'?', rest)) => !name.is_empty() && glob_segment_matches(rest, &name[1..]),
        Some((&c, rest)) => {
            matches!(name.split_first(), Some((&n, nrest)) if n == c && glob_segment_matches(rest, nrest))
        }
    }
}

/// Whether the RELATIVE path `rel` (forward-slash segments, relative to the manifest dir) matches the
/// glob `pat`. `**` matches any number of path segments (incl. zero — so `**/x.cdz` matches `x.cdz` and
/// `a/b/x.cdz`); every other segment matches by [`glob_segment_matches`]. A trailing `**` matches
/// everything below. Segment counts must line up otherwise.
fn glob_path_matches(pat: &str, rel: &str) -> bool {
    fn go(pat: &[&str], seg: &[&str]) -> bool {
        match pat.split_first() {
            None => seg.is_empty(),
            Some((&"**", prest)) => {
                // `**` consumes zero or more path segments.
                (0..=seg.len()).any(|i| go(prest, &seg[i..]))
            }
            Some((&p, prest)) => match seg.split_first() {
                Some((&s, srest)) => {
                    glob_segment_matches(
                        &p.chars().collect::<Vec<_>>(),
                        &s.chars().collect::<Vec<_>>(),
                    ) && go(prest, srest)
                }
                None => false,
            },
        }
    }
    let ps: Vec<&str> = pat.split('/').filter(|s| !s.is_empty()).collect();
    let ss: Vec<&str> = rel.split('/').filter(|s| !s.is_empty()).collect();
    go(&ps, &ss)
}

/// Expand a manifest file-list against `dir`: a LITERAL entry (no wildcard) is kept as-is (resolved to
/// `dir/entry`); a GLOB entry (`*.cdz`, `sub/*.cdz`, `**/x.cdz`) is expanded to every SOURCE file under
/// `dir` whose relative path matches, path-SORTED. The manifest itself (`Project.cdz`) is never matched
/// by a glob (it is the manifest, not a member). Any file whose relative path matches an `exclude`
/// pattern (literal or glob) is then REMOVED — so `tests = ["*.cdz"]` + `exclude = ["demo.cdz"]` skips
/// the demo. Results are de-duplicated in first-appearance order. Returns paths (`dir/rel`) ready to
/// hand to the per-file runner/compiler.
fn expand_manifest_globs(
    dir: &std::path::Path,
    patterns: &[String],
    exclude: &[String],
) -> Vec<String> {
    // Collect the directory's source files ONCE (relative forward-slash paths), only if a glob is present.
    let has_glob = patterns.iter().any(|p| is_glob(p));
    let mut rels: Vec<String> = Vec::new();
    if has_glob {
        let mut abs = Vec::new();
        let _ = collect_source_dir(dir, &mut abs); // best-effort; a bad dir yields no matches
        for a in abs {
            if let Ok(r) = std::path::Path::new(&a).strip_prefix(dir) {
                let rel = r.to_string_lossy().replace('\\', "/");
                if rel != MANIFEST_NAME {
                    rels.push(rel);
                }
            }
        }
        rels.sort();
    }
    // A file's relative path is excluded if it matches any `exclude` pattern (literal == or glob match).
    let is_excluded = |rel: &str| -> bool {
        exclude.iter().any(|e| {
            if is_glob(e) {
                glob_path_matches(e, rel)
            } else {
                e == rel
            }
        })
    };
    let mut out: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut push = |rel: String, out: &mut Vec<String>| {
        if is_excluded(&rel) {
            return;
        }
        let full = dir.join(&rel).to_string_lossy().into_owned();
        if seen.insert(full.clone()) {
            out.push(full);
        }
    };
    for pat in patterns {
        if is_glob(pat) {
            for rel in rels.iter().filter(|r| glob_path_matches(pat, r)) {
                push(rel.clone(), &mut out);
            }
        } else {
            push(pat.clone(), &mut out);
        }
    }
    out
}

/// Peel a leading `(comment TEXT FORM)` / `(doc TEXT FORM)` wrapper to the FORM it decorates — the
/// reader attaches a `//` line comment or `///` doc as such a wrapper around the following top-level
/// form, so a commented `(def …)` is nested inside it. Returns `id` unchanged when it is not a
/// comment/doc wrapper. Handles a stacked wrapper (`(comment … (doc … (def …)))`) by iterating.
fn unwrap_comment(
    arenas: &cadenza_syntax::Arenas,
    id: cadenza_syntax::StructId,
) -> cadenza_syntax::StructId {
    let mut cur = id;
    loop {
        // `(comment TEXT FORM)` / `(doc TEXT FORM)` — the decorated form is the LAST child.
        let inner = arenas
            .as_form(cur, "comment")
            .or_else(|| arenas.as_form(cur, "doc"))
            .and_then(|tail| tail.last().copied());
        match inner {
            Some(next) if next != cur => cur = next,
            _ => return cur,
        }
    }
}

/// Parse a `Project.cdz`'s arena into a [`Manifest`] by reading its well-known top-level `(def NAME
/// VALUE)` forms. Mirrors [`declared_import_paths`]'s arena walk: a `(do …)` root's children (or a lone
/// root form) are the top-level items; each `(def name value)` whose name matches a known field fills
/// it. Unknown defs are ignored (forward-compatible — a newer manifest field is a no-op to an older
/// `cdz`), so the manifest never fails to parse on an unrecognized key.
fn parse_manifest(arenas: &cadenza_syntax::Arenas) -> Manifest {
    let root = arenas.root;
    let items: Vec<cadenza_syntax::StructId> = match arenas.as_form(root, "do") {
        Some(tail) => tail.to_vec(),
        None => vec![root],
    };
    let mut m = Manifest::default();
    for item in items {
        // Unwrap a leading `(comment TEXT FORM)` / `(doc TEXT FORM)` wrapper: the reader attaches a `//`
        // line comment or `///` doc as `(comment "…" <the form>)`, so a commented `def` is nested one
        // level in. Peel it to reach the real `(def …)` (manifests are naturally commented).
        let item = unwrap_comment(arenas, item);
        // A `(def NAME VALUE)` — the name is the first child, the value the second.
        let Some(tail) = arenas.as_form(item, "def") else {
            continue;
        };
        let (Some(&name_id), Some(&value_id)) = (tail.first(), tail.get(1)) else {
            continue;
        };
        let Some(name) = arenas.as_name(name_id) else {
            continue;
        };
        match name {
            "name" => m.name = manifest_strings(arenas, value_id).into_iter().next(),
            "entry" => m.entry = manifest_strings(arenas, value_id).into_iter().next(),
            "modules" => m.modules = manifest_strings(arenas, value_id),
            "tests" => m.tests = manifest_strings(arenas, value_id),
            "exclude" => m.exclude = manifest_strings(arenas, value_id),
            "opt-level" => m.opt_level = manifest_strings(arenas, value_id).into_iter().next(),
            _ => {} // an unrecognized def — ignore (forward-compatible)
        }
    }
    m
}

/// Load + parse the `Project.cdz` manifest at `dir/Project.cdz`, if present. `Ok(None)` when there is no
/// manifest there (the caller falls back to its non-manifest behavior); `Err` when a manifest EXISTS but
/// fails to parse (a genuine authoring error worth surfacing).
///
/// A MALFORMED manifest is a hard error for EVERY project command, not a silent empty manifest. The ML
/// reader RECOVERS from a parse error (it prints each error, then hands back a truncated `<error>`-node
/// arena), so `parse_manifest` over that recovered arena would yield an ALL-DEFAULT `Manifest` — making a
/// broken `Project.cdz` look like a valid-but-empty one. `cdz build`/`check` happened to fail later (no
/// `entry`), but `cdz test`/`metadata`/`clean` proceeded with rc=0 as if the manifest were empty. Use the
/// COUNTED loader and reject a manifest with any recovered parse error, so all commands fail uniformly.
fn load_manifest(dir: &std::path::Path) -> Result<Option<(std::path::PathBuf, Manifest)>, String> {
    let path = dir.join(MANIFEST_NAME);
    if !path.is_file() {
        return Ok(None);
    }
    let spec = path.to_string_lossy().into_owned();
    let (_source, arenas, _spans, parse_errors) = load_program_spanned_counted(&spec)?;
    if parse_errors > 0 {
        // The specific `file:line:col: error: …` lines were already printed by the loader; give a summary
        // so the failure is unambiguous (and non-zero exit) rather than a silently-empty manifest.
        return Err(format!(
            "{}: the manifest does not parse ({parse_errors} error{})",
            path.display(),
            if parse_errors == 1 { "" } else { "s" }
        ));
    }
    Ok(Some((path, parse_manifest(&arenas))))
}

/// Search UP from the current working directory for the nearest `Project.cdz` — the current dir, then
/// each ancestor, stopping at the first that holds one (like `cargo` locating `Cargo.toml`). Returns the
/// manifest path, or `None` if no ancestor has one. Used when `cdz test` is invoked with NO argument.
fn find_manifest_upward() -> Option<PathBuf> {
    let mut dir = std::env::current_dir().ok()?;
    loop {
        let candidate = dir.join(MANIFEST_NAME);
        if candidate.is_file() {
            return Some(candidate);
        }
        if !dir.pop() {
            return None; // reached the filesystem root with no manifest
        }
    }
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
    load_program_spanned_counted(file).map(|(s, a, sp, _)| (s, a, sp))
}

/// [`load_program_spanned`] plus the COUNT of recovered parse errors. The ML reader RECOVERS from a
/// syntax error (it prints each, then returns a truncated-but-well-formed arena of `<error>` placeholder
/// nodes rather than aborting), so a caller that loads the file and proceeds — `cdz check` — would
/// otherwise never learn the parse failed: it exits by whether the SEMANTIC fault set is empty, and a
/// clean truncation (unclosed `(`, an arm-less `match`) leaves NO downstream semantic fault, so the check
/// reported SUCCESS on a file that does not parse. Return the count so `run_check` can force a nonzero
/// exit (a parse error IS an error-severity fault: its `<error>` placeholders are not the program the
/// author wrote) and suppress the placeholder cascade. `0` on the s-expr surface (its reader hard-`Err`s
/// on a malformed program, handled below — reaching `Ok` there means it parsed).
fn load_program_spanned_counted(
    file: &str,
) -> Result<
    (
        String,
        cadenza_syntax::Arenas,
        cadenza_syntax::spans::SpanTable,
        usize,
    ),
    String,
> {
    let source = std::fs::read_to_string(file).map_err(|e| format!("reading {file}: {e}"))?;
    parse_program_spanned_counted(file, source)
}

/// Parse an already-read program `source` (whose surface is inferred from `file`'s extension) into its
/// arenas + span table + recovered-parse-error count — the pure-parse core of
/// [`load_program_spanned_counted`], split out so a caller holding the source by other means (an editor's
/// in-memory buffer, via `cdz lsp`'s overlay) parses it WITHOUT a disk read. `file` names the surface and
/// prefixes diagnostics; `source` is its text. Behaviour is byte-identical to the disk path (the disk
/// wrapper is just `read_to_string` + this).
pub(crate) fn parse_program_spanned_counted(
    file: &str,
    source: String,
) -> Result<
    (
        String,
        cadenza_syntax::Arenas,
        cadenza_syntax::spans::SpanTable,
        usize,
    ),
    String,
> {
    // An EMPTY (or whitespace-only) source has NO top-level form — an "empty program" error on BOTH
    // surfaces (exits nonzero, `file:1:1: error: empty program`). Checked BEFORE the surface split so
    // both agree: the s-expr `read_all_spanned` fallback would otherwise build a rootless synthetic
    // `(do)` that silently checks clean (`cdz check empty.sexp` exiting 0), and the ML `read_ml`
    // surfaces "empty program" as a printed parse error but then RETURNS OK and proceeds over the empty
    // arena, so `cdz check empty.cdz` ALSO exited 0 despite printing the error. One early `Err` fixes
    // both. (The common "I made the file but haven't written anything" mistake; a comment-only file is
    // a rarer edge left to each reader's own path.)
    if source.trim().is_empty() {
        return Err(format!("{file}:1:1: error: empty program"));
    }
    let is_ml = is_ml_source(file);
    if is_ml {
        let parsed = cadenza_syntax::parser::read_ml(&source);
        // Render each recovered parse error in the SAME `file:line:col: error: message` shape as the
        // semantic diagnostics — not the raw `ParseError { span: Span { … }, message: "…" }` Debug dump,
        // and not mislabeled a "warning" (a parse error leaves `<error>` placeholder nodes that will
        // cascade into spurious downstream faults, so it is a genuine error the user must fix first). The
        // parser RECOVERS (it never aborts), so several may print; the compile still proceeds over the
        // recovered arena, exactly as before — only the wording changes.
        // ONE line-start index over the source, so each error's line:col is a binary search, not a
        // from-start newline scan — a broken ML file recovers N cascading errors (a mid-edit unmatched
        // token yields ~5 per line), and the per-error from-start `line_col` made rendering them
        // O(errors × source_len) = O(N²) (2003 errors over a 3200-line file = 549ms, ~97% in `line_col`).
        let index = cadenza_syntax::query::driver::LineIndex::new(&source);
        for e in &parsed.errors {
            let (line, col) = index.line_col(&source, e.span.start);
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
        Ok((source, arenas, spans, parsed.errors.len()))
    } else {
        // (Empty/whitespace-only source is handled uniformly before the surface split above.)
        // Mirror the driver's root convention (`query::driver::load`): a SINGLE top-level form stays
        // BARE (so a lone `(module …)`/`(def …)` is the root the compiler scans), and only MULTIPLE
        // forms wrap in a synthetic `(do …)`. `read_spanned` succeeds iff there's exactly one form
        // (it errors on trailing input); fall back to `read_all_spanned` for several. Using
        // `read_all_spanned` unconditionally would wrap a lone `(module …)` in `(do …)`, and the
        // compiler's top-level scan would then see the module as one opaque item and find no defs.
        let (raw_arenas, raw_spans) = match cadenza_syntax::sexpr::read_spanned(&source) {
            Ok(pair) => pair,
            // `read_spanned` errors on trailing input (a multi-form file), so fall back to
            // `read_all_spanned`. Its error's trailing `at byte N` is mapped to `at line:col` so a
            // multi-line s-expr parse error points at a navigable place (matching the ML/`check`
            // rendering), not a raw byte offset.
            Err(_) => cadenza_syntax::sexpr::read_all_spanned(&source).map_err(|e| {
                format!(
                    "{file}: {}",
                    cadenza_syntax::convert::locate_byte_in_message(&e.0, &source)
                )
            })?,
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
        // The s-expr reader has no partial-recovery mode — it reaches here only on a fully-parsed program
        // (a malformed one took the `Err` above), so there are no recovered parse errors to count.
        Ok((source, arenas, spans, 0))
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
        _ => {
            // CANONICALIZE + REMAP the span table, mirroring the ML branch (and
            // `parse_program_spanned_counted`): the `Diagnostics` query answers with CANONICAL node ids
            // (the arena is `codec::encode`d, which canonicalizes), so the span table `cdz fix` indexes a
            // fix_node into must be keyed by canonical ids too. A LONE form is built canonically already
            // (identity map — the previously-correct single-form case is unchanged), but the MULTI-form
            // `read_all_spanned` fallback wraps the roots in a synthetic `(do …)` whose head is built LAST;
            // canonicalization reorders the ids, so an un-remapped table maps the canonical fix_node to a
            // NEIGHBOUR's span — landing the edit on the wrong node (rewriting a param's TYPE, or the whole
            // param list, DESTROYING the function). Remap keys the table by canonical ids on both surfaces.
            let (raw_arenas, raw_spans) = match cadenza_syntax::sexpr::read_spanned(text) {
                Ok(pair) => pair,
                Err(_) => cadenza_syntax::sexpr::read_all_spanned(text).ok()?,
            };
            let (arenas, id_map) = cadenza_syntax::canon::canonicalize_with_map(&raw_arenas);
            Some(raw_spans.remap(&id_map, arenas.structure.len()))
        }
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
        _ => {
            // CANONICALIZE, matching the ML branch + `reparse_spans`'s s-expr arm: the next `Diagnostics`
            // pass and the span table must agree on canonical node ids (a multi-form program's synthetic
            // `(do …)` reorders ids under canonicalization). Reading raw here and canonicalizing keeps the
            // arena consistent with the remapped span table. A lone form's map is identity (no-op).
            let raw_arenas = match cadenza_syntax::sexpr::read_spanned(text) {
                Ok((arenas, _)) => arenas,
                Err(_) => cadenza_syntax::sexpr::read_all(text).ok()?,
            };
            Some(cadenza_syntax::canon::canonicalize_with_map(&raw_arenas).0)
        }
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
    // Fix-engine internals the perf-regression tests drive directly (now in the `fix` module).
    use crate::fix::{TRANSFORM_SIBLING_CLONES, localized_change, transform_target};

    #[test]
    fn localized_change_diffs_only_the_target_subtree_not_the_whole_program() {
        // REGRESSION (perf): `cdz check` computes each fixable diagnostic's byte-edits via
        // `fix_edits`, which previously built the WHOLE new tree (`transform_target`, deep-cloning every
        // untouched sibling) and diffed it against the whole old tree — O(program) PER fix. A file with N
        // fixable warnings (a WIDE match with N unused-binder arms) was thus O(N²) (a 800-arm match: 372ms).
        // FIX: `localized_change` returns only the CHANGED subtree pair `(target_node, replacement)` — a
        // fix touches one node, and `edits_preserving` emits edits only within the changed span, so diffing
        // the local pair is byte-identical to diffing the whole tree, at O(target-subtree) not O(program).
        //
        // Lock it in structurally: for a `replace` fix on a target deep in a WIDE tree, `old_sub` is the
        // target's OWN small subtree — its node count is independent of the surrounding program width (a
        // revert to diffing the whole tree would make it the program size). Build `(root c0 c1 … target)`
        // with N wide sibling subtrees + a shallow `target` atom, and assert the diffed subtree is tiny.
        use cadenza_syntax::query::Tree;
        use cadenza_syntax::{StructId, ast::Leaf};
        fn count(t: &Tree) -> usize {
            match t {
                Tree::Atom(..) => 1,
                Tree::List(items, _) => 1 + items.iter().map(count).sum::<usize>(),
            }
        }
        fn sibling(width: usize, next: &mut u32) -> Tree {
            // A `(f a0 a1 … a{width})` list — a wide sibling subtree that must NOT be diffed.
            let mut kids = Vec::new();
            for _ in 0..width {
                let id = *next;
                *next += 1;
                kids.push(Tree::Atom(Leaf::Name("a".to_string()), Some(StructId(id))));
            }
            Tree::List(kids, Some(StructId(*next)))
        }
        let build = |n_sibs: usize, sib_width: usize| -> (Tree, StructId) {
            let mut next = 1000u32;
            let mut kids = Vec::new();
            for _ in 0..n_sibs {
                kids.push(sibling(sib_width, &mut next));
                next += 1;
            }
            let target = StructId(1);
            kids.push(Tree::Atom(Leaf::Name("y".to_string()), Some(target)));
            (Tree::List(kids, Some(StructId(0))), target)
        };
        // A `replace y → _y` fix. `old_sub` must be just the `y` atom (1 node), regardless of the N wide
        // siblings around it.
        let (small, tgt_s) = build(4, 4);
        let (big, tgt_b) = build(400, 8);
        let idx_small = OriginPaths::of(&small);
        let idx_big = OriginPaths::of(&big);
        let (os_small, _) =
            localized_change(&small, &idx_small, "replace", tgt_s, "_y").expect("found");
        let (os_big, _) = localized_change(&big, &idx_big, "replace", tgt_b, "_y").expect("found");
        assert_eq!(
            count(os_small),
            1,
            "the target atom is the whole diffed subtree"
        );
        assert_eq!(
            count(os_small),
            count(os_big),
            "the diffed subtree is the TARGET's ({} nodes), independent of program width — a revert to \
             diffing the whole tree would scale with the {}-sibling program",
            count(os_small),
            400
        );
    }

    #[test]
    fn origin_paths_index_locates_every_node_and_its_parent() {
        // REGRESSION (perf): `cdz check` located each fix's target by an O(program) origin SCAN
        // (`find_by_origin`), run PER fixable diagnostic → O(N × program) = O(N²) on a file with many
        // fixable warnings (`find_by_origin`+`Tree::origin` were ~82% of a wide-fixable-warnings check).
        // FIX: `OriginPaths::of` builds an `origin → path-from-root` index in ONE walk (shared across all a
        // file's fixes), so `node`/`parent` locate in O(depth) by following the path — not by scanning.
        //
        // Lock in the index's CORRECTNESS (a wrong path silently mis-fixes): for EVERY origin-bearing node
        // in a mixed tree, `OriginPaths::node` returns exactly that node (same origin), and `parent` returns
        // the list that directly holds it. A regression to a scan-free-but-wrong path would fail here.
        use cadenza_syntax::query::Tree;
        use cadenza_syntax::{StructId, ast::Leaf};
        // `(root (a b) c (d (e f)))` with distinct origins — a mix of depths and sibling positions.
        let leaf = |id: u32, n: &str| Tree::Atom(Leaf::Name(n.to_string()), Some(StructId(id)));
        let tree = Tree::List(
            vec![
                Tree::List(vec![leaf(2, "a"), leaf(3, "b")], Some(StructId(1))),
                leaf(4, "c"),
                Tree::List(vec![leaf(6, "e"), leaf(7, "f")], Some(StructId(5))),
            ],
            Some(StructId(0)),
        );
        let idx = OriginPaths::of(&tree);
        // Every origin resolves to the SAME node.
        for id in [0u32, 1, 2, 3, 4, 5, 6, 7] {
            let sid = StructId(id);
            let n = idx.node(&tree, sid).expect("origin is indexed");
            assert_eq!(n.origin(), Some(sid), "node({id}) has origin {id}");
        }
        // A missing origin → None (no panic, no spurious hit).
        assert!(
            idx.node(&tree, StructId(99)).is_none(),
            "absent origin → None"
        );
        // The root has no parent; a nested node's parent is the list directly holding it.
        assert!(
            idx.parent(&tree, StructId(0)).is_none(),
            "the root has no parent"
        );
        assert_eq!(
            idx.parent(&tree, StructId(2)).and_then(|p| p.origin()),
            Some(StructId(1)),
            "`a`'s parent is the `(a b)` list"
        );
        assert_eq!(
            idx.parent(&tree, StructId(6)).and_then(|p| p.origin()),
            Some(StructId(5)),
            "`e`'s parent is the `(e f)` list"
        );
        assert_eq!(
            idx.parent(&tree, StructId(4)).and_then(|p| p.origin()),
            Some(StructId(0)),
            "`c`'s parent is the root"
        );
    }

    #[test]
    fn transform_target_does_not_clone_untouched_subtrees() {
        // REGRESSION (perf): `transform_target` (used by `cdz check`/`fix` to rebuild the parsed tree with
        // one node replaced — run PER fixable diagnostic) `out.push(child.clone())`-ed EVERY child of every
        // visited list before checking whether that list even contained the target, discarding the `out` on
        // a miss. So computing a fix beside a deep sibling deep-cloned that sibling's whole subtree at each
        // level → O(depth²) per fix; a file with N fixable warnings → O(N³) (a 400-deep-tuple match with
        // 400 unused binders: 7.3s). FIX: find the ONE hit child first (a miss clones nothing), then clone
        // only the SIBLINGS of the hit path.
        //
        // Lock it in via `TRANSFORM_SIBLING_CLONES`: transforming a target that sits at the SHALLOW end of a
        // spine, beside a DEEP untouched subtree, must clone O(spine-siblings) nodes — NOT the deep subtree.
        // Build `(root (deep …) target)`: a deeply-nested left child + a shallow `target` sibling at the
        // root. The transform touches `target`; the deep child is an untouched sibling cloned exactly ONCE
        // (one node handle — a `Tree` clone is a deep copy, but we count sibling-clone OPERATIONS along the
        // spine, which must stay constant regardless of the deep child's DEPTH).
        use cadenza_syntax::query::Tree;
        use cadenza_syntax::{StructId, ast::Leaf};
        // A left child nested `depth` deep; the target is a shallow atom sibling at the root (origin 1).
        fn deep(depth: usize, next_id: &mut u32) -> Tree {
            let id = *next_id;
            *next_id += 1;
            if depth == 0 {
                Tree::Atom(Leaf::Name("leaf".to_string()), Some(StructId(id)))
            } else {
                Tree::List(vec![deep(depth - 1, next_id)], Some(StructId(id)))
            }
        }
        let build = |depth: usize| -> (Tree, StructId) {
            let mut next = 100u32;
            let child = deep(depth, &mut next);
            let target = StructId(1);
            let tree = Tree::List(
                vec![
                    child,
                    Tree::Atom(Leaf::Name("target".to_string()), Some(target)),
                ],
                Some(StructId(0)),
            );
            (tree, target)
        };
        fn clones_for(tree: &Tree, target: StructId) -> u64 {
            TRANSFORM_SIBLING_CLONES.with(|c| c.set(0));
            let mut f = |_n: &Tree| -> Option<Tree> {
                Some(Tree::Atom(Leaf::Name("_t".to_string()), None))
            };
            let out = transform_target(tree, target, &mut f);
            assert!(out.is_some(), "the target must be found and transformed");
            TRANSFORM_SIBLING_CLONES.with(|c| c.get())
        }
        // The deep sibling grows 8× (depth 50 → 400); the sibling-clone COUNT must stay CONSTANT (the root
        // has one sibling to clone regardless of its depth). A per-level clone-all would grow ~with depth.
        let (t50, tgt50) = build(50);
        let (t400, tgt400) = build(400);
        let c50 = clones_for(&t50, tgt50);
        let c400 = clones_for(&t400, tgt400);
        assert_eq!(
            c50, c400,
            "transform_target must clone only the hit path's SIBLINGS ({c50}), not the untouched deep \
             subtree — a miss clones nothing (the O(depth²)-per-fix regression: {c400} at depth 400)"
        );
        assert!(
            c400 <= 4,
            "one shallow sibling at the root → a tiny constant clone count, got {c400}"
        );
    }

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

    #[test]
    fn an_empty_program_is_an_error_on_both_surfaces() {
        // An empty (or whitespace-only) file has no top-level form → an "empty program" error on BOTH
        // the s-expr AND ML surfaces (so `cdz check` exits nonzero). The s-expr `read_all_spanned`
        // fallback once built a rootless `(do)` that checked clean (exit 0); the ML `read_ml` printed
        // "empty program" but RETURNED OK and proceeded (also exit 0). A unified pre-split check errors
        // both. A valid single-form file on each surface still loads.
        let dir = tmp("emptyprog");
        // Empty AND whitespace-only, on BOTH surfaces, all error "empty program".
        for (name, body) in [
            ("e.sexp", ""),
            ("ws.sexp", "   \n  "),
            ("e.cdz", ""),
            ("ws.cdz", "\n\n"),
        ] {
            let f = dir.join(name);
            std::fs::write(&f, body).unwrap();
            let err = load_program_spanned(&f.to_string_lossy())
                .expect_err("an empty program must error");
            assert!(err.contains("empty program"), "got {err} for {name}");
        }
        // A valid single-form file on each surface still loads.
        let sok = dir.join("v.sexp");
        std::fs::write(&sok, "(module m (def (a) 1) (export a))").unwrap();
        assert!(
            load_program_spanned(&sok.to_string_lossy()).is_ok(),
            "valid s-expr loads"
        );
        let mok = dir.join("v.cdz");
        std::fs::write(&mok, "let m = fn() => 1 in m").unwrap();
        assert!(
            load_program_spanned(&mok.to_string_lossy()).is_ok(),
            "valid ML loads"
        );
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
