use cadenza_gcode::{parse_gcode, transpile_to_cadenza};

#[test]
fn test_snapshot_simple_gcode() {
    let gcode = include_str!("../test-data/simple.gcode");
    let program = parse_gcode(gcode).expect("Failed to parse GCode");
    let cadenza = transpile_to_cadenza(&program).expect("Failed to transpile");

    insta::assert_snapshot!("simple_gcode", cadenza);
}

#[test]
fn test_snapshot_complex_gcode() {
    let gcode = include_str!("../test-data/complex.gcode");
    let program = parse_gcode(gcode).expect("Failed to parse GCode");
    let cadenza = transpile_to_cadenza(&program).expect("Failed to transpile");

    insta::assert_snapshot!("complex_gcode", cadenza);
}
