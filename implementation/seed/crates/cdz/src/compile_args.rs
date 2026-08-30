//! `cdz compile`'s argument struct — a cdz-LOCAL mirror of `rcdzc`'s CLI compile args, so the thin
//! front-end owns arg parsing WITHOUT depending on `rcdzc::cli::CompileArgs` (whose fields are private +
//! which a `!standalone` build must not link). The accessors project the parsed flags onto the
//! `cadenza-compile-abi` boundary types, and BOTH dispatch paths consume those: a `standalone` build
//! hands them to `rcdzc::cli::run_with_specs` (the parsed-values compile core), a `!standalone` build
//! hands them to the `cdz-compile` delegate. This is the thin-`cdz` end state — `cdz` is the front door
//! that parses; the compiler (bundled or delegated) is reached through these values, never this struct.
//!
//! DRIFT NOTE: this MIRRORS `rcdzc::cli::CompileArgs`'s flag surface (names/values/defaults/conflicts) so
//! `cdz compile` parses byte-identically to the standalone `rcdzc`/`cdz-compile` binary. A new compile
//! flag must be added to BOTH (accepted per the operator thin-dispatcher direction — the abi crate is
//! dep-light + clap-free, so the args struct can't live there). The `standalone` in-process path calls
//! `run_with_specs`, which takes exactly these accessor values.

use cadenza_compile_abi::{OptLevel, OverflowMode, OverflowSpec, Target};
use std::path::{Path, PathBuf};

/// The parsed `cdz compile` arguments (see module docs). Mirrors `rcdzc::cli::CompileArgs`. `Parser`
/// (not just `Args`) so it embeds as the `Compile` subcommand AND parses standalone in tests
/// (`try_parse_from`); the embedded `cdz compile` name comes from the outer command.
#[derive(clap::Parser)]
#[command(
    name = "cdz-compile",
    about = "Compile Cadenza artifacts/sources to one or more backend targets."
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
    /// file's `(export …)` forms the component boundary. Ignored for a single-file compile.
    #[arg(long, value_name = "NAME")]
    entry: Option<String>,

    /// SPLICE mode: FLAT-MERGE every `ast` INPUT into ONE program + add a single `(export <SYM>)`, then
    /// compile that as a single standalone component (the two-stage test-shred per-test compile).
    /// DISTINCT from `--entry` (which LINKS files as a cross-component package); mutually exclusive.
    #[arg(long, value_name = "SYM", conflicts_with = "entry")]
    export: Option<String>,

    /// The INTERFACE this component publishes its exports under, when compiled as a cross-component
    /// PROVIDER (X4b) — e.g. `--component-name cadenza:math/api`. Injected as a `KIND_COMPONENT_NAME`
    /// artifact. Absent → the component exports its boundary funcs at top level.
    #[arg(long, value_name = "INTERFACE")]
    component_name: Option<String>,

    /// Optimization LEVEL (`O0`..`O3`) — how much work the compiler spends optimizing the emitted
    /// program. `O1` is the default. A higher level never changes what the program MEANS, only the
    /// artifact shape.
    #[arg(long, value_name = "LEVEL", default_value_t = OptLevelArg::default())]
    opt_level: OptLevelArg,

    /// The GLOBAL signed-integer overflow policy — `trap` / `wrap`. Omitted → fall through to the
    /// built-in `Trap`. The `Project.cdz` `def overflow-signed` global; a module pragma still overrides.
    #[arg(long, value_name = "MODE")]
    overflow_signed: Option<OverflowModeArg>,

    /// The GLOBAL unsigned-integer overflow policy (`trap`/`wrap`) — the `def overflow-unsigned` global.
    #[arg(long, value_name = "MODE")]
    overflow_unsigned: Option<OverflowModeArg>,

    /// Write the compiler's DIAGNOSTICS (the well-formedness fault set) to this path as a SIDE ARTIFACT —
    /// the `KIND_DIAGNOSTICS` wire. Written unconditionally; the process still exits with the normal
    /// compile status. Powers the corpus C1 diagnostic-quality grade.
    #[arg(long, value_name = "PATH")]
    emit_diagnostics: Option<PathBuf>,
}

impl CompileArgs {
    /// The raw input specs (`path` / `name=path` / `kind:name=path`) — the front-end pre-processes SOURCE
    /// inputs into artifacts before compiling; the compile core / delegate reads the rest through here.
    pub fn input_specs(&self) -> &[String] {
        &self.inputs
    }

    /// The explicit backend targets requested (empty when none — the callee applies the `wasm` default).
    pub fn targets(&self) -> Vec<Target> {
        self.target.iter().map(|&t| t.to_core()).collect()
    }

    /// Where output is written (`-o`), if given.
    pub fn out_path(&self) -> Option<PathBuf> {
        self.out.clone()
    }

    /// The `--entry <NAME>` of a multi-file package, if given.
    pub fn entry(&self) -> Option<&str> {
        self.entry.as_deref()
    }

    /// The `--export <SYM>` splice-mode boundary export, if given.
    pub fn export(&self) -> Option<&str> {
        self.export.as_deref()
    }

    /// The `--component-name <INTERFACE>`, if given.
    pub fn component_name(&self) -> Option<&str> {
        self.component_name.as_deref()
    }

    /// The requested optimization [`OptLevel`] (`--opt-level`, default `OptLevel::default()`).
    pub fn opt_level(&self) -> OptLevel {
        self.opt_level.to_core()
    }

    /// The GLOBAL overflow policy (`--overflow-signed`/`--overflow-unsigned`) as an [`OverflowSpec`].
    /// Either sub-form absent → `None` (that signedness falls through to the built-in `Trap`).
    pub fn overflow_spec(&self) -> OverflowSpec {
        OverflowSpec {
            signed: self.overflow_signed.map(OverflowModeArg::to_core),
            unsigned: self.overflow_unsigned.map(OverflowModeArg::to_core),
        }
    }

    /// The `--emit-diagnostics <PATH>` side-artifact path, if given.
    pub fn emit_diagnostics(&self) -> Option<&Path> {
        self.emit_diagnostics.as_deref()
    }
}

/// The `--opt-level` choice, as a clap-parsed value — its own enum so clap validates the spelling and
/// `--help` lists `o0..o3`. Mirrors `rcdzc::cli::OptLevelArg`.
#[derive(Clone, Copy, clap::ValueEnum, Default)]
pub enum OptLevelArg {
    /// Fast dev iteration — canonicalization only.
    #[value(name = "o0", alias = "O0")]
    O0,
    /// The DEFAULT — `O0` plus cheap local cleanups.
    #[default]
    #[value(name = "o1", alias = "O1")]
    O1,
    /// Release — `O1` plus whole-function analyses.
    #[value(name = "o2", alias = "O2")]
    O2,
    /// Aggressive — `O2` plus whole-program / speculative passes.
    #[value(name = "o3", alias = "O3")]
    O3,
}

impl OptLevelArg {
    fn to_core(self) -> OptLevel {
        match self {
            OptLevelArg::O0 => OptLevel::O0,
            OptLevelArg::O1 => OptLevel::O1,
            OptLevelArg::O2 => OptLevel::O2,
            OptLevelArg::O3 => OptLevel::O3,
        }
    }
}

impl std::fmt::Display for OptLevelArg {
    // `default_value_t` needs `Display`; the string must match a `#[value(name)]` so the default
    // round-trips through clap's parser.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            OptLevelArg::O0 => "o0",
            OptLevelArg::O1 => "o1",
            OptLevelArg::O2 => "o2",
            OptLevelArg::O3 => "o3",
        };
        f.write_str(s)
    }
}

/// The `--overflow-signed`/`--overflow-unsigned` mode, as a clap-parsed value. Mirrors
/// `rcdzc::cli::OverflowModeArg`.
#[derive(Clone, Copy, clap::ValueEnum)]
pub enum OverflowModeArg {
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

/// A backend target, as a clap-parsed value. Mirrors `rcdzc::cli::TargetArg`.
#[derive(Clone, Copy, clap::ValueEnum)]
pub enum TargetArg {
    /// A WebAssembly component.
    Wasm,
    /// A WebAssembly component carrying EMBEDDED DWARF debug info (Mode E). Needs a `spans` input.
    WasmDebug,
    /// A DETACHED DWARF sidecar (Mode S) — a `<name>.dwarf` file. Also needs a `spans` input.
    Dwarf,
    /// Rust source (a `.rs` module, no FFI).
    Rust,
    /// Rust source in ASYNC, GAS-METERED form.
    RustAsync,
    /// Cadenza binary AST — the OPTIMIZED program lowered BACK to Cadenza (`.ast`).
    Cadenza,
}

impl TargetArg {
    fn to_core(self) -> Target {
        match self {
            TargetArg::Wasm => Target::Wasm,
            TargetArg::WasmDebug => Target::WasmDebug,
            TargetArg::Dwarf => Target::Dwarf,
            TargetArg::Rust => Target::Rust,
            TargetArg::RustAsync => Target::RustAsync,
            TargetArg::Cadenza => Target::Cadenza,
        }
    }
}
