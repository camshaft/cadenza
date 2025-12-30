use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use std::path::{Path, PathBuf};
use xshell::Shell;

#[derive(Args)]
pub struct Semantics {
    #[command(subcommand)]
    command: SemanticsCommand,
}

#[derive(Subcommand)]
pub enum SemanticsCommand {
    /// Extract test cases from semantics markdown files
    Extract,
    /// Generate a progress report showing test status
    Report,
}

impl Semantics {
    pub fn run(&self, sh: &Shell) -> Result<()> {
        self.command.run(sh)
    }
}

impl SemanticsCommand {
    pub fn run(&self, sh: &Shell) -> Result<()> {
        match self {
            SemanticsCommand::Extract => extract_tests(sh),
            SemanticsCommand::Report => generate_report(sh),
        }
    }
}

fn extract_tests(sh: &Shell) -> Result<()> {
    let semantics_dir = PathBuf::from("docs/semantics");
    let output_dir = PathBuf::from("crates/cadenza-compiler/test-data/semantics");

    // Create output directory
    sh.create_dir(&output_dir)?;

    println!("Extracting tests from semantics documentation...");
    println!("Output directory: {}", output_dir.display());
    println!();

    let mut total_tests = 0;

    // Process each markdown file
    for entry in sh.read_dir(&semantics_dir)? {
        let path = entry;

        // Skip non-markdown files and README
        if path.extension().is_none_or(|ext| ext != "md") {
            continue;
        }
        if path.file_name().is_some_and(|name| name == "README.md") {
            continue;
        }

        let category = extract_category(&path)?;
        println!(
            "Processing {}...",
            path.file_name().unwrap().to_string_lossy()
        );

        let content = sh.read_file(&path)?;
        let tests = parse_tests(&content)?;

        println!("  Found {} tests", tests.len());
        for test in tests {
            let test_name = sanitize_test_name(&test.name);
            let base_path = output_dir.join(format!("{}-{}", category, test_name));

            // Write input file
            let input_path = base_path.with_extension("cdz");
            sh.write_file(&input_path, &test.input)
                .with_context(|| format!("Failed to write {}", input_path.display()))?;

            // Write expected file
            let expected_path = base_path.with_extension("expected");
            sh.write_file(&expected_path, &test.expected)
                .with_context(|| format!("Failed to write {}", expected_path.display()))?;

            println!("    ✓ {}", test_name);
            total_tests += 1;
        }
        println!();
    }

    println!("Extracted {} test cases", total_tests);
    println!();

    Ok(())
}

fn generate_report(_sh: &Shell) -> Result<()> {
    println!("Semantics test report generation not yet implemented");
    println!("This will show test status by category");
    Ok(())
}

/// Extract category name from filename (e.g., "01-literals.md" -> "literals")
fn extract_category(path: &Path) -> Result<String> {
    let filename = path
        .file_stem()
        .and_then(|s| s.to_str())
        .context("Invalid filename")?;

    // Remove leading number and dash if present (e.g., "01-literals" -> "literals")
    let category = if let Some(idx) = filename.find('-') {
        if filename[..idx].chars().all(|c| c.is_ascii_digit()) {
            &filename[idx + 1..]
        } else {
            filename
        }
    } else {
        filename
    };

    Ok(category.to_string())
}

/// Sanitize test name to create a stable filename
/// Converts to lowercase, replaces spaces/special chars with hyphens
fn sanitize_test_name(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c
            } else if c.is_whitespace() || c == '-' {
                '-'
            } else {
                '_'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

#[derive(Debug)]
struct Test {
    name: String,
    input: String,
    expected: String,
}

/// Parse tests from markdown content
fn parse_tests(content: &str) -> Result<Vec<Test>> {
    let mut tests = Vec::new();
    let lines: Vec<&str> = content.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i].trim();

        // Look for test headings: ### Test: <name>
        if line.starts_with("### Test:") {
            let name = line.strip_prefix("### Test:").unwrap().trim().to_string();

            // Skip ERROR prefix for cleaner names but keep it in the name
            // This makes the filename descriptive
            let clean_name = name.clone();

            i += 1;

            // Find Input section
            let mut input = String::new();
            let mut found_input = false;

            while i < lines.len() {
                if lines[i].trim() == "**Input:**" {
                    found_input = true;
                    i += 1;
                    // Skip empty lines
                    while i < lines.len() && lines[i].trim().is_empty() {
                        i += 1;
                    }
                    // Expect code fence
                    if i < lines.len() && lines[i].trim().starts_with("```") {
                        i += 1;
                        // Collect code until closing fence
                        while i < lines.len() && !lines[i].trim().starts_with("```") {
                            if !input.is_empty() {
                                input.push('\n');
                            }
                            input.push_str(lines[i]);
                            i += 1;
                        }
                        i += 1; // Skip closing fence
                    }
                    break;
                }
                i += 1;
            }

            if !found_input {
                continue; // Skip this test if no input found
            }

            // Find Output section
            let mut expected = String::new();
            let mut found_output = false;

            while i < lines.len() {
                if lines[i].trim() == "**Output:**" {
                    found_output = true;
                    i += 1;
                    // Skip empty lines
                    while i < lines.len() && lines[i].trim().is_empty() {
                        i += 1;
                    }
                    // Expect code fence
                    if i < lines.len() && lines[i].trim().starts_with("```") {
                        i += 1;
                        // Collect code until closing fence
                        while i < lines.len() && !lines[i].trim().starts_with("```") {
                            if !expected.is_empty() {
                                expected.push('\n');
                            }
                            expected.push_str(lines[i]);
                            i += 1;
                        }
                        i += 1; // Skip closing fence
                    }
                    break;
                }
                i += 1;
            }

            if !found_output {
                continue; // Skip this test if no output found
            }

            // Add the test
            tests.push(Test {
                name: clean_name,
                input,
                expected,
            });
        } else {
            i += 1;
        }
    }

    Ok(tests)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_test_name() {
        assert_eq!(sanitize_test_name("Simple integer"), "simple-integer");
        assert_eq!(
            sanitize_test_name("ERROR - Unclosed character literal"),
            "error-unclosed-character-literal"
        );
        assert_eq!(sanitize_test_name("Multi-line string"), "multi-line-string");
        assert_eq!(
            sanitize_test_name("String with Unicode"),
            "string-with-unicode"
        );
    }

    #[test]
    fn test_extract_category() {
        let path = Path::new("01-literals.md");
        assert_eq!(extract_category(path).unwrap(), "literals");

        let path = Path::new("variables.md");
        assert_eq!(extract_category(path).unwrap(), "variables");
    }
}
