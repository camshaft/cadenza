use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use std::{collections::HashSet, fs, path::PathBuf};
use xshell::{Shell, cmd};

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
    Test {
        /// Accept new snapshots (copy .snap.new to .snap)
        #[arg(long)]
        accept: bool,
        /// Optional pattern to filter test names (e.g., "function" or "closure")
        patterns: Vec<String>,
    },
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
            KCommand::Test { patterns, accept } => {
                test(sh, patterns.iter().map(|v| v.as_str()).collect(), *accept)
            }
            KCommand::Run { file } => run_single(sh, file),
        }
    }
}

fn kompile(sh: &Shell) -> Result<()> {
    let k_dir = PathBuf::from("reference/k");
    // Use absolute path for output directory to avoid issues with push_dir
    let repo_root = sh.current_dir();
    let output_dir = repo_root.join("target/k");

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

    // Create output directory before changing directories
    fs::create_dir_all(&output_dir)?;

    let _pwd = sh.push_dir(&k_dir);

    // Compile K definition using absolute path
    // Using Haskell backend for better portability
    cmd!(sh, "kompile cadenza.k -o {output_dir} --backend haskell")
        .run()
        .context("Failed to compile K definition")?;

    println!();
    println!("✓ K definition compiled successfully");

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TestResult {
    Pass,
    Fail,
    NeedsReview,
    Error,
}

fn test(sh: &Shell, patterns: HashSet<&str>, accept: bool) -> Result<()> {
    // First, extract semantics tests
    println!("Extracting semantics tests...");
    cmd!(sh, "cargo xtask semantics extract")
        .run()
        .context("Failed to extract semantics tests")?;
    println!();

    // Ensure K definition is compiled
    let repo_root = sh.current_dir();
    let output_dir = repo_root.join("target/k");
    if !output_dir.join("cadenza-kompiled").exists() {
        println!("K definition not compiled. Compiling...");
        kompile(sh)?;
        println!();
    }

    // Build cadenza CLI once
    println!("Building Cadenza CLI...");
    cmd!(sh, "cargo build --bin cadenza")
        .run()
        .context("Failed to build Cadenza CLI")?;
    println!();

    let cadenza_bin = repo_root.join("target/debug/cadenza");

    let test_data_dir = PathBuf::from("crates/cadenza-compiler/test-data/semantics");
    let output_test_dir = output_dir.join("tests");
    let snapshot_dir = repo_root.join("reference/k/snapshots");
    fs::create_dir_all(&output_test_dir)?;
    fs::create_dir_all(&snapshot_dir)?;

    if !patterns.is_empty() {
        println!(
            "Running K framework tests matching {}...",
            patterns.iter().copied().collect::<Vec<_>>().join(", ")
        );
    } else {
        println!("Running K framework tests...");
    }
    println!("================================");
    println!();

    let mut total = 0;
    let mut passed = 0;
    let mut failed = 0;
    let mut needs_review = 0;
    let mut errors = 0;

    // Collect and sort test files
    let mut test_files: Vec<_> = sh
        .read_dir(&test_data_dir)?
        .into_iter()
        .filter(|e| e.extension().is_some_and(|ext| ext == "cdz"))
        .collect();
    test_files.sort();

    // Iterate through test files
    for entry in test_files {
        let basename = entry.file_stem().unwrap().to_string_lossy().to_string();

        // Filter by pattern if provided
        if !patterns.is_empty() && !patterns.iter().any(|p| basename.contains(p)) {
            continue;
        }

        total += 1;
        let expected_file = test_data_dir.join(format!("{}.expected", basename));
        let snapshot_file = snapshot_dir.join(format!("{}.snap", basename));
        let snapshot_new_file = snapshot_dir.join(format!("{}.snap.new", basename));

        // Read the input file content
        let input_content = fs::read_to_string(&entry)?;

        // Convert .cdz to AST and write to file
        let ast_file = output_test_dir.join(format!("{}.ast", basename));
        let ast_output = cmd!(sh, "{cadenza_bin} ast {entry}")
            .quiet()
            .ignore_stderr()
            .read();

        let ast_content = match ast_output {
            Ok(content) => {
                sh.write_file(&ast_file, &content)?;
                content
            }
            Err(e) => {
                println!("✗ {} (AST conversion failed)", basename);
                write_snapshot_file(
                    &snapshot_new_file,
                    &basename,
                    &input_content,
                    None,
                    None,
                    None,
                    Some(&format!("AST conversion failed: {:?}", e)),
                )?;
                errors += 1;
                continue;
            }
        };

        // Run through K interpreter
        let k_output = cmd!(sh, "krun {ast_file} -d {output_dir}")
            .quiet()
            .ignore_stderr()
            .read();

        let k_result = match k_output {
            Ok(output) => output,
            Err(e) => {
                println!("✗ {} (K execution failed)", basename);
                write_snapshot_file(
                    &snapshot_new_file,
                    &basename,
                    &input_content,
                    Some(&ast_content),
                    None,
                    None,
                    Some(&format!("K execution failed: {:?}", e)),
                )?;
                errors += 1;
                continue;
            }
        };

        // Sanitize paths in K output (remove absolute paths)
        let k_result = sanitize_k_output(&k_result, &repo_root);

        // Read expected output if it exists
        let expected = if expected_file.exists() {
            Some(fs::read_to_string(&expected_file)?)
        } else {
            None
        };

        // Write .snap.new file
        write_snapshot_file(
            &snapshot_new_file,
            &basename,
            &input_content,
            Some(&ast_content),
            Some(&k_result),
            expected.as_deref(),
            None,
        )?;

        // Compare with existing .snap file to determine result
        let result = if snapshot_file.exists() {
            let existing_snap = fs::read_to_string(&snapshot_file)?;
            let new_snap = fs::read_to_string(&snapshot_new_file)?;
            if existing_snap == new_snap {
                // Exact match - test passes, remove .snap.new
                fs::remove_file(&snapshot_new_file)?;
                TestResult::Pass
            } else {
                TestResult::Fail
            }
        } else {
            // No existing snapshot - needs review
            TestResult::NeedsReview
        };

        // If accept mode, copy .snap.new to .snap
        if accept && (result == TestResult::NeedsReview || result == TestResult::Fail) {
            fs::copy(&snapshot_new_file, &snapshot_file)?;
            fs::remove_file(&snapshot_new_file)?;
            println!("✓ {} (accepted)", basename);
            passed += 1;
        } else {
            match result {
                TestResult::Pass => {
                    println!("✓ {}", basename);
                    passed += 1;
                }
                TestResult::Fail => {
                    println!("✗ {} (output changed, see .snap.new)", basename);
                    failed += 1;
                }
                TestResult::NeedsReview => {
                    println!("? {} (new test, needs review)", basename);
                    needs_review += 1;
                }
                TestResult::Error => {
                    errors += 1;
                }
            }
        }
    }

    println!();
    println!("================================");
    println!("Test Results:");
    println!("  Total:        {}", total);
    println!("  Passed:       {}", passed);
    println!("  Failed:       {}", failed);
    println!("  Needs Review: {}", needs_review);
    println!("  Errors:       {}", errors);
    println!();
    println!("Snapshots directory: {}", snapshot_dir.display());

    if needs_review > 0 {
        println!();
        println!(
            "To accept new snapshots, run: cargo xtask k test --accept{}",
            patterns
                .iter()
                .map(|p| format!(" {p}"))
                .collect::<Vec<_>>()
                .join("")
        );
    }

    if failed > 0 {
        anyhow::bail!(
            "K framework tests failed: {failed} test(s) have different output than expected"
        );
    }

    Ok(())
}

/// Sanitize K output by removing absolute paths and replacing with relative paths
fn sanitize_k_output(output: &str, repo_root: &std::path::Path) -> String {
    let repo_str = repo_root.to_string_lossy();
    // Replace full repo path with empty string to leave just the relative path
    // Need to also remove the trailing slash to avoid double slashes
    let repo_with_slash = format!("{}/", repo_str);
    output.replace(&*repo_with_slash, "")
}

fn write_snapshot_file(
    path: &PathBuf,
    name: &str,
    input: &str,
    ast: Option<&str>,
    k_output: Option<&str>,
    expected: Option<&str>,
    error: Option<&str>,
) -> Result<()> {
    use std::fmt::Write;

    let mut content = String::new();
    writeln!(content, "---")?;
    writeln!(
        content,
        "source: crates/cadenza-compiler/test-data/semantics/{}.cdz",
        name
    )?;
    writeln!(content, "---")?;
    writeln!(content, "# {}", name)?;
    writeln!(content)?;

    // Input source code
    writeln!(content, "## Input")?;
    writeln!(content)?;
    writeln!(content, "```cadenza")?;
    writeln!(content, "{}", input.trim())?;
    writeln!(content, "```")?;
    writeln!(content)?;

    if let Some(a) = ast {
        writeln!(content, "## AST")?;
        writeln!(content)?;
        writeln!(content, "```lisp")?;
        writeln!(content, "{}", a.trim())?;
        writeln!(content, "```")?;
        writeln!(content)?;
    }

    if let Some(exp) = expected {
        writeln!(content, "## Expected")?;
        writeln!(content)?;
        writeln!(content, "```")?;
        writeln!(content, "{}", exp.trim())?;
        writeln!(content, "```")?;
        writeln!(content)?;
    }

    if let Some(err) = error {
        writeln!(content, "## Error")?;
        writeln!(content)?;
        writeln!(content, "{}", err)?;
        writeln!(content)?;
    }

    if let Some(k) = k_output {
        writeln!(content, "## K Output")?;
        writeln!(content)?;
        writeln!(content, "```")?;
        writeln!(content, "{}", k.trim())?;
        writeln!(content, "```")?;
        writeln!(content)?;
    }

    fs::write(path, content)?;
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
    let repo_root = sh.current_dir();
    let output_dir = repo_root.join("target/k");
    if !output_dir.join("cadenza-kompiled").exists() {
        println!("K definition not compiled. Compiling...");
        kompile(sh)?;
        println!();
    }

    // Build cadenza CLI if needed
    if cmd!(sh, "cargo build --bin cadenza").quiet().run().is_err() {
        anyhow::bail!("Failed to build Cadenza CLI");
    }

    let cadenza_bin = repo_root.join("target/debug/cadenza");
    let ast_file = output_dir.join("temp.ast");

    println!("Converting {} to AST...", file.display());
    let ast_content = cmd!(sh, "{cadenza_bin} ast {file}")
        .read()
        .context("Failed to convert to AST")?;

    println!("AST:");
    println!("{}", ast_content);
    println!();

    sh.write_file(&ast_file, &ast_content)?;

    println!("Running through K interpreter...");
    cmd!(sh, "krun {ast_file} -d {output_dir}")
        .run()
        .context("Failed to run through K")?;

    Ok(())
}
