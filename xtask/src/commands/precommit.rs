use anyhow::Result;
use clap::Args;
use xshell::{Shell, cmd};

#[derive(Args)]
pub struct Precommit;

impl Precommit {
    pub fn run(&self, sh: &Shell) -> Result<()> {
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

        // Apply rustfmt
        eprintln!("Applying cargo fmt...");
        cmd!(sh, "cargo +nightly fmt --all").run()?;

        // Run clippy
        eprintln!("Running cargo clippy...");
        cmd!(
            sh,
            "cargo clippy --all-features --all-targets --workspace -- -D warnings"
        )
        .run()?;

        eprintln!("Precommit checks passed!");
        Ok(())
    }
}
