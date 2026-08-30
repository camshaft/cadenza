//! `cdz-compile` — compile Cadenza binary-AST artifacts to one or more backend targets.
//!
//! ```text
//! cdz-compile ast:main=program.ast [ast:<name>=module-<name>.ast]… \
//!             [wit-world:<name>=wit-world.ast] [--component-name <iface>] [--entry main] \
//!             --target wasm -o guest.wasm
//! ```
//!
//! A thin shim over `rcdzc_cli` (the embeddable clap `CompileArgs` + `run`), exposing the reference
//! compiler as a STANDALONE binary with a COMPILER-ONLY dependency closure (`rcdzc` + the thin clap
//! layer, WITHOUT the rest of `cdz`). It is the per-case `build` phase of the nix corpus caching
//! pipeline (`design/DESIGN-corpus-nix-per-case-caching.md`): its derivation rotates only when the
//! compiler changes, so a run-metadata edit never rebuilds and a compiler change that emits
//! byte-identical wasm leaves the downstream `exec` derivation's inputs untouched. Because the shred
//! already hands over artifacts in the compiler's native form (`ast:`/`wit-world:` binary AST + a
//! `--component-name` string), this bin is a pure passthrough — it adds no transform.
//!
//! The unified `cdz` binary mounts the SAME clap surface as `cdz compile`; this bin is that code
//! without the rest of `cdz` (LSP, syntax tooling, corpus, run), which is the point — a smaller closure
//! that the monolithic `cdz` (rebuilt on ANY subcommand edit) cannot give. Living in `rcdzc-cli` (not
//! `rcdzc`) is what keeps the compiler LIBRARY free of `clap`.

use std::process::ExitCode;

use clap::Parser;

/// The `cdz-compile` command line — the compiler's [`rcdzc_cli::CompileArgs`] under this bin's own name
/// (the flattened struct declares its own `name = "rcdzc"`, which the outer name here overrides for help
/// and usage). Its inputs, targets, `--entry`, and `--component-name` are exactly the compiler's.
#[derive(Parser)]
#[command(
    name = "cdz-compile",
    about = "Compile Cadenza binary-AST artifacts to backend targets (compiler-only closure)."
)]
struct Cli {
    #[command(flatten)]
    compile: rcdzc_cli::CompileArgs,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    rcdzc_cli::run(cli.compile, "cdz-compile")
}
