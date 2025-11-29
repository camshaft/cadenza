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
            CiCommand::Fmt => {
                // Check if nightly rustfmt is available, install if not
                if cmd!(sh, "cargo +nightly fmt --version")
                    .quiet()
                    .run()
                    .is_err()
                {
                    eprintln!("Installing nightly rustfmt...");
                    cmd!(
                        sh,
                        "rustup toolchain install nightly --profile minimal --component rustfmt"
                    )
                    .run()?;
                }
                eprintln!("Running cargo fmt check...");
                cmd!(sh, "cargo +nightly fmt --all -- --check").run()?;
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
                // Check if nightly is available, install if not
                if cmd!(sh, "cargo +nightly --version").quiet().run().is_err() {
                    eprintln!("Installing nightly toolchain...");
                    cmd!(sh, "rustup toolchain install nightly --profile minimal").run()?;
                }
                // Check if cargo-udeps is available, install if not
                if cmd!(sh, "cargo +nightly udeps --version")
                    .quiet()
                    .run()
                    .is_err()
                {
                    eprintln!("Installing cargo-udeps...");
                    cmd!(sh, "cargo install cargo-udeps --locked").run()?;
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
