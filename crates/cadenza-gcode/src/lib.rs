//! GCode parsing and transpilation to Cadenza.
//!
//! This crate provides functionality to parse GCode files (primarily RepRap flavor)
//! and transpile them to Cadenza source code. The transpiled code can then be
//! parsed, type-checked, and executed using Cadenza's interpreter.
//!
//! # Example
//!
//! ```rust
//! use cadenza_gcode::{parse_gcode, transpile_to_cadenza};
//!
//! let gcode = "G28\nG1 X100 Y50 F3000\nM104 S200\n";
//! let commands = parse_gcode(gcode).unwrap();
//! let cadenza_code = transpile_to_cadenza(&commands).unwrap();
//! ```

pub mod ast;
pub mod error;
pub mod parser;
pub mod transpiler;

mod generated;

pub use ast::CommandCode;
pub use error::{Error, Result};
pub use parser::{parse_gcode, parse_gcode_lines};
pub use transpiler::{TranspilerConfig, transpile_to_cadenza, transpile_with_config};
