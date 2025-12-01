use anyhow::Result;
use clap::{Args, Subcommand};
use xshell::Shell;

#[derive(Args)]
pub struct Hooks {
    #[command(subcommand)]
    command: HooksCommand,
}

#[derive(Subcommand)]
pub enum HooksCommand {
    /// Install git hooks
    Install,
}

impl Hooks {
    pub fn run(&self, sh: &Shell) -> Result<()> {
        match &self.command {
            HooksCommand::Install => {
                let hooks_src = sh.current_dir().join("hooks");
                let hooks_dst = sh.current_dir().join(".git/hooks");

                if !hooks_src.exists() {
                    anyhow::bail!("hooks directory not found. Are you in the repository root?");
                }

                if !hooks_dst.exists() {
                    anyhow::bail!(".git/hooks directory not found. Is this a git repository?");
                }

                // Copy pre-commit hook
                let pre_commit_src = hooks_src.join("pre-commit");
                let pre_commit_dst = hooks_dst.join("pre-commit");

                if pre_commit_src.exists() {
                    eprintln!("Installing pre-commit hook...");
                    std::fs::copy(&pre_commit_src, &pre_commit_dst)?;

                    // Make the hook executable on Unix
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        let mut perms = std::fs::metadata(&pre_commit_dst)?.permissions();
                        perms.set_mode(0o755);
                        std::fs::set_permissions(&pre_commit_dst, perms)?;
                    }

                    eprintln!("Pre-commit hook installed to .git/hooks/pre-commit");
                } else {
                    eprintln!("No pre-commit hook found in hooks directory");
                }

                eprintln!("Git hooks installed successfully!");
                Ok(())
            }
        }
    }
}
