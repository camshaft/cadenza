//! Cadenza CLI - A unified command-line interface for the Cadenza language toolchain.
//!
//! This binary provides various commands for working with Cadenza, including:
//! - `lsp`: Start a Language Server Protocol server for editor integration

mod lsp;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "cadenza")]
#[command(about = "Cadenza language toolchain", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
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
        Commands::Lsp => {
            lsp::start_server().await?;
        }
    }

    Ok(())
}
