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
            } else if self.peek_char().is_some_and(|c| c.is_ascii_alphabetic()) {
                self.parse_command();
                self.skip_line_whitespace();
                if self.peek_char() == Some(';') {
                    self.parse_comment();
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
        while self.pos < self.src.len() {
            let ch = self.src.as_bytes()[self.pos];
            if ch == b' ' || ch == b'\t' {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn skip_newline(&mut self) {
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
    }

    fn parse_comment(&mut self) {
        let start = self.pos;
        self.pos += 1; // skip ';'

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
            if ch.is_ascii_alphanumeric() {
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
        // Parameter is like X100 or Y50 or just X (flag)
        // We'll parse it as: [X, 100] where X is the receiver (letter) and 100 is the argument

        let letter_start = self.pos;
        self.pos += 1;
        let letter_text = &self.src[letter_start..self.pos];

        // Check if there's a value after the letter
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
