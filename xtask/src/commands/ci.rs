use anyhow::Result;
use clap::{Args, Subcommand};
use xshell::{Shell, cmd};

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

#[derive(Args)]
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
                CiCommand::Test(TestArgs { args: vec![] }).run(sh)?;
                Ok(())
            }
        }
    }
}

impl CiCommand {
    pub fn run(&self, sh: &Shell) -> Result<()> {
        match self {
            CiCommand::Fmt => {
                eprintln!("Running cargo fmt check...");
                cmd!(sh, "cargo fmt --all -- --check").run()?;
                Ok(())
            }
            CiCommand::Clippy => {
                eprintln!("Running cargo clippy...");
                cmd!(
                    sh,
                    "cargo clippy --all-features --all-targets --workspace -- -D warnings"
                )
                .run()?;
                Ok(())
            }
            CiCommand::Udeps => {
                eprintln!("Running cargo udeps...");
                cmd!(sh, "cargo udeps --workspace --all-targets").run()?;
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
