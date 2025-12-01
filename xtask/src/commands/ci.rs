use anyhow::Result;
use clap::{Args, Subcommand};
use xshell::{Shell, cmd};

use super::common;

#[derive(Args)]
pub struct Ci {
    #[command(subcommand)]
    command: Option<CiCommand>,
}

#[derive(Subcommand)]
pub enum CiCommand {
    /// Run cargo fmt check
    Fmt,
    /// Run cargo clippy
    Clippy,
    /// Run cargo udeps to check for unused dependencies
    Udeps,
    /// Run cargo test
    Test(TestArgs),
}

#[derive(Args, Default)]
pub struct TestArgs {
    /// Additional arguments to pass to cargo test
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    args: Vec<String>,
}

impl Ci {
    pub fn run(&self, sh: &Shell) -> Result<()> {
        match &self.command {
            Some(cmd) => cmd.run(sh),
            None => {
                // Run all CI checks
                CiCommand::Fmt.run(sh)?;
                CiCommand::Clippy.run(sh)?;
                CiCommand::Udeps.run(sh)?;
                CiCommand::Test(TestArgs::default()).run(sh)?;
                Ok(())
            }
        }
    }
}

impl CiCommand {
    pub fn run(&self, sh: &Shell) -> Result<()> {
        match self {
            CiCommand::Fmt => common::run_fmt_check(sh),
            CiCommand::Clippy => common::run_clippy(sh),
            CiCommand::Udeps => {
                // Ensure nightly with rustfmt is available
                // (rustfmt is needed for codegen to format output)
                common::ensure_nightly_rustfmt(sh)?;
                // Check if cargo-udeps is available, install if not
                if cmd!(sh, "cargo +nightly udeps --version")
                    .quiet()
                    .run()
                    .is_err()
                {
                    eprintln!("Installing cargo-udeps...");
                    cmd!(sh, "cargo +nightly install cargo-udeps --locked").run()?;
                }
                eprintln!("Running cargo udeps...");
                cmd!(sh, "cargo +nightly udeps --workspace --all-targets").run()?;
                Ok(())
            }
            CiCommand::Test(test_args) => {
                eprintln!("Running cargo test...");
                let args = &test_args.args;
                cmd!(sh, "cargo test {args...}").run()?;
                Ok(())
            }
        }
    }
}
