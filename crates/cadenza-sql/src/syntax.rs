//! SQL lexer and parser that produces Cadenza-compatible AST.
//!
//! This module treats SQL as an alternative syntax for Cadenza. It lexes and parses
//! SQL into a CST that can be directly evaluated by the Cadenza eval crate.
//!
//! # Architecture
//!
//! - **Lexer**: Tokenizes SQL (SELECT, FROM, WHERE, identifiers, operators, etc.)
//! - **Parser**: Builds GreenNode CST using cadenza-syntax token kinds
//! - **AST**: SQL statements become Apply nodes that call macros
//!
//! # Example
//!
//! ```
//! use cadenza_sql::parse;
//! use cadenza_eval::eval;
//!
//! let sql = "SELECT * FROM users WHERE age > 18";
//! let root = parse(sql);
//! // eval() doesn't care that this came from SQL - it's just an AST
//! ```

use cadenza_syntax::{parse::Parse, token::Kind};
use cadenza_tree::GreenNodeBuilder;

/// Parse SQL source into a Cadenza-compatible AST.
pub fn parse(src: &str) -> Parse {
    Parser::new(src).parse()
}

struct Parser<'src> {
    src: &'src str,
    pos: usize,
    builder: GreenNodeBuilder,
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

        // Parse statements until end of input
        while self.pos < self.src.len() {
            self.skip_whitespace_and_comments();
            if self.pos >= self.src.len() {
                break;
            }

            // Parse a statement
            self.parse_statement();

            // Skip optional semicolon
            self.skip_whitespace_and_comments();
            if self.peek_char() == Some(';') {
                let start = self.pos;
                self.pos += 1;
                let text = &self.src[start..self.pos];
                self.builder.token(Kind::Semicolon.into(), text);
            }
        }

        self.builder.finish_node();

        Parse {
            green: self.builder.finish(),
            errors: self.errors,
        }
    }

    fn skip_whitespace_and_comments(&mut self) {
        loop {
            let start_pos = self.pos;

            // Skip whitespace
            self.skip_whitespace();

            // Skip line comments (-- comment)
            if self.peek_char() == Some('-') && self.peek_ahead(1) == Some('-') {
                self.parse_line_comment();
            }
            // Skip block comments (/* comment */)
            else if self.peek_char() == Some('/') && self.peek_ahead(1) == Some('*') {
                self.parse_block_comment();
            }

            // If we didn't skip anything, we're done
            if self.pos == start_pos {
                break;
            }
        }
    }

    fn skip_whitespace(&mut self) {
        let start = self.pos;
        while self.pos < self.src.len() {
            let ch = self.src.as_bytes()[self.pos];
            if ch == b' ' || ch == b'\t' || ch == b'\n' || ch == b'\r' {
                self.pos += 1;
            } else {
                break;
            }
        }
        if self.pos > start {
            let text = &self.src[start..self.pos];
            // Emit whitespace as appropriate token
            if text.contains('\n') || text.contains('\r') {
                self.builder.token(Kind::Newline.into(), text);
            } else {
                self.builder.token(Kind::Space.into(), text);
            }
        }
    }

    fn parse_line_comment(&mut self) {
        let start = self.pos;
        // Skip --
        self.pos += 2;

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

    fn parse_block_comment(&mut self) {
        let start = self.pos;
        // Skip /*
        self.pos += 2;

        // Read until */
        while self.pos < self.src.len() {
            if self.peek_char() == Some('*') && self.peek_ahead(1) == Some('/') {
                self.pos += 2;
                break;
            }
            self.pos += 1;
        }

        let content = &self.src[start..self.pos];
        self.builder.token(Kind::CommentContent.into(), content);
    }

    fn parse_statement(&mut self) {
        // Look at the first keyword to determine statement type
        let keyword = self.peek_keyword();

        match keyword.to_uppercase().as_str() {
            "SELECT" => self.parse_select_statement(),
            "INSERT" => self.parse_insert_statement(),
            "UPDATE" => self.parse_update_statement(),
            "DELETE" => self.parse_delete_statement(),
            "CREATE" => self.parse_create_statement(),
            "DROP" => self.parse_drop_statement(),
            "ALTER" => self.parse_alter_statement(),
            _ => {
                // Unknown statement, skip to next semicolon or end
                while self.pos < self.src.len() && self.peek_char() != Some(';') {
                    self.pos += 1;
                }
            }
        }
    }

    fn parse_select_statement(&mut self) {
        // Create Apply node: [SELECT, columns, FROM, table, WHERE, condition]
        self.builder.start_node(Kind::Apply.into());

        // SELECT keyword as receiver
        self.builder.start_node(Kind::ApplyReceiver.into());
        self.parse_keyword_identifier();
        self.builder.finish_node();

        self.skip_whitespace_and_comments();

        // Parse column list
        self.builder.start_node(Kind::ApplyArgument.into());
        self.parse_expression_list();
        self.builder.finish_node();

        self.skip_whitespace_and_comments();

        // FROM keyword
        if self.peek_keyword().to_uppercase() == "FROM" {
            self.builder.start_node(Kind::ApplyArgument.into());
            self.parse_keyword_identifier();
            self.builder.finish_node();

            self.skip_whitespace_and_comments();

            // Table name(s)
            self.builder.start_node(Kind::ApplyArgument.into());
            self.parse_expression_list();
            self.builder.finish_node();

            self.skip_whitespace_and_comments();
        }

        // WHERE clause
        if self.peek_keyword().to_uppercase() == "WHERE" {
            self.builder.start_node(Kind::ApplyArgument.into());
            self.parse_keyword_identifier();
            self.builder.finish_node();

            self.skip_whitespace_and_comments();

            self.builder.start_node(Kind::ApplyArgument.into());
            self.parse_expression();
            self.builder.finish_node();

            self.skip_whitespace_and_comments();
        }

        // ORDER BY clause
        if self.peek_keyword().to_uppercase() == "ORDER" {
            self.builder.start_node(Kind::ApplyArgument.into());
            self.parse_keyword_identifier();
            self.builder.finish_node();

            self.skip_whitespace_and_comments();

            // BY keyword
            if self.peek_keyword().to_uppercase() == "BY" {
                self.builder.start_node(Kind::ApplyArgument.into());
                self.parse_keyword_identifier();
                self.builder.finish_node();

                self.skip_whitespace_and_comments();

                self.builder.start_node(Kind::ApplyArgument.into());
                self.parse_expression_list();
                self.builder.finish_node();

                self.skip_whitespace_and_comments();
            }
        }

        // LIMIT clause
        if self.peek_keyword().to_uppercase() == "LIMIT" {
            self.builder.start_node(Kind::ApplyArgument.into());
            self.parse_keyword_identifier();
            self.builder.finish_node();

            self.skip_whitespace_and_comments();

            self.builder.start_node(Kind::ApplyArgument.into());
            self.parse_expression();
            self.builder.finish_node();
        }

        self.builder.finish_node();
    }

    fn parse_insert_statement(&mut self) {
        // INSERT INTO table (columns) VALUES (values)
        self.builder.start_node(Kind::Apply.into());

        // INSERT keyword as receiver
        self.builder.start_node(Kind::ApplyReceiver.into());
        self.parse_keyword_identifier();
        self.builder.finish_node();

        self.skip_whitespace_and_comments();

        // INTO keyword
        if self.peek_keyword().to_uppercase() == "INTO" {
            self.builder.start_node(Kind::ApplyArgument.into());
            self.parse_keyword_identifier();
            self.builder.finish_node();

            self.skip_whitespace_and_comments();

            // Table name
            self.builder.start_node(Kind::ApplyArgument.into());
            self.parse_identifier();
            self.builder.finish_node();

            self.skip_whitespace_and_comments();
        }

        // Column list (columns)
        if self.peek_char() == Some('(') {
            self.builder.start_node(Kind::ApplyArgument.into());
            self.parse_parenthesized_list();
            self.builder.finish_node();

            self.skip_whitespace_and_comments();
        }

        // VALUES keyword
        if self.peek_keyword().to_uppercase() == "VALUES" {
            self.builder.start_node(Kind::ApplyArgument.into());
            self.parse_keyword_identifier();
            self.builder.finish_node();

            self.skip_whitespace_and_comments();

            // Value list
            self.builder.start_node(Kind::ApplyArgument.into());
            self.parse_parenthesized_list();
            self.builder.finish_node();
        }

        self.builder.finish_node();
    }

    fn parse_update_statement(&mut self) {
        // UPDATE table SET column = value WHERE condition
        self.builder.start_node(Kind::Apply.into());

        // UPDATE keyword as receiver
        self.builder.start_node(Kind::ApplyReceiver.into());
        self.parse_keyword_identifier();
        self.builder.finish_node();

        self.skip_whitespace_and_comments();

        // Table name
        self.builder.start_node(Kind::ApplyArgument.into());
        self.parse_identifier();
        self.builder.finish_node();

        self.skip_whitespace_and_comments();

        // SET keyword
        if self.peek_keyword().to_uppercase() == "SET" {
            self.builder.start_node(Kind::ApplyArgument.into());
            self.parse_keyword_identifier();
            self.builder.finish_node();

            self.skip_whitespace_and_comments();

            // Assignment list
            self.builder.start_node(Kind::ApplyArgument.into());
            self.parse_expression_list();
            self.builder.finish_node();

            self.skip_whitespace_and_comments();
        }

        // WHERE clause
        if self.peek_keyword().to_uppercase() == "WHERE" {
            self.builder.start_node(Kind::ApplyArgument.into());
            self.parse_keyword_identifier();
            self.builder.finish_node();

            self.skip_whitespace_and_comments();

            self.builder.start_node(Kind::ApplyArgument.into());
            self.parse_expression();
            self.builder.finish_node();
        }

        self.builder.finish_node();
    }

    fn parse_delete_statement(&mut self) {
        // DELETE FROM table WHERE condition
        self.builder.start_node(Kind::Apply.into());

        // DELETE keyword as receiver
        self.builder.start_node(Kind::ApplyReceiver.into());
        self.parse_keyword_identifier();
        self.builder.finish_node();

        self.skip_whitespace_and_comments();

        // FROM keyword
        if self.peek_keyword().to_uppercase() == "FROM" {
            self.builder.start_node(Kind::ApplyArgument.into());
            self.parse_keyword_identifier();
            self.builder.finish_node();

            self.skip_whitespace_and_comments();

            // Table name
            self.builder.start_node(Kind::ApplyArgument.into());
            self.parse_identifier();
            self.builder.finish_node();

            self.skip_whitespace_and_comments();
        }

        // WHERE clause
        if self.peek_keyword().to_uppercase() == "WHERE" {
            self.builder.start_node(Kind::ApplyArgument.into());
            self.parse_keyword_identifier();
            self.builder.finish_node();

            self.skip_whitespace_and_comments();

            self.builder.start_node(Kind::ApplyArgument.into());
            self.parse_expression();
            self.builder.finish_node();
        }

        self.builder.finish_node();
    }

    fn parse_create_statement(&mut self) {
        // CREATE TABLE table (columns)
        self.builder.start_node(Kind::Apply.into());

        // CREATE keyword as receiver
        self.builder.start_node(Kind::ApplyReceiver.into());
        self.parse_keyword_identifier();
        self.builder.finish_node();

        self.skip_whitespace_and_comments();

        // TABLE/INDEX/VIEW keyword
        self.builder.start_node(Kind::ApplyArgument.into());
        self.parse_keyword_identifier();
        self.builder.finish_node();

        self.skip_whitespace_and_comments();

        // Table/object name
        self.builder.start_node(Kind::ApplyArgument.into());
        self.parse_identifier();
        self.builder.finish_node();

        self.skip_whitespace_and_comments();

        // Definition (columns)
        if self.peek_char() == Some('(') {
            self.builder.start_node(Kind::ApplyArgument.into());
            self.parse_parenthesized_list();
            self.builder.finish_node();
        }

        self.builder.finish_node();
    }

    fn parse_drop_statement(&mut self) {
        // DROP TABLE table
        self.builder.start_node(Kind::Apply.into());

        // DROP keyword as receiver
        self.builder.start_node(Kind::ApplyReceiver.into());
        self.parse_keyword_identifier();
        self.builder.finish_node();

        self.skip_whitespace_and_comments();

        // TABLE/INDEX/VIEW keyword
        self.builder.start_node(Kind::ApplyArgument.into());
        self.parse_keyword_identifier();
        self.builder.finish_node();

        self.skip_whitespace_and_comments();

        // Object name
        self.builder.start_node(Kind::ApplyArgument.into());
        self.parse_identifier();
        self.builder.finish_node();

        self.builder.finish_node();
    }

    fn parse_alter_statement(&mut self) {
        // ALTER TABLE table action
        self.builder.start_node(Kind::Apply.into());

        // ALTER keyword as receiver
        self.builder.start_node(Kind::ApplyReceiver.into());
        self.parse_keyword_identifier();
        self.builder.finish_node();

        self.skip_whitespace_and_comments();

        // Parse all remaining tokens as arguments
        while self.pos < self.src.len() && self.peek_char() != Some(';') {
            self.skip_whitespace_and_comments();
            if self.pos >= self.src.len() || self.peek_char() == Some(';') {
                break;
            }

            self.builder.start_node(Kind::ApplyArgument.into());
            if self.peek_char() == Some('(') {
                self.parse_parenthesized_list();
            } else if self.is_keyword_start() {
                self.parse_keyword_identifier();
            } else {
                self.parse_expression();
            }
            self.builder.finish_node();
        }

        self.builder.finish_node();
    }

    fn parse_expression_list(&mut self) {
        // Parse comma-separated list of expressions
        loop {
            self.parse_expression();

            self.skip_whitespace_and_comments();

            if self.peek_char() == Some(',') {
                let start = self.pos;
                self.pos += 1;
                let text = &self.src[start..self.pos];
                self.builder.token(Kind::Comma.into(), text);
                self.skip_whitespace_and_comments();
            } else {
                break;
            }
        }
    }

    fn parse_expression(&mut self) {
        // Parse a basic expression (identifier, number, string, or operator)
        let ch = self.peek_char();

        if ch == Some('*') {
            // Wildcard
            let start = self.pos;
            self.pos += 1;
            let text = &self.src[start..self.pos];
            self.builder.start_node(Kind::Identifier.into());
            self.builder.token(Kind::Star.into(), text);
            self.builder.finish_node();
        } else if ch == Some('\'') || ch == Some('"') {
            // String literal
            self.parse_string();
        } else if ch.is_some_and(|c| c.is_ascii_digit()) {
            // Number
            self.parse_number();
        } else if ch.is_some_and(|c| c.is_ascii_alphabetic() || c == '_') {
            // Check if this is a binary operator expression
            self.parse_identifier();

            self.skip_whitespace_and_comments();

            // Check for binary operators
            let op_ch = self.peek_char();
            if op_ch == Some('=') || op_ch == Some('>') || op_ch == Some('<') || op_ch == Some('!')
            {
                // This is a binary operation
                // We already parsed the left side, now parse operator and right side
                self.parse_operator();
                self.skip_whitespace_and_comments();
                self.parse_expression();
            }
        } else if ch == Some('(') {
            // Parenthesized expression or subquery
            self.parse_parenthesized_list();
        } else {
            // Unknown, skip
            if ch.is_some() {
                self.pos += 1;
            }
        }
    }

    fn parse_operator(&mut self) {
        let start = self.pos;

        // Handle multi-character operators
        let ch = self.peek_char();
        if ch == Some('=') {
            self.pos += 1;
            let text = &self.src[start..self.pos];
            self.builder.token(Kind::Equal.into(), text);
        } else if ch == Some('>') {
            self.pos += 1;
            if self.peek_char() == Some('=') {
                self.pos += 1;
            }
            let text = &self.src[start..self.pos];
            self.builder.token(Kind::Greater.into(), text);
        } else if ch == Some('<') {
            self.pos += 1;
            if self.peek_char() == Some('=') || self.peek_char() == Some('>') {
                self.pos += 1;
            }
            let text = &self.src[start..self.pos];
            self.builder.token(Kind::Less.into(), text);
        } else if ch == Some('!') {
            self.pos += 1;
            if self.peek_char() == Some('=') {
                self.pos += 1;
            }
            let text = &self.src[start..self.pos];
            self.builder.token(Kind::Bang.into(), text);
        }
    }

    fn parse_parenthesized_list(&mut self) {
        // Parse (...)
        let start = self.pos;
        if self.peek_char() != Some('(') {
            return;
        }

        self.pos += 1;
        let lparen = &self.src[start..self.pos];
        self.builder.token(Kind::LParen.into(), lparen);

        self.skip_whitespace_and_comments();

        // Parse contents - keep consuming until we hit )
        while self.pos < self.src.len() && self.peek_char() != Some(')') {
            // Parse everything until comma or )
            self.parse_list_item();

            self.skip_whitespace_and_comments();

            if self.peek_char() == Some(',') {
                let comma_start = self.pos;
                self.pos += 1;
                let comma_text = &self.src[comma_start..self.pos];
                self.builder.token(Kind::Comma.into(), comma_text);
                self.skip_whitespace_and_comments();
            }
        }

        if self.peek_char() == Some(')') {
            let rparen_start = self.pos;
            self.pos += 1;
            let rparen = &self.src[rparen_start..self.pos];
            self.builder.token(Kind::RParen.into(), rparen);
        }
    }

    fn parse_list_item(&mut self) {
        // Parse a list item - everything until comma or closing paren
        // This handles complex column definitions like "id INTEGER PRIMARY KEY"
        while self.pos < self.src.len() {
            let ch = self.peek_char();
            if ch == Some(',') || ch == Some(')') {
                break;
            } else if ch == Some('(') {
                // Nested parentheses
                self.parse_parenthesized_list();
            } else if ch == Some('\'') || ch == Some('"') {
                self.parse_string();
            } else if ch.is_some_and(|c| c.is_ascii_digit()) {
                self.parse_number();
            } else if ch.is_some_and(|c| c.is_ascii_whitespace()) {
                self.skip_whitespace_and_comments();
            } else if ch.is_some_and(|c| c.is_ascii_alphabetic() || c == '_') {
                self.parse_identifier();
                self.skip_whitespace_and_comments();
            } else {
                // Unknown character, skip it
                self.pos += 1;
            }
        }
    }

    fn parse_identifier(&mut self) {
        let start = self.pos;

        // Handle quoted identifiers
        if self.peek_char() == Some('"') || self.peek_char() == Some('`') {
            let quote = self.peek_char().unwrap();
            self.pos += 1;
            while self.pos < self.src.len() && self.peek_char() != Some(quote) {
                self.pos += 1;
            }
            if self.peek_char() == Some(quote) {
                self.pos += 1;
            }
        } else {
            // Regular identifier
            while self.pos < self.src.len() {
                let ch = self.src.as_bytes()[self.pos];
                if ch.is_ascii_alphanumeric() || ch == b'_' || ch == b'.' {
                    self.pos += 1;
                } else {
                    break;
                }
            }
        }

        let text = &self.src[start..self.pos];

        self.builder.start_node(Kind::Identifier.into());
        self.builder.token(Kind::Identifier.into(), text);
        self.builder.finish_node();
    }

    fn parse_keyword_identifier(&mut self) {
        // Parse a keyword and wrap it as an identifier
        self.parse_identifier();
    }

    fn parse_string(&mut self) {
        let quote = self.peek_char().unwrap();
        let start = self.pos;
        self.pos += 1; // Skip opening quote

        while self.pos < self.src.len() {
            let ch = self.peek_char().unwrap();
            if ch == quote {
                self.pos += 1;
                break;
            } else if ch == '\\' {
                self.pos += 2; // Skip escape sequence
            } else {
                self.pos += 1;
            }
        }

        let text = &self.src[start..self.pos];

        self.builder.start_node(Kind::Literal.into());
        self.builder.start_node(Kind::StringContent.into());
        self.builder.token(Kind::StringContent.into(), text);
        self.builder.finish_node();
        self.builder.finish_node();
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

    fn peek_keyword(&self) -> String {
        let mut end = self.pos;
        while end < self.src.len() {
            let ch = self.src.as_bytes()[end];
            if ch.is_ascii_alphanumeric() || ch == b'_' {
                end += 1;
            } else {
                break;
            }
        }
        self.src[self.pos..end].to_string()
    }

    fn is_keyword_start(&self) -> bool {
        self.peek_char().is_some_and(|c| c.is_ascii_alphabetic())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cadenza_eval::{BuiltinMacro, Compiler, Env, Type, Value, eval};

    #[test]
    fn test_execute_sql_with_macros() {
        let sql = "SELECT * FROM users WHERE age > 18";

        // Parse SQL directly into Cadenza AST
        let parse_result = parse(sql);
        let root = parse_result.ast();

        let mut compiler = Compiler::new();
        let mut env = Env::new();

        // Register SELECT macro
        compiler.define_macro(
            "SELECT".into(),
            Value::BuiltinMacro(BuiltinMacro {
                name: "SELECT",
                signature: Type::function(vec![Type::Unknown], Type::Nil),
                func: |_args, _ctx| Ok(Value::Nil),
            }),
        );

        // Evaluate - eval doesn't care this came from SQL!
        let results = eval(&root, &mut env, &mut compiler);
        assert!(!results.is_empty());
    }
}
