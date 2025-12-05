//! Example: Custom command handler registration
//!
//! Demonstrates how to register custom GCode command handlers.

use cadenza_gcode::{CommandCode, TranspilerConfig, parse_gcode, transpile_with_config};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let gcode = r#"
; Custom GCode example
G28           ; Standard home
G29           ; Custom bed leveling
G1 X100 Y100  ; Move
"#;

    // Parse GCode
    let program = parse_gcode(gcode)?;

    // Create custom configuration
    let mut config = TranspilerConfig::default();

    // Register custom handler for G29 (bed leveling)
    config.register_handler(CommandCode::G(29), "handle_bed_leveling".to_string());

    // Register custom handler for a hypothetical G42 (custom command)
    config.register_handler(CommandCode::G(42), "handle_custom_operation".to_string());

    // Transpile with custom configuration
    let cadenza = transpile_with_config(&program, &config)?;

    println!("{}", cadenza);

    Ok(())
}
