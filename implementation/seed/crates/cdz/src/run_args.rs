//! The thin-`cdz` `run` command's args — a git-plugin RAW-PASSTHROUGH surface (operator/concierge:
//! approach B, seq-…). `cdz run` FORWARDS to the external `cdz-run` binary (which holds wasmtime + the
//! full run/grade/core/precompile CLI); `cdz` does NOT re-declare cdz-run's 27-field `RunArgs` (that mirror
//! would be the cross-crate drift trap — a cdz-run field-add breaking the cdz build — now 27-wide, and 13
//! of those fields are cdz-run-BINARY concerns the nix corpus/compiler-ml/AOT pipelines drive directly).
//!
//! So `cdz` parses only what its OWN front-end paths need — the `component` (to detect a `Project.cdz`
//! target vs a pre-built `.wasm` vs a source-file mistake) and the PROJECT build flags (`--release`/
//! `--opt-level`, consumed by `run_project`'s build-then-run) — and forwards everything ELSE verbatim to
//! `cdz-run`. Its `--help` therefore shows a passthrough; the real run flags live on `cdz-run --help`.
//!
//! CONVENTION: the `component` is the FIRST positional (`cdz run <component> [flags…]`, or `cdz run
//! [--release]` with no component = the current-directory project). Flags placed BEFORE the component are
//! not front-end-interpreted (they ride `rest` to `cdz-run`) — the standard thin-dispatcher constraint,
//! since `cdz` cannot know cdz-run's flag arities to skip flag-values when locating the positional.

use std::path::PathBuf;

/// `cdz run` args: the front-end-interpreted subset + a verbatim passthrough tail for `cdz-run`.
#[derive(clap::Args, Clone, Default)]
pub struct RunArgs {
    /// The component `.wasm` to run (or `-` for stdin), OR omitted / a `Project.cdz` directory to
    /// build+run the project (the `cargo run` analogue). Must be the FIRST argument; every following
    /// argument is forwarded verbatim to `cdz-run`.
    pub component: Option<PathBuf>,

    /// PROJECT mode only: build the entry at the RELEASE tier (`O2`) before running (`cargo run --release`
    /// analogue). Consumed by `cdz` for the build; not forwarded. Ignored for a pre-built component.
    #[arg(long)]
    pub release: bool,

    /// PROJECT mode only: the optimization LEVEL (`O0`..`O3`) to build the entry at before running,
    /// overriding `--release`/the manifest. Consumed by `cdz` for the build; not forwarded.
    #[arg(long, value_name = "LEVEL")]
    pub opt_level: Option<String>,

    /// Every remaining argument, forwarded VERBATIM to the `cdz-run` binary (`--call`/`--arg`/`--format`/
    /// `--host-response`/`--peer`/…, plus the cdz-run-binary grade/core-module/precompile modes). `cdz`
    /// does not interpret these — `cdz-run` re-parses them as its full `RunArgs`. `allow_hyphen_values` so
    /// a leading-`-` value (`--arg -4`) rides through untouched.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub rest: Vec<String>,
}
