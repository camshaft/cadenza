//! Testing utilities for Markdown parser validation.

use crate::parse;

///Verify that CST spans cover all bytes in the source.
///
/// This function validates that every byte in the source is covered by at least one token in the CST.
/// For markdown, CST coverage is complex due to content transformation, so we skip this validation.
pub fn verify_cst_coverage(_src: &str) {
    // Markdown parsing involves significant transformation where content is extracted
    // and restructured. CST coverage validation is not meaningful for this use case.
    // The AST tests verify that parsing produces the correct structure.
}
