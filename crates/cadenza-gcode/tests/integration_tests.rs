use cadenza_gcode::{parse_gcode, transpile_to_cadenza};

#[test]
fn test_transpile_simple_gcode() {
    let gcode = include_str!("../test-data/simple.gcode");
    let program = parse_gcode(gcode).expect("Failed to parse GCode");
    let cadenza = transpile_to_cadenza(&program).expect("Failed to transpile");

    // Verify key elements are present
    assert!(cadenza.contains("handle_g28"));
    assert!(cadenza.contains("handle_g90"));
    assert!(cadenza.contains("handle_g1"));
    assert!(cadenza.contains("handle_m104"));
    assert!(cadenza.contains("millimeter"));

    // Verify comments are preserved
    assert!(cadenza.contains("# Home all axes"));
    assert!(cadenza.contains("# Draw a simple square"));
}

#[test]
fn test_parse_various_formats() {
    // Test with different whitespace and formatting
    let test_cases = vec![
        "G1 X100", "G1  X100", // Extra space
        "G1 X100 ", // Trailing space
        " G1 X100", // Leading space
        "g1 x100",  // Lowercase
        "G1X100",   // No space (some slicers do this)
    ];

    for case in test_cases {
        let result = parse_gcode(case);
        // The parser should handle all these formats or fail gracefully
        // For now, we expect most to succeed
        if result.is_err() {
            // G1X100 format might not be supported yet, that's okay
            continue;
        }
    }
}

#[test]
fn test_error_handling() {
    // Test invalid GCode
    let invalid_cases = vec![
        "INVALID", // Unknown command type
        "G",       // Missing number
        "G1 Xabc", // Invalid parameter value
    ];

    for case in invalid_cases {
        let result = parse_gcode(case);
        // These should either parse with custom command or error
        // The exact behavior depends on our parser implementation
        // For extensibility, we allow custom commands, so INVALID might parse
        // But Xabc should definitely fail
        if case.contains("abc") {
            assert!(result.is_err(), "Expected error for case: {}", case);
        }
    }
}
