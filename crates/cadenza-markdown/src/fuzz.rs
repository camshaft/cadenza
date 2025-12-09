//! Fuzz tests for the Markdown parser.
//!
//! These tests use property-based testing with bolero to ensure the parser
//! is robust against arbitrary input and doesn't crash or loop infinitely.

use crate::parse;

/// Fuzz test to ensure parser doesn't crash or loop infinitely on arbitrary input.
///
/// This test generates arbitrary byte sequences and verifies that:
/// 1. The parser completes without panicking
/// 2. The parser doesn't enter an infinite loop
/// 3. The parser produces a valid CST with a root node
#[test]
fn parse_no_crash() {
    bolero::check!().for_each(|input| {
        let Ok(input) = std::str::from_utf8(input) else {
            return;
        };

        std::thread::scope(|s| {
            let handle = s.spawn(|| run_test(input));

            // Wait up to 1 second for parser to complete
            let start = std::time::Instant::now();
            while start.elapsed() < std::time::Duration::from_secs(1) {
                if handle.is_finished() {
                    // Parser completed successfully
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }

            // If we get here, parser is taking too long
            panic!("Parser took longer than 1 second - likely infinite loop");
        });
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
        cadenza_syntax::token::Kind::Root,
        "Parser should always produce a Root node"
    );

    // 3. The CST should have a valid structure (this just exercises the tree)
    let _descendants = cst.descendants_with_tokens().count();

    // Note: We don't check for parse errors here because arbitrary input
    // is expected to produce errors. We only care that we don't crash.
}
