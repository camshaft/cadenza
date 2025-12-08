//! Testing utilities for the Cadenza REPL.
//!
//! This module provides helper functions for testing the REPL by piping input
//! and capturing output.

use std::io::Write;
use std::process::{Command, Stdio};

/// Run the REPL with the given input and return the combined stdout and stderr output.
///
/// This function spawns the REPL process, pipes the input to stdin,
/// and captures both stdout and stderr. This is used for snapshot testing REPL sessions.
pub fn repl(input: &str) -> String {
    // Get the path to the cadenza binary
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let workspace_dir = std::path::Path::new(manifest_dir)
        .parent()
        .and_then(|p| p.parent())
        .expect("Failed to find workspace directory");
    
    let binary_path = workspace_dir
        .join("target")
        .join("debug")
        .join("cadenza");

    // Spawn the REPL process
    let mut child = Command::new(binary_path)
        .arg("repl")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn REPL process");

    // Write input to stdin
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(input.as_bytes())
            .expect("Failed to write to REPL stdin");
        // Close stdin to signal EOF
        drop(stdin);
    }

    // Wait for the process to complete and capture output
    let output = child.wait_with_output().expect("Failed to wait for REPL");

    // Combine stdout and stderr
    let mut result = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stderr.is_empty() {
        result.push_str(&stderr);
    }
    
    result
}
