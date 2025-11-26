use anyhow::Result;
use clap::Args;
use xshell::{Shell, cmd};

#[derive(Args)]
pub struct Test {
    #[arg(long, default_value = "dev")]
    profile: String,
}

impl Test {
    pub fn run(&self, sh: &Shell) -> Result<()> {
        // Run Rust tests
        let cargo = cmd!(sh, "cargo test").arg("--profile").arg(&self.profile);
        cargo.run()?;

        // Run TypeScript tests
        if sh.path_exists("tools/dashboard/deno.json") {
            let _dir = sh.push_dir("tools/dashboard");
            cmd!(sh, "deno task test").run()?;
        }

        // Run Python tests
        if sh.path_exists("clients/python") {
            let _dir = sh.push_dir("clients/python");
            cmd!(sh, "uv run pytest").run()?;
        }

        Ok(())
    }
}
