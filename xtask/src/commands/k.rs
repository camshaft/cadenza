use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use std::path::PathBuf;
use xshell::{cmd, Shell};

#[derive(Args)]
pub struct K {
    #[command(subcommand)]
    command: KCommand,
}

#[derive(Subcommand)]
pub enum KCommand {
    /// Compile the K definition
    Kompile,
    /// Run K framework tests
    Test,
    /// Run a single Cadenza file through K
    Run {
        /// Path to the .cdz file to run
        file: PathBuf,
    },
}

impl K {
    pub fn run(&self, sh: &Shell) -> Result<()> {
        self.command.run(sh)
    }
}

impl KCommand {
    pub fn run(&self, sh: &Shell) -> Result<()> {
        match self {
            KCommand::Kompile => kompile(sh),
            KCommand::Test => test(sh),
            KCommand::Run { file } => run_single(sh, file),
        }
    }
}

fn kompile(sh: &Shell) -> Result<()> {
    let k_dir = PathBuf::from("reference/k");
    let output_dir = PathBuf::from("target/k");

    // Check if K framework is installed
    if cmd!(sh, "which kompile").quiet().run().is_err() {
        anyhow::bail!(
            "K framework not found. Please install K framework.\n\
             See reference/k/README.md for installation instructions."
        );
    }

    println!("Compiling K definition...");
    println!("Input:  {}/cadenza.k", k_dir.display());
    println!("Output: {}", output_dir.display());
    println!();

    let _pwd = sh.push_dir(&k_dir);
    
    // Create output directory
    sh.create_dir(&output_dir)?;

    // Compile K definition
    cmd!(sh, "kompile cadenza.k --directory ../../{output_dir} --backend llvm")
        .run()
        .context("Failed to compile K definition")?;

    println!();
    println!("✓ K definition compiled successfully");

    Ok(())
}

fn test(sh: &Shell) -> Result<()> {
    // First, extract semantics tests
    println!("Extracting semantics tests...");
    cmd!(sh, "cargo xtask semantics extract")
        .run()
        .context("Failed to extract semantics tests")?;
    println!();

    // Ensure K definition is compiled
    let output_dir = PathBuf::from("target/k");
    if !output_dir.join("cadenza-kompiled").exists() {
        println!("K definition not compiled. Compiling...");
        kompile(sh)?;
        println!();
    }

    // Build cadenza CLI if needed
    println!("Building Cadenza CLI...");
    cmd!(sh, "cargo build --bin cadenza")
        .run()
        .context("Failed to build Cadenza CLI")?;
    println!();

    let test_data_dir = PathBuf::from("crates/cadenza-compiler/test-data/semantics");
    let output_test_dir = output_dir.join("tests");
    sh.create_dir(&output_test_dir)?;

    println!("Running K framework tests...");
    println!("================================");
    println!();

    let mut total = 0;
    let passed = 0;
    let mut failed = 0;
    let mut not_impl = 0;

    // Iterate through test files
    for entry in sh.read_dir(&test_data_dir)? {
        if !entry.extension().map_or(false, |ext| ext == "cdz") {
            continue;
        }

        total += 1;
        let basename = entry.file_stem().unwrap().to_string_lossy();
        let expected_file = test_data_dir.join(format!("{}.expected", basename));

        if !expected_file.exists() {
            println!("⊘ {} (no expected file)", basename);
            not_impl += 1;
            continue;
        }

        // Convert .cdz to AST
        let ast_file = output_test_dir.join(format!("{}.ast", basename));
        if cmd!(sh, "cargo run --bin cadenza ast {entry}")
            .quiet()
            .ignore_stdout()
            .ignore_stderr()
            .run()
            .is_err()
        {
            println!("✗ {} (AST conversion failed)", basename);
            failed += 1;
            continue;
        }

        // Run through K interpreter (output not used for now)
        let _output_file = output_test_dir.join(format!("{}.out", basename));
        if cmd!(sh, "krun {ast_file} --directory target/k")
            .quiet()
            .ignore_stdout()
            .ignore_stderr()
            .run()
            .is_err()
        {
            println!("✗ {} (K execution failed)", basename);
            failed += 1;
            continue;
        }

        // For now, mark as not implemented since we're setting up infrastructure
        println!("⊘ {} (not yet implemented)", basename);
        not_impl += 1;
    }

    println!();
    println!("================================");
    println!("Test Results:");
    println!("  Total:  {}", total);
    println!("  Passed: {}", passed);
    println!("  Failed: {}", failed);
    println!("  Not Implemented: {}", not_impl);
    println!();

    Ok(())
}

fn run_single(sh: &Shell, file: &PathBuf) -> Result<()> {
    // Check if K framework is installed
    if cmd!(sh, "which krun").quiet().run().is_err() {
        anyhow::bail!(
            "K framework not found. Please install K framework.\n\
             See reference/k/README.md for installation instructions."
        );
    }

    // Ensure K definition is compiled
    let output_dir = PathBuf::from("target/k");
    if !output_dir.join("cadenza-kompiled").exists() {
        println!("K definition not compiled. Compiling...");
        kompile(sh)?;
        println!();
    }

    // Build cadenza CLI if needed
    if cmd!(sh, "cargo build --bin cadenza").quiet().run().is_err() {
        anyhow::bail!("Failed to build Cadenza CLI");
    }

    let ast_file = output_dir.join("temp.ast");

    println!("Converting {} to AST...", file.display());
    cmd!(sh, "cargo run --bin cadenza ast {file}")
        .read()
        .context("Failed to convert to AST")?;

    let ast_content = cmd!(sh, "cargo run --bin cadenza ast {file}")
        .read()
        .context("Failed to convert to AST")?;
    
    println!("AST:");
    println!("{}", ast_content);
    println!();

    sh.write_file(&ast_file, &ast_content)?;

    println!("Running through K interpreter...");
    cmd!(sh, "krun {ast_file} --directory target/k")
        .run()
        .context("Failed to run through K")?;

    Ok(())
}
