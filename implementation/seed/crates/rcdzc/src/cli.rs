//! The `rcdzc` compile command surface, factored into the library so BOTH the standalone `rcdzc` bin
//! and the unified `cdz` bin drive ONE implementation. The thin bins call [`parse_and_run`], or embed
//! [`CompileArgs`] as a subcommand and call [`run`].
//!
//! Artifacts-in, artifacts-out: takes one or more NAMED input artifacts and a list of backend
//! targets (default `wasm`), runs the pure [`crate::compile`], and writes each produced artifact to a
//! file. Diagnostics go to stderr; a nonzero exit means at least one error diagnostic.
//!
//! This is the HOST boundary — it owns the filesystem and argument parsing, the concerns the pure
//! core deliberately excludes so that core ports to the Cadenza self-host. It is intentionally thin:
//! all compilation logic lives behind `crate::compile`.
//!
//! Usage:
//!   rcdzc <input.ast>… [--target wasm]… [-o OUT]
//!   rcdzc kind:name=path.ast --target wasm -o build/
//!   rcdzc main.ast -o out/hello.wasm       # single output → an exact file path
//!   rcdzc - -o -                           # stdin → stdout: composes in a pipe (single artifact)
//!
//! An input is `path`, `name=path`, or `kind:name=path`; kind defaults to `ast`, name to the file
//! stem. `-o` is a DIRECTORY into which each artifact is written as `<name>.<ext-for-kind>` — EXCEPT
//! when exactly one artifact is produced and `-o` does not name an existing directory, in which case
//! `-o` is the exact output FILE path. With no `-o`, artifacts are written to the current directory.

use crate::{Artifact, OptLevel, Severity, Target, compile_with_opt};
use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;

/// Compile Cadenza binary-AST artifacts to one or more backend targets. `#[command(...)]` here names
/// the standalone `rcdzc` bin; embedded as a `cdz compile` subcommand the outer bin supplies the name.
#[derive(Parser)]
#[command(
    name = "rcdzc",
    about = "The reference Cadenza → component compiler (artifacts in, artifacts out)."
)]
pub struct CompileArgs {
    /// Input artifacts: `path`, `name=path`, or `kind:name=path` (kind defaults to `ast`). Under
    /// `cdz compile`, a bare `path` may also be a SOURCE file (parsed in-process) or a DIRECTORY
    /// (recursed for every `.cdz`/`.ml`/`.sexp` source — a whole package tree; pair with `--entry`).
    #[arg(required = true, value_name = "INPUT")]
    inputs: Vec<String>,

    /// Backend target(s) to emit; repeatable. Defaults to `wasm` when none is given.
    #[arg(long, short, value_name = "TARGET")]
    target: Vec<TargetArg>,

    /// Where to write output. A directory holding `<name>.<ext>` per artifact; or, when a single
    /// artifact is produced and this is not an existing directory, the exact output file path.
    /// Defaults to the current directory.
    #[arg(long, short, value_name = "OUT")]
    out: Option<PathBuf>,

    /// The ENTRY file of a multi-file PACKAGE, by name (`DESIGN-package-linking.md`). Required when
    /// more than one `ast`/source input is given (including a recursed directory): it names which
    /// file's `(export …)` forms the component boundary. The other files are libraries reachable only
    /// through an explicit `(import …)`. Ignored for a single-file compile (that lone file is the
    /// entry). A file's name is its stem (`app.cdz` → `app`, `src/lib/util.cdz` → `util`) or the
    /// `name=` of an explicit `kind:name=path` spec.
    #[arg(long, value_name = "NAME")]
    entry: Option<String>,

    /// The INTERFACE this component publishes its exports under, when compiled as a cross-component
    /// PROVIDER (`DESIGN-cross-component-interop-rcdzc.md` X4b) — e.g. `--component-name cadenza:math/api`.
    /// A peer consumer binds to it with an `(effect …)` `(bind "cadenza:math/api")`-ed to this name (the
    /// effects-unified surface, U2). Injected as a `KIND_COMPONENT_NAME` artifact. Absent (the common
    /// case) → the component exports its boundary funcs at top level.
    #[arg(long, value_name = "INTERFACE")]
    component_name: Option<String>,

    /// Optimization LEVEL — how much work the compiler spends optimizing the emitted program, trading
    /// compile time for output quality (`DESIGN-tiered-optimization-levels-rcdzc.md`). `O0` = fast
    /// dev-iteration (canonicalization only); `O1` = the DEFAULT (cheap local cleanups); `O2` = release
    /// (whole-function analyses — LICM, global CSE, accumulator intro, inlining); `O3` = aggressive
    /// (whole-program / speculative). Omitted → the declared default (`O1`), so a non-interactive build
    /// picks a level without asking. A higher level never changes what the program MEANS, only how the
    /// artifact is shaped.
    #[arg(long, value_name = "LEVEL", default_value_t = OptLevelArg::default())]
    opt_level: OptLevelArg,

    /// Write the compiler's DIAGNOSTICS (the well-formedness fault set) to this path as a SIDE ARTIFACT —
    /// the `KIND_DIAGNOSTICS` wire (one fault per line, 8 TAB columns: severity/code/node/fix-kind/fix-node/
    /// fix-replacement/fix-verified/message), byte-identical to the `Query::Diagnostics` sidecar result
    /// (both call `sidecar::diagnostics_wire`, so they never drift). Written UNCONDITIONALLY — even when the
    /// compile has errors/declines (the fault set is exactly what a caller wants then) — and the process
    /// still exits with the NORMAL compile status (this flag is a side-channel, never a gate). Powers the
    /// corpus C1 diagnostic-quality grade (v-corpus-harness): `mkCorpusBuild` runs `--emit-diagnostics
    /// $out/diagnostics`, and the exec phase grades the captured wire.
    #[arg(long, value_name = "PATH")]
    emit_diagnostics: Option<PathBuf>,
}

/// The `--opt-level` choice, as a clap-parsed value (its own enum so clap validates the spelling and
/// `--help` lists `o0..o3` — the same wrapper pattern as [`TargetArg`], keeping the CLI surface here and
/// mapping to the core [`OptLevel`] via [`From`]). Its `Default` mirrors `OptLevel::default()` so a
/// no-flag build gets the core's declared default without duplicating which level that is.
#[derive(Clone, Copy, clap::ValueEnum)]
enum OptLevelArg {
    /// Fast dev iteration — canonicalization only (the cheapest correct emit).
    #[value(name = "o0", alias = "O0")]
    O0,
    /// The DEFAULT — `O0` plus cheap local cleanups (copy prop, algebraic identities, local CSE).
    #[value(name = "o1", alias = "O1")]
    O1,
    /// Release — `O1` plus whole-function analyses (LICM, global CSE, accumulator intro, inlining).
    #[value(name = "o2", alias = "O2")]
    O2,
    /// Aggressive — `O2` plus whole-program / speculative passes.
    #[value(name = "o3", alias = "O3")]
    O3,
}

impl Default for OptLevelArg {
    fn default() -> Self {
        // Mirror the core's declared default so `cdz compile` with no `--opt-level` = `rcdzc::compile`.
        OptLevelArg::from_core(OptLevel::default())
    }
}

impl OptLevelArg {
    /// The core [`OptLevel`] this CLI choice selects.
    fn to_core(self) -> OptLevel {
        match self {
            OptLevelArg::O0 => OptLevel::O0,
            OptLevelArg::O1 => OptLevel::O1,
            OptLevelArg::O2 => OptLevel::O2,
            OptLevelArg::O3 => OptLevel::O3,
        }
    }

    /// The CLI wrapper for a core [`OptLevel`] — used to derive `Default` from `OptLevel::default()`.
    fn from_core(level: OptLevel) -> Self {
        match level {
            OptLevel::O0 => OptLevelArg::O0,
            OptLevel::O1 => OptLevelArg::O1,
            OptLevel::O2 => OptLevelArg::O2,
            OptLevel::O3 => OptLevelArg::O3,
        }
    }
}

impl std::fmt::Display for OptLevelArg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `default_value_t` prints the default via Display; match clap's lower-case value spelling.
        write!(
            f,
            "{}",
            match self {
                OptLevelArg::O0 => "o0",
                OptLevelArg::O1 => "o1",
                OptLevelArg::O2 => "o2",
                OptLevelArg::O3 => "o3",
            }
        )
    }
}

/// A backend target, as a clap-parsed value (its own enum so clap validates the spelling and `--help`
/// lists the choices — extending to a second target is one variant here).
#[derive(Clone, Copy, clap::ValueEnum)]
enum TargetArg {
    /// A WebAssembly component.
    Wasm,
    /// A WebAssembly component carrying EMBEDDED DWARF debug info (Mode E) — steps through Cadenza
    /// source and inspects scalar arguments in GDB/LLDB/Chrome. Needs a `spans` input (supply it as
    /// `spans:NAME=path.spans`); without it the compile declines. Debug sections are inert + strippable.
    ///
    /// Selecting a debug-carrying target is a `--target` choice the caller makes, not a specification
    /// ambiguity — it changes which artifact a deployer wants, not what the program means.
    //= spec/capabilities/debug-information.md#whether-to-emit-debug-information-is-a-user-facing-choice
    //# Whether a derivation emits debug information MUST be treated as a user-facing build choice rather than a specification ambiguity, because it changes which artifact a deployer wants rather than what the program means.
    WasmDebug,
    /// A DETACHED DWARF sidecar (Mode S) — a `<name>.dwarf` file carrying only the debug sections, for a
    /// debugger to load alongside a lean (undecorated) component. Also needs a `spans` input.
    Dwarf,
    /// Rust source (a `.rs` module linking into a Rust codebase, no FFI).
    Rust,
    /// Rust source in ASYNC, GAS-METERED form: every fn is `async` and threads `env: &mut impl CdzEnv`,
    /// awaiting `env.consume(1)` at entry so the host meters fuel and can yield cooperatively.
    RustAsync,
    /// Cadenza binary AST — the OPTIMIZED program (after resolution/inference/const-fold/optimization)
    /// lowered BACK to Cadenza and emitted as the binary AST (`.ast`). Feed it back through the compiler
    /// (round-trip idempotence), pipe it into the syntax system for sexpr/ML, or hand it to the oracle.
    Cadenza,
}

impl From<TargetArg> for Target {
    fn from(t: TargetArg) -> Target {
        match t {
            TargetArg::Wasm => Target::Wasm,
            TargetArg::WasmDebug => Target::WasmDebug,
            TargetArg::Dwarf => Target::Dwarf,
            TargetArg::Rust => Target::Rust,
            TargetArg::RustAsync => Target::RustAsync,
            TargetArg::Cadenza => Target::Cadenza,
        }
    }
}

impl CompileArgs {
    /// The raw input specs (`path` / `name=path` / `kind:name=path`) — read by a wrapping driver (the
    /// `cdz` bin) that may pre-process SOURCE-file inputs into artifacts before compiling.
    pub fn input_specs(&self) -> &[String] {
        &self.inputs
    }

    /// The explicit backend targets requested on the command line (empty when none was given — the
    /// caller applies the default, see [`run_prepared`]).
    pub fn targets(&self) -> Vec<Target> {
        self.target.iter().map(|&t| t.into()).collect()
    }

    /// Where output is written (`-o`), if given.
    pub fn out_path(&self) -> Option<PathBuf> {
        self.out.clone()
    }

    /// The `--entry <NAME>` of a multi-file package, if given — the file whose exports form the
    /// component boundary. A wrapping driver turns this into a `KIND_ENTRY` input artifact.
    pub fn entry(&self) -> Option<&str> {
        self.entry.as_deref()
    }

    /// The `--component-name <INTERFACE>`, if given — the interface a cross-component PROVIDER publishes
    /// its exports under. A wrapping driver turns this into a `KIND_COMPONENT_NAME` input artifact.
    pub fn component_name(&self) -> Option<&str> {
        self.component_name.as_deref()
    }

    /// The requested optimization [`OptLevel`] (`--opt-level`, default `OptLevel::default()`). A wrapping
    /// driver (the `cdz` bin) reads this to call [`run_prepared`] with the chosen level.
    pub fn opt_level(&self) -> OptLevel {
        self.opt_level.to_core()
    }
}

/// Parse the whole `rcdzc` CLI from `std::env::args` and run it — the standalone bin's `main`.
pub fn parse_and_run() -> ExitCode {
    run(CompileArgs::parse(), "rcdzc")
}

/// Run one compile command, reporting tool-level errors under `prog` (the invoking binary's name).
/// This is the host boundary: filesystem + args + the trace sink.
pub fn run(cli: CompileArgs, prog: &str) -> ExitCode {
    // Install the trace sink at the HOST boundary (the lib core only EMITS events). Output goes to
    // stderr, filtered by `CDZ_LOG` (e.g. `CDZ_LOG=rcdzc::infer=trace` to watch only inference, or
    // `CDZ_LOG=rcdzc=trace` for every decision). With no `CDZ_LOG` set, nothing is installed and the
    // `trace!` sites compile to no-ops at runtime — a normal run pays nothing. The subscriber living
    // only in the binary is the right split: the lib is instrumentation-only, the bin decides the sink.
    //
    // A TOOL-PRIVATE env var (not the shared `RUST_LOG`): the pipeline runs as `cargo xtask` driving
    // `cdz-syntax | rcdzc | cdz-run`, and `RUST_LOG` would fan out to cargo, wasmtime, and every other
    // `tracing`/`env_logger` consumer in those processes. `CDZ_LOG` is read only by this subscriber, so
    // `CDZ_LOG=rcdzc=trace` shows the compiler's decisions and nothing else's noise.
    if std::env::var("CDZ_LOG").is_ok() {
        use tracing_subscriber::{EnvFilter, fmt};
        let _ = fmt()
            .with_env_filter(EnvFilter::from_env("CDZ_LOG"))
            .with_writer(std::io::stderr)
            // Show each event's source file:line — the trace sites map decisions straight back to
            // the code that made them, which is the whole point of a debugging trace. `tracing`
            // captures the call-site location in the event metadata (no cost when no subscriber is
            // installed), so this is a pure formatting choice here at the host boundary.
            .with_file(true)
            .with_line_number(true)
            // Drop the timestamp and level: every event here is a `TRACE` and the wall-clock time is
            // noise for reading a compile's decision flow — the file:line + target + message is what
            // matters. (The target still prefixes each line, so the module is clear.)
            .without_time()
            .with_level(false)
            .try_init();
    }

    // Read each named input artifact — from disk, or from stdin when the path is `-` (so the bin
    // composes in a pipe: `… | rcdzc - -o -`). A `-` input takes the kind/name from its spec, both
    // defaulting to `ast`/`main` since a piped artifact has no file stem to name it after.
    let mut inputs: Vec<Artifact> = Vec::new();
    for spec in &cli.inputs {
        let parsed = parse_input_spec(spec);
        let bytes = if parsed.path.as_os_str() == "-" {
            let mut buf = Vec::new();
            if let Err(e) = std::io::Read::read_to_end(&mut std::io::stdin(), &mut buf) {
                eprintln!("{prog}: cannot read stdin: {e}");
                return ExitCode::FAILURE;
            }
            buf
        } else {
            match std::fs::read(&parsed.path) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("{prog}: cannot read {}: {e}", parsed.path.display());
                    return ExitCode::FAILURE;
                }
            }
        };
        inputs.push(Artifact::new(parsed.kind, parsed.name, bytes));
    }

    // A `--entry <NAME>` names the package entry file — inject it as a `KIND_ENTRY` artifact (its bytes
    // ARE the entry name), the same stream `compile()` reads the entry from (`DESIGN-package-linking.md`
    // §3c). Absent, a multi-`ast` package declines (no rule to pick the entry); a single-file compile
    // needs none.
    if let Some(entry) = cli.entry() {
        inputs.push(entry_artifact(entry));
    }
    // A `--component-name <INTERFACE>` names the interface a PROVIDER publishes its exports under — inject
    // it as a `KIND_COMPONENT_NAME` artifact (X4b).
    if let Some(iface) = cli.component_name() {
        inputs.push(component_name_artifact(iface));
    }

    // Read `opt_level` + `emit_diagnostics` BEFORE moving `cli.out` into the call (a partial move would
    // poison `cli`, and the `&Path` must borrow a local, not a moved-from `cli`).
    let opt_level = cli.opt_level();
    let emit_diagnostics = cli.emit_diagnostics.clone();
    run_prepared(
        inputs,
        &cli.targets(),
        cli.out,
        opt_level,
        emit_diagnostics.as_deref(),
        prog,
    )
}

/// Build the `KIND_ENTRY` input artifact naming a package's entry file — its bytes are the entry name.
/// Shared by `run` (artifacts-in) and the `cdz` driver (source-in), so both deliver a package the same
/// way.
pub fn entry_artifact(name: &str) -> Artifact {
    Artifact::new(crate::link::KIND_ENTRY, "entry", name.as_bytes().to_vec())
}

/// Build the `KIND_COMPONENT_NAME` input artifact naming a provider's published interface (X4b) — its
/// bytes are the interface name. Shared by `run` and the `cdz` driver.
pub fn component_name_artifact(iface: &str) -> Artifact {
    Artifact::new(
        crate::link::KIND_COMPONENT_NAME,
        "component-name",
        iface.as_bytes().to_vec(),
    )
}

/// Compile a set of ALREADY-BUILT input artifacts to the requested targets and write the outputs — the
/// host boundary's compile+report+write tail, exposed so a wrapping driver (the `cdz` bin) can
/// pre-build artifacts from SOURCE files (parsing them in-process with its front-end, injecting the
/// `ast` + `spans` artifacts) and reuse the identical output-writing behavior. `targets` is the
/// explicit `--target` list (empty ⇒ apply the default here). `out` is the `-o` destination.
pub fn run_prepared(
    inputs: Vec<Artifact>,
    targets: &[Target],
    out: Option<PathBuf>,
    opt_level: OptLevel,
    emit_diagnostics: Option<&std::path::Path>,
    prog: &str,
) -> ExitCode {
    // Apply the target default here (so both `run` and an external driver get the same rule): explicit
    // targets win; else `[Wasm]` UNLESS a `sidecar` input drives the run (then its Emit requests name
    // the targets, and a default `wasm` would force an unwanted component for a query-only sidecar).
    // The default is the UNDECORATED `Wasm` component (debug excluded), so a non-interactive build that
    // names no debug target proceeds without asking whether to emit debug information. WHICH target to
    // emit is an open point resolvable more than one way; it carries this declared default (`[Wasm]`),
    // so a build reaching it without an explicit `--target` applies the default rather than halting.
    //= spec/capabilities/debug-information.md#whether-to-emit-debug-information-is-a-user-facing-choice
    //# Whether a derivation emits debug information MUST carry a declared default so that a non-interactive or autonomous build proceeds without asking.
    //= spec/capabilities/build-modes.md#an-open-point-carries-a-declared-default
    //# A specification point that a conforming generation could resolve in more than one way MUST carry a declared default that states the conforming choice to apply when the point is otherwise unresolved.
    //= spec/capabilities/build-modes.md#autonomous-mode-applies-a-declared-default-instead-of-asking
    //# An autonomous build MUST resolve a specification ambiguity by applying the point's declared default.
    let has_sidecar = inputs
        .iter()
        .any(|a| a.kind == crate::sidecar::KIND_SIDECAR);
    let targets: Vec<Target> = if !targets.is_empty() {
        targets.to_vec()
    } else if has_sidecar {
        Vec::new()
    } else {
        vec![Target::Wasm]
    };
    // Run the compile on a worker thread with a stack sized to reach the recursive-descent depth
    // guard, so pathologically deep input DECLINES (the guard trips) rather than overflowing the
    // native stack and aborting — the `decline-don't-crash` contract, made independent of whatever
    // stack the ambient thread happens to have. See `rcdzc::host`.
    let out_dest = out;
    let cli_out = &out_dest;
    let out = crate::run_with_compiler_stack(|| compile_with_opt(&inputs, &targets, opt_level));

    // `--emit-diagnostics <path>`: write the DIAGNOSTICS wire as a side artifact BEFORE reporting/writing,
    // UNCONDITIONALLY (even on an error/decline compile — the fault set is exactly what a caller wants
    // then), reusing `sidecar::diagnostics_wire` so it is byte-identical to the `Query::Diagnostics` result.
    // A write failure is a warning, not a compile failure — the flag is a side-channel, so the process
    // still exits with the NORMAL compile status below (it never gates). Powers the corpus C1 grade.
    if let Some(path) = emit_diagnostics {
        let wire = crate::sidecar::diagnostics_wire(&out.diagnostics);
        if let Err(e) = std::fs::write(path, &wire) {
            eprintln!(
                "{prog}: cannot write --emit-diagnostics {}: {e}",
                path.display()
            );
        }
    }

    // Report diagnostics (stderr). When the inputs carry a `spans` side-table (present whenever the run
    // compiled a SOURCE file — `cdz compile foo.cdz`), map each diagnostic's node to a source
    // `path:line:col` prefix, so `compile` gives the SAME located errors as `check` rather than leaking a
    // raw internal `(node N)` id. Without spans (a bare artifacts-in compile) the node id still rides
    // along for a caller that holds its own table — the historical behavior, unchanged.
    // Each `spans` input paired with its ARTIFACT NAME — the name is what the `link-map` keys a file by
    // (`FileSpan.path`), which `SpanData.module_path` (a debug basename) does not preserve, so a linked
    // demux must match on the name.
    let span_tables: Vec<(String, crate::spans::SpanData)> = inputs
        .iter()
        .filter(|a| a.kind == crate::spans::KIND_SPANS)
        .filter_map(|a| crate::spans::decode(&a.bytes).map(|s| (a.name.clone(), s)))
        .collect();
    // The package `link-map` (if any): a LINKED build reports a diagnostic's GLOBAL node id (post-splice,
    // offset by the file's base), but each file's span table is keyed by LOCAL (pre-link) ids — so a raw
    // lookup misses in every table and the error loses its `file:line:col`. The link-map demuxes: the
    // global id `n` falls in one file's `[base, base+count)` → `(path, n - base)` = the file + LOCAL id
    // its span table can resolve (`DESIGN-package-linking.md` §6). Empty for a single-file compile, whose
    // ids are already local (the direct lookup below handles it).
    let link_map: Vec<crate::link::FileSpan> = out
        .artifacts
        .iter()
        .find(|a| a.kind == crate::link::KIND_LINK_MAP)
        .map(|a| crate::link::decode_link_map(&a.bytes))
        .unwrap_or_default();
    // Locate a diagnostic's node as `(&span-table, start_byte)`. First try the LINKED demux (global id →
    // file + local id via the link-map), then fall back to a DIRECT lookup across the tables (a
    // single-file build, whose ids are already local). Returns the OWNING table (not just its path) so
    // line/col are read off the right file even if two files share a basename. `None` for a node no
    // table covers.
    let locate = |node: u32| -> Option<(&crate::spans::SpanData, u32)> {
        // Linked: find the file whose global range contains the node, then resolve the LOCAL id in the
        // span table with the matching artifact name.
        if let Some(fs) = link_map
            .iter()
            .find(|f| f.contains(crate::ast::StructId(node)))
        {
            let local = node - fs.struct_base;
            if let Some((_, s)) = span_tables.iter().find(|(name, _)| *name == fs.path)
                && let Some((start, _)) = s.range(crate::ast::StructId(local))
            {
                return Some((s, start));
            }
        }
        // Single-file (or a node the link-map didn't cover): the id is already local to some table.
        span_tables.iter().find_map(|(_, s)| {
            s.range(crate::ast::StructId(node))
                .map(|(start, _)| (s, start))
        })
    };
    // The START byte offset of a diagnostic's node in its source, if locatable — the sort key that
    // orders diagnostics as a reader scans top-to-bottom. Keyed by (module path, start) so a two-file
    // package still orders deterministically.
    let start_of = |d: &crate::Diagnostic| -> Option<(String, u32)> {
        d.node
            .and_then(|n| locate(n).map(|(s, start)| (s.module_path.clone(), start)))
    };
    // Report in SOURCE ORDER (by module path, then start byte), not fault-collection order — the tree
    // walk that gathers faults does not visit strictly left-to-right, so without this a reader sees an
    // error at column 22 before one at column 21, or a derived error above the line that caused it. A
    // diagnostic with no locatable span (no spans supplied, or a spanless synthesized node) sorts LAST,
    // keeping its relative order via the STABLE sort — so the ordering stays a deterministic function of
    // the source (`diagnostics.md` §Diagnostics Are Emitted In A Deterministic Order), now also legible.
    let mut ordered: Vec<&crate::Diagnostic> = out.diagnostics.iter().collect();
    ordered.sort_by(|a, b| match (start_of(a), start_of(b)) {
        (Some(ka), Some(kb)) => ka.cmp(&kb),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
    // One LINE-START INDEX per span table, built ONCE — so rendering each diagnostic's `line:col` is a
    // binary search, not `line_at`/`col_at`'s O(byte_off) scan from the start of the source. A program
    // with MANY diagnostics (e.g. an unused-binding warning per def in a wide module) mapped each fault's
    // offset over the whole source → O(faults × source_len) = O(N²); the index makes it linear. Matched
    // to the located table by pointer identity (`locate` returns a borrow into `span_tables`).
    let line_starts: Vec<(&crate::spans::SpanData, crate::spans::LineStarts)> = span_tables
        .iter()
        .map(|(_, s)| (s, s.line_starts()))
        .collect();
    for d in ordered {
        let sev = match d.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        // Prefer a source location from the spans: locate the node (via the linked demux or the direct
        // lookup), then render `path:line:col` via the prebuilt per-table line-start index. Fall back to
        // `(node N)` when no spans were supplied, or to nothing when the diagnostic carries no node.
        let located = d.node.and_then(|n| {
            locate(n).map(|(s, start)| {
                let (line, col) = line_starts
                    .iter()
                    .find(|(t, _)| std::ptr::eq(*t, s))
                    .map(|(_, idx)| idx.line_col(start))
                    .unwrap_or_else(|| (s.line_at(start), s.col_at(start)));
                format!("{}:{}:{}", s.module_path, line, col)
            })
        });
        match (located, d.node) {
            (Some(loc), _) => match &d.code {
                Some(code) => eprintln!("{loc}: {sev} [{code}]: {}", d.message),
                None => eprintln!("{loc}: {sev}: {}", d.message),
            },
            (None, node) => {
                let at = node.map(|n| format!(" (node {n})")).unwrap_or_default();
                match &d.code {
                    Some(code) => eprintln!("{prog}: {sev} [{code}]{at}: {}", d.message),
                    None => eprintln!("{prog}: {sev}{at}: {}", d.message),
                }
            }
        }
    }

    // The package `link-map` (`kind == "link-map"`) is a diagnostics DEMUX companion, not a primary
    // output — it does not count toward the "single artifact ⇒ exact file" / `-o -` decisions (else a
    // plain `-o app.wasm` component build would flip to directory mode the moment a package emits one).
    // It is written only in DIRECTORY mode (as `link-map.txt`); a `-o FILE` / `-o -` build, which names
    // one output, skips it.
    let primary: Vec<&Artifact> = out
        .artifacts
        .iter()
        .filter(|a| a.kind != crate::link::KIND_LINK_MAP)
        .collect();

    // `-o -`: write the single produced artifact's bytes to stdout (so the bin composes in a pipe:
    // `… | rcdzc - -o - | cdz-run`). Only meaningful for a single artifact — a multi-artifact build
    // has no one stream to write, so that is an error rather than an ambiguous concatenation.
    if cli_out.as_deref().map(|p| p.as_os_str()) == Some(std::ffi::OsStr::new("-")) {
        match primary.as_slice() {
            [art] => {
                if let Err(e) = std::io::Write::write_all(&mut std::io::stdout(), &art.bytes) {
                    eprintln!("{prog}: cannot write stdout: {e}");
                    return ExitCode::FAILURE;
                }
            }
            [] => {} // no artifact (errors already reported); fall through to the exit status.
            many => {
                eprintln!(
                    "cdz: `-o -` writes ONE artifact to stdout, but {} were produced",
                    many.len()
                );
                return ExitCode::FAILURE;
            }
        }
        return if out.has_error() {
            ExitCode::FAILURE
        } else {
            ExitCode::SUCCESS
        };
    }

    // Decide whether `-o` names an exact output FILE (a single PRIMARY artifact, not an existing
    // directory) or a DIRECTORY to write each `<name>.<ext>` into. The `link-map` companion does not
    // count — a lone component that also emits a `link-map` still writes to the exact `-o FILE`.
    let single_file_out: Option<&PathBuf> = match (cli_out, primary.as_slice()) {
        (Some(p), [_one]) if !p.is_dir() => Some(p),
        _ => None,
    };

    // A FAILED build writes NO output — like `cargo build`, which leaves no partial artifact on a compile
    // error. Without this, an errored build still wrote the `link-map` companion (`link-map.txt`) it
    // produced alongside the — absent — component, leaving a stray sidecar with no `.wasm` beside it (a
    // confusing partial state that also misleads a follow-up tool reading the map). Bail before the write
    // loop: the errors were already reported above, so just exit failure with a clean directory.
    if out.has_error() {
        return ExitCode::FAILURE;
    }

    // Write each produced artifact. In single-file (`-o FILE`) mode, write ONLY the primary artifact
    // there and skip the `link-map` companion (a `-o FILE` caller named one output). In directory mode,
    // write everything (the `link-map` lands as `link-map.txt` beside the outputs).
    for art in &out.artifacts {
        if single_file_out.is_some() && art.kind == crate::link::KIND_LINK_MAP {
            continue;
        }
        let path = match single_file_out {
            // Single artifact, `-o FILE`: write bytes to that exact path.
            Some(file) => file.clone(),
            // Otherwise: `<outdir>/<name>.<ext>`, outdir defaulting to the current directory.
            None => {
                let dir = cli_out.clone().unwrap_or_else(|| PathBuf::from("."));
                dir.join(format!("{}.{}", art.name, ext_for_kind(&art.kind)))
            }
        };
        if let Err(e) = std::fs::write(&path, &art.bytes) {
            eprintln!("{prog}: cannot write {}: {e}", path.display());
            return ExitCode::FAILURE;
        }
        eprintln!("cdz: wrote {} ({} bytes)", path.display(), art.bytes.len());
    }

    if out.has_error() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// A parsed input spec: the artifact kind, its logical name, and the file to read it from.
struct InputSpec {
    kind: String,
    name: String,
    path: PathBuf,
}

/// Parse an input spec: `path`, `name=path`, or `kind:name=path`. Kind defaults to `ast`; name
/// defaults to the file stem.
fn parse_input_spec(spec: &str) -> InputSpec {
    // Split an optional `kind:` prefix — only when it looks like one (no path separator or `=` before
    // the colon), so a Windows-y or `name=path` spec is not mistaken for a kind.
    let (kind, rest) = match spec.split_once(':') {
        Some((k, r)) if !k.contains('/') && !k.contains('=') => (k.to_string(), r),
        _ => (Artifact::KIND_AST.to_string(), spec),
    };
    // Split an optional `name=` prefix.
    let (name, path) = match rest.split_once('=') {
        Some((n, p)) => (n.to_string(), PathBuf::from(p)),
        None => {
            let path = PathBuf::from(rest);
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("input")
                .to_string();
            (stem, path)
        }
    };
    InputSpec { kind, name, path }
}

/// The file extension a produced artifact of the given kind is written with.
fn ext_for_kind(kind: &str) -> &str {
    match kind {
        "component" => "wasm",
        // The `EmitTestsShred` MAIN provider component — written `main.wasm` (its artifact NAME is "main") so
        // `cdz test --emit-shred -o D` produces the fixed `D/main.wasm` the per-test targets `--peer`-link.
        "component-provider" => "wasm",
        // The `EmitTestsShred` cadenza-ast manifest — a `codec::encode`d value (the `(shred-manifest …)`
        // tree), written `manifest.cdzb` (binary cadenza-ast, decoded with `cdz convert --from binary`).
        "shred-manifest" => "cdzb",
        "rust" => "rs",
        // A detached DWARF sidecar (Mode S) is a bare `.wasm`-format core module of debug sections;
        // written with a `.dwarf` extension so it is distinct from the runnable `<name>.wasm`.
        "dwarf" => "dwarf",
        // Sidecar QUERY results are UTF-8 text (a rendered type, a newline-separated node-id list) —
        // written with a `.txt` extension. A `sidecar` INPUT is read generically as `kind:name=path`,
        // so no case is needed for it here (this maps only produced OUTPUT kinds). The package
        // `link-map` (a diagnostics demux table) is likewise UTF-8 text.
        "type-info" | "uses" | "link-map" => "txt",
        other => other,
    }
}
