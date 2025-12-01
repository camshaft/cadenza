use anyhow::Result;
use clap::{Args, Subcommand};
use xshell::{Shell, cmd};
use std::path::Path;

#[derive(Args)]
pub struct Explorer {
    #[command(subcommand)]
    command: ExplorerCommand,
}

#[derive(Subcommand)]
pub enum ExplorerCommand {
    /// Start the development server with hot reloading
    Dev,
    /// Build the explorer for production deployment
    Build,
}

impl Explorer {
    pub fn run(&self, sh: &Shell) -> Result<()> {
        self.command.run(sh)
    }
}

impl ExplorerCommand {
    pub fn run(&self, sh: &Shell) -> Result<()> {
        match self {
            ExplorerCommand::Dev => run_dev(sh),
            ExplorerCommand::Build => run_build(sh),
        }
    }
}

fn ensure_wasm_pack(sh: &Shell) -> Result<()> {
    if cmd!(sh, "wasm-pack --version").quiet().run().is_err() {
        eprintln!("Installing wasm-pack...");
        cmd!(sh, "cargo install wasm-pack --locked").run()?;
    }
    Ok(())
}

fn build_wasm(sh: &Shell, release: bool) -> Result<()> {
    ensure_wasm_pack(sh)?;
    
    let web_crate = Path::new("crates/cadenza-web");
    let _dir = sh.push_dir(web_crate);
    
    eprintln!("Building WASM module...");
    if release {
        cmd!(sh, "wasm-pack build --target web --out-dir app/pkg").run()?;
    } else {
        cmd!(sh, "wasm-pack build --target web --out-dir app/pkg --dev").run()?;
    }
    
    Ok(())
}

fn ensure_npm_deps(sh: &Shell) -> Result<()> {
    let app_dir = Path::new("crates/cadenza-web/app");
    let _dir = sh.push_dir(app_dir);
    
    // Check if node_modules exists
    if !sh.path_exists("node_modules") {
        eprintln!("Installing npm dependencies...");
        cmd!(sh, "npm install").run()?;
    }
    
    Ok(())
}

fn run_dev(sh: &Shell) -> Result<()> {
    // Build WASM in dev mode first
    build_wasm(sh, false)?;
    
    // Ensure npm deps
    ensure_npm_deps(sh)?;
    
    let app_dir = Path::new("crates/cadenza-web/app");
    let _dir = sh.push_dir(app_dir);
    
    eprintln!("Starting development server...");
    eprintln!("Note: For Rust file watching, run `cargo watch -w ../src -s 'wasm-pack build --target web --out-dir app/pkg --dev'` in another terminal");
    
    // Start vite dev server
    cmd!(sh, "npm run dev").run()?;
    
    Ok(())
}

fn run_build(sh: &Shell) -> Result<()> {
    // Build WASM in release mode
    build_wasm(sh, true)?;
    
    // Ensure npm deps
    ensure_npm_deps(sh)?;
    
    let app_dir = Path::new("crates/cadenza-web/app");
    let _dir = sh.push_dir(app_dir);
    
    eprintln!("Building production bundle...");
    cmd!(sh, "npm run build").run()?;
    
    eprintln!("Build complete! Output in crates/cadenza-web/app/dist/");
    
    Ok(())
}
