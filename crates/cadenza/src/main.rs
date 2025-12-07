//! Cadenza CLI - A unified command-line interface for the Cadenza language toolchain.
//!
//! This binary provides various commands for working with Cadenza, including:
//! - `repl`: Start an interactive REPL with history, syntax highlighting, and auto-completion
//! - `lsp`: Start a Language Server Protocol server for editor integration

mod lsp;
mod repl;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "cadenza")]
#[command(about = "Cadenza language toolchain", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start an interactive REPL (Read-Eval-Print Loop)
    Repl {
        /// Load a Cadenza file into the REPL scope before starting
        #[arg(short, long, value_name = "FILE")]
        load: Option<PathBuf>,
    },
    /// Start the Language Server Protocol server
    Lsp,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Repl { load } => {
            repl::start_repl(load)?;
        }
        Commands::Lsp => {
            lsp::start_server().await?;
        }
    }

    Ok(())
}
