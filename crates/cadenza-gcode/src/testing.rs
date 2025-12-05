//! Testing utilities for GCode parser validation.

use crate::parse;

/// Verify that CST spans cover all bytes in the source and token text matches source.
///
/// This function validates two important properties:
/// 1. Every byte in the source is covered by at least one token in the CST
/// 2. Each token's text exactly matches the corresponding source text at that position
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

            // Verify token text matches source
            let token_text = token.text();
            let source_text = &src[start..end];
            assert_eq!(
                token_text, source_text,
                "Token text mismatch at {}..{}: token='{}', source='{}'",
                start, end, token_text, source_text
            );
        }
    }

    // Verify all bytes are covered
    for (i, &is_covered) in covered.iter().enumerate() {
        assert!(is_covered, "Byte at position {} not covered in CST", i);
    }
}
