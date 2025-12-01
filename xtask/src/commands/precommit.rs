use anyhow::Result;
use clap::Args;
use xshell::Shell;

use super::common;

#[derive(Args)]
pub struct Precommit;

impl Precommit {
    pub fn run(&self, sh: &Shell) -> Result<()> {
        // Check rustfmt (does not modify files)
        common::run_fmt_check(sh)?;

        // Run clippy
        common::run_clippy(sh)?;

        eprintln!("Precommit checks passed!");
        Ok(())
    }
}
