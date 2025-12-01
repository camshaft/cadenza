use anyhow::Result;
use xshell::{Shell, cmd};

/// Ensures nightly rustfmt is available, installing if necessary
pub fn ensure_nightly_rustfmt(sh: &Shell) -> Result<()> {
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
    Ok(())
}

/// Run rustfmt check (does not modify files)
pub fn run_fmt_check(sh: &Shell) -> Result<()> {
    ensure_nightly_rustfmt(sh)?;
    eprintln!("Running cargo fmt check...");
    cmd!(sh, "cargo +nightly fmt --all -- --check").run()?;
    Ok(())
}

/// Apply rustfmt to all files
pub fn run_fmt(sh: &Shell) -> Result<()> {
    ensure_nightly_rustfmt(sh)?;
    eprintln!("Applying cargo fmt...");
    cmd!(sh, "cargo +nightly fmt --all").run()?;
    Ok(())
}

/// Run clippy with all warnings treated as errors
pub fn run_clippy(sh: &Shell) -> Result<()> {
    eprintln!("Running cargo clippy...");
    cmd!(
        sh,
        "cargo clippy --all-features --all-targets --workspace -- -D warnings"
    )
    .run()?;
    Ok(())
}
