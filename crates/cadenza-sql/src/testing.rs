//! Testing utilities for SQL parser.

use crate::parse;

/// Verify that all bytes in the source are covered by the CST.
pub fn verify_cst_coverage(sql: &str) {
    let parse_result = parse(sql);
    let cst = parse_result.syntax();

    // Verify that CST covers all source bytes
    let mut covered = vec![false; sql.len()];

    for token in cst.descendants_with_tokens() {
        if let cadenza_tree::SyntaxElement::Token(token) = token {
            let range = token.text_range();
            for i in range.start().into()..range.end().into() {
                covered[i] = true;
            }
        }
    }

    for (i, &is_covered) in covered.iter().enumerate() {
        if !is_covered {
            panic!(
                "Byte at position {} is not covered by CST: {:?}",
                i,
                &sql[i..i + 1]
            );
        }
    }
}
