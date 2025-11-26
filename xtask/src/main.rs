use anyhow::Result;
use clap::Parser;
use xshell::Shell;

mod commands;

#[derive(Parser)]
#[command(name = "xtask")]
struct Cli {
    #[command(subcommand)]
    command: commands::Command,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let sh = Shell::new()?;

    cli.command.run(&sh)
}
