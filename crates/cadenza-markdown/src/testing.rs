//! Testing utilities for Markdown parser validation.

use crate::parse;

/// Verify that CST spans cover all bytes in the source.
///
/// This function validates that every byte in the source is covered by at least one token in the CST.
/// Unlike the gcode parser, we don't verify that token text matches source text because markdown
/// parsing involves transformation (e.g., "#" becomes "h1" identifier).
pub fn verify_cst_coverage(src: &str) {
    let parse_result = parse(src);
    let cst = parse_result.syntax();

    // Track which bytes are covered by tokens
    let mut covered = vec![false; src.len()];

    // Iterate through all tokens in the CST
    for node in cst.descendants_with_tokens() {
        if let Some(token) = node.as_token() {
            let range = token.text_range();
            let start: usize = range.start().into();
            let end: usize = range.end().into();

            // Mark bytes as covered
            for item in &mut covered[start..end] {
                *item = true;
            }
        }
    }

    // Verify all bytes are covered
    for (i, &is_covered) in covered.iter().enumerate() {
        assert!(is_covered, "Byte at position {} not covered in CST", i);
    }
}
