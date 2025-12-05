//! GCode lexer and parser that produces Cadenza-compatible AST.
//!
//! This module treats GCode as an alternative syntax for Cadenza. It lexes and parses
//! GCode into a Rowan CST that can be directly evaluated by the Cadenza eval crate.
//!
//! # Architecture
//!
//! - **Lexer**: Tokenizes GCode (G1, X100, comments, etc.)
//! - **Parser**: Builds Rowan GreenNode CST using cadenza-syntax token kinds
//! - **AST**: GCode commands become Apply nodes, parameters become Apply nodes with letter as receiver
//!
//! # Example
//!
//! ```
//! use cadenza_gcode::parse;
//! use cadenza_eval::eval;
//!
//! let gcode = "G28\nG1 X100 Y50\n";
//! let root = parse(gcode);
//! // eval() doesn't care that this came from GCode - it's just an AST
//! ```

use cadenza_syntax::{parse::Parse, token::Kind};
use rowan::GreenNodeBuilder;

/// Parse GCode source into a Cadenza-compatible AST.
pub fn parse(src: &str) -> Parse {
    Parser::new(src).parse()
}

struct Parser<'src> {
    src: &'src str,
    pos: usize,
    builder: GreenNodeBuilder<'static>,
    errors: Vec<cadenza_syntax::parse::ParseError>,
}

impl<'src> Parser<'src> {
    fn new(src: &'src str) -> Self {
        Self {
            src,
            pos: 0,
            builder: GreenNodeBuilder::new(),
            errors: Vec::new(),
        }
    }

    fn parse(mut self) -> Parse {
        self.builder.start_node(Kind::Root.into());

        while self.pos < self.src.len() {
            self.skip_line_whitespace();
            if self.pos >= self.src.len() {
                break;
            }

            let ch = self.peek_char();
            if ch == Some('\n') || ch == Some('\r') {
                self.skip_newline();
            } else if ch == Some(';') {
                self.parse_comment();
                self.skip_newline();
            } else if ch == Some('(') {
                self.parse_parentheses_comment();
                // Comments might be inline, only skip newline if we're at one
                if self.peek_char() == Some('\n') || self.peek_char() == Some('\r') {
                    self.skip_newline();
                }
            } else if ch == Some('%') {
                self.parse_percent_delimiter();
                self.skip_newline();
            } else if self.is_line_number_start() {
                // Line number - parse N with number and optional command as arguments
                self.parse_line_number_with_command();
                self.skip_newline();
            } else if self.peek_char().is_some_and(|c| c.is_ascii_alphabetic()) {
                self.parse_command();
                self.skip_line_whitespace();
                if self.peek_char() == Some(';') {
                    self.parse_comment();
                } else if self.peek_char() == Some('(') {
                    self.parse_parentheses_comment();
                }
                self.skip_newline();
            } else {
                // Skip unexpected character
                self.pos += 1;
            }
        }

        self.builder.finish_node();

        Parse {
            green: self.builder.finish(),
            errors: self.errors,
        }
    }

    fn skip_line_whitespace(&mut self) {
        let start = self.pos;
        while self.pos < self.src.len() {
            let ch = self.src.as_bytes()[self.pos];
            if ch == b' ' || ch == b'\t' {
                self.pos += 1;
            } else {
                break;
            }
        }
        if self.pos > start {
            let text = &self.src[start..self.pos];
            self.builder.token(Kind::Space.into(), text);
        }
    }

    fn skip_newline(&mut self) {
        let start = self.pos;
        if self.pos < self.src.len() {
            let ch = self.src.as_bytes()[self.pos];
            if ch == b'\r' {
                self.pos += 1;
                if self.pos < self.src.len() && self.src.as_bytes()[self.pos] == b'\n' {
                    self.pos += 1;
                }
            } else if ch == b'\n' {
                self.pos += 1;
            }
        }
        if self.pos > start {
            let text = &self.src[start..self.pos];
            self.builder.token(Kind::Newline.into(), text);
        }
    }

    fn parse_comment(&mut self) {
        // The comment includes the semicolon and everything up to (but not including) the newline
        let start = self.pos;

        // Read until end of line
        while self.pos < self.src.len() {
            let ch = self.peek_char().unwrap();
            if ch == '\n' || ch == '\r' {
                break;
            }
            self.pos += 1;
        }

        let content = &self.src[start..self.pos];
        self.builder.token(Kind::CommentContent.into(), content);
    }

    fn parse_command(&mut self) {
        // Track the start of this command line for checksum validation
        let line_start = self.pos;

        // Start an Apply node for the command
        self.builder.start_node(Kind::Apply.into());

        // Parse command code (G1, M104, etc.) as the receiver
        self.builder.start_node(Kind::ApplyReceiver.into());
        self.parse_identifier();
        self.builder.finish_node();

        // Parse parameters as arguments
        loop {
            self.skip_line_whitespace();
            let ch = self.peek_char();
            if ch == Some(';') || ch == Some('\n') || ch == Some('\r') || ch.is_none() {
                break;
            }
            // Check for inline parentheses comment
            if ch == Some('(') {
                self.parse_parentheses_comment();
                continue; // Continue parsing parameters after comment
            }
            // Check for checksum marker
            if ch == Some('*') {
                self.parse_checksum(line_start);
                break;
            }
            if self.peek_char().is_some_and(|c| c.is_ascii_alphabetic()) {
                self.builder.start_node(Kind::ApplyArgument.into());
                self.parse_parameter();
                self.builder.finish_node();
            } else {
                break;
            }
        }

        self.builder.finish_node();
    }

    fn parse_identifier(&mut self) {
        let start = self.pos;
        while self.pos < self.src.len() {
            let ch = self.src.as_bytes()[self.pos];
            if ch.is_ascii_alphanumeric() || ch == b'_' {
                self.pos += 1;
            } else {
                break;
            }
        }
        let text = &self.src[start..self.pos];

        self.builder.start_node(Kind::Identifier.into());
        self.builder.token(Kind::Identifier.into(), text);
        self.builder.finish_node();
    }

    fn parse_parameter(&mut self) {
        // Parameter can be:
        // 1. X100 or Y50 (traditional GCode - single letter + value)
        // 2. PIN=my_led (Klipper named parameter - multi-char identifier with =)
        // 3. X (flag without value)

        let letter_start = self.pos;

        // Parse the first character (letter)
        self.pos += 1;

        // Check if this is a Klipper-style named parameter (more letters/underscores before =)
        let mut is_klipper_style = false;
        let save_pos = self.pos;

        while self.pos < self.src.len() {
            let ch = self.peek_char().unwrap();
            if ch.is_ascii_alphabetic() || ch == '_' {
                self.pos += 1;
                is_klipper_style = true;
            } else if ch == '=' && is_klipper_style {
                // This is Klipper format
                break;
            } else {
                // Not Klipper format, restore position
                self.pos = save_pos;
                break;
            }
        }

        let letter_text = &self.src[letter_start..self.pos];

        // Check for equals sign (Klipper format: NAME=value)
        if self.peek_char() == Some('=') {
            let eq_start = self.pos;
            self.pos += 1; // Move past '='
            let eq_text = &self.src[eq_start..self.pos];

            // Create Apply node: [=, NAME, value]
            // Tokens must be in source order, but nodes determine structure
            self.builder.start_node(Kind::Apply.into());

            // First, emit NAME as first argument (comes first in source)
            self.builder.start_node(Kind::ApplyArgument.into());
            self.builder.start_node(Kind::Identifier.into());
            self.builder.token(Kind::Identifier.into(), letter_text);
            self.builder.finish_node();
            self.builder.finish_node();

            // Then emit = as receiver (comes second in source)
            self.builder.start_node(Kind::ApplyReceiver.into());
            self.builder.start_node(Kind::Identifier.into());
            self.builder.token(Kind::Equal.into(), eq_text);
            self.builder.finish_node();
            self.builder.finish_node();

            // Finally parse the value after '=' as second argument
            self.builder.start_node(Kind::ApplyArgument.into());
            self.parse_parameter_value();
            self.builder.finish_node();

            self.builder.finish_node();
        } else {
            // Traditional GCode format or flag
            let has_value = self.pos < self.src.len()
                && self
                    .peek_char()
                    .is_some_and(|c| c.is_ascii_digit() || c == '.' || c == '-');

            if has_value {
                // Create Apply node: [Letter, value]
                self.builder.start_node(Kind::Apply.into());

                // Letter as receiver
                self.builder.start_node(Kind::ApplyReceiver.into());
                self.builder.start_node(Kind::Identifier.into());
                self.builder.token(Kind::Identifier.into(), letter_text);
                self.builder.finish_node();
                self.builder.finish_node();

                // Value as argument
                self.builder.start_node(Kind::ApplyArgument.into());
                self.parse_number();
                self.builder.finish_node();

                self.builder.finish_node();
            } else {
                // Just the letter as an identifier (flag)
                self.builder.start_node(Kind::Identifier.into());
                self.builder.token(Kind::Identifier.into(), letter_text);
                self.builder.finish_node();
            }
        }
    }

    fn parse_parameter_value(&mut self) {
        // Parse value after '=' in Klipper format
        // Can be a number or an identifier/string
        let ch = self.peek_char();
        if ch.is_some_and(|c| c.is_ascii_digit() || c == '.' || c == '-') {
            self.parse_number();
        } else if ch.is_some_and(|c| c.is_ascii_alphabetic() || c == '_') {
            // Parse identifier value
            let start = self.pos;
            while self.pos < self.src.len() {
                let ch = self.peek_char().unwrap();
                if ch.is_ascii_alphanumeric() || ch == '_' {
                    self.pos += 1;
                } else {
                    break;
                }
            }
            let text = &self.src[start..self.pos];
            self.builder.start_node(Kind::Identifier.into());
            self.builder.token(Kind::Identifier.into(), text);
            self.builder.finish_node();
        }
    }

    fn parse_checksum(&mut self, line_start: usize) {
        // Parse checksum in format *NN where NN is a number
        // GCode checksum is XOR of all bytes before the asterisk
        if self.peek_char() != Some('*') {
            return;
        }

        let checksum_start = self.pos;
        self.pos += 1; // Move past '*'

        // Parse the checksum number
        let num_start = self.pos;
        while self.pos < self.src.len() {
            let ch = self.peek_char().unwrap();
            if ch.is_ascii_digit() {
                self.pos += 1;
            } else {
                break;
            }
        }

        let text = &self.src[checksum_start..self.pos];

        // Validate checksum
        let checksum_str = &self.src[num_start..self.pos];
        if let Ok(expected_checksum) = checksum_str.parse::<u8>() {
            // Calculate actual checksum (XOR of all bytes before *)
            let mut calculated: u8 = 0;
            for &byte in self.src[line_start..checksum_start].as_bytes() {
                calculated ^= byte;
            }

            if calculated != expected_checksum {
                // Emit error node for invalid checksum
                self.builder.start_node(Kind::Error.into());
                self.builder.token(Kind::CommentContent.into(), text);
                self.builder.finish_node();

                // Add parse error
                self.errors.push(cadenza_syntax::parse::ParseError {
                    message: format!(
                        "Invalid checksum: expected {}, got {}",
                        calculated, expected_checksum
                    ),
                    span: (checksum_start..self.pos).into(),
                });
                return;
            }
        }

        // Emit checksum as a comment token to preserve it in CST
        self.builder.token(Kind::CommentContent.into(), text);
    }

    fn parse_number(&mut self) {
        let start = self.pos;
        let mut has_dot = false;

        // Handle negative sign
        if self.peek_char() == Some('-') {
            self.pos += 1;
        }

        // Parse digits and possibly a decimal point
        while self.pos < self.src.len() {
            let ch = self.peek_char().unwrap();
            if ch.is_ascii_digit() {
                self.pos += 1;
            } else if ch == '.' && !has_dot {
                has_dot = true;
                self.pos += 1;
            } else {
                break;
            }
        }

        let text = &self.src[start..self.pos];

        // Wrap in Literal node
        self.builder.start_node(Kind::Literal.into());

        if has_dot {
            self.builder.start_node(Kind::Float.into());
            self.builder.token(Kind::Float.into(), text);
            self.builder.finish_node();
        } else {
            self.builder.start_node(Kind::Integer.into());
            self.builder.token(Kind::Integer.into(), text);
            self.builder.finish_node();
        }

        self.builder.finish_node();
    }

    fn peek_char(&self) -> Option<char> {
        if self.pos < self.src.len() {
            Some(self.src.as_bytes()[self.pos] as char)
        } else {
            None
        }
    }

    fn peek_ahead(&self, offset: usize) -> Option<char> {
        let pos = self.pos + offset;
        if pos < self.src.len() {
            Some(self.src.as_bytes()[pos] as char)
        } else {
            None
        }
    }

    fn is_line_number_start(&self) -> bool {
        // Line numbers start with 'N' followed immediately by a digit
        self.peek_char() == Some('N') && self.peek_ahead(1).is_some_and(|c| c.is_ascii_digit())
    }

    fn parse_line_number_with_command(&mut self) {
        // Line numbers are in format N123 [command]
        // Parse as Apply node: [N, 123, [command]] if command is present
        // This allows the N macro to set up environment before executing the command
        
        let n_start = self.pos;
        
        // Start Apply node: [N, number, [command]]
        // This wraps the command as an argument, allowing N macro to control environment
        self.builder.start_node(Kind::Apply.into());

        // 'N' as receiver - extract from source
        self.pos += 1; // Move past 'N'
        let n_text = &self.src[n_start..self.pos];
        
        self.builder.start_node(Kind::ApplyReceiver.into());
        self.builder.start_node(Kind::Identifier.into());
        self.builder.token(Kind::Identifier.into(), n_text);
        self.builder.finish_node();
        self.builder.finish_node();

        // Parse the number as first argument
        self.builder.start_node(Kind::ApplyArgument.into());
        self.parse_number();
        self.builder.finish_node();

        // Check if there's a command following on the same line
        self.skip_line_whitespace();
        if self.peek_char().is_some_and(|c| c.is_ascii_alphabetic()) {
            // Parse the command as second argument to N
            self.builder.start_node(Kind::ApplyArgument.into());
            self.parse_command();
            self.builder.finish_node();
            
            // Handle any trailing comments
            self.skip_line_whitespace();
            if self.peek_char() == Some(';') {
                self.parse_comment();
            }
        }

        self.builder.finish_node();
    }

    fn parse_parentheses_comment(&mut self) {
        // Parentheses comments are an alternative GCode comment style
        // Format: (comment text)
        let start = self.pos;

        // Skip opening parenthesis
        self.pos += 1;

        // Read until closing parenthesis or end of line
        while self.pos < self.src.len() {
            if let Some(ch) = self.peek_char() {
                if ch == ')' {
                    self.pos += 1;
                    break;
                } else if ch == '\n' || ch == '\r' {
                    // Unclosed comment - stop at newline
                    break;
                }
                self.pos += 1;
            } else {
                break;
            }
        }

        let content = &self.src[start..self.pos];
        self.builder.token(Kind::CommentContent.into(), content);
    }

    fn parse_percent_delimiter(&mut self) {
        // Percent signs (%) are used to delimit programs in some GCode dialects
        // Format: % on its own line
        // We'll parse them as comment tokens to preserve them
        let start = self.pos;

        // Skip '%'
        self.pos += 1;

        let text = &self.src[start..self.pos];
        self.builder.token(Kind::CommentContent.into(), text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cadenza_eval::{BuiltinMacro, Compiler, Env, Type, Value, eval};

    #[test]
    fn test_execute_gcode_with_macros() {
        let gcode = "G28\nG1 X100 Y50\n";

        // Parse GCode directly into Cadenza AST
        let parse_result = parse(gcode);
        let root = parse_result.ast();

        let mut compiler = Compiler::new();
        let mut env = Env::new();

        // Register G28 macro
        compiler.define_macro(
            "G28".into(),
            Value::BuiltinMacro(BuiltinMacro {
                name: "G28",
                signature: Type::function(vec![], Type::Nil),
                func: |_args, _ctx| Ok(Value::Nil),
            }),
        );

        // Register G1 macro that accepts parameters as [X, 100], [Y, 50]
        compiler.define_macro(
            "G1".into(),
            Value::BuiltinMacro(BuiltinMacro {
                name: "G1",
                signature: Type::function(vec![Type::Unknown, Type::Unknown], Type::Nil),
                func: |args, _ctx| {
                    // Args should be apply nodes like [X, 100], [Y, 50]
                    assert_eq!(args.len(), 2);
                    Ok(Value::Nil)
                },
            }),
        );

        // Register parameter name macros (X, Y, etc.)
        for letter in &["X", "Y", "Z", "E", "F", "S"] {
            compiler.define_macro(
                (*letter).into(),
                Value::BuiltinMacro(BuiltinMacro {
                    name: letter,
                    signature: Type::function(vec![Type::Unknown], Type::Unknown),
                    func: |_args, _ctx| Ok(Value::Nil),
                }),
            );
        }

        // Evaluate - eval doesn't care this came from GCode!
        let results = eval(&root, &mut env, &mut compiler);
        assert_eq!(results.len(), 2);
    }
}
