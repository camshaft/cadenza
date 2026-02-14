use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use core::fmt;
use rayon::prelude::*;
use std::{
    collections::HashSet,
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use xshell::{Shell, cmd};

#[derive(Args)]
pub struct K {
    #[command(subcommand)]
    command: KCommand,
}

#[derive(Clone, Copy, Debug, Default, clap::ValueEnum)]
pub enum Accept {
    All,
    Changed,
    #[default]
    None,
}

impl fmt::Display for Accept {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Accept::All => "all",
            Accept::Changed => "changed",
            Accept::None => "none",
        }
        .fmt(f)
    }
}

#[derive(Subcommand)]
pub enum KCommand {
    /// Compile the K definition
    Kompile,
    /// Run K framework tests
    Test {
        /// Accept new snapshots (copy .snap.new to .snap)
        #[arg(long, default_value_t = Accept::default())]
        accept: Accept,
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

fn test(sh: &Shell, patterns: HashSet<&str>, accept: Accept) -> Result<()> {
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

    // Collect and sort test files
    let mut test_files: Vec<_> = sh
        .read_dir(&test_data_dir)?
        .into_iter()
        .filter(|e| e.extension().is_some_and(|ext| ext == "cdz"))
        .collect();
    test_files.sort();

    // Filter by pattern and collect test info
    let tests_to_run: Vec<_> = test_files
        .into_iter()
        .filter_map(|entry| {
            let basename = entry.file_stem().unwrap().to_string_lossy().to_string();
            if !patterns.is_empty() && !patterns.iter().any(|p| basename.contains(p)) {
                return None;
            }
            Some((entry, basename))
        })
        .collect();

    // Thread-safe counters
    let total = tests_to_run.len();
    let passed = Arc::new(Mutex::new(0usize));
    let failed = Arc::new(Mutex::new(0usize));
    let needs_review = Arc::new(Mutex::new(0usize));
    let errors = Arc::new(Mutex::new(0usize));

    // Run tests in parallel
    tests_to_run.par_iter().for_each(|(entry, basename)| {
        let result = run_single_test(
            &cadenza_bin,
            &output_dir,
            &test_data_dir,
            &output_test_dir,
            &snapshot_dir,
            &repo_root,
            entry,
            basename,
            accept,
        );

        match result {
            Ok(TestResult::Pass) => {
                println!("✓ {}", basename);
                *passed.lock().unwrap() += 1;
            }
            Ok(TestResult::Fail) => {
                println!("✗ {} (output changed, see .snap.new)", basename);
                *failed.lock().unwrap() += 1;
            }
            Ok(TestResult::NeedsReview) => match accept {
                Accept::None => {
                    println!("? {} (new test, needs review)", basename);
                    *needs_review.lock().unwrap() += 1;
                }
                Accept::All | Accept::Changed => {
                    println!("✓ {} (accepted)", basename);
                    *passed.lock().unwrap() += 1;
                }
            },
            Ok(TestResult::Error) | Err(_) => {
                *errors.lock().unwrap() += 1;
            }
        }
    });

    let passed = *passed.lock().unwrap();
    let failed = *failed.lock().unwrap();
    let needs_review = *needs_review.lock().unwrap();
    let errors = *errors.lock().unwrap();

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

fn run_single_test(
    cadenza_bin: &PathBuf,
    output_dir: &PathBuf,
    test_data_dir: &PathBuf,
    output_test_dir: &PathBuf,
    snapshot_dir: &PathBuf,
    repo_root: &PathBuf,
    entry: &PathBuf,
    basename: &str,
    accept: Accept,
) -> Result<TestResult> {
    let expected_file = test_data_dir.join(format!("{}.expected", basename));
    let snapshot_file = snapshot_dir.join(format!("{}.snap", basename));
    let snapshot_new_file = snapshot_dir.join(format!("{}.snap.new", basename));

    // Read the input file content
    let input_content = fs::read_to_string(entry)?;

    // Convert .cdz to AST and write to file
    let ast_file = output_test_dir.join(format!("{}.ast", basename));
    let ast_output = std::process::Command::new(cadenza_bin)
        .arg("ast")
        .arg(entry)
        .output()
        .context("Failed to run cadenza ast")?;

    let ast_content = if ast_output.status.success() {
        let content = String::from_utf8_lossy(&ast_output.stdout).to_string();
        fs::write(&ast_file, &content)?;
        content
    } else {
        println!("✗ {} (AST conversion failed)", basename);
        write_snapshot_file(
            &snapshot_new_file,
            basename,
            &input_content,
            None,
            None,
            None,
            Some(&format!("AST conversion failed: {:?}", ast_output.stderr)),
        )?;
        return Ok(TestResult::Error);
    };

    // Run through K interpreter with --search-final for better performance
    let k_output = std::process::Command::new("krun")
        .arg(&ast_file)
        .arg("-d")
        .arg(output_dir)
        .arg("--search-final")
        .output()
        .context("Failed to run krun")?;

    let k_result = if k_output.status.success() {
        String::from_utf8_lossy(&k_output.stdout).to_string()
    } else {
        println!("✗ {} (K execution failed)", basename);
        write_snapshot_file(
            &snapshot_new_file,
            basename,
            &input_content,
            Some(&ast_content),
            None,
            None,
            Some(&format!("K execution failed: {:?}", k_output.stderr)),
        )?;
        return Ok(TestResult::Error);
    };

    // Sanitize paths in K output (remove absolute paths)
    let k_result = sanitize_k_output(&k_result, repo_root);

    // Read expected output if it exists
    let expected = if expected_file.exists() {
        Some(fs::read_to_string(&expected_file)?)
    } else {
        None
    };

    // Write .snap.new file
    write_snapshot_file(
        &snapshot_new_file,
        basename,
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

    // Handle accept modes
    match accept {
        Accept::All => {
            // Accept both new and changed snapshots
            if result == TestResult::NeedsReview || result == TestResult::Fail {
                fs::copy(&snapshot_new_file, &snapshot_file)?;
                fs::remove_file(&snapshot_new_file)?;
                return Ok(TestResult::NeedsReview); // Return NeedsReview to signal it was accepted
            }
        }
        Accept::Changed => {
            // Accept only changed snapshots, not new ones
            if result == TestResult::Fail {
                fs::copy(&snapshot_new_file, &snapshot_file)?;
                fs::remove_file(&snapshot_new_file)?;
                return Ok(TestResult::NeedsReview); // Return NeedsReview to signal it was accepted
            }
        }
        Accept::None => {
            // Don't accept anything - fall through
        }
    }

    Ok(result)
}

/// Sanitize K output by removing absolute paths and search-final wrapper
fn sanitize_k_output(output: &str, repo_root: &std::path::Path) -> String {
    let repo_str = repo_root.to_string_lossy();
    let mut sanitized = output.replace(&*repo_str, "<repo>");

    // Strip search-final wrapper if present
    // Format: { Result:GeneratedTopCell #Equals <generatedTop>...</generatedTop> }
    if sanitized.starts_with('{') && sanitized.contains("#Equals") {
        // Find the start of <generatedTop>
        let start_tag = "<generatedTop>";
        if let Some(start) = sanitized.find(start_tag) {
            let substr = &sanitized[start + start_tag.len()..];
            // Find the end of </generatedTop>
            if let Some(end) = substr.rfind("</generatedTop>") {
                sanitized = substr[..end].to_string();
            }
        }
    }

    // Dedent the output - remove common leading whitespace
    dedent(&sanitized)
}

/// Remove common leading whitespace from all lines
fn dedent(s: &str) -> String {
    let lines: Vec<&str> = s.lines().collect();
    if lines.is_empty() {
        return s.to_string();
    }

    // Find minimum indentation (excluding empty lines and root element tags)
    let min_indent = lines
        .iter()
        .enumerate()
        .filter(|(i, line)| {
            !line.trim().is_empty()
                && *i != 0  // Skip first line (opening tag)
                && *i != lines.len() - 1 // Skip last line (closing tag)
        })
        .map(|(_, line)| line.chars().take_while(|c| c.is_whitespace()).count())
        .min()
        .unwrap_or(0);

    // Remove that amount of whitespace from each line
    lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            if line.trim().is_empty() {
                ""
            } else if i == 0 {
                // Opening tag - keep as is (should have no indentation)
                line
            } else if i == lines.len() - 1 {
                // Closing tag - strip all leading whitespace
                line.trim_start()
            } else {
                // Content lines - dedent by min_indent
                &line[min_indent.min(line.len())..]
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
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
    cmd!(sh, "krun {ast_file} -d {output_dir} --search-final")
        .run()
        .context("Failed to run through K")?;

    Ok(())
}
