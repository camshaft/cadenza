//! Example: Transpile a GCode file to Cadenza
//!
//! Usage:
//!   cargo run --example transpile < input.gcode
//!
//! Or with a file:
//!   cargo run --example transpile test-data/simple.gcode

use cadenza_gcode::{parse_gcode, transpile_to_cadenza};
use std::{
    env, fs,
    io::{self, Read},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Get input from file argument or stdin
    let input = if let Some(filename) = env::args().nth(1) {
        fs::read_to_string(filename)?
    } else {
        let mut buffer = String::new();
        io::stdin().read_to_string(&mut buffer)?;
        buffer
    };

    // Parse GCode
    let program = parse_gcode(&input)?;

    // Transpile to Cadenza
    let cadenza = transpile_to_cadenza(&program)?;

    // Output result
    println!("{}", cadenza);

    Ok(())
}
