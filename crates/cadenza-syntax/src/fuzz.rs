//! Fuzz tests for the Cadenza syntax parser.
//!
//! These tests use property-based testing with bolero to ensure the parser
//! is robust against arbitrary input and doesn't crash or loop infinitely.

use crate::parse::parse;

/// Fuzz test to ensure parser doesn't crash or loop infinitely on arbitrary input.
///
/// This test generates arbitrary byte sequences and verifies that:
/// 1. The parser completes without panicking
/// 2. The parser doesn't enter an infinite loop
/// 3. The parser produces a valid CST with a root node
#[test]
fn parse_no_crash() {
    bolero::check!().for_each(|input| {
        let input = String::from_utf8_lossy(input);
        run_test(&input);
    });
}

fn run_test(input: &str) {
    // Parse the arbitrary input
    let result = parse(input);

    // Basic sanity checks:
    // 1. We should get a CST back
    let cst = result.syntax();

    // 2. The root should exist and be a Root node
    assert_eq!(
        cst.kind(),
        crate::token::Kind::Root,
        "Parser should always produce a Root node"
    );

    // 3. The CST should have a valid structure (this just exercises the tree)
    let _descendants = cst.descendants_with_tokens().count();

    // Note: We don't check for parse errors here because arbitrary input
    // is expected to produce errors. We only care that we don't crash.
}
