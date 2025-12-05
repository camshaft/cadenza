//! GCode lexer and parser that produces Cadenza-compatible AST.
//!
//! This module treats GCode as an alternative syntax for Cadenza. It lexes and parses
//! GCode into a Rowan CST that can be directly evaluated by the Cadenza eval crate.
//!
//! # Architecture
//!
//! - **Lexer**: Tokenizes GCode (G1, X100, comments, etc.)
//! - **Parser**: Builds Rowan GreenNode CST using cadenza-syntax token kinds
//! - **AST**: GCode commands become Apply nodes, parameters become literals
//!
//! # Example
//!
//! ```
//! use cadenza_gcode::gcode_parse;
//! use cadenza_eval::eval;
//!
//! let gcode = "G28\nG1 X100 Y50\n";
//! let root = gcode_parse(gcode);
//! // eval() doesn't care that this came from GCode - it's just an AST
//! ```

use cadenza_syntax::{parse::Parse, token::Kind};
use rowan::GreenNodeBuilder;

/// Parse GCode source into a Cadenza-compatible AST.
pub fn gcode_parse(src: &str) -> Parse {
    GCodeParser::new(src).parse()
}

struct GCodeParser<'src> {
    src: &'src str,
    offset: usize,
    builder: GreenNodeBuilder<'static>,
    errors: Vec<cadenza_syntax::parse::ParseError>,
}

impl<'src> GCodeParser<'src> {
    fn new(src: &'src str) -> Self {
        Self {
            src,
            offset: 0,
            builder: GreenNodeBuilder::new(),
            errors: Vec::new(),
        }
    }

    fn parse(mut self) -> Parse {
        self.builder.start_node(Kind::Root.into());

        for line in self.src.lines() {
            let line_start = self.offset;
            self.parse_line(line, line_start);
            self.offset = line_start + line.len() + 1; // +1 for newline
        }

        self.builder.finish_node();

        Parse {
            green: self.builder.finish(),
            errors: self.errors,
        }
    }

    fn parse_line(&mut self, line: &str, line_start: usize) {
        let line = line.trim();

        if line.is_empty() {
            return;
        }

        // Handle comments
        if line.starts_with(';') {
            // Comments are trivia, we can skip them or add as whitespace
            return;
        }

        // Handle inline comments
        let code_part = if let Some(pos) = line.find(';') {
            line[..pos].trim()
        } else {
            line
        };

        if code_part.is_empty() {
            return;
        }

        // Parse command as an Apply node
        self.parse_command(code_part, line_start);
    }

    fn parse_command(&mut self, code: &str, line_start: usize) {
        let parts: Vec<&str> = code.split_whitespace().collect();
        if parts.is_empty() {
            return;
        }

        // Start an Apply node for the command
        self.builder.start_node(Kind::Apply.into());

        // Parse command code (G1, M104, etc.) as the receiver
        self.builder.start_node(Kind::ApplyReceiver.into());
        self.parse_command_identifier(parts[0], line_start);
        self.builder.finish_node();

        // Parse parameters as key-value pair arguments: [=, key, value]
        for param in &parts[1..] {
            self.builder.start_node(Kind::ApplyArgument.into());
            self.parse_parameter_as_assignment(param, line_start);
            self.builder.finish_node();
        }

        self.builder.finish_node();
    }

    fn parse_command_identifier(&mut self, cmd: &str, _line_start: usize) {
        // Create an Identifier node (not just a token)
        self.builder.start_node(Kind::Identifier.into());
        self.builder.token(Kind::Identifier.into(), cmd);
        self.builder.finish_node();
    }

    fn parse_parameter_as_assignment(&mut self, param: &str, _line_start: usize) {
        if param.is_empty() {
            return;
        }

        let letter = param.chars().next().unwrap();
        let value_str = &param[1..];

        // Create an Apply node for assignment: [=, key, value]
        self.builder.start_node(Kind::Apply.into());

        // The "=" operator as receiver
        self.builder.start_node(Kind::ApplyReceiver.into());
        self.builder.start_node(Kind::Identifier.into());
        self.builder.token(Kind::Identifier.into(), "=");
        self.builder.finish_node();
        self.builder.finish_node();

        // The parameter letter as the first argument (key)
        self.builder.start_node(Kind::ApplyArgument.into());
        self.builder.start_node(Kind::Identifier.into());
        self.builder
            .token(Kind::Identifier.into(), &letter.to_string());
        self.builder.finish_node();
        self.builder.finish_node();

        // The value as the second argument
        self.builder.start_node(Kind::ApplyArgument.into());
        if value_str.is_empty() {
            // For flags with no value, use identifier "true"
            self.builder.start_node(Kind::Identifier.into());
            self.builder.token(Kind::Identifier.into(), "true");
            self.builder.finish_node();
        } else {
            // Parse the numeric value without assuming units
            self.emit_number_literal(value_str);
        }
        self.builder.finish_node();

        self.builder.finish_node(); // Close Apply for =
    }

    fn emit_number_literal(&mut self, value: &str) {
        // Wrap in Literal node, then Integer/Float node, then the token
        self.builder.start_node(Kind::Literal.into());

        if value.contains('.') {
            self.builder.start_node(Kind::Float.into());
            self.builder.token(Kind::Float.into(), value);
            self.builder.finish_node(); // Close Float node
        } else {
            self.builder.start_node(Kind::Integer.into());
            self.builder.token(Kind::Integer.into(), value);
            self.builder.finish_node(); // Close Integer node
        }

        self.builder.finish_node(); // Close Literal
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cadenza_eval::{BuiltinMacro, Compiler, Env, Type, Value, eval};

    #[test]
    fn test_parse_simple_gcode() {
        let gcode = "G28\n";
        let parse = gcode_parse(gcode);
        let root = parse.ast();

        // Should have one expression
        let items: Vec<_> = root.items().collect();
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn test_parse_with_params() {
        let gcode = "G1 X100 Y50\n";
        let parse = gcode_parse(gcode);
        let root = parse.ast();

        let items: Vec<_> = root.items().collect();
        assert_eq!(items.len(), 1);

        // Check the structure: should be [G1, [=, X, 100], [=, Y, 50]]
        let expr = items.first().unwrap();
        println!("Parsed expression: {:?}", expr);
    }

    #[test]
    fn test_execute_gcode_with_macros() {
        let gcode = "G28\nG1 X100 Y50\n";

        // Parse GCode directly into Cadenza AST
        let parse = gcode_parse(gcode);
        let root = parse.ast();

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

        // Register G1 macro that accepts key-value pairs
        compiler.define_macro(
            "G1".into(),
            Value::BuiltinMacro(BuiltinMacro {
                name: "G1",
                signature: Type::function(vec![Type::Unknown, Type::Unknown], Type::Nil),
                func: |args, _ctx| {
                    // Args should be key-value pairs like [=, X, 100], [=, Y, 50]
                    assert_eq!(args.len(), 2);
                    Ok(Value::Nil)
                },
            }),
        );

        // Register = operator
        compiler.define_macro(
            "=".into(),
            Value::BuiltinMacro(BuiltinMacro {
                name: "=",
                signature: Type::function(vec![Type::Unknown, Type::Unknown], Type::Unknown),
                func: |_args, _ctx| Ok(Value::Nil),
            }),
        );

        // Evaluate - eval doesn't care this came from GCode!
        let results = eval(&root, &mut env, &mut compiler);
        assert_eq!(results.len(), 2);
    }
}
