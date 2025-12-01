use anyhow::Result;
use clap::Args;
use xshell::Shell;

use super::common;

#[derive(Args)]
pub struct Fmt;

impl Fmt {
    pub fn run(&self, sh: &Shell) -> Result<()> {
        // Apply rustfmt to all files
        common::run_fmt(sh)?;
        Ok(())
    }
}
