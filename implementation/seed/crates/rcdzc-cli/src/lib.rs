//! The `rcdzc` compiler's COMMAND-LINE surface — the `clap` arg parsing + the trace-sink install, kept
//! OUT of the compiler library so `rcdzc` stays a pure library with no `clap` dependency (operator
//! directive 2026-08-30). Both the standalone `cdz-compile` bin (this crate) and the unified `cdz` bin
//! drive ONE implementation: the clap-free compile driver in `rcdzc::cli` (`run_with_specs` and the
//! `run_prepared*` tail). This crate is only the clap layer on top — it parses args into the library's
//! core types and forwards them to that driver.
//!
//! This is the HOST boundary — it owns argument parsing (here) and, via `rcdzc::cli`, the filesystem.
//! The pure core (`rcdzc::compile`) deliberately excludes both so it ports to the Cadenza self-host.
//!
//! Usage:
//!   cdz-compile <input.ast>… [--target wasm]… [-o OUT]
//!   cdz-compile kind:name=path.ast --target wasm -o build/
//!   cdz-compile main.ast -o out/hello.wasm       # single output → an exact file path
//!   cdz-compile - -o -                           # stdin → stdout: composes in a pipe (single artifact)
//!
//! An input is `path`, `name=path`, or `kind:name=path`; kind defaults to `ast`, name to the file
//! stem. `-o` is a DIRECTORY into which each artifact is written as `<name>.<ext-for-kind>` — EXCEPT
//! when exactly one artifact is produced and `-o` does not name an existing directory, in which case
//! `-o` is the exact output FILE path. With no `-o`, artifacts are written to the current directory.

use clap::Parser;
use rcdzc::db::{OverflowMode, OverflowSpec};
use rcdzc::{OptLevel, Target};
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

    /// SPLICE mode: FLAT-MERGE every `ast` INPUT into ONE program (concatenating their top-level defs) +
    /// add a single `(export <SYM>)`, then compile that as a single standalone component. This is the
    /// two-stage test-shred per-test compile: `rcdzc <closure.cdzb> <test.cdzb> --export <sym> -o test.wasm`
    /// — the shared-closure fragment + the per-test fragment (each a no-export `(do (def..)..)` from
    /// `--target cadenza`'s fragment mode) merge into one `(do (def..)+ (export <sym>))`. `<SYM>` names the
    /// test's boundary export (a def in the merged program). DISTINCT from `--entry`: `--entry` LINKS files
    /// as a cross-component PACKAGE (via `(import …)`), whereas `--export` CONCATENATES them into ONE
    /// component — so a CA-cached shared-closure fragment is reused across a suite's tests with no per-test
    /// re-lower. Mutually exclusive with `--entry`.
    #[arg(long, value_name = "SYM", conflicts_with = "entry")]
    export: Option<String>,

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

    /// The GLOBAL signed-integer overflow policy for this compile — `trap` (overflow is a fault) or `wrap`
    /// (modulo 2^width). Omitted → no global default (that signedness falls through to the built-in
    /// `Trap`). This is the `Project.cdz` `def overflow-signed` manifest global reaching the compiler; a
    /// module `(pragma overflow (signed …))` still OVERRIDES it (precedence: module > this global > trap).
    #[arg(long, value_name = "MODE")]
    overflow_signed: Option<OverflowModeArg>,

    /// The GLOBAL unsigned-integer overflow policy (`trap`/`wrap`) — the `Project.cdz` `def
    /// overflow-unsigned` global; same precedence (module pragma > this > trap). Omitted → fall through.
    #[arg(long, value_name = "MODE")]
    overflow_unsigned: Option<OverflowModeArg>,

    /// Write the compiler's DIAGNOSTICS (the well-formedness fault set) to this path as a SIDE ARTIFACT —
    /// the `KIND_DIAGNOSTICS` wire (byte-identical to the `Query::Diagnostics` sidecar result). Written
    /// UNCONDITIONALLY — even when the compile has errors/declines (the fault set is exactly what a caller
    /// wants then) — and the process still exits with the NORMAL compile status (this flag is a
    /// side-channel, never a gate). Powers the corpus C1 diagnostic-quality grade (v-corpus-harness).
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

/// The `--overflow-signed`/`--overflow-unsigned` choice, as a clap-parsed value (its own enum so clap
/// validates the spelling `trap`/`wrap` and `--help` lists them), mapping to the core
/// [`OverflowMode`]. `trap` = overflow is a fault; `wrap` = modulo 2^width.
#[derive(Clone, Copy, Debug, clap::ValueEnum)]
enum OverflowModeArg {
    Trap,
    Wrap,
}

impl OverflowModeArg {
    fn to_core(self) -> OverflowMode {
        match self {
            OverflowModeArg::Trap => OverflowMode::Trap,
            OverflowModeArg::Wrap => OverflowMode::Wrap,
        }
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
    /// caller applies the default, see `rcdzc::cli::run_prepared`).
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
    /// driver (the `cdz` bin) reads this to call `rcdzc::cli::run_prepared` with the chosen level.
    pub fn opt_level(&self) -> OptLevel {
        self.opt_level.to_core()
    }

    /// The GLOBAL overflow policy (`--overflow-signed`/`--overflow-unsigned`) as an [`OverflowSpec`] —
    /// the pair a wrapping driver (the `cdz` bin) hands to `rcdzc::cli::run_prepared_with_overflow` to
    /// seed `db.global_overflow`. Either sub-form absent → `None` (that signedness falls through to the
    /// next precedence level — the built-in `Trap`).
    pub fn overflow_spec(&self) -> OverflowSpec {
        OverflowSpec {
            signed: self.overflow_signed.map(OverflowModeArg::to_core),
            unsigned: self.overflow_unsigned.map(OverflowModeArg::to_core),
        }
    }
}

/// Parse the whole `rcdzc` CLI from `std::env::args` and run it — the standalone bin's `main`.
pub fn parse_and_run() -> ExitCode {
    run(CompileArgs::parse(), "rcdzc")
}

/// Run one compile command, reporting tool-level errors under `prog` (the invoking binary's name).
/// This is the host boundary: argument parsing (via [`CompileArgs`]) + the trace sink here, and the
/// filesystem in the clap-free `rcdzc::cli` driver this forwards to.
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

    // Delegate to the clap-free compile core (`rcdzc::cli::run_with_specs`). `run` is the bin front
    // door — it owns the trace-sink install above — and `run_with_specs` is the reusable core a
    // DIFFERENT front-end (`cdz` in a `!standalone` build, which can't construct this crate's PRIVATE
    // `CompileArgs`) also calls with its OWN parsed args. Byte-identical: `run` pulls each field/accessor
    // and forwards.
    let targets = cli.targets();
    let opt_level = cli.opt_level();
    let overflow = cli.overflow_spec();
    rcdzc::cli::run_with_specs(
        &cli.inputs,
        &targets,
        cli.out.clone(),
        cli.entry(),
        cli.export.as_deref(),
        cli.component_name(),
        opt_level,
        overflow,
        cli.emit_diagnostics.as_deref(),
        prog,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `--overflow-signed`/`--overflow-unsigned` parse to the core [`OverflowMode`] and project to an
    /// [`OverflowSpec`] via `overflow_spec()`; an ABSENT flag is `None` (that signedness falls through to
    /// the built-in `Trap`, NOT an implicit trap override — the module-pragma > global > trap precedence
    /// relies on `None` meaning "no global default").
    #[test]
    fn overflow_flags_parse_to_a_global_spec_and_absent_is_none() {
        use clap::Parser;
        assert_eq!(OverflowModeArg::Trap.to_core(), OverflowMode::Trap);
        assert_eq!(OverflowModeArg::Wrap.to_core(), OverflowMode::Wrap);
        // Both flags → both sides of the spec.
        let both = CompileArgs::try_parse_from([
            "rcdzc",
            "prog.cdz",
            "--overflow-signed",
            "wrap",
            "--overflow-unsigned",
            "trap",
        ])
        .expect("parses");
        assert_eq!(
            both.overflow_spec(),
            OverflowSpec {
                signed: Some(OverflowMode::Wrap),
                unsigned: Some(OverflowMode::Trap),
            }
        );
        // Absent → None/None (the default): falls through to trap, distinct from an explicit `trap` global.
        let none = CompileArgs::try_parse_from(["rcdzc", "prog.cdz"]).expect("parses");
        assert_eq!(none.overflow_spec(), OverflowSpec::default());
        assert_eq!(none.overflow_spec().signed, None);
    }
}
