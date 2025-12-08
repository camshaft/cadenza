//! Testing utilities for the Cadenza REPL.
//!
//! This module provides helper functions for testing the REPL using in-memory
//! I/O streams instead of spawning a separate process.

use std::io::Cursor;

/// Run the REPL with the given input and return the combined stdout and stderr output.
///
/// This function uses the generic `run_repl` function with in-memory I/O streams,
/// making it suitable for unit testing without requiring a built binary.
pub fn repl(input: &str) -> String {
    let input_cursor = Cursor::new(input.as_bytes());
    let mut output = Vec::new();
    let mut error = Vec::new();

    // Run the REPL with in-memory I/O
    let _ = crate::repl::run_repl(input_cursor, &mut output, &mut error, None);

    // Combine stdout and stderr
    let mut result = String::from_utf8_lossy(&output).to_string();
    let stderr = String::from_utf8_lossy(&error);
    if !stderr.is_empty() {
        result.push_str(&stderr);
    }

    result
}
