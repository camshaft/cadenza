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

use clap::{CommandFactory, Parser, Subcommand};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use cadenza_syntax::cli as syntax_cli;
// The in-process compiler CLI — used ONLY on the `standalone` dispatch branches (`run_with_specs` /
// `run_prepared_with_overflow`); a `!standalone` build delegates + never links `rcdzc`, so gate the import.
#[cfg(feature = "standalone")]
use rcdzc::cli as compiler_cli;

// The LSP server (`cdz lsp`) — its own module so `main.rs` gains only the `Cmd::Lsp` arm + dispatch,
// keeping the server implementation (owned by the v-lsp vertical) out of the shared command file. Behind
// the default-on `lsp` feature (sole user of lsp-server/lsp-types) so a `--no-default-features` seedCompiler
// build sheds them.
#[cfg(feature = "lsp")]
mod lsp;

// The structural fix-application engine, shared by `cdz fix` / `cdz check --json` / `cdz lsp` codeAction.
mod fix;
use fix::{FileTree, OriginPaths, apply_fix_to_source, fix_edits};

// The import-closure loader, shared by `cdz check`/… and `cdz lsp` (cross-file analysis).
mod closure;
use closure::{declared_import_paths, load as load_import_closure_with};

// cadenza-docs I2 (assembly half): the `cdz doc-module` merge — parse rcdzc's `export-types` sidecar
// blob + graft resolved `(ty …)` into the structural doc-module from `cadenza_syntax::doc_item::project`.
// Symbol-independent of the sidecar Query (the handler drives `run_sidecar` and hands the blob here).
mod doc_module;

// Delegated compilation — spawn the standalone `cdz-compile` instead of linking `rcdzc` in-process
// (`design/DESIGN-cdz-delegate-compile.md`). Compiled when the `standalone` feature is OFF (the nix
// build's `--no-default-features` packaging); the default (`standalone` ON) bundles the compiler
// in-process. `cdz compile`/`cdz build` route through [`dispatch_compile_args`] /
// [`dispatch_compile_prepared`], which pick delegation vs in-process at compile time.
// `cdz compile`'s cdz-LOCAL arg struct — the front-end parses it in BOTH builds (standalone dispatches
// it in-process via `run_with_specs`; !standalone delegates it), so it is UNCONDITIONAL, not gated.
mod compile_args;
#[cfg(not(feature = "standalone"))]
mod delegate;

/// The unified tool. The name reported in tool-level diagnostics is `cdz`.
const PROG: &str = "cdz";

/// Load a program with its span table via [`load_program_spanned`], or PRINT the error to stderr and
/// `return ExitCode::FAILURE` from the enclosing command handler. Expands to the load result tuple
/// `(source, arenas, span_table)` on success — the shared load-or-bail preamble every span-mapped query
/// handler (`type`, `uses`, `def`, `scope`, `exports`, `symbols`, …) opens with. Only usable in a fn that
/// returns `ExitCode` (the early return is `ExitCode::FAILURE`); a handler returning something else keeps
/// its own `match`.
macro_rules! load_spanned_or_bail {
    ($file:expr) => {
        match load_program_spanned($file) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("{PROG}: {e}");
                return ExitCode::FAILURE;
            }
        }
    };
}

/// An RAII guard that best-effort removes a temp path when dropped — used by the driver-scaffolding
/// paths (`run_ml`, `chor`, the sandboxed-build queries) that write pid-stamped temp files/dirs and must
/// clean them on EVERY exit path (success, error, or panic-unwind). `RemoveOnDrop::file` removes a file,
/// `RemoveOnDrop::dir` removes a directory tree; removal errors are ignored (the path may already be gone).
struct RemoveOnDrop {
    path: std::path::PathBuf,
    is_dir: bool,
}

impl RemoveOnDrop {
    fn file(path: std::path::PathBuf) -> Self {
        Self {
            path,
            is_dir: false,
        }
    }

    fn dir(path: std::path::PathBuf) -> Self {
        Self { path, is_dir: true }
    }
}

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        let _ = if self.is_dir {
            std::fs::remove_dir_all(&self.path)
        } else {
            std::fs::remove_file(&self.path)
        };
    }
}

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

// `large_enum_variant` (Run's `RunArgs` ~424 bytes vs the ~176-byte next): INTENTIONAL, not worth boxing.
// `Cmd` is a SHORT-LIVED CLI DISPATCH value — clap parses it ONCE at startup, `main` matches it ONCE, and it
// is dropped; it is never stored in bulk, cloned in a loop, or on a hot path, so the inter-variant size gap
// costs nothing. Boxing the large variant (`Run(Box<RunArgs>)`) would ALSO fight the clap `Subcommand` derive
// (it flattens the variant's `Args` fields; `Box<RunArgs>` is not `Args`), forcing a manual parse. So suppress.
#[allow(clippy::large_enum_variant)]
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
    /// Apply a canonicalizing normalization codemod (opt-in, distinct from `fmt`): currently the
    /// single-clause-irrefutable-`match`→`let` rewrite (`--match-to-let`).
    Normalize(syntax_cli::NormalizeArgs),

    // ── compiler (rcdzc) ────────────────────────────────────────────────────────────────────────
    /// Compile binary-AST artifacts to one or more backend targets (wasm/rust). The `rcdzc` surface.
    Compile(compile_args::CompileArgs),

    // ── run (cdz-run) ───────────────────────────────────────────────────────────────────────────
    /// Run a finished wasm component: link it (resolving its value-heap runtime by content address from
    /// the store), call an export (the sole function export by default), and print the rendered result.
    /// A trap or error goes to stderr with a non-zero exit. Folded in from the `cdz-run` bin so a single
    /// `cdz` on the PATH both compiles and runs (`cdz compile foo.cdz -o - | cdz run -`). A run is capped
    /// at a wall-clock deadline (default 30s) so a runaway loop TRAPS rather than hanging — set
    /// `CDZ_RUN_TIMEOUT_SECS=<n>` to change it, `=0` to disable (e.g. under a debugger).
    Run(cdz_run::cli::RunArgs),

    // ── corpus (cdz-corpus) ─────────────────────────────────────────────────────────────────────
    /// Read + migrate the executable-semantics corpus (`records`/`migrate`/`check`) — the maintenance
    /// tool for `spec/semantics/*.sexp`. Folded in from the `cdz-corpus` bin so it needn't be a separate
    /// binary on the PATH. `cdz corpus records FILE…` emits the flat record stream the gate consumes;
    /// `migrate` projects a `.sexp` corpus to literate markdown; `check` proves a migration is
    /// behaviour-preserving. Behind the `corpus` feature (default-on): the compiler-ml spine-gate's
    /// test-runner cdz builds without it (corpus-independent `cdz test`) to break the seedCompiler→corpus
    /// dep edge that over-triggered the spine on corpus-only MRs.
    #[cfg(feature = "corpus")]
    Corpus(cdz_corpus::cli::CorpusArgs),

    // ── calc (cdz-calc) ─────────────────────────────────────────────────────────────────────────
    /// The calculator REPL over the real language, exact by construction (`cdz calc`, alias `cdz repl`) — a
    /// PASSTHROUGH to the standalone `cdz-calc` binary (`cdz calc <args…>` == `cdz-calc <args…>`), so a single
    /// `cdz` on the PATH reaches the calculator. It is exec-not-link (like `cdz run`/`smith`/`cad`) so `cdz`
    /// sheds the `cdz-calc` library — and with it the transitive `cdz-run`→wasmtime the REPL pulls in to run
    /// compiled exprs (thin-`cdz` seam, `design/DESIGN-cdz-plugin-dispatch.md`). Run `cdz calc --help` for the
    /// full flag set (`--once`, `--plain`, `--sexpr`, `--no-exact`, …) — the standalone bin's own.
    #[command(alias = "repl")]
    Calc(CalcArgs),

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
    /// `opt_level`, the `modules`/`tests`/`exclude` PATTERNS, and the `deps` path-dependencies) plus their
    /// RESOLVED glob-expanded file sets (`entry_file`, `module_files`, `test_files`), so a consumer sees
    /// both intent and concrete files (and the project graph) without re-implementing glob resolution.
    Metadata(MetadataArgs),
    /// Print the project's DEPENDENCY TREE (the `cargo tree` analogue): the root project and, indented
    /// beneath it, each `def deps` path-dependency — recursively, so a dep's own deps appear nested. Each
    /// node shows `name (path)`. A dependency that cannot be resolved (no `Project.cdz` at its path) is
    /// marked `*unresolved*`, and a dependency already shown higher in the tree is marked `(*)` and not
    /// re-expanded (so a dependency cycle terminates). Resolves the same `Project.cdz` as `cdz build`/`cdz
    /// metadata` (searching upward when given no argument).
    Tree(TreeArgs),
    /// Add a PATH DEPENDENCY to the project's `Project.cdz` (the `cargo add` analogue): append PATH to the
    /// manifest's `def deps` list (creating the `def deps` line if absent), so you don't hand-edit the
    /// manifest. Idempotent — an already-present path is a no-op with a notice, not a duplicate. Edits the
    /// manifest TEXT in place (preserving its formatting + comments), then re-parses to confirm it's still
    /// valid. Warns (but still adds) when PATH has no `Project.cdz` yet — the dep may not exist yet, same as
    /// `cdz tree`'s `*unresolved*`. Resolves the same `Project.cdz` as `cdz build` (searching upward with
    /// no `--manifest`).
    Add(AddArgs),

    /// Remove a PATH DEPENDENCY from the project's `Project.cdz` (the `cargo remove` analogue): drop PATH
    /// from the manifest's `def deps` list, so you don't hand-edit the manifest. Idempotent — a path that
    /// isn't a declared dependency is a no-op with a notice. Edits the manifest TEXT in place (preserving
    /// its formatting + comments), then re-parses to confirm it's still valid + no longer declares PATH,
    /// rolling back on any failure. Resolves the same `Project.cdz` as `cdz build`/`cdz add` (searching
    /// upward with no `--manifest`). The inverse of `cdz add`.
    Remove(RemoveArgs),

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
    #[cfg(feature = "completions")]
    Completions(CompletionsArgs),

    // ── toolchain health ────────────────────────────────────────────────────────────────────────
    /// Diagnose the `cdz` TOOLCHAIN environment (a `cargo`-doctor-style preflight): the `cdz` version +
    /// path, whether the standalone `cdz-run` binary is present (INFORMATIONAL only — `cdz run`/`cdz test`
    /// run in-process, so it is optional), and whether the value-heap runtime store holds the runtime
    /// `cdz` compiles against (needed to run a program that builds heap values). Exits non-zero only if the
    /// runtime STORE is missing/stale — the sole toolchain fault that breaks `cdz run`/`test` — so CI/setup
    /// scripts can gate on `cdz doctor`. `--store <DIR>` checks a specific store.
    Doctor(DoctorArgs),

    // ── fuzzing / differential testing (PASSTHROUGH to the separate-workspace cdz-smith bin) ──────
    /// Run the `cdz-smith` FUZZER / differential-testing driver — a PASSTHROUGH that execs the standalone
    /// `cdz-smith` binary and forwards every argument to it (`cdz smith <args…>` == `cdz-smith <args…>`),
    /// so a single `cdz` on the PATH also reaches the fuzzer for discoverability. It is a passthrough, NOT
    /// a linked-in subcommand, DELIBERATELY: `cdz-smith` is a SEPARATE cargo workspace (excluded from the
    /// seed workspace) because its coverage-guided `bolero` engine pins a `toml_datetime` that cannot
    /// co-resolve with the surface's in one lockfile, and its differential oracle pulls the
    /// wasmtime+cranelift tree that must never link into `cdz`. Exec-not-link keeps that isolation intact.
    /// The binary is located beside the running `cdz` (then on `$PATH`); if absent, build it with
    /// `cargo build -p cdz-smith`. All args/flags/exit-code are the standalone bin's — run `cdz smith --help`.
    #[command(alias = "fuzz")]
    Smith(SmithArgs),

    // ── CAD mesh export (PASSTHROUGH to the separate-workspace cdz-cad bin) ────────────────────────
    /// Run the `cdz-cad` native CAD MESH driver — a PASSTHROUGH that execs the standalone `cdz-cad` binary
    /// and forwards every argument to it (`cdz cad <args…>` == `cdz-cad <args…>`), so a single `cdz` on the
    /// PATH also reaches the mesh exporter for discoverability. It reads a rendered `Solid` s-expr (from a
    /// FILE or `-` stdin — e.g. `cdz run model.cdz | cdz cad - -o out.stl`) and writes a mesh, the output
    /// format dispatched by extension (`.stl` → binary STL, `.glb` → binary glTF). It is a passthrough, NOT
    /// a linked-in subcommand, DELIBERATELY: `cdz-cad` is a SEPARATE cargo workspace (excluded from the seed
    /// workspace) because its `manifold-csg` mesh backend builds the C++ manifold3d library via cmake, which
    /// must never enter `cdz`'s workspace/lockfile. Exec-not-link keeps that isolation intact. The binary is
    /// located beside the running `cdz` (then on `$PATH`); if absent, build it with `cargo build -p cdz-cad`.
    /// All args/flags/exit-code are the standalone bin's — run `cdz cad --help` (or `cdz-cad`'s usage).
    Cad(CadArgs),

    // ── unit testing ─────────────────────────────────────────────────────────────────────────────
    /// Compile a SEPARATE test component from a FILE's `@test`-marked NULLARY definitions and run each,
    /// reporting pass/fail. Each `@test def` crosses the boundary as a nullary entry the runner invokes;
    /// a test that RETURNS (unit) PASSES, one that TRAPS FAILS (an assertion emits its message via a host
    /// effect, then traps). The report/host effect is compiled in ONLY here — a normal `cdz compile` build
    /// never carries it. Runs each test IN-PROCESS (wasmtime + the runner are linked into `cdz` via the
    /// `cdz-run` library — no sibling binary on the PATH), so a single `cdz` both compiles and runs the
    /// suite. Exits non-zero if any test fails.
    Test(TestArgs),

    // ── watch ───────────────────────────────────────────────────────────────────────────────────
    /// Watch the project's source files and RE-RUN a command on every change (the `cargo watch`
    /// analogue) — the fast edit→feedback loop. `cdz watch` re-runs `cdz check` on save (diagnostics as
    /// you type, from the shell); `cdz watch --exec test` (or `build`/`run`) re-runs that instead. Resolves the
    /// same `Project.cdz` as `cdz build`/`test` (searching upward when given no argument) and watches its
    /// declared source set + the manifest. Rapid saves are DEBOUNCED (coalesced) and an in-flight run is
    /// superseded by the next change, so a burst of edits triggers ONE re-run. Ctrl-C exits.
    #[cfg(feature = "watch")]
    Watch(WatchArgs),

    // ── semantic queries — the in-process win (both libraries + spans) ──────────────────────────
    /// The solved type of a definition NAME in FILE, rendered (a compiler query over the type column).
    Type(TypeArgs),
    /// The inferred type of the node at a source BYTE OFFSET in FILE — a "type at cursor" (hover).
    TypeAt(TypeAtArgs),
    /// Every source location that references the definition/type NAME in FILE, as `file:line:col`
    /// (`--json` emits one structured object per reference for an editor's find-all-references).
    Uses(UsesArgs),
    /// Report every well-formedness fault in FILE (type mismatch, unbound name, …) as
    /// `file:line:col: severity [CODE]: message` — "diagnostics as you type". No export/run needed;
    /// exits non-zero if any error-severity fault is present. Surfaces every CODED fault from the whole
    /// pipeline — including emit/lowering ones (e.g. CDZ0304 constant divide-by-zero) — since
    /// `compile::diagnostics` collects coded faults. It does NOT surface a CODELESS emit-path DECLINE:
    /// e.g. the float/set/map compound-ordering carve-out over a PARAMETER (`f(x: Float64, …) = (x,1) <
    /// (y,2)`) is a code-less `Reject::decline`, so `check` exits 0 while `cdz compile` rejects it. (A
    /// LITERAL compound-ordering folds to a coded fault and DOES surface — so it's the codeless declines,
    /// not "the emit pass", that check misses.) Whether those permanent carve-outs SHOULD be coded
    /// rejections (surfacing here for free) or stay declines is a spec decline-vs-reject question in
    /// flight (v-diagnostics + operator). Also out of scope: a fault only a RUN would hit (no export/run).
    Check(CheckArgs),
    /// Apply every VERIFIED fix in FILE — each proposed fix that, applied + re-checked, actually clears
    /// its diagnostic — and write the repaired program back (or preview with `--diff`/`--dry-run`).
    /// Turns "here is the fix" into "fixed it": the capstone of `cdz check`'s structured suggestions.
    Fix(FixArgs),
    /// Go-to-definition: the defining occurrence of the name at a source BYTE OFFSET in FILE, as
    /// `file:line:col` (`--json` emits it as a structured `{file,line,col}` object for an editor).
    Def(DefArgs),
    /// The bindings visible at a source BYTE OFFSET in FILE — "variable scope tracking". Each visible
    /// binding as `file:line:col: name : type` (innermost first; `--json` emits one structured object per
    /// binding for an editor's scope/completion view).
    Scope(ScopeArgs),
    /// The module's exported interface: each `(export …)` name and its type, as
    /// `file:line:col: name : type` (`--json` emits one structured object per export for a tool).
    Exports(ExportsArgs),
    /// The document OUTLINE of FILE: every top-level declaration (value/function/type/effect/module)
    /// classified by kind, as `file:line:col: kind name` — the LSP `documentSymbol` analogue (`--json`
    /// emits one structured object per declaration for an editor/tool). The superset companion of `cdz
    /// exports` (which lists only the exported subset): `symbols` lists EVERY declaration, private ones
    /// included, so an editor can render a symbol tree / breadcrumb.
    Symbols(SymbolsArgs),
    /// SEMANTIC SYNTAX HIGHLIGHTING for FILE: every token CLASSIFIED by the role it plays (type vs
    /// constructor vs local vs call vs unbound), as `file:line:col: kind` — the LSP `semanticTokens`
    /// analogue, coloured by MEANING (the compiler's columns) rather than by spelling (`--json` emits one
    /// structured object per token for an editor).
    Highlight(HighlightArgs),
    /// The documentation of a definition NAME in FILE — its `(doc "…")` text, or a built-in's
    /// documentation (a prelude module's `(meta doc)` channel, or a grammar keyword's help) when the
    /// name is not a user definition. The doc companion of `cdz type`. `--json` emits a
    /// `{name, exists, documented, doc}` object so a tool distinguishes documented / undocumented /
    /// unknown without parsing the prose.
    Doc(DocArgs),
    /// The documentation of the definition at a source BYTE OFFSET in FILE — a "documentation at cursor"
    /// hover. Resolves the offset to a node, then to the definition it is or references, and prints that
    /// definition's `(doc "…")` text. The doc companion of `cdz type-at`/`cdz def`.
    DocAt(DocAtOffsetArgs),
    /// Extract FILE's public doc surface into a TYPE-ENRICHED `doc-module` doc-AST (cadenza-docs I2):
    /// per exported `def`/`type`/`effect`, a `doc-item` with its name, structured `(sig …)`, `///`
    /// prose, `(kind …)`, `(visibility …)`, and the RESOLVED `(ty …)` type sub-AST (from the compiler's
    /// sidecar). Reads FILE, writes the doc-AST to stdout (canonical binary by default, or a surface via
    /// `--to` for inspection). The output IS `cdzast` — a queryable structured doc index. (Distinct from
    /// `cdz doc NAME` — that looks up ONE definition's doc text; this projects the WHOLE program.)
    DocModule(DocModuleArgs),
    /// Every CONCRETE INSTANTIATION of a generic / ad-hoc-polymorphic definition NAME in FILE — the
    /// monomorphized functions one source definition becomes. Reports each specialization's concrete
    /// arguments (a recursive generic at each element type, a type-valued-parameter def at each type, and
    /// a `const` dictionary parameter at each concrete dictionary — the ad-hoc-polymorphism case).
    Instantiations(InstantiationsArgs),
    /// The EMITTED-FUNCTION LAYOUT of FILE's whole (linked import-closure) program — each reachable
    /// definition's absolute wasm FUNCTION INDEX + a stable content-hash of its `(def …)` AST subtree, in
    /// func-index order, preceded by a `defs-begin<TAB><import-base><TAB>-` marker (runtime-op imports
    /// occupy `0..import-base`). Drives `Query::FuncLayout`, which forces monomorphization + lays out the
    /// boundary exactly as a real emit does, so the reported set + order equal what `cdz test`/`cdz build`
    /// would emit. Each row is `<func-index>\t<content-hash-16hex>\t<name>`. The content-hash is a function
    /// of the def's own subtree, so a def byte-identical across two programs hashes the SAME regardless of
    /// its global position — the basis for compile-reuse cache-keying and the byte-identity witness that a
    /// shared import-closure emits identically across test files.
    FuncLayout(FuncLayoutArgs),
    /// The `@param` WIDGET MANIFEST of FILE — one record per `@param(widget: …) name : Type` site, the
    /// data a HOST (browser/CAD/notebook) reads to render a control per program parameter. Prints
    /// `file:line:col: name : type [widget=… range=[lo,hi] options=[…] default=…]` per site (`--json`
    /// emits one structured object per param — `name`, `type`, `widget`, `range`, `options`, `default`,
    /// and `file`/`line`/`col` — the shape a widget host consumes). The declared type is the checker's
    /// (via the type column); range/options/default are rendered from the source arena.
    ParamManifest(ParamManifestArgs),

    // ── editor integration ────────────────────────────────────────────────────────────────────────
    /// Run a Language Server (LSP) over stdio — the persistent editor face of the in-process query
    /// engine. An editor launches `cdz lsp` and speaks the Language Server Protocol; the server holds
    /// each open document in memory and republishes its diagnostics on every edit ("diagnostics as you
    /// type"), reusing the SAME compiler queries the one-shot subcommands drive. No arguments — it
    /// communicates only over stdin/stdout.
    #[cfg(feature = "lsp")]
    Lsp,

    // ── Cadenza-in-Cadenza (ML) compiler conformance ────────────────────────────────────────────────
    /// Run a single corpus program through the CADENZA-IN-CADENZA (ML) compiler and print a
    /// machine-readable VERDICT — the seam the `cargo xtask gate` `cadenza-ml` target invokes to measure
    /// the self-hosted compiler's conformance against the SHARED corpus (`spec/semantics/*.sexp`), the same
    /// corpus rcdzc is gated against (operator directive 2026-07-17). Reads the program SOURCE from a FILE
    /// argument, or from stdin when no file is given. Prints ONE verdict line to stdout:
    /// `value <sexpr>` (ran to that value) | `declined` (well-formed but not-yet-supported by the ML
    /// compiler's current language subset — a coverage-not-yet, not an error) | `error <msg>`. Exits 0 for
    /// any RUN OUTCOME — a decline/error is a verdict, not a shell failure; the gate maps stdout→a per-case
    /// outcome and reports a climbing `cadenza-ml conformance: X/N` line. The one non-zero exit is a
    /// HARNESS-level failure that produced no verdict (a file/stdin READ error), so a script tells "the ML
    /// compiler judged this program" from "the input couldn't be read at all". STUB: currently declines
    /// every program (the
    /// ML compiler's source front-end lands behind this subcommand next, per the A/B/C ruling); this fixes
    /// the interface so the gate wiring can land against 0/N green and each ML feature flips cases without
    /// any xtask change.
    RunMl(RunMlArgs),

    /// Compile a single program to the RUST backend, run it natively, and print the SAME machine-readable
    /// verdict shape as `cdz run-ml` — the seam the fuzzer's rust-vs-wasm differential ORACLE invokes so a
    /// program's Rust-backend value can be compared to its wasm value (from `cdz run`) like-for-like. Reads
    /// the program SOURCE from a FILE argument, or from stdin when no file is given. Emits `--target rust`,
    /// `rustc`-compiles the module (linking the pre-built `cdz-rt`/`cdz-num` rlibs beside the `cdz` binary)
    /// with a driver that renders the boundary value via the shared `cdz-rust-render` crate (byte-identical
    /// to what `cdz-run` prints), runs it, and prints ONE verdict line to stdout: `value <sexpr>` (ran to
    /// that value) | `declined` (front-end reject OR rust-backend not-yet — coverage, not a mismatch) |
    /// `trap <msg>` (a Cadenza trap = a Rust panic) | `error <msg>` (the emitted `.rs` failed to `rustc` — a
    /// MISCOMPILE the fuzzer files). Keeping `declined` and `error` DISTINCT is the fuzzer's one requirement
    /// beyond run-ml's grammar. Exits 0 for any RUN outcome (a verdict is not a shell failure); a non-zero
    /// exit is a HARNESS/USAGE failure that produced no verdict — a source READ error, or a usage mistake
    /// (a bad/ambiguous `--call`, or an arg-taking export the nullary driver can't invoke).
    RunRust(RunRustArgs),

    /// Run a program through the Cadenza-in-Cadenza (ML) compiler's WASM-EMIT backend and print a verdict
    /// — the W4 emit-equals-interpret differential seam. compiler-ml's `emit-any-src-bytes` (emit-rec-db, the
    /// unified auto-router: a self-recursive def-env routes to the multi-function recursive assembler, else
    /// delegates byte-identically to emit-db's single-main `emit-src-bytes`) compiles the source to a CORE
    /// wasm MODULE (`main : () -> i64` + a fn per recursive def, importing nothing), which this runs
    /// standalone via `wasmtime::Module` (NOT a component — no value-heap runtime), invoking `main` and
    /// printing its i64. Reads source from a FILE or stdin (like `cdz run-ml`); prints ONE verdict line:
    /// `value <n>` (the emitted module ran to that i64 — incl. a RECURSIVE program like `fac(5)`→120 via
    /// real `call` instrs) | `declined` (out of the emit subset — `emit-any-src-bytes` returned
    /// `None`, OR the emitted module TRAPPED at run time: div0/mod0/`MIN/-1` — mapped to `declined` so it
    /// matches the eval-db oracle `cdz run-ml` produces for those) | `error <msg>` (a harness break: the
    /// emitted module failed to build/instantiate). Exit 0 for any run outcome (a harness/usage failure that
    /// produced no verdict is the sole non-zero exit) — IDENTICAL to `cdz run-ml`, so the W4 gate compares
    /// run-emitted's value against run-ml's value case-by-case (emit ≡ interpret).
    RunEmitted(RunEmittedArgs),

    /// Project a CHOREOGRAPHY into one self-contained program per actor (the choreographic-protocols
    /// sidecar). Given a global-protocol source FILE — a constructor-form Cadenza module that
    /// `export { protocol, roles }` (protocol : Chor, roles : List(String)) — this runs the projection via
    /// the `implementation/choreography` package (on the RUST compiler), and for each declared role emits
    /// that actor's COMPILABLE Cadenza program (an `effect Comm` + a `def main` performing the role's
    /// projected sends/recvs). With `--out <dir>` it writes `out/<Role>.cdz` per actor; with `--compile` it
    /// then `cdz compile`s each to `out/<Role>.wasm` — a self-contained wasm component per actor. Without
    /// `--out` it prints the per-actor bundle to stdout. This is "define the protocol once, shred it into
    /// one correct-by-construction program per actor at compile time." An un-projectable protocol (a role
    /// left guessing at a choice) is rejected with the offending role named, before any actor is emitted.
    Chor(ChorArgs),
}

/// Install a `RUST_LOG`-gated trace subscriber, so rcdzc's rich `trace!` events (lower/eval/resolve
/// decisions) actually surface during a `cdz` compile. rcdzc's lib is instrumentation-only — it EMITS
/// events but installs no sink; a bin must install one for them to fire. `cdz` is the user-facing entry
/// (not an internal `cargo xtask` pipeline stage, where rcdzc's own bin uses the tool-private `CDZ_LOG`
/// to avoid `RUST_LOG` fanning out to cargo/wasmtime), so it reads the SHARED `RUST_LOG` — the standard
/// knob (`RUST_LOG=rcdzc=trace cdz compile …`). With `RUST_LOG` UNSET, nothing is installed and every
/// `trace!` site is a runtime no-op — a normal run pays nothing. Writes to stderr (stdout carries the
/// tool's real output — the emitted `.rs`/wasm/verdict — which a trace must never corrupt).
fn init_tracing() {
    if std::env::var_os("RUST_LOG").is_none() {
        return;
    }
    use tracing_subscriber::{EnvFilter, fmt};
    // `try_init` (not `init`): if a subscriber is somehow already installed, don't panic — a best-effort
    // sink is the right posture for a debugging aid.
    let _ = fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        // file:line maps each traced decision straight back to the code that made it — the point of a
        // debugging trace. Captured in the event metadata at no cost when no subscriber is installed.
        .with_file(true)
        .with_line_number(true)
        .try_init();
}

fn main() -> ExitCode {
    init_tracing();
    // git-style plugin dispatch (the thin-`cdz` seam, `design/DESIGN-cdz-plugin-dispatch.md`): if the
    // first positional token names a subcommand `cdz` does NOT know AND a `cdz-<name>` binary resolves,
    // forward to it and propagate its exit code — the generalization of the existing `cdz smith`/`cdz cad`
    // passthroughs to EVERY tool on the PATH. It is a FALLBACK, tried before clap parses: a KNOWN clap
    // subcommand always takes precedence (builtin-first, like git), so this is behavior-neutral until a
    // subcommand's in-process arm is removed (then `cdz <name>` naturally falls through to its external
    // `cdz-<name>` plugin). An unknown token with no resolvable plugin falls through to clap, which prints
    // its standard "unrecognized subcommand" error unchanged.
    if let Some(code) = try_plugin_dispatch() {
        return code;
    }
    // Top-level `cdz --help`/`-h`/`help`: augment clap's help with a git-style listing of the external
    // `cdz-<name>` plugins discovered on PATH (each best-effort annotated with its `--cdz-summary` line).
    // Only the TOP level — `cdz <sub> --help` stays clap's.
    if wants_toplevel_help(&std::env::args().collect::<Vec<_>>()) {
        return print_help_with_plugins();
    }
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
        Cmd::Normalize(a) => syntax_cli::run(syntax_cli::Cmd::Normalize(a), PROG),
        // The compiler command. `cdz` (unlike bare `rcdzc`) holds the front-end, so it can accept a
        // SOURCE file directly — parsing it in-process to the `ast` artifact, and (for a debug target)
        // the `spans` artifact too — rather than requiring a pre-built binary AST.
        Cmd::Compile(a) => run_compile(a),
        // `cdz run` — mounted from the `cdz-run` lib; the same code the standalone `cdz-run` bin runs.
        // When the `component` arg is a PROJECT (a `Project.cdz` or a directory holding one), `cdz`
        // BUILDS the manifest's entry first (the `cargo run` analogue), then runs the produced component;
        // otherwise it runs the given `.wasm`/stdin component directly.
        Cmd::Run(a) if run_target_is_project(a.component.as_deref()) => run_project(&a),
        // A SOURCE file passed to `cdz run` (`cdz run foo.sexp`) is a common mistake — `cdz run` runs a
        // COMPILED component, so it would otherwise fail with the cryptic "invalid component: failed to
        // parse WebAssembly module". Catch it early with an actionable message pointing at the two real
        // paths: compile-then-run, or run the whole project.
        Cmd::Run(a) if run_arg_is_source_file(a.component.as_deref()) => {
            let path = a
                .component
                .as_deref()
                .unwrap_or(std::path::Path::new(""))
                .display();
            eprintln!(
                "{PROG} run: `{path}` is a SOURCE file, but `cdz run` runs a COMPILED component. \
                 Compile it first — `cdz compile {path} -o out.wasm && cdz run out.wasm` (or pipe: \
                 `cdz compile {path} -o - | cdz run -`); or run the whole project with `cdz run <dir>`."
            );
            ExitCode::FAILURE
        }
        // Direct "run a COMPILED component" — FORWARD to the external `cdz-run` binary rather than linking
        // `cdz_run::cli::run` in-process (the thin-`cdz` seam: the runner holds wasmtime and should be reached
        // on PATH — `design/DESIGN-cdz-plugin-dispatch.md` S4). This arm is byte-for-byte the standalone
        // `cdz-run` (SAME `cdz_run::cli::run`, only the diagnostic prog-name differs), so forwarding the raw
        // argv after `run` — which `cdz-run` re-parses as the identical `RunArgs` — is behavior-preserving.
        // Resolves via `$CDZ_RUN_BIN` (v-nix injects it at the seed `cdz run` sites, #5115) → co-built sibling
        // → `$PATH`. The project (`run_project`, needs the compiler) + source-file guard arms above stay
        // in-process. (Does NOT yet drop the `cdz-run` dep — `run_project`/`run-rust`/`test` still link it;
        // this is the first of the runner severings that culminate in `cdz` shedding `cdz-run` + wasmtime.)
        Cmd::Run(_) => {
            let forwarded: Vec<String> = std::env::args().skip(2).collect();
            let program =
                locate_plugin("run").unwrap_or_else(|| PathBuf::from(bin_name("cdz-run")));
            passthrough_status(&program, &forwarded, "cdz-run")
        }
        // `cdz corpus` — mounted from the `cdz-corpus` lib; the same code the standalone bin runs.
        #[cfg(feature = "corpus")]
        Cmd::Corpus(a) => cdz_corpus::cli::run(&a, PROG),
        // `cdz calc` — mounted from the `cdz-calc` lib; the same code the standalone `cdz-calc` bin runs.
        Cmd::Calc(a) => run_calc(&a),
        Cmd::Build(a) => run_build(&a),
        Cmd::Metadata(a) => run_metadata(&a),
        Cmd::Tree(a) => run_tree(&a),
        Cmd::Add(a) => run_add(&a),
        Cmd::Remove(a) => run_remove(&a),
        Cmd::Clean(a) => run_clean(&a),
        Cmd::New(a) => run_new(&a),
        Cmd::Init(a) => run_init(&a),
        #[cfg(feature = "completions")]
        Cmd::Completions(a) => run_completions(&a),
        Cmd::Doctor(a) => run_doctor(&a),
        Cmd::Smith(a) => run_smith(&a),
        Cmd::Cad(a) => run_cad(&a),
        Cmd::Test(a) => run_test(&a),
        #[cfg(feature = "watch")]
        Cmd::Watch(a) => run_watch(&a),
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
        Cmd::DocModule(a) => run_doc_module(&a),
        Cmd::Instantiations(a) => run_instantiations(&a),
        Cmd::FuncLayout(a) => run_func_layout(&a),
        Cmd::ParamManifest(a) => run_param_manifest(&a),
        #[cfg(feature = "lsp")]
        Cmd::Lsp => run_lsp(),
        Cmd::RunMl(a) => run_run_ml(&a),
        Cmd::RunRust(a) => run_run_rust(&a),
        Cmd::RunEmitted(a) => run_run_emitted(&a),
        Cmd::Chor(a) => run_chor(&a),
    }
}

/// `cdz lsp` — run the stdio Language Server to completion. Returns FAILURE only on a transport-level
/// error (a broken stream); a clean client shutdown is SUCCESS. The server itself never fails on a bad
/// buffer — a query is total (an un-analyzable document yields empty diagnostics, never a crash).
#[cfg(feature = "lsp")]
fn run_lsp() -> ExitCode {
    match lsp::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{PROG} lsp: {e}");
            ExitCode::FAILURE
        }
    }
}

// ── Cadenza-in-Cadenza (ML) compiler conformance seam ─────────────────────────────────────────────

#[derive(clap::Args)]
struct RunMlArgs {
    /// The corpus program SOURCE file (s-expr / ml surface). OMITTED → read the program from stdin. The
    /// `cargo xtask gate` `cadenza-ml` target passes each `spec/semantics/*.sexp` case's program here.
    file: Option<String>,
}

#[derive(clap::Args)]
struct ChorArgs {
    /// The global-protocol SOURCE file: a constructor-form Cadenza module that `export { protocol, roles }`
    /// (protocol : Chor built from the `chor` package's constructors, roles : List(String)).
    file: String,
    /// Directory to write one `<Role>.cdz` per actor into. OMITTED → print the per-actor bundle to stdout.
    #[arg(long)]
    out: Option<String>,
    /// After writing `<Role>.cdz` files (requires `--out`), `cdz compile` each to `<Role>.wasm` — a
    /// self-contained wasm component per actor.
    #[arg(long)]
    compile: bool,
}

/// `cdz run-ml` — run ONE corpus program through the Cadenza-in-Cadenza (ML) compiler and print a
/// machine-readable verdict for the gate. Contract (fixed, invariant to the source-reader design):
///   - input: program source from FILE, else stdin.
///   - stdout: exactly ONE verdict line — `value <n>` (bare scalar, matching cdz-run's Ran::Value render so
///     the gate's differential vs rcdzc compares like-for-like) | `declined` | `error <msg>`.
///   - exit: ALWAYS 0 (a decline/error is a VERDICT, not a shell failure; the gate reserves non-zero for a
///     genuine harness crash — a driver-write / compiler-invocation failure).
///
/// MECHANISM: the ML compiler is WRITTEN IN CADENZA (`implementation/compiler-ml/src/`); its source front-end
/// `sread-eval.run-src : String -> Option(Int64)` reads a program's canonical s-expr SOURCE, runs it through
/// resolve→infer→lower→eval, and returns `Some value` | `None` (declined / out-of-subset). We invoke it by
/// generating a tiny DRIVER program that embeds the corpus source as a compile-time STRING LITERAL (String
/// can't cross the component boundary as an arg, but a literal is compile-time — no crossing) and calls
/// `run-src`, then compile+run it and read the rendered `Option`. The driver MUST live in the compiler-ml
/// `src/` dir because `import "sread-eval"` resolves RELATIVE TO THE ENTRY FILE'S DIR (no `--search-path`).
///
/// Resolves `implementation/compiler-ml/src` via [`find_impl_src_dir`] (robust upward search from cwd + exe
/// dir), so `cdz run-ml` works from any working directory. `None` if not found.
fn find_compiler_ml_src() -> Option<std::path::PathBuf> {
    find_impl_src_dir("implementation/compiler-ml/src")
}

/// Locate `implementation/choreography/src` by searching UP from the cwd and the exe's dir (same robust
/// strategy as `find_compiler_ml_src`), so `cdz chor` resolves the sidecar package's sources from any cwd.
/// The generated driver + copied protocol module must live here so their `import "chor-driver"` /
/// `import "chor"` resolve (imports are entry-file-dir-relative).
fn find_choreography_src() -> Option<std::path::PathBuf> {
    find_impl_src_dir("implementation/choreography/src")
}

/// Locate a repo-relative `implementation/<pkg>/src` dir ROBUSTLY (not assuming cwd == repo root): walk
/// upward from the current dir, then from the exe's dir, returning the first ancestor that contains `rel`.
/// So a `cdz` sidecar-driver subcommand works from any working directory — e.g. `cargo test`, whose
/// per-crate cwd is NOT the repo root (the bug that reded the shared gate). `None` if `rel` is nowhere on
/// either upward path. The shared search behind `find_compiler_ml_src` / `find_choreography_src`.
fn find_impl_src_dir(rel: &str) -> Option<std::path::PathBuf> {
    let mut roots: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd);
    }
    if let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|p| p.to_path_buf()))
    {
        roots.push(exe_dir);
    }
    for start in roots {
        let mut cur: Option<&std::path::Path> = Some(start.as_path());
        while let Some(dir) = cur {
            let candidate = dir.join(rel);
            if candidate.is_dir() {
                return Some(candidate);
            }
            cur = dir.parent();
        }
    }
    None
}

/// Cheap SOURCE-SHAPE gate: could this program POSSIBLY be in the ML compiler's supported subset (bare
/// prefix expressions — int/bool/identifier literals, `(let ((x v)) b)`, `(if c t e)`, `(op a b)`)? A `true`
/// answer only means "worth compiling"; the real reader/pipeline still decides value-vs-decline. A `false`
/// answer FAST-DECLINES without the (tens-of-seconds) whole-pipeline compile. Conservative: reject the forms
/// the subset definitely can't express — a top-level module `(do …)` / `(def …)` / `(export …)`, a string
/// (`"`), a float/decimal (`.`), and empty input — so the gate never pays the compile cost for them.
fn looks_in_ml_subset(src: &str) -> bool {
    let s = src.trim();
    if s.is_empty() {
        return false;
    }
    // A quoted string is out of the integer/bool subset. A DECIMAL/FLOAT (a `.` adjacent to a digit, e.g.
    // `3.14` / `.5`) is out too — but a POSITIONAL TUPLE PROJECTION `(. t i)`, where `.` is a HEAD atom
    // (flanked by `(` and whitespace, never a digit), IS in subset now that the port lowers `(. tuple i)` to a
    // scalar element read. So decline `"` and a float-`.`, but admit a projection-`.` (a record/non-tuple
    // operand or non-int index still declines in the port — coverage-not-yet — so admitting it is sound).
    // STRINGS S1: a SCALAR-STRING form `(String.byte-len "…")` / `(String.scalar-len "…")` CONST-FOLDS to an
    // Int and runs to the same value the rcdzc oracle produces, so admit it — the differential then exercises
    // the landed string-length ops (they render as a plain Int, which the differential already handles). Any
    // OTHER quoted string — a bare string, or a string-RESULT op the port can't render as a scalar — is still
    // out of the scalar subset → decline (else the oracle renders a string the port declines, a false
    // differential disagreement).
    let scalar_string_form =
        s.starts_with("(String.byte-len \"") || s.starts_with("(String.scalar-len \"");
    if s.contains('"') && !scalar_string_form {
        return false;
    }
    let b = s.as_bytes();
    let has_float = s.match_indices('.').any(|(i, _)| {
        (i > 0 && b[i - 1].is_ascii_digit()) || (i + 1 < b.len() && b[i + 1].is_ascii_digit())
    });
    if has_float {
        return false;
    }
    // A `(do …)` MODULE with a NULLARY `main` IS in subset — the ML reader peels `main` to its body and
    // resolves calls to sibling `def`s (nullary + UNtyped parameterized helpers), so a multi-definition
    // module like `(do (def (g) 7) (def (f x) (+ x (g))) (def (main) (f 5)) (export main))` runs end-to-end.
    // Detect a nullary main ANYWHERE in the module (`(def (main) ` — the `)` right after `main` = no params),
    // not just as the FIRST def, so helper-first modules qualify too. A PARAMETERIZED main
    // `(def (main (: n …)) …)` has a `(` after `main` (never `(def (main) `), so it does NOT match here.
    //
    // We DELIBERATELY no longer cap the `(def` count at 1: the reader handles UNTYPED helper defs + calls, and
    // the cadenza-ml conformance step is REPORT-ONLY (differential vs the wasm oracle, never a gate baseline).
    //
    // We STILL fast-decline a `(fn …)` lambda — the reader has no lambda form.
    //
    // We NO LONGER exclude a TYPE ANNOTATION `(: … …)`. That exclusion was load-bearing WHILE the ML
    // front-end read a typed annotation but DISCARDED the type (an overflow like `(: a Int8)` fed `200`, an
    // unbound type `(: x foo)`, or a mismatch `(: x Bool)` ran to a bogus value instead of rejecting) — so
    // fast-declining kept the W4 differential honest. That is no longer true: the narrow-int annotation
    // ENFORCEMENT slice landed — an in-range `(: 100 Int8)` runs to 100, an overflow `(: 200 Int8)` /
    // `(: 300 UInt8)` and a same-width arith overflow `(+ (: 100 Int8) (: 100 Int8))` all correctly DECLINE
    // (CDZ0304), matching the reference verbatim. The exclusion was ALSO over-broad: it fired only in the
    // nullary-main-module BRANCH, so a BARE `(: 100 Int8)` already ran (to 100) while the SAME annotation
    // inside `(do (def (main) …) (export main))` wrongly fast-declined — a spurious wrapper-vs-bare
    // asymmetry (v-inference root-caused this; it was NOT a scale-emergent emit bug). Dropping `(: ` lets
    // both forms run and be correctly checked by the (now-enforcing) pipeline. `(fn` stays excluded.
    let is_nullary_main_module = {
        let no_ws: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
        no_ws.starts_with("(do ") && no_ws.contains("(def (main) ")
    };
    if is_nullary_main_module {
        if s.contains("(fn") {
            return false;
        }
        return true;
    }
    // A bare `(do …)` that is NOT the nullary-main shape, or any top-level definition/module, is out of
    // subset (the bare-expression subset is a single expression).
    if s.starts_with("(do") || s.contains("(def") || s.contains("(export") || s.contains("(fn") {
        return false;
    }
    true
}

/// Read a verdict-runner's program SOURCE (`cdz run-ml`/`run-rust`/`run-emitted`) from `file` or stdin.
/// A missing `file` arg reads stdin (the gate/oracle pipe programs in); an EXPLICIT `-` reads stdin too —
/// the stdin marker `cdz fmt -`/`convert -`/`compile -`/`run -` already use, so a script that pipes with a
/// `-` (the shell convention everywhere else) doesn't hit `read_to_string("-")` leaking the raw
/// `cannot read -: No such file or directory (os error 2)` errno. `cmd` labels the reserved harness-error
/// message. `Err(())` is the sole non-zero harness exit (a read failure produced no verdict); the caller
/// maps it to `ExitCode::FAILURE`.
fn read_verdict_source(file: Option<&str>, cmd: &str) -> Result<String, ()> {
    use std::io::Read;
    match file {
        Some(path) if path != "-" => std::fs::read_to_string(path).map_err(|e| {
            eprintln!("{PROG} {cmd}: cannot read {path}: {e}");
        }),
        _ => {
            let mut buf = String::new();
            std::io::stdin().read_to_string(&mut buf).map_err(|e| {
                eprintln!("{PROG} {cmd}: cannot read stdin: {e}");
            })?;
            Ok(buf)
        }
    }
}

fn run_run_ml(args: &RunMlArgs) -> ExitCode {
    // 1. Read the program source (file or stdin, incl. an explicit `-`). A read failure is the reserved
    //    harness-error path.
    let source = match read_verdict_source(args.file.as_deref(), "run-ml") {
        Ok(s) => s,
        Err(()) => return ExitCode::FAILURE,
    };

    // 1b. FAST-DECLINE out-of-subset programs WITHOUT compiling. Building the driver (which links the whole
    //     compiler-ml pipeline) costs tens of seconds — far too slow to run per corpus case, and pointless
    //     for the ~majority of corpus programs the ML front-end can't express yet. A cheap source scan
    //     rejects anything outside the supported bare-expression subset (int/bool/ident/`(let …)`/`(if …)`/
    //     `(op …)`): a top-level `(do …)`/`(def …)`/`(export …)` module, or an unbalanced/empty form. Only a
    //     program that PASSES this shape gate pays the compile cost — so `run-ml` is fast for the common
    //     out-of-subset case and the gate never hangs on the whole-pipeline compile.
    if !looks_in_ml_subset(source.trim()) {
        println!("declined");
        return ExitCode::SUCCESS;
    }

    // 2. Generate the driver: it embeds the source as a Cadenza string literal and calls run-src. The
    //    literal must escape `\` and `"`; the corpus s-expr programs are single-line ASCII (no newlines),
    //    so this minimal escape is sufficient (a defensive newline→space keeps it one line regardless).
    let escaped = source
        .trim()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', " ");
    let driver = format!(
        "import {{ run-src-typed }} from \"sread-eval\"\ndef main() = run-src-typed(\"{escaped}\")\nexport {{ main }}\n"
    );

    // 3. Write the driver INTO the compiler-ml src dir (so `import \"sread-eval\"` resolves — imports are
    //    entry-dir-relative). Located ROBUSTLY (not assuming cwd == repo root): search upward from the cwd
    //    AND from the exe's dir for `implementation/compiler-ml/src`, so `cdz run-ml` works from any cwd
    //    (e.g. under `cargo test`, whose cwd is the crate dir, not the repo root).
    let src_dir = match find_compiler_ml_src() {
        Some(d) => d,
        None => {
            eprintln!(
                "{PROG} run-ml: compiler-ml src dir not found (searched up from cwd + exe dir)"
            );
            return ExitCode::FAILURE;
        }
    };
    // PER-PROCESS driver filename (pid-stamped), NOT a fixed `zz-run-ml-driver.cdz` (Copilot PR #536):
    // the differential ML gate runs many corpus cases, often in PARALLEL, each shelling `cdz run-ml`. A
    // fixed name means concurrent invocations write + delete the SAME file — one run clobbers another's
    // driver (compiling a wrong/half-written program) or deletes it mid-compile, and an interrupted run
    // leaves it behind. A pid-stamped name is unique per invocation (matching `compile_run_ml_driver`'s
    // pid-stamped `tmp_wasm`), so parallel run-ml can't race. It must still live in `src_dir` (a sibling of
    // the compiler-ml sources) because `import "sread-eval"` resolves RELATIVE TO THE ENTRY FILE'S DIR —
    // only the FILENAME varies; every return below still removes it.
    let driver_path = src_dir.join(format!("zz-run-ml-driver-{}.cdz", std::process::id()));
    if let Err(e) = std::fs::write(&driver_path, &driver) {
        eprintln!("{PROG} run-ml: cannot write driver: {e}");
        return ExitCode::FAILURE;
    }
    // The driver lives in the SHARED compiler-ml src tree (imports resolve entry-dir-relative, so it can't
    // go in a temp dir). Remove it on EVERY exit path via an RAII guard, not a single trailing line: a
    // panic or an early `return` (e.g. a future guard added below `compile_run_ml_driver`) would otherwise
    // leak a `zz-run-ml-driver-<pid>.cdz` into every agent's `git status`. (A SIGTERM'd process still can't
    // run Drop — that residue is what the `.gitignore` entry covers — but this closes every in-process path.)
    let _driver_guard = RemoveOnDrop::file(driver_path.clone());

    // 4. Compile + run the driver by shelling `cdz` to itself (install-location-independent via current_exe),
    //    the same compile→run path the gate uses for rcdzc. Capture stdout (the rendered Option). The
    //    `_driver_guard` above removes the driver when this fn returns (any path).
    let verdict = compile_run_ml_driver(&driver_path);

    match verdict {
        Ok(v) => {
            println!("{v}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            // A harness-level failure (couldn't invoke the compiler/runner). Non-zero so the gate flags it
            // distinctly from a verdict; the message goes to stderr, stdout stays verdict-free.
            eprintln!("{PROG} run-ml: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Compile + run the generated driver and MAP its output to a verdict line. `cdz compile <driver> -o <tmp>`
/// then `cdz run <tmp> --call main`; the run prints `(: (Some N) (Option Int64))` / `(: (None …) …)`. Returns
/// the verdict STRING (`value N` | `declined` | `error …`) on a successful invocation, or `Err` for a
/// harness failure (couldn't spawn the compile/run subprocess). A driver COMPILE failure maps to
/// `error <diagnostic>` (surfacing the real CDZ error), NOT `declined` — the driver is fixed per program
/// (the user source is a runtime string arg), so a compile failure is a harness/front-end error, not an
/// out-of-subset decline (that happens at run time via `run-src-typed` → `None`).
fn compile_run_ml_driver(driver_path: &std::path::Path) -> Result<String, String> {
    use std::process::Command;
    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let tmp_wasm = std::env::temp_dir().join(format!("cdz-run-ml-{}.wasm", std::process::id()));

    // Compile the DRIVER. Crucial: the driver embeds the user program as a STRING literal handed to
    // `run-src-typed` (compiled INSIDE the guest at run time) — so this outer `cdz compile` only builds the
    // fixed driver + `sread-eval`, which is IDENTICAL for every program. Therefore a driver compile FAILURE
    // is NOT "the user program is out of the ML subset" (that decline happens at RUN time when
    // `run-src-typed` returns `None`, and is pre-empted by `looks_in_ml_subset`); it's a genuine
    // HARNESS/COMPILE error (sread-eval itself failing to build, a driver-gen bug, a real front-end
    // diagnostic like an unbound name). Surface it as `error <diagnostic>` — NOT a blanket `declined`, which
    // silently swallowed the real CDZ error and made a specific fault look like "run-ml declines everything"
    // (corpus-bugfix/v-compiler-ml UX report). Reserve `declined` for a genuine not-yet-supported construct.
    let compile = Command::new(&exe)
        .arg("compile")
        .arg(driver_path)
        .arg("-o")
        .arg(&tmp_wasm)
        .output()
        .map_err(|e| format!("spawn compile: {e}"))?;
    if !compile.status.success() {
        let _ = std::fs::remove_file(&tmp_wasm);
        // Prefer the compiler's first diagnostic line (stderr, else stdout); collapse to one line so the
        // verdict stays single-line. Fall back to a generic message if the compiler emitted nothing.
        let diag = {
            let stderr = String::from_utf8_lossy(&compile.stderr);
            let stdout = String::from_utf8_lossy(&compile.stdout);
            let first = stderr
                .lines()
                .chain(stdout.lines())
                .map(str::trim)
                .find(|l| !l.is_empty())
                .unwrap_or("driver compile failed with no diagnostic");
            first.to_string()
        };
        return Ok(format!("error {diag}"));
    }

    // Run.
    let run = Command::new(&exe)
        .arg("run")
        .arg(&tmp_wasm)
        .arg("--call")
        .arg("main")
        .output()
        .map_err(|e| format!("spawn run: {e}"));
    let _ = std::fs::remove_file(&tmp_wasm);
    let run = run?;
    let out = String::from_utf8_lossy(&run.stdout);
    if !run.status.success() {
        // The driver compiled but TRAPPED at run time (e.g. divide-by-zero) — a decline-shaped outcome for
        // the differential (rcdzc would trap too; the gate treats trap-vs-value, but for the ML subset a
        // trap is "no value" → declined keeps run-ml total and the gate never sees a spurious value).
        return Ok("declined".to_string());
    }
    Ok(parse_ml_option_render(out.trim()))
}

/// Map cdz-run's rendered `Option((Int64, Int64))` to a verdict. The driver calls `run-src-typed`, which
/// returns `Some (value, isBool)`: rendered as `(: (Some (tuple V B)) (Option (Tuple Int64 Int64)))`. The
/// verdict is ALWAYS `value <scalar>` (the contract shape the run_ml_cli tests + gate wiring pin) where
/// `<scalar>` is exactly what `rcdzc`'s `Ran::Value` renders bare, so the differential strips `value ` and
/// compares the scalar against the oracle:
///   • `B == 0` (Int-typed program) → `value V` — `V` is the bare integer (`rcdzc` renders e.g. `42`).
///   • `B == 1` (Bool-typed program) → `value true` / `value false` — Core encodes Bool as the Int 0/1, but
///     `rcdzc` renders a Bool as `true`/`false`; we render the SAME scalar so the strings agree.
///   • `(: (None …) …)` (or any unexpected shape) → `declined` (conservative — never emit a bogus `value`).
fn parse_ml_option_render(rendered: &str) -> String {
    // Find `(tuple ` (the Some payload) and take the two whitespace-separated tokens up to the next `)`.
    if let Some(rest) = rendered.split("(tuple ").nth(1)
        && let Some(inner) = rest.split(')').next()
    {
        let mut it = inner.split_whitespace();
        if let (Some(v), Some(b)) = (it.next(), it.next()) {
            return match b {
                // Bool-typed: render the scalar rcdzc renders (0 → `false`, non-zero → `true`).
                "1" => format!("value {}", if v == "0" { "false" } else { "true" }),
                _ => format!("value {v}"),
            };
        }
    }
    "declined".to_string()
}

/// `cdz chor <protocol.cdz> [--out <dir>] [--compile]` — project a choreography into one program per actor.
///
/// Runs the projection through the `implementation/choreography` Cadenza package (on the rust compiler): it
/// copies the user's protocol module into the package src dir (so its imports resolve), generates a driver
/// that calls `render-all(roles, protocol)` and returns the per-actor bundle as a String, compiles+runs it,
/// then splits the bundle on `==== <Role> ====` markers. With `--out` it writes each actor's module to
/// `<dir>/<Role>.cdz` (and, with `--compile`, `cdz compile`s each to `<dir>/<Role>.wasm`); otherwise it
/// prints the bundle. Temp files (the copied protocol + the driver) are removed on every exit via RAII.
fn run_chor(args: &ChorArgs) -> ExitCode {
    // 1. Read the protocol module source.
    let proto_src = match std::fs::read_to_string(&args.file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{PROG} chor: cannot read {}: {e}", args.file);
            return ExitCode::FAILURE;
        }
    };

    // 2. Locate the choreography package src dir (imports resolve entry-dir-relative, so the driver + the
    //    copied protocol must live there).
    let src_dir = match find_choreography_src() {
        Some(d) => d,
        None => {
            eprintln!(
                "{PROG} chor: choreography src dir not found (searched up from cwd + exe dir for implementation/choreography/src)"
            );
            return ExitCode::FAILURE;
        }
    };

    // 3. Generate a driver that renders every actor. Two input surfaces, chosen by file extension:
    //    - `.sexp` / `.chor`: the file is a bare protocol S-EXPR — `(seq (comm Buyer Seller Title) …)`. The
    //      driver embeds it as a string literal and does `from-ast(read(src))` (chor-sread), inferring the
    //      role set from the protocol via `roles-of` (self-describing — no separate roles decl needed).
    //    - anything else (`.cdz`): the file is a CONSTRUCTOR-FORM module that `export { protocol, roles }`;
    //      the driver imports it. Copied into the package src dir so its imports resolve.
    //    Temp files (driver + any copied module) are pid-stamped (no concurrent-invocation race) and
    //    RAII-removed on every exit path.
    let pid = std::process::id();
    let driver_path = src_dir.join(format!("zz-chor-driver-{pid}.cdz"));
    let is_sexp = args.file.ends_with(".sexp") || args.file.ends_with(".chor");
    // `_proto_guard` must outlive the driver run, so bind it in this scope even when unused (the .sexp path).
    let mut _proto_guard: Option<RemoveOnDrop> = None;
    let driver = if is_sexp {
        // Embed the s-expr as a one-line Cadenza string literal (escape `\`, `"`, newline→space — protocol
        // s-exprs are whitespace-insensitive, and `read` skips interior whitespace) and call the package's
        // `shred-sexp` (read → from-ast → roles-of → render-all), which keeps the driver a single call.
        let escaped = proto_src
            .trim()
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', " ");
        format!(
            "import {{ shred-sexp }} from \"chor-driver\"\ndef main() = shred-sexp(\"{escaped}\")\nexport {{ main }}\n"
        )
    } else {
        let proto_mod = format!("zz-chor-proto-{pid}");
        let proto_path = src_dir.join(format!("{proto_mod}.cdz"));
        if let Err(e) = std::fs::write(&proto_path, &proto_src) {
            eprintln!("{PROG} chor: cannot write protocol temp module: {e}");
            return ExitCode::FAILURE;
        }
        _proto_guard = Some(RemoveOnDrop::file(proto_path.clone()));
        format!(
            "import {{ render-all }} from \"chor-driver\"\nimport {{ protocol, roles }} from \"{proto_mod}\"\ndef main() = render-all(roles, protocol)\nexport {{ main }}\n"
        )
    };
    if let Err(e) = std::fs::write(&driver_path, &driver) {
        eprintln!("{PROG} chor: cannot write driver: {e}");
        return ExitCode::FAILURE;
    }
    let _driver_guard = RemoveOnDrop::file(driver_path.clone());

    // 4. Compile + run the driver, capturing the rendered String bundle.
    let bundle = match compile_run_chor_driver(&driver_path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{PROG} chor: {e}");
            return ExitCode::FAILURE;
        }
    };

    // 5. Split into per-actor sections on the `==== <Role> ====` markers.
    let actors = split_actor_bundle(&bundle);
    if actors.is_empty() {
        // The bundle carries the specific verdict (from chor-driver's `render-all`/`shred-sexp`); tailor the
        // message to it so the user gets the actual cause + fix, not a catch-all guess.
        eprintln!("{PROG} chor: {}", chor_no_actors_reason(&bundle, is_sexp));
        return ExitCode::FAILURE;
    }

    // 5b. WARN on recursion degradation. `render-compilable` does not yet emit a `def main` back-jump for a
    // recursive protocol (LRecT/LVarT), so it emits a `-- rec: unsupported` (LRecT) or `-- var: unsupported`
    // (LVarT) marker inside an otherwise valid, COMPILABLE stub actor (the loop body is dropped). Without this warning the emitted actor
    // compiles+runs fine but silently does NOT loop, so a user could believe a recursive protocol shredded
    // correctly. Name every affected role so the degradation is visible, not buried in a comment.
    let degraded: Vec<&str> = actors
        .iter()
        .filter(|(_role, module)| chor_module_is_rec_degraded(module))
        .map(|(role, _module)| role.as_str())
        .collect();
    if !degraded.is_empty() {
        eprintln!(
            "{PROG} chor: warning: recursive protocol — {} emitted a NON-LOOPING stub (the loop body is \
             dropped; rec-emit is not yet supported). The actor compiles but does not repeat; treat it as a \
             single-session projection only.",
            degraded.join(", ")
        );
    }

    match &args.out {
        None => {
            print!("{bundle}");
            ExitCode::SUCCESS
        }
        Some(dir) => {
            if let Err(e) = std::fs::create_dir_all(dir) {
                eprintln!("{PROG} chor: cannot create out dir {dir}: {e}");
                return ExitCode::FAILURE;
            }
            for (role, module) in &actors {
                let cdz_path = std::path::Path::new(dir).join(format!("{role}.cdz"));
                if let Err(e) = std::fs::write(&cdz_path, module) {
                    eprintln!("{PROG} chor: cannot write {}: {e}", cdz_path.display());
                    return ExitCode::FAILURE;
                }
                println!("wrote {}", cdz_path.display());
                if args.compile {
                    let exe = match std::env::current_exe() {
                        Ok(e) => e,
                        Err(e) => {
                            eprintln!("{PROG} chor: current_exe: {e}");
                            return ExitCode::FAILURE;
                        }
                    };
                    let wasm_path = std::path::Path::new(dir).join(format!("{role}.wasm"));
                    let out = std::process::Command::new(&exe)
                        .arg("compile")
                        .arg(&cdz_path)
                        .arg("-o")
                        .arg(&wasm_path)
                        .output();
                    match out {
                        Ok(o) if o.status.success() => {
                            println!("compiled {}", wasm_path.display());
                        }
                        Ok(o) => {
                            eprintln!(
                                "{PROG} chor: compiling {} failed:\n{}",
                                cdz_path.display(),
                                String::from_utf8_lossy(&o.stderr)
                            );
                            return ExitCode::FAILURE;
                        }
                        Err(e) => {
                            eprintln!("{PROG} chor: spawn compile: {e}");
                            return ExitCode::FAILURE;
                        }
                    }
                }
            }
            ExitCode::SUCCESS
        }
    }
}

/// Compile + run the `cdz chor` driver (shelling `cdz` to itself), returning the rendered String bundle with
/// the `(: "…" String)` boundary wrapper stripped + unescaped.
fn compile_run_chor_driver(driver_path: &std::path::Path) -> Result<String, String> {
    use std::process::Command;
    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let tmp_wasm = std::env::temp_dir().join(format!("cdz-chor-{}.wasm", std::process::id()));
    let compile = Command::new(&exe)
        .arg("compile")
        .arg(driver_path)
        .arg("-o")
        .arg(&tmp_wasm)
        .output()
        .map_err(|e| format!("spawn compile: {e}"))?;
    if !compile.status.success() {
        let _ = std::fs::remove_file(&tmp_wasm);
        let stderr = String::from_utf8_lossy(&compile.stderr);
        // The common author mistake is a constructor-form protocol file that doesn't export `protocol` /
        // `roles`. rcdzc surfaces that as a `does not export …` diagnostic naming the INTERNAL temp module
        // (`zz-chor-proto-<pid>`), which leaks an implementation detail. Give a clean, actionable message
        // instead; other compile failures still dump the raw diagnostics (which point at the user's source).
        if stderr.contains("does not export") {
            return Err(
                "the protocol file must export both `protocol` (a Chor) and `roles` (a List(String)) — e.g. `export { protocol, roles }` — or use a bare `.sexp`/`.chor` protocol file (no exports needed)."
                    .to_string(),
            );
        }
        return Err(format!("the protocol file did not compile:\n{stderr}"));
    }
    let run = Command::new(&exe)
        .arg("run")
        .arg(&tmp_wasm)
        .arg("--call")
        .arg("main")
        .output();
    let _ = std::fs::remove_file(&tmp_wasm);
    let run = run.map_err(|e| format!("spawn run: {e}"))?;
    if !run.status.success() {
        return Err(format!(
            "driver trapped at run time:\n{}",
            String::from_utf8_lossy(&run.stderr)
        ));
    }
    Ok(unwrap_string_value(
        String::from_utf8_lossy(&run.stdout).trim(),
    ))
}

/// Strip `cdz run`'s String boundary render `(: "<body>" String)` back to `<body>`, unescaping `\n`, `\"`,
/// `\\`. If the output isn't the expected wrapper (e.g. already bare), return it as-is.
fn unwrap_string_value(rendered: &str) -> String {
    let inner = rendered
        .strip_prefix("(: \"")
        .and_then(|s| s.strip_suffix("\" String)"))
        .unwrap_or(rendered);
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Split a `render-all` bundle into (role, module) pairs on `==== <Role> ====` section markers.
fn split_actor_bundle(bundle: &str) -> Vec<(String, String)> {
    let mut actors: Vec<(String, String)> = Vec::new();
    let mut current: Option<(String, String)> = None;
    for line in bundle.lines() {
        if let Some(rest) = line.strip_prefix("==== ")
            && let Some(role) = rest.strip_suffix(" ====")
        {
            if let Some((r, m)) = current.take() {
                actors.push((r, m.trim_end().to_string()));
            }
            current = Some((role.trim().to_string(), String::new()));
        } else if let Some((_, m)) = current.as_mut() {
            m.push_str(line);
            m.push('\n');
        }
    }
    if let Some((r, m)) = current.take() {
        actors.push((r, m.trim_end().to_string()));
    }
    actors
}

/// Does an emitted actor module carry the recursion-degradation marker? `render-compilable` does not yet emit
/// a `def main` back-jump for a recursive protocol (LRecT/LVarT), so it leaves a `-- rec: unsupported` /
/// `-- var: unsupported` marker in an otherwise-compilable stub whose loop body is dropped. `cdz chor` uses
/// this to WARN (the stub compiles but does not loop, so the degradation must be surfaced, not buried).
fn chor_module_is_rec_degraded(module: &str) -> bool {
    module.contains("-- rec: unsupported") || module.contains("-- var: unsupported")
}

/// Turn a no-actors `render-all`/`shred-sexp` bundle verdict into an ACTIONABLE `cdz chor` error message
/// (rustc bar: name the cause + the fix). The bundle is the driver's own verdict string — `not-a-protocol`
/// (only the `.sexp` path: unreadable/unparseable), `not-projectable: <role>`, `not-well-formed`, or (the
/// `.cdz` path) a run that emitted no `==== <Role> ====` sections, usually a missing `protocol`/`roles`
/// export. `is_sexp` disambiguates the last case so we never tell a `.sexp` user about exports they don't need.
fn chor_no_actors_reason(bundle: &str, is_sexp: bool) -> String {
    let b = bundle.trim();
    if b == "not-a-protocol" {
        "the file is not a readable protocol s-expr (check for balanced parens and a known head: `seq`, `comm`, `choice`, `branch`, `rec`, `var`, `done`)".to_string()
    } else if let Some(role) = b.strip_prefix("not-projectable: ") {
        format!(
            "role `{role}` lacks knowledge of a choice — have the deciding role send `{role}` a distinct selection message as its first action in every branch it participates in, or make `{role}`'s behaviour identical in all branches"
        )
    } else if b == "not-well-formed" {
        "the protocol is not well-formed (undeclared/self comm, undeclared chooser, non-branch in a choice, or an unbound rec-var)".to_string()
    } else if is_sexp {
        format!("no actors emitted. Bundle: {bundle}")
    } else {
        format!(
            "no actors emitted — the protocol file must export both `protocol` and `roles`. Bundle: {bundle}"
        )
    }
}

#[derive(clap::Args)]
struct RunEmittedArgs {
    /// The program SOURCE file (s-expr / ml surface). OMITTED → read the program from stdin. Mirrors
    /// `cdz run-ml`'s input contract so the W4 emit-equals-interpret differential harness is symmetric.
    file: Option<String>,
}

/// `cdz run-emitted` — run a program through compiler-ml's WASM-EMIT backend + print a verdict (the W4
/// emit-equals-interpret seam). See the `RunEmitted` Cmd doc for the contract. MECHANISM (mirrors run-ml):
/// write a driver `import { emit-any-src-bytes } from "emit-rec-db"` whose `main = emit-any-src-bytes("<source>")` into
/// `implementation/compiler-ml/src/` (imports resolve entry-dir-relative), compile+run it via `cdz` to
/// SELF; the driver returns `Option(List UInt8)` — `None` → declined; `Some(list …)` → the raw module bytes
/// (parsed from the rendered decimal-`u8` list, lossless), which we run as a core `wasmtime::Module`
/// (`cdz_run::run_core_module`): a returned i64 → `value <n>`, a run-time TRAP → `declined` (matches the
/// eval-db oracle for div0/mod0/`MIN/-1`), an invalid/uninstantiable module → `error <msg>`.
fn run_run_emitted(args: &RunEmittedArgs) -> ExitCode {
    // 1. Read the program source (file or stdin, incl. an explicit `-`). A read failure is the reserved
    //    harness-error path.
    let source = match read_verdict_source(args.file.as_deref(), "run-emitted") {
        Ok(s) => s,
        Err(()) => return ExitCode::FAILURE,
    };

    // 2. Generate the driver: embed the source as a Cadenza string literal + call emit-any-src-bytes. Escape
    //    `\`/`"` (corpus programs are single-line ASCII; the newline→space is defensive).
    let escaped = source
        .trim()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', " ");
    let driver = format!(
        "import {{ emit-any-src-bytes }} from \"emit-rec-db\"\ndef main() = emit-any-src-bytes(\"{escaped}\")\nexport {{ main }}\n"
    );

    // 3. Write the driver INTO the compiler-ml src dir (so `import "emit-rec-db"` resolves — imports are
    //    entry-dir-relative), located robustly (cwd-independent). Pid-stamped + RAII-cleaned like run-ml's.
    let src_dir = match find_compiler_ml_src() {
        Some(d) => d,
        None => {
            eprintln!(
                "{PROG} run-emitted: compiler-ml src dir not found (searched up from cwd + exe dir)"
            );
            return ExitCode::FAILURE;
        }
    };
    let driver_path = src_dir.join(format!("zz-run-emitted-driver-{}.cdz", std::process::id()));
    if let Err(e) = std::fs::write(&driver_path, &driver) {
        eprintln!("{PROG} run-emitted: cannot write driver: {e}");
        return ExitCode::FAILURE;
    }
    let _driver_guard = RemoveOnDrop::file(driver_path.clone());

    // 4. Compile + run the driver via `cdz` to self → the rendered `Option(List UInt8)`, then map to a
    //    verdict: None → declined; Some(bytes) → run the core module (i64 → value, trap → declined,
    //    bad-artifact → error). A harness failure (couldn't invoke the compiler/runner) is non-zero.
    match emit_and_run_module(&driver_path) {
        Ok(verdict) => {
            println!("{verdict}");
            ExitCode::SUCCESS
        }
        Err(msg) => {
            eprintln!("{PROG} run-emitted: {msg}");
            ExitCode::FAILURE
        }
    }
}

/// Compile + run the run-emitted driver (via `cdz` to self), capture the rendered `Option(List UInt8)`, and
/// map it to a verdict STRING. A driver COMPILE failure → `declined` (out of subset — the front-end can't
/// build it). A run that yields `None` → `declined`. `Some(list …)` → parse the raw module bytes and run
/// them as a core module: i64 → `value <n>`, trap → `declined`, invalid module → `error …`. `Err` is a
/// harness failure (couldn't spawn the compiler/runner) — the caller makes it a non-zero exit.
fn emit_and_run_module(driver_path: &std::path::Path) -> Result<String, String> {
    use std::process::Command;
    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let tmp_wasm =
        std::env::temp_dir().join(format!("cdz-run-emitted-{}.wasm", std::process::id()));

    // Compile the driver. A compile failure → the program is out of the emit subset → declined.
    let compile = Command::new(&exe)
        .arg("compile")
        .arg(driver_path)
        .arg("-o")
        .arg(&tmp_wasm)
        .output()
        .map_err(|e| format!("spawn compile: {e}"))?;
    if !compile.status.success() {
        let _ = std::fs::remove_file(&tmp_wasm);
        return Ok("declined".to_string());
    }
    // Run the driver → the rendered `(: (Some (list <u8>…)) …)` / `(: None …)`.
    let run = Command::new(&exe)
        .arg("run")
        .arg(&tmp_wasm)
        .arg("--call")
        .arg("main")
        .output()
        .map_err(|e| format!("spawn run: {e}"));
    let _ = std::fs::remove_file(&tmp_wasm);
    let run = run?;
    if !run.status.success() {
        // The driver (the compiler processing the source) TRAPPED while emitting — e.g. an out-of-range
        // Int64 LITERAL (`- 0 2^63`) overflows the checked arithmetic that embeds it. The program is not
        // expressible by the emit path → `declined`, which AGREES with `run-ml` (the eval-db oracle prints
        // `declined` for the same source). NOT an `Err` (that would exit non-zero — the reserved
        // harness-READ-failure class — and grade harness-broken / NotYet instead of agree-on-declined;
        // breaker + v-compiler-ml W4). This mirrors `compile_run_ml_driver`, which also maps a driver trap
        // to `declined`.
        return Ok("declined".to_string());
    }
    let out = String::from_utf8_lossy(&run.stdout);
    let Some(bytes) = parse_emitted_byte_list(out.trim()) else {
        // `None` (or an unrecognized render) → the program is out of the emit subset → declined.
        return Ok("declined".to_string());
    };
    // Run the emitted CORE module. A returned i64 → value; a trap (div0/mod0/MIN÷-1) → declined (matches
    // the eval-db oracle); an invalid/uninstantiable module → error (a bad artifact = a real emit bug).
    // Run the emitted CORE module through the external `cdz-run` binary (thin-`cdz` seam — the runner holds
    // wasmtime, reached on PATH via `$CDZ_RUN_BIN` → sibling → `$PATH`) rather than linking
    // `cdz_run::run_core_module` in-process. `cdz-run --core-module` prints one verdict line (`value <n>` /
    // `trap` / `error <msg>`); map it to this fn's contract (Value→`value <n>`, Trap→`declined`, Err→`error`),
    // preserving the exact strings run-ml/run-emitted/chor + the fuzzer's differential depend on. A spawn
    // failure is an outer `Err` (the reserved harness-failure class, same as the compile/run spawns above).
    let core_wasm = std::env::temp_dir().join(format!("cdz-core-{}.wasm", std::process::id()));
    if let Err(e) = std::fs::write(&core_wasm, &bytes) {
        return Err(format!("write core module: {e}"));
    }
    let program = locate_plugin("run").unwrap_or_else(|| PathBuf::from(bin_name("cdz-run")));
    let core = Command::new(&program)
        .arg("--core-module")
        .arg(&core_wasm)
        .arg("--core-export")
        .arg("main")
        .output();
    let _ = std::fs::remove_file(&core_wasm);
    let core = core.map_err(|e| format!("spawn cdz-run --core-module: {e}"))?;
    if !core.status.success() {
        return Err(format!(
            "cdz-run --core-module failed: {}",
            String::from_utf8_lossy(&core.stderr).trim()
        ));
    }
    let verdict = String::from_utf8_lossy(&core.stdout);
    let verdict = verdict.trim();
    // `trap` → `declined` (matches the eval-db oracle); `value <n>` and `error <msg>` pass through unchanged.
    Ok(if verdict == "trap" {
        "declined".to_string()
    } else {
        verdict.to_string()
    })
}

/// Parse compiler-ml's `emit-src-bytes` rendered result into the raw module bytes. The render is
/// `(: (Some (list <u8> <u8> …)) (Option (List UInt8)))` for a Some, or `(: None …)` for a decline. The
/// byte list is decimal `u8` integers (0..=255) — lossless through text (no escaping, unlike a `Bytes`
/// `b"…"` render), so we collect them directly. Returns `None` for a `None` result (declined) or any
/// render without a parseable `(list …)` payload.
fn parse_emitted_byte_list(rendered: &str) -> Option<Vec<u8>> {
    // Locate the `(list ` payload of the `Some` (a `None` result has no `(list `). Take up to its close.
    let after = rendered.split("(list").nth(1)?;
    let inner = after.split(')').next()?;
    let mut bytes = Vec::new();
    for tok in inner.split_whitespace() {
        // Every token must be a `u8`; anything else means an unexpected render — bail (treat as declined).
        bytes.push(tok.parse::<u8>().ok()?);
    }
    Some(bytes)
}

#[derive(clap::Args)]
struct RunRustArgs {
    /// The program SOURCE file (s-expr / ml surface). OMITTED → read the program from stdin. Mirrors
    /// `cdz run-ml`'s input contract so the fuzzer's differential harness is symmetric.
    file: Option<String>,
    /// The export to invoke (default: the sole exported nullary `main`). The scalar/nullary case needs
    /// none; a `--call NAME` selects a specific export for a future arg-taking case.
    #[arg(long)]
    call: Option<String>,
}

/// `cdz run-rust` — compile a program to the RUST backend, run it natively, print ONE verdict line.
///
/// The fuzzer's rust-vs-wasm differential ORACLE shells this to get the Rust-backend value and compare it
/// to the wasm value (`cdz run`) — so the render MUST match `cdz-run`'s byte-for-byte (it uses the shared
/// `cdz-rust-render` crate, the same one the corpus gate's `--target rust` path uses). Verdict grammar:
/// `value <sexpr>` (ran to that value — bare, exactly as `cdz-run` prints); `declined` (the front-end
/// REJECTED the program, OR the rust backend doesn't emit it yet — coverage-not-yet, NOT a mismatch the
/// fuzzer files); `trap <msg>` (the program TRAPPED at run time — a Cadenza trap lowered to a Rust panic,
/// compared by reason); `error <msg>` (the emitted `.rs` FAILED to `rustc` — a bad artifact, a MISCOMPILE
/// the fuzzer files). `declined` vs `error` are kept DISTINCT (the fuzzer's one requirement beyond run-ml).
/// Exit is 0 for any RUN outcome (a verdict is not a shell failure); a NON-ZERO exit is a HARNESS/USAGE
/// failure that produced no verdict — a file/stdin READ error, OR a usage mistake (a bad/ambiguous `--call`,
/// or an arg-taking export the nullary driver can't invoke). A harness ENVIRONMENT breakage that occurs
/// mid-run (can't spawn the compiler/rustc) surfaces as an `error <msg>` VERDICT + exit 0, so the oracle
/// always gets a line (Copilot PR #547/#551).
///
/// MECHANISM (mirrors the gate's `run_program_rust`, now that its render half is the shared crate): shell
/// `cdz compile - -o - --target rust` (self, via `current_exe`) to emit the `.rs`; wrap it in `mod prog {…}`
/// and add a driver `fn main` that calls the export and prints `cdz_rust_render::cdz_render_expr(...)`;
/// `rustc -O` it, linking the pre-built `cdz-rt`/`cdz-num` rlibs that sit BESIDE the `cdz` binary in
/// `target/<profile>/` (the same dir `current_exe` lives in); run the binary and map its outcome to a verdict.
fn run_run_rust(args: &RunRustArgs) -> ExitCode {
    // 1. Read the program source (file or stdin, incl. an explicit `-`). A read failure is the reserved
    //    harness-error path.
    let source = match read_verdict_source(args.file.as_deref(), "run-rust") {
        Ok(s) => s,
        Err(()) => return ExitCode::FAILURE,
    };

    // 2. Emit the Rust module: shell `cdz compile - -o - --target rust` to SELF (install-location-independent
    //    via `current_exe`). A compile FAILURE means the front-end rejected the program or the rust backend
    //    declines it → `declined` (coverage-not-yet), NOT an `error` (an error is a bad ARTIFACT — see step 5).
    //    An ENVIRONMENT failure (can't find/spawn self) is surfaced as an `error <msg>` VERDICT on stdout + exit
    //    0, NOT a non-zero shell exit: the fuzzer's oracle always expects a verdict line (the sole non-zero exit
    //    is a source READ failure above), so a harness breakage must not look like a crash (Copilot PR #547).
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => {
            println!("error current_exe: {e}");
            return ExitCode::SUCCESS;
        }
    };
    let module = match emit_rust_module(&exe, &source) {
        EmitOutcome::Module(m) => m,
        EmitOutcome::Declined => {
            println!("declined");
            return ExitCode::SUCCESS;
        }
        EmitOutcome::Harness(msg) => {
            // Couldn't even run the compiler (spawn/temp-write failure) — a harness breakage, surfaced as an
            // `error` verdict + exit 0 so the oracle gets a line rather than a silent non-zero crash.
            println!("error {msg}");
            return ExitCode::SUCCESS;
        }
    };

    // 3. Determine the export to invoke + its Cadenza result type (read off the `// cdz-return[<ident>]:`
    //    note the backend emits). With no `--call`, use the SOLE exported `pub fn`; if the module has
    //    SEVERAL (multiple exports), do NOT guess — require `--call` (Copilot PR #547: splitting on the
    //    first `pub fn` picked an arbitrary export and could run the wrong one).
    let export = match &args.call {
        Some(name) => cdz_rust_render::rust_ident(name),
        None => {
            let names = emitted_pub_fn_names(&module);
            match names.as_slice() {
                [one] => one.clone(),
                [] => {
                    // No exported fn — the backend produced nothing runnable → declined.
                    println!("declined");
                    return ExitCode::SUCCESS;
                }
                many => {
                    eprintln!(
                        "{PROG} run-rust: the program exports {} functions ({}); pass `--call NAME` to \
                         pick one",
                        many.len(),
                        many.join(", ")
                    );
                    return ExitCode::FAILURE;
                }
            }
        }
    };
    // 3b. VALIDATE the chosen export against the emitted module's signature BEFORE building the driver, so
    //     a USAGE problem (a bad `--call` name, or an arg-taking export the nullary driver can't invoke)
    //     is a clean harness error — NOT the `error` verdict, which is reserved for a rust-backend MISCOMPILE
    //     (an emitted `.rs` that fails rustc). Without this, `--call nope` or an arg-taking `main` fell
    //     through to rustc as `error error[E0425]/E0061`, which the fuzzer would file as a spurious miscompile.
    match export_param_arity(&module, &export) {
        Some(0) => {} // a nullary export — the driver can call it directly.
        Some(n) => {
            eprintln!(
                "{PROG} run-rust: export `{export}` takes {n} argument(s); `cdz run-rust` runs only a \
                 NULLARY export (no `--arg` passthrough yet) — pick a nullary export or wrap it"
            );
            return ExitCode::FAILURE;
        }
        None => {
            eprintln!(
                "{PROG} run-rust: no exported `{export}` in the compiled program{}",
                match &args.call {
                    Some(_) => " (check the `--call` name against the program's `(export …)`)",
                    None => "",
                }
            );
            return ExitCode::FAILURE;
        }
    }
    let ret_ty = cdz_rust_render::cdz_return_type(&module, &export);

    // 4. Build a driver: wrap the emitted module in `mod prog {…}` (so its `pub fn main` becomes
    //    `prog::main` and does NOT collide with the driver's own `fn main`), then print the boundary value
    //    rendered by the SHARED crate — byte-identical to what `cdz-run` prints.
    //
    // A DIVERGING export (`cdz-return[export]: !` — a provable-trap program lowers to `pub fn <export>() ->
    // ! { panic!(…) }`) is a special case: its result has type `!`, so there is NOTHING to bind or render —
    // the call itself panics. Emit a driver that just CALLS it (no `let __r`, no `println!`): `prog::main()`
    // diverges → panics → `compile_and_run_rust_driver` maps the panic to the `trap` verdict, matching
    // wasm's clean `trap unreachable`. Without this, run-rust reported `error` for EVERY diverging program:
    // first the post-call render was unreachable (rustc `-D warnings` → hard error), and even silencing that
    // (`#[allow(unreachable_code)]`) then failed because `!`/`()` doesn't `Display` — a `!`-typed result
    // can't be rendered at all. So diverging programs are handled by NOT rendering (breaker + corpus-bugfix,
    // whose fuzzer rust-vs-wasm differential this false-`error` would otherwise poison for every trap case).
    let driver = if ret_ty.as_deref() == Some("!") {
        format!(
            "#[allow(warnings)]\nmod prog {{\n{module}\n}}\nfn main() {{\n    prog::{export}();\n}}\n"
        )
    } else {
        let render = match &ret_ty {
            Some(ty) => {
                let sums = cdz_rust_render::cdz_sum_descriptors(&module);
                let newtypes = cdz_rust_render::cdz_newtype_descriptors(&module);
                let sum_params = cdz_rust_render::cdz_sum_params(&module);
                let qualified_heads = cdz_rust_render::cdz_sum_qualified_heads(&module);
                let unit_form = cdz_rust_render::cdz_unit_form(&module, &export);
                let scale = cdz_rust_render::cdz_scale(&module, &export);
                let qty_at = cdz_rust_render::cdz_qty_at(&module, &export);
                cdz_rust_render::cdz_render_expr(
                    ty,
                    &sums,
                    &newtypes,
                    &sum_params,
                    unit_form.as_deref(),
                    scale,
                    &qty_at,
                    &qualified_heads,
                )
            }
            // No `cdz-return` note (an older/void export) — fall back to Display of the result.
            None => "format!(\"{}\", __r)".to_string(),
        };
        format!(
            "#[allow(warnings)]\nmod prog {{\n{module}\n}}\nfn main() {{\n    let __r = prog::{export}();\n    println!(\"{{}}\", {render});\n}}\n"
        )
    };

    // 5. rustc the driver, linking the pre-built `cdz-rt`/`cdz-num` rlibs beside the `cdz` binary (same
    //    dir `current_exe` is in — `cargo build` puts `libcdz_{rt,num}.rlib` in `target/<profile>/`). A
    //    UNIQUE per-process temp dir (pid-stamped) so concurrent oracle invocations never race prog.rs/prog.
    // A rustc/driver HARNESS failure (couldn't write prog.rs, spawn rustc, or exec the binary) is surfaced
    // as an `error <msg>` VERDICT + exit 0 — the oracle always expects a verdict line; a non-zero shell exit
    // would look like a crash indistinguishable from a real harness break (Copilot PR #547). `compile_and_run_
    // rust_driver` already returns the value/error/trap VERDICT as `Ok`; its `Err` is only a harness break.
    match compile_and_run_rust_driver(&exe, &driver) {
        Ok(verdict) => println!("{verdict}"),
        Err(msg) => println!("error {msg}"),
    }
    ExitCode::SUCCESS
}

/// The names of every top-level `pub fn` the emitted module declares, in source order — used to pick the
/// default export (exactly one → use it; several → require `--call`). A `pub fn <name>(` / `pub fn
/// <name><generics>` at line start (the backend emits each export as a top-level `pub fn`); a nested/inner
/// `pub fn` would not be at column 0, so keying on the line start avoids counting one.
fn emitted_pub_fn_names(module: &str) -> Vec<String> {
    let mut names = Vec::new();
    for line in module.lines() {
        if let Some(rest) = line.strip_prefix("pub fn ") {
            let name = rest.split(['(', '<']).next().unwrap_or("").trim();
            if !name.is_empty() {
                names.push(name.to_string());
            }
        }
    }
    names
}

/// The parameter arity of the emitted `pub fn <export>(…)` — `Some(n)` with `n` the top-level parameter
/// count, or `None` if the module declares no such `pub fn` (a bad `--call` name / nothing runnable).
/// Reads the emitted Rust signature: finds `pub fn <export>` (also `pub fn <export><generics>` for the
/// async form), takes the `(…)` parameter list up to the MATCHING close paren, and counts top-level
/// commas (0 params → arity 0; else commas+1). A conservative textual read of the backend's own output —
/// good enough to tell a nullary export from an arg-taking one and to detect an absent export.
fn export_param_arity(module: &str, export: &str) -> Option<usize> {
    // Find `pub fn <export>` where the name is a whole token (followed by `(` or `<`, not more ident chars).
    let needle = format!("pub fn {export}");
    let mut search_from = 0;
    let after = loop {
        let rel = module.get(search_from..)?.find(&needle)?;
        let idx = search_from + rel + needle.len();
        match module[idx..].chars().next() {
            Some('(') | Some('<') => break &module[idx..],
            // A longer name that merely starts with `export` (e.g. `main2`) — keep searching.
            _ => search_from = idx,
        }
    };
    // Skip any generic list `<…>` (the async form `pub fn f<E: CdzEnv>(…)`) to reach the param `(`.
    let params_open = after.find('(')?;
    let rest = &after[params_open + 1..];
    // Take up to the matching close paren, tracking nesting so a param type like `(i64, i64)` or a
    // generic `Vec<(A, B)>` doesn't end the list early.
    let mut depth = 0i32;
    let mut end = None;
    for (i, c) in rest.char_indices() {
        match c {
            '(' | '<' | '[' => depth += 1,
            ')' if depth == 0 => {
                end = Some(i);
                break;
            }
            ')' | '>' | ']' => depth -= 1,
            _ => {}
        }
    }
    let params = rest[..end?].trim();
    if params.is_empty() {
        return Some(0);
    }
    // Count TOP-LEVEL commas (a comma inside a nested `(…)`/`<…>`/`[…]` is part of one param's type).
    let mut depth = 0i32;
    let mut commas = 0usize;
    for c in params.chars() {
        match c {
            '(' | '<' | '[' => depth += 1,
            ')' | '>' | ']' => depth -= 1,
            ',' if depth == 0 => commas += 1,
            _ => {}
        }
    }
    Some(commas + 1)
}

/// The outcome of emitting a program to the rust backend: the `.rs` module text, a DECLINE (front-end
/// reject / backend not-yet), or a HARNESS failure (couldn't spawn the compiler).
enum EmitOutcome {
    Module(String),
    Declined,
    Harness(String),
}

/// Shell `<exe> compile <src>.sexp -o - --target rust` → the emitted `.rs` on stdout. The source is
/// written to a per-process temp `.sexp` FILE (not piped to stdin `-`, which `cdz compile` reads as a
/// pre-built binary AST — a `.sexp` file's extension selects in-process SOURCE parsing). A non-zero
/// compile exit is a DECLINE (front-end reject or rust-backend not-yet); a spawn/IO failure is a harness error.
fn emit_rust_module(exe: &std::path::Path, source: &str) -> EmitOutcome {
    use std::process::Command;
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("cdz-run-rust-emit-{}-{seq}", std::process::id()));
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return EmitOutcome::Harness(format!("create temp dir: {e}"));
    }
    let _guard = RemoveOnDrop::dir(dir.clone());
    let src = dir.join("prog.sexp");
    if let Err(e) = std::fs::write(&src, source) {
        return EmitOutcome::Harness(format!("write source: {e}"));
    }
    let out = match Command::new(exe)
        .arg("compile")
        .arg(&src)
        .args(["-o", "-", "--target", "rust"])
        .output()
    {
        Ok(o) => o,
        Err(e) => return EmitOutcome::Harness(format!("spawn compile: {e}")),
    };
    if !out.status.success() {
        return EmitOutcome::Declined;
    }
    EmitOutcome::Module(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Write the `driver` to a per-process temp dir, `rustc -O` it (linking the `cdz-rt`/`cdz-num` rlibs that
/// sit beside `exe`), run the binary, and map the outcome to a verdict STRING. A rustc failure is
/// `error <first-stderr-line>` (a bad artifact = a miscompile); a non-zero RUN is `trap <…>` (a panic =
/// a Cadenza trap); success is `value <stdout>` (the rendered boundary value). The temp dir is removed on
/// every return via an RAII guard.
/// Resolve the directory holding cargo's HASHED dependency rlibs, given the dir the `cdz` bin sits in.
/// Normally `<lib_dir>/deps`. But a `cargo test`-built bin ALREADY lives in `.../deps/`, so the deps dir is
/// `lib_dir` ITSELF — appending `deps` there gives `.../deps/deps` (nonexistent), which is the PR#772 bug
/// (the hashed `libcdz_num-<hash>.rlib` search dir goes missing → E0433). Detect the already-in-`deps` case
/// by the dir's own name so we never double-append.
fn resolve_deps_dir(lib_dir: &std::path::Path) -> std::path::PathBuf {
    if lib_dir.file_name().is_some_and(|n| n == "deps") {
        lib_dir.to_path_buf()
    } else {
        lib_dir.join("deps")
    }
}

/// The `run-rust` backend-rlib search ROOTS, in priority order: the `CDZ_RUST_RLIB_DIR` override (when
/// set) FIRST, then the exe-relative `lib_dir`. The nix `cdz` package sets the override because its
/// `bin/` ships NO rlibs beside the exe (so the exe-relative search alone finds none → `E0433 cannot
/// find crate cdz_num` on every `run-rust`); a plain `cargo build`/`cargo xtask build` leaves it unset
/// and keeps the exe-relative behavior (the rlibs sit beside the `cdz` bin). Pure (the override is
/// passed in, not read from the env) so the precedence is unit-testable.
fn rust_rlib_search_roots(
    lib_dir: &std::path::Path,
    override_dir: Option<std::path::PathBuf>,
) -> Vec<std::path::PathBuf> {
    let mut roots: Vec<std::path::PathBuf> = Vec::new();
    if let Some(d) = override_dir {
        roots.push(d);
    }
    roots.push(lib_dir.to_path_buf());
    roots
}

/// Locate a backend dependency rlib (`cdz_rt`/`cdz_num`) for the `run-rust` link. Prefer the PLAIN
/// top-level `lib<crate>.rlib` in `lib_dir` (a `cargo build`-built workspace has it beside the `cdz` bin);
/// else fall back to the NEWEST hashed `lib<crate>-<hash>.rlib` in `deps_dir` (what `cargo test` produces —
/// the plain name is often absent there, only the hashed one). `deps_dir` is the caller's resolved
/// hashed-rlib dir (`lib_dir/deps`, OR `lib_dir` itself when the bin already lives in `deps/`). Newest-by-
/// mtime so a rebuild's fresh artifact wins over a stale one. `None` if neither exists (the caller then
/// omits the `--extern`, as before — a program that references the crate then fails rustc loudly, which is
/// strictly better than a silent wrong link).
fn find_backend_rlib(
    lib_dir: &std::path::Path,
    deps_dir: &std::path::Path,
    crate_name: &str,
) -> Option<std::path::PathBuf> {
    let plain = lib_dir.join(format!("lib{crate_name}.rlib"));
    if plain.exists() {
        return Some(plain);
    }
    // deps/lib<crate>-<hash>.rlib — pick the most recently modified match.
    let prefix = format!("lib{crate_name}-");
    std::fs::read_dir(deps_dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| {
            let n = e.file_name();
            let n = n.to_string_lossy();
            n.starts_with(&prefix) && n.ends_with(".rlib")
        })
        .max_by_key(|e| {
            e.metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
        })
        .map(|e| e.path())
}

fn compile_and_run_rust_driver(exe: &std::path::Path, driver: &str) -> Result<String, String> {
    use std::process::Command;
    let lib_dir = exe
        .parent()
        .ok_or_else(|| "cannot locate the cdz binary's directory".to_string())?;
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("cdz-run-rust-{}-{seq}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let _guard = RemoveOnDrop::dir(dir.clone());
    let src = dir.join("prog.rs");
    let bin = dir.join("prog");
    std::fs::write(&src, driver).map_err(|e| format!("write driver: {e}"))?;

    // rustc, linking the pre-built rlibs beside the cdz binary (built by `cargo build`/`cargo xtask
    // build` into `target/<profile>/`). `--extern` only makes a crate available (not force-linked), so
    // passing both is harmless when the program references neither.
    let mut cmd = Command::new("rustc");
    cmd.args(["-O", "--edition", "2021"])
        .arg(&src)
        .arg("-o")
        .arg(&bin);
    // rlib search ROOTS, in priority order. `CDZ_RUST_RLIB_DIR` (set by the nix `cdz` package — whose
    // `bin/` ships NO rlibs beside the exe, so the exe-relative search alone finds none and every
    // `run-rust` fails `E0433 cannot find crate cdz_num`) is searched FIRST when present; the exe-relative
    // `lib_dir` (a `cargo build` / `cargo xtask build` workspace has the rlibs beside the `cdz` bin) is the
    // fallback, so a plain cargo build is unaffected. Each root also contributes its resolved `deps/`
    // (cargo's hashed-rlib dir); deduped, `-L`'d for every existing dir.
    let roots = rust_rlib_search_roots(
        lib_dir,
        std::env::var_os("CDZ_RUST_RLIB_DIR").map(std::path::PathBuf::from),
    );
    // The `-L dependency=` search path: each root + its `deps/`. `resolve_deps_dir` handles a root that is
    // ITSELF `.../deps` (a `cargo test`-located bin): it stays `deps`, never `deps/deps` (PR#772 review).
    let mut search_dirs: Vec<std::path::PathBuf> = Vec::new();
    for root in &roots {
        for d in [root.clone(), resolve_deps_dir(root)] {
            if d.is_dir() && !search_dirs.contains(&d) {
                search_dirs.push(d);
            }
        }
    }
    for d in &search_dirs {
        cmd.arg("-L").arg(format!("dependency={}", d.display()));
    }
    // Locate each backend rlib ROBUSTLY across the roots (override first, then exe-relative): the plain
    // top-level `lib<crate>.rlib` (a `cargo build` workspace) OR the newest `deps/lib<crate>-<hash>.rlib`
    // (what `cargo test` produces — the plain name may be ABSENT there). EVERY emitted program references
    // `cdz_num` (the always-emitted `Ast` enum, since Ast.Int carries a `cdz_num::Big`), so a missing
    // rlib means a cryptic `E0433 cannot find crate cdz_num` — find-either-anywhere fixes it wherever the
    // artifact lives. `--extern` only MAKES a crate available (not force-linked), so naming an unused one
    // stays harmless.
    //
    // The full set MIRRORS the corpus rust-exec grader (`cdz-rust-run`'s `compile_and_run`): a native
    // VALUE-ENCODE program references `cadenza_ast` (the AST builder) + `num_bigint` (IntValue bridge), and a
    // runtime `String.concat`/`from-bytes` NFC-normalizes via `unicode_normalization` (the `Core::NfcNormalize`
    // emit, FINDING #23 rust parity). Those three are STAGED beside `cdz_rt`/`cdz_num` in the nix
    // `CDZ_RUST_RLIB_DIR` rlib set (`cadenza_ast` plain top-level; `num_bigint` + `unicode_normalization`
    // hashed in its `deps/`, pulled in via cadenza-ast's `std` feature), so `find_backend_rlib`'s
    // plain-then-hashed search resolves each. Without them a `cdz run-rust` differential run of such a program
    // failed `E0433 cannot find crate …` where the corpus grader (which links the full set) passed.
    for crate_name in [
        "cdz_rt",
        "cdz_num",
        "cadenza_ast",
        "num_bigint",
        "unicode_normalization",
    ] {
        if let Some(rlib) = roots
            .iter()
            .find_map(|root| find_backend_rlib(root, &resolve_deps_dir(root), crate_name))
        {
            cmd.arg("--extern")
                .arg(format!("{crate_name}={}", rlib.display()));
        }
    }
    let compile = cmd
        .output()
        .map_err(|e| format!("rustc failed to launch: {e}"))?;
    if !compile.status.success() {
        // The emitted `.rs` did not compile — a BAD ARTIFACT (a rust-backend miscompile the fuzzer files),
        // NOT a decline. Report the first stderr line (often the root error).
        let stderr = String::from_utf8_lossy(&compile.stderr);
        let first = stderr
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("rustc failed")
            .trim();
        return Ok(format!("error {first}"));
    }
    // Run the compiled driver. A non-zero exit (a panic) is a TRAP (a Cadenza trap lowered to a Rust
    // panic); the panic MESSAGE is the reason.
    let run = Command::new(&bin)
        .output()
        .map_err(|e| format!("run failed to launch: {e}"))?;
    if !run.status.success() {
        let stderr = String::from_utf8_lossy(&run.stderr);
        return Ok(format!("trap {}", panic_reason(&stderr)));
    }
    let value = String::from_utf8_lossy(&run.stdout).trim().to_string();
    Ok(format!("value {value}"))
}

/// Extract a DETERMINISTIC panic reason from a Rust panic's stderr — the trap message the differential
/// oracle compares. Rust prints `thread 'main' panicked at <FILE>:<LINE>:<COL>:` followed by the payload
/// message on the NEXT line. The `<FILE>:<LINE>:<COL>` is a per-run temp path (`/tmp/cdz-run-rust-…/prog.rs`),
/// so returning THAT line makes the reason vary run-to-run (Copilot PR #547). Return the payload MESSAGE
/// (the line after "panicked at …") instead — stable across runs and the actual reason. Falls back to the
/// first non-empty line if the format is unexpected.
fn panic_reason(stderr: &str) -> String {
    let lines: Vec<&str> = stderr.lines().collect();
    // Modern format: the line AFTER `… panicked at …:` is the payload message.
    if let Some(i) = lines.iter().position(|l| l.contains("panicked at")) {
        if let Some(msg) = lines.get(i + 1) {
            let m = msg.trim();
            if !m.is_empty() {
                return m.to_string();
            }
        }
        // Older format: `… panicked at '<payload>', <file>:<line>` — the quoted payload AFTER "panicked
        // at" (search from there, so the `'` in `thread 'main'` before it isn't mistaken for the open quote).
        if let Some(pa) = lines[i].find("panicked at") {
            let tail = &lines[i][pa..];
            if let Some(start) = tail.find('\'')
                && let Some(end) = tail.rfind('\'')
                && end > start
            {
                return tail[start + 1..end].to_string();
            }
        }
    }
    lines
        .iter()
        .map(|l| l.trim())
        .find(|l| !l.is_empty())
        .unwrap_or("panic")
        .to_string()
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
/// Dispatch a PURE ARTIFACTS-IN `cdz compile` (no source-file input): spawn `cdz-compile` under the
/// `delegate-compile` feature, else run the compiler in-process (`compiler_cli::run`, byte-for-byte the
/// standalone `rcdzc` bin's behavior). One seam so the two builds stay behavior-identical.
fn dispatch_compile_args(args: compile_args::CompileArgs) -> ExitCode {
    #[cfg(not(feature = "standalone"))]
    {
        delegate::delegate_args(&args, PROG)
    }
    #[cfg(feature = "standalone")]
    {
        // The front-end owns arg parsing (cdz-local `CompileArgs`); the in-process compiler is reached
        // through `run_with_specs` (the parsed-values core), NOT this crate's private `CompileArgs`.
        compiler_cli::run_with_specs(
            args.input_specs(),
            &args.targets(),
            args.out_path(),
            args.entry(),
            args.export(),
            args.component_name(),
            args.opt_level(),
            args.overflow_spec(),
            args.emit_diagnostics(),
            PROG,
        )
    }
}

/// Dispatch a compile of already-prepared input artifacts (the source-file path): spawn `cdz-compile`
/// under the `delegate-compile` feature (materializing the artifacts to temp files), else run the
/// compiler in-process (`compiler_cli::run_prepared`). The delegated path is behavior-identical because
/// `cdz-compile` runs the same `run_prepared` over the same artifacts (located diagnostics included).
fn dispatch_compile_prepared(
    inputs: Vec<cadenza_compile_abi::Artifact>,
    targets: &[cadenza_compile_abi::Target],
    out: Option<PathBuf>,
    opt_level: cadenza_compile_abi::OptLevel,
    overflow: cadenza_compile_abi::OverflowSpec,
) -> ExitCode {
    #[cfg(not(feature = "standalone"))]
    {
        delegate::delegate_from_artifacts(
            &inputs,
            targets,
            out.as_deref(),
            opt_level,
            overflow,
            PROG,
        )
    }
    #[cfg(feature = "standalone")]
    {
        compiler_cli::run_prepared_with_overflow(
            inputs, targets, out, opt_level, overflow, None, PROG,
        )
    }
}

fn run_compile(args: compile_args::CompileArgs) -> ExitCode {
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
        return dispatch_compile_args(args);
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
    let mut inputs: Vec<cadenza_compile_abi::Artifact> = Vec::new();
    for spec in &specs {
        if is_source_file(spec) {
            // Parse the source in-process, keeping the span table (the whole-program form, as the gate
            // and the semantic queries use).
            let (source, arenas, spantable) = load_spanned_or_bail!(spec);
            let name = program_name(spec);
            inputs.push(cadenza_compile_abi::Artifact::new(
                cadenza_compile_abi::Artifact::KIND_AST,
                name.clone(),
                cadenza_syntax::codec::encode(&arenas),
            ));
            {
                let span_data = span_data_of(spec, &source, &spantable);
                inputs.push(cadenza_compile_abi::Artifact::new(
                    cadenza_compile_abi::spans::KIND_SPANS,
                    name,
                    cadenza_compile_abi::spans::encode(&span_data),
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
        inputs.push(cadenza_compile_abi::abi::entry_artifact(entry));
    } else if let Some(entry) = &entry_from_closure {
        inputs.push(cadenza_compile_abi::abi::entry_artifact(entry));
    }
    // A `--component-name <INTERFACE>` names the interface a cross-component PROVIDER publishes its exports
    // under — inject it as a `KIND_COMPONENT_NAME` artifact (X4b), same as the artifacts-in `run` path.
    if let Some(iface) = args.component_name() {
        inputs.push(cadenza_compile_abi::abi::component_name_artifact(iface));
    }
    // Thread the requested `--opt-level` (default `O1`) + `--overflow-signed`/`--overflow-unsigned`
    // (default none) through to the compile — `cdz compile --opt-level O2 --overflow-signed wrap foo.cdz`
    // selects the tier + the global overflow policy, same as the artifacts-in `rcdzc` path.
    dispatch_compile_prepared(
        inputs,
        &targets,
        args.out_path(),
        args.opt_level(),
        args.overflow_spec(),
    )
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
    targets: &[cadenza_compile_abi::Target],
    opt_level: cadenza_compile_abi::OptLevel,
    overflow: cadenza_compile_abi::OverflowSpec,
) -> ExitCode {
    let mut inputs: Vec<cadenza_compile_abi::Artifact> = Vec::new();
    for spec in specs {
        let (source, arenas, spantable) = load_spanned_or_bail!(spec);
        let name = program_name(spec);
        inputs.push(cadenza_compile_abi::Artifact::new(
            cadenza_compile_abi::Artifact::KIND_AST,
            name.clone(),
            cadenza_syntax::codec::encode(&arenas),
        ));
        let span_data = span_data_of(spec, &source, &spantable);
        inputs.push(cadenza_compile_abi::Artifact::new(
            cadenza_compile_abi::spans::KIND_SPANS,
            name,
            cadenza_compile_abi::spans::encode(&span_data),
        ));
    }
    if let Some(entry) = entry {
        inputs.push(cadenza_compile_abi::abi::entry_artifact(entry));
    }
    // `run_prepared` applies the `[Wasm]` default when `targets` is empty, matching a bare `cdz compile`.
    // `opt_level` is the resolved build tier; `overflow` the resolved global overflow policy.
    dispatch_compile_prepared(inputs, targets, out, opt_level, overflow)
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
    let targets = [cadenza_compile_abi::Target::from(args.target)];
    compile_source_specs(
        &project.specs,
        Some(&project.entry_name),
        args.out.clone(),
        &targets,
        opt_level,
        manifest_overflow_spec(&project.m),
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
    opt_level: cadenza_compile_abi::OptLevel,
    overflow: cadenza_compile_abi::OverflowSpec,
) -> Result<Option<Vec<u8>>, ()> {
    compile_project_component_bytes_named(specs, entry, opt_level, overflow, None)
}

/// [`compile_project_component_bytes`] plus an optional `--component-name` — the interface a
/// cross-component PROVIDER publishes its exports under (`cadenza:<pkg>/<iface>`). A path-dep is compiled
/// with its interface name so the consumer can peer-bind it; a plain `cdz run`/`cdz build` passes `None`.
fn compile_project_component_bytes_named(
    specs: &[String],
    entry: &str,
    opt_level: cadenza_compile_abi::OptLevel,
    overflow: cadenza_compile_abi::OverflowSpec,
    component_name: Option<&str>,
) -> Result<Option<Vec<u8>>, ()> {
    let mut inputs: Vec<cadenza_compile_abi::Artifact> = Vec::new();
    for spec in specs {
        let (source, arenas, spantable) = match load_program_spanned(spec) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("{PROG}: {e}");
                return Err(());
            }
        };
        let name = program_name(spec);
        inputs.push(cadenza_compile_abi::Artifact::new(
            cadenza_compile_abi::Artifact::KIND_AST,
            name.clone(),
            cadenza_syntax::codec::encode(&arenas),
        ));
        let span_data = span_data_of(spec, &source, &spantable);
        inputs.push(cadenza_compile_abi::Artifact::new(
            cadenza_compile_abi::spans::KIND_SPANS,
            name,
            cadenza_compile_abi::spans::encode(&span_data),
        ));
    }
    inputs.push(cadenza_compile_abi::abi::entry_artifact(entry));
    // A path-dep publishes its exports under an interface name (the same `--component-name` a manual
    // cross-component provider uses), so the consumer's `--peer <iface>=<path>` binds it by that name.
    if let Some(iface) = component_name {
        inputs.push(cadenza_compile_abi::abi::component_name_artifact(iface));
    }
    dispatch_project_to_bytes(inputs, opt_level, overflow)
}

/// Compile prepared project `inputs` to the wasm COMPONENT BYTES in-memory: spawn `cdz-compile` under
/// `!standalone` (capturing its `-o -` stdout), else run the compiler in-process. One seam so the quiet
/// build (`cdz run`/`test`) stays behavior-identical across the two builds. `Ok(Some(bytes))` = a
/// component was produced; `Ok(None)` = compiled but none; `Err(())` = a reported compile failure.
fn dispatch_project_to_bytes(
    inputs: Vec<cadenza_compile_abi::Artifact>,
    opt_level: cadenza_compile_abi::OptLevel,
    overflow: cadenza_compile_abi::OverflowSpec,
) -> Result<Option<Vec<u8>>, ()> {
    #[cfg(not(feature = "standalone"))]
    {
        delegate::delegate_project_to_bytes(&inputs, opt_level, overflow, PROG)
    }
    #[cfg(feature = "standalone")]
    {
        // Compile on the compiler-stack worker (deep-recursion guard), same as `check_one`/`run_prepared`.
        let out = rcdzc::run_with_compiler_stack(|| {
            rcdzc::compile_with_opt_and_overflow(
                &inputs,
                &[cadenza_compile_abi::Target::Wasm],
                opt_level,
                overflow,
            )
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

/// Whether the `cdz run` component arg names a SOURCE file (`.cdz`/`.ml`/`.sexp`/`.sexpr`) rather than a
/// compiled `.wasm` component — the common `cdz run foo.sexp` mistake. Checked AFTER `run_target_is_project`
/// (a dir/`Project.cdz` is a project, not a loose source) and only for a real path arg (`None`/`-` stdin
/// are not source-file specs). Reuses [`is_source_file`]'s extension set.
fn run_arg_is_source_file(component: Option<&std::path::Path>) -> bool {
    component
        .and_then(|p| p.to_str())
        .is_some_and(|s| s != "-" && is_source_file(s))
}

/// `cdz run <project>` — BUILD the project's manifest entry, then RUN the produced component (the `cargo
/// run` analogue). Resolves the same `Project.cdz` as `cdz build` (via [`resolve_project_specs`]),
/// compiles the entry (+ modules) to component bytes IN-MEMORY (quiet — no `cdz: wrote …` notice, so a
/// project run doesn't leak its internal temp path), writes them to a temp `.wasm` in the manifest dir for
/// the runner, then delegates to the same `cdz-run` code path the direct `cdz run <file>` uses — passing
/// through `--call`/`--arg`/`--store`/`--host-response`/`--peer` unchanged. The temp is removed after.
/// Reconstruct the argv to hand the external `cdz-run` binary from a parsed [`cdz_run::cli::RunArgs`] —
/// the thin-`cdz` seam for forwarding `run_project`'s run-step (which runs a freshly-built component) to the
/// runner binary instead of linking `cdz_run::cli::run` in-process. `cdz-run` `#[command(flatten)]`s the SAME
/// `RunArgs`, so a faithful argv re-parses into an identical struct → behavior-preserving. Emits ONLY the
/// RUN-relevant fields; the PROJECT-build fields (`release`/`opt_level`) and GRADE-mode fields
/// (`grade`/`compile_status`/`compile_diag`/`component_name`/`baseline`) are intentionally omitted — they do
/// not affect running an already-built `.wasm` (a `cdz run <project>` never carries the grade flags, and the
/// build tier was already consumed to produce the component). The mapping is pinned by a unit test.
fn cdz_run_forward_argv(a: &cdz_run::cli::RunArgs) -> Vec<String> {
    use cdz_run::cli::OutputFormat;
    let mut v: Vec<String> = Vec::new();
    if let Some(c) = &a.component {
        v.push(c.to_string_lossy().into_owned());
    }
    if let Some(call) = &a.call {
        v.push("--call".into());
        v.push(call.clone());
    }
    for arg in &a.args {
        v.push("--arg".into());
        v.push(arg.clone());
    }
    if a.call_twice {
        v.push("--call-twice".into());
    }
    for ta in &a.then_args {
        v.push("--then-arg".into());
        v.push(ta.clone());
    }
    if a.drop_handle {
        v.push("--drop-handle".into());
    }
    if let Some(m) = &a.call_member {
        v.push("--call-member".into());
        v.push(m.clone());
    }
    v.push("--format".into());
    v.push(
        match a.format {
            OutputFormat::Sexp => "sexp",
            OutputFormat::BinaryAst => "binary-ast",
        }
        .into(),
    );
    if let Some(rt) = &a.runtime {
        v.push("--runtime".into());
        v.push(rt.to_string_lossy().into_owned());
    }
    if let Some(st) = &a.store {
        v.push("--store".into());
        v.push(st.to_string_lossy().into_owned());
    }
    for hr in &a.host_responses {
        v.push("--host-response".into());
        v.push(hr.clone());
    }
    for p in &a.peers {
        v.push("--peer".into());
        v.push(p.clone());
    }
    if a.report_live_objects {
        v.push("--report-live-objects".into());
    }
    v
}

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
        manifest_overflow_spec(&project.m),
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
    // PATH DEPENDENCIES: build each `def deps` sibling project to its own component (published under
    // `cadenza:<dep>/api`) and hand them to the runner as PEERS — `run_with_peers` composes them with the
    // consumer in one wasmtime store (v-peer-linking's cross-component binding). A build/resolve failure
    // is reported and aborts the run; the temp dep components are cleaned up afterward.
    let dep_peers = match build_path_deps(&project, opt_level, manifest_overflow_spec(&project.m)) {
        Ok(p) => p,
        Err(()) => {
            let _ = std::fs::remove_file(&out_wasm);
            return ExitCode::FAILURE;
        }
    };
    // Run the freshly-built component through the SAME `cdz-run` path as a direct `cdz run <file>`: clone
    // the parsed args, point `component` at the built wasm, and append each dep as a `--peer iface=path`
    // (the other flags pass through unchanged). A CLI-given `--peer` is preserved; deps are added on top.
    let mut run_args = args.clone();
    run_args.component = Some(out_wasm.clone());
    for (iface, path) in &dep_peers {
        run_args.peers.push(format!("{iface}={}", path.display()));
    }
    // Run the built component through the external `cdz-run` binary (thin-`cdz` seam — the runner holds
    // wasmtime, reached on PATH via `$CDZ_RUN_BIN` → sibling → `$PATH`) rather than linking
    // `cdz_run::cli::run` in-process. `cdz-run` re-parses the reconstructed argv into the same `RunArgs`,
    // so this is behavior-preserving; v-nix injects `$CDZ_RUN_BIN` at the seed `cdz run` sites (#5115).
    let program = locate_plugin("run").unwrap_or_else(|| PathBuf::from(bin_name("cdz-run")));
    let code = passthrough_status(&program, &cdz_run_forward_argv(&run_args), "cdz-run");
    let _ = std::fs::remove_file(&out_wasm); // best-effort cleanup of the temp artifact
    for (_iface, path) in &dep_peers {
        let _ = std::fs::remove_file(path); // clean up each temp dep component
    }
    code
}

/// Build each PATH DEPENDENCY of `project` to its own component and return the `(interface, temp-wasm-path)`
/// peer list `run_project` hands to the runner. For each `def deps` entry (a sibling project dir), resolve
/// its `Project.cdz`, compile its entry to a component published under `cadenza:<dep-name>/api`, and write
/// that to a pid-stamped temp `.wasm` beside the dep's manifest. The dep is built at the SAME opt tier as
/// the consumer so they pin the same value-heap runtime hash (a prerequisite for `run_with_peers` to
/// compose them — v-peer-linking's shared-runtime rule). Returns `Err(())` (diagnostics already printed) on
/// any dep-resolve/build failure; the caller cleans up whatever temps were produced.
fn build_path_deps(
    project: &ProjectSpecs,
    opt_level: cadenza_compile_abi::OptLevel,
    overflow: cadenza_compile_abi::OverflowSpec,
) -> Result<Vec<(String, std::path::PathBuf)>, ()> {
    // Delegate to the fallible core; on ANY error, clean up the temp dep components already written so a
    // mid-loop failure (dep N fails to build) doesn't leak deps 1..N-1's `.cdz-run-dep-*` files (the
    // caller only cleans the CONSUMER's temp). Cleanup is best-effort — the error is already reported.
    let mut peers = Vec::new();
    match build_path_deps_into(project, opt_level, overflow, &mut peers) {
        Ok(()) => Ok(peers),
        Err(()) => {
            for (_iface, path) in &peers {
                let _ = std::fs::remove_file(path);
            }
            Err(())
        }
    }
}

/// The fallible core of [`build_path_deps`]: push each built dep's `(interface, temp-wasm)` into `peers`.
/// Kept separate so the wrapper can clean up `peers`' temps on an error return (no leak on mid-failure).
fn build_path_deps_into(
    project: &ProjectSpecs,
    opt_level: cadenza_compile_abi::OptLevel,
    overflow: cadenza_compile_abi::OverflowSpec,
    peers: &mut Vec<(String, std::path::PathBuf)>,
) -> Result<(), ()> {
    if project.m.deps.is_empty() {
        return Ok(());
    }
    let manifest_dir = project
        .mpath
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    for dep in &project.m.deps {
        // Today the only dep source is a PATH; a future registry source would branch here to fetch+build
        // instead of resolving a sibling dir. (The `DepSource` enum is what lets that slot in later.) The
        // explicit `match` — not a `let`-destructure — is deliberate: adding a `Registry` variant makes
        // THIS arm non-exhaustive, forcing a compile error here so the resolve path can't silently ignore
        // the new source. (clippy's single-pattern suggestion would erase that guard.)
        #[allow(clippy::infallible_destructuring_match)]
        let dep_path = match dep {
            DepSource::Path(p) => p,
        };
        // Resolve the dep dir RELATIVE to the consumer's manifest dir (so `../lib` means a sibling of the
        // consumer, not of the cwd).
        let dep_dir = manifest_dir.join(dep_path);
        let dep_specs =
            match resolve_project_specs(Some(&dep_dir.to_string_lossy()), "cdz run (dep)") {
                Ok(s) => s,
                Err(_) => {
                    eprintln!("{PROG}: dependency `{dep_path}`: could not resolve its Project.cdz");
                    return Err(());
                }
            };
        // The published interface name: `cadenza:<dep-name>/api`, where <dep-name> is the dep's manifest
        // `name` (falling back to its directory name). This is the ONE string both sides agree on — the
        // consumer's source binds `(bind E "cadenza:<dep-name>/api")` and we pass it as the peer key.
        let dep_name = dep_specs.m.name.clone().unwrap_or_else(|| {
            dep_dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("dep")
                .to_string()
        });
        // The dep name becomes a COMPONENT-MODEL interface SEGMENT (`cadenza:<dep-name>/api`), which admits
        // only lowercase ASCII letters/digits/hyphens (the kebab convention). A `name` with a space or other
        // out-of-alphabet char (e.g. `def name = "my lib"`) would build a malformed interface string and
        // fail OPAQUELY deep in wasmtime at compose. Reject it HERE with a clear diagnostic naming the dep +
        // the offending name + the rule — the same "name the real problem before an opaque downstream error"
        // shape as the collision check just below. (Empty is also invalid — an interface segment needs ≥1
        // char.)
        let name_ok = !dep_name.is_empty()
            && dep_name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if !name_ok {
            eprintln!(
                "{PROG}: dependency `{dep_path}` has a `name` (`{dep_name}`) that is not a valid interface \
                 segment — a dependency's `def name` becomes `cadenza:<name>/api`, so it must be lowercase \
                 ASCII letters, digits, and hyphens only (e.g. `my-lib`, not `{dep_name}`)"
            );
            return Err(());
        }
        let iface = format!("cadenza:{dep_name}/api");
        // Two deps publishing the SAME interface (same `name`) would collide — the runner binds each peer
        // under its interface name, so a duplicate is an opaque wasmtime-linker failure at compose. Detect
        // it HERE with a clear diagnostic naming the clashing deps, before building/composing.
        if peers.iter().any(|(existing, _)| *existing == iface) {
            eprintln!(
                "{PROG}: two dependencies publish the same interface `{iface}` (dependency `{dep_path}` \
                 and an earlier one share the name `{dep_name}`) — give each dependency a distinct \
                 `def name` in its `Project.cdz` so their peer interfaces don't collide"
            );
            return Err(());
        }
        let dep_bytes = match compile_project_component_bytes_named(
            &dep_specs.specs,
            &dep_specs.entry_name,
            opt_level,
            overflow,
            Some(&iface),
        ) {
            Ok(Some(b)) => b,
            Ok(None) => {
                eprintln!("{PROG}: dependency `{dep_path}` built no runnable component");
                return Err(());
            }
            Err(()) => return Err(()),
        };
        let dep_out_dir = dep_specs
            .mpath
            .parent()
            .map(std::path::Path::to_path_buf)
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        let dep_wasm = dep_out_dir.join(format!(
            ".cdz-run-dep-{}-{}.wasm",
            dep_specs.entry_name,
            std::process::id()
        ));
        if let Err(e) = std::fs::write(&dep_wasm, &dep_bytes) {
            eprintln!(
                "{PROG}: writing dependency component {}: {e}",
                dep_wasm.display()
            );
            return Err(());
        }
        peers.push((iface, dep_wasm));
    }
    Ok(())
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
        Ok(Some((mpath, m))) => {
            // A non-string `name` was silently dropped to None; unlike a required field, it has a safe
            // fallback (the manifest's directory name for the published `cadenza:<name>/api` interface), so
            // WARN (the declared name is being ignored) and continue rather than fail.
            if m.name_malformed {
                eprintln!(
                    "{PROG}: warning: {}: `name` is not a string (expected `def name = \"my-lib\"`) — \
                     ignoring it and using the directory name for the project/interface name",
                    mpath.display()
                );
            }
            // A non-string `opt-level` was silently dropped to None; unlike a required field, it has a safe
            // default, so WARN (the declared setting is being ignored) and continue rather than fail.
            if m.opt_level_malformed {
                eprintln!(
                    "{PROG}: warning: {}: `opt-level` is not a string (expected `def opt-level = \"O2\"`, \
                     one of O0/O1/O2/O3) — ignoring it and using the default tier",
                    mpath.display()
                );
            }
            // An invalid `overflow-signed`/`overflow-unsigned` (wrong type or a value outside {trap, wrap})
            // was dropped to None; it has a safe default (`trap`), so WARN the declared policy is ignored.
            if m.overflow_signed_malformed {
                eprintln!(
                    "{PROG}: warning: {}: `overflow-signed` is not one of \"trap\"/\"wrap\" (e.g. \
                     `def overflow-signed = \"wrap\"`) — ignoring it and using the default `trap`",
                    mpath.display()
                );
            }
            if m.overflow_unsigned_malformed {
                eprintln!(
                    "{PROG}: warning: {}: `overflow-unsigned` is not one of \"trap\"/\"wrap\" (e.g. \
                     `def overflow-unsigned = \"wrap\"`) — ignoring it and using the default `trap`",
                    mpath.display()
                );
            }
            if !m.duplicate_keys.is_empty() {
                // Last-wins silently discards the earlier `def` — warn so a duplicated `entry`/`opt-level`/…
                // (which can quietly change what builds) isn't a surprise.
                eprintln!(
                    "{PROG}: warning: {}: manifest declares {} more than once — the LAST value wins, the \
                     earlier one(s) are ignored",
                    mpath.display(),
                    m.duplicate_keys
                        .iter()
                        .map(|k| format!("`{k}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            Ok((dir, mpath, m))
        }
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
        if m.entry_malformed {
            // `def entry` IS present but its value isn't a string — name the real problem (wrong type),
            // not "no entry" (which would tell the user to add an entry they already wrote).
            eprintln!(
                "{PROG}: {}: `entry` must be a string naming the boundary file (e.g. \
                 `def entry = \"main.cdz\"`), not a number/other value",
                mpath.display()
            );
        } else {
            eprintln!(
                "{PROG}: {}: the manifest declares no `entry` (add `def entry = \"<file>\"` naming the \
                 component's boundary file)",
                mpath.display()
            );
        }
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
) -> Result<cadenza_compile_abi::OptLevel, String> {
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
) -> Result<cadenza_compile_abi::OptLevel, String> {
    use std::str::FromStr;
    if let Some(s) = flag_opt_level {
        return cadenza_compile_abi::OptLevel::from_str(s)
            .map_err(|e| format!("--opt-level `{s}`: {e}"));
    }
    if let Some(s) = manifest_opt_level {
        return cadenza_compile_abi::OptLevel::from_str(s)
            .map_err(|e| format!("{}: `opt-level` `{s}`: {e}", mpath.display()));
    }
    if release {
        return Ok(cadenza_compile_abi::OptLevel::O2);
    }
    Ok(cadenza_compile_abi::OptLevel::default())
}

/// The GLOBAL overflow policy a `Project.cdz` manifest declares, as the `cadenza_compile_abi::OverflowSpec` the
/// compiler seeds `db.global_overflow` with. Precedence (numeric-model.md §Overflow): a module
/// `(pragma overflow …)` overrides this global manifest default, which overrides the built-in `Trap`.
/// Reads the validated `def overflow-signed`/`overflow-unsigned` fields (#5290): a valid `"trap"`/`"wrap"`
/// maps to the matching mode; an ABSENT or MALFORMED field is `None` (that signedness falls through to
/// the built-in `Trap` — a malformed value was already WARNED at parse and uses the default, so `None`
/// matches). No CLI-flag precedence here: a project build (`cdz run`/`build`) has no `--overflow` flag,
/// so the manifest IS the global level.
fn manifest_overflow_spec(m: &Manifest) -> cadenza_compile_abi::OverflowSpec {
    fn mode(field: &Option<String>, malformed: bool) -> Option<cadenza_compile_abi::OverflowMode> {
        if malformed {
            return None;
        }
        match field.as_deref() {
            Some("trap") => Some(cadenza_compile_abi::OverflowMode::Trap),
            Some("wrap") => Some(cadenza_compile_abi::OverflowMode::Wrap),
            _ => None,
        }
    }
    cadenza_compile_abi::OverflowSpec {
        signed: mode(&m.overflow_signed, m.overflow_signed_malformed),
        unsigned: mode(&m.overflow_unsigned, m.overflow_unsigned_malformed),
    }
}

// ── project metadata ─────────────────────────────────────────────────────────────────────────────

#[derive(clap::Args)]
struct MetadataArgs {
    /// The project to describe: a `Project.cdz` manifest, or a DIRECTORY holding one. OMITTED → search up
    /// from the current directory for the nearest `Project.cdz` (like `cdz build`/`cdz test`).
    dir: Option<String>,
}

#[derive(clap::Args)]
struct TreeArgs {
    /// The project whose dependency tree to print: a `Project.cdz` manifest, or a DIRECTORY holding one.
    /// OMITTED → search up from the current directory for the nearest `Project.cdz` (like `cdz build`).
    dir: Option<String>,
    /// Emit the tree as a nested JSON object instead of the box-drawing text — the shape a tool consumes
    /// to read the project graph without parsing the connectors. Each node is
    /// `{name, path, deps: [...]}`; a node also carries `unresolved: true` (no `Project.cdz` at its path)
    /// or `repeated: true` (already shown higher up — a cycle/diamond, not re-expanded), mirroring the
    /// text form's `*unresolved*` / `(*)` markers. The root node's `path` is its directory.
    #[arg(long)]
    json: bool,
}

#[derive(clap::Args)]
struct AddArgs {
    /// The PATH dependency to add — a sibling project directory (relative to the manifest), e.g. `../lib`.
    path: String,
    /// The project to add the dependency TO: a `Project.cdz` or a directory holding one. OMITTED → search
    /// up from the current directory for the nearest `Project.cdz` (like `cdz build`).
    #[arg(long)]
    manifest: Option<String>,
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
    // The GLOBAL integer-overflow policy (signed + unsigned) — `"trap"`/`"wrap"`, or `null` when the
    // manifest sets none (the compiler's default `trap` applies). A malformed/unknown value reads `null`
    // here with the reason surfaced in `warnings` (matching how `opt_level` reports a dropped value).
    match &m.overflow_signed {
        Some(p) => obj.string("overflow_signed", p),
        None => obj.raw("overflow_signed", "null"),
    }
    match &m.overflow_unsigned {
        Some(p) => obj.string("overflow_unsigned", p),
        None => obj.raw("overflow_unsigned", "null"),
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
    // The DEPENDENCIES (`def deps`) — the projects this project links across the component boundary, so a
    // tool sees the project graph. Reported as the raw manifest refs (a path today; a registry ref later);
    // empty `[]` for a standalone project.
    let dep_texts: Vec<String> = m
        .deps
        .iter()
        .map(|d| d.as_manifest_text().to_string())
        .collect();
    obj.raw("deps", &str_array(&dep_texts));
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
    // Manifest WARNINGS — the machine-readable twin of the stderr warnings the project COMMANDS
    // (build/test/tree/…) emit via `resolve_project_manifest`. `cdz metadata` resolves via `load_manifest`
    // directly (it never builds), so those eprintln warnings don't fire here; a consumer reading ONLY
    // `cdz metadata` would see `"name":null`/`"opt_level":null` with no way to tell a MALFORMED value (wrong
    // type, silently dropped) from an ABSENT one. Surface each as a string so an editor/build-tool learns
    // WHY a field is null, and about a silently-dropped duplicate — without re-parsing the manifest. Empty
    // `[]` for a clean manifest (the common case).
    let mut warnings: Vec<String> = Vec::new();
    if m.name_malformed {
        warnings.push(
            "`name` is not a string — ignored; the directory name is used for the project/interface name"
                .to_string(),
        );
    }
    if m.opt_level_malformed {
        warnings
            .push("`opt-level` is not a string (expected one of O0/O1/O2/O3) — ignored, using the default tier".to_string());
    }
    if m.overflow_signed_malformed {
        warnings.push(
            "`overflow-signed` is not one of \"trap\"/\"wrap\" — ignored, using the default `trap`"
                .to_string(),
        );
    }
    if m.overflow_unsigned_malformed {
        warnings.push(
            "`overflow-unsigned` is not one of \"trap\"/\"wrap\" — ignored, using the default `trap`"
                .to_string(),
        );
    }
    if !m.duplicate_keys.is_empty() {
        warnings.push(format!(
            "manifest declares {} more than once — the LAST value wins, the earlier one(s) are ignored",
            m.duplicate_keys
                .iter()
                .map(|k| format!("`{k}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    obj.raw("warnings", &str_array(&warnings));
    println!("{}", obj.finish());
    ExitCode::SUCCESS
}

/// `cdz tree [DIR]` — print the project's DEPENDENCY TREE (the `cargo tree` analogue). Resolves the root
/// `Project.cdz` (same resolution as `cdz build`/`cdz metadata`), prints its `name (dir)`, then recurses
/// into each `def deps` PATH dependency — indented beneath its parent — so the whole transitive graph is
/// legible. A dep dir is resolved RELATIVE to its parent's manifest dir (same as `build_path_deps`). Two
/// termination guards keep it total on any graph: a dep whose `Project.cdz` doesn't resolve is printed
/// `*unresolved*` (not fatal — a partial tree is more useful than an abort), and a dep dir already shown
/// higher in the walk is printed with a `(*)` marker and NOT re-expanded (so a dependency CYCLE — or a
/// diamond — terminates instead of looping forever).
fn run_tree(args: &TreeArgs) -> ExitCode {
    let (dir, _mpath, m) = match resolve_project_manifest(args.dir.as_deref(), "cdz tree") {
        Ok(v) => v,
        Err(code) => return code,
    };
    let root_name = m.name.clone().unwrap_or_else(|| "<unnamed>".to_string());
    // Canonicalize a dir for the visited-set key (so `../a` and an absolute path to the same dir collide),
    // falling back to the raw path when canonicalization fails (a not-yet-existing dep dir).
    let canon = |p: &std::path::Path| -> std::path::PathBuf {
        std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
    };
    let mut visited: std::collections::HashSet<std::path::PathBuf> =
        std::collections::HashSet::new();
    visited.insert(canon(&dir));
    if args.json {
        // The root node object: `{name, path, deps: [...]}` — same shape a tool consumes for the whole
        // graph. The root's `path` is its directory (deps carry their manifest-relative path spelling).
        let deps = dep_subtree_json(&dir, &m, &mut visited);
        use cadenza_syntax::query::json;
        let mut obj = json::Object::new();
        obj.string("name", &root_name);
        obj.string("path", &dir.to_string_lossy());
        obj.raw("deps", &deps);
        println!("{}", obj.finish());
    } else {
        // The root line: `name (dir)`, then the recursive box-drawing subtree.
        println!("{root_name} ({})", dir.display());
        print_dep_subtree(&dir, &m, "", &mut visited);
    }
    ExitCode::SUCCESS
}

/// Build the JSON array of `manifest_dir`'s `def deps` as nested `{name, path, deps}` node objects — the
/// `--json` counterpart of [`print_dep_subtree`], sharing its resolution + cycle/diamond guard. A dep with
/// no resolvable `Project.cdz` is `{path, unresolved: true}`; a dep already shown higher up is
/// `{name, path, repeated: true}` (not re-expanded), mirroring the text form's `*unresolved*` / `(*)`.
fn dep_subtree_json(
    manifest_dir: &std::path::Path,
    m: &Manifest,
    visited: &mut std::collections::HashSet<std::path::PathBuf>,
) -> String {
    use cadenza_syntax::query::json;
    let canon = |p: &std::path::Path| -> std::path::PathBuf {
        std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
    };
    let mut arr = json::Array::new();
    for dep in &m.deps {
        #[allow(clippy::infallible_destructuring_match)]
        let dep_path = match dep {
            DepSource::Path(p) => p,
        };
        let dep_dir = manifest_dir.join(dep_path);
        let mut obj = json::Object::new();
        match load_manifest(&dep_dir) {
            Ok(Some((_dpath, dm))) => {
                let dep_name = dm.name.clone().unwrap_or_else(|| "<unnamed>".to_string());
                obj.string("name", &dep_name);
                obj.string("path", dep_path);
                let key = canon(&dep_dir);
                if visited.contains(&key) {
                    obj.raw("repeated", "true");
                } else {
                    visited.insert(key);
                    obj.raw("deps", &dep_subtree_json(&dep_dir, &dm, visited));
                }
            }
            _ => {
                obj.string("path", dep_path);
                obj.raw("unresolved", "true");
            }
        }
        arr.raw(&obj.finish());
    }
    arr.finish()
}

/// Recursively print `manifest_dir`'s `def deps` beneath it, each with a tree connector under `prefix`.
/// `visited` holds the canonical dirs already shown, so a cycle/diamond is marked `(*)` and not re-walked.
fn print_dep_subtree(
    manifest_dir: &std::path::Path,
    m: &Manifest,
    prefix: &str,
    visited: &mut std::collections::HashSet<std::path::PathBuf>,
) {
    let canon = |p: &std::path::Path| -> std::path::PathBuf {
        std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
    };
    let n = m.deps.len();
    for (i, dep) in m.deps.iter().enumerate() {
        #[allow(clippy::infallible_destructuring_match)]
        let dep_path = match dep {
            DepSource::Path(p) => p,
        };
        let last = i + 1 == n;
        let connector = if last { "└── " } else { "├── " };
        // The child prefix continues the vertical bars for non-last siblings.
        let child_prefix = format!("{prefix}{}", if last { "    " } else { "│   " });
        let dep_dir = manifest_dir.join(dep_path);
        match load_manifest(&dep_dir) {
            Ok(Some((_dpath, dm))) => {
                let dep_name = dm.name.clone().unwrap_or_else(|| "<unnamed>".to_string());
                let key = canon(&dep_dir);
                if visited.contains(&key) {
                    // Already shown higher up (a diamond or a cycle) — mark and don't re-expand.
                    println!("{prefix}{connector}{dep_name} ({dep_path}) (*)");
                } else {
                    visited.insert(key);
                    println!("{prefix}{connector}{dep_name} ({dep_path})");
                    print_dep_subtree(&dep_dir, &dm, &child_prefix, visited);
                }
            }
            // No manifest at the dep path, or it didn't parse — a partial tree beats an abort.
            _ => println!("{prefix}{connector}{dep_path} *unresolved*"),
        }
    }
}

/// `cdz add PATH [--manifest DIR]` — add a PATH dependency to the project's `Project.cdz` (the `cargo add`
/// analogue). Resolves the manifest (same as `cdz build`; upward search when `--manifest` is omitted),
/// dedup-checks PATH against the manifest's parsed `def deps` (an already-present path is an idempotent
/// no-op notice), then edits the manifest TEXT in place — inserting into an existing `def deps = [...]`
/// list, or adding a fresh `def deps = ["PATH"]` line when none exists. Text-editing (not re-serializing
/// the arena) preserves the user's formatting + comments. After writing, it RE-PARSES the manifest to
/// confirm the edit left a valid `Project.cdz` (and that PATH is now among the deps), rolling back on any
/// parse failure so a broken edit never lands. A PATH with no `Project.cdz` yet is a WARNING, not a
/// refusal — the dep may not exist yet (same tolerance as `cdz tree`'s `*unresolved*`).
fn run_add(args: &AddArgs) -> ExitCode {
    let (dir, mpath, m) = match resolve_project_manifest(args.manifest.as_deref(), "cdz add") {
        Ok(v) => v,
        Err(code) => return code,
    };
    // Idempotent: an already-declared path is a no-op (not a duplicate). Compare against the parsed deps.
    if m.deps.iter().any(|d| d.as_manifest_text() == args.path) {
        eprintln!(
            "{PROG}: `{}` already declares `{}` as a dependency — nothing to add",
            mpath.display(),
            args.path
        );
        return ExitCode::SUCCESS;
    }
    // Refuse a SELF-dependency: a path that resolves to the project's OWN directory (`cdz add .`, or a
    // `../proj` pointing back). A project depending on itself is meaningless — `cdz run` would build the
    // project a second time as its own "peer" (wasted work; a nonsensical manifest entry). Compare
    // CANONICALIZED paths so `.`, `./`, and a roundabout `../proj` are all caught; fall back to a literal
    // `.`/empty check if canonicalize fails (e.g. a not-yet-existing path can't be a self-dep anyway).
    let is_self = match (
        std::fs::canonicalize(dir.join(&args.path)),
        std::fs::canonicalize(&dir),
    ) {
        (Ok(dep), Ok(proj)) => dep == proj,
        _ => matches!(args.path.trim_end_matches('/'), "." | ""),
    };
    if is_self {
        eprintln!(
            "{PROG}: `{}` resolves to the project's own directory — a project cannot depend on itself \
             (a `def deps` entry names ANOTHER project to compose with); not adding",
            args.path
        );
        return ExitCode::FAILURE;
    }
    // A dep that doesn't resolve to a `Project.cdz` YET is a warning, not a refusal (the dir may be added
    // later; `cdz build`/`cdz tree` handle an unresolvable dep gracefully). Resolve relative to the
    // manifest dir, exactly as `build_path_deps` / `cdz tree` do.
    if !dir.join(&args.path).join(MANIFEST_NAME).is_file() {
        eprintln!(
            "{PROG}: warning: `{}` has no `{MANIFEST_NAME}` yet — adding it anyway (a `cdz build` will \
             report it unresolved until it exists)",
            args.path
        );
    }
    // Read the manifest TEXT (not the arena) so the edit preserves formatting + comments.
    let text = match std::fs::read_to_string(&mpath) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{PROG}: reading {}: {e}", mpath.display());
            return ExitCode::FAILURE;
        }
    };
    // Escape the path for the `"…"` string literal (a path with a `"`/`\` is unusual but must not malform
    // the manifest), via the canonical `cadenza_syntax` escaper so it re-parses exactly.
    let quoted = format!("\"{}\"", cadenza_syntax::literal::escape_string(&args.path));
    // Two edit shapes, both preserving surrounding text:
    //  - an existing `def deps = [ … ]` line: insert the new entry before the closing `]` (with a `, ` if
    //    the list is non-empty, so `["../a"]` → `["../a", "../b"]` and `[]` → `["../b"]`);
    //  - no `def deps` at all: append a fresh `def deps = ["PATH"]` line.
    let new_text = if let Some(open) = text.find("def deps") {
        // Find this `def deps` clause's `[` and its matching `]` (the manifest is a flat list of defs, so
        // the first `]` after the `[` closes it — no nested brackets in a string-list value).
        let Some(lb) = text[open..].find('[').map(|i| open + i) else {
            eprintln!(
                "{PROG}: {}: malformed `def deps` (no `[`) — not editing",
                mpath.display()
            );
            return ExitCode::FAILURE;
        };
        let Some(rb) = text[lb..].find(']').map(|i| lb + i) else {
            eprintln!(
                "{PROG}: {}: malformed `def deps` (no `]`) — not editing",
                mpath.display()
            );
            return ExitCode::FAILURE;
        };
        let inner = text[lb + 1..rb].trim();
        let insert = if inner.is_empty() {
            quoted.clone() // `[]` → `["PATH"]`
        } else {
            format!("{inner}, {quoted}") // `["../a"]` → `["../a", "PATH"]`
        };
        format!("{}[{insert}]{}", &text[..lb], &text[rb + 1..])
    } else {
        // No `def deps` — append a fresh line (a trailing newline first if the file doesn't end in one).
        let sep = if text.ends_with('\n') { "" } else { "\n" };
        format!("{text}{sep}def deps = [{quoted}]\n")
    };
    // Write, then RE-PARSE to confirm the edit is a valid manifest that now declares PATH. On any failure,
    // roll back to the original text so a broken edit never persists.
    if let Err(e) = std::fs::write(&mpath, &new_text) {
        eprintln!("{PROG}: writing {}: {e}", mpath.display());
        return ExitCode::FAILURE;
    }
    match load_manifest(&dir) {
        Ok(Some((_p, m2))) if m2.deps.iter().any(|d| d.as_manifest_text() == args.path) => {
            eprintln!("{PROG}: added `{}` to {}", args.path, mpath.display());
            ExitCode::SUCCESS
        }
        _ => {
            // The edit produced an invalid / unexpected manifest — restore the original.
            let _ = std::fs::write(&mpath, &text);
            eprintln!(
                "{PROG}: the edit did not produce a valid `{MANIFEST_NAME}` declaring `{}` — reverted \
                 (please add it by hand)",
                args.path
            );
            ExitCode::FAILURE
        }
    }
}

#[derive(clap::Args)]
struct RemoveArgs {
    /// The PATH dependency to remove — the same path text it was added with (relative to the manifest),
    /// e.g. `../lib`. Matched against the manifest's parsed `def deps` verbatim.
    path: String,
    /// The project to remove the dependency FROM: a `Project.cdz` or a directory holding one. OMITTED →
    /// search up from the current directory for the nearest `Project.cdz` (like `cdz build`/`cdz add`).
    #[arg(long)]
    manifest: Option<String>,
}

/// `cdz remove PATH [--manifest DIR]` — remove a PATH dependency from the project's `Project.cdz` (the
/// `cargo remove` analogue, the inverse of `cdz add`). Resolves the manifest (same as `cdz add`), checks
/// PATH is actually a declared dep (a path that isn't → an idempotent no-op notice), then rebuilds the
/// `def deps = [...]` list from the parsed deps MINUS `PATH` and rewrites just that clause's `[...]` —
/// rebuilding (rather than string-splicing out one entry + its comma) cleanly handles every position
/// (first/middle/last/only). Text-editing preserves the user's formatting + comments elsewhere. After
/// writing it RE-PARSES to confirm a valid manifest that NO LONGER declares PATH, rolling back on failure.
fn run_remove(args: &RemoveArgs) -> ExitCode {
    let (dir, mpath, m) = match resolve_project_manifest(args.manifest.as_deref(), "cdz remove") {
        Ok(v) => v,
        Err(code) => return code,
    };
    // Idempotent: removing a path that isn't a declared dependency is a no-op (not an error).
    if !m.deps.iter().any(|d| d.as_manifest_text() == args.path) {
        eprintln!(
            "{PROG}: `{}` does not declare `{}` as a dependency — nothing to remove",
            mpath.display(),
            args.path
        );
        return ExitCode::SUCCESS;
    }
    // Read the manifest TEXT (not the arena) so the edit preserves formatting + comments elsewhere.
    let text = match std::fs::read_to_string(&mpath) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{PROG}: reading {}: {e}", mpath.display());
            return ExitCode::FAILURE;
        }
    };
    // Rebuild the deps list as the remaining paths (parsed order), each re-quoted via the canonical
    // escaper so it re-parses exactly — then replace ONLY this `def deps` clause's `[...]` content. This
    // drops PATH regardless of its position and never leaves a dangling/leading comma.
    let remaining: Vec<String> = m
        .deps
        .iter()
        .map(|d| d.as_manifest_text())
        .filter(|p| p != &args.path)
        .map(|p| format!("\"{}\"", cadenza_syntax::literal::escape_string(p)))
        .collect();
    let Some(open) = text.find("def deps") else {
        // `deps` parsed non-empty but no `def deps` in text — shouldn't happen; don't guess, refuse.
        eprintln!(
            "{PROG}: {}: `def deps` not found in the manifest text — not editing",
            mpath.display()
        );
        return ExitCode::FAILURE;
    };
    let Some(lb) = text[open..].find('[').map(|i| open + i) else {
        eprintln!(
            "{PROG}: {}: malformed `def deps` (no `[`) — not editing",
            mpath.display()
        );
        return ExitCode::FAILURE;
    };
    let Some(rb) = text[lb..].find(']').map(|i| lb + i) else {
        eprintln!(
            "{PROG}: {}: malformed `def deps` (no `]`) — not editing",
            mpath.display()
        );
        return ExitCode::FAILURE;
    };
    let new_text = if remaining.is_empty() {
        // Removing the LAST dep: drop the whole `def deps` line rather than leave a dangling `def deps = []`
        // (the `cargo remove` behavior — an emptied section is removed, not left as an empty stub). Cut from
        // the start of `def deps` through the closing `]` PLUS a single trailing newline (so no blank line is
        // left behind). If the clause has no trailing newline (EOF), just cut through `]`. Any leading
        // indentation before `def deps` on its line is preserved by cutting from `open` (the `d` of `def`).
        let after = &text[rb + 1..];
        let after = after.strip_prefix('\n').unwrap_or(after);
        format!("{}{}", &text[..open], after)
    } else {
        format!(
            "{}[{}]{}",
            &text[..lb],
            remaining.join(", "),
            &text[rb + 1..]
        )
    };
    // Write, then RE-PARSE to confirm the edit is valid AND no longer declares PATH. Roll back on failure.
    if let Err(e) = std::fs::write(&mpath, &new_text) {
        eprintln!("{PROG}: writing {}: {e}", mpath.display());
        return ExitCode::FAILURE;
    }
    match load_manifest(&dir) {
        Ok(Some((_p, m2))) if !m2.deps.iter().any(|d| d.as_manifest_text() == args.path) => {
            eprintln!("{PROG}: removed `{}` from {}", args.path, mpath.display());
            ExitCode::SUCCESS
        }
        _ => {
            // The edit produced an invalid / unexpected manifest (or PATH still present) — restore original.
            let _ = std::fs::write(&mpath, &text);
            eprintln!(
                "{PROG}: the edit did not produce a valid `{MANIFEST_NAME}` without `{}` — reverted \
                 (please remove it by hand)",
                args.path
            );
            ExitCode::FAILURE
        }
    }
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
        cadenza_compile_abi::Request::Query(cadenza_compile_abi::sidecar::Query::Exports),
    );
    let Some(bytes) = out.artifact(cadenza_compile_abi::sidecar::KIND_EXPORTS) else {
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
    // WARN if the derived project name won't be a valid component-model interface SEGMENT (lowercase ASCII
    // letters/digits/hyphens). A standalone project with any name builds+runs fine, so this is NOT a
    // failure — but the moment another project takes THIS one as a `def deps` dependency, its name becomes
    // `cadenza:<name>/api` and `cdz run` rejects it (the dep-name validation). Warn HERE, where the author
    // is choosing the name, rather than letting it surprise them (or a consumer) later. Same alphabet as
    // the dep-name check, kept in sync.
    let name_ok = !proj_name.is_empty()
        && proj_name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !name_ok {
        eprintln!(
            "{PROG}: warning: project name `{proj_name}` is not a valid interface segment (lowercase ASCII \
             letters, digits, hyphens) — it builds fine standalone, but a project that depends on this one \
             (`def deps`) will be rejected, since a dependency's name becomes `cadenza:<name>/api`. Consider \
             a kebab-case name (e.g. `my-app`) + editing `def name` in the generated {MANIFEST_NAME}"
        );
    }
    // The scaffolded entry is written in its CANONICAL (`cdz fmt`) form, so a fresh project passes
    // `cdz fmt --check` immediately (a CI that fmt-checks shouldn't fail on the scaffold). The ML printer
    // puts a blank line between a top-level `def` and the `export` block — the previous single-line-gap
    // template failed `cdz fmt --check` on a brand-new project.
    // The scaffolded entry ships a starter `@test` alongside `main`, so a fresh project is BUILDABLE,
    // RUNNABLE, and TESTABLE out of the box — `cd <name> && cdz test` passes immediately (the `cargo new`
    // convention of a green starter test), instead of the previous scaffold's `cdz test` dead-ending on
    // "the manifest declares no `tests`". Written in CANONICAL (`cdz fmt`) form so the fresh project also
    // passes `cdz fmt --check` (the ML printer puts `@test` on its own line, a blank line before `export`;
    // the s-expr equality operator is `=`, not `==`).
    let (ext, entry_src) = if sexpr {
        (
            "sexp",
            "(do (def (main) 0) \
             (@ test (def (main_is_zero) (if (= (main) 0) unit (trap \"main\")))) \
             (export main))\n"
                .to_string(),
        )
    } else {
        (
            "cdz",
            "def main() -> Int64 = 0\n\n\
             @test\n\
             def main_is_zero() = if main() == 0 then unit else trap(\"main\")\n\n\
             export { main }\n"
                .to_string(),
        )
    };
    let entry_file = format!("main.{ext}");
    // ESCAPE the project name for the `"…"` string literal — the dir name is user-controlled, so a name
    // with a `"`, `\`, or control char would otherwise inject into (and malform) the generated
    // Project.cdz. `entry_file` is always `main.cdz`/`main.sexp` (no escaping needed), but escape it too
    // for uniformity. Uses the canonical `cadenza_syntax` escaper so the manifest re-parses exactly.
    // Declare the entry as the test suite too (`def tests = ["main.cdz"]`), so `cdz test` runs the
    // scaffolded `@test` out of the box rather than reporting "the manifest declares no `tests`". The entry
    // carries both `main` and a starter `@test`, so one file is entry + suite for a fresh project.
    let manifest_src = format!(
        "def name = \"{}\"\ndef entry = \"{}\"\ndef tests = [\"{}\"]\n",
        cadenza_syntax::literal::escape_string(&proj_name),
        cadenza_syntax::literal::escape_string(&entry_file),
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

#[cfg(feature = "completions")]
#[derive(clap::Args)]
struct CompletionsArgs {
    /// The shell to generate a completion script for.
    #[arg(value_enum)]
    shell: clap_complete::Shell,
}

/// `cdz completions <shell>` — print a shell completion script for `cdz` to stdout, generated from the
/// clap command tree (so it can never drift from the real subcommands/flags). The user redirects it to
/// their shell's completion location. Codegen only; always succeeds.
#[cfg(feature = "completions")]
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
    /// scripts (the `cdz metadata`/`cdz check --json` shape). The exit code is unchanged (non-zero iff the
    /// runtime STORE is missing/stale; the `cdz-run` presence is informational and never affects it).
    #[arg(long)]
    json: bool,
}

/// `cdz doctor` — a preflight health check of the `cdz` TOOLCHAIN environment (the `cargo`-doctor
/// analogue), so a broken setup surfaces before a `cdz run`/`cdz test` fails mid-operation. It reports
/// three things and exits non-zero if a component that would break run/test is missing. First, the `cdz`
/// version + executable path (what a bug report should cite). Second, the sibling `cdz-run` binary — this
/// is now PURELY INFORMATIONAL: `cdz run`/`cdz test` run IN-PROCESS (wasmtime + the runner are linked into
/// `cdz` via the `cdz-run` library), so a missing standalone `cdz-run` binary does NOT break anything and
/// is NOT a doctor failure — it's reported only as a convenience note (a standalone runner some scripts
/// still invoke). Third, the value-heap runtime store: present, and holding the runtime `cdz` compiles
/// against (`REQUIRED_RUNTIME_HASH`) — without it, running a program that builds heap values can't resolve
/// its runtime by content address (a scalar/const program still runs without the store, and the note says
/// so). Only a not-ok STORE is an ERROR (rc≠0), so a setup/CI script can gate on `cdz doctor`.
fn run_doctor(args: &DoctorArgs) -> ExitCode {
    // Compute the three checks into structured `(status, detail)` values FIRST, so the human and `--json`
    // outputs are the same facts rendered two ways (they can't drift). `status` is "ok" for a healthy
    // component or a distinct problem label ("missing"/"stale") a consumer can branch on.
    let exe = std::env::current_exe()
        .ok()
        .map(|p| p.display().to_string());

    // Standalone `cdz-run` runner — informational only (run/test are in-process; its absence breaks
    // nothing).
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
    // ONLY a not-ok store is a run/test-breaking problem now — `cdz run`/`cdz test` run in-process, so the
    // standalone `cdz-run` binary's presence is informational and never flips the verdict. (This also makes
    // `cdz doctor` env-independent for the runner: a bare `cargo test` checkout that never built `cdz-run`
    // is still `ok` when the store is present.)
    let ok = store_status == "ok";

    if args.json {
        use cadenza_syntax::query::json;
        let mut obj = json::Object::new();
        obj.string("version", env!("CARGO_PKG_VERSION"));
        match &exe {
            Some(p) => obj.string("path", p),
            None => obj.raw("path", "null"),
        }
        let mut cr = json::Object::new();
        // `present`: whether the standalone binary was found beside `cdz` (informational — run/test are
        // in-process, so this never affects `ok`). The `ok` key is kept as an alias for back-compat.
        cr.raw("present", if cdz_run.is_some() { "true" } else { "false" });
        cr.raw("ok", if cdz_run.is_some() { "true" } else { "false" });
        cr.raw("required", "false"); // in-process run/test need no standalone runner
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
        Some(p) => println!(
            "  cdz-run: present ({p}) — standalone runner (optional; run/test are in-process)"
        ),
        None => println!(
            "  cdz-run: not present — optional; `cdz run`/`cdz test` run IN-PROCESS, so no standalone \
             runner is needed (build one with `cargo build --bin cdz-run` only if a script invokes it directly)"
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

// ── cdz smith (fuzzer passthrough) ─────────────────────────────────────────────────────────────────

#[derive(clap::Args)]
struct SmithArgs {
    /// Every argument after `cdz smith` is forwarded VERBATIM to the standalone `cdz-smith` binary
    /// (`trailing_var_arg` + `allow_hyphen_values` so flags like `--iters 100` pass through untouched
    /// rather than being parsed as `cdz`'s own). `cdz smith --help` prints the standalone bin's help.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    args: Vec<String>,
}

/// Locate a sibling passthrough binary (`stem`) beside THIS `cdz` — the install-location-independent
/// `current_exe().parent()/<stem>` path. Used ONLY by the `!standalone` compile-delegation (`delegate.rs`,
/// which resolves `cdz-compile`), hence the matching `cfg` — the subcommand passthroughs go through the
/// fuller [`locate_plugin`] resolver. Carries the platform executable suffix via [`bin_name`].
#[cfg(not(feature = "standalone"))]
fn locate_sibling_bin(stem: &str) -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|dir| dir.join(bin_name(stem))))
        .filter(|p| p.exists())
}

// ── git-style plugin dispatch (thin-`cdz` seam) ──────────────────────────────────────────────────────

/// The plugin-dispatch FALLBACK, tried before clap parses (see the call in [`main`]). Returns
/// `Some(exit_code)` when the invocation was forwarded to an external `cdz-<name>` plugin, or `None` to
/// let clap handle it (no first token, a flag, a KNOWN subcommand, or an unknown token with no resolvable
/// plugin). Reads argv directly (`std::env::args`) because the decision precedes clap parsing.
fn try_plugin_dispatch() -> Option<ExitCode> {
    let args: Vec<String> = std::env::args().collect();
    let sub = args.get(1)?;
    // Never intercept a flag (`--help`, `-V`, `--version`) or clap's own `help` subcommand — those are
    // `cdz`'s to handle. Only a bare subcommand token is a plugin-dispatch candidate.
    if sub.starts_with('-') || sub == "help" {
        return None;
    }
    // Builtin-first (git's rule): a KNOWN clap subcommand (or alias) always runs in-process, so this
    // fallback is behavior-neutral for every subcommand `cdz` still owns.
    if is_known_subcommand(sub) {
        return None;
    }
    // Unknown token: forward to `cdz-<sub>` if one resolves, else fall through to clap's error.
    let plugin = locate_plugin(sub)?;
    Some(passthrough_status(
        &plugin,
        &args[2..],
        &format!("cdz-{sub}"),
    ))
}

/// Is `name` a subcommand (or alias) `cdz`'s clap tree knows? Drives the builtin-first precedence in
/// [`try_plugin_dispatch`] AND the plugin-listing skip in [`discover_plugins`]. Enumerated from the
/// derived [`Cli`] command so it can never drift from the actual subcommand set. `help` counts as known
/// (clap's auto-help subcommand, which `get_subcommands` omits) so a stray `cdz-help` on PATH — e.g. a
/// devShell wrapper — is never dispatched to or listed as a plugin.
fn is_known_subcommand(name: &str) -> bool {
    name == "help"
        || Cli::command()
            .get_subcommands()
            .any(|c| c.get_name() == name || c.get_all_aliases().any(|a| a == name))
}

/// The `$CDZ_<NAME>_BIN` override key for a plugin — the same explicit-path injection convention the
/// compile delegate uses (`$CDZ_COMPILE_BIN`), so nix can hand `cdz` the exact content-addressed plugin.
/// `<NAME>` is the subcommand upper-cased with every non-alphanumeric byte mapped to `_` (so `run-rust`
/// → `CDZ_RUN_RUST_BIN`).
fn plugin_env_key(name: &str) -> String {
    let mut key = String::from("CDZ_");
    for c in name.chars() {
        key.push(if c.is_ascii_alphanumeric() {
            c.to_ascii_uppercase()
        } else {
            '_'
        });
    }
    key.push_str("_BIN");
    key
}

/// Resolve an external `cdz-<name>` plugin binary. Resolution order (mirrors the delegate + sibling
/// convention): **`$CDZ_<NAME>_BIN` (explicit path) → sibling (`current_exe().parent()/cdz-<name>`) →
/// `$PATH`.** `None` if unresolved (the caller then falls through to clap). Reads the process env/PATH and
/// hands the pure decision to [`resolve_plugin`] so the priority logic is unit-testable without touching
/// global state.
fn locate_plugin(name: &str) -> Option<PathBuf> {
    let env_val = std::env::var_os(plugin_env_key(name));
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf));
    let path_dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    resolve_plugin(name, env_val, exe_dir.as_deref(), &path_dirs)
}

/// The pure plugin-resolution decision (env override → sibling dir → PATH dirs), split out so the
/// priority order is testable with tempdir fixtures and no process-global env mutation.
fn resolve_plugin(
    name: &str,
    env_val: Option<OsString>,
    exe_dir: Option<&Path>,
    path_dirs: &[PathBuf],
) -> Option<PathBuf> {
    // 1. Explicit `$CDZ_<NAME>_BIN` path (honored only if it points at a real file).
    if let Some(v) = env_val {
        let p = PathBuf::from(v);
        if p.is_file() {
            return Some(p);
        }
    }
    let stem = bin_name(&format!("cdz-{name}"));
    // 2. A `cdz-<name>` beside this `cdz` (the co-built location).
    if let Some(dir) = exe_dir {
        let cand = dir.join(&stem);
        if cand.is_file() {
            return Some(cand);
        }
    }
    // 3. First `cdz-<name>` on `$PATH`.
    path_dirs
        .iter()
        .map(|d| d.join(&stem))
        .find(|p| p.is_file())
}

/// Does argv request the TOP-LEVEL help (`cdz --help` / `cdz -h` / `cdz help`)? Only the top level —
/// `cdz <sub> --help` stays clap's. Bare `cdz` is left to clap (its usage error), unchanged.
fn wants_toplevel_help(args: &[String]) -> bool {
    args.len() == 2 && matches!(args[1].as_str(), "--help" | "-h" | "help")
}

/// The `--cdz-summary` sentinel a plugin answers with its one-line help (see
/// `design/DESIGN-cdz-plugin-dispatch.md`): a plugin invoked with exactly this flag prints ONE line to
/// stdout and exits 0. `cdz --help` queries every discovered `cdz-<name>` with it and aggregates.
const CDZ_SUMMARY_FLAG: &str = "--cdz-summary";

/// Print `cdz`'s own (clap) help, then a git-style listing of the EXTERNAL `cdz-<name>` plugins
/// discovered on PATH — each best-effort annotated with its one-line `--cdz-summary`. This is the
/// aggregation half of the plugin model: builtins come from clap, external commands are discovered.
fn print_help_with_plugins() -> ExitCode {
    let mut cmd = Cli::command();
    let _ = cmd.print_long_help();
    println!();
    let plugins = discover_plugins();
    if !plugins.is_empty() {
        println!("Plugin commands (external `cdz-<name>` binaries found on PATH):");
        for (name, summary) in plugins {
            match summary {
                Some(s) => println!("  {name:<16} {s}"),
                None => println!("  {name}"),
            }
        }
    }
    ExitCode::SUCCESS
}

/// Discover the external `cdz-<name>` plugin commands reachable as `cdz <name>`: walk the sibling dir
/// (beside this `cdz`) then every `$PATH` entry, collect `cdz-<name>` executables, and best-effort query
/// each for its one-line summary. Returns `(name, summary?)` sorted by name. A name that shadows a KNOWN
/// clap subcommand is skipped (builtin-first — such a plugin is never reachable). First location wins on
/// a duplicate name (sibling before PATH, mirroring [`resolve_plugin`]'s precedence).
fn discover_plugins() -> Vec<(String, Option<String>)> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf));
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(d) = exe_dir {
        dirs.push(d);
    }
    if let Some(p) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&p));
    }
    discover_plugin_names(&dirs, is_known_subcommand)
        .into_iter()
        .map(|name| {
            let summary = locate_plugin(&name).and_then(|bin| plugin_summary(&bin));
            (name, summary)
        })
        .collect()
}

/// The pure plugin-DISCOVERY walk (split out for unit testing without executing anything): scan `dirs`
/// in order for files named `cdz-<name>` (platform suffix aware), yielding each `<name>` once (first
/// occurrence wins), skipping any `is_builtin(name)` (builtin-first), sorted. Does NOT query summaries.
fn discover_plugin_names(dirs: &[PathBuf], is_builtin: impl Fn(&str) -> bool) -> Vec<String> {
    let prefix = "cdz-";
    let suffix = if cfg!(windows) { ".exe" } else { "" };
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if !entry.path().is_file() {
                continue;
            }
            let Ok(fname) = entry.file_name().into_string() else {
                continue;
            };
            let Some(rest) = fname.strip_prefix(prefix) else {
                continue;
            };
            let name = match rest.strip_suffix(suffix) {
                Some(n) if !suffix.is_empty() => n,
                _ => rest,
            };
            if name.is_empty() || is_builtin(name) {
                continue;
            }
            seen.insert(name.to_string());
        }
    }
    seen.into_iter().collect()
}

/// Best-effort query of a plugin's one-line summary: run `<bin> --cdz-summary`, and if it exits 0 with
/// exactly one non-empty stdout line, return it (trimmed). Any other outcome (non-zero, empty, multi-line,
/// spawn error) → `None` — the plugin is then listed by name only (git's graceful `help -a` degrade), so a
/// foreign or older `cdz-*` that does not speak the sentinel never breaks `cdz --help`.
fn plugin_summary(bin: &Path) -> Option<String> {
    use std::io::Read;
    use std::process::Stdio;
    // Bound the query: a foreign/misbehaving `cdz-*` that hangs (or blocks reading stdin, or never exits)
    // on `--cdz-summary` must NOT hang `cdz --help`. Spawn with piped stdout + null stdin, poll `try_wait`
    // to a short deadline, and KILL on timeout → `None` (listed by name only). A well-formed plugin exits
    // in microseconds, so the normal path pays ~one poll.
    let mut child = std::process::Command::new(bin)
        .arg(CDZ_SUMMARY_FLAG)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = std::time::Instant::now() + PLUGIN_SUMMARY_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Err(_) => return None,
        }
    };
    if !status.success() {
        return None;
    }
    // The process has exited; drain its (small, one-line) buffered stdout. A plugin that flooded stdout
    // past the pipe buffer would have blocked on write → never exited → already timed out above.
    let mut text = String::new();
    child.stdout.take()?.read_to_string(&mut text).ok()?;
    parse_plugin_summary(&text)
}

/// The per-plugin `--cdz-summary` query timeout (see [`plugin_summary`]). Short — a well-formed plugin
/// answers instantly; this only bounds a misbehaving one so it can't hang `cdz --help`.
const PLUGIN_SUMMARY_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(500);

/// Parse a plugin's `--cdz-summary` stdout: exactly ONE non-empty line → `Some(trimmed)`, else `None`
/// (empty or multi-line is not a well-formed summary). Split out so the well-formed/degrade rule is
/// unit-testable without spawning a process.
fn parse_plugin_summary(stdout: &str) -> Option<String> {
    let mut lines = stdout.lines().filter(|l| !l.trim().is_empty());
    let first = lines.next()?.trim().to_string();
    match lines.next() {
        Some(_) => None, // more than one non-empty line → not a well-formed summary
        None if first.is_empty() => None,
        None => Some(first),
    }
}

/// `cdz smith <args…>` (alias `cdz fuzz`) — a PASSTHROUGH to the standalone `cdz-smith` fuzzer/differential
/// driver: exec the sibling binary and forward argv + exit code, so a single `cdz` on the PATH reaches the
/// fuzzer for discoverability. It is exec-not-link ON PURPOSE — `cdz-smith` is a SEPARATE cargo workspace
/// (its `bolero` engine + wasmtime/cranelift oracle cannot co-resolve into `cdz`'s lockfile), so linking it
/// in would reintroduce the exact dependency conflict its workspace-exclusion exists to prevent. Resolves
/// the binary beside `cdz` first (the co-built location), then falls back to `$PATH` (an installed one).
fn run_smith(args: &SmithArgs) -> ExitCode {
    // Resolve `cdz-smith` through the SAME plugin resolver as every other forwarded subcommand
    // ([`locate_plugin`]): `$CDZ_SMITH_BIN` (explicit path, for nix injection) → co-built sibling → `$PATH`.
    // Falls back to the bare `cdz-smith` name so `passthrough_status` still emits the actionable build hint
    // when nothing resolves (the separate-workspace bin isn't produced by an ordinary `cargo build`). The
    // bare name carries the platform executable suffix (`.exe` on Windows) so a PATH lookup resolves.
    let program = locate_plugin("smith").unwrap_or_else(|| PathBuf::from(bin_name("cdz-smith")));
    passthrough_status(&program, &args.args, "cdz-smith")
}

#[derive(clap::Args)]
struct CadArgs {
    /// Every argument after `cdz cad` is forwarded VERBATIM to the standalone `cdz-cad` binary
    /// (`trailing_var_arg` + `allow_hyphen_values` so `cdz cad - -o out.stl --segments 64` passes through
    /// untouched rather than being parsed as `cdz`'s own). `cdz cad --help` prints the standalone bin's usage.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    args: Vec<String>,
}

/// `cdz cad <args…>` — a PASSTHROUGH to the standalone `cdz-cad` CAD mesh driver: exec the sibling binary
/// and forward argv + exit code, so a single `cdz` on the PATH reaches the mesh exporter for discoverability
/// (`cdz run model.cdz | cdz cad - -o out.stl`). It is exec-not-link ON PURPOSE — `cdz-cad` is a SEPARATE
/// cargo workspace (its `manifold-csg` backend builds the C++ manifold3d library via cmake), so linking it
/// in would pull that native/cmake build into `cdz`'s workspace — the exact thing its exclusion prevents.
/// Resolves the binary beside `cdz` first (the co-built location), then falls back to `$PATH`. Shares
/// `passthrough_status` with `cdz smith` so exit-code + not-found handling stay identical.
fn run_cad(args: &CadArgs) -> ExitCode {
    // Same plugin resolution as the other forwards ([`locate_plugin`]): `$CDZ_CAD_BIN` → sibling → `$PATH`,
    // then the bare `cdz-cad` name so `passthrough_status`'s not-found hint still fires.
    let program = locate_plugin("cad").unwrap_or_else(|| PathBuf::from(bin_name("cdz-cad")));
    passthrough_status(&program, &args.args, "cdz-cad")
}

#[derive(clap::Args)]
struct CalcArgs {
    /// Every argument after `cdz calc` is forwarded VERBATIM to the standalone `cdz-calc` binary
    /// (`trailing_var_arg` + `allow_hyphen_values` so `cdz calc --once "1/2 + 1/3"` passes through untouched
    /// rather than being parsed as `cdz`'s own). `cdz calc --help` prints the standalone bin's usage.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    args: Vec<String>,
}

/// `cdz calc <args…>` (alias `cdz repl`) — a PASSTHROUGH to the standalone `cdz-calc` calculator REPL: exec
/// the sibling binary and forward argv + exit code, so a single `cdz` on the PATH reaches the calculator. It
/// is exec-not-link so `cdz` no longer links `cdz-calc` — which pulls `cdz-run` (→ wasmtime) in-process to run
/// compiled exprs — shedding that transitive runner weight from `cdz`'s graph. Resolves via the standard
/// `$CDZ_CALC_BIN` → sibling → `$PATH` ([`locate_plugin`]); the calc engine itself is v-guide-infra's.
fn run_calc(args: &CalcArgs) -> ExitCode {
    let program = locate_plugin("calc").unwrap_or_else(|| PathBuf::from(bin_name("cdz-calc")));
    passthrough_status(&program, &args.args, "cdz-calc")
}

/// The platform executable NAME for a bare tool stem — appends `.exe` on Windows so a `$PATH` lookup of
/// the fallback resolves (mirrors `locate_cdz_run`/`locate_cdz_smith`'s sibling-path handling). On unix the
/// stem is the name.
fn bin_name(stem: &str) -> String {
    if cfg!(windows) {
        format!("{stem}.exe")
    } else {
        stem.to_string()
    }
}

/// Map a child process's exit `code` (from `ExitStatus::code()`) to a `cdz` `ExitCode`, WITHOUT wrapping.
/// A code in `1..=255` forwards as-is; a code outside `0..=255` (possible on some platforms) or a
/// signal-killed child (`None`) maps to `FAILURE` — critically NOT `as u8` truncation, which would map a
/// child exit `256` to `0` and report a FAILURE as SUCCESS (PR#747 review). `Some(0)` never reaches here
/// (the caller handles `status.success()` first); if it did, it forwards as `SUCCESS`.
fn exit_code_from_child(code: Option<i32>) -> ExitCode {
    code.and_then(|c| u8::try_from(c).ok())
        .map(ExitCode::from)
        .unwrap_or(ExitCode::FAILURE)
}

/// Exec `program` forwarding `args`, propagate its exit code, and on a spawn failure print an actionable
/// error (a NotFound gets the "build it" hint keyed by `tool`, the crate name to `cargo build -p`). Shared
/// by the passthrough subcommands (`cdz smith`, `cdz cad`, …) so exit-code + not-found handling can't drift.
/// The exit code is forwarded via `u8::try_from` (NOT `as u8`): a raw code outside `0..=255` — possible on
/// some platforms — must NOT wrap (e.g. `256 as u8 == 0` would report a child FAILURE as SUCCESS); an
/// out-of-range or signal-killed (`code() == None`) child maps to `FAILURE`.
fn passthrough_status(program: &std::path::Path, args: &[String], tool: &str) -> ExitCode {
    let mut cmd = std::process::Command::new(program);
    cmd.args(args);
    match cmd.status() {
        Ok(status) => {
            if status.success() {
                ExitCode::SUCCESS
            } else {
                exit_code_from_child(status.code())
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!(
                "{PROG}: {tool} not found beside `cdz` or on `$PATH` — it is a SEPARATE-workspace build \
                 (deliberately not linked into `cdz`); build it with `cargo build -p {tool}` and re-run"
            );
            ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!("{PROG}: could not run {tool} ({}): {e}", program.display());
            ExitCode::FAILURE
        }
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
///  4. Run each test IN-PROCESS via the `cdz-run` LIBRARY (`run_capturing` — no sibling binary), calling
///     the test's kebab export. The export RETURNING = PASS; it TRAPPING = FAIL. A failure's message rides
///     an OBSERVED host-op entry (the assertion text the test emitted via its report host effect before
///     trapping); `run_capturing`'s observed-op list also yields the `Test.gen-int` count that distinguishes a
///     property test from a plain unit test — no subprocess, no stderr parsing.
///
/// Exits non-zero if ANY test fails (or if a file's compile declines / no `@test` is present) — the CI
/// shape. FILE may be a DIRECTORY: every source file under it (recursively, `.cdz`/`.ml`/`.sexp`) is run
/// and the pass/fail totals are aggregated, so `cdz test <dir>` runs a whole package's suite in one call.
///
/// The precompiled test components for a `cdz test <dir>` run, from ONE shared-arena `EmitTestsComposed`
/// compile: each file's `@test` CONSUMER component (keyed by file-link name), plus the ONE shared-closure
/// PROVIDER component every consumer imports + its interface name. `run_test_file` looks up its consumer and
/// runs it linked against the provider peer (`run_with_peers`) over one shared runtime — so the shared
/// closure is LOWERED and EMITTED once (a provider component) instead of re-embedded in every file's
/// component (the >98% per-file emit/JIT cost). BEST-EFFORT: on any hiccup (compile declines, provider or a
/// file's consumer absent, single file, multi-dir stem-collision) `run_test_file` FALLS BACK to its exact
/// per-file `EmitTests` compile — behavior is never worse than before.
/// A shared-closure PROVIDER peer: its component bytes, the interface name it exports (the consumer imports
/// under this exact name), and the closure's CONTENT HASH (the same `Query::ClosureHash` value the
/// `.provider.wasm` is keyed by, when available). The content hash — NOT the group key (which is the import
/// NAME-set, stable across content edits) — is what a JIT-artifact (cwasm) cache must key on: a content change
/// with an unchanged import set must invalidate the cwasm, else a stale compiled provider would be reused.
type ProviderPeer = (Vec<u8>, String, Option<String>);

#[derive(Default)]
struct Precompiled {
    /// file-link-name → (that file's `@test` CONSUMER component bytes, the PROVIDER-GROUP key it links
    /// against). The group key indexes `providers`. A file absent here fell back (self-contained, decline,
    /// or its group produced no provider) and `run_test_file` re-emits it standalone.
    components: std::collections::HashMap<String, (Vec<u8>, String)>,
    /// provider-group key → the group's [`ProviderPeer`]. ONE entry per GENUINE shared closure (Option-A
    /// grouping — a `cdz test <dir>` over a HETEROGENEOUS tree emits one provider per closure, NOT one
    /// whole-compiler union over every file). A consumer links against `providers[its group key]`; a missing
    /// entry ⇒ that group declined ⇒ its files fall back per-file.
    providers: std::collections::HashMap<String, ProviderPeer>,
    /// For a SINGLE-file `cdz test <file>` run ONLY: the file's import closure, loaded once and SHARED with
    /// `run_test_file` so it isn't parsed twice (PR#907 — dropping the `files.len() < 2` blanket-skip meant a
    /// single file's closure was loaded here for the cache decision AND again in `run_test_file`). `Rc` so the
    /// share is a refcount bump, not a deep clone of the arenas. `None` for a multi-file run (each file loads
    /// its own closure once in `run_test_file`, as before — stashing all N would raise peak memory for no gain).
    single_file_closure: Option<std::rc::Rc<Vec<closure::LoadedFile>>>,
}

/// Compile ONE closure-group's shared provider + its `@test` consumers, with the cross-invocation provider
/// cache — the per-group unit [`precompile_tests_per_file`] runs after partitioning the target files by
/// shared closure. `ast_inputs` is the UNION of this group's closure ASTs (the group's target `@test` files +
/// the shared libs they import, deduped by link name); `entry` is any closure file name (drives linking, does
/// not restrict which files' @tests emit). Returns the provider peer (bytes + interface name) when the
/// composed emit produced one, plus the per-file consumer components (named by file-link). SINGLE-MONO flow:
/// ONE `EmitTestsComposed` (one monomorphize+layout) yields the closure-hash sidecar plus the composed
/// provider plus every file's consumer; the cache decision is then made from that emitted hash, so a HIT
/// reuses the persisted `.provider.wasm` (discarding the emitted provider) while a MISS atomic-persists the
/// emitted provider. This replaces an earlier two-drive flow (`Query::ClosureHash`, then a HIT
/// `EmitTestsConsumerOnly` or a MISS `EmitTestsComposed`) that paid the closure monomorphize+layout TWICE on a
/// HIT; folding to one emit pays it once. Best-effort throughout: a decline yields `(None, [])` and every file
/// in the group falls back to its own `EmitTests`.
fn precompile_group(
    ast_inputs: Vec<cadenza_compile_abi::Artifact>,
    entry: &str,
    cache_dir: Option<&std::path::Path>,
) -> (Option<ProviderPeer>, Vec<(String, Vec<u8>)>) {
    let entry_marker = cadenza_compile_abi::abi::entry_artifact(entry);
    let drive = |req: cadenza_compile_abi::Request| -> cadenza_compile_abi::CompileOutput {
        let mut inputs = ast_inputs.clone();
        inputs.push(cadenza_compile_abi::Artifact::new(
            cadenza_compile_abi::sidecar::KIND_SIDECAR,
            "drive",
            cadenza_compile_abi::sidecar::encode(&[req]),
        ));
        inputs.push(entry_marker.clone());
        rcdzc::run_with_compiler_stack(|| rcdzc::compile(&inputs, &[]))
    };

    // CROSS-INVOCATION PROVIDER CACHE, codegen-skip-on-HIT flow: drive `EmitTestsConsumerOnly` FIRST — one
    // monomorphize+layout that emits the closure CONTENT-HASH sidecar (`KIND_CLOSURE_HASH`, hoisted onto this
    // path by rcdzc #1502) + every file's CONSUMER component, but NO provider (`emit_provider=false`). We
    // decide HIT/MISS from that hash:
    //   • HIT (a validated `.provider.wasm` exists for the hash): pair the CACHED provider with the
    //     ConsumerOnly consumers — DONE, and we NEVER emit the provider. This SKIPS the ~215s provider CODEGEN
    //     (the ~570-def self-host closure's emit) that dominates the warm-once cost — the whole point of the
    //     cache. (Measured by v-compiler-perf, #1502: the ~231s HIT precompile was "~all of it the provider
    //     emit that gets thrown away.")
    //   • MISS: drive `EmitTestsComposed` — which emits the provider — and PERSIST it by the hash so the next
    //     run HITs. We pay the provider codegen ONLY when there's no cached provider to reuse.
    // The KEY is v-rust-backend's canonical `closure_content_hash`; #1502's `consumer_only_emits_the_closure_
    // hash_sidecar` locks that ConsumerOnly's hash EQUALS Composed's, so the HIT decision (from ConsumerOnly)
    // and the persisted key (from Composed) agree by construction.
    //
    // Why ConsumerOnly-first (not the prior single-`EmitTestsComposed`): Composed ALWAYS emits the provider —
    // paying its ~215s codegen even on a HIT, then discarding it. ConsumerOnly's mono+layout is the ~15s floor
    // (measured: `cdz func-layout`), with only thin consumer codegen — so a HIT collapses ~230s→~20s. The
    // trade is a MISS now pays TWO monos (ConsumerOnly ~15s + Composed ~230s ≈ 245s vs single-Composed's
    // ~230s) — a negligible regression on the RARE miss (only the first warm, or when the closure content
    // changes) for a large win on the COMMON hit (every re-gate against a stable closure). NOTE the hash is
    // over THIS GROUP's closure only — grouping shrinks each provider AND scopes each cache entry to one
    // closure (a lib change busts only the groups whose closure includes it).
    let consumer_out = drive(cadenza_compile_abi::Request::EmitTestsConsumerOnly);
    let closure_hash = consumer_out
        .artifact(cadenza_compile_abi::sidecar::KIND_CLOSURE_HASH)
        .map(|b| String::from_utf8_lossy(b).trim().to_string())
        .filter(|h| !h.is_empty());

    // OBSERVABILITY (`CDZ_PROVIDER_CACHE_TRACE` = any non-empty value): emit ONE line to stderr PER GROUP
    // recording the cache decision + closure key, so a caller can VERIFY a run warmed/hit the cache (which
    // group's provider persisted vs was reused) and a test can distinguish a HIT from the standalone fallback.
    // Off by default → zero output on the normal path; peer of `CDZ_WASM_BACKTRACE` / `CDZ_DUMP_TEST_WASM`.
    let trace = |ev: &str| {
        if std::env::var("CDZ_PROVIDER_CACHE_TRACE").is_ok_and(|v| !v.trim().is_empty()) {
            let key = closure_hash.as_deref().unwrap_or("<no-hash>");
            let dir = cache_dir
                .map(|d| d.display().to_string())
                .unwrap_or_else(|| "<no-cache-dir>".into());
            eprintln!("[provider-cache] {ev} key={key} dir={dir}");
        }
    };

    // A VALIDATED cached provider for this key, if one exists. VALIDATE the bytes compile BEFORE trusting the
    // hit path: a truncated / corrupt / stale-format cache file must NOT break `cdz test` (it would surface
    // later as an opaque per-file "invalid peer component" compile error). If it doesn't compile, discard it →
    // treat as a MISS (emit + re-persist via the Composed drive below), which self-heals the bad entry.
    let cached_provider = closure_hash
        .as_ref()
        .and_then(|h| cache_dir.map(|d| d.join(format!("{h}.provider.wasm"))))
        .filter(|p| p.is_file())
        .and_then(|p| std::fs::read(&p).ok())
        .filter(|bytes| cdz_run::compile_component(bytes).is_ok());

    // Decide the peer provider AND which drive's output supplies the consumers/iface:
    //   HIT   → cached provider + the ConsumerOnly consumers (no Composed drive: skips the provider codegen).
    //   DECLINE (no hash) → ConsumerOnly emitted no shared-closure hash; a Composed drive would re-mono and
    //           re-decline, so DON'T drive it — fall back per-file (peer stays None below).
    //   MISS  → drive Composed to emit the provider, persist it, and use ITS output (provider + consumers are
    //           generated together, guaranteed consistent).
    let (provider, out) = if let Some(cached) = cached_provider {
        trace("hit");
        (Some(cached), consumer_out)
    } else if closure_hash.is_none() {
        trace("decline no-shared-closure");
        (None, consumer_out)
    } else {
        // MISS: emit the provider via Composed (the only drive that emits it), then persist by the hash.
        let composed_out = drive(cadenza_compile_abi::Request::EmitTestsComposed);
        let emitted_provider = composed_out
            .artifacts
            .iter()
            .find(|a| a.kind == "component-provider")
            .map(|p| p.bytes.clone());
        if let (Some(bytes), Some(dir), Some(key)) = (&emitted_provider, cache_dir, &closure_hash) {
            // Best-effort ATOMIC persist: write a pid-stamped temp in the SAME dir, then rename onto the
            // content-addressed key — rename is atomic on POSIX, so a reader (incl. a CONCURRENT `cdz test`)
            // never sees a partial file at the key, and a crash mid-write leaves only the temp (never a
            // truncated file at the key that a later run would HIT as corrupt). A write/rename FAILURE
            // (full/RO FS) just means the next run re-emits — no correctness impact.
            let _ = std::fs::create_dir_all(dir);
            let final_path = dir.join(format!("{key}.provider.wasm"));
            let tmp = dir.join(format!(".{key}.provider.wasm.{}.tmp", std::process::id()));
            if std::fs::write(&tmp, bytes).is_err() {
                let _ = std::fs::remove_file(&tmp);
                trace("miss no-persist(write-failed)");
            } else if std::fs::rename(&tmp, &final_path).is_err() {
                // Rename failed. On POSIX rare (a real FS error). On WINDOWS `rename` fails when the dest
                // EXISTS — the self-heal case (a corrupt {key} present) where the corrupt file must NOT
                // survive: best-effort remove the dest + retry once; if that also fails, drop the temp.
                let _ = std::fs::remove_file(&final_path);
                if std::fs::rename(&tmp, &final_path).is_err() {
                    let _ = std::fs::remove_file(&tmp);
                    trace("miss no-persist(rename-failed)");
                } else {
                    trace("miss persisted");
                }
            } else {
                trace("miss persisted");
            }
        } else {
            // MISS but nothing to persist (no cache dir, no emitted provider, or no key) — still use whatever
            // provider was emitted (may be `None` → the group falls back per-file below).
            trace(if emitted_provider.is_none() {
                "miss no-persist(no-provider)"
            } else {
                "miss no-persist(no-key-or-dir)"
            });
        }
        (emitted_provider, composed_out)
    };

    // Demux: the `component-name` sidecar carries the provider's interface string; the N `component` artifacts
    // are the per-file consumers (named by file-link). A DECLINE (ill-typed @test, or an un-representable
    // higher-order cross-edge in THIS group's union) yields no provider/consumers → this group's files fall
    // back to their own per-file `EmitTests` (which re-surfaces any fault located; we do NOT report here).
    let iface = out
        .artifacts
        .iter()
        .find(|a| a.kind == "component-name")
        .map(|a| String::from_utf8_lossy(&a.bytes).into_owned());
    let consumers = out
        .artifacts
        .iter()
        .filter(|a| a.kind == "component")
        .map(|a| (a.name.clone(), a.bytes.clone()))
        .collect();
    // Pair the provider with its interface name only when BOTH are present — a consumer can only be linked
    // against a peer we can name; else the group's files fall back per-file (safe degrade). Carry the closure
    // CONTENT HASH so a JIT-artifact (cwasm) cache can key on it (content-addressed, not the import-name group
    // key) — the cwasm must invalidate when the closure content changes even if the import set doesn't.
    let peer = provider
        .zip(iface)
        .map(|(bytes, iface)| (bytes, iface, closure_hash.clone()));
    (peer, consumers)
}

fn precompile_tests_per_file(files: &[String]) -> Precompiled {
    use std::collections::HashMap;
    if files.is_empty() {
        return Precompiled::default();
    }
    // NOTE (was `files.len() < 2`): the composed path serves TWO wins, and a single target file benefits from
    // ONE of them. (i) BATCH amortization — lower the shared closure once across N files in this invocation —
    // is genuinely N/A for one file. (ii) The CROSS-INVOCATION PROVIDER CACHE — persist the shared-closure
    // provider so a LATER `cdz test <that file>` is a consumer-only HIT that skips the ~381s closure lower — is
    // exactly the single-file-local-verify win, and it applies whenever ONE file imports a big shared closure
    // (v-compiler-ml verifying a witness against the ~1360-def self-host closure). So we no longer blanket-skip
    // on a single file; the real "nothing to do here" test is whether the closure UNION has a cross-file member
    // — checked below (`asts.len() < 2`) AFTER we gather it, since a lone SELF-CONTAINED file (no imports) has
    // no provider to hoist or cache and must stay on its byte-identical per-file compile.
    // CORRECTNESS GATE (PR#881): a closure file's link name is its dir-BLIND STEM (`program_name` =
    // file_stem). The union below dedups by that stem AND `run_test_file` looks its component up by the same
    // stem — so two DIFFERENT-directory target files with the SAME stem (e.g. two `t.cdz`, or a `lib.cdz` in
    // each of two subdirs) would collapse to one AST and a lookup could fetch the WRONG dir's component,
    // MISATTRIBUTING pass/fail (the best-effort fallback only fires on an ABSENT component, not a
    // present-but-wrong one). So only take the shared-precompile fast path when every target file shares ONE
    // parent directory — then a shared stem means genuinely the same file, and the stem key is unambiguous.
    // Otherwise return empty ⇒ every file falls back to its own per-file compile (correct, just not amortized).
    // `cdz test <dir>` (recursive) is the multi-dir case this guards; a flat single-dir suite keeps the win.
    let parent_of = |p: &str| {
        std::path::Path::new(p)
            .parent()
            .map(|d| d.to_path_buf())
            .unwrap_or_default()
    };
    let first_dir = parent_of(&files[0]);
    if files.iter().any(|f| parent_of(f) != first_dir) {
        return Precompiled::default();
    }
    // GROUP the target files by their genuine SHARED CLOSURE, then compose ONE provider per group (Option-A).
    // WHY NOT one union over all files: a `cdz test <dir>` over a HETEROGENEOUS tree (e.g. compiler-ml/src, 44
    // files importing ~20 distinct libs: parse-db, db, sread-eval, infer-db, …) would fold every file's
    // cross-edges into ONE provider ≈ the whole compiler — the heaviest possible emit, and one un-representable
    // higher-order cross-edge ANYWHERE in that union declines the WHOLE dir to per-file. Grouping by shared
    // closure keeps each provider SMALL + homogeneous (the 9 `sread-eval-*` files → one sread-eval provider; a
    // `conformance-db` file → another) and DECLINE-ISOLATED (a decline drops only its group). It also scopes
    // each cache entry to one closure (a lib change busts only the groups whose closure includes it).
    //
    // The GROUP KEY is the file's IMPORTED-closure name-set (its transitive-closure link names MINUS itself) —
    // computed free from the closure we already load. Keyed by SET EQUALITY, NOT overlap: equality does not
    // re-collapse on a near-universal base (`db` is in almost every closure, so overlap-grouping would merge
    // everything back into one union — the exact defect we're fixing), while equality groups genuinely-identical
    // closures (the homogeneous families the composed path handles). A file with an EMPTY imported set (a
    // self-contained file, no shared closure to hoist) is dropped from grouping → it falls back to standalone.
    let cache_dir = provider_cache_dir();
    // group key (sorted, `\0`-joined imported-closure names) → (union ASTs by link name, an entry name, the
    // TARGET file stems bucketed into this group).
    struct Group {
        asts: HashMap<String, cadenza_compile_abi::Artifact>,
        entry: String,
        // The stems of the TARGET `@test` files that fell into THIS group (their `closure[0].name`). A group's
        // composed emit produces a consumer for EVERY closure member that has `@test`s — but an
        // imported-with-tests member (e.g. `parse-db`: imported into ~10 groups' closures AND a target of its
        // OWN group) is a target of only ONE group. We store its consumer ONLY from its own group (below), so a
        // stem's consumer is never overwritten by a group where it's merely an imported member linked against
        // the WRONG provider (PR#914 correctness — the grouping-era cousin of the PR#881 stem collision).
        targets: std::collections::HashSet<String>,
    }
    // For a SINGLE-file run, keep the loaded closure to SHARE with `run_test_file` (it would otherwise re-load
    // + re-parse the same file — PR#907). Only the single-file case: stashing all N of a dir run would raise
    // peak memory (holding every file's closure at once) for no gain — a dir file loads its closure once in
    // `run_test_file` regardless (it never reuses a sibling's).
    let single = files.len() == 1;
    let mut single_file_closure: Option<std::rc::Rc<Vec<closure::LoadedFile>>> = None;
    let mut groups: HashMap<String, Group> = HashMap::new();
    for f in files {
        let Ok(closure) = load_import_closure_with(f, &|_| None) else {
            continue; // a file that fails to load falls back to its own compile (reports the error located)
        };
        // The entry (element 0) is the target file; the rest are its imported closure. A file with NO imported
        // siblings is self-contained — no provider to hoist/cache, no per-file emit to amortize — so it keeps
        // its byte-identical standalone `EmitTests` path (not grouped). We still stash it (single-file case)
        // so `run_test_file` reuses the parse.
        if closure.len() >= 2 {
            let mut imported: Vec<String> = closure[1..].iter().map(|cf| cf.name.clone()).collect();
            imported.sort();
            imported.dedup();
            let key = imported.join("\0");
            let group = groups.entry(key).or_insert_with(|| Group {
                asts: HashMap::new(),
                entry: closure[0].name.clone(),
                targets: std::collections::HashSet::new(),
            });
            // This file is a TARGET of this group (its @tests are what we run here). Record its stem so we keep
            // only ITS-group consumer, not a same-stem consumer emitted as an imported member of another group.
            group.targets.insert(closure[0].name.clone());
            for cf in &closure {
                group.asts.entry(cf.name.clone()).or_insert_with(|| {
                    cadenza_compile_abi::Artifact::new(
                        cadenza_compile_abi::Artifact::KIND_AST,
                        cf.name.clone(),
                        cadenza_syntax::codec::encode(&cf.arenas),
                    )
                });
            }
        }
        // Stash the single-file closure for reuse (after building any group above — the closure is still owned
        // here; the group only borrowed it). A self-contained single file (`closure.len() < 2`) is stashed too:
        // it skips grouping but `run_test_file` still reuses the parse.
        if single {
            single_file_closure = Some(std::rc::Rc::new(closure));
        }
    }
    if groups.is_empty() {
        // Nothing to compose — but a single-file run still hands its stashed closure to `run_test_file` so the
        // parse isn't repeated (a self-contained single file, or a single importing file whose group declined).
        return Precompiled {
            single_file_closure,
            ..Precompiled::default()
        };
    }

    // Compose each group independently. A group whose composed emit declines contributes no provider/consumers
    // (its files fall back per-file); the others still get their shared provider. Each consumer records WHICH
    // group provider it links against, so `run_test_file` binds it to the right peer.
    let mut precompiled = Precompiled {
        single_file_closure,
        ..Precompiled::default()
    };
    for (key, group) in groups {
        let targets = group.targets;
        let ast_inputs: Vec<cadenza_compile_abi::Artifact> = group.asts.into_values().collect();
        let (provider, consumers) =
            precompile_group(ast_inputs, &group.entry, cache_dir.as_deref());
        let Some(provider) = provider else {
            continue; // group declined / no nameable provider → its files fall back standalone
        };
        precompiled.providers.insert(key.clone(), provider);
        for (name, bytes) in consumers {
            // Keep ONLY consumers for files that are TARGETS of this group. The composed emit produces a
            // consumer for every closure member that has `@test`s, but an imported-with-tests member (e.g.
            // `parse-db`) is a target of just ONE group; storing its consumer from a group where it's only an
            // imported member would OVERWRITE (last-group-wins) its own-group consumer → `run_test_file` links
            // it against the wrong group's provider (PR#914). Filtering by target keeps each stem's consumer
            // from its own group, keyed to the provider whose closure it was actually emitted against.
            if targets.contains(&name) {
                precompiled.components.insert(name, (bytes, key.clone()));
            }
        }
    }
    precompiled
}

/// The directory the shared-closure PROVIDER components are cached in, content-addressed by the closure hash
/// — `$CDZ_PROVIDER_CACHE` if set (and non-empty), else `<default-store>/providers` (the store is already the
/// per-checkout content-addressed artifact dir). Reusing the store dir keeps the cache co-located with the
/// runtime it pairs with + swept by the same tooling. Returns `Option` (the call site degrades to "no cache"
/// on `None`) so a future store-resolution failure can opt out cleanly; today it always resolves to `Some`
/// (`default_store` is infallible), so caching is always available — a write failure is the actual degrade path.
fn provider_cache_dir() -> Option<std::path::PathBuf> {
    if let Ok(d) = std::env::var("CDZ_PROVIDER_CACHE") {
        let d = d.trim();
        if !d.is_empty() {
            return Some(std::path::PathBuf::from(d));
        }
    }
    Some(default_store().join("providers"))
}

/// Enumerate the resolved suite's `@test` definitions as a CADENZA-AST-BINARY value and return — the body
/// of `cdz test --list`. WASMTIME-FREE by construction: it loads each file's import closure, builds the
/// compiler `Db`, and reads `db.test_defs()` — the same front-half `cdz test` runs BEFORE any wasm emit/JIT
/// (`run_test_file`'s enumeration head), stopping there. It compiles nothing and links no runtime, so a
/// `--no-default-features` `cdz` (no `cdz-run`) still produces it.
///
/// Output: the SAME `(test-list (test <name> <is-property> <file>)…)` cadenza-ast value the DELEGATE path
/// (`rcdzc::sidecar` `Query::TestList`, `KIND_TEST_LIST`) emits — one `(test …)` child per test, POSITIONAL:
/// `name` (`Str`), `is-property` (`Bool`), `file` (`Str`) — `codec::encode`d and written verbatim to stdout.
/// This is the operator cadenza-ast-binary-everywhere directive (NO JSON) and keeps `--list` FORMAT-IDENTICAL
/// across the `standalone` (this in-process path) and delegate builds, so v-nix's dynamic-derivations
/// discovery decodes ONE format with the shared `codec` regardless of which `cdz` it invokes. The names come
/// from the `Db`, NOT a regex (the compiler's own source carries `@test` as a parsed token — a regex would
/// massively over-count, per v-test-shred). `is-property` is `!def.params.is_empty() || name.ends_with("-gen")`
/// — a `@test` taking parameters (or the `Test.gen` property wrapper) is a property test; a nullary one is a
/// plain unit test (matches the delegate path's `compile_tests` classification exactly).
///
/// Enumeration mirrors `run_test_file` exactly: a PACKAGE (a file that declares imports) links its whole
/// closure and keeps only the ENTRY file's own `@test`s (an imported library's tests belong to THAT file,
/// counted when it is itself the entry — a directory run visits each); a lone file decodes directly. Dedup
/// is PER FILE (`seen`), matching the run. Order is the resolved-`files` order (path-sorted / manifest
/// order) then declaration order — deterministic, so a drift-guard comparing a fresh `--list` to a
/// committed one is stable. Ignores `--filter`/`--tag`: a manifest must enumerate the WHOLE suite.
fn list_tests(files: &[String], format: ListFormat) -> ExitCode {
    match format {
        // DEFAULT: the canonical cadenza-ast-BINARY `(test-list …)` value, written VERBATIM to stdout (the
        // delegate path's `Query::TestList` bytes are likewise raw; consumers decode with the shared `codec`).
        ListFormat::Binary => match list_test_bytes(files) {
            Ok(bytes) => {
                use std::io::Write as _;
                match std::io::stdout().write_all(&bytes) {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(e) => {
                        eprintln!("{PROG}: --list: could not write the test-list: {e}");
                        ExitCode::FAILURE
                    }
                }
            }
            Err(code) => code,
        },
        // `--format nix`: the eval-readable nix attrset list (v-nix's scoped-cached-IFD discovery source),
        // printed to stdout (the discovery drv redirects to `$out`, a single `import`-able file).
        ListFormat::Nix => match collect_test_entries(files) {
            Ok(entries) => {
                print!("{}", list_test_nix(entries));
                ExitCode::SUCCESS
            }
            Err(code) => code,
        },
    }
}

/// Enumerate the resolved suite's `@test`s and return the `codec::encode`d `(test-list (test <name>
/// <is-property> <file>)…)` cadenza-ast value (the enumeration half of [`list_tests`], factored out so it
/// is unit-testable without capturing stdout). `Err(ExitCode::FAILURE)` on a load/decode/link fault (a
/// broken project cannot be honestly enumerated — failing red is what the drift-guard wants).
fn list_test_bytes(files: &[String]) -> Result<Vec<u8>, ExitCode> {
    // Both `--list` projections (cadenza-ast-binary + `--format nix`) share ONE enumeration; the binary form
    // encodes each collected `(name, is_property, file)` as a `(test …)` child of `(test-list …)`.
    let entries = collect_test_entries(files)?;
    let mut b = cadenza_syntax::Builder::new();
    let mut children: Vec<cadenza_syntax::StructId> = Vec::with_capacity(entries.len() + 1);
    children.push(b.name("test-list"));
    for (name, is_property, file) in &entries {
        let head = b.name("test");
        let name_n = b.atom_leaf(cadenza_syntax::Leaf::Str(name.as_str().into()));
        let isprop_n = b.atom_leaf(cadenza_syntax::Leaf::Bool(*is_property));
        let file_n = b.atom_leaf(cadenza_syntax::Leaf::Str(file.as_str().into()));
        children.push(b.list(vec![head, name_n, isprop_n, file_n]));
    }
    let root = b.list(children);
    Ok(cadenza_syntax::codec::encode(&b.finish(root)))
}

/// A nix STRING literal for `s` — quotes + escapes `"`, `\`, a `${` antiquotation opener, and newlines, so
/// a `@test` name or source path with a special char can't break the emitted (and `import`-ed) nix.
fn nix_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '$' if chars.peek() == Some(&'{') => out.push_str("\\$"),
            '\n' => out.push_str("\\n"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `--list --format nix`: a PURE, IFD-cache-stable nix attrset list — `[ { name = "…"; is_property = …;
/// file = "…"; } … ]` — the eval-readable projection v-nix's scoped-cached-IFD discovery derivation writes to
/// `$out` and the flake `import`s. SORTED by `(file, name)` so an identical `@test` set yields BYTE-IDENTICAL
/// output (the discovery drv is then content-stable — eval re-reads only on a real test add/remove, not
/// ordering noise). Attr names (`name`/`is_property`/`file`) match the emit-shred manifest so the fan-out's
/// `(file-stem, name)` join is clean. Pure: no timestamps/hashed paths, only the enumerated fields.
fn list_test_nix(mut entries: Vec<(String, bool, String)>) -> String {
    entries.sort_by(|a, b| a.2.cmp(&b.2).then_with(|| a.0.cmp(&b.0)));
    let mut s = String::from("[\n");
    for (name, is_property, file) in &entries {
        s.push_str(&format!(
            "  {{ name = {}; is_property = {is_property}; file = {}; }}\n",
            nix_str(name),
            nix_str(file),
        ));
    }
    s.push_str("]\n");
    s
}

/// Enumerate the resolved suite's `@test`s as owned `(name, is_property, file)` tuples — the walk shared by
/// [`list_test_bytes`] (cadenza-ast-binary) and [`list_test_nix`]. Same semantics as [`list_tests`]: follow
/// each file's import closure, build the compiler `Db`, keep only the ENTRY file's own `@test`s in a package
/// (byte-for-byte `run_test_file`'s filter), dedup per file. `is_property` = `!params.is_empty() ||
/// name.ends_with("-gen")` (the delegate `compile_tests` classification). Wasmtime-free.
fn collect_test_entries(files: &[String]) -> Result<Vec<(String, bool, String)>, ExitCode> {
    let mut entries: Vec<(String, bool, String)> = Vec::new();
    for file in files {
        // Follow the file's import closure — the SAME linked program `cdz test`/`cdz check` sees, so a test
        // in a module that imports a sibling enumerates against the same package. A load error is FATAL for
        // `--list` (a broken project cannot be honestly enumerated; failing red is what the drift-guard wants).
        let closure = match load_import_closure_with(file, &|_| None) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("{PROG}: {e}");
                return Err(ExitCode::FAILURE);
            }
        };
        let is_package = !declared_import_paths(&closure[0].arenas).is_empty();
        // Encode each closure file's AST to the canonical binary form (the front-end↔compiler bridge), then
        // build the `Db` the enumeration reads — a package links every file into one arena + loads it WITH
        // its linkage (so `file_of` can scope tests to the entry); a lone file decodes directly.
        let ast_arts: Vec<cadenza_compile_abi::Artifact> = closure
            .iter()
            .map(|f| {
                cadenza_compile_abi::Artifact::new(
                    cadenza_compile_abi::Artifact::KIND_AST,
                    f.name.clone(),
                    cadenza_syntax::codec::encode(&f.arenas),
                )
            })
            .collect();
        let (db, entry_filter) = if is_package {
            let mut rcdzc_files = Vec::with_capacity(ast_arts.len());
            for art in &ast_arts {
                let Some(a) = cadenza_syntax::codec::decode(&art.bytes) else {
                    eprintln!("{PROG}: {file}: could not decode `{}`'s AST", art.name);
                    return Err(ExitCode::FAILURE);
                };
                rcdzc_files.push((art.name.clone(), a));
            }
            let program = match rcdzc::link::link(&rcdzc_files, &closure[0].name) {
                Ok(p) => p,
                Err(r) => {
                    eprintln!("{PROG}: {file}: {}", r.message);
                    return Err(ExitCode::FAILURE);
                }
            };
            let linkage = program.linkage();
            let entry_ix = program.entry;
            let db = rcdzc::db::Db::load_linked(program.arenas, Some(linkage.clone()));
            (db, Some((linkage, entry_ix)))
        } else {
            let Some(rcdzc_arenas) = cadenza_syntax::codec::decode(&ast_arts[0].bytes) else {
                eprintln!("{PROG}: {file}: could not decode the program's AST");
                return Err(ExitCode::FAILURE);
            };
            (rcdzc::db::Db::load(rcdzc_arenas), None)
        };
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for i in db.test_defs() {
            // In a PACKAGE, `test_defs()` sees every linked file's `@test`s — keep only the ENTRY file's own
            // (an imported library's tests are enumerated when it is itself the entry), byte-for-byte the
            // `run_test_file` filter, so `--list` and the run enumerate the identical set.
            if let Some((linkage, entry_ix)) = &entry_filter {
                match linkage.file_of(db.defs[i].sig_occ) {
                    Some(fi) if fi == *entry_ix => {}
                    _ => continue,
                }
            }
            let name = db.defs[i].name.clone();
            if !seen.insert(name.clone()) {
                continue;
            }
            // A `@test` taking parameters (or the `Test.gen` `-gen` property wrapper) is a PROPERTY test (run
            // over generated inputs); a nullary one is a plain unit test. This matches the delegate path's
            // `compile_tests` classification EXACTLY, so `--list` agrees across both builds.
            let is_property = !db.defs[i].params.is_empty() || name.ends_with("-gen");
            entries.push((name, is_property, file.clone()));
        }
    }
    Ok(entries)
}

/// `cdz test --emit-shred` — the compiler-driven test SHRED (the operator model), the body behind the flag.
/// Drives the `EmitTestsShred` sidecar IN-PROCESS (linked `rcdzc`, the same in-process compile the `cdz test`
/// runner uses — no wasmtime, no cdz-run) PER PROJECT FILE (each its own shared-closure GROUP: a multi-file
/// project is NOT one linkable program — independent files don't share an entry, and packages are DAGs), and
/// writes a single FLAT `out_dir/`: `main-<group>.wasm` (each group's emitted library, when it has one) +
/// `test-<name>.wasm` (the per-`@test` components, flat) + ONE `manifest.cdzb` (the merged cadenza-ast-binary
/// manifest). Each group's per-program manifest carries `main-file` = "main.wasm" (has-lib) or "" (standalone);
/// here we REWRITE it to this group's real `main-<group>.wasm` (or keep "" for standalone) and MERGE all
/// groups' entries into the one manifest a runner reads (`cdz-run <target> --call <export> [--peer
/// <main-iface>=<main-file>] --store S`). Compile-only; exits non-zero if any file fails to compile.
fn run_emit_shred(
    files: &[String],
    out_dir: &std::path::Path,
    standalone: bool,
    two_stage: bool,
) -> ExitCode {
    if let Err(e) = std::fs::create_dir_all(out_dir) {
        eprintln!(
            "{PROG}: --emit-shred: cannot create {}: {e}",
            out_dir.display()
        );
        return ExitCode::FAILURE;
    }
    // Mode selection (§S6b): TWO-STAGE (`--two-stage`) emits cadenza-ast FRAGMENTS — one shared-closure
    // `closure-<i>.cdzb` + one per-`@test` `test-<name>.cdzb` — spliced+compiled LATER by the fan-out
    // (`rcdzc closure.cdzb test.cdzb --export <name>`), for standalone-everywhere heavy suites without the
    // O(tests×closure) blowup. STANDALONE (`--standalone`) emits each `@test` as a self-contained WASM
    // component (NO main). Else the shared-main peer WASM shred. `--two-stage` wins if both are set.
    let shred_req = if two_stage {
        cadenza_compile_abi::Request::EmitTestsShredTwoStage
    } else if standalone {
        cadenza_compile_abi::Request::EmitTestsShredStandalone
    } else {
        cadenza_compile_abi::Request::EmitTestsShred
    };
    // The shared-artifact file EXTENSION + per-test target extension: two-stage writes cadenza-ast fragments
    // (`.cdzb`), the wasm modes write components (`.wasm`).
    let ext = if two_stage { "cdzb" } else { "wasm" };
    // The merged manifest's entries, collected across groups as owned fields (each group's arena is dropped
    // before the next): (name, is_property, file, export, target, main-iface, main-file).
    let mut all_entries: Vec<(String, bool, String, String, String, String, String)> = Vec::new();
    // Target FILE basenames already written — so a `@test` name that repeats across files (e.g. choreography's
    // ~3) gets a UNIQUE target file (disambiguated `-<group>`), never overwriting a sibling (the flat layout's
    // one requirement, v-test-shred). The manifest `target` field is rewritten to the unique name it reads.
    let mut written_targets: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut any_fail = false;
    for (i, file) in files.iter().enumerate() {
        // GROUP = one project file + its import closure. Load it, encode each closure file's AST, drive
        // `EmitTestsShred` in-process (link + emit over this group's linked program). A file's closure is its
        // own group; a standalone file (no imports) is a lone-file group (→ possibly no main).
        let closure = match load_import_closure_with(file, &|_| None) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("{PROG}: {e}");
                any_fail = true;
                continue;
            }
        };
        let mut inputs: Vec<cadenza_compile_abi::Artifact> = closure
            .iter()
            .map(|f| {
                cadenza_compile_abi::Artifact::new(
                    cadenza_compile_abi::Artifact::KIND_AST,
                    f.name.clone(),
                    cadenza_syntax::codec::encode(&f.arenas),
                )
            })
            .collect();
        inputs.push(cadenza_compile_abi::Artifact::new(
            cadenza_compile_abi::sidecar::KIND_SIDECAR,
            "drive",
            cadenza_compile_abi::sidecar::encode(std::slice::from_ref(&shred_req)),
        ));
        inputs.push(cadenza_compile_abi::abi::entry_artifact(&closure[0].name));
        let out = rcdzc::run_with_compiler_stack(|| rcdzc::compile(&inputs, &[]));
        // A per-`@test` DECLINE (a compound/closure-param test that can't cross the peer boundary — the
        // deferred #4031 limit) is error-severity, but it is INFORMATIONAL for the shred, NOT a failure: the
        // compile still emits the SHREDDABLE tests + a manifest listing them (the runner runs what shredded +
        // skips the rest). So report the diagnostics (so a decline is visible) but do NOT fail the run or SKIP
        // the file — proceed to take its shreddable output (a file with 3 ok + 2 declined tests still
        // contributes its 3, rather than being dropped whole). `--emit-shred` exits 0 whenever it writes a
        // manifest; only a HARD I/O failure (below) fails it. (`--standalone` has no peer boundary → no
        // declines → this is a clean full shred.)
        if out.has_error() {
            report_errors(&out);
        }
        // ENTRY-SCOPE by the manifest `file` field. A PACKAGE's linked program enumerates EVERY linked file's
        // `@test`s (not just the entry's) — so without this, each file re-emits the WHOLE package's tests
        // (cad: 996 entries for 138 real tests). Each `@test` belongs to its OWN source file's group (emitted
        // when THAT file is the entry), so keep only entries whose `file` == this entry file's stem, and write
        // only those tests' components. An independent-file suite (iterators) has file == entry_stem for all
        // (its closure is just itself), so nothing is dropped there.
        let entry_stem = closure[0].name.clone();
        // The group's SHARED artifact: two-stage → the `closure` ast fragment (→ `closure-<i>.cdzb`); the
        // wasm modes → the `component-provider` main (→ `main-<i>.wasm`). Empty when the group has none (a
        // standalone wasm shred, or a two-stage suite whose closure declined).
        let has_main = if two_stage {
            out.artifacts
                .iter()
                .any(|a| a.kind == cadenza_compile_abi::Artifact::KIND_AST && a.name == "closure")
        } else {
            out.artifacts.iter().any(|a| a.kind == "component-provider")
        };
        let group_main_file = if !has_main {
            String::new()
        } else if two_stage {
            format!("closure-{i}.cdzb")
        } else {
            format!("main-{i}.wasm")
        };
        // Decode the group's manifest → the OWN `@test`s (this file's own, by the `file` field). For each, pick
        // a UNIQUE target FILE name (disambiguate a cross-file name collision with `-<group>`), map the
        // rcdzc consumer artifact name (`test-<name>`) → that unique file, and push the entry with `target`
        // rewritten to it (+ `main-file` → this group's real main / "" standalone). `own` drives the writes.
        let mut own: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        if let Some(m) = out
            .artifacts
            .iter()
            .find(|a| a.kind == cadenza_compile_abi::sidecar::KIND_SHRED_MANIFEST)
        {
            let Some(arenas) = cadenza_syntax::codec::decode(&m.bytes) else {
                eprintln!("{PROG}: --emit-shred: could not decode {file}'s shred manifest");
                any_fail = true;
                continue;
            };
            if let Some(entries) = arenas.as_form(arenas.root, "shred-manifest") {
                for &e in entries {
                    let Some(f) = arenas.as_form(e, "entry") else {
                        continue;
                    };
                    if f.len() != 7 {
                        continue;
                    }
                    let name = arenas.as_str(f[0]).unwrap_or("").to_string();
                    let test_file = arenas.as_str(f[2]).unwrap_or("");
                    if test_file != entry_stem {
                        continue; // an imported file's @test — its OWN group emits it (no cross-file dup)
                    }
                    // Unique target file: `test-<name>.<ext>`, else `test-<name>-<group>.<ext>` on a
                    // cross-file name collision (group index is unique, and within a group `@test` names are
                    // unique). `<ext>` = `cdzb` (two-stage fragment) or `wasm` (compiled component).
                    let mut target = format!("test-{name}.{ext}");
                    if written_targets.contains(&target) {
                        target = format!("test-{name}-{i}.{ext}");
                    }
                    written_targets.insert(target.clone());
                    own.insert(format!("test-{name}"), target.clone());
                    all_entries.push((
                        name,
                        arenas.as_bool(f[1]).unwrap_or(false),
                        test_file.to_string(),
                        arenas.as_str(f[3]).unwrap_or("").to_string(),
                        target,
                        arenas.as_str(f[5]).unwrap_or("").to_string(),
                        group_main_file.clone(),
                    ));
                }
            }
        }
        // Write the group's SHARED artifact (the closure fragment / main provider — only when this file HAS
        // own tests that link it, else it is an orphan) + the OWN per-`@test` artifacts, each to its UNIQUE
        // target file (from `own`). Two-stage artifacts are kind `ast` (`closure` + `test-<name>` fragments);
        // the wasm modes are `component-provider` (main) + `component` (per-test consumer).
        let write_to = |rel: &str, bytes: &[u8], any_fail: &mut bool| {
            let p = out_dir.join(rel);
            if let Err(e) = std::fs::write(&p, bytes) {
                eprintln!("{PROG}: --emit-shred: cannot write {}: {e}", p.display());
                *any_fail = true;
            }
        };
        for a in &out.artifacts {
            if two_stage {
                if a.kind != cadenza_compile_abi::Artifact::KIND_AST {
                    continue;
                }
                if a.name == "closure" {
                    if !own.is_empty() {
                        write_to(&group_main_file, &a.bytes, &mut any_fail);
                    }
                } else if let Some(target) = own.get(&a.name) {
                    write_to(target, &a.bytes, &mut any_fail);
                }
            } else {
                match a.kind.as_str() {
                    "component-provider" if !own.is_empty() => {
                        write_to(&group_main_file, &a.bytes, &mut any_fail)
                    }
                    "component" => {
                        if let Some(target) = own.get(&a.name) {
                            write_to(target, &a.bytes, &mut any_fail)
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    // The MERGED manifest — ONE `(shred-manifest (entry name is-property file export target main-iface
    // main-file)…)` across all groups, `codec::encode`d (the cadenza-ast-binary tooling format).
    let mut b = cadenza_syntax::Builder::new();
    let mut children: Vec<cadenza_syntax::StructId> = Vec::with_capacity(all_entries.len() + 1);
    children.push(b.name("shred-manifest"));
    for (name, is_prop, file, export, target, iface, main_file) in &all_entries {
        let head = b.name("entry");
        let name_n = b.atom_leaf(cadenza_syntax::Leaf::Str(name.as_str().into()));
        let isprop_n = b.atom_leaf(cadenza_syntax::Leaf::Bool(*is_prop));
        let file_n = b.atom_leaf(cadenza_syntax::Leaf::Str(file.as_str().into()));
        let export_n = b.atom_leaf(cadenza_syntax::Leaf::Str(export.as_str().into()));
        let target_n = b.atom_leaf(cadenza_syntax::Leaf::Str(target.as_str().into()));
        let iface_n = b.atom_leaf(cadenza_syntax::Leaf::Str(iface.as_str().into()));
        let mainfile_n = b.atom_leaf(cadenza_syntax::Leaf::Str(main_file.as_str().into()));
        children.push(b.list(vec![
            head, name_n, isprop_n, file_n, export_n, target_n, iface_n, mainfile_n,
        ]));
    }
    let root = b.list(children);
    let manifest_path = out_dir.join("manifest.cdzb");
    if let Err(e) = std::fs::write(
        &manifest_path,
        cadenza_syntax::codec::encode(&b.finish(root)),
    ) {
        eprintln!(
            "{PROG}: --emit-shred: cannot write {}: {e}",
            manifest_path.display()
        );
        any_fail = true;
    }
    if any_fail {
        ExitCode::FAILURE
    } else {
        eprintln!(
            "cdz: shredded {} test(s) into {}",
            all_entries.len(),
            out_dir.display()
        );
        ExitCode::SUCCESS
    }
}

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
        // A single-file target. If it's a COMPILED artifact (`.wasm`) rather than a source file, guide the
        // user instead of the misleading "0 tests found — add `@test`" (a `.wasm` has no source to scan):
        // `cdz test` runs a SOURCE file's `@test`s, the inverse of `cdz run`, which runs the `.wasm`.
        if !is_source_file(&target) && path.extension().and_then(|e| e.to_str()) == Some("wasm") {
            eprintln!(
                "{PROG} test: `{target}` is a COMPILED component, but `cdz test` runs a SOURCE file's \
                 `@test` definitions. Pass the source (`.cdz`/`.ml`/`.sexp`) instead — e.g. `cdz test \
                 src.cdz`; `cdz run {target}` is how you run a compiled component."
            );
            return ExitCode::FAILURE;
        }
        vec![target.clone()]
    };

    // `--list`: ENUMERATE the resolved suite's `@test` names as a cadenza-ast-binary `(test-list …)` value
    // and EXIT — no check-gate, no emit, no JIT, no wasmtime. This is the compiler-informed discovery source
    // v-nix's dynamic-derivations fan-out reads (no committed index, no IFD); it must be cheap and touch NONE
    // of the run machinery below. Short-circuit here, right after resolving `files`, so it shares the exact
    // file-resolution `cdz test` uses (manifest / dir walk / one file) but nothing after it.
    if args.list {
        return list_tests(&files, args.format);
    }
    // `--emit-shred`: shred the suite into per-@test wasm + a manifest (compile-only), then EXIT. Shares the
    // exact file-resolution above; the per-group emit + write is `run_emit_shred`.
    if args.emit_shred {
        let Some(out_dir) = args.out_dir.as_deref() else {
            eprintln!("{PROG} test: --emit-shred requires --out-dir <DIR>");
            return ExitCode::FAILURE;
        };
        return run_emit_shred(&files, out_dir, args.standalone, args.two_stage);
    }

    // GATE ON `cdz check` CLEAN FIRST — before running any `@test`. A source file that fails to PARSE (an
    // unclosed paren, a truncated form) is RECOVERED by the reader (it prints the errors, then hands back a
    // truncated arena of `<error>` placeholders), so the defs that DID parse still compile + run and the
    // suite reports "N passed, 0 failed" while the parse-broken sibling def is SILENTLY ABSENT. That is
    // precisely how a paren-imbalance regression landed GREEN through the fleet-gate `cdz test` step and then
    // blocked the pr-sync queue at the fresh full check (v-syntax's 76-min post-mortem, routed by concierge).
    // `cdz check` already exits non-zero on any error-severity fault (parse OR type), following each file's
    // import closure; run it over the SAME resolved files here and FAIL RED if any has an error, rather than
    // run a suite whose green is a lie. Dedup by canonical path (mirror `run_check`): `check_one` checks a
    // file's whole closure, so a module pulled into an earlier target's closure needn't be re-checked.
    //
    // SKIP the check-gate in `--warm-only` mode: a warm pass runs NO `@test` (it emits+JITs the shared-closure
    // provider into the cache, then exits), so the "green suite is a lie" risk this gate guards against cannot
    // arise — there is no suite. The check itself is expensive (each `check_one` type-checks the file's WHOLE
    // import closure — for a large self-host suite that's the ~570-def closure re-checked, the dominant residual
    // of a warm-once now that the emit is cached), so re-checking here just to immediately exit is pure waste.
    // The ACTUAL per-file `cdz test` sweep that later CONSUMES this warm cache runs its OWN check-gate (this
    // same block, `warm_only=false`), so the false-green protection is preserved exactly where a suite runs.
    if !args.warm_only {
        let canon = |p: &str| {
            std::fs::canonicalize(p)
                .map(|c| c.to_string_lossy().into_owned())
                .unwrap_or_else(|_| p.to_string())
        };
        let mut covered: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut check_failed = false;
        for f in &files {
            let canon_f = canon(f);
            if covered.contains(&canon_f) {
                continue;
            }
            let (had_error, closure_paths) = check_one(f, false, false, false);
            check_failed |= had_error;
            covered.insert(canon_f);
            for path in &closure_paths {
                covered.insert(canon(path));
            }
        }
        if check_failed {
            eprintln!(
                "{PROG} test: the project has errors (above) — NOT running the suite. Any of: a def that \
                 fails to PARSE (silently absent), a def that fails to RESOLVE/TYPE-CHECK, or a file that \
                 fails to READ leaves a suite whose green would be a lie; fix the errors (or run `cdz \
                 check` to see them) first."
            );
            return ExitCode::FAILURE;
        }
    }

    // The runtime store (shared across files). `cdz` runs each test IN-PROCESS — wasmtime + the runner
    // are linked in via the `cdz-run` LIBRARY, not shelled out to a sibling `cdz-run` BINARY — so the
    // one-binary guarantee holds for `cdz test` exactly as it does for `cdz run`: a single `cdz` on the
    // PATH both compiles AND runs the tests, with no second executable to install.
    let store = args.store.clone().unwrap_or_else(default_store);
    let multi = files.len() > 1;

    // Shared-arena lower-once: compile all target files in ONE EmitTestsPerFile pass (lowers the shared
    // closure once, emits one component per file). `run_test_file` looks its component up by name instead of
    // re-lowering the whole closure per file. Best-effort — an empty map (single file, or a union hiccup)
    // just means every file falls back to its own per-file compile, byte-identical to before. This ALSO
    // persists each closure group's provider to the cross-invocation cache — which is exactly the warm a
    // subsequent per-file sweep reuses.
    let precompile_start = std::time::Instant::now();
    let precompiled = precompile_tests_per_file(&files);
    // `--report-time`: the PRECOMPILE phase (per-closure emit — `EmitTestsComposed` on a `.provider.wasm` MISS
    // is the heavy ~270s+ closure LOWER; `EmitTestsConsumerOnly` on a HIT is cheap — plus the `Query::
    // ClosureHash` layout pass). Distinct from the provider JIT below: this pins whether the warm-once cost is
    // the EMIT (provider-cache miss) or the JIT (cwasm miss), the exact split pr-sync needs.
    if args.report_time {
        println!(
            "⏱ precompile: {} shared-closure provider(s) emitted/loaded in {}ms",
            precompiled.providers.len(),
            precompile_start.elapsed().as_millis()
        );
    }

    // JIT each shared-closure PROVIDER ONCE for the whole project, up front — then every file's composition
    // reuses the JIT'd provider `Component` instead of re-JITing it from bytes per file. `Component::new` (the
    // wasmtime JIT) of the heavy closure (the ~1360-def self-host provider) is the DOMINANT per-file startup
    // cost — the "sits there for a bit when each file's tests start" stall — so hoisting it out of the per-file
    // loop makes the project JIT the closure 1×, not N× (the rust-test-harness model: compile the shared code
    // once, then run every test against it). Each file still gets its own thin consumer + its own
    // per-file/per-test PASS/FAIL run below, so localization is untouched — we collapse the JIT, not the
    // reporting. A provider that fails to JIT here is simply omitted → that group's files fall back to their
    // standalone per-file compile in `run_test_file` (best-effort, no worse than before).
    // DESERIALIZE from a persisted cwasm when possible: the group `key` is the closure's content hash, so with
    // a cache dir we use `compile_provider_cached` — it persists the JIT'd artifact content-addressed by
    // (closure-hash ‖ engine fingerprint) and DESERIALIZES it (fast, ~seconds) on a later gate with an
    // unchanged closure, skipping the ~270s cold re-JIT of the heavy self-host closure. This runs BEFORE the
    // `--warm-only` early-return too: `--warm-only` (the gate's serial warm pass) must persist the CWASM, not
    // just the `.provider.wasm` emit — else the per-file sweep workers each cwasm-MISS and re-JIT (the 270s
    // stall stays). So warming = emit-persist (precompile above) + JIT-persist (here). Without a cache dir,
    // fall back to a plain in-process JIT.
    let provider_jit_start = std::time::Instant::now();
    let provider_cwasm_dir = provider_cache_dir();
    let jit_providers: std::collections::HashMap<String, cdz_run::CompiledProvider> = precompiled
        .providers
        .iter()
        .filter_map(|(key, (bytes, iface, content_hash))| {
            // Key the cwasm by the closure CONTENT HASH (not the import-name group `key`), so a content edit
            // invalidates it. Only cache when we HAVE a content hash + a cache dir; else plain in-process JIT.
            let compiled = match (&provider_cwasm_dir, content_hash) {
                (Some(dir), Some(hash)) => {
                    cdz_run::compile_provider_cached(bytes, iface.clone(), dir, hash)
                }
                _ => cdz_run::compile_provider(bytes, iface.clone()),
            };
            compiled.ok().map(|p| (key.clone(), p))
        })
        .collect();

    // `--report-time`: the PROJECT-WIDE provider JIT/deserialize — the dominant cost, paid ONCE here (the
    // provider-JIT-once fix) rather than per file. On a cwasm HIT this is a fast deserialize (~seconds); on a
    // MISS it's the full ~270s JIT. Printed BEFORE the `--warm-only` return so a warming run ALSO shows it —
    // the gate warms via `--warm-only`, so this line is how pr-sync/the operator see whether the warm step
    // itself HIT the cwasm (fast) or had to re-JIT (slow) it.
    if args.report_time && !jit_providers.is_empty() {
        println!(
            "⏱ provider JIT: {} shared closure(s) JIT'd/loaded once in {}ms",
            jit_providers.len(),
            provider_jit_start.elapsed().as_millis()
        );
    }
    // `--warm-only`: the emit cache (`.provider.wasm`, precompile above) AND the JIT cache (`.cwasm`, the
    // provider-JIT just above) are now both persisted. Stop here WITHOUT running the tests — a subsequent
    // per-file sweep HITS both (skips the closure emit AND the ~270s re-JIT). Report what warmed.
    if args.warm_only {
        let groups = precompiled.providers.len();
        let jitted = jit_providers.len();
        println!(
            "warmed {groups} shared-closure provider(s) — {jitted} JIT-cached (cwasm) — into the cache \
             ({} target file(s) across the suite); a per-file `cdz test` sweep will now reuse both",
            files.len()
        );
        return ExitCode::SUCCESS;
    }
    let pre = PrecompiledRun {
        precompiled: &precompiled,
        jit_providers: &jit_providers,
        report_time: args.report_time,
    };

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
            &store,
            args.trials,
            args.seed,
            &pre,
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
    // A SINGLE explicit `cdz test <file>` that found ZERO tests is almost always a mistake — the user meant
    // to test something (e.g. wrote an UNKNOWN test-ish annotation like `@property`, which is silently
    // stripped so its def is not a test, leaving the file with no `@test`). Without a note this exits 0 with
    // NO output — a whole file can be dead + "green" by omission (breaker's silent-no-op finding). Print a
    // hint (still exit 0 — an empty file is not a failure, and this must not red the storeless library case).
    // Only for a single explicit file: a DIRECTORY/package run legitimately has test-free library modules,
    // and per-file "0 tests" there would be noise (each already headed by its path). `@test` is the property
    // spelling (a parameterized `@test`); `@property` is NOT a supported annotation (operator ruling).
    if !multi
        && total_pass == 0
        && total_fail == 0
        && !any_error
        && let Some(file) = files.first()
    {
        // Distinguish "no @test at all" from "a --tag/--filter EXCLUDED every test". Blaming a missing
        // `@test` when the real cause is an over-narrow selector (e.g. a typo'd `--tag`) points the user at
        // the wrong fix — the file may be full of tests the filter skipped. Only the unfiltered case is a
        // genuine "add a `@test`" situation.
        match (args.tag.as_deref(), args.filter.as_deref()) {
            // BOTH selectors present: they AND-compose, so either (or their intersection) could be empty.
            // Don't falsely blame one (a matching `--tag` with a missing `--filter` would be mis-reported) —
            // name both and point at their empty intersection.
            (Some(t), Some(f)) => println!(
                "0 tests matched `--tag {t}` AND `--filter {f}` in {file} — no `@test` both carries \
                 `@tag(\"{t}\")` and has a name containing `{f}` (loosen or drop a selector)."
            ),
            (Some(t), None) => println!(
                "0 tests matched `--tag {t}` in {file} — no `@test` carries that `@tag(\"{t}\")` (check for a \
                 typo, or drop `--tag` to run every test)."
            ),
            (None, Some(f)) => println!(
                "0 tests matched `--filter {f}` in {file} — no `@test` name contains that substring (check \
                 for a typo, or drop `--filter` to run every test)."
            ),
            (None, None) => println!(
                "0 tests found in {file} — a test needs the `@test` annotation (a parameterized `@test` is a \
                 property test); an unrecognized annotation is silently ignored."
            ),
        }
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
/// The project-wide precompiled state a `cdz test` run threads into each file's [`run_test_file`]: the
/// per-closure grouping's components/providers ([`Precompiled`]) PLUS the providers JIT'd ONCE up front
/// (shared across every file so the heavy closure isn't re-JIT'd per file). Bundled so `run_test_file` takes
/// one context arg instead of two parallel maps.
struct PrecompiledRun<'a> {
    precompiled: &'a Precompiled,
    jit_providers: &'a std::collections::HashMap<String, cdz_run::CompiledProvider>,
    /// `--report-time`: emit per-phase (compose/run) + per-test durations (like `cargo test --report-time`).
    report_time: bool,
}

fn run_test_file(
    file: &str,
    filter: Option<&str>,
    tag: Option<&str>,
    store: &std::path::Path,
    trials: u64,
    seed: u64,
    pre: &PrecompiledRun<'_>,
) -> Result<(usize, usize), ()> {
    let precompiled = pre.precompiled;
    let jit_providers = pre.jit_providers;
    // Follow the entry file's IMPORT CLOSURE so a test in a module that imports a sibling (e.g. a pass
    // that reuses another module's type) resolves + runs — `cdz test FILE` sees the SAME linked program
    // `cdz check FILE` does. A file that imports nothing loads as a lone file, byte-identical to a
    // standalone single-file test compile; only a file carrying an `(import …)` pulls its siblings in.
    //
    // REUSE the closure `precompile_tests_per_file` already loaded for a SINGLE-file run (PR#907 — avoid
    // re-parsing the same file's whole closure twice). The stash is `Some` only for a single-file `cdz test
    // <file>` (a dir run loads each file's closure once here, never a sibling's); `Rc` so this is a refcount
    // bump. A multi-file run (or a defensive `None`) loads fresh, byte-identical to before.
    let loaded;
    let closure: &[closure::LoadedFile] = match &precompiled.single_file_closure {
        Some(rc) => rc.as_slice(),
        None => {
            loaded = match load_import_closure_with(file, &|_| None) {
                Ok(f) => f,
                Err(e) => {
                    eprintln!("{PROG}: {e}");
                    return Err(());
                }
            };
            &loaded
        }
    };
    let is_package = !declared_import_paths(&closure[0].arenas).is_empty();

    // Encode each closure file's `ast` ONCE — the per-file artifacts feed BOTH the `Db` that enumerates
    // the ENTRY file's `@test` names and the package emit compile below. The front-end (`cadenza_syntax`)
    // and compiler (`rcdzc`) have DISTINCT arena types; the canonical binary form is the bridge.
    let ast_arts: Vec<cadenza_compile_abi::Artifact> = closure
        .iter()
        .map(|f| {
            cadenza_compile_abi::Artifact::new(
                cadenza_compile_abi::Artifact::KIND_AST,
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
            let Some(a) = cadenza_syntax::codec::decode(&art.bytes) else {
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
        let Some(rcdzc_arenas) = cadenza_syntax::codec::decode(&ast_arts[0].bytes) else {
            eprintln!("{PROG}: {file}: could not decode the program's AST");
            return Err(());
        };
        (rcdzc::db::Db::load(rcdzc_arenas), None)
    };
    // Each test's name PLUS the generators for its parameters (empty = a plain nullary test, run once;
    // non-empty = a PROPERTY test, run `trials` times with generated inputs). A param whose type is not a
    // generatable scalar makes `param_generators` return `None` — reported per test, not aborting the run.
    let mut tests: Vec<TestSpec> = Vec::new();
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
        // `@requires` bounds for constrained generation — captured before `param_generators` re-borrows `db`
        // mutably. Only meaningful when the test is a boundary-arg (scalar-param) property; the -gen wrapper
        // path (compound params) draws internally and isn't clamped here (a later increment).
        let (bounds, relations) = param_bounds(&db, i);
        let gens = param_generators(&mut db, i);
        // For a `-gen` wrapper, capture its parameter `GenTy` now (db is in scope here, not in the run loop).
        // Only meaningful for the exhaustive-newtype enumeration below; a non-wrapper test yields `None`.
        let gen_ty = name
            .ends_with("-gen")
            .then(|| rcdzc::proptest_gen::gen_ty_of_wrapper_param(&db, &name))
            .flatten();
        tests.push(TestSpec {
            name,
            gens,
            exhaustive,
            bounds,
            relations,
            gen_ty,
        });
    }
    if tests.is_empty() {
        // No matching `@test` here. A file with no tests (e.g. a pure library module in a package dir, or
        // a `--filter` that selects nothing) is vacuously green — return (0, 0) and print nothing, so a
        // directory run aggregates without a spurious error line per test-free file.
        return Ok((0, 0));
    }

    // The test component. FAST PATH (Option-C composed): if the shared-arena precompile produced this file's
    // CONSUMER component (keyed by its link name) AND a shared-closure PROVIDER peer, use them — the consumer
    // imports the closure from the provider, so the whole closure was emitted ONCE (in the provider) instead
    // of re-embedded here. SLOW PATH (miss — single file, decline, multi-dir stem-collision, or the file
    // wasn't in the composed set): compile this file alone with an `EmitTests` request, exactly as before
    // (`layout::compute_tests`; a package's `entry` marker drives linking). A per-file DECLINE is reported
    // located here (the fallback owns error reporting — the precompile does not).
    // The composed consumer + shared provider for this file, if the precompile produced them. The composition
    // is JIT-compiled ONCE below (`compile_composition`) and reused across every trial, so a multi-trial
    // property test does NOT re-JIT per trial (PR#892 (a) — the earlier `has_multi_trial` fall-back guard is
    // obsolete now that the composed path reuses the JIT like the standalone path does).
    // Look up this file's consumer + the GROUP provider it links against (Option-A per-closure grouping — a
    // consumer records its group key, indexing `providers`). A consumer present but whose group provider is
    // absent (shouldn't happen — they're inserted together — but degrade safely) falls back per-file.
    let composed =
        precompiled
            .components
            .get(&closure[0].name)
            .and_then(|(consumer, group_key)| {
                precompiled.providers.get(group_key).map(
                    |(provider_bytes, iface, _content_hash)| {
                        (consumer.clone(), provider_bytes, iface, group_key.as_str())
                    },
                )
            });
    let component: Vec<u8> = if let Some((consumer, _, _, _)) = &composed {
        consumer.clone()
    } else {
        let mut inputs = ast_arts;
        inputs.push(cadenza_compile_abi::Artifact::new(
            cadenza_compile_abi::sidecar::KIND_SIDECAR,
            "drive",
            cadenza_compile_abi::sidecar::encode(&[cadenza_compile_abi::Request::EmitTests]),
        ));
        if is_package {
            inputs.push(cadenza_compile_abi::abi::entry_artifact(&closure[0].name));
        }
        let out = rcdzc::run_with_compiler_stack(|| rcdzc::compile(&inputs, &[]));
        let Some(component) = out.artifact("component") else {
            // The test compile declined — report its errors (a parameterized `@test`, an ill-typed test body,
            // an invalid-kebab `@test` name, …). We HOLD the closure files (source + spans), so render each
            // fault at its `file:line:col` (the located reporter), not the bare `cdz: error …` — an anchored
            // decline (e.g. CDZ0201 on a bad `@test` name) then points at the name occurrence like `cdz check`.
            report_errors_located(&out, closure);
            return Err(());
        };
        component.to_vec()
    };

    // DEBUG (CDZ_DUMP_TEST_WASM): write the emitted test component to that path, for a WAT-diff of the
    // instantiation-set-dependent emit (bug#4). Throwaway.
    if let Ok(path) = std::env::var("CDZ_DUMP_TEST_WASM") {
        // Report the write outcome honestly — don't print "wrote …" when the write FAILED (a swallowed
        // permission/path error made this debug dump claim false success; PR#584 nit).
        match std::fs::write(&path, &component) {
            Ok(()) => eprintln!(
                "[dump] wrote test component ({} bytes) to {path}",
                component.len()
            ),
            Err(e) => eprintln!("[dump] FAILED to write test component to {path}: {e}"),
        }
    }

    // Resolve the value-heap runtime ONCE for this file's test component (reused across every test + trial):
    // the component records the exact runtime hash it was emitted against, and we read `<store>/<hash>.wasm`
    // BY CONTENT ADDRESS — the same resolution `cdz run` uses. A scalar/const test component imports no
    // runtime, so `required_runtime` returns `None` and we run with no runtime (no store needed). A missing
    // store entry is reported here, once, rather than as a trap inside each test.
    //
    // COMPOSED path: the consumer is CROSS-EDGE-EXCLUDING — the heap-using shared closure was hoisted into the
    // PROVIDER — so a consumer can import NO runtime while its provider peer DOES (e.g. a cad test whose heap
    // ops all live in the shared closure). `run_composition` composes ONE runtime for whichever of consumer OR
    // peer declares it (they pin the SAME runtime by content hash), reading the bytes from `opts.runtime`. So
    // we must resolve the runtime from EITHER component: try the consumer first, then fall back to the
    // provider. Resolving from only the consumer (as the standalone path does) left `opts.runtime = None` for a
    // consumer that imports no runtime but whose provider requires it → "requires the value-heap runtime …
    // but none was provided" for every grouped cad test (the reject). A shared runtime is a single instance, so
    // either source's identical bytes serve both.
    let runtime = {
        let consumer_rt = match resolve_test_runtime(&component, store) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("{PROG}: {file}: {e}");
                return Err(());
            }
        };
        match (consumer_rt, composed.as_ref()) {
            // Consumer already pins the runtime — use it (same bytes the provider would resolve to).
            (Some(rt), _) => Some(rt),
            // Consumer imports no runtime but a provider peer may: resolve from the provider bytes.
            (None, Some((_, provider_bytes, _, _))) => {
                match resolve_test_runtime(provider_bytes, store) {
                    Ok(r) => r,
                    Err(e) => {
                        eprintln!("{PROG}: {file}: {e}");
                        return Err(());
                    }
                }
            }
            // Standalone (or composed with a runtime-free provider): no runtime needed.
            (None, _) => None,
        }
    };

    // Build the per-trial RUN TARGET, JIT-compiling ONCE + reusing across every test + trial — `Component::new`
    // (the wasmtime JIT) is the DOMINANT per-run cost (~8s for the self-host component vs ~0.1s to run it), so
    // compiling once instead of once-per-trial is the whole point. STANDALONE (common): the self-contained
    // component. COMPOSED (Option-C): the consumer + its shared-closure provider peer, JIT'd into ONE
    // `CompiledComposition` (consumer + peer Components) reused per trial — so a multi-trial property test
    // does NOT re-JIT the composition per trial (PR#892 materialize-once fix). Both reuse across trials.
    // `--report-time`: time the COMPOSE phase (this file's consumer JIT — the provider was JIT'd once up front).
    let compose_start = std::time::Instant::now();
    let target = if let Some((consumer, provider_bytes, iface, group_key)) = composed {
        // Prefer the PROJECT-WIDE pre-JIT'd provider (JIT'd ONCE in `run_test`, shared across every file) so
        // the heavy shared closure is not re-JIT'd per file — the per-file startup-stall fix. Only the thin
        // consumer is JIT'd here. Fall back to JITing the provider from bytes if it's somehow absent from the
        // map (a provider that failed to pre-JIT, or a caller that didn't pre-JIT) — behavior-identical, just
        // without the reuse. Both paths produce the SAME `CompiledComposition`, so the run is unchanged.
        let composition = match jit_providers.get(group_key) {
            Some(jit_provider) => cdz_run::compile_composition_with_providers(
                &consumer,
                std::slice::from_ref(jit_provider),
            ),
            None => cdz_run::compile_composition(
                &consumer,
                &[cdz_run::Peer {
                    bytes: provider_bytes.clone(),
                    interface: iface.clone(),
                }],
            ),
        };
        match composition {
            Ok(c) => RunTarget::Composed(c),
            Err(e) => {
                eprintln!("{PROG}: {file}: could not compile the composed test component: {e:#}");
                return Err(());
            }
        }
    } else {
        match cdz_run::compile_component(&component) {
            Ok(c) => RunTarget::Standalone(c),
            Err(e) => {
                eprintln!("{PROG}: {file}: could not compile the test component: {e:#}");
                return Err(());
            }
        }
    };

    // Run each test IN-PROCESS (via the `cdz-run` library — no sibling binary), in declaration order. A
    // NULLARY test runs ONCE — PASS = the export returned, FAIL = it trapped. A PROPERTY test (parameters)
    // runs `trials` times with generated inputs; it PASSES only if every trial returns, and FAILS on the
    // first trapping trial — reported with the failing inputs (shrunk toward a minimal counterexample) + the
    // seed to replay. The runtime cache dir is the store, so the JIT-compiled runtime is reused per trial.
    let compose_ms = compose_start.elapsed().as_millis();
    let run_start = std::time::Instant::now();

    let mut passed = 0usize;
    let mut failed = 0usize;
    for TestSpec {
        name,
        gens,
        exhaustive,
        bounds,
        relations,
        gen_ty,
    } in &tests
    {
        let kebab = cadenza_syntax::extern_name::kebab_extern_name(name);
        let run_one = |arg_vals: &[String]| -> TrialOutcome {
            run_one_trial(&target, runtime.as_deref(), &kebab, store, arg_vals)
        };
        // `--report-time`: per-TEST duration. Snapshot the fail-counter + a timer around this test's run;
        // after its `match` arm prints PASS/FAIL, emit a ` ⏱ {name} {ms}ms` line (like `cargo --report-time`).
        let test_start = std::time::Instant::now();
        let fail_before = failed;
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
            // An `@exhaustive` over a BOUNDED `@invariant` NEWTYPE (`Percent = Pct(Int64)` with
            // `@invariant [0,100]`) CAN be enumerated: its `-gen` wrapper param is a single-variant `Sum`
            // whose payload is an `IntRange{lo,hi}`, and the `IntRange` decode map `v = lo + (pool & MAX) %
            // span` is INVERTIBLE — feeding pool int `v-lo` drives the wrapper over the exact value `v`. So
            // run it once per `v in lo..=hi` (a PROOF over the in-domain set), if `span` fits the cap. Any
            // other compound shape (a List/Tuple/multi-variant sum, or a too-wide range) falls through to the
            // clean decline below.
            Some(gens)
                if gens.is_empty()
                    && *exhaustive
                    && exhaustive_newtype_range(gen_ty.as_ref()).is_some() =>
            {
                let (lo, hi) = exhaustive_newtype_range(gen_ty.as_ref()).unwrap();
                // Render via the WHOLE Sum GenTy (it consumes selector then payload, matching the pool below),
                // so a failing case decodes to `S(v)`, the full nominal value.
                let full_gt = gen_ty.as_ref().unwrap();
                let span = (hi - lo + 1) as usize;
                let run_pool = |pool: &[i64]| -> TrialOutcome {
                    run_one_trial_with_pool(&target, runtime.as_deref(), &kebab, store, &[], pool).0
                };
                // The `-gen` wrapper for a single-variant `Sum` newtype draws a variant SELECTOR first
                // (`sel = gen % k`, here k=1 → any int selects the sole variant), THEN the `IntRange` payload
                // (`v = lo + (gen & MAX) % span`). So the pool for value `v` is `[selector=0, v-lo]` — mirror
                // the decode order exactly (a 1-element pool would run dry on the payload draw → a spurious
                // body trap). `pool_for(v)` builds it; `render_pool_value` decodes the SAME pool to `S(v)`.
                let pool_for = |v: i64| -> [i64; 2] { [0, v.wrapping_sub(lo)] };
                let failing =
                    (lo..=hi).find(|&v| matches!(run_pool(&pool_for(v)), TrialOutcome::Fail(_)));
                match failing {
                    None => {
                        passed += 1;
                        println!("PASS {name} (exhaustive, {span} cases)");
                    }
                    Some(v) => {
                        failed += 1;
                        // Render the failing case as the wrapper's decoded VALUE (`S(2)`), not a raw pool int.
                        let rendered = render_pool_value(full_gt, &pool_for(v))
                            .unwrap_or_else(|| v.to_string());
                        let msg = match run_pool(&pool_for(v)) {
                            TrialOutcome::Fail(Some(m)) => format!(": {m}"),
                            _ => String::new(),
                        };
                        println!(
                            "FAIL {name}{msg}\n  counterexample: {name}({rendered})  (exhaustive — the \
                             domain contains a failing case)"
                        );
                    }
                }
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
            // performs `Test.gen-int`: a GENERATOR-DRIVEN property test (a nullary wrapper that pulls random
            // ints from the runner to build its own inputs — the compound/int-stream route) vs a plain
            // unit test (pulls no generated int). Decide it by RUNNING once under a seeded int pool and
            // counting the `Test.gen-int` calls the guest made.
            Some(gens) if gens.is_empty() => {
                match run_gen_driven(
                    &target,
                    runtime.as_deref(),
                    &kebab,
                    store,
                    trials,
                    seed,
                    gen_ty.as_ref(),
                ) {
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
                        // Prefer the CONCRETE VALUE the shrunk pool decodes to (e.g. `never-three([0,0,0])`)
                        // over the raw driver ints: recover the wrapper's original compound parameter type
                        // (the pre-synthesis def `<name-without-gen>` survives, `@test`-stripped, with its
                        // param type intact) and re-run the SAME generator derivation over the shrunk pool.
                        // Falls back to the raw-int line when the type can't be recovered/decoded (a shape
                        // the decoder doesn't yet render) — never a wrong value.
                        let pool_ints: Vec<i64> =
                            fail.inputs.iter().filter_map(|s| s.parse().ok()).collect();
                        let rendered = rcdzc::proptest_gen::gen_ty_of_wrapper_param(&db, name)
                            .and_then(|gty| render_pool_value(&gty, &pool_ints));
                        match rendered {
                            // Render the counterexample as a call to the ORIGINAL test (the `-gen` suffix is
                            // the synthesized-wrapper detail; `never_three([0,0,0])` reads as the user wrote
                            // it), while the `FAIL` line keeps the wrapper name the runner reports throughout.
                            Some(value) => {
                                let orig = name.strip_suffix("-gen").unwrap_or(name);
                                println!(
                                    "FAIL {name}{msg}\n  counterexample: {orig}({value})  (seed {seed}; \
                                     replay with `--seed {seed}`)"
                                )
                            }
                            None => {
                                let pool = fail
                                    .inputs
                                    .iter()
                                    .map(|s| s.as_str())
                                    .collect::<Vec<_>>()
                                    .join(", ");
                                println!(
                                    "FAIL {name}{msg}\n  counterexample: generated ints [{pool}]  \
                                     (seed {seed}; replay with `--seed {seed}`)"
                                );
                            }
                        }
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
            Some(gens) => match run_property(gens, bounds, relations, trials, seed, &run_one) {
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
        // Per-test duration (like `cargo test --report-time`) — a compact line under the test's PASS/FAIL,
        // emitted only under `--report-time` so the default output is unchanged. Label the outcome so a slow
        // PASS and a slow FAIL are both attributable at a glance.
        if pre.report_time {
            let outcome = if failed > fail_before { "FAIL" } else { "PASS" };
            println!(
                "  ⏱ {outcome} {name} {}ms",
                test_start.elapsed().as_millis()
            );
        }
    }

    // Per-STEP timing for this file (compose = this file's consumer JIT; run = all its tests) — the "where the
    // time goes" breakdown the operator asked for. The heavy shared-closure provider JIT is NOT here — it's
    // paid ONCE up front in `run_test` (reported there), which is the whole point of the provider-JIT-once fix.
    if pre.report_time {
        println!(
            "  ⏱ {file}: compose {compose_ms}ms · run {}ms",
            run_start.elapsed().as_millis()
        );
    }

    println!("\n{passed} passed, {failed} failed");
    Ok((passed, failed))
}

// ── cdz watch ──────────────────────────────────────────────────────────────────────────────────

/// Is `path` a Cadenza SOURCE file (or the project manifest) — the only changes `cdz watch` re-runs on?
/// A POSITIVE filter (source extensions + `Project.cdz`) is what keeps `watch` from self-triggering: a
/// `watch build`/`watch test` writes `.wasm`/`.rs`/`.dwarf`/`link-map.txt`/`.cdz-run-*` artifacts INTO the
/// watched dir, and an editor churns swap/temp files — none of those are source, so none re-fire the run.
#[cfg(feature = "watch")]
fn is_watch_trigger(path: &std::path::Path) -> bool {
    if path.file_name().and_then(|n| n.to_str()) == Some(MANIFEST_NAME) {
        return true;
    }
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("cdz" | "ml" | "sexp" | "sexpr")
    )
}

/// `cdz watch [target] --exec <check|test|build>` — the `cargo watch` analogue. Resolve the project's
/// manifest directory (the same `Project.cdz` `cdz build`/`test` use, searching upward when omitted),
/// watch that directory recursively, and RE-RUN the chosen command whenever a SOURCE file (or the
/// manifest) changes. The loop runs the command once up front (initial feedback, like `cargo watch`),
/// then blocks on the filesystem event channel; on a source-file event it DEBOUNCES (keeps draining
/// events for `debounce_ms` so a burst of saves — or an editor's write-then-rename — coalesces into ONE
/// run), re-runs SYNCHRONOUSLY (so runs never overlap — the concierge guard), then inspects the events
/// that arrived DURING the run: artifact-only churn (the run's own outputs) is discarded, but a SOURCE
/// edit made mid-run is NOT reflected in the run that just finished, so it triggers one more re-run
/// rather than being dropped until the next event. Ctrl-C exits. The re-run itself is the ordinary
/// in-process `run_check`/`run_test`/`run_build`/`run_project`.
#[cfg(feature = "watch")]
fn run_watch(args: &WatchArgs) -> ExitCode {
    use notify::{RecursiveMode, Watcher};
    use std::sync::mpsc;
    use std::time::Duration;

    // Resolve the project dir to watch (no entry requirement — `check` needs none; `build`/`test` report
    // their own missing-entry error on the re-run). This validates the target up front so `cdz watch` on a
    // manifest-less dir fails immediately rather than watching nothing.
    let (dir, _mpath, m) = match resolve_project_manifest(args.target.as_deref(), "cdz watch") {
        Ok(v) => v,
        Err(code) => return code,
    };

    // The re-run closure: construct the chosen command's Args targeted at the resolved manifest dir, and
    // invoke the ordinary handler in-process. `--store` threads through to the commands that resolve the
    // value-heap runtime (`test`/`run`); `check`/`build` don't take a store. Returns the command's code.
    let store = args.store.clone();
    let call = args.call.clone();
    let run_args = args.args.clone();
    let filter = args.filter.clone();
    let tag = args.tag.clone();
    let trials = args.trials;
    let seed = args.seed;
    // Clear the terminal before each run (`--clear`, like `cargo watch -c`). A `Copy` bool captured
    // separately from `args` (which is moved into the `rerun` closure below). The clear is emitted BEFORE
    // the run's banner so each run's output starts on a fresh screen; a no-op when `--clear` is unset.
    let clear = args.clear;
    let clear_screen = move || {
        if clear {
            use std::io::Write;
            print!("\x1b[2J\x1b[H"); // ANSI: erase display + move cursor home
            let _ = std::io::stdout().flush();
        }
    };
    let dir_str = dir.to_string_lossy().into_owned();
    let rerun = move || -> ExitCode {
        match args.exec {
            WatchCmd::Check => run_check(&CheckArgs {
                file: Some(dir_str.clone()),
                json: false,
                verify_fixes: false,
                diagnostics_wire: false, // watch is an interactive re-check; the raw grader wire is a one-shot mode
            }),
            WatchCmd::Test => run_test(&TestArgs {
                file: Some(dir_str.clone()),
                filter: filter.clone(),
                tag: tag.clone(),
                store: store.clone(),
                trials,
                seed,
                warm_only: false, // watch RUNS the tests on each change, never a warm-only pass
                report_time: false, // watch is an interactive re-run; timing is an opt-in of a direct run
                list: false, // watch RE-RUNS the suite; enumeration-and-exit is a one-shot direct-run mode
                format: ListFormat::Binary, // moot when list=false
                emit_shred: false, // watch RE-RUNS; the shred build-output is a one-shot direct-run mode
                out_dir: None,
                standalone: false,
                two_stage: false,
            }),
            WatchCmd::Build => run_build(&BuildArgs {
                dir: Some(dir_str.clone()),
                out: None,
                release: false,
                opt_level: None,
                target: BuildTargetArg::Wasm,
            }),
            // `run` in PROJECT mode: `component = the dir` routes through `run_project` (build the entry,
            // then run it), the same path `cdz run <dir>` takes. `store` threads through for a heap run.
            // A watch `run` sets only the entry + its interactive call/args/store; every other flag takes
            // its default (no grade, no leak-ceiling, no verdict/diagnostics wire, sexp render). The spread
            // keeps this site compiling when a new `RunArgs` field is added (the cross-crate E0063 class).
            WatchCmd::Run => run_project(&cdz_run::cli::RunArgs {
                component: Some(std::path::PathBuf::from(&dir_str)),
                call: call.clone(),
                args: run_args.clone(),
                store: store.clone(),
                ..Default::default()
            }),
        }
    };

    let label = match args.exec {
        WatchCmd::Check => "check",
        WatchCmd::Test => "test",
        WatchCmd::Build => "build",
        WatchCmd::Run => "run",
    };

    // Set up the recursive watch on the manifest directory. `notify`'s recommended watcher is the
    // platform-native backend (inotify/FSEvents/kqueue). Events flow over an mpsc channel.
    let (tx, rx) = mpsc::channel();
    let mut watcher = match notify::recommended_watcher(move |res| {
        // A send failure only means the receiver was dropped (we're exiting) — nothing to do.
        let _ = tx.send(res);
    }) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("{PROG}: cannot create a filesystem watcher: {e}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(e) = watcher.watch(&dir, RecursiveMode::Recursive) {
        eprintln!("{PROG}: cannot watch {}: {e}", dir.display());
        return ExitCode::FAILURE;
    }
    // ALSO watch each PATH DEPENDENCY's directory, so editing a dep's source re-triggers the run — the
    // multi-project edit loop (`cdz watch --exec run` on a project with `def deps`). The dep dir is
    // resolved relative to this manifest's dir (same as `build_path_deps`). A dep whose dir can't be
    // watched (e.g. it doesn't exist yet) is noted but NOT fatal — the run itself reports an unresolvable
    // dep; here we just skip watching it. The source-file filter still gates re-runs, so a dep's own
    // `.wasm` build artifacts don't self-trigger.
    for dep in &m.deps {
        #[allow(clippy::infallible_destructuring_match)]
        let dep_path = match dep {
            DepSource::Path(p) => p,
        };
        let dep_dir = dir.join(dep_path);
        if let Err(e) = watcher.watch(&dep_dir, RecursiveMode::Recursive) {
            eprintln!(
                "{PROG}: note: not watching dependency `{dep_path}` ({}): {e}",
                dep_dir.display()
            );
        }
    }

    let debounce = Duration::from_millis(args.debounce_ms);
    eprintln!(
        "{PROG}: watching {} — re-running `cdz {label}` on change (Ctrl-C to stop)",
        dir.display()
    );

    // Whether a batch of filesystem events touched a SOURCE file / the manifest — the only changes that
    // warrant a re-run. Artifact writes (a `build`/`run`'s own `.wasm`/`link-map.txt`) and editor temp
    // churn are ignored, so they never self-trigger.
    let batch_touches_source = |batch: &[notify::Result<notify::Event>]| -> bool {
        batch.iter().any(|res| {
            res.as_ref()
                .map(|ev| ev.paths.iter().any(|p| is_watch_trigger(p)))
                .unwrap_or(false)
        })
    };

    // 1. Initial run (once — the initial feedback, like `cargo watch`).
    clear_screen();
    let _ = rerun();

    // Drain the STARTUP event burst before arming the change loop. macOS FSEvents delivers a spurious
    // create/coalesced event for the pre-existing watched directory right after the watch begins (Linux
    // inotify does not) — without this drain, that event would trip the loop's change path and fire a
    // SPURIOUS extra run on startup (a real double-build on macOS). Give FSEvents a brief moment to emit
    // that burst, then discard everything queued. The settle is SHORT + FIXED (not per-event) and bounded
    // to the startup window, so it can't swallow a user's later edit — a real change after this returns to
    // the normal blocking `recv` below. (The startup burst lands within a few ms of arming the watch; a
    // 150ms settle covers it well under the test/debounce windows without adding meaningful startup lag.)
    std::thread::sleep(Duration::from_millis(150));
    while rx.try_recv().is_ok() {}

    // 2/3. Event loop: block for a change, debounce-coalesce, re-run, then check whether MORE source
    // edits arrived DURING the run — if so, run again (a mid-run save must not be lost).
    let mut pending = false; // a source edit seen while a run was in flight, not yet acted on
    loop {
        if pending {
            // A source edit landed DURING the last run — it is real + unreflected, so re-run NOW without
            // blocking or re-checking (we already confirmed it touched source when we set `pending`).
            // Still coalesce whatever else is immediately queued so a mid-run burst folds into this run.
            pending = false;
            while rx.try_recv().is_ok() {}
        } else {
            // Block until some event arrives (or the channel closes → exit), then coalesce the debounce
            // window and re-run only if the batch touched a SOURCE file / the manifest.
            let first = match rx.recv() {
                Ok(ev) => ev,
                Err(_) => return ExitCode::SUCCESS, // watcher dropped
            };
            let mut batch = vec![first];
            while let Ok(ev) = rx.recv_timeout(debounce) {
                batch.push(ev);
            }
            if !batch_touches_source(&batch) {
                continue; // artifact/temp churn only — nothing to re-run
            }
        }
        clear_screen();
        eprintln!("{PROG}: ⟳ change detected — re-running `cdz {label}`");
        let _ = rerun();
        // Inspect events that arrived DURING the re-run. Artifact-only churn (the run's own outputs) is
        // discarded — already reflected. But a SOURCE edit made mid-run is NOT reflected in the run we
        // just finished, so flag it (`pending`) to re-run once more rather than silently dropping that
        // save. (`pending` re-runs UNCONDITIONALLY next iteration — the source check happens HERE, so an
        // artifact-only follow-on batch can't cancel a real mid-run edit.)
        let mut during = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            during.push(ev);
        }
        if batch_touches_source(&during) {
            pending = true;
        }
    }
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

/// One `@test`/`@exhaustive` def selected to run, with everything the runner needs to drive it: its `name`,
/// the per-parameter generators (`None` = a param type the runner can't generate; empty = a nullary def run
/// once), whether it is `@exhaustive`, and the per-parameter `@requires` integer `bounds` for constrained
/// generation (empty ⇒ unconstrained). Distilled once per test in the collection loop before the run loop.
struct TestSpec {
    name: String,
    gens: Option<Vec<GenKind>>,
    exhaustive: bool,
    bounds: Vec<ParamBound>,
    /// Recognized ORDER relations between two integer params from `@requires` (e.g. `(< a b)`), enforced by
    /// rejection sampling in the generator. Empty for the common single-param / unconstrained case.
    relations: Vec<Relation>,
    /// The synthesized `-gen` wrapper's parameter `GenTy` (via `gen_ty_of_wrapper_param`), captured at BUILD
    /// time because `db` is not in scope in the run loop. `Some` only for a `-gen` wrapper; `None` for a plain
    /// or scalar test. Used by the `@exhaustive` path to ENUMERATE a bounded newtype domain (a single-variant
    /// `Sum` whose payload is an `IntRange` — feed pool values `0..span` to drive the wrapper over `lo..=hi`).
    gen_ty: Option<rcdzc::proptest_gen::GenTy>,
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

/// The constraint a `@requires` precondition imposes on one scalar parameter, so the generator draws only
/// IN-DOMAIN values and never trips the (D) body-entry precondition trap. For an INTEGER param it is an
/// inclusive range `[lo, hi]` (`i128` so a full `i64`/`u64` range plus a `±1` strict-to-inclusive adjustment
/// never overflows). For a BOOL param, `bool_force` pins it: a bare-Bool precondition `@requires(b)` requires
/// `b` true, so the generator must draw `true` (a random `false` would trip the pre-trap). `None` = no Bool
/// constraint. The two are independent (a param is one or the other kind); an int param never sets
/// `bool_force`, a bool param never narrows `[lo, hi]`.
#[derive(Clone, Copy)]
struct ParamBound {
    lo: i128,
    hi: i128,
    bool_force: Option<bool>,
}

impl ParamBound {
    /// The widest bound — no constraint. Narrowed by each recognized `@requires` comparison.
    fn unbounded() -> Self {
        ParamBound {
            lo: i128::MIN,
            hi: i128::MAX,
            bool_force: None,
        }
    }
    /// Clamp a drawn value into `[lo, hi]`. An empty range (lo > hi, from contradictory requires) leaves the
    /// value unchanged — the precondition is unsatisfiable and the trap is the correct outcome, not our job
    /// to hide.
    fn clamp(&self, v: i128) -> i128 {
        if self.lo > self.hi {
            v
        } else {
            v.clamp(self.lo, self.hi)
        }
    }
    /// Whether this bound narrows anything (worth applying).
    fn is_constrained(&self) -> bool {
        self.lo != i128::MIN || self.hi != i128::MAX
    }
}

/// A recognized relation between two integer parameters from a `@requires` — e.g. `(< a b)` or `(= a b)`,
/// where both sides are param names (not a param-vs-literal, which a `ParamBound` already covers). Unlike a
/// per-param range clamp, a relation COUPLES two params, so it cannot be satisfied by clamping one in
/// isolation. Two enforcement strategies by operator. An ORDER op (`< <= > >=`) is enforced by REJECTION
/// SAMPLING: re-draw (advancing the seed deterministically) until every relation holds, bounded by fuel. An
/// EQUALITY (`=`) is enforced by PROPAGATION: the right param's value is copied FROM the left, so `a = b`
/// holds by construction with ZERO rejection (two independent draws are ~never equal, so rejection would only
/// exhaust fuel — propagation is the reject-free analogue of clamping for a range bound). Any unrecognized
/// shape stays unconstrained exactly as before. `op` is one of the recognized operator strings; `left`/`right`
/// are parameter POSITIONS (matching the `GenKind` vec order).
#[derive(Clone, Copy)]
struct Relation {
    left: usize,
    op: &'static str,
    right: usize,
}

/// Whether `l OP r` holds for the recognized operators (an unrecognized op vacuously holds — it was never
/// recorded, so this is only reached for `< <= > >= =`). After `propagate_equalities` runs, an `=` relation
/// always holds; it is still checked here so the rejection loop's `relations_hold` guard is total.
fn relation_holds(op: &str, l: i64, r: i64) -> bool {
    match op {
        "<" => l < r,
        "<=" => l <= r,
        ">" => l > r,
        ">=" => l >= r,
        "=" => l == r,
        _ => true,
    }
}

/// Enforce each EQUALITY relation (`(= a b)`) by copying the LEFT param's value onto the RIGHT — so `a = b`
/// holds BY CONSTRUCTION, no rejection. Iterated to a fixpoint (bounded by the equality count) so a chain
/// `a = b and b = c` fully propagates (all become `a`) regardless of the order the relations were recorded.
/// Order relations are untouched here (they go through rejection sampling). Applied after each draw and after
/// building each shrink trial.
fn propagate_equalities(relations: &[Relation], inputs: &mut [String]) {
    let eq_count = relations.iter().filter(|r| r.op == "=").count();
    for _ in 0..eq_count {
        for rel in relations {
            if rel.op == "=" && rel.left < inputs.len() && rel.right < inputs.len() {
                inputs[rel.right] = inputs[rel.left].clone();
            }
        }
    }
}

/// Whether EVERY recorded relation holds over the rendered `inputs`. A param whose rendered value does not
/// parse as an `i64` (e.g. a Bool `"true"`) makes its relation vacuously hold — relations are only recorded
/// between integer params, so this is a defensive skip, never the common path.
fn relations_hold(relations: &[Relation], inputs: &[String]) -> bool {
    relations.iter().all(|rel| {
        let (Some(l), Some(r)) = (inputs.get(rel.left), inputs.get(rel.right)) else {
            return true;
        };
        match (l.parse::<i64>(), r.parse::<i64>()) {
            (Ok(l), Ok(r)) => relation_holds(rel.op, l, r),
            _ => true,
        }
    })
}

/// Per-parameter integer bounds distilled from a def's `@requires` preconditions (empty ⇒ no bound). The
/// generator applies these so a `@test` over a `@requires`-constrained def draws only inputs SATISFYING the
/// precondition — the (D) enforcement traps a violated precondition at body entry (a HARD contract for every
/// caller, `verify_enforce.rs`), so feeding an out-of-domain draw would spuriously FAIL the test rather than
/// exercise the property. This is the runner-side half of "constrained generation": recognize simple
/// range/comparison predicates over a single scalar param (`(>= x LO)`, `(< x HI)`, `(= x K)`, and their
/// mirrors), and constrain that param's draw. An UNRECOGNIZED predicate shape leaves the param unbounded —
/// the draw is unconstrained, exactly as before (never wrong: an over-broad draw that happens to satisfy the
/// pre still passes; only a pre-violating draw was the bug, and a recognized bound removes those).
///
/// Only INTEGER params are bounded here (the boundary-arg route generates Int/Bool/Float/Char; a comparison
/// bound is meaningful for integers — Bool/Float/Char preconditions fall through unrecognized). Keyed by
/// parameter POSITION (matching the `GenKind` vec order).
fn param_bounds(db: &rcdzc::db::Db, def: usize) -> (Vec<ParamBound>, Vec<Relation>) {
    let params = &db.defs[def].params;
    // Map each param NAME to its position, so a `(>= name lit)` predicate targets the right slot.
    let pos_of: std::collections::HashMap<&str, usize> = params
        .iter()
        .enumerate()
        .filter_map(|(i, &p)| {
            // A param is a bare name atom or an annotated `(: name T)` binder — the name is the head child.
            let name_node = db
                .ast
                .as_form(p, ":")
                .and_then(|t| t.first().copied())
                .unwrap_or(p);
            db.ast.as_name(name_node).map(|n| (n, i))
        })
        .collect();
    let mut bounds = vec![ParamBound::unbounded(); params.len()];
    let mut relations = Vec::new();
    for &pred in db.requires_of(def) {
        narrow_from_predicate(db, pred, &pos_of, &mut bounds, &mut relations);
    }
    (bounds, relations)
}

/// Narrow `bounds` (and collect `relations`) from ONE `@requires` predicate AST node. Recognizes a
/// comparison `(OP a b)` for OP in `>= > <= < =`: a (param, literal) pairing in either order narrows that
/// param's `ParamBound`; a (param, param) pairing for an ORDER op (`< <= > >=`) or `=` records a `Relation`
/// between the two params (a coupled constraint a single-param clamp cannot express — an order relation is
/// satisfied by rejection sampling, an equality by propagation). It also descends a conjunction
/// `(and p q …)` / `(& p q …)` so `(and (>= x 0) (< x 100))` bounds `x` to `[0, 99]`. Anything else (a call,
/// a non-linear predicate) is left unrecognized — no change, exactly as before.
fn narrow_from_predicate(
    db: &rcdzc::db::Db,
    pred: cadenza_syntax::ast::StructId,
    pos_of: &std::collections::HashMap<&str, usize>,
    bounds: &mut [ParamBound],
    relations: &mut Vec<Relation>,
) {
    // Descend a conjunction: every conjunct constrains independently.
    for head in ["and", "&"] {
        if let Some(tail) = db.ast.as_form(pred, head) {
            for &conj in tail {
                narrow_from_predicate(db, conj, pos_of, bounds, relations);
            }
            return;
        }
    }
    // A BARE PARAM NAME predicate — `@requires(b)` where `b` is a Bool param — requires that param TRUE. A
    // random `false` draw would trip the (D) pre-trap, so pin the generated value to `true` (the Bool analogue
    // of pinning an int to a constant). A bare name in a `@requires` can only be a Bool param (the enforcement
    // wraps `(if PRE BODY (trap))`, which type-checks only for a Bool `PRE`); a name that is not a param
    // (a prelude/global) simply isn't in `pos_of` and is left unconstrained.
    if let Some(name) = db.ast.as_name(pred)
        && let Some(&i) = pos_of.get(name)
    {
        bounds[i].bool_force = Some(true);
        return;
    }
    // A comparison `(OP lhs rhs)`. Identify OP, then the (param, literal) pairing in either order.
    for op in [">=", ">", "<=", "<", "="] {
        let Some(t) = db.ast.as_form(pred, op) else {
            continue;
        };
        if t.len() != 2 {
            return;
        }
        let (lhs, rhs) = (t[0], t[1]);
        let as_i128 = |v: &cadenza_syntax::ast::IntValue| v.to_i128();
        // `(OP param lit)` — the common spelling.
        if let (Some(name), Some(lit)) = (db.ast.as_name(lhs), db.ast.as_int(rhs).and_then(as_i128))
        {
            if let Some(&i) = pos_of.get(name) {
                apply_cmp(op, lit, false, &mut bounds[i]);
            }
            return;
        }
        // `(OP lit param)` — the mirrored spelling; the operator flips (lit < x ⇒ x > lit).
        if let (Some(lit), Some(name)) = (db.ast.as_int(lhs).and_then(as_i128), db.ast.as_name(rhs))
        {
            if let Some(&i) = pos_of.get(name) {
                apply_cmp(op, lit, true, &mut bounds[i]);
            }
            return;
        }
        // `(OP param param)` — a RELATION between two params. An ORDER op (`< <= > >=`) is satisfied by
        // rejection sampling; an EQUALITY `=` is satisfied by propagation (copy left→right). A param compared
        // to itself is skipped: `(< a a)` is unsatisfiable (leave it to trap, not our job to mask), and
        // `(= a a)` is trivially true (no constraint).
        if let (Some(ln), Some(rn)) = (db.ast.as_name(lhs), db.ast.as_name(rhs))
            && let (Some(&li), Some(&ri)) = (pos_of.get(ln), pos_of.get(rn))
            && li != ri
            && matches!(op, "<" | "<=" | ">" | ">=" | "=")
        {
            relations.push(Relation {
                left: li,
                op,
                right: ri,
            });
        }
        return; // recognized OP; a (param, param) relation was recorded above if applicable
    }
}

/// Narrow one `ParamBound` by `param OP lit` (or, when `mirrored`, `lit OP param`). A strict `<`/`>` becomes
/// an inclusive bound via `±1` (integers). `=` pins both ends.
fn apply_cmp(op: &str, lit: i128, mirrored: bool, b: &mut ParamBound) {
    // Normalize the mirrored form `lit OP param` to `param OP' lit`: `lit < x` ⇔ `x > lit`, etc.
    let op = if mirrored {
        match op {
            "<" => ">",
            ">" => "<",
            "<=" => ">=",
            ">=" => "<=",
            other => other, // `=` is symmetric
        }
    } else {
        op
    };
    match op {
        ">=" => b.lo = b.lo.max(lit),
        ">" => b.lo = b.lo.max(lit.saturating_add(1)),
        "<=" => b.hi = b.hi.min(lit),
        "<" => b.hi = b.hi.min(lit.saturating_sub(1)),
        "=" => {
            b.lo = b.lo.max(lit);
            b.hi = b.hi.min(lit);
        }
        _ => {}
    }
}

/// The number of `(list …)` elements a synthesized variable-length list generator produces candidates for
/// (mirrors `proptest_gen::G1_LIST_LEN`). The wrapper draws a count `c = gen % (LEN+1)`, then LEN candidate
/// elements, keeping the first `c`. The decoder MUST use the same LEN to consume the pool in lockstep.
const RUNNER_LIST_LEN: usize = 3;

/// Render the concrete value a shrunk driver `pool` decodes to for the wrapper's parameter generator shape
/// `gty`, mirroring the derivation `proptest_gen::build_gen` synthesized into the `-gen` wrapper — so the
/// reported counterexample is the actual value that failed (`[0, 0, 0]` / `(1, false)` / `Err(3)`) rather
/// than the raw driver ints. `gty` is `proptest_gen`'s OWN `GenTy` (via `gen_ty_of_wrapper_param`), so the
/// decode vocabulary is the SAME one the generator was built from — it can never drift out of sync, and a
/// `Sum`/nested shape is covered identically to the wrapper. The pool is consumed via a shared cursor in the
/// SAME order the wrapper pulls `Test.gen-int`. `None` only if the pool runs dry (a malformed shrink).
fn render_pool_value(gty: &rcdzc::proptest_gen::GenTy, pool: &[i64]) -> Option<String> {
    let mut cursor = 0usize;
    decode_value(gty, pool, &mut cursor)
}

/// If `gen_ty` is a `-gen` wrapper param that CAN be exhaustively enumerated — a single-variant `Sum` newtype
/// whose sole payload is an `IntRange{lo,hi}` (a bounded `@invariant` newtype like `Percent = Pct(Int64)` with
/// `@invariant [0,100]`) AND whose domain size `hi-lo+1` fits [`MAX_EXHAUSTIVE_CASES`] — return `(lo, hi)`.
/// `None` for any other shape (a List/Tuple/multi-variant sum, a non-IntRange payload, or a range too large to
/// enumerate) — the caller then declines the `@exhaustive` cleanly. This is what lets `@exhaustive` PROVE a
/// property over a small refined newtype's whole domain (drive the wrapper over each `v in lo..=hi`) instead
/// of sampling / declining.
fn exhaustive_newtype_range(gen_ty: Option<&rcdzc::proptest_gen::GenTy>) -> Option<(i64, i64)> {
    use rcdzc::proptest_gen::GenTy;
    let GenTy::Sum { variants, .. } = gen_ty? else {
        return None;
    };
    // A single-variant newtype whose one payload is an IntRange.
    let [(_, Some(GenTy::IntRange { lo, hi }))] = variants.as_slice() else {
        return None;
    };
    let (lo, hi) = (*lo, *hi);
    // A valid range (lo<=hi) whose size fits the enumeration cap — else decline (too large to prove).
    let span = (hi as i128) - (lo as i128) + 1;
    (span >= 1 && span <= MAX_EXHAUSTIVE_CASES as i128).then_some((lo, hi))
}

/// One step of the pool→value decode (see [`render_pool_value`]). `cursor` advances by exactly the number of
/// `Test.gen-int` ints the corresponding `build_gen` arm consumes, in the same order.
fn decode_value(
    gty: &rcdzc::proptest_gen::GenTy,
    pool: &[i64],
    cursor: &mut usize,
) -> Option<String> {
    use rcdzc::proptest_gen::GenTy;
    // Pull the next driver int (the wrapper's `Test.gen-int`); `None` if the shrunk pool is exhausted.
    let next = |cursor: &mut usize| -> Option<i64> {
        let v = pool.get(*cursor).copied()?;
        *cursor += 1;
        Some(v)
    };
    match gty {
        // A scalar Int consumes one int: the value IS that int.
        GenTy::Int => Some(next(cursor)?.to_string()),
        // A range-constrained int consumes one int, mapped into `[lo, hi]` EXACTLY as the generator's
        // `build_gen` IntRange arm does: mask to non-negative (`& i64::MAX`), `% SPAN`, `+ lo`. Mirroring the
        // derivation keeps the decoded counterexample equal to the value that actually ran.
        GenTy::IntRange { lo, hi } => {
            let span = hi.wrapping_sub(*lo).wrapping_add(1);
            let v = lo.wrapping_add((next(cursor)? & i64::MAX).rem_euclid(span));
            Some(v.to_string())
        }
        // A Bool consumes one int, taken as its parity (`gen % 2 == 0`) — the `build_gen` Bool derivation.
        GenTy::Bool => Some((next(cursor)?.rem_euclid(2) == 0).to_string()),
        // A Float consumes one int, converted to an integer-valued float (`FloatN.of-int`).
        GenTy::Float(_) => Some(format!("{}.0", next(cursor)?)),
        // A variable-length list: a count `c = MIN + (gen % (LEN+1-MIN))` then LEN candidate elements (all
        // drawn), value = the first `c`. `min_len` (a min-length refinement floor, clamped to LEN) mirrors
        // the generator's `build_var_list_gen` count formula EXACTLY so the decode stays in lockstep with the
        // run. The decoder draws all LEN elements regardless of `c`, same as the wrapper.
        GenTy::List(elem, min_len) => {
            let min = (*min_len).min(RUNNER_LIST_LEN);
            let span = (RUNNER_LIST_LEN + 1 - min) as i64;
            // Mirror the generator EXACTLY: `(gen & i64::MAX) % span` (mask non-negative, then mod), NOT
            // rem_euclid — the wrapper masks the sign bit, which differs from rem_euclid for a negative gen.
            let c = min + ((next(cursor)? & i64::MAX) % span) as usize;
            let mut elems = Vec::with_capacity(RUNNER_LIST_LEN);
            for _ in 0..RUNNER_LIST_LEN {
                elems.push(decode_value(elem, pool, cursor)?);
            }
            elems.truncate(c);
            Some(format!("[{}]", elems.join(", ")))
        }
        // A tuple draws one value per slot, in order.
        GenTy::Tuple(slots) => {
            let mut vals = Vec::with_capacity(slots.len());
            for slot in slots.iter() {
                vals.push(decode_value(slot, pool, cursor)?);
            }
            Some(format!("({})", vals.join(", ")))
        }
        // A record draws one value per field, in the field order `build_gen` used.
        GenTy::Record(fields) => {
            let mut parts = Vec::with_capacity(fields.len());
            for (fname, fty) in fields.iter() {
                let v = decode_value(fty, pool, cursor)?;
                parts.push(format!("{fname}: {v}"));
            }
            Some(format!("{{{}}}", parts.join(", ")))
        }
        // A user SUM: the wrapper draws a selector `sel = gen % k` FIRST, then EVERY variant's payload
        // unconditionally (in order), and keeps variant `sel`. The decoder mirrors that EXACTLY — draw the
        // selector, then decode each variant's payload advancing the cursor over ALL of them, keeping only
        // the selected variant's rendering (`Err(3)`, or a bare `None` for a nullary variant). Draining every
        // payload keeps the cursor correct even when the sum is NESTED inside an enclosing compound.
        GenTy::Sum { variants, .. } => {
            if variants.is_empty() {
                return None;
            }
            let k = variants.len();
            let sel = (next(cursor)?.rem_euclid(k as i64)) as usize;
            let mut selected: Option<String> = None;
            for (i, (vname, payload)) in variants.iter().enumerate() {
                let rendered = match payload {
                    None => vname.clone(),
                    Some(pty) => format!("{vname}({})", decode_value(pty, pool, cursor)?),
                };
                if i == sel {
                    selected = Some(rendered);
                }
            }
            selected
        }
        // A Set: the generator draws a count `c = (gen & i64::MAX) % (LEN+1)` then folds `c` `Set.insert`s of
        // the first `c` of `RUNNER_LIST_LEN` candidate elements over the empty set (a VARIABLE-cardinality set,
        // so the empty/singleton sets are reachable — see `build_var_set_gen`). Mirror it EXACTLY: draw the
        // count, decode all `RUNNER_LIST_LEN` candidates (cursor advances over every one, so a NESTED Set stays
        // in lockstep), keep the length-`c` prefix, then DEDUP by rendered value (a collision yields a smaller
        // set, as `Set.insert` does). `{}` for c=0. A refined-newtype element renders in-domain via its GenTy.
        GenTy::Set(elem) => {
            let span = (RUNNER_LIST_LEN + 1) as i64;
            let c = ((next(cursor)? & i64::MAX) % span) as usize;
            let mut drawn = Vec::with_capacity(RUNNER_LIST_LEN);
            for _ in 0..RUNNER_LIST_LEN {
                drawn.push(decode_value(elem, pool, cursor)?);
            }
            drawn.truncate(c);
            let mut seen: Vec<String> = Vec::with_capacity(c);
            for e in drawn {
                if !seen.contains(&e) {
                    seen.push(e);
                }
            }
            Some(format!("{{{}}}", seen.join(", ")))
        }
        // A Map: the generator draws a count `c = (gen & i64::MAX) % (LEN+1)` then folds `c` `Map.insert`s of
        // the first `c` of `RUNNER_LIST_LEN` candidate key/value pairs over `(Map.empty)` (a VARIABLE-size map,
        // so the empty/small maps are reachable — see `build_var_map_gen`). Mirror it EXACTLY: draw the count,
        // decode all `RUNNER_LIST_LEN` (key, value) candidate pairs (cursor advances over every one, so a
        // NESTED Map stays in lockstep), keep the length-`c` prefix, then apply LAST-WRITE-WINS by rendered key
        // (preserving first-insertion order, as the insert fold does). `{}` for c=0. Refined-newtype key/value
        // decodes in-domain via its own GenTy.
        GenTy::Map(kty, vty) => {
            let span = (RUNNER_LIST_LEN + 1) as i64;
            let c = ((next(cursor)? & i64::MAX) % span) as usize;
            let mut drawn: Vec<(String, String)> = Vec::with_capacity(RUNNER_LIST_LEN);
            for _ in 0..RUNNER_LIST_LEN {
                let k = decode_value(kty, pool, cursor)?;
                let v = decode_value(vty, pool, cursor)?;
                drawn.push((k, v));
            }
            drawn.truncate(c);
            let mut entries: Vec<(String, String)> = Vec::with_capacity(c);
            for (k, v) in drawn {
                // Last-write-wins: update an existing key's value in place (keeping its position), else append.
                if let Some(slot) = entries.iter_mut().find(|(ek, _)| ek == &k) {
                    slot.1 = v;
                } else {
                    entries.push((k, v));
                }
            }
            let parts: Vec<String> = entries.iter().map(|(k, v)| format!("{k}: {v}")).collect();
            Some(format!("{{{}}}", parts.join(", ")))
        }
    }
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

/// How a file's `@test` component is run per trial. STANDALONE: a self-contained component, JIT-compiled ONCE
/// and reused across every trial (the common path — `run_capturing_compiled`). COMPOSED (Option-C): a
/// cross-edge-EXCLUDING consumer + its shared-closure provider peer, JIT-compiled ONCE into a
/// `CompiledComposition` and reused across every trial (`run_composition_capturing`) — so a multi-trial
/// property test no longer re-JITs consumer+peer per trial (PR#892). Both paths yield the SAME
/// `(Outcome, observed-op-list)` shape, so the trial logic (gen-int count, failure message) is identical.
enum RunTarget {
    Standalone(cdz_run::CompiledComponent),
    Composed(cdz_run::CompiledComposition),
}

/// Run the test component IN-PROCESS once, calling `kebab` with `arg_vals` (rendered arg text). PASS = the
/// export returned; FAIL carries the failure message the test reported (via its `Test`/report host effect)
/// if any. `runtime` is the value-heap runtime bytes the component was resolved against (or `None` for a
/// scalar/const test component that imports no runtime).
fn run_one_trial(
    target: &RunTarget,
    runtime: Option<&[u8]>,
    kebab: &str,
    store: &std::path::Path,
    arg_vals: &[String],
) -> TrialOutcome {
    run_one_trial_with_pool(target, runtime, kebab, store, arg_vals, &[]).0
}

/// Whether to include the full wasm BACKTRACE (the `<wasm function N>` frames wasmtime captures on a trap)
/// in a trapping test's FAIL message, rather than trimming to the reason's first line. Off by default (a
/// one-line counterexample stays legible); enabled by setting `CDZ_WASM_BACKTRACE` to any non-empty value
/// other than `0`/`false`. A debug lever for localizing a COMPILED trap — the frame indices are the only
/// locus for a self-host trap, where the usual isolate-the-case repro doesn't reproduce it.
fn wasm_backtrace_enabled() -> bool {
    match std::env::var("CDZ_WASM_BACKTRACE") {
        Ok(v) => {
            let v = v.trim();
            !v.is_empty() && v != "0" && !v.eq_ignore_ascii_case("false")
        }
        Err(_) => false,
    }
}

/// The well-known GENERATOR effect operation a property test performs to pull one random `Int64` from the
/// runner's driver: `Test.gen-int : Unit -> Int64` (the "well-known `Test` effect extends" convention — the
/// same `Test` effect that carries `fail`). `cdz test` answers a `Test.gen-int` performance with the next int
/// from a seeded pool, so a generator built on this ONE op — bolero's Driver model, one int source that
/// type-directed generation decodes — needs no per-shape host coordination.
const GEN_OP_LABEL: &str = "test.gen-int";

/// Run the test component IN-PROCESS (via the `cdz-run` LIBRARY — `run_capturing`, no sibling binary),
/// ALSO supplying a seeded int `pool` as ordered `Test.gen-int=<n>` host responses (consumed IN ORDER by each
/// `Test.gen-int` performance — a result-bearing op; a unit op like `Test.fail` consumes none). Returns the
/// trial outcome AND how many `Test.gen-int` calls the guest actually made (counted from the OBSERVED host-op
/// list `run_capturing` returns) — the signal that distinguishes a PROPERTY test (pulls ≥1 generated int)
/// from a plain unit test (pulls none). An unconsumed pool response is harmless (ignored).
fn run_one_trial_with_pool(
    target: &RunTarget,
    runtime: Option<&[u8]>,
    kebab: &str,
    store: &std::path::Path,
    arg_vals: &[String],
    pool: &[i64],
) -> (TrialOutcome, usize) {
    // Each pool int becomes a `Test.gen-int` host response, consumed in order. The op label pairs it with the
    // call for the ordered-consume model (the value is coerced to the op's `Int64` result at binding).
    let host_responses: Vec<cdz_run::HostResponse> = pool
        .iter()
        .map(|n| cdz_run::HostResponse {
            op: "Test.gen-int".to_string(),
            value: n.to_string(),
        })
        .collect();
    // FINDING#23: the runtime imports `cadenza:nfc/normalize`, but cdz-run now SELF-RESOLVES that NFC
    // component from the store (keyed off `runtime_cache_dir`, set below) inside its compose step — no `nfc`
    // field to thread here.
    let opts = cdz_run::RunOpts {
        export: Some(kebab.to_string()),
        args: arg_vals.to_vec(),
        runtime: runtime.map(<[u8]>::to_vec),
        runtime_cache_dir: Some(store.to_path_buf()),
        host_responses,
        // `cdz run` JIT-compiles the freshly-built project (it HAS the compiler); precompiled/deserialize
        // mode is the cranelift-free corpus-exec path, not this front-end.
        precompiled: false,
    };
    // Both targets were JIT-compiled ONCE by the caller + are reused across every trial (`Component::new` is
    // ~99% of a run's cost). STANDALONE runs the compiled component; COMPOSED links the compiled consumer
    // against the compiled provider peer over one runtime. Both return the SAME `(Outcome, observed)` shape,
    // so the trial logic below is identical.
    let run_result = match target {
        RunTarget::Standalone(compiled) => {
            cdz_run::run_capturing_compiled(compiled, &opts, None, false, None)
        }
        RunTarget::Composed(composition) => cdz_run::run_composition_capturing(composition, &opts),
    };
    match run_result {
        Ok((outcome, observed)) => {
            let gens = count_gen_calls(&observed);
            let trial = match outcome {
                cdz_run::Outcome::Value(_) => TrialOutcome::Pass,
                // A trapping test FAILS. Prefer the assertion message the test emitted (via its report host
                // effect, e.g. `Test.fail("…")`) — it rides an OBSERVED op entry as `<op>\t<message>`. But if
                // there is NO such op, the body TRAPPED for another reason (an arithmetic OVERFLOW `+ traps:
                // overflows Int64`, a div-by-zero, an explicit `trap("…")`) — and that reason is exactly what
                // distinguishes "the property BODY TRAPPED" from "the property RETURNED FALSE". The runtime's
                // `Trap(reason)` carries that reason, so fall back to it (prefixed so the author sees the body
                // trapped rather than the property being false — a very different diagnosis, e.g. a full-domain
                // Int64 generator whose unguarded `+` overflows on two large samples is NOT a real violation).
                cdz_run::Outcome::Trap(reason) => {
                    TrialOutcome::Fail(observed_failure_message(&observed).or_else(|| {
                        // A wasmtime trap renders as `wasm trap: <reason>` followed by a multi-line wasm
                        // BACKTRACE (`0: 0x… - <wasm function N>` frames). By default trim to the FIRST line
                        // — the actionable reason — so the one-line counterexample report stays legible.
                        // With `CDZ_WASM_BACKTRACE` set, keep the WHOLE reason (frames included): a compiled
                        // trap (esp. self-host, where the isolated-repro trick fails) is hard to localize
                        // without the `<wasm function N>` frame indices, and there is no other way to see them
                        // in `cdz test` (v-wasm-opt's diagnostic-quality gap — the backtrace IS captured, it
                        // was just being discarded here).
                        let trimmed = reason.trim();
                        (!trimmed.is_empty()).then(|| {
                            if wasm_backtrace_enabled() {
                                format!("body trapped: {trimmed}")
                            } else {
                                format!(
                                    "body trapped: {}",
                                    reason.lines().next().unwrap_or("").trim()
                                )
                            }
                        })
                    }))
                }
            };
            (trial, gens)
        }
        // A run-level error (an invalid component, an unresolvable runtime the pre-check missed) — surface
        // it as a failure so the test is reported, not silently skipped.
        Err(e) => (
            TrialOutcome::Fail(Some(format!("could not run test: {e:#}"))),
            0,
        ),
    }
}

/// How many `Test.gen-int` performances the guest made, from the OBSERVED host-op list `run_capturing` returns
/// (each entry is a dotted `E.op`, optionally `\t<str-args>`). `> 0` ⇒ the test is a PROPERTY test driven
/// by the int pool. Matches the op field (before any tab) case-insensitively against the `Test.gen-int` label.
fn count_gen_calls(observed: &[String]) -> usize {
    observed
        .iter()
        .filter(|entry| {
            let op = entry.split('\t').next().unwrap_or(entry);
            op.eq_ignore_ascii_case(GEN_OP_LABEL)
        })
        .count()
}

/// The assertion message a trapping test reported, from the OBSERVED host-op list. `run_capturing`
/// records each string-carrying host call as `<op>\t<message>`, but that is EVERY string-arg op — a
/// `log.emit("…")` a test performs before it fails carries a message too. So match ONLY a REPORTING op
/// (one whose dotted name ends in `.fail` — `test.fail`/`report.fail`, the ops a failing assertion
/// performs), not just the first tab-carrying entry, or a benign log line would be misreported as the
/// failure message. The LAST such `.fail` wins (the one closest to the trap). `None` if no reporting op
/// carried a message (a bare trap with no assertion text).
fn observed_failure_message(observed: &[String]) -> Option<String> {
    observed.iter().rev().find_map(|entry| {
        let (op, msg) = entry.split_once('\t')?;
        // The op field is a dotted `E.op`; a reporting op ends in `.fail` (case-insensitive, since the
        // observed op label preserves the boundary spelling — `Test.fail`/`test.fail`).
        op.to_ascii_lowercase()
            .ends_with(".fail")
            .then(|| msg.to_string())
    })
}

/// Resolve the value-heap runtime bytes the test `component` requires, BY CONTENT ADDRESS from `store` —
/// the same content-addressed resolution `cdz run` performs. Returns `Ok(None)` for a scalar/const
/// component that imports no runtime (no store needed), `Ok(Some(bytes))` when the store holds the exact
/// required hash, and `Err` (a clear, once-per-file message) when the component needs a runtime the store
/// does not hold — reported before running rather than as an opaque trap inside each test.
fn resolve_test_runtime(
    component: &[u8],
    store: &std::path::Path,
) -> Result<Option<Vec<u8>>, String> {
    let req = match cdz_run::required_runtime(component) {
        Ok(Some(req)) => req,
        Ok(None) => return Ok(None), // scalar/const test component — no runtime import
        Err(e) => return Err(format!("could not inspect the test component: {e:#}")),
    };
    if req.hash.is_empty() {
        return Err(
            "the test component imports the value-heap runtime but records no content address to \
             resolve it by (an unpinned runtime import)"
                .to_string(),
        );
    }
    let path = store.join(format!("{}.wasm", req.hash));
    if !path.is_file() {
        return Err(format!(
            "no runtime of content address {} in the store at {} — build it (`cargo xtask build`) so \
             `cdz test` can run a heap-value test",
            req.hash,
            store.display()
        ));
    }
    std::fs::read(&path)
        .map(Some)
        .map_err(|e| format!("reading the stored runtime {}: {e}", path.display()))
}

/// Run a PROPERTY test `trials` times with generated inputs, returning `None` if every trial passed or the
/// first counterexample (SHRUNK toward a minimal failing input). Generation is seeded (`seed`) so a run is
/// reproducible; each trial advances the seed deterministically (`seed + trial`), so the failing trial's
/// inputs re-generate identically on replay. On the first failing trial, `shrink` searches for a smaller
/// still-failing input before reporting.
fn run_property(
    gens: &[GenKind],
    bounds: &[ParamBound],
    relations: &[Relation],
    trials: u64,
    seed: u64,
    run_one: &dyn Fn(&[String]) -> TrialOutcome,
) -> Option<PropertyFailure> {
    for trial in 0..trials {
        let inputs = generate_inputs(gens, bounds, relations, seed.wrapping_add(trial));
        if let TrialOutcome::Fail(message) = run_one(&inputs) {
            let (inputs, message) = shrink(gens, bounds, relations, &inputs, message, run_one);
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
/// `Test.gen-int` int. The FIRST run uses a seeded pool (`seed`); if the guest consumed ZERO generated ints
/// it is a plain unit test — return its outcome directly (one run, today's semantics, unaffected by the
/// unconsumed pool). If it consumed ≥1, it is a property test: run `trials` trials each with a FRESH
/// seeded pool (`seed + trial`, reproducible), failing on the first trapping trial with the SHRUNK pool.
fn run_gen_driven(
    target: &RunTarget,
    runtime: Option<&[u8]>,
    kebab: &str,
    store: &std::path::Path,
    trials: u64,
    seed: u64,
    gen_ty: Option<&rcdzc::proptest_gen::GenTy>,
) -> GenDrivenOutcome {
    let run_pool = |pool: &[i64]| -> (TrialOutcome, usize) {
        run_one_trial_with_pool(target, runtime, kebab, store, &[], pool)
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
        return GenDrivenOutcome::Property(Some(shrink_pool(
            &pool0, gens0, message, gen_ty, &run_pool,
        )));
    }
    // Remaining trials, each a fresh seeded pool.
    for trial in 1..trials {
        let pool = gen_pool(seed.wrapping_add(trial), GEN_POOL_SIZE);
        let (outcome, gens) = run_pool(&pool);
        if let TrialOutcome::Fail(message) = outcome {
            return GenDrivenOutcome::Property(Some(shrink_pool(
                &pool, gens, message, gen_ty, &run_pool,
            )));
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
    gen_ty: Option<&rcdzc::proptest_gen::GenTy>,
    run_pool: &dyn Fn(&[i64]) -> (TrialOutcome, usize),
) -> PropertyFailure {
    // Only the CONSUMED prefix matters — the generator pulled `gens` ints; the rest of the pool is inert.
    let mut best: Vec<i64> = pool.iter().take(gens).copied().collect();
    let mut best_msg = message;
    // DECODED-SPACE shrink for a single-IntRange newtype (`Percent = Pct(Int64)` with `@invariant [0,100]`):
    // the wrapper pool is `[selector, payload]` and the payload decodes `v = lo + (payload & MAX) % span`,
    // which is NOT monotonic in the raw payload int — so the generic raw-int halving below cannot converge to
    // the domain-minimal (it reported e.g. Pct(67), not the true boundary). Here we bisect the DECODED value
    // toward `lo` directly (candidate value `c` ⇒ pool payload `c - lo`, the invertible map), keeping any `c`
    // that still fails, so the counterexample shrinks to the smallest in-domain failing value. Only this
    // single-IntRange-newtype shape is handled (its pool layout is known: selector at 0, payload at 1);
    // compound/multi-leaf shapes fall through to the generic pass unchanged.
    if let Some((lo, hi)) = exhaustive_newtype_range(gen_ty)
        && best.len() >= 2
    {
        // DECODED-SPACE shrink toward the boundary. The generic raw-int halving below cannot converge for an
        // IntRange leaf (decode `v = lo + (payload & MAX) % span` is non-monotonic in the raw int), so bisect
        // the DECODED value: find the LEAST `v in [lo, hi]` that still fails, via the invertible map (pool
        // payload = v - lo). This assumes the common upward-closed fail-set (`v >= threshold`), the shape a
        // refined-newtype property almost always has; a fail-set that isn't upward-closed still yields a
        // VALID failing value (never a wrong one — every kept candidate is RE-RUN and confirmed to fail),
        // just not necessarily the global minimum. `hi_fail` = a known-failing upper bound (the current
        // counterexample's value); `lo_pass` = the greatest value known to PASS (or lo-1 if lo itself fails).
        let decoded = |payload: i64| lo.wrapping_add((payload & i64::MAX).rem_euclid(hi - lo + 1));
        let mut hi_fail = decoded(best[1]); // the current failing value
        let mut lo_pass = lo - 1; // exclusive lower fence: everything <= lo_pass is presumed passing
        while lo_pass + 1 < hi_fail {
            let mid = lo_pass + (hi_fail - lo_pass) / 2;
            // Run the property at decoded value `mid` (pool payload = mid - lo), without holding a borrow of
            // `best` across the mutation below.
            let outcome = {
                let mut trial = best.clone();
                trial[1] = mid.wrapping_sub(lo);
                run_pool(&trial).0
            };
            match outcome {
                TrialOutcome::Fail(m) => {
                    hi_fail = mid; // mid fails → the boundary is at or below mid
                    best[1] = mid.wrapping_sub(lo);
                    best_msg = m;
                }
                _ => lo_pass = mid, // mid passes → the boundary is above mid
            }
        }
        return PropertyFailure {
            inputs: best.iter().map(|n| n.to_string()).collect(),
            message: best_msg,
        };
    }
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
/// Generate one input tuple, SATISFYING the `@requires` constraints: per-param `bounds` are applied by
/// clamping in `draw_inputs`, and cross-param `relations` (e.g. `(< a b)`) are satisfied by REJECTION
/// SAMPLING — re-draw from a fresh derived seed until every relation holds, bounded by `RELATION_FUEL`
/// re-draws. Clamping keeps generation a pure function of the seed for the common (no-relation) case, so
/// reproducibility is unchanged; when relations ARE present, the returned tuple is still a deterministic
/// function of `seed` (the same seed re-derives the same accepted draw). If fuel is exhausted (a relation
/// too tight to hit by sampling), the last draw is returned unchanged — the (D) precondition trap then
/// fires and the property reports honestly rather than looping forever.
fn generate_inputs(
    gens: &[GenKind],
    bounds: &[ParamBound],
    relations: &[Relation],
    seed: u64,
) -> Vec<String> {
    // Draw, then PROPAGATE equalities (copy left→right so `(= a b)` holds by construction), then check the
    // remaining ORDER relations. Propagation is applied to every attempt so the order check sees the
    // post-propagation values.
    let mut first = draw_inputs(gens, bounds, seed);
    propagate_equalities(relations, &mut first);
    if relations.is_empty() || relations_hold(relations, &first) {
        return first;
    }
    // An ORDER relation is still violated — re-draw from a distinct derived seed until all hold, bounded by
    // fuel. The derived seed `seed ^ (k * ODD)` keeps every attempt a deterministic function of the original
    // seed. (Equalities always hold post-propagation, so only an unsatisfiable order relation exhausts fuel.)
    const RELATION_FUEL: u64 = 256;
    for k in 1..=RELATION_FUEL {
        let mut candidate = draw_inputs(gens, bounds, seed ^ k.wrapping_mul(0x9E3779B97F4A7C15));
        propagate_equalities(relations, &mut candidate);
        if relations_hold(relations, &candidate) {
            return candidate;
        }
    }
    first // fuel exhausted: return the first draw; the precondition trap reports honestly
}

/// Draw one input tuple from `seed`, applying only the per-param `bounds` clamps (no relation handling —
/// that is `generate_inputs`'s rejection loop). Split out so the rejection loop can re-draw cheaply.
fn draw_inputs(gens: &[GenKind], bounds: &[ParamBound], seed: u64) -> Vec<String> {
    use bolero_generator::driver::{self, Rng};
    use bolero_generator::{ValueGenerator, produce};
    let rng = rand_from_seed(seed);
    let mut d = Rng::new(rng, &driver::Options::default());
    gens.iter()
        .enumerate()
        .map(|(i, g)| match g {
            GenKind::Bool => {
                // A `@requires(b)` bare-Bool precondition pins this param (`bool_force`); otherwise draw
                // randomly. Pinning (not re-draw) keeps generation a pure function of the seed.
                match bounds.get(i).and_then(|b| b.bool_force) {
                    Some(forced) => forced.to_string(),
                    None => produce::<bool>()
                        .generate(&mut d)
                        .unwrap_or(false)
                        .to_string(),
                }
            }
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
                // `@requires`-constrained generation: if this param carries a recognized integer bound,
                // CLAMP the drawn value into it so the drawn input SATISFIES the precondition (the (D)
                // body-entry enforcement traps a violated pre — an out-of-domain draw would spuriously fail
                // the test). Clamp (not re-draw) keeps generation a pure function of the seed, so the
                // reproducibility contract (a replayed seed reproduces the inputs) still holds. An
                // unconstrained param is unchanged. The clamp is in the value's own signed i128 space, then
                // `render_int` re-narrows to the width.
                let bounded = match bounds.get(i) {
                    Some(b) if b.is_constrained() => b.clamp(raw as i128),
                    _ => raw as i128,
                };
                render_int(bounded as i64, *signed, *width)
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
    bounds: &[ParamBound],
    relations: &[Relation],
    inputs: &[String],
    message: Option<String>,
    run_one: &dyn Fn(&[String]) -> TrialOutcome,
) -> (Vec<String>, Option<String>) {
    let mut best = inputs.to_vec();
    let mut best_msg = message;
    for (i, g) in gens.iter().enumerate() {
        for candidate in shrink_candidates(g, &best[i]) {
            // A shrink candidate must still SATISFY this param's `@requires` bound — otherwise shrinking an
            // integer toward 0 could push it out of the precondition domain, and the (D) body-entry trap
            // would be mistaken for "still fails", yielding an out-of-domain (spurious) counterexample.
            // Skip a candidate the bound rejects (an unconstrained param admits every candidate).
            if let Some(b) = bounds.get(i)
                && b.is_constrained()
                && let Ok(n) = candidate.parse::<i64>()
                && b.clamp(n as i128) != n as i128
            {
                continue;
            }
            // Likewise a BOOL param pinned by `@requires(b)` must not shrink off its forced value — shrinking
            // `true`→`false` would break the precondition and trip the (D) pre-trap (a spurious "still fails").
            if let Some(b) = bounds.get(i)
                && let Some(forced) = b.bool_force
                && candidate != forced.to_string()
            {
                continue;
            }
            let mut trial = best.clone();
            trial[i] = candidate;
            // PROPAGATE equalities first: shrinking the LEFT param of `(= a b)` must carry to the right so the
            // pair stays equal (a shrink of the right param is a copy, harmlessly overwritten — the right is
            // slaved to the left). Then a shrink must not break a cross-param ORDER RELATION (`(< a b)`) —
            // shrinking `b` toward 0 could make `a < b` false, and the (D) trap would masquerade as "still
            // fails". Skip a trial that violates any relation (no relations ⇒ admits every candidate).
            propagate_equalities(relations, &mut trial);
            if !relations_hold(relations, &trial) {
                continue;
            }
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

/// The default content-addressed runtime store — the `CDZ_STORE` env var if set, else
/// `<repo>/target/cadenza-store` resolved relative to this binary (`target/<profile>/cdz` → up two →
/// `target` → `cadenza-store`). Mirrors `cdz-run`'s own `default_store` (flag > `CDZ_STORE` > compiled
/// default) so `cdz test`, `cdz run`, and a direct `cdz-run` all agree on where the value-heap runtime
/// lives — a single env var repoints the whole store at a Nix-provided path (R4).
fn default_store() -> PathBuf {
    if let Some(dir) = std::env::var_os("CDZ_STORE") {
        return PathBuf::from(dir);
    }
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
) -> cadenza_compile_abi::spans::SpanData {
    let spans: Vec<(u32, u32)> = (0..spantable.len())
        .map(
            |i| match spantable.get(cadenza_syntax::StructId(i as u32)) {
                Some(sp) => (sp.start as u32, (sp.end - sp.start) as u32),
                None => (0, 0),
            },
        )
        .collect();
    cadenza_compile_abi::spans::SpanData {
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
fn read_artifact_spec(spec: &str) -> Result<cadenza_compile_abi::Artifact, String> {
    // Split an optional `kind:` prefix (only when it looks like one), then an optional `name=` prefix.
    let (kind, rest) = match spec.split_once(':') {
        Some((k, r)) if !k.contains('/') && !k.contains('=') => (k.to_string(), r),
        _ => (cadenza_compile_abi::Artifact::KIND_AST.to_string(), spec),
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
    Ok(cadenza_compile_abi::Artifact::new(kind, name, bytes))
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

impl From<BuildTargetArg> for cadenza_compile_abi::Target {
    fn from(t: BuildTargetArg) -> cadenza_compile_abi::Target {
        match t {
            BuildTargetArg::Wasm => cadenza_compile_abi::Target::Wasm,
            BuildTargetArg::Rust => cadenza_compile_abi::Target::Rust,
        }
    }
}

// ── unit testing ───────────────────────────────────────────────────────────────────────────────────

/// The `cdz test --list` output format. `Binary` is the canonical cadenza-ast-binary `(test-list …)` value;
/// `Nix` is the eval-readable nix attrset-list projection for v-nix's scoped-cached-IFD discovery derivation.
#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum ListFormat {
    /// cadenza-ast-binary `(test-list (test <name> <is-property> <file>)…)` — the canonical form (default).
    #[value(name = "binary")]
    Binary,
    /// A pure nix attrset list `[ { name; is_property; file; } … ]`, sorted by (file, name), for IFD.
    #[value(name = "nix")]
    Nix,
}

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
    /// Report TIMING — like `cargo test --report-time`, so it's explicit "where the time goes". Off by
    /// default (the normal PASS/FAIL output is unchanged). When set, emits `⏱` lines. Up front, ALWAYS a
    /// `⏱ precompile: N shared-closure provider(s) emitted/loaded in Xms` line (the per-closure EMIT, cheap
    /// on a `.provider.wasm` cache hit / the heavy closure lower on a miss); and — ONLY when the suite has at
    /// least one shared closure to JIT (none → this line is omitted) — a `⏱ provider JIT: N shared closure(s)
    /// JIT'd/loaded once in Xms` line (JIT'd once per project, not per file — a fast cwasm deserialize on a
    /// hit, the full JIT on a miss). Then per test an indented `⏱ PASS|FAIL <name> Xms` under its result; and
    /// per file a `⏱ <file>: compose Xms · run Yms` (compose = that file's consumer JIT, run = all its
    /// tests). The up-front lines split the warm-once cost into EMIT vs JIT so a warming/gate run shows which
    /// cache missed.
    #[arg(long)]
    report_time: bool,
    /// WARM the shared-closure provider cache for the resolved suite, then EXIT WITHOUT running any tests.
    /// Emits + persists each closure GROUP's provider once (serially), so a SUBSEQUENT per-file `cdz test`
    /// sweep — e.g. the gate running each file as its own process for runaway-compile localization — HITS the
    /// cache instead of every file cold-emitting the shared closure in parallel (the N×-redundant-emit race:
    /// the 8 `sread-eval-*` files each re-emitting the ~1360-def provider). Warm-once-then-sweep collapses that
    /// to a single provider emit per closure. The `@test` check-gate still runs (a warm over a parse-broken
    /// suite fails RED, same as a normal run). Exit 0 iff the check passed and the providers were warmed.
    #[arg(long)]
    warm_only: bool,
    /// ENUMERATE the resolved suite's `@test` definitions as a cadenza-ast-binary value and EXIT — compile
    /// NOTHING, run nothing, link no wasmtime. Writes the `(test-list (test <name> <is-property> <file>)…)`
    /// value (`codec::encode`d, POSITIONAL fields) verbatim to stdout — the SAME shape the delegate compiler
    /// query (`Query::TestList`) emits, so `--list` is format-identical across the standalone + delegate
    /// builds (operator cadenza-ast-binary-everywhere directive, NO JSON; decode with `cdz convert --from
    /// binary` / the shared `codec`). The names come from the compiler `Db` (`db.test_defs`), NOT a source
    /// regex (the compiler's own source contains `@test` as a parsed token, so a regex massively over-counts).
    /// `is-property` is true for a `@test` that takes parameters or the `Test.gen` `-gen` wrapper (a nullary
    /// test is a plain unit test). This is the compiler-informed discovery source v-nix's DYNAMIC-DERIVATIONS
    /// fan-out reads to build one CA-derivation per test (no committed index, no IFD). Wasmtime-free by
    /// construction: it loads the import closure, builds the `Db`, and enumerates — the same front-half `cdz
    /// test` runs BEFORE any emit/JIT — so a `--no-default-features` `cdz` (no `cdz-run` link) can still
    /// produce it. Ignores `--filter`/`--tag` (a manifest must list the WHOLE suite). Peer of `--emit-shred`
    /// (which adds the per-test wasm + the full manifest as a BUILD output); `--list` is the NAMES-only half.
    #[arg(long)]
    list: bool,
    /// The `--list` output FORMAT. `binary` (DEFAULT): the canonical cadenza-ast-binary `(test-list …)` value
    /// (no-JSON mandate; a build-time decoder reads it). `nix`: a PURE, sorted, eval-readable nix attrset list
    /// `[ { name; is_property; file; } … ]` — the projection v-nix's scoped-cached-IFD test-shred discovery
    /// derivation writes to `$out` and the flake `import`s (nix-eval cannot parse the binary form). Only
    /// meaningful with `--list`.
    #[arg(long, value_enum, default_value_t = ListFormat::Binary)]
    format: ListFormat,
    /// SHRED the resolved suite into per-`@test` wasm COMPONENTS + a manifest, into `--out-dir`, and EXIT
    /// (compile only — NO run, no wasmtime). The compiler-driven test shred (operator model): each project
    /// file (its own shared-closure group) emits — via the `EmitTestsShred` sidecar, IN-PROCESS — a MAIN
    /// component (its emitted library, `main-<group>.wasm`) when it has one, plus one thin CONSUMER per `@test`
    /// (`test-<name>.wasm`) that links main + exports just that test; a file with no emitted library (all
    /// inlined/prims) emits SELF-CONTAINED per-test components + no main. Writes a single FLAT `<out-dir>/`:
    /// `main-<group>.wasm` (per group) + `test-<name>.wasm` (flat) + `manifest.cdzb` (ONE cadenza-ast-binary
    /// manifest, `(shred-manifest (entry name is-property file export target main-iface main-file)…)`). A
    /// runner (v-test-shred's nix matrix) then fans out one derivation per entry: `cdz-run <target> --call
    /// <export> [--peer <main-iface>=<main-file>] --store S` → exit code (0=PASS/trap=FAIL). Requires
    /// `--out-dir`. Wasmtime-free (the EMIT; the RUN is the external `cdz-run`). Peer of `--list` (the
    /// eval-time names-only half); this is the BUILD-output half.
    #[arg(long)]
    emit_shred: bool,
    /// The output directory for `--emit-shred` (created if absent). Required with `--emit-shred`.
    #[arg(long)]
    out_dir: Option<PathBuf>,
    /// STANDALONE `--emit-shred`: emit each `@test` as a SELF-CONTAINED component (its library inlined), NO
    /// main, `main-file=""` (the runner uses no `--peer`). The operator-approved hybrid uses this for
    /// SMALL-closure suites (iterators/cad/choreography) — no peer boundary, so a compound-param `@test` that
    /// would decline at the peer boundary (#4031) shreds cleanly here (FULL coverage), at the cost of
    /// re-embedding each test's closure (fine for a small closure; the shared-main peer path stays for the big
    /// compiler-ml closure). Only meaningful with `--emit-shred`.
    #[arg(long)]
    standalone: bool,
    /// TWO-STAGE `--emit-shred` (§S6b, standalone-everywhere heavy suites): emit cadenza-ast FRAGMENTS, not
    /// wasm — one shared-closure `closure-<group>.cdzb` (the reachable non-`@test` library) + one per-`@test`
    /// `test-<name>.cdzb`, with `main-file=closure-<group>.cdzb`/`target=test-<name>.cdzb` in the manifest.
    /// The fan-out then splice-COMPILES each: `rcdzc <closure> <test> --export <name> -o <name>.wasm` — so the
    /// heavy closure lowers ONCE + CA-caches (v-nix), each test is cheap codegen: O(closure_once + tests×body)
    /// instead of `--standalone`'s O(tests×closure). Only meaningful with `--emit-shred`; wins over
    /// `--standalone` if both are set.
    #[arg(long)]
    two_stage: bool,
}

// ── cdz watch ──────────────────────────────────────────────────────────────────────────────────

/// Which command `cdz watch` re-runs on each change. Kept to the project-scoped commands that read the
/// same `Project.cdz` (so the watched file set and the re-run target agree).
#[cfg(feature = "watch")]
#[derive(clap::ValueEnum, Clone, Copy, PartialEq, Eq)]
enum WatchCmd {
    /// Re-run `cdz check` (report diagnostics) — the default, the cheapest + most useful "on save" loop.
    Check,
    /// Re-run `cdz test` (the project's `@test` suite).
    Test,
    /// Re-run `cdz build` (compile the entry to a component).
    Build,
    /// Re-run `cdz run` (build the entry, then run it and print its value) — a live "run on save" loop.
    Run,
}

#[cfg(feature = "watch")]
#[derive(clap::Args)]
struct WatchArgs {
    /// The project to watch: a `Project.cdz` or a directory holding one. OMITTED → search up from the
    /// current directory for the nearest `Project.cdz` (like `cdz build`/`test`). The manifest's DIRECTORY
    /// is watched RECURSIVELY; a change re-runs only when it touches a Cadenza source file
    /// (`.cdz`/`.ml`/`.sexp`/`.sexpr`) or the manifest itself (build artifacts + editor temp files are
    /// ignored, so they never self-trigger).
    target: Option<String>,
    /// Which command to re-run on each change: `check` (default), `test`, `build`, or `run`.
    #[arg(long, value_enum, default_value_t = WatchCmd::Check)]
    exec: WatchCmd,
    /// The export to call each `--exec run` re-run (like `cdz run --call`). Ignored by the other execs.
    /// OMITTED → the entry's sole function export.
    #[arg(long)]
    call: Option<String>,
    /// An argument to pass to the `--exec run` entry, repeatable (like `cdz run --arg`) — so a `main`
    /// that TAKES arguments can be watched (before this, run-mode passed none, so an arg-taking entry
    /// errored on every run). `allow_hyphen_values` so a negative number (`--arg -4`) is the value, not
    /// a flag. Ignored by the other execs.
    #[arg(long = "arg", value_name = "VALUE", allow_hyphen_values = true)]
    args: Vec<String>,
    /// Run only `@test`s whose name CONTAINS this substring on each `--exec test` re-run (like `cdz test
    /// --filter`) — so a watch can focus one failing test instead of the whole suite. Ignored by the
    /// other execs.
    #[arg(long)]
    filter: Option<String>,
    /// Run only `@test`s carrying this `@tag("…")` on each `--exec test` re-run (like `cdz test --tag`);
    /// composes with `--filter` by AND. Ignored by the other execs.
    #[arg(long)]
    tag: Option<String>,
    /// Trials per PROPERTY test on each `--exec test` re-run (like `cdz test --trials`). Default 100.
    /// Ignored by the other execs.
    #[arg(long, default_value_t = 100)]
    trials: u64,
    /// The random SEED for property-input generation on each `--exec test` re-run (like `cdz test
    /// --seed`) — reproducible from it. Default 0. Ignored by the other execs.
    #[arg(long, default_value_t = 0)]
    seed: u64,
    /// Clear the terminal before EACH run (the initial one and every re-run), like `cargo watch -c` — so
    /// each run's output starts on a fresh screen instead of scrolling endlessly. Emits the ANSI
    /// clear-screen + cursor-home sequence; harmless if stdout is redirected to a file (it just writes the
    /// bytes). Off by default (output accumulates).
    #[arg(long)]
    clear: bool,
    /// The debounce window in milliseconds — filesystem events within this window of each other are
    /// COALESCED into a single re-run (so saving several files at once, or an editor's write-then-rename,
    /// triggers one run, not a storm). Default 400ms.
    #[arg(long, default_value_t = 400)]
    debounce_ms: u64,
    /// The runtime store `cdz test`/`cdz build` resolve the value-heap runtime from (passed through to the
    /// re-run). Defaults to `<repo>/target/cadenza-store`.
    #[arg(long)]
    store: Option<PathBuf>,
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
    /// Emit each reference as a machine-readable JSON object (one per line) instead of the human
    /// `file:line:col` text — the shape an editor consumes for a find-all-references result without
    /// re-parsing the text layout. Each object has `file` and, when the referencing node has a known
    /// span (the normal case), `line` + `col` (a spanless node falls back to a raw `node` id instead) —
    /// the `cdz symbols --json`/`cdz exports --json`/`cdz check --json` machine-readable convention.
    #[arg(long)]
    json: bool,
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
    /// Dump the compiler's RAW `KIND_DIAGNOSTICS` artifact bytes to stdout VERBATIM (the 8-column TAB wire
    /// `severity  code  node  fix-node  fix-replacement  fix-verified  message` that `Query::Diagnostics`
    /// emits) and exit 0 REGARDLESS of fault presence — the pass/fail call belongs to the CONSUMER, not the
    /// exit code. This is the machine wire a GRADER reads (`cdz-corpus-grade::parse_diagnostics`, the C1
    /// diagnostic-QUALITY grade), distinct from `--json` (which PROJECTS diagnostics to editor fix-edits
    /// `[{start,end,text}]` for an IDE/agent). Takes precedence over `--json`/`--verify-fixes` when combined;
    /// on a compile that produces no diagnostics artifact, emits nothing and still exits 0.
    #[arg(long)]
    diagnostics_wire: bool,
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

/// Parse a source BYTE OFFSET argument (`def`/`scope`/`type-at`/`doc-at`) with an ACTIONABLE message.
/// clap's default `usize` parser rejects a bad value with a bare `invalid digit found in string` — which
/// doesn't tell an editor/script author WHAT the argument is (a 0-based byte offset) or how it went wrong
/// (non-numeric vs negative). These are AI-native/editor-facing commands where a caller can easily pass a
/// stale or mis-typed offset, so name the expectation. A plain `usize::from_str` already rejects a leading
/// `-` (a negative offset is nonsensical) — surface that as its own note rather than the digit-parse blur.
fn parse_byte_offset(s: &str) -> Result<usize, String> {
    s.parse::<usize>().map_err(|_| {
        if s.starts_with('-') {
            format!("`{s}` is negative — a source byte offset is a 0-based, non-negative integer")
        } else {
            format!(
                "`{s}` is not a byte offset — expected a 0-based, non-negative integer (the UTF-8 byte \
                 position in the file, e.g. the editor cursor offset)"
            )
        }
    })
}

#[derive(clap::Args)]
struct DefArgs {
    /// The program file (`.cdz`/`.ml` → ml, `.sexp`/`.sexpr` → sexpr).
    file: String,
    /// The source BYTE OFFSET of the reference to jump from (0-based, UTF-8 bytes).
    #[arg(value_parser = parse_byte_offset)]
    offset: usize,
    /// Emit the definition location as a machine-readable JSON object (`{file, line, col}`) instead of the
    /// human `file:line:col` text — the shape an editor consumes for a go-to-definition jump without
    /// re-parsing the text (the `cdz symbols --json`/`cdz check --json` convention). No output when there
    /// is no navigable definition (a non-reference / a built-in) — that still exits non-zero with a note.
    #[arg(long)]
    json: bool,
}

#[derive(clap::Args)]
struct ScopeArgs {
    /// The program file (`.cdz`/`.ml` → ml, `.sexp`/`.sexpr` → sexpr).
    file: String,
    /// The source BYTE OFFSET whose visible bindings to list (0-based, UTF-8 bytes).
    #[arg(value_parser = parse_byte_offset)]
    offset: usize,
    /// Emit each visible binding as a machine-readable JSON object (one per line) instead of the human
    /// `file:line:col: name : type` text — the shape an editor consumes for a scope/completion view
    /// without re-parsing the text layout. Each object has `file`, `name`, `type`, and — when the binder
    /// has a known span (the normal case) — `line` + `col` (omitted for a span-less binder) — the `cdz
    /// symbols --json`/`cdz check --json` machine-readable convention.
    #[arg(long)]
    json: bool,
}

#[derive(clap::Args)]
struct ExportsArgs {
    /// The program file (`.cdz`/`.ml` → ml, `.sexp`/`.sexpr` → sexpr).
    file: String,
    /// Emit each export as a machine-readable JSON object (one per line) instead of the human
    /// `file:line:col: name : type` text — the shape a tool consumes to read a module's public interface
    /// without re-parsing the text layout. Each object has `file`, `name`, `type`, and — when the export's
    /// def has a known span (the normal case) — `line` + `col` (omitted for a span-less export) — the
    /// `cdz symbols --json`/`cdz check --json` machine-readable convention.
    #[arg(long)]
    json: bool,
}

#[derive(clap::Args)]
struct SymbolsArgs {
    /// The program file (`.cdz`/`.ml` → ml, `.sexp`/`.sexpr` → sexpr).
    file: String,
    /// Emit each declaration as a machine-readable JSON object (one per line) instead of the human
    /// `file:line:col: kind name` text — the shape an editor / tool consumes to build a symbol tree
    /// without re-parsing the text layout. Each object has `file`, `kind`, `name`, and — when the
    /// declaration's name node has a known span (the normal case) — `line` + `col` (omitted for a
    /// span-less declaration) — the `cdz check --json`/`cdz metadata` convention. The `documentSymbol`
    /// payload.
    #[arg(long)]
    json: bool,
}

#[derive(clap::Args)]
struct InstantiationsArgs {
    /// The generic / ad-hoc-polymorphic definition name whose concrete instantiations to enumerate.
    name: String,
    /// The program file (`.cdz`/`.ml` → ml, `.sexp`/`.sexpr` → sexpr).
    file: String,
}

#[derive(clap::Args)]
struct FuncLayoutArgs {
    /// The program file (`.cdz`/`.ml` → ml, `.sexp`/`.sexpr` → sexpr). Its whole import closure is laid
    /// out, so a package entry reports every reachable def across the linked program (the same set an
    /// emit produces), not just the entry file's own defs.
    file: String,
}

#[derive(clap::Args)]
struct ParamManifestArgs {
    /// The program file (`.cdz`/`.ml` → ml, `.sexp`/`.sexpr` → sexpr).
    file: String,
    /// Emit each `@param` site as a machine-readable JSON object (one per line) instead of the human
    /// `file:line:col: name : type [widget=… …]` text — the shape a widget HOST consumes to render
    /// controls without re-parsing. Each object has `name`, `type`, `widget` (or null), `range` (a
    /// `[lo, hi]` array or null), `options` (an array or null), `default` (the rendered value or null),
    /// `file`, and — when the param's name node has a known span — `line` + `col`. Null-not-omitted for
    /// absent config so the host gets a stable schema (the `cdz check --json`/`cdz metadata` convention).
    #[arg(long)]
    json: bool,
}

#[derive(clap::Args)]
struct HighlightArgs {
    /// The program file (`.cdz`/`.ml` → ml, `.sexp`/`.sexpr` → sexpr).
    file: String,
    /// Emit each classified token as a machine-readable JSON object (one per line) instead of the human
    /// `file:line:col: kind` text — the shape an editor consumes for semantic syntax highlighting without
    /// re-parsing the text layout. Each object has `file`, `line`, `col`, `kind` (the `cdz symbols
    /// --json`/`cdz check --json` machine-readable convention). The `semanticTokens` payload.
    #[arg(long)]
    json: bool,
}

#[derive(clap::Args)]
struct TypeAtArgs {
    /// The program file (`.cdz`/`.ml` → ml, `.sexp`/`.sexpr` → sexpr).
    file: String,
    /// The source BYTE OFFSET to type — the cursor position (0-based, UTF-8 bytes).
    #[arg(value_parser = parse_byte_offset)]
    offset: usize,
}

#[derive(clap::Args)]
struct DocArgs {
    /// The definition (or built-in) name to document.
    name: String,
    /// The program file (`.cdz`/`.ml` → ml, `.sexp`/`.sexpr` → sexpr).
    file: String,
    /// Emit the result as a JSON object instead of the raw doc text — the shape a tool/editor consumes
    /// for a hover without pattern-matching the prose. `{name, exists, documented, doc}`: `exists` is
    /// false only for an unresolvable name (a typo); `documented` is true only when the name carries doc
    /// text; `doc` is that text, or null when absent/unknown. So a consumer distinguishes the three total
    /// outcomes (documented / exists-but-undocumented / no-such-definition) without parsing sentinels.
    #[arg(long)]
    json: bool,
}

#[derive(clap::Args)]
struct DocAtOffsetArgs {
    /// The program file (`.cdz`/`.ml` → ml, `.sexp`/`.sexpr` → sexpr).
    file: String,
    /// The source BYTE OFFSET whose documentation to show — the cursor position (0-based, UTF-8 bytes).
    #[arg(value_parser = parse_byte_offset)]
    offset: usize,
}

#[derive(clap::Args)]
struct DocModuleArgs {
    /// The program file (`.cdz`/`.ml` → ml, `.sexp`/`.sexpr` → sexpr).
    file: String,
    /// Output format for the doc-AST. Defaults to `binary` (canonical `cdzast\x00\x01` — the doc index
    /// is a binary AST); use `sexpr`/`ml` to inspect it.
    #[arg(short, long, value_enum)]
    to: Option<syntax_cli::Fmt>,
    /// The module name recorded in the emitted `(doc-module "…")`. Defaults to the file's stem.
    #[arg(short, long)]
    module: Option<String>,
    /// Target line width for a text output surface.
    #[arg(short, long, default_value_t = 100)]
    width: usize,
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
        cadenza_compile_abi::Request::Query(cadenza_compile_abi::sidecar::Query::TypeOf {
            name: args.name.clone(),
        }),
    );
    match out.artifact(cadenza_compile_abi::sidecar::KIND_TYPE_INFO) {
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
    let (source, arenas, spans) = load_spanned_or_bail!(&args.file);
    let Some(node) = spans.node_at_offset(args.offset) else {
        eprintln!(
            "{PROG}: no node at byte offset {} in {}",
            args.offset, args.file
        );
        return ExitCode::FAILURE;
    };
    let out = run_sidecar(
        &arenas,
        cadenza_compile_abi::Request::Query(cadenza_compile_abi::sidecar::Query::TypeAt {
            node: node.0,
        }),
    );
    let Some(bytes) = out.artifact(cadenza_compile_abi::sidecar::KIND_TYPE_AT) else {
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
        cadenza_compile_abi::Request::Query(cadenza_compile_abi::sidecar::Query::DocOf {
            name: args.name.clone(),
        }),
    );
    match out.artifact(cadenza_compile_abi::sidecar::KIND_DOC) {
        Some(bytes) => {
            let text = String::from_utf8_lossy(bytes);
            // The `DocOf` query is TOTAL — it returns a doc artifact for THREE outcomes: the doc text, a
            // "no documentation for `X`" line (a REAL definition that carries no doc), and a "no such
            // definition `X`" line (the name resolves to NOTHING — a typo). The first two are a SUCCESS
            // (`X` exists; asking for its doc is a legitimate answer), but an unresolvable name is a
            // FAILURE — a caller/script should tell "you misspelled the name" from "this exists but is
            // undocumented". `is_no_such_definition` matches the exact sentinel for the queried name (not a
            // loose prefix on the doc prose — the pr467 brittleness fix, shared with `cdz type`).
            let no_such = is_no_such_definition(&text, &args.name);
            // The undocumented-but-real sentinel is the compiler's exact `no documentation for `X`` line
            // (rcdzc `DocOf`), matched precisely (not a loose prefix) so a real doc that happens to start
            // with those words isn't misread.
            let undocumented = text.trim() == format!("no documentation for `{}`", args.name);
            if args.json {
                use cadenza_syntax::query::json;
                let mut obj = json::Object::new();
                obj.string("name", &args.name);
                obj.raw("exists", if no_such { "false" } else { "true" });
                obj.raw(
                    "documented",
                    if !no_such && !undocumented {
                        "true"
                    } else {
                        "false"
                    },
                );
                // `doc` is the actual doc text only when documented; null for the two "no doc" outcomes,
                // so a consumer never mistakes a sentinel line for real documentation.
                if !no_such && !undocumented {
                    obj.string("doc", text.trim_end());
                } else {
                    obj.raw("doc", "null");
                }
                println!("{}", obj.finish());
            } else {
                println!("{text}");
            }
            if no_such {
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

/// `cdz doc-module FILE` — extract FILE's public doc surface into a TYPE-ENRICHED `doc-module` doc-AST
/// (cadenza-docs I2). The (option-C) ASSEMBLY point: the structural projection is single-sourced in
/// `cadenza_syntax::doc_item::project` (I1), the resolved types come from the compiler's sidecar
/// (`Query::ExportedTypes`), and this bin — the one place both crates meet — merges them via
/// `crate::doc_module`. Emits the doc-AST to stdout (canonical binary by default; a surface via `--to`).
fn run_doc_module(args: &DocModuleArgs) -> ExitCode {
    use cadenza_syntax::convert::{self, Format, Options};

    let (_source, arenas) = match load_program(&args.file) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{PROG}: {e}");
            return ExitCode::FAILURE;
        }
    };

    // The module name: `--module`, else the file's stem, else "module".
    let module_name = args.module.clone().unwrap_or_else(|| {
        std::path::Path::new(&args.file)
            .file_stem()
            .and_then(|s| s.to_str())
            .map(str::to_string)
            .unwrap_or_else(|| "module".to_string())
    });

    // 1. Resolved types from the compiler sidecar (Query::ExportedTypes → the KIND_EXPORT_TYPES blob).
    let sidecar_out = run_sidecar(
        &arenas,
        cadenza_compile_abi::Request::Query(cadenza_compile_abi::sidecar::Query::ExportedTypes),
    );
    let types = match sidecar_out.artifact(cadenza_compile_abi::sidecar::KIND_EXPORT_TYPES) {
        Some(blob) => doc_module::parse_export_types(blob),
        // No blob (a program with no exports / a compile fault): proceed with no types — the structural
        // doc-module still emits, items just carry no (ty …). A doc build never hard-fails here.
        None => std::collections::BTreeMap::new(),
    };

    // 2. Structural projection (I1) + 3. merge the resolved (ty …) into each doc-item.
    let structural = cadenza_syntax::doc_item::project(&arenas, &module_name);
    let doc_ast = doc_module::merge_types(&structural, &types);

    // 4. Emit the doc-AST — canonical binary by default, or a text surface via --to.
    let to = args.to.map(Format::from).unwrap_or(Format::Binary);
    let opts = Options {
        width: args.width,
        ..Options::default()
    };
    let output = match convert::write_with(&doc_ast, to, opts) {
        Ok(bytes) => bytes,
        Err(e) => {
            eprintln!("{PROG}: emitting doc-module: {e}");
            return ExitCode::FAILURE;
        }
    };
    use std::io::Write;
    if std::io::stdout().write_all(&output).is_err() {
        return ExitCode::FAILURE;
    }
    if to != Format::Binary {
        let _ = std::io::stdout().write_all(b"\n");
    }
    ExitCode::SUCCESS
}

/// `cdz doc-at FILE OFFSET` — the "documentation at cursor" query. Resolves the source byte offset to the
/// innermost node id (via the span table this process kept), drives the compiler's `DocAt { node }` query,
/// and prints the documentation of the definition that node is or references. The offset→node split keeps
/// the compiler span-free, exactly as `type-at`/`def` do. An empty result (a node that documents nothing)
/// prints a "no documentation" line.
fn run_doc_at(args: &DocAtOffsetArgs) -> ExitCode {
    let (source, arenas, spans) = load_spanned_or_bail!(&args.file);
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
        cadenza_compile_abi::Request::Query(cadenza_compile_abi::sidecar::Query::DocAt {
            node: node.0,
        }),
    );
    let Some(bytes) = out.artifact(cadenza_compile_abi::sidecar::KIND_DOC) else {
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
    let (source, arenas, spans) = load_spanned_or_bail!(&args.file);
    let out = run_sidecar(
        &arenas,
        cadenza_compile_abi::Request::Query(cadenza_compile_abi::sidecar::Query::UsesOf {
            name: args.name.clone(),
        }),
    );
    let Some(bytes) = out.artifact(cadenza_compile_abi::sidecar::KIND_USES) else {
        report_errors(&out);
        return ExitCode::FAILURE;
    };
    let text = String::from_utf8_lossy(bytes);
    // Each line is a bare reference node-id. A non-empty line that is NOT an integer is a sidecar
    // format skew — flag it (fail at the end) rather than silently dropping the reference (PR #525's
    // silent-drop class); a blank line is skipped.
    let mut malformed = false;
    let mut ids: Vec<u32> = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        match t.parse::<u32>() {
            Ok(id) => ids.push(id),
            Err(_) => {
                report_malformed_query_row("uses", line);
                malformed = true;
            }
        }
    }
    if ids.is_empty() && !malformed {
        eprintln!("{PROG}: no references to `{}` in {}", args.name, args.file);
        return ExitCode::SUCCESS;
    }
    // ONE line-start index over the source, so each reference's line:col is a binary search, not a
    // from-start newline scan — `cdz uses` over N references was O(N × source_len) = O(N²) (a name with
    // 4000 references = 207ms, 99.9% in `line_col`); with the index it is linear.
    let index = cadenza_syntax::query::driver::LineIndex::new(&source);
    for id in ids {
        let line_col = spans
            .get(cadenza_syntax::StructId(id))
            .map(|span| index.line_col(&source, span.start));
        if args.json {
            use cadenza_syntax::query::json;
            let mut obj = json::Object::new();
            obj.string("file", &args.file);
            match line_col {
                Some((line, col)) => {
                    obj.raw("line", &line.to_string());
                    obj.raw("col", &col.to_string());
                }
                // A referencing occurrence with no recorded span (should not happen for a user node)
                // still reports the raw id rather than dropping it silently.
                None => obj.raw("node", &id.to_string()),
            }
            println!("{}", obj.finish());
        } else {
            match line_col {
                Some((line, col)) => println!("{}:{line}:{col}", args.file),
                // A referencing occurrence with no recorded span (should not happen for a user node)
                // still reports the raw id rather than dropping it silently.
                None => println!("{}:node {id}", args.file),
            }
        }
    }
    if malformed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
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
        cadenza_compile_abi::Request::Query(cadenza_compile_abi::sidecar::Query::Diagnostics),
    );
    let bytes = out.artifact(cadenza_compile_abi::sidecar::KIND_DIAGNOSTICS)?; // no artifact → failed at entry
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
        let (had_error, closure_paths) =
            check_one(f, args.json, args.verify_fixes, args.diagnostics_wire);
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
///
/// CODED faults surface (incl. emit/lowering); CODELESS declines do NOT. The diagnostics come from
/// `Query::Diagnostics` (`compile::diagnostics` → `collect_faults`), which is NOT front-end-only — it
/// surfaces error-severity CODED faults from the whole pipeline, including emit/lowering ones.
/// `compile.rs`'s `layout` decline path deliberately runs `collect_faults` and reports the coded fault
/// set precisely so `check` ≡ `compile` FOR CODED FAULTS (its comments call out avoiding a "check≡compile
/// discrepancy"). Verified on trunk, both sides run both ways:
///   - CDZ0304 constant `(/ 5 0)` (a coded lowering reject) -> check exit=1, surfaced. [SURFACES]
///   - LITERAL compound-ordering `(1.0,2.0) < (3.0,4.0)` -> FOLDS to a coded fault -> check exit=1.
///     [SURFACES]
///   - PARAMETER compound-ordering `f(x: Float64, y: Float64) = (x,1) < (y,2)` -> a CODELESS
///     `Reject::decline` (the float-leaf-no-total-order carve-out) -> check exit=0 while `cdz compile`
///     exit=1. [HIDDEN] -- `collect_faults` has no code to collect for a code-less decline.
///
/// So the gap is precisely the CODELESS emit-path declines, not "check skips emit" (it doesn't). A prior
/// framing that called check "front-end only" (and, in the first correction, "≡ compile on rejects") was
/// wrong on both counts — the accurate split is coded-surfaces / codeless-hides, and a const-foldable
/// LITERAL masks it (probe with a PARAMETER). Whether the PERMANENT carve-outs (float-leaf compound
/// ordering/compare, bare-float compare) should be promoted from codeless decline to a CODED rejection —
/// which would make them surface here for free like CDZ0304 — is a spec decline-vs-reject question in
/// flight (v-diagnostics owns the reclassification; the corpus pins them as `(declines)` at
/// `03-equality-and-observation.sexp:720/739/783`, so the category flip is an operator call). If the
/// ruling keeps them declines but asks check to surface PERMANENT ones, that marker-consumption lands
/// HERE (this crate); if it makes them coded, no change here is needed.
fn check_one(
    file: &str,
    json: bool,
    verify_fixes: bool,
    diagnostics_wire: bool,
) -> (bool, Vec<String>) {
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
        let mut inputs: Vec<cadenza_compile_abi::Artifact> = files
            .iter()
            .map(|f| {
                cadenza_compile_abi::Artifact::new(
                    cadenza_compile_abi::Artifact::KIND_AST,
                    f.name.clone(),
                    cadenza_syntax::codec::encode(&f.arenas),
                )
            })
            .collect();
        inputs.push(cadenza_compile_abi::Artifact::new(
            cadenza_compile_abi::sidecar::KIND_SIDECAR,
            "drive",
            cadenza_compile_abi::sidecar::encode(&[cadenza_compile_abi::Request::Query(
                cadenza_compile_abi::sidecar::Query::Diagnostics,
            )]),
        ));
        inputs.push(cadenza_compile_abi::abi::entry_artifact(&files[0].name));
        dispatch_query_over_inputs(inputs, cadenza_compile_abi::sidecar::KIND_DIAGNOSTICS)
    } else {
        run_sidecar(
            &files[0].arenas,
            cadenza_compile_abi::Request::Query(cadenza_compile_abi::sidecar::Query::Diagnostics),
        )
    };
    let Some(bytes) = out.artifact(cadenza_compile_abi::sidecar::KIND_DIAGNOSTICS) else {
        // `--diagnostics-wire`: no artifact = the diagnostics query failed to produce a wire (a compile that
        // didn't reach the fault set). The GRADER decides pass/fail, so emit nothing + exit 0 (never a hard
        // error), rather than the normal error path — the wire mode's contract is "the raw bytes, or empty".
        if diagnostics_wire {
            return (false, files.into_iter().map(|f| f.path).collect());
        }
        report_errors(&out);
        // The diagnostics query itself failed = an error. The closure DID load, so still hand its paths
        // back for the caller's coverage set (the files were checked as far as this point).
        let closure_paths = files.into_iter().map(|f| f.path).collect();
        return (true, closure_paths);
    };
    // `--diagnostics-wire`: dump the RAW `KIND_DIAGNOSTICS` bytes to stdout VERBATIM and return WITHOUT
    // demuxing/formatting (that is `--json`/human's job) — the machine wire a grader parses. Exit 0
    // regardless of faults (`had_error=false`): the CONSUMER judges quality, not this exit code.
    if diagnostics_wire {
        use std::io::Write;
        let _ = std::io::stdout().write_all(bytes);
        return (false, files.into_iter().map(|f| f.path).collect());
    }
    let text = String::from_utf8_lossy(bytes);
    let mut any_error = false;
    // The package demux table (`link-map`) — absent for a single file, so every node belongs to the
    // entry with its local id == the global id.
    let link_map = out
        .artifact(cadenza_compile_abi::link_map::KIND_LINK_MAP)
        .map(cadenza_compile_abi::link_map::decode_link_map)
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
        // NOTE (historical): an `insert` fix splices ARM/child scaffold whose syntax only exists
        // in-context (a match/handle arm). This was previously DROPPED on the ML surface because the
        // textedit `render` printed a spliced arm node in ISOLATION — an arm `(pat body)` rendered
        // standalone as the APPLICATION `pat(body)`, not `| pat => body`, so the splice was invalid ML.
        // v-syntax's `render_child` fix (`2da1d3bb3`) now renders an inserted match arm AS an arm
        // (`\n  | pat => body`, `|`-led, reparseable), so an `insert` on an ML file flows through the
        // now-correct render path and produces a valid splice — no longer dropped. Safety net remains:
        // `do_fix_edits` only advertises the fix when the built patch is non-empty (below), and `cdz fix`
        // re-parses the applied result, so any residual insert shape that still can't render into ML
        // self-drops (no fix advertised) rather than emitting a broken edit. So we no longer null the
        // node up front — the InsertArms CDZ0210 "add the missing arm" fix is now applyable on `.cdz`
        // (the primary user surface), which it wasn't before this + the render fix.

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
        // An `insert` fix splices ARM/child scaffold whose syntax is CONTEXT-SENSITIVE on the ML surface:
        // a match arm renders `| pat => body` (valid, since v-syntax's `render_child` handles it), but a
        // handle-op arm renders as a bare application `op(…)` that does NOT parse in a handler position
        // (`expected in` / `keyword used outside its form`). So a single blanket "ML inserts are fine" is
        // WRONG (it surfaced a CDZ0405 effect-handler fix whose splice is invalid ML) and so is a blanket
        // "ML inserts are dropped" (it hid the CDZ0210 match-arm fix, the flagship actionable fix). The
        // sound test is BEHAVIORAL: apply the ML insert and re-parse — keep it only if the result parses.
        // A match-arm insert survives (valid ML), a handle-op insert self-drops (unparseable), with no
        // hardcoded per-diagnostic list. Only ML `insert` pays the extra parse (s-expr arms render the
        // same in/out of context; replace/wrap/delete are already surface-correct).
        // Resolve the fix's OWN target file (a fix may land in an imported library, not the entry) to
        // decide surface + validate the splice against that file.
        let fix_target_is_ml = file_of_node(fix_node)
            .map(|(fi, _)| is_ml_source(&files[fi].path))
            .unwrap_or(false);
        let ml_insert_reparses = |kind: &str, node: &str, repl: &str| -> bool {
            match do_fix_apply(kind, node, repl) {
                Some(edited) => cadenza_syntax::parser::read_ml(&edited).errors.is_empty(),
                None => false,
            }
        };
        let drop_unparseable_ml_insert = fix_kind == "insert"
            && fix_node != "-"
            && fix_target_is_ml
            && !ml_insert_reparses(fix_kind, fix_node, fix_repl);

        let patch = if fix_node != "-" && !drop_unparseable_ml_insert {
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
    let (source, arenas, _) = load_spanned_or_bail!(&args.file);
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
            cadenza_compile_abi::Request::Query(cadenza_compile_abi::sidecar::Query::Diagnostics),
        );
        let Some(bytes) = out.artifact(cadenza_compile_abi::sidecar::KIND_DIAGNOSTICS) else {
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
    let (source, arenas, spans) = load_spanned_or_bail!(&args.file);
    let Some(node) = spans.node_at_offset(args.offset) else {
        eprintln!(
            "{PROG}: no node at byte offset {} in {}",
            args.offset, args.file
        );
        return ExitCode::FAILURE;
    };
    let out = run_sidecar(
        &arenas,
        cadenza_compile_abi::Request::Query(cadenza_compile_abi::sidecar::Query::ResolveOf {
            node: node.0,
        }),
    );
    let Some(bytes) = out.artifact(cadenza_compile_abi::sidecar::KIND_RESOLVE) else {
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
            if args.json {
                use cadenza_syntax::query::json;
                let mut obj = json::Object::new();
                obj.string("file", &args.file);
                obj.raw("line", &line.to_string());
                obj.raw("col", &col.to_string());
                println!("{}", obj.finish());
            } else {
                println!("{}:{line}:{col}", args.file);
            }
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
    let (source, arenas, spans) = load_spanned_or_bail!(&args.file);
    let Some(node) = spans.node_at_offset(args.offset) else {
        eprintln!(
            "{PROG}: no node at byte offset {} in {}",
            args.offset, args.file
        );
        return ExitCode::FAILURE;
    };
    let out = run_sidecar(
        &arenas,
        cadenza_compile_abi::Request::Query(cadenza_compile_abi::sidecar::Query::ScopeAt {
            node: node.0,
        }),
    );
    let Some(bytes) = out.artifact(cadenza_compile_abi::sidecar::KIND_SCOPE) else {
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
    // Both output shapes — the human `file:line:col: name : type` and the `--json` object — are computed
    // from the SAME resolved `(name, type, line, col)` so they can't drift (mirrors `cdz exports`).
    let index = cadenza_syntax::query::driver::LineIndex::new(&source);
    let mut malformed = false;
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let mut cols = line.splitn(3, '\t');
        let (name, ty, binder) = match (cols.next(), cols.next(), cols.next()) {
            (Some(n), Some(t), Some(b)) => (n, t, b),
            _ => {
                report_malformed_query_row("scope", line);
                malformed = true;
                continue;
            }
        };
        let line_col = binder
            .parse::<u32>()
            .ok()
            .and_then(|b| spans.get(cadenza_syntax::StructId(b)))
            .map(|span| index.line_col(&source, span.start));
        if args.json {
            use cadenza_syntax::query::json;
            let mut obj = json::Object::new();
            obj.string("file", &args.file);
            if let Some((l, c)) = line_col {
                obj.raw("line", &l.to_string());
                obj.raw("col", &c.to_string());
            }
            obj.string("name", name);
            obj.string("type", ty);
            println!("{}", obj.finish());
        } else {
            let loc = match line_col {
                Some((l, c)) => format!("{}:{l}:{c}", args.file),
                None => args.file.clone(),
            };
            println!("{loc}: {name} : {ty}");
        }
    }
    if malformed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// `cdz exports FILE` — the module's exported interface. Drives `Query::Exports` (each exported name +
/// its type + the def's name node), and prints `file:line:col: name : type` per export. The
/// module-interface-at-a-glance view.
fn run_exports(args: &ExportsArgs) -> ExitCode {
    let (source, arenas, spans) = load_spanned_or_bail!(&args.file);
    let out = run_sidecar(
        &arenas,
        cadenza_compile_abi::Request::Query(cadenza_compile_abi::sidecar::Query::Exports),
    );
    let Some(bytes) = out.artifact(cadenza_compile_abi::sidecar::KIND_EXPORTS) else {
        report_errors(&out);
        return ExitCode::FAILURE;
    };
    let text = String::from_utf8_lossy(bytes);
    if text.trim().is_empty() {
        eprintln!("{PROG}: {} exports nothing", args.file);
        return ExitCode::SUCCESS;
    }
    // Each line is `name<TAB>type<TAB>def-name-node-id` (`-` when the export names no def).
    // One line-start index (binary-searched line:col) so a wide export list stays linear. Both output
    // shapes — the human `file:line:col: name : type` and the `--json` object — are computed from the
    // SAME resolved `(name, type, line, col)` so they can't drift (mirrors `cdz symbols`).
    let index = cadenza_syntax::query::driver::LineIndex::new(&source);
    let mut malformed = false;
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let mut cols = line.splitn(3, '\t');
        let (name, ty, node) = match (cols.next(), cols.next(), cols.next()) {
            (Some(n), Some(t), Some(d)) => (n, t, d),
            _ => {
                report_malformed_query_row("exports", line);
                malformed = true;
                continue;
            }
        };
        let line_col = node
            .parse::<u32>()
            .ok()
            .and_then(|d| spans.get(cadenza_syntax::StructId(d)))
            .map(|span| index.line_col(&source, span.start));
        if args.json {
            use cadenza_syntax::query::json;
            let mut obj = json::Object::new();
            obj.string("file", &args.file);
            if let Some((l, c)) = line_col {
                obj.raw("line", &l.to_string());
                obj.raw("col", &c.to_string());
            }
            obj.string("name", name);
            obj.string("type", ty);
            println!("{}", obj.finish());
        } else {
            let loc = match line_col {
                Some((l, c)) => format!("{}:{l}:{c}", args.file),
                None => args.file.clone(),
            };
            println!("{loc}: {name} : {ty}");
        }
    }
    if malformed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// `cdz symbols FILE` — the document OUTLINE: every top-level declaration classified by kind, as
/// `file:line:col: kind name`. Rides the `Symbols` sidecar query, then maps each declaration's NAME node
/// to a source location through the span table. The superset companion of `cdz exports` — it lists EVERY
/// declaration (private ones too), not just the exported subset, so an editor can render a symbol tree.
fn run_symbols(args: &SymbolsArgs) -> ExitCode {
    let (source, arenas, spans) = load_spanned_or_bail!(&args.file);
    let out = run_sidecar(
        &arenas,
        cadenza_compile_abi::Request::Query(cadenza_compile_abi::sidecar::Query::Symbols),
    );
    let Some(bytes) = out.artifact(cadenza_compile_abi::sidecar::KIND_SYMBOLS) else {
        report_errors(&out);
        return ExitCode::FAILURE;
    };
    let text = String::from_utf8_lossy(bytes);
    if text.trim().is_empty() {
        eprintln!("{PROG}: {} declares nothing", args.file);
        return ExitCode::SUCCESS;
    }
    // Each line is `name<TAB>kind<TAB>name-node-id`. One line-start index (binary-searched line:col) so a
    // wide declaration list stays linear (the same swap `exports`/`highlight` carry). Both output shapes —
    // the human `file:line:col: kind name` and the `--json` object — are computed from the SAME resolved
    // `(name, kind, line, col)` so they can't drift.
    let index = cadenza_syntax::query::driver::LineIndex::new(&source);
    let mut malformed = false;
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let mut cols = line.splitn(3, '\t');
        let (name, kind, node) = match (cols.next(), cols.next(), cols.next()) {
            (Some(n), Some(k), Some(d)) => (n, k, d),
            // A row that isn't the expected `name<TAB>kind<TAB>node` shape is a sidecar format skew — fail
            // loudly rather than silently drop the symbol (PR #525's silent-drop class).
            _ => {
                report_malformed_query_row("symbols", line);
                malformed = true;
                continue;
            }
        };
        // Resolve the name-node id to a `line:col` (or `None` if the span table has no entry — then the
        // human form prints just the file and the JSON omits line/col).
        let line_col = node
            .parse::<u32>()
            .ok()
            .and_then(|d| spans.get(cadenza_syntax::StructId(d)))
            .map(|span| index.line_col(&source, span.start));
        if args.json {
            use cadenza_syntax::query::json;
            let mut obj = json::Object::new();
            obj.string("file", &args.file);
            if let Some((l, c)) = line_col {
                obj.raw("line", &l.to_string());
                obj.raw("col", &c.to_string());
            }
            obj.string("kind", kind);
            obj.string("name", name);
            println!("{}", obj.finish());
        } else {
            let loc = match line_col {
                Some((l, c)) => format!("{}:{l}:{c}", args.file),
                None => args.file.clone(),
            };
            println!("{loc}: {kind} {name}");
        }
    }
    if malformed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// `cdz param-manifest FILE` — the `@param` WIDGET MANIFEST: one record per `@param(widget: …) name : Type`
/// site, the data a HOST (browser/CAD/notebook) reads to render a control per program parameter. Drives
/// `Query::ParamManifest` (the sidecar half, owned jointly with v-metaprogramming's `scan_manifest`), whose
/// wire answer is one TAB-separated line per site
/// `name<TAB>widget<TAB>type<TAB>range-lo<TAB>range-hi<TAB>options<TAB>default<TAB>name-node`. The compiler
/// renders the declared TYPE (its type column); the value fields are ARENA NODE IDS this CLI renders from
/// the shared-`StructId` source arena (`sexpr::print_from`), and `name-node` is mapped to `file:line:col`
/// via the span table — the "compiler emits identity, front-end owns spans + value rendering" split. Human
/// form: `file:line:col: name : type [widget=… range=[lo,hi] options=… default=…]` per site (the bracketed
/// config only for present fields); `--json` emits one object per param with null-not-omitted config.
fn run_param_manifest(args: &ParamManifestArgs) -> ExitCode {
    let (source, arenas, spans) = load_spanned_or_bail!(&args.file);
    let out = run_sidecar(
        &arenas,
        cadenza_compile_abi::Request::Query(cadenza_compile_abi::sidecar::Query::ParamManifest),
    );
    let Some(bytes) = out.artifact(cadenza_compile_abi::sidecar::KIND_PARAM_MANIFEST) else {
        report_errors(&out);
        return ExitCode::FAILURE;
    };
    let text = String::from_utf8_lossy(bytes);
    if text.trim().is_empty() {
        eprintln!("{PROG}: {} declares no @param sites", args.file);
        return ExitCode::SUCCESS;
    }
    let index = cadenza_syntax::query::driver::LineIndex::new(&source);
    // Render an arena value-node id (from the wire) to its source s-expression, or `None` for the `-`
    // sentinel (an absent config field). Node-id-keyed → the shared `StructId` space lets this CLI print
    // the exact source form the compiler pointed at, without re-parsing.
    let render_node = |field: &str| -> Option<String> {
        let id: u32 = field.parse().ok()?;
        Some(cadenza_syntax::sexpr::print_from(
            &arenas,
            cadenza_syntax::StructId(id),
        ))
    };
    // Track whether any NON-EMPTY line failed to parse into the expected 8-column shape. A blank line is
    // skipped silently (harmless); a malformed row is a LOUD error, not a silent drop — otherwise a sidecar
    // output-format regression would drop params while `cdz param-manifest` still reported success, masking
    // the break (Copilot PR #525 — the silent-pass-on-malformed-input class). We report the offending line
    // and FAIL at the end, so a format skew surfaces immediately instead of a quietly-short manifest.
    let mut malformed = false;
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        // `name  widget  type  range-lo  range-hi  options  default  name-node` — split into exactly 8.
        let cols: Vec<&str> = line.splitn(8, '\t').collect();
        let [
            name,
            widget,
            ty,
            range_lo,
            range_hi,
            options,
            default,
            name_node,
        ] = cols[..]
        else {
            eprintln!(
                "{PROG}: internal: could not parse a param-manifest row (expected 8 tab-separated \
                 fields, got {}) — the sidecar output format may have changed: {line:?}",
                cols.len()
            );
            malformed = true;
            continue;
        };
        let widget = (widget != "-").then_some(widget);
        // A range is the two element nodes rendered as `[lo, hi]` (both present or neither).
        let range = match (render_node(range_lo), render_node(range_hi)) {
            (Some(lo), Some(hi)) => Some((lo, hi)),
            _ => None,
        };
        let options = render_node(options);
        let default = render_node(default);
        let line_col = name_node
            .parse::<u32>()
            .ok()
            .and_then(|d| spans.get(cadenza_syntax::StructId(d)))
            .map(|span| index.line_col(&source, span.start));
        if args.json {
            use cadenza_syntax::query::json;
            let mut obj = json::Object::new();
            obj.string("file", &args.file);
            if let Some((l, c)) = line_col {
                obj.raw("line", &l.to_string());
                obj.raw("col", &c.to_string());
            }
            obj.string("name", name);
            obj.string("type", ty);
            // Null-not-omitted for absent config, so a host gets a STABLE schema across sites.
            match widget {
                Some(w) => obj.string("widget", w),
                None => obj.raw("widget", "null"),
            }
            match &range {
                Some((lo, hi)) => {
                    let mut arr = json::Array::new();
                    arr.string(lo);
                    arr.string(hi);
                    obj.raw("range", &arr.finish());
                }
                None => obj.raw("range", "null"),
            }
            match &options {
                Some(o) => obj.string("options", o),
                None => obj.raw("options", "null"),
            }
            match &default {
                Some(d) => obj.string("default", d),
                None => obj.raw("default", "null"),
            }
            println!("{}", obj.finish());
        } else {
            let loc = match line_col {
                Some((l, c)) => format!("{}:{l}:{c}", args.file),
                None => args.file.clone(),
            };
            // The bracketed config lists only PRESENT fields (a compact human summary).
            let mut cfg = Vec::new();
            if let Some(w) = widget {
                cfg.push(format!("widget={w}"));
            }
            if let Some((lo, hi)) = &range {
                cfg.push(format!("range=[{lo},{hi}]"));
            }
            if let Some(o) = &options {
                cfg.push(format!("options={o}"));
            }
            if let Some(d) = &default {
                cfg.push(format!("default={d}"));
            }
            let cfg = if cfg.is_empty() {
                String::new()
            } else {
                format!(" [{}]", cfg.join(" "))
            };
            println!("{loc}: {name} : {ty}{cfg}");
        }
    }
    // A malformed row means the command could not faithfully read its own tool's output — fail loudly
    // rather than return success with a silently-short manifest.
    if malformed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
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
    let (source, arenas, spans) = load_spanned_or_bail!(&args.file);
    let out = run_sidecar(
        &arenas,
        cadenza_compile_abi::Request::Query(cadenza_compile_abi::sidecar::Query::Instantiations {
            name: args.name.clone(),
        }),
    );
    let Some(bytes) = out.artifact(cadenza_compile_abi::sidecar::KIND_INSTANTIATIONS) else {
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
    let mut malformed = false;
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let mut cols = line.split('\t');
        match cols.next() {
            Some("disp") => {
                let (node, disp) = match (cols.next(), cols.next()) {
                    (Some(n), Some(d)) => (n, d),
                    _ => {
                        report_malformed_query_row("instantiations", line);
                        malformed = true;
                        continue;
                    }
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
                    _ => {
                        report_malformed_query_row("instantiations", line);
                        malformed = true;
                        continue;
                    }
                };
                let pretty = arglist
                    .split(';')
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join(", ");
                println!("{}:   {}[{pretty}] → {spec}", loc_of(node), args.name);
            }
            // A row whose leading tag is neither `disp` nor `inst` is a format skew — fail loudly rather
            // than silently drop it (the silent-skip class; the other query readers already do this).
            _ => {
                report_malformed_query_row("instantiations", line);
                malformed = true;
            }
        }
    }
    if malformed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// `cdz func-layout FILE` — the emitted-function LAYOUT of FILE's whole (linked import-closure) program.
/// Follows the entry's IMPORT CLOSURE (like `cdz check`), so a package entry lays out every reachable def
/// across the linked program — the SAME func-index set + order a real emit (`cdz test`/`cdz build`) uses.
/// Drives `Query::FuncLayout`, which forces monomorphization then lays out the boundary — rooting on the
/// program's `(export …)` clauses, and (since the @test-rooted fallback) falling back to the `@test` defs
/// when there is no export, so a pure-`@test` file (a compiler-ml conformance file) lays out too. Prints
/// the query's rows verbatim: a `defs-begin<TAB><import-base><TAB>-` marker then one
/// `<func-index>\t<hash16>\t<name>` row per reachable def, func-index-ascending. The rows are already
/// machine-readable (TAB-separated); this is a pass-through so a consumer (the compile-reuse witness, a
/// cache-key builder) reads the layout directly. A layout that DECLINES — only when there is NEITHER an
/// export NOR a `@test` to anchor emit — yields the EMPTY output (no marker, no rows); still a total query,
/// rc 0.
fn run_func_layout(args: &FuncLayoutArgs) -> ExitCode {
    // Follow the entry file's IMPORT CLOSURE so the layout spans the whole linked program (a compiler-ml
    // entry pulls its ~1360-def closure), matching what an emit sees. A file importing nothing loads as a
    // lone file, byte-identical to a standalone layout.
    let files = match load_import_closure_with(&args.file, &|_| None) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("{PROG}: {e}");
            return ExitCode::FAILURE;
        }
    };
    // Route through the package/link path whenever the ENTRY declares an `(import …)` — splice all closure
    // files into one program so cross-file references resolve, exactly as the `check` package path does.
    let is_package = !declared_import_paths(&files[0].arenas).is_empty();
    let out = if is_package {
        let mut inputs: Vec<cadenza_compile_abi::Artifact> = files
            .iter()
            .map(|f| {
                cadenza_compile_abi::Artifact::new(
                    cadenza_compile_abi::Artifact::KIND_AST,
                    f.name.clone(),
                    cadenza_syntax::codec::encode(&f.arenas),
                )
            })
            .collect();
        inputs.push(cadenza_compile_abi::Artifact::new(
            cadenza_compile_abi::sidecar::KIND_SIDECAR,
            "drive",
            cadenza_compile_abi::sidecar::encode(&[cadenza_compile_abi::Request::Query(
                cadenza_compile_abi::sidecar::Query::FuncLayout,
            )]),
        ));
        inputs.push(cadenza_compile_abi::abi::entry_artifact(&files[0].name));
        dispatch_query_over_inputs(inputs, cadenza_compile_abi::sidecar::KIND_FUNC_LAYOUT)
    } else {
        run_sidecar(
            &files[0].arenas,
            cadenza_compile_abi::Request::Query(cadenza_compile_abi::sidecar::Query::FuncLayout),
        )
    };
    let Some(bytes) = out.artifact(cadenza_compile_abi::sidecar::KIND_FUNC_LAYOUT) else {
        // No artifact = the AST itself failed to decode/compile at the entry (a total query otherwise
        // always produces the func-layout artifact — the marker + rows, or the EMPTY string when the
        // layout declines with neither an export nor a `@test`).
        report_errors(&out);
        return ExitCode::FAILURE;
    };
    let text = String::from_utf8_lossy(bytes);
    // Validate each row's shape loudly rather than silently passing a format-skewed line through (the
    // silent-skip class other query readers guard against). The marker is `defs-begin<TAB>N<TAB>-`; every
    // other row is `<idx>\t<hash>\t<name>`. A layout decline is the empty string — a valid (rc 0) result.
    let mut malformed = false;
    for (n, line) in text.lines().enumerate() {
        if line.is_empty() {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        let ok = if n == 0 {
            cols.len() == 3 && cols[0] == "defs-begin" && cols[1].parse::<u32>().is_ok()
        } else {
            // idx is a func-index number or `-` (an emitted def with no assigned slot is reported `-`);
            // hash is 16 hex digits; name is non-empty.
            cols.len() == 3
                && (cols[0] == "-" || cols[0].parse::<u32>().is_ok())
                && cols[1].len() == 16
                && cols[1].chars().all(|c| c.is_ascii_hexdigit())
                && !cols[2].is_empty()
        };
        if !ok {
            report_malformed_query_row("func-layout", line);
            malformed = true;
            continue;
        }
        println!("{line}");
    }
    if malformed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// `cdz highlight FILE` — semantic syntax highlighting: every classified token as `file:line:col: kind`.
/// Rides the `Highlight` sidecar query (the same one the browser IDE's `semantic_tokens` calls), then
/// maps each node id to a source location through the span table. A token whose node has no span is
/// skipped (should not happen for a user leaf).
fn run_highlight(args: &HighlightArgs) -> ExitCode {
    let (source, arenas, spans) = load_spanned_or_bail!(&args.file);
    let out = run_sidecar(
        &arenas,
        cadenza_compile_abi::Request::Query(cadenza_compile_abi::sidecar::Query::Highlight),
    );
    let Some(bytes) = out.artifact(cadenza_compile_abi::sidecar::KIND_HIGHLIGHT) else {
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
    // Each line is `node-id<TAB>kind`. Map the node to a `file:line:col`, skipping a span-less node (so
    // every emitted token — human OR `--json` — carries a real position; no raw-id fallback here). Both
    // output shapes are computed from the SAME resolved `(kind, line, col)` so they can't drift.
    let mut malformed = false;
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let mut cols = line.splitn(2, '\t');
        let (node, kind) = match (cols.next(), cols.next()) {
            (Some(n), Some(k)) => (n, k),
            _ => {
                report_malformed_query_row("highlight", line);
                malformed = true;
                continue;
            }
        };
        if let Some(span) = node
            .parse::<u32>()
            .ok()
            .and_then(|d| spans.get(cadenza_syntax::StructId(d)))
        {
            let (l, c) = index.line_col(&source, span.start);
            if args.json {
                use cadenza_syntax::query::json;
                let mut obj = json::Object::new();
                obj.string("file", &args.file);
                obj.raw("line", &l.to_string());
                obj.raw("col", &c.to_string());
                obj.string("kind", kind);
                println!("{}", obj.finish());
            } else {
                println!("{}:{l}:{c}: {kind}", args.file);
            }
        }
    }
    if malformed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

// ── shared plumbing ────────────────────────────────────────────────────────────────────────────────

/// Compile `arenas` under a single sidecar request, on the compiler's stack-guarded worker thread.
fn run_sidecar(
    arenas: &cadenza_syntax::Arenas,
    request: cadenza_compile_abi::Request,
) -> cadenza_compile_abi::CompileOutput {
    run_sidecar_many(arenas, &[request])
}

/// Drive a BATCH of sidecar requests over one program in a single compile. A request list is ordered
/// and the `Db`'s columns are shared/warm across the batch, so N `TypeAt` queries (one per match
/// binding, for `--where`) cost one `Db::load` + shared inference, not N separate compiles.
fn run_sidecar_many(
    arenas: &cadenza_syntax::Arenas,
    requests: &[cadenza_compile_abi::Request],
) -> cadenza_compile_abi::CompileOutput {
    // Under `!standalone` (the nix delegating build), a SINGLE query spawns `cdz-compile` instead of
    // running the compiler in-process — the request is built as a binary-AST tree via cadenza-syntax and
    // the single result artifact is captured off `cdz-compile`'s `-o -` stdout. A batch (`--where`, N
    // requests) or a non-query returns `None` and falls through to the in-process path below (rcdzc is
    // still linked; delegating the batch reader is a later slice, gated on positional result naming).
    #[cfg(not(feature = "standalone"))]
    {
        if let Some(out) = delegate::run_sidecar_delegated(arenas, requests, PROG) {
            return out;
        }
    }
    let ast = cadenza_syntax::codec::encode(arenas);
    let sidecar = cadenza_compile_abi::sidecar::encode(requests);
    let inputs = vec![
        cadenza_compile_abi::Artifact::new(cadenza_compile_abi::Artifact::KIND_AST, "main", ast),
        cadenza_compile_abi::Artifact::new(
            cadenza_compile_abi::sidecar::KIND_SIDECAR,
            "drive",
            sidecar,
        ),
    ];
    // No emit target: a query-only run (`DESIGN-sidecar-api.md` query-only mode). The stack guard keeps
    // pathologically deep input a decline, not a crash.
    rcdzc::run_with_compiler_stack(|| rcdzc::compile(&inputs, &[]))
}

/// Run a SINGLE-RESULT sidecar query over already-prepared `inputs` (a multi-file / package query — the
/// caller built the `ast`[s] + `KIND_SIDECAR` request + `entry`), producing a `CompileOutput` the caller
/// reads via `out.artifact(result_kind)`. Under `!standalone` this delegates to `cdz-compile` (spawn +
/// capture the `-o -` result, tagged `result_kind`); under `standalone` it runs the compiler in-process.
/// The caller knows the request yields exactly one result of `result_kind` (a lone `Query`).
fn dispatch_query_over_inputs(
    inputs: Vec<cadenza_compile_abi::Artifact>,
    result_kind: &str,
) -> cadenza_compile_abi::CompileOutput {
    #[cfg(not(feature = "standalone"))]
    {
        delegate::run_query_over_inputs(&inputs, result_kind, PROG)
    }
    #[cfg(feature = "standalone")]
    {
        let _ = result_kind; // the in-process output already carries every artifact by kind
        rcdzc::run_with_compiler_stack(|| rcdzc::compile(&inputs, &[]))
    }
}

/// Report a sidecar QUERY RESULT ROW that did not parse into its expected shape — the shared loud-failure
/// path for the `cdz` query readers that call it: `uses`, `scope`, `exports`, `symbols`, `highlight`, and
/// `instantiations`. (`param-manifest` has its OWN equivalent error path — the original fix from PR #525 —
/// and does not route through here.) Each reader splits a query's TAB-separated output into a fixed set of
/// fields; a row that does not match is NOT silently dropped (which would mask a sidecar output-format
/// regression behind a success exit + a silently-short result — the class Copilot flagged, PR #525/#530).
/// Instead the reader calls this to name the query + the offending line, then FAILS at the end. `query` is
/// the query name for the message; returns nothing — the caller sets its own `malformed` flag.
fn report_malformed_query_row(query: &str, line: &str) {
    eprintln!(
        "{PROG}: internal: could not parse a `{query}` result row — the sidecar query output format may \
         have changed (expected TAB-separated fields): {line:?}"
    );
}

/// Report a compile output's error diagnostics to stderr (used when a query produced no artifact —
/// which for a TOTAL query means the AST itself failed to decode/compile at the entry).
fn report_errors(out: &cadenza_compile_abi::CompileOutput) {
    for d in &out.diagnostics {
        if d.severity == cadenza_compile_abi::Severity::Error {
            match &d.code {
                Some(code) => eprintln!("{PROG}: error [{code}]: {}", d.message),
                None => eprintln!("{PROG}: error: {}", d.message),
            }
        }
    }
}

/// Report a compile output's error diagnostics to stderr WITH a source location — the located counterpart
/// of [`report_errors`], for a caller that HOLDS the program's files (source + span table). When a
/// diagnostic carries a source anchor (`d.node`), map it to `file:line:col` (demuxing a package node to its
/// file via the `link-map`, like the check path) and print `file:line:col: error [CODE]: message` — the
/// SAME located shape `cdz check` uses. This closes the gap where the emit path (`cdz test`, `cdz build`)
/// declines with a well-anchored diagnostic (e.g. an invalid-kebab `@test`/export name, `node = Some`) yet
/// the reporter dropped the anchor and printed only `cdz: error [CODE]: …`. A diagnostic with no anchor (or
/// an unmappable node) falls back to the bare `cdz: error …` line, so it is never worse than `report_errors`.
fn report_errors_located(out: &cadenza_compile_abi::CompileOutput, files: &[closure::LoadedFile]) {
    // Per-file line index (binary-searched line:col), parallel to `files` — linear even with many faults.
    let indices: Vec<_> = files
        .iter()
        .map(|f| cadenza_syntax::query::driver::LineIndex::new(&f.source))
        .collect();
    // Demux a GLOBAL node id to `(file index, that file's LOCAL id)`. A single file (no link-map) is the
    // identity `(0, id)`; a package finds the file whose `[base, base+count)` range holds the id. `None`
    // for a node in no file (a synthesized/prelude node).
    let link_map = out
        .artifact(cadenza_compile_abi::link_map::KIND_LINK_MAP)
        .map(cadenza_compile_abi::link_map::decode_link_map)
        .unwrap_or_default();
    let file_of_node = |n: u32| -> Option<(usize, u32)> {
        if link_map.is_empty() {
            return files.first().map(|_| (0usize, n));
        }
        let fs = link_map
            .iter()
            .find(|fs| n >= fs.struct_base && n < fs.struct_base + fs.struct_count)?;
        let fi = files.iter().position(|f| f.name == fs.path)?;
        Some((fi, n - fs.struct_base))
    };
    // A node id → its `file:line:col` label, or `None` if unanchored/unmappable (then the bare form prints).
    let loc_label = |node: Option<u32>| -> Option<String> {
        let (fi, local) = file_of_node(node?)?;
        let span = files[fi].spans.get(cadenza_syntax::StructId(local))?;
        let (l, c) = indices[fi].line_col(&files[fi].source, span.start);
        Some(format!("{}:{l}:{c}", files[fi].path))
    };
    for d in &out.diagnostics {
        if d.severity != cadenza_compile_abi::Severity::Error {
            continue;
        }
        let code = match &d.code {
            Some(code) => format!(" [{code}]"),
            None => String::new(),
        };
        match loc_label(d.node) {
            Some(loc) => eprintln!("{loc}: error{code}: {}", d.message),
            None => eprintln!("{PROG}: error{code}: {}", d.message),
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
/// - `def name = "…"`            — the project name. NOT display-only: it becomes the published
///   package interface segment `cadenza:<name>/api` that a DEPENDENT binds against, so it must be a valid
///   lowercase interface segment (a `--check`-style `name_malformed`/dep-name validation enforces this);
///   falls back to the manifest's DIRECTORY name when absent.
/// - `def entry = "main.cdz"`     — the entry module `cdz build`/`run` compiles as the component root.
/// - `def modules = ["a.cdz", …]` — the library modules the package links (the entry's importables).
/// - `def tests = ["*.cdz", …]`   — the modules whose `@test` defs `cdz test` runs.
/// - `def exclude = ["x.cdz", …]` — files REMOVED from `modules`/`tests` after glob expansion (skip a
///   demo/fixture a wildcard would otherwise sweep up).
/// - `def deps = ["../lib", …]`   — PATH dependencies: sibling project dirs `cdz run` builds + peer-binds
///   across the component boundary (each published as `cadenza:<dep>/api`).
/// - `def overflow-signed = "trap"` / `def overflow-unsigned = "trap"` — the project's GLOBAL integer
///   overflow policy for signed/unsigned arithmetic, one of `"trap"` (fault on overflow) or `"wrap"`
///   (two's-complement). Absent → the compiler default `trap`. This is the global default a module
///   `#[overflow(...)]` pragma overrides; the effective policy enters the reproducible build hash (v-nix).
#[derive(Default, Debug)]
struct Manifest {
    name: Option<String>,
    /// Set when the manifest HAS a `def name` but its value is NOT a string (e.g. `def name = 42`) — so
    /// `name` resolves to `None` (no string extracted) yet the field is PRESENT. Unlike `entry` (required
    /// → hard error), `name` has a safe fallback (the manifest's directory name, used for the published
    /// `cadenza:<name>/api` interface segment), so a consumer WARNS (the declared name was silently dropped)
    /// and continues rather than failing. `false` when `name` is absent OR a valid string.
    name_malformed: bool,
    entry: Option<String>,
    /// Set when the manifest HAS a `def entry` but its value is NOT a string (e.g. `def entry = 42` or
    /// `def entry = true`) — so `entry` resolves to `None` (no string extracted) yet the field is PRESENT.
    /// Lets a consumer emit "entry must be a string" instead of the misleading "declares no `entry`" (which
    /// would tell the user to add an entry they already wrote). `false` when `entry` is absent OR a valid
    /// string.
    entry_malformed: bool,
    modules: Vec<String>,
    tests: Vec<String>,
    exclude: Vec<String>,
    /// The project's default optimization level for `cdz build` (`def opt-level = "O2"`), as the raw
    /// string — parsed via `rcdzc::OptLevel::FromStr` at use. A `--opt-level`/`--release` flag overrides
    /// it. `None` = no manifest default (the build falls back to `--release`'s `O2` or the default `O1`).
    opt_level: Option<String>,
    /// Set when the manifest HAS a `def opt-level` but its value is NOT a string (e.g. `def opt-level =
    /// 42`) — so `opt_level` resolves to `None` (no string extracted) yet the field is PRESENT. Unlike
    /// `entry` (required → hard error), `opt-level` has a safe default, so a consumer WARNS (the setting
    /// was silently dropped) and continues rather than failing. `false` when absent OR a valid string.
    opt_level_malformed: bool,
    /// The project's GLOBAL integer-overflow policy for SIGNED arithmetic (`def overflow-signed =
    /// "trap"`), as the raw string — one of `"trap"` (a checked op that faults on overflow) or `"wrap"`
    /// (two's-complement wraparound). `None` = no manifest setting, so the compiler's DEFAULT applies
    /// (`trap`, the 2026-08-29 ruling). This is the GLOBAL default in the per-node overflow resolution a
    /// module `#[overflow(...)]` pragma OVERRIDES (precedence: module pragma > this global > default trap).
    /// The effective policy MUST enter the reproducible build hash (a program's meaning is fixed by
    /// source+manifest, never an ambient flag) — that hash folding is v-nix's lane; this field is the
    /// source of truth it reads.
    overflow_signed: Option<String>,
    /// Set when the manifest HAS a `def overflow-signed` but its value is NOT a valid policy string — the
    /// wrong TYPE (e.g. `def overflow-signed = 42`) OR a string outside `{trap, wrap}` (e.g. `"saturate"`,
    /// not yet supported). `overflow_signed` resolves to `None` (the default `trap` applies) yet the field
    /// is PRESENT. Like `opt-level`, this has a safe default, so a consumer WARNS (the declared policy was
    /// ignored) and continues rather than failing. `false` when absent OR a valid `"trap"`/`"wrap"`.
    overflow_signed_malformed: bool,
    /// The project's GLOBAL integer-overflow policy for UNSIGNED arithmetic (`def overflow-unsigned =
    /// "wrap"`), the unsigned twin of [`Manifest::overflow_signed`] — `"trap"`/`"wrap"`, `None` → default
    /// `trap`. Signed + unsigned are configured SEPARATELY (a project may want checked signed but wrapping
    /// unsigned, or vice versa). Same precedence + build-hash contract as the signed field.
    overflow_unsigned: Option<String>,
    /// Set when the manifest HAS a `def overflow-unsigned` but its value is not a valid `{trap, wrap}`
    /// string — the unsigned twin of [`Manifest::overflow_signed_malformed`]. `false` when absent OR valid.
    overflow_unsigned_malformed: bool,
    /// Known manifest keys declared MORE THAN ONCE (e.g. two `def entry` lines). The parser is last-wins
    /// (each arm overwrites), so a duplicate silently discards the earlier value — a `def entry = "a.cdz"`
    /// followed by `def entry = "b.cdz"` builds `b.cdz` with no hint the first was dropped. A consumer WARNS
    /// (naming each duplicated key + that the last wins) so the author isn't surprised. Empty = no dup.
    duplicate_keys: Vec<String>,
    /// DEPENDENCIES (`def deps = ["../mathlib", …]`) — the projects this project links across the component
    /// boundary. Each is a [`DepSource`]; today the only source is a PATH (a sibling project dir), but the
    /// type is an enum so a REGISTRY source (npm first — operator direction) can slot in LATER without a
    /// rewrite. `cdz run` builds each dep's `Project.cdz` entry to its own component and PEER-BINDS its
    /// exported interface into this project's consumer (the cross-component interop v-peer-linking owns — a
    /// runtime compose via `run_with_peers`, NOT a compile-time merge). Empty = a standalone project.
    deps: Vec<DepSource>,
}

/// WHERE a dependency comes from. A dep SOURCE abstraction (operator direction): path is the first — and
/// today only — implementation, but making it an enum leaves room for a REGISTRY-backed source (npm the
/// first registry) to be added as a variant later WITHOUT reworking the manifest model or the resolve
/// flow. Not yet built: only `Path` resolves; a future `Registry` variant would carry `{ name, version
/// req, registry }` and `build_path_deps` would grow an arm that fetches + builds it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum DepSource {
    /// A PATH dependency: a sibling project directory (relative to the consumer's manifest dir). Its
    /// `Project.cdz` entry is built to a component published as `cadenza:<dep-name>/api` (derived from the
    /// dep's `name`/dir), which the consumer's source binds with `(bind E "cadenza:<dep-name>/api")`.
    Path(String),
    // Registry { name: String, version: String, registry: Option<String> },  // LATER (npm first)
}

impl DepSource {
    /// Parse a manifest dep entry into a `DepSource`. Today a bare string is a PATH (the only form) — this
    /// is where a structured form (e.g. `(dep (registry "pkg" "^1"))`) would branch to `Registry` later.
    fn from_manifest_string(s: String) -> DepSource {
        DepSource::Path(s)
    }

    /// The raw manifest text of this dep — for `cdz metadata`'s `deps` array + diagnostics. A path is its
    /// path string; a future registry source would render its package ref.
    fn as_manifest_text(&self) -> &str {
        match self {
            DepSource::Path(p) => p,
        }
    }
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
    // A list literal is the `List` compound. Read it with `compound_form_of` (the transitional DUAL-READ):
    // it accepts the M2 native ctor-LEAF-KIND head (`Leaf::Ctor(List)`, what the reader now emits after the
    // native-compound flag-day #5112) AND the legacy STRING-primitive head (`("list" …)`). The old
    // `as_ctor_form(value_id, "list")` matched ONLY the string head, so after #5112 a manifest's
    // `def tests = ["src/*.cdz"]` parsed to a native `List` compound that this returned EMPTY for → every
    // project reported "declares no `tests`" (a manifest-tests resolution regression, distinct from the run
    // path which resolves files via this same reader — both were broken; the eval-outage masked it).
    if let Some(elems) = arenas.compound_form_of(value_id, cadenza_syntax::ast::CompoundCtor::List)
    {
        return elems
            .iter()
            .filter_map(|&e| arenas.as_str(e))
            .map(str::to_string)
            .collect();
    }
    Vec::new()
}

/// The values a `def overflow-signed`/`def overflow-unsigned` manifest field accepts — the closed policy
/// alphabet. `trap` = a checked op that faults on overflow; `wrap` = two's-complement wraparound.
/// (`saturate` is a plausible future member, deliberately NOT accepted yet — an unknown value is rejected,
/// not silently treated as the default.)
const OVERFLOW_POLICY_VALUES: [&str; 2] = ["trap", "wrap"];

/// Parse a `def overflow-signed`/`overflow-unsigned` value into `(policy, malformed)`. A valid `"trap"`/
/// `"wrap"` string → `(Some(policy), false)`. A wrong TYPE (non-string) OR a string outside the closed
/// `{trap, wrap}` alphabet → `(None, true)` — the field is present but unusable, so the default `trap`
/// applies and the consumer WARNS (mirrors `opt-level`'s safe-default-with-warning handling). This keeps
/// the effective policy well-defined: a project never silently compiles under a mis-typed policy string.
fn resolve_overflow_field(
    arenas: &cadenza_syntax::Arenas,
    value_id: cadenza_syntax::StructId,
) -> (Option<String>, bool) {
    match manifest_strings(arenas, value_id).into_iter().next() {
        Some(s) if OVERFLOW_POLICY_VALUES.contains(&s.as_str()) => (Some(s), false),
        // A string outside {trap, wrap}, OR a non-string value → malformed (present but ignored).
        _ => (None, true),
    }
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
    // Known keys already seen — to flag a DUPLICATE `def` (last-wins would otherwise silently drop the
    // earlier value). Only KNOWN keys are tracked (an unrecognized/forward-compat def isn't a duplicate
    // worth warning about); each key is recorded into `m.duplicate_keys` at most once.
    let mut seen: Vec<&str> = Vec::new();
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
        // A KNOWN key seen before is a duplicate (last-wins); record it once so a consumer can warn.
        if matches!(
            name,
            "name"
                | "entry"
                | "modules"
                | "tests"
                | "exclude"
                | "opt-level"
                | "overflow-signed"
                | "overflow-unsigned"
                | "deps"
        ) {
            if seen.contains(&name) {
                if !m.duplicate_keys.iter().any(|k| k == name) {
                    m.duplicate_keys.push(name.to_string());
                }
            } else {
                seen.push(name);
            }
        }
        match name {
            "name" => {
                m.name = manifest_strings(arenas, value_id).into_iter().next();
                // `def name` present but no string extracted → wrong TYPE (a number/bool/other). Record it
                // so a consumer can WARN the declared name was ignored (it falls back to the directory name
                // for the published `cadenza:<name>/api` interface) rather than silently dropping it.
                m.name_malformed = m.name.is_none();
            }
            "entry" => {
                m.entry = manifest_strings(arenas, value_id).into_iter().next();
                // `def entry` is present; if no string came out of it, the value is the wrong TYPE (a
                // number/bool/other), not a `"file.cdz"` — record that so the "no entry" path can instead
                // say "entry must be a string" rather than sending the user to add an entry they wrote.
                m.entry_malformed = m.entry.is_none();
            }
            "modules" => m.modules = manifest_strings(arenas, value_id),
            "tests" => m.tests = manifest_strings(arenas, value_id),
            "exclude" => m.exclude = manifest_strings(arenas, value_id),
            "opt-level" => {
                m.opt_level = manifest_strings(arenas, value_id).into_iter().next();
                // `def opt-level` present but no string extracted → wrong TYPE (not `"O2"`). Record it so a
                // consumer can WARN the setting was ignored rather than silently building at the default.
                m.opt_level_malformed = m.opt_level.is_none();
            }
            "overflow-signed" => {
                // Accept only a valid policy string `"trap"`/`"wrap"`; a wrong TYPE (non-string) OR an
                // unknown value (e.g. `"saturate"`, not yet supported) resolves to None + malformed, so the
                // default `trap` applies and a consumer warns rather than silently honoring a bad setting.
                (m.overflow_signed, m.overflow_signed_malformed) =
                    resolve_overflow_field(arenas, value_id);
            }
            "overflow-unsigned" => {
                (m.overflow_unsigned, m.overflow_unsigned_malformed) =
                    resolve_overflow_field(arenas, value_id);
            }
            "deps" => {
                m.deps = manifest_strings(arenas, value_id)
                    .into_iter()
                    .map(DepSource::from_manifest_string)
                    .collect()
            }
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
    // A DIRECTORY passed where a program FILE is expected: `read_to_string` would surface a raw
    // `Is a directory (os error 21)` — an errno leak, not a diagnostic. The by-name/by-offset query
    // commands (`type`/`doc`/`uses`/`def`/`scope`/`type-at`/`…`) all funnel through here, so pre-check
    // once and give a clean, actionable message naming a concrete file to pass — consistent with the
    // directory guidance `cdz check`/`cdz compile` already give (they walk a dir; a single-file query
    // command takes ONE file, so it points the user at naming one).
    if std::path::Path::new(file).is_dir() {
        return Err(format!(
            "{file} is a directory — this command takes a single program file; name one \
             (e.g. `{file}/main.cdz`)"
        ));
    }
    // A bare `-` (the stdin marker) reaches here only for a command that does NOT read stdin — the query
    // commands (`type`/`doc`/`check`/`…`) take a named FILE, whose extension picks the surface. Without a
    // guard, `read_to_string("-")` leaks `reading -: No such file or directory (os error 2)` (it looks for
    // a file literally named `-`). Give a clean message pointing at the commands that DO consume stdin
    // (`cdz fmt -`/`cdz convert -`/`cdz compile -`/`cdz run -`, which take an explicit `--from`/surface, and
    // the verdict runners `cdz run-ml -`/`cdz run-rust -`/`cdz run-emitted -`).
    if file == "-" {
        return Err(
            "reading a program from stdin (`-`) is not supported by this command; pass a FILE \
             (its extension picks the surface). The commands that read stdin are `cdz fmt -`, \
             `cdz convert -`, the `cdz compile -`/`cdz run -` pipe, and the verdict runners \
             `cdz run-ml -`/`cdz run-rust -`/`cdz run-emitted -`"
                .to_string(),
        );
    }
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

    let (source, arenas, spans) = load_spanned_or_bail!(&file);

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
    let requests: Vec<cadenza_compile_abi::Request> = typed_nodes
        .iter()
        .map(|&n| {
            cadenza_compile_abi::Request::Query(cadenza_compile_abi::sidecar::Query::TypeAt {
                node: n,
            })
        })
        .collect();
    let out = run_sidecar_many(&arenas, &requests);
    // node id → rendered type. The `type-at` result artifacts come back in REQUEST ORDER (the compiler
    // materializes one result per request, in order — `compile.rs`), so pair them POSITIONALLY with
    // `typed_nodes` rather than parsing a per-artifact node-id NAME. This keeps the reader agnostic to the
    // result-artifact NAMING, so the delegated batch path (which names results positionally, not by the
    // queried node) reads the same — and a naming change on the compiler side is transparent here.
    let type_at: Vec<&cadenza_compile_abi::Artifact> = out
        .artifacts
        .iter()
        .filter(|a| a.kind == cadenza_compile_abi::sidecar::KIND_TYPE_AT)
        .collect();
    let mut node_ty: std::collections::HashMap<u32, String> = std::collections::HashMap::new();
    for (&n, art) in typed_nodes.iter().zip(&type_at) {
        node_ty.insert(n, String::from_utf8_lossy(&art.bytes).into_owned());
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
    fn resolve_deps_dir_does_not_double_append_when_bin_is_in_deps() {
        // PR#772 regression: run-rust must find cargo's hashed rlibs (libcdz_num-<hash>.rlib) whether the
        // `cdz` bin sits in `target/<profile>/` (deps = <that>/deps) OR in `target/<profile>/deps/` (a
        // `cargo test`-located bin — deps IS the bin's own dir). A blind `lib_dir.join("deps")` in the
        // latter case gives `.../deps/deps` (nonexistent) → the hashed rlib isn't found → E0433.
        use std::path::Path;
        // Bin in target/<profile>/ → deps dir is a `deps` CHILD.
        assert_eq!(
            resolve_deps_dir(Path::new("/w/target/debug")),
            Path::new("/w/target/debug/deps"),
            "a non-deps lib_dir gets a `deps` child appended"
        );
        // Bin ALREADY in .../deps/ → deps dir is lib_dir ITSELF (NOT lib_dir/deps).
        assert_eq!(
            resolve_deps_dir(Path::new("/w/target/debug/deps")),
            Path::new("/w/target/debug/deps"),
            "a lib_dir already named `deps` is used as-is, NOT double-appended to deps/deps"
        );
        // Nested/edge: a path ending in `deps` at any depth is treated as the deps dir.
        assert_eq!(
            resolve_deps_dir(Path::new("/a/b/c/deps")),
            Path::new("/a/b/c/deps"),
            "any dir named `deps` is the deps dir"
        );
    }

    #[test]
    fn rust_rlib_search_roots_prefers_the_env_override_then_falls_back_to_exe_relative() {
        use std::path::{Path, PathBuf};
        // No CDZ_RUST_RLIB_DIR → search ONLY the exe-relative dir (a `cargo build` workspace has the
        // rlibs beside the `cdz` bin — the historical behavior, unchanged).
        assert_eq!(
            rust_rlib_search_roots(Path::new("/w/target/release"), None),
            vec![PathBuf::from("/w/target/release")],
            "no override → just the exe-relative root"
        );
        // Override set (the nix `cdz` package, whose bin/ has no rlibs) → the override is searched FIRST,
        // with the exe-relative dir kept as the fallback (so a cargo build with the env set still works).
        assert_eq!(
            rust_rlib_search_roots(
                Path::new("/nix/store/x-cdz/bin"),
                Some(PathBuf::from("/nix/store/y-cdz-rlibs/lib"))
            ),
            vec![
                PathBuf::from("/nix/store/y-cdz-rlibs/lib"),
                PathBuf::from("/nix/store/x-cdz/bin"),
            ],
            "override is searched first, exe-relative is the fallback"
        );
    }

    #[test]
    fn exit_code_from_child_does_not_wrap_an_out_of_range_code() {
        // The PR#747 review bug: `code as u8` truncates, so a child exit 256 would become 0 (SUCCESS) and a
        // 257 would become 1 — silently misreporting a failing child. `exit_code_from_child` must map any
        // out-of-`u8`-range code (and a signal-killed `None`) to a FAILURE, never wrap it into a false code.
        // We can't portably construct an `ExitCode` to compare, so assert via the underlying conversion the
        // fn is built on: `u8::try_from` REJECTS out-of-range (unlike `as u8`), which is the whole fix.
        assert_eq!(
            u8::try_from(256_i32).ok(),
            None,
            "256 is out of u8 range (would be 0 under `as u8`)"
        );
        assert_eq!(
            u8::try_from(257_i32).ok(),
            None,
            "257 too (would be 1 under `as u8`)"
        );
        assert_eq!(
            u8::try_from(-1_i32).ok(),
            None,
            "a negative code is out of range"
        );
        // In-range codes forward exactly (no truncation for the common 1..=255 case).
        assert_eq!(u8::try_from(1_i32).ok(), Some(1u8));
        assert_eq!(u8::try_from(42_i32).ok(), Some(42u8));
        assert_eq!(u8::try_from(255_i32).ok(), Some(255u8));
        // And the fn itself: a signal-killed child (None) → FAILURE, not a panic.
        let _ = exit_code_from_child(None);
        let _ = exit_code_from_child(Some(256));
        let _ = exit_code_from_child(Some(42));
    }

    #[test]
    fn chor_unwrap_string_value_strips_boundary_and_unescapes() {
        // `cdz run` renders a String return as `(: "<escaped>" String)`; unwrap recovers the raw text.
        assert_eq!(unwrap_string_value("(: \"hello\" String)"), "hello");
        assert_eq!(
            unwrap_string_value("(: \"a\\nb\\tc\\\"d\" String)"),
            "a\nb\tc\"d"
        );
        // Not the wrapper → returned as-is (defensive).
        assert_eq!(unwrap_string_value("bare"), "bare");
    }

    #[test]
    fn chor_split_actor_bundle_splits_on_role_markers() {
        // The `render-all` bundle: `==== <Role> ====` header then that actor's module, per actor.
        let bundle =
            "==== Buyer ====\neffect Comm =\ndef main() = 0\n\n==== Seller ====\ndef main() = 1\n";
        let actors = split_actor_bundle(bundle);
        assert_eq!(actors.len(), 2);
        assert_eq!(actors[0].0, "Buyer");
        assert!(actors[0].1.contains("effect Comm ="));
        assert!(actors[0].1.contains("def main() = 0"));
        assert_eq!(actors[1].0, "Seller");
        assert!(actors[1].1.contains("def main() = 1"));
        // A bundle with no markers yields no actors (the run_chor empty-guard fires).
        assert!(split_actor_bundle("no markers here").is_empty());
    }

    #[test]
    fn chor_module_rec_degradation_is_detected() {
        // A recursive protocol's emitted actor carries the `-- rec: unsupported` / `-- var: unsupported`
        // marker (render-compilable can't emit a def-main back-jump yet); `cdz chor` must detect it to warn.
        let rec_stub = "effect Comm = ...\ndef main() =\n  unit  -- rec: unsupported in first-cut render-compilable\nexport { main }\n";
        assert!(
            chor_module_is_rec_degraded(rec_stub),
            "a rec-unsupported stub is degraded"
        );
        let var_stub = "def main() =\n  unit  -- var: unsupported in first-cut render-compilable\n";
        assert!(
            chor_module_is_rec_degraded(var_stub),
            "a var-unsupported stub is degraded"
        );
        // A normal linear actor (no marker) is NOT degraded — no spurious warning.
        let ok = "effect Comm = ...\ndef main() =\n  let _ = Comm.send(\"Title\") in\n  unit\nexport { main }\n";
        assert!(
            !chor_module_is_rec_degraded(ok),
            "a non-recursive actor must not be flagged degraded"
        );
    }

    #[test]
    fn chor_no_actors_reason_is_tailored_to_the_verdict() {
        // Each driver verdict maps to an actionable message naming the cause + fix (not a catch-all guess).
        // not-a-protocol (only the .sexp path): points at parens + valid heads, NOT exports.
        let r = chor_no_actors_reason("not-a-protocol", true);
        assert!(r.contains("not a readable protocol s-expr") && r.contains("balanced parens"));
        assert!(
            !r.contains("export"),
            "the .sexp not-a-protocol case must not mention exports"
        );
        // not-projectable: names the role AND BOTH valid fixes — the selection-message notification and the
        // rule-(b) escape hatch (make the role's behaviour identical in all branches), matching the package's
        // own `chor-diag` diagnostic so the shipping CLI message is at parity.
        let r = chor_no_actors_reason("not-projectable: Shipper", true);
        assert!(r.contains("Shipper") && r.contains("selection message"));
        assert!(
            r.contains("identical in all branches"),
            "the not-projectable message must also offer the rule-(b) fix (identical behaviour in all branches)"
        );
        // not-well-formed: lists the concrete wf failure modes.
        let r = chor_no_actors_reason("not-well-formed", false);
        assert!(r.contains("not well-formed") && r.contains("self comm"));
        // .cdz path with an unrecognized (non-verdict) bundle: guide the user to export protocol + roles.
        let r = chor_no_actors_reason("", false);
        assert!(r.contains("export both `protocol` and `roles`"));
        // .sexp path with an unrecognized bundle: generic (no misleading exports mention).
        let r = chor_no_actors_reason("something-unexpected", true);
        assert!(r.contains("no actors emitted") && !r.contains("export"));
    }

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
                kids.push(Tree::Atom(Leaf::Name("a".into()), Some(StructId(id))));
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
            kids.push(Tree::Atom(Leaf::Name("y".into()), Some(target)));
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
        let leaf = |id: u32, n: &str| Tree::Atom(Leaf::Name(n.into()), Some(StructId(id)));
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
                Tree::Atom(Leaf::Name("leaf".into()), Some(StructId(id)))
            } else {
                Tree::List(vec![deep(depth - 1, next_id)], Some(StructId(id)))
            }
        }
        let build = |depth: usize| -> (Tree, StructId) {
            let mut next = 100u32;
            let child = deep(depth, &mut next);
            let target = StructId(1);
            let tree = Tree::List(
                vec![child, Tree::Atom(Leaf::Name("target".into()), Some(target))],
                Some(StructId(0)),
            );
            (tree, target)
        };
        fn clones_for(tree: &Tree, target: StructId) -> u64 {
            TRANSFORM_SIBLING_CLONES.with(|c| c.set(0));
            let mut f =
                |_n: &Tree| -> Option<Tree> { Some(Tree::Atom(Leaf::Name("_t".into()), None)) };
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

    #[test]
    fn debug_module_path_covers_the_documented_edge_specs() {
        // The doc-comment promises three edges the happy-path test above skips; pin them so DWARF
        // determinism can't silently regress on an unusual spec (DESIGN-debug-info-rcdzc.md §4).
        // (1) EMPTY spec (only when a spec is itself empty) — no leading `./` to strip, so verbatim (empty).
        assert_eq!(debug_module_path(""), "");
        // (2) A trailing-slash dir-like path is relative → kept verbatim (not an absolute-path reduction).
        assert_eq!(debug_module_path("src/"), "src/");
        // (3) On this POSIX host a backslash is NOT a path separator, so a Windows-style relative path is
        //     kept verbatim (it degrades to file-name only on a Windows host) — still deterministic here.
        assert_eq!(debug_module_path("a\\b.cdz"), "a\\b.cdz");
        // A bare `./` strips to empty (the prefix is the whole spec).
        assert_eq!(debug_module_path("./"), "");
    }

    #[test]
    fn program_name_falls_back_to_main_when_a_spec_has_no_file_stem() {
        // `program_name` is the artifact/query name a spec defaults to; a normal path yields its stem,
        // but a stem-less spec (root, `..`, or empty) falls back to the literal "main" — pin the fallback
        // so a query artifact never ends up unnamed.
        assert_eq!(program_name("src/app.cdz"), "app");
        assert_eq!(program_name("app.sexp"), "app");
        assert_eq!(program_name("/"), "main");
        assert_eq!(program_name(".."), "main");
        assert_eq!(program_name(""), "main");
    }

    #[test]
    fn resolve_opt_level_precedence_follows_flag_then_manifest_then_release_then_default() {
        use cadenza_compile_abi::OptLevel;
        let mp = std::path::Path::new("Project.cdz");
        // The FLAG wins over everything (manifest + release both present, flag still decides).
        assert_eq!(
            resolve_opt_level_precedence(Some("O3"), true, Some("O0"), mp).unwrap(),
            OptLevel::O3,
            "an explicit --opt-level wins over the manifest AND --release"
        );
        // No flag → the MANIFEST wins over --release.
        assert_eq!(
            resolve_opt_level_precedence(None, true, Some("O0"), mp).unwrap(),
            OptLevel::O0,
            "the manifest opt-level wins over --release"
        );
        // No flag, no manifest, --release → O2.
        assert_eq!(
            resolve_opt_level_precedence(None, true, None, mp).unwrap(),
            OptLevel::O2,
            "--release with nothing else is O2"
        );
        // Nothing at all → the default (O1).
        assert_eq!(
            resolve_opt_level_precedence(None, false, None, mp).unwrap(),
            OptLevel::default(),
            "no flag, no manifest, no --release → the default tier"
        );
        assert_eq!(
            OptLevel::default(),
            OptLevel::O1,
            "the documented default is O1"
        );
    }

    #[test]
    fn resolve_opt_level_precedence_errors_name_the_source_of_a_bad_level() {
        let mp = std::path::Path::new("proj/Project.cdz");
        // A malformed FLAG level errors, and the message names `--opt-level` so a typo is a clear failure.
        let flag_err = resolve_opt_level_precedence(Some("Oops"), false, None, mp).unwrap_err();
        assert!(
            flag_err.contains("--opt-level") && flag_err.contains("Oops"),
            "a bad flag level names the flag + the value: {flag_err}"
        );
        // A malformed MANIFEST level errors naming the manifest PATH (not the flag), so the fault is
        // attributed to the right source.
        let man_err = resolve_opt_level_precedence(None, false, Some("O9"), mp).unwrap_err();
        assert!(
            man_err.contains("proj/Project.cdz") && man_err.contains("O9"),
            "a bad manifest level names the manifest path + the value: {man_err}"
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
    fn is_source_file_accepts_the_four_source_surfaces_and_rejects_others() {
        // The recognized source surfaces — same set as the import resolver's precedence. Each accepted.
        for ext in [".cdz", ".ml", ".sexp", ".sexpr"] {
            assert!(
                is_source_file(&format!("app{ext}")),
                "a bare path ending {ext} is source"
            );
            assert!(
                is_source_file(&format!("lib/nested{ext}")),
                "a nested path ending {ext} is source"
            );
        }
        // A non-source extension, or none, is not source.
        assert!(!is_source_file("README.md"), "md is not source");
        assert!(!is_source_file("notes.txt"), "txt is not source");
        assert!(!is_source_file("app"), "no extension is not source");
        // A .sexp-lookalike that does not END with the extension is not source.
        assert!(
            !is_source_file("app.sexp.bak"),
            "a .bak masquerading is not source"
        );
    }

    #[test]
    fn is_source_file_rejects_artifact_specs_even_with_a_source_extension() {
        // The `kind:`/`name=` discrimination: an explicit artifact spec is passed through raw, NOT
        // auto-parsed — even when its path has a source extension. So a `:` or `=` anywhere disqualifies it
        // (the guard that keeps `cdz compile prog.cdz sidecar:d=drive.bin` from parsing the drive spec).
        assert!(
            !is_source_file("ast:m=app.cdz"),
            "a kind:name=path artifact spec is not auto-parsed source"
        );
        assert!(
            !is_source_file("m=app.sexp"),
            "a name=path spec is not auto-parsed source"
        );
        assert!(
            !is_source_file("spans:x.sexp"),
            "a kind:path spec is not auto-parsed source"
        );
    }

    #[test]
    fn run_target_is_project_recognizes_bare_dir_and_manifest_not_wasm_or_source() {
        use std::path::Path;
        // A bare `cdz run` (no arg) → the current-directory project (the `cargo run` analogue).
        assert!(run_target_is_project(None), "bare `cdz run` is a project");

        // A real DIRECTORY → a project (build+run its `Project.cdz`) — even when the directory's
        // name ends in a source extension. `run_target_is_project` is checked BEFORE the
        // source-file arm (main.rs Cmd::Run arms), so a directory always wins the project route.
        let dir = tmp("runtarget");
        assert!(
            run_target_is_project(Some(&dir)),
            "a directory is a project target"
        );
        let dir_named_like_source = dir.join("weird.cdz");
        std::fs::create_dir_all(&dir_named_like_source).unwrap();
        assert!(
            run_target_is_project(Some(&dir_named_like_source)),
            "a DIRECTORY named `weird.cdz` is still a project, not a loose source"
        );

        // A path whose file name IS the manifest (`Project.cdz`) → the project itself.
        let manifest = dir.join(MANIFEST_NAME);
        std::fs::write(&manifest, "").unwrap();
        assert!(
            run_target_is_project(Some(&manifest)),
            "a `Project.cdz` path is a project target"
        );

        // A pre-built `.wasm` component, a loose (non-dir, non-manifest) source file, and `-`
        // (stdin) are NOT projects → they take the direct run path, not the project build.
        assert!(
            !run_target_is_project(Some(Path::new("app.wasm"))),
            "a `.wasm` component is not a project"
        );
        assert!(
            !run_target_is_project(Some(Path::new("loose.cdz"))),
            "a loose source file is not a project"
        );
        assert!(
            !run_target_is_project(Some(Path::new("-"))),
            "`-` (stdin) is not a project"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn run_arg_is_source_file_recognizes_source_specs_but_not_wasm_dir_or_stdin() {
        use std::path::Path;
        // The four source surfaces route to the "you passed SOURCE to `cdz run`" arm (the common
        // `cdz run foo.sexp` mistake path, which builds+runs the loose file).
        for ext in [".cdz", ".ml", ".sexp", ".sexpr"] {
            assert!(
                run_arg_is_source_file(Some(Path::new(&format!("prog{ext}")))),
                "a loose {ext} path is a source-file run arg"
            );
        }
        // A compiled component, an extensionless path, `-` (stdin), and the absent arg are NOT
        // source-file specs → they fall through to the direct-component run path.
        assert!(
            !run_arg_is_source_file(Some(Path::new("app.wasm"))),
            "a `.wasm` component is not a source file"
        );
        assert!(
            !run_arg_is_source_file(Some(Path::new("app"))),
            "an extensionless path is not a source file"
        );
        assert!(
            !run_arg_is_source_file(Some(Path::new("-"))),
            "`-` (stdin) is not a source-file spec"
        );
        assert!(
            !run_arg_is_source_file(None),
            "a bare `cdz run` has no source-file arg"
        );
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
            cadenza_compile_abi::Request::Query(cadenza_compile_abi::sidecar::Query::Diagnostics),
        );
        let bytes = out
            .artifact(cadenza_compile_abi::sidecar::KIND_DIAGNOSTICS)
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

    /// A manifest's `def tests = ["…"]` (and `modules`/`exclude`) parses through the READER, so the list
    /// literal is whatever native shape the reader emits — after the native-compound flag-day (#5112) that
    /// is a `List` compound with a native ctor-LEAF head (`Leaf::Ctor(List)`), NOT a `("list" …)` STRING
    /// head. `manifest_strings` once matched only the string head (`as_ctor_form(_, "list")`), so a real
    /// `Project.cdz` parsed to an EMPTY `tests` list → every project reported `declares no `tests`` (both
    /// the run path and `cdz test --list`; the eval-outage masked it). This pins the fix: a manifest read
    /// through the actual reader must yield the declared entries. Guards the `compound_form_of` dual-read.
    #[test]
    fn parse_manifest_reads_a_native_list_tests_field() {
        let dir = tmp("manifest-native-list");
        let file = dir.join("Project.cdz");
        std::fs::write(
            &file,
            "def name = \"demo\"\ndef modules = [\"src/*.cdz\"]\ndef tests = [\"src/a.cdz\", \"src/b.cdz\"]\ndef exclude = []\n",
        )
        .unwrap();
        let (_source, arenas, _spans) =
            load_program_spanned(&file.to_string_lossy()).expect("manifest loads");
        let m = parse_manifest(&arenas);
        assert_eq!(m.name.as_deref(), Some("demo"), "def name reads");
        assert_eq!(
            m.modules,
            vec!["src/*.cdz"],
            "def modules reads the native list"
        );
        assert_eq!(
            m.tests,
            vec!["src/a.cdz", "src/b.cdz"],
            "def tests reads the native `List`-compound literal (regression: `declares no tests`)"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The GLOBAL overflow-policy manifest fields (`def overflow-signed`/`overflow-unsigned`, #5290):
    /// a valid `"trap"`/`"wrap"` reads through; a value outside `{trap, wrap}` (or a wrong type) resolves
    /// to `None` + `malformed` (the default `trap` applies, a warning fires); an ABSENT field is `None`
    /// but NOT malformed. Signed + unsigned are independent. Guards the closed-alphabet resolution the
    /// numeric-model spec + v-inference's per-node resolution + v-nix's build-hash all read.
    #[test]
    fn parse_manifest_reads_overflow_policy_fields() {
        // Valid + independent: signed wrap, unsigned trap.
        let dir = tmp("manifest-overflow-valid");
        let file = dir.join("Project.cdz");
        std::fs::write(
            &file,
            "def name = \"demo\"\ndef entry = \"main.cdz\"\ndef overflow-signed = \"wrap\"\ndef overflow-unsigned = \"trap\"\n",
        )
        .unwrap();
        let (_s, arenas, _sp) = load_program_spanned(&file.to_string_lossy()).expect("loads");
        let m = parse_manifest(&arenas);
        assert_eq!(m.overflow_signed.as_deref(), Some("wrap"), "signed reads");
        assert!(!m.overflow_signed_malformed);
        assert_eq!(
            m.overflow_unsigned.as_deref(),
            Some("trap"),
            "unsigned reads"
        );
        assert!(!m.overflow_unsigned_malformed);
        std::fs::remove_dir_all(&dir).ok();

        // Unknown value (`saturate` not yet supported) → None + malformed on signed; a valid unsigned
        // alongside is unaffected.
        let dir = tmp("manifest-overflow-unknown");
        let file = dir.join("Project.cdz");
        std::fs::write(
            &file,
            "def entry = \"main.cdz\"\ndef overflow-signed = \"saturate\"\ndef overflow-unsigned = \"wrap\"\n",
        )
        .unwrap();
        let (_s, arenas, _sp) = load_program_spanned(&file.to_string_lossy()).expect("loads");
        let m = parse_manifest(&arenas);
        assert_eq!(m.overflow_signed, None, "unknown value drops to None");
        assert!(m.overflow_signed_malformed, "unknown value is malformed");
        assert_eq!(m.overflow_unsigned.as_deref(), Some("wrap"));
        assert!(!m.overflow_unsigned_malformed);
        std::fs::remove_dir_all(&dir).ok();

        // Absent → None but NOT malformed (the default `trap` applies silently, no warning).
        let dir = tmp("manifest-overflow-absent");
        let file = dir.join("Project.cdz");
        std::fs::write(&file, "def entry = \"main.cdz\"\n").unwrap();
        let (_s, arenas, _sp) = load_program_spanned(&file.to_string_lossy()).expect("loads");
        let m = parse_manifest(&arenas);
        assert_eq!(m.overflow_signed, None);
        assert!(!m.overflow_signed_malformed, "absent is not malformed");
        assert_eq!(m.overflow_unsigned, None);
        assert!(!m.overflow_unsigned_malformed);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// `manifest_overflow_spec` projects the parsed manifest overflow fields to the `cadenza_compile_abi::OverflowSpec`
    /// the compile threads into `db.global_overflow` (O2): a valid `"trap"`/`"wrap"` → the matching mode,
    /// ABSENT or MALFORMED → `None` (falls through to the built-in `Trap` — a malformed value was warned +
    /// uses the default, so `None` matches). Signed + unsigned independent. This is the manifest→compiler
    /// half of the overflow global (twin 3); the rcdzc mechanism (`--overflow-*` → `db.global_overflow`) is O1.
    #[test]
    fn manifest_overflow_spec_maps_valid_absent_and_malformed() {
        use rcdzc::db::{OverflowMode, OverflowSpec};
        let spec = |signed: Option<&str>, s_mal: bool, unsigned: Option<&str>, u_mal: bool| {
            let m = Manifest {
                overflow_signed: signed.map(str::to_string),
                overflow_signed_malformed: s_mal,
                overflow_unsigned: unsigned.map(str::to_string),
                overflow_unsigned_malformed: u_mal,
                ..Manifest::default()
            };
            manifest_overflow_spec(&m)
        };
        // Valid + independent: signed wrap, unsigned trap.
        assert_eq!(
            spec(Some("wrap"), false, Some("trap"), false),
            OverflowSpec {
                signed: Some(OverflowMode::Wrap),
                unsigned: Some(OverflowMode::Trap),
            }
        );
        // Absent → None/None (default: fall through to the built-in Trap).
        assert_eq!(spec(None, false, None, false), OverflowSpec::default());
        // Malformed → None (it was warned at parse + uses default trap), even if a raw string lingers.
        assert_eq!(
            spec(Some("saturate"), true, None, false),
            OverflowSpec::default()
        );
    }

    /// `cdz test --list` emits the cadenza-ast-binary `(test-list (test <name> <is-property> <file>)…)`
    /// value (NOT JSON) — the operator cadenza-ast-binary-everywhere directive + format-identical to the
    /// delegate `Query::TestList` path, so v-nix's dynamic-derivations discovery decodes ONE format. Pins:
    /// the bytes DECODE as cadenza-ast (a JSON regression would fail `codec::decode`), the root is a
    /// `(test-list …)` form, and each `@test` appears once as a positional `(test name is-property file)`.
    #[test]
    fn list_tests_emits_cadenza_ast_binary_test_list() {
        let dir = tmp("list-tests-binary");
        let f = dir.join("suite.cdz");
        // Two nullary @tests: a plain unit test + a `-gen` name (the `Test.gen` property wrapper the delegate
        // path flags is-property, exercised here without needing a param so the file loads trivially).
        std::fs::write(
            &f,
            "@test\ndef alpha-passes() = unit\n\n@test\ndef beta-gen() = unit\n",
        )
        .unwrap();
        let bytes =
            list_test_bytes(&[f.to_string_lossy().into_owned()]).expect("enumerates the suite");
        // Must DECODE as a cadenza-ast value — a JSON regression (serde bytes) would not.
        let a = cadenza_syntax::codec::decode(&bytes)
            .expect("--list output is cadenza-ast binary, not JSON");
        let children = a
            .as_form(a.root, "test-list")
            .expect("root is a `(test-list …)` form");
        assert_eq!(children.len(), 2, "both @tests enumerated once");
        let mut names: Vec<String> = Vec::new();
        for &c in children {
            let fields = a
                .as_form(c, "test")
                .expect("each child is a `(test …)` form");
            assert_eq!(fields.len(), 3, "positional name / is-property / file");
            names.push(a.as_str(fields[0]).expect("name is a Str").to_string());
        }
        names.sort();
        assert_eq!(
            names,
            vec!["alpha-passes".to_string(), "beta-gen".to_string()]
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// `--list --format nix` emits a PURE, `(file, name)`-SORTED nix attrset list — the eval-readable
    /// projection v-nix's scoped-cached-IFD discovery drv writes to `$out` + imports. Pins the exact shape
    /// (attr names `name`/`is_property`/`file` matching the emit-shred manifest) + the stable sort (so an
    /// identical `@test` set → byte-identical output → the discovery drv is content-stable for the IFD cache).
    #[test]
    fn list_test_nix_emits_a_sorted_pure_attrset_list() {
        // Unsorted input across two files; nix output must sort by (file, name): a.cdz/apple, a.cdz/beta,
        // b.cdz/zebra — regardless of enumeration order.
        let entries = vec![
            ("zebra".to_string(), false, "b.cdz".to_string()),
            ("beta".to_string(), false, "a.cdz".to_string()),
            ("apple".to_string(), true, "a.cdz".to_string()),
        ];
        let nix = list_test_nix(entries);
        assert_eq!(
            nix,
            "[\n  \
             { name = \"apple\"; is_property = true; file = \"a.cdz\"; }\n  \
             { name = \"beta\"; is_property = false; file = \"a.cdz\"; }\n  \
             { name = \"zebra\"; is_property = false; file = \"b.cdz\"; }\n\
             ]\n"
        );
    }

    /// `nix_str` quotes + escapes the chars that would break an emitted/`import`-ed nix string: `"`, `\`, a
    /// `${` antiquotation opener, and newline.
    #[test]
    fn nix_str_escapes_nix_special_chars() {
        assert_eq!(nix_str("plain-name"), "\"plain-name\"");
        assert_eq!(nix_str("a\"b"), "\"a\\\"b\"");
        assert_eq!(nix_str("a\\b"), "\"a\\\\b\"");
        assert_eq!(nix_str("a${b}"), "\"a\\${b}\""); // the `${` opener escaped; a bare `}` is fine
        assert_eq!(nix_str("a\nb"), "\"a\\nb\"");
    }

    /// `cdz test --emit-shred --two-stage` (§S6b stage-2) writes cadenza-ast FRAGMENTS to the out-dir: one
    /// shared `closure-<group>.cdzb` (the reachable non-`@test` library), one per-`@test` `test-<name>.cdzb`,
    /// and a merged `manifest.cdzb` whose per-entry `main-file` is the group closure fragment, `target` the
    /// per-test fragment, and `export` the test symbol — the two fragments and symbol the fan-out
    /// splice-compiles via `rcdzc <main-file> <target> --export <export>`. Pins the surface's file-naming and
    /// manifest rewrite; the wasm peer/standalone paths write components, whereas this is the FRAGMENT shape.
    #[test]
    fn emit_shred_two_stage_writes_closure_and_per_test_fragments_and_manifest() {
        let dir = tmp("emit-shred-two-stage");
        let suite = dir.join("suite.sexp");
        // A recursive helper `tri` (emitted standalone, so it lands in the shared closure) + two `@test`s.
        std::fs::write(
            &suite,
            "(do \
             (def (tri (: n Int64)) (if (= n 0) 0 (+ n (tri (- n 1))))) \
             (@ test (def (t-a) (if (= (tri 3) 6) unit (trap \"x\")))) \
             (@ test (def (t-b) (if (= (tri 4) 10) unit (trap \"x\")))))",
        )
        .unwrap();
        let out = dir.join("out");
        // The written artifacts ARE the behavior contract (`ExitCode` is not `PartialEq`); a failed emit
        // would not produce the fragment set + manifest asserted below.
        let _ = run_emit_shred(
            &[suite.to_string_lossy().into_owned()],
            &out,
            /*standalone*/ false,
            /*two_stage*/ true,
        );
        // The shared closure fragment (group 0) + one per-test fragment each, all `.cdzb`, NO `.wasm`.
        assert!(
            out.join("closure-0.cdzb").is_file(),
            "closure-0.cdzb written"
        );
        assert!(out.join("test-t-a.cdzb").is_file(), "test-t-a.cdzb written");
        assert!(out.join("test-t-b.cdzb").is_file(), "test-t-b.cdzb written");
        let wasm: Vec<_> = std::fs::read_dir(&out)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|x| x == "wasm"))
            .collect();
        assert!(
            wasm.is_empty(),
            "two-stage writes fragments, no wasm: {wasm:?}"
        );
        // The manifest: per-entry target=test-<name>.cdzb, main-file=closure-0.cdzb, export=<name>.
        let mbytes = std::fs::read(out.join("manifest.cdzb")).expect("manifest.cdzb");
        let a = cadenza_syntax::codec::decode(&mbytes).expect("manifest decodes as cadenza-ast");
        let entries = a
            .as_form(a.root, "shred-manifest")
            .expect("(shred-manifest …)");
        assert_eq!(entries.len(), 2, "one entry per @test");
        for &e in entries {
            let f = a.as_form(e, "entry").expect("(entry …)");
            // [0]name [1]isprop [2]file [3]export [4]target [5]iface [6]main-file
            let name = a.as_str(f[0]).unwrap_or("");
            assert_eq!(a.as_str(f[3]), Some(name), "export = the test name");
            assert_eq!(
                a.as_str(f[4]),
                Some(format!("test-{name}.cdzb").as_str()),
                "target = the per-test fragment"
            );
            assert_eq!(
                a.as_str(f[6]),
                Some("closure-0.cdzb"),
                "main-file = the group's shared closure fragment"
            );
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn emitted_pub_fn_names_lists_each_top_level_export() {
        // `cdz run-rust` picks the sole `pub fn` as the default export; several → require --call. The
        // list must be by source order and count only TOP-LEVEL (column-0) `pub fn`s (Copilot PR #547).
        let module =
            "#![allow(warnings)]\npub fn main() -> i64 { 1 }\npub fn other() -> i64 { 2 }\n";
        assert_eq!(emitted_pub_fn_names(module), vec!["main", "other"]);
        // A nested/inner `pub fn` (indented) is not a module export — not counted.
        let one = "pub fn main() -> i64 {\n    pub fn helper() {}\n    1\n}\n";
        assert_eq!(emitted_pub_fn_names(one), vec!["main"]);
        assert!(emitted_pub_fn_names("fn not_pub() {}\n").is_empty());
    }

    #[test]
    fn export_param_arity_reads_the_signature() {
        assert_eq!(
            export_param_arity("pub fn main() -> i64 { 0 }", "main"),
            Some(0)
        );
        assert_eq!(
            export_param_arity("pub fn f(n: i64) -> i64 { n }", "f"),
            Some(1)
        );
        // A tuple/generic PARAM type has inner commas that must NOT inflate the arity.
        assert_eq!(
            export_param_arity("pub fn g(p: (i64, i64), q: Vec<(A, B)>) -> i64 { 0 }", "g"),
            Some(2)
        );
        // Absent export → None (a bad --call).
        assert_eq!(
            export_param_arity("pub fn main() -> i64 { 0 }", "nope"),
            None
        );
        // A name that is only a PREFIX of an emitted fn must not match.
        assert_eq!(
            export_param_arity("pub fn main2() -> i64 { 0 }", "main"),
            None
        );
    }

    #[test]
    fn panic_reason_is_deterministic_no_temp_path() {
        // Modern Rust: `… panicked at <FILE>:<LINE>:<COL>:` then the payload on the NEXT line. Return the
        // payload (stable), NOT the temp-path line (per-run — Copilot PR #547).
        let modern = "thread 'main' panicked at /tmp/cdz-run-rust-123-4/prog.rs:9:14:\nattempt to divide by zero\nnote: run with RUST_BACKTRACE=1 …";
        assert_eq!(panic_reason(modern), "attempt to divide by zero");
        // The reason must not carry the per-run temp path or line number.
        assert!(!panic_reason(modern).contains("/tmp/"));
        assert!(!panic_reason(modern).contains("prog.rs"));
        // Older format: `panicked at '<payload>', <file>:<line>` → the quoted payload.
        let older = "thread 'main' panicked at 'overflow', /tmp/x/prog.rs:1:1";
        assert_eq!(panic_reason(older), "overflow");
        // Unexpected format → first non-empty line, never empty.
        assert_eq!(panic_reason("\n\nboom\n"), "boom");
        assert_eq!(panic_reason(""), "panic");
    }

    #[test]
    fn propagate_equalities_copies_left_onto_right_to_a_fixpoint() {
        let eq = |left, right| Relation {
            left,
            op: "=",
            right,
        };
        let strs = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect::<Vec<_>>();

        // A single `(= a b)` copies param 0's value onto param 1.
        let mut v = strs(&["7", "3"]);
        propagate_equalities(&[eq(0, 1)], &mut v);
        assert_eq!(v, strs(&["7", "7"]));

        // A chain `(= a b)`, `(= b c)` propagates to a fixpoint — all take param 0's value — even though `b`
        // is only set on the first pass; the fixpoint iteration carries it to `c`.
        let mut v = strs(&["5", "1", "9"]);
        propagate_equalities(&[eq(0, 1), eq(1, 2)], &mut v);
        assert_eq!(v, strs(&["5", "5", "5"]));

        // Fixpoint is ORDER-INDEPENDENT: the same chain recorded in reverse still fully propagates.
        let mut v = strs(&["5", "1", "9"]);
        propagate_equalities(&[eq(1, 2), eq(0, 1)], &mut v);
        assert_eq!(v, strs(&["5", "5", "5"]));

        // An ORDER relation is NOT propagated (only `=` is) — the values are left untouched here.
        let mut v = strs(&["2", "8"]);
        propagate_equalities(
            &[Relation {
                left: 0,
                op: "<",
                right: 1,
            }],
            &mut v,
        );
        assert_eq!(v, strs(&["2", "8"]));
    }

    // ── git-style plugin dispatch (thin-`cdz` seam) ──────────────────────────────────────────────────

    #[test]
    fn plugin_env_key_upper_snakes_the_subcommand() {
        // The `$CDZ_<NAME>_BIN` override key: upper-case, non-alphanumeric → `_`, `_BIN` suffix.
        assert_eq!(plugin_env_key("run"), "CDZ_RUN_BIN");
        assert_eq!(plugin_env_key("run-rust"), "CDZ_RUN_RUST_BIN");
        assert_eq!(plugin_env_key("Corpus"), "CDZ_CORPUS_BIN");
    }

    #[test]
    fn is_known_subcommand_recognizes_builtins_but_not_plugins() {
        // Builtin-first precedence: the in-process subcommands (+ their aliases) are known; an arbitrary
        // plugin name is not, so it is eligible for external `cdz-<name>` dispatch. Enumerated from the
        // derived clap tree so this can't drift from the actual subcommand set.
        assert!(is_known_subcommand("compile"));
        assert!(is_known_subcommand("run"));
        assert!(is_known_subcommand("convert"));
        assert!(!is_known_subcommand("frobnicate"));
        assert!(!is_known_subcommand("run-quux"));
    }

    #[test]
    fn resolve_plugin_honors_env_then_sibling_then_path() {
        // A tempdir sandbox with a fake `cdz-frob` in three candidate locations, exercising the
        // env → sibling → PATH priority WITHOUT mutating process-global env (the pure `resolve_plugin`
        // takes the env value / exe-dir / PATH dirs explicitly).
        let root = std::env::temp_dir().join(format!("cdz-plugin-resolve-{}", std::process::id()));
        let _guard = RemoveOnDrop::dir(root.clone());
        let env_dir = root.join("env");
        let sib_dir = root.join("sib");
        let path_dir = root.join("path");
        for d in [&env_dir, &sib_dir, &path_dir] {
            std::fs::create_dir_all(d).unwrap();
        }
        let touch = |dir: &Path, stem: &str| {
            let p = dir.join(bin_name(stem));
            std::fs::write(&p, b"#!/bin/sh\n").unwrap();
            p
        };
        let env_bin = touch(&env_dir, "cdz-frob");
        let sib_bin = touch(&sib_dir, "cdz-frob");
        let _path_bin = touch(&path_dir, "cdz-frob");
        let path_dirs = vec![path_dir.clone()];

        // 1. Env override wins over everything (points at a real file).
        assert_eq!(
            resolve_plugin(
                "frob",
                Some(env_bin.clone().into_os_string()),
                Some(&sib_dir),
                &path_dirs
            ),
            Some(env_bin.clone())
        );
        // 2. A non-existent env override is IGNORED → falls to the sibling dir.
        assert_eq!(
            resolve_plugin(
                "frob",
                Some(OsString::from("/no/such/cdz-frob")),
                Some(&sib_dir),
                &path_dirs
            ),
            Some(sib_bin.clone())
        );
        // 3. No env, no sibling match → PATH lookup.
        let empty_dir = root.join("empty");
        std::fs::create_dir_all(&empty_dir).unwrap();
        assert_eq!(
            resolve_plugin("frob", None, Some(&empty_dir), &path_dirs),
            Some(path_dir.join(bin_name("cdz-frob")))
        );
        // 4. Nothing resolves → None (caller falls through to clap's error).
        assert_eq!(
            resolve_plugin(
                "frob",
                None,
                Some(&empty_dir),
                std::slice::from_ref(&empty_dir)
            ),
            None
        );
    }

    #[test]
    fn wants_toplevel_help_only_for_bare_top_level_help() {
        let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(wants_toplevel_help(&v(&["cdz", "--help"])));
        assert!(wants_toplevel_help(&v(&["cdz", "-h"])));
        assert!(wants_toplevel_help(&v(&["cdz", "help"])));
        // NOT bare `cdz` (clap's usage), NOT a real subcommand, NOT `cdz <sub> --help` (clap's).
        assert!(!wants_toplevel_help(&v(&["cdz"])));
        assert!(!wants_toplevel_help(&v(&["cdz", "compile"])));
        assert!(!wants_toplevel_help(&v(&["cdz", "convert", "--help"])));
    }

    #[test]
    fn discover_plugin_names_walks_dedups_and_skips_builtins() {
        let root = std::env::temp_dir().join(format!("cdz-plugin-discover-{}", std::process::id()));
        let _guard = RemoveOnDrop::dir(root.clone());
        let d1 = root.join("d1");
        let d2 = root.join("d2");
        for d in [&d1, &d2] {
            std::fs::create_dir_all(d).unwrap();
        }
        let touch = |dir: &Path, fname: &str| std::fs::write(dir.join(fname), b"x").unwrap();
        // d1: two real plugins + a builtin-shadowing name + a non-cdz file.
        touch(&d1, &bin_name("cdz-foo"));
        touch(&d1, &bin_name("cdz-compile")); // shadows a builtin → skipped
        touch(&d1, "cdz"); // the dispatcher itself is not `cdz-<name>` → skipped
        touch(&d1, "unrelated");
        // d2: a new plugin + a DUPLICATE of foo (d1 wins, but dedup means one `foo` regardless).
        touch(&d2, &bin_name("cdz-bar"));
        touch(&d2, &bin_name("cdz-foo"));

        // Treat only "compile" as a builtin here (isolate the skip logic from the real clap tree).
        let names = discover_plugin_names(&[d1.clone(), d2.clone()], |n| n == "compile");
        assert_eq!(names, vec!["bar".to_string(), "foo".to_string()]);
    }

    #[cfg(unix)]
    #[test]
    fn plugin_summary_reads_one_line_and_degrades_gracefully() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("cdz-plugin-summary-{}", std::process::id()));
        let _guard = RemoveOnDrop::dir(root.clone());
        std::fs::create_dir_all(&root).unwrap();
        let write_script = |name: &str, body: &str| {
            let p = root.join(name);
            std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            p
        };
        // A well-formed plugin: prints exactly one line on --cdz-summary, exit 0.
        let good = write_script("cdz-good", "echo 'run a thing well'");
        assert_eq!(plugin_summary(&good), Some("run a thing well".to_string()));
        // Non-zero exit → None (graceful degrade).
        let fails = write_script("cdz-fails", "exit 3");
        assert_eq!(plugin_summary(&fails), None);
        // Multi-line stdout → not a well-formed summary → None.
        let chatty = write_script("cdz-chatty", "echo one; echo two");
        assert_eq!(plugin_summary(&chatty), None);
        // A plugin that HANGS past the timeout is killed → None (never hangs `cdz --help`).
        let hangs = write_script("cdz-hangs", "sleep 30");
        let t0 = std::time::Instant::now();
        assert_eq!(plugin_summary(&hangs), None);
        assert!(
            t0.elapsed() < std::time::Duration::from_secs(5),
            "plugin_summary must time out fast, not wait for the hung child"
        );
    }

    #[test]
    fn parse_plugin_summary_accepts_one_line_rejects_empty_and_multi() {
        assert_eq!(
            parse_plugin_summary("run a thing\n"),
            Some("run a thing".to_string())
        );
        assert_eq!(
            parse_plugin_summary("  trimmed  \n"),
            Some("trimmed".to_string())
        );
        assert_eq!(parse_plugin_summary(""), None);
        assert_eq!(parse_plugin_summary("\n  \n"), None);
        assert_eq!(parse_plugin_summary("one\ntwo\n"), None);
    }

    // Pins the run_project run-step forward's RunArgs->argv reconstruction (a missed/mis-mapped field would
    // silently regress `cdz run <project>`). Every RUN-relevant field set distinctively; the build/grade
    // fields (release/opt_level/grade/compile_status/compile_diag/component_name/baseline) are set to their
    // zero values and MUST NOT appear in the argv.
    #[test]
    fn cdz_run_forward_argv_maps_every_run_relevant_field() {
        use cdz_run::cli::{OutputFormat, RunArgs};
        let full = RunArgs {
            component: Some(std::path::PathBuf::from("built.wasm")),
            call: Some("go".into()),
            args: vec!["-4".into(), "x".into()],
            call_twice: true,
            then_args: vec!["7".into()],
            drop_handle: true,
            call_member: Some("len".into()),
            format: OutputFormat::BinaryAst,
            runtime: Some(std::path::PathBuf::from("/rt.wasm")),
            store: Some(std::path::PathBuf::from("/store")),
            host_responses: vec!["ask.ask=10".into()],
            peers: vec!["cadenza:math/api=math.wasm".into()],
            report_live_objects: true,
            // Every remaining field (the build/grade wire: release/opt_level/grade/compile_status/
            // compile_diag/diagnostics/component_name/tolerate_fewer_live_objects/baseline/emit_verdict/
            // core_*) stays at its zero default and MUST NOT emit any argv — the spread pins exactly that.
            ..Default::default()
        };
        assert_eq!(
            cdz_run_forward_argv(&full),
            vec![
                "built.wasm",
                "--call",
                "go",
                "--arg",
                "-4",
                "--arg",
                "x",
                "--call-twice",
                "--then-arg",
                "7",
                "--drop-handle",
                "--call-member",
                "len",
                "--format",
                "binary-ast",
                "--runtime",
                "/rt.wasm",
                "--store",
                "/store",
                "--host-response",
                "ask.ask=10",
                "--peer",
                "cadenza:math/api=math.wasm",
                "--report-live-objects",
            ]
        );
        // Minimal: only a component + the always-emitted default `--format sexp`; nothing else leaks.
        // Only a component; every other field defaults (format→Sexp), so nothing but the component + the
        // always-emitted default `--format sexp` leaks into the argv.
        let minimal = RunArgs {
            component: Some(std::path::PathBuf::from("c.wasm")),
            ..Default::default()
        };
        assert_eq!(
            cdz_run_forward_argv(&minimal),
            vec!["c.wasm", "--format", "sexp"]
        );
    }
}
