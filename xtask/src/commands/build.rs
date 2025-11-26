use anyhow::Result;
use clap::Args;
use xshell::{Shell, cmd};

#[derive(Args)]
pub struct Build {
    #[arg(long, default_value = "dev")]
    profile: String,
}

impl Build {
    pub fn run(&self, sh: &Shell) -> Result<()> {
        // Build Rust components
        let cargo = cmd!(sh, "cargo build").arg("--profile").arg(&self.profile);
        cargo.run()?;

        // Build TypeScript components
        if sh.path_exists("tools/dashboard/deno.json") {
            let _dir = sh.push_dir("tools/dashboard");
            cmd!(sh, "deno task build").run()?;
        }

        Ok(())
    }
}
