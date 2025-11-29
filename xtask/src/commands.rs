use anyhow::Result;
use clap::Subcommand;
use xshell::Shell;

pub mod build;
pub mod ci;
pub mod test;

#[derive(Subcommand)]
pub enum Command {
    /// Build all components
    Build(build::Build),
    /// Run CI checks (fmt, clippy, udeps, test). Runs all if no subcommand specified.
    Ci(ci::Ci),
    /// Run tests
    Test(test::Test),
}

impl Command {
    pub fn run(self, sh: &Shell) -> Result<()> {
        match self {
            Command::Build(cmd) => cmd.run(sh),
            Command::Ci(cmd) => cmd.run(sh),
            Command::Test(cmd) => cmd.run(sh),
        }
    }
}
