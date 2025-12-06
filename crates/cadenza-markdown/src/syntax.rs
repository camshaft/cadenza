//! Markdown lexer and parser that produces Cadenza-compatible AST.
//!
//! This module treats Markdown as an alternative syntax for Cadenza. It lexes and parses
//! Markdown into a Rowan CST that can be directly evaluated by the Cadenza eval crate.
//!
//! # Architecture
//!
//! - **Lexer**: Tokenizes Markdown (headings, paragraphs, code blocks, etc.)
//! - **Parser**: Builds Rowan GreenNode CST using cadenza-syntax token kinds
//! - **AST**: Markdown elements become Apply nodes that call macros with content
//!
//! # Example
//!
//! ```
//! use cadenza_markdown::parse;
//! use cadenza_eval::eval;
//!
//! let markdown = "# Hello\n\nWorld!";
//! let root = parse(markdown);
//! // eval() doesn't care that this came from Markdown - it's just an AST
//! ```

use cadenza_syntax::{parse::Parse, token::Kind};
use rowan::GreenNodeBuilder;

/// Parse Markdown source into a Cadenza-compatible AST.
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

        loop {
            self.skip_blank_lines();
            if self.pos >= self.src.len() {
                break;
            }

            // Check what kind of element we're parsing
            if self.is_heading() {
                self.parse_heading();
            } else if self.is_code_fence() {
                self.parse_code_fence();
            } else if self.is_list_item() {
                self.parse_list();
            } else {
                // Default to paragraph
                self.parse_paragraph();
            }
        }

        self.builder.finish_node();

        Parse {
            green: self.builder.finish(),
            errors: self.errors,
        }
    }

    fn skip_blank_lines(&mut self) {
        while self.pos < self.src.len() {
            let ch = self.peek_char();
            if ch == Some('\n') || ch == Some('\r') {
                self.skip_newline();
            } else if ch == Some(' ') || ch == Some('\t') {
                // Check if this line is all whitespace
                let line_start = self.pos;
                self.skip_line_whitespace();
                if self.peek_char() == Some('\n') || self.peek_char() == Some('\r') || self.peek_char().is_none() {
                    self.skip_newline();
                } else {
                    // Not a blank line, restore position
                    self.pos = line_start;
                    break;
                }
            } else {
                break;
            }
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

    fn is_heading(&self) -> bool {
        // Headings start with # at the beginning of a line
        self.peek_char() == Some('#')
    }

    fn is_code_fence(&self) -> bool {
        // Code fences start with ``` or ~~~
        (self.peek_char() == Some('`') && self.peek_ahead(1) == Some('`') && self.peek_ahead(2) == Some('`'))
            || (self.peek_char() == Some('~') && self.peek_ahead(1) == Some('~') && self.peek_ahead(2) == Some('~'))
    }

    fn is_list_item(&self) -> bool {
        // List items start with - or * followed by space
        (self.peek_char() == Some('-') || self.peek_char() == Some('*')) 
            && self.peek_ahead(1) == Some(' ')
    }

    fn parse_heading(&mut self) {
        // Count the number of # characters to determine heading level
        let hash_start = self.pos;
        let mut level = 0;
        while self.pos < self.src.len() && self.peek_char() == Some('#') && level < 6 {
            self.pos += 1;
            level += 1;
        }

        // Heading must be followed by space
        if self.peek_char() != Some(' ') {
            // Not a valid heading, treat as paragraph
            self.pos = hash_start;
            self.parse_paragraph();
            return;
        }

        // Capture the hash markers
        let hash_text = &self.src[hash_start..self.pos];

        // Skip the space after #
        let space_start = self.pos;
        self.pos += 1;
        let space_text = &self.src[space_start..self.pos];

        // Create Apply node: [###, content] where # count indicates level
        self.builder.start_node(Kind::Apply.into());

        // Hash markers as receiver (identifier)
        self.builder.start_node(Kind::ApplyReceiver.into());
        self.builder.start_node(Kind::Identifier.into());
        self.builder.token(Kind::Identifier.into(), hash_text);
        self.builder.finish_node();
        self.builder.finish_node();

        // Emit space as trivia
        self.builder.token(Kind::Space.into(), space_text);

        // Heading text as literal argument
        self.builder.start_node(Kind::ApplyArgument.into());
        let content_start = self.pos;
        
        // Read until end of line
        while self.pos < self.src.len() {
            let ch = self.peek_char().unwrap();
            if ch == '\n' || ch == '\r' {
                break;
            }
            self.pos += 1;
        }

        let content = &self.src[content_start..self.pos];
        // Emit content directly from source
        self.builder.start_node(Kind::Literal.into());
        self.builder.start_node(Kind::StringContent.into());
        self.builder.token(Kind::StringContent.into(), content);
        self.builder.finish_node();
        self.builder.finish_node();
        
        self.builder.finish_node();

        self.builder.finish_node();

        // Skip the newline
        if self.peek_char().is_some() {
            self.skip_newline();
        }
    }

    fn parse_code_fence(&mut self) {
        let fence_char = self.peek_char().unwrap();
        let fence_start = self.pos;

        // Skip opening fence (``` or ~~~)
        self.pos += 3;
        let fence_text = &self.src[fence_start..self.pos];
        
        // Parse language identifier
        let lang_start = self.pos;
        while self.pos < self.src.len() {
            let ch = self.peek_char().unwrap();
            if ch == ' ' || ch == '\n' || ch == '\r' {
                break;
            }
            self.pos += 1;
        }
        let lang_end = self.pos;

        // Skip rest of line (parameters are not supported yet)
        while self.pos < self.src.len() && self.peek_char() != Some('\n') && self.peek_char() != Some('\r') {
            self.pos += 1;
        }
        let line_end = self.pos;
        
        self.skip_newline();

        // Read code content until closing fence
        let code_start = self.pos;
        
        while self.pos < self.src.len() {
            // Check if we're at the closing fence
            if self.peek_char() == Some(fence_char) 
                && self.peek_ahead(1) == Some(fence_char) 
                && self.peek_ahead(2) == Some(fence_char) {
                break;
            }
            
            self.pos += 1;
        }

        let code_end = self.pos;

        // Skip closing fence if present
        let close_fence_start = self.pos;
        if self.pos < self.src.len() {
            self.pos += 3;
        }

        // Skip to end of line (but don't consume newline yet)
        while self.pos < self.src.len() && self.peek_char() != Some('\n') && self.peek_char() != Some('\r') {
            self.pos += 1;
        }
        
        let fence_end = self.pos;
        
        // Now consume the newline if present
        if self.peek_char().is_some() {
            self.skip_newline();
        }

        // Create Apply node: [```, language, content]
        self.builder.start_node(Kind::Apply.into());

        // Fence markers as receiver (identifier)
        self.builder.start_node(Kind::ApplyReceiver.into());
        self.builder.start_node(Kind::Identifier.into());
        self.builder.token(Kind::Identifier.into(), fence_text);
        self.builder.finish_node();
        self.builder.finish_node();

        // Emit language and rest of first line as trivia
        let first_line_rest = &self.src[lang_start..line_end];
        self.builder.token(Kind::CommentContent.into(), first_line_rest);

        // Language as first argument
        self.builder.start_node(Kind::ApplyArgument.into());
        let language = &self.src[lang_start..lang_end];
        self.builder.start_node(Kind::Literal.into());
        self.builder.start_node(Kind::StringContent.into());
        self.builder.token(Kind::StringContent.into(), language);
        self.builder.finish_node();
        self.builder.finish_node();
        self.builder.finish_node();

        // Code content as second argument
        self.builder.start_node(Kind::ApplyArgument.into());
        let code_content = &self.src[code_start..code_end];
        self.builder.start_node(Kind::Literal.into());
        self.builder.start_node(Kind::StringContent.into());
        self.builder.token(Kind::StringContent.into(), code_content);
        self.builder.finish_node();
        self.builder.finish_node();
        self.builder.finish_node();

        // Emit closing fence and rest of line as trivia
        let closing_text = &self.src[close_fence_start..fence_end];
        self.builder.token(Kind::CommentContent.into(), closing_text);

        self.builder.finish_node();
    }

    fn parse_list(&mut self) {
        // Start list Apply node: [-, items...] using first list marker as identifier
        self.builder.start_node(Kind::Apply.into());

        let marker_char = self.peek_char().unwrap(); // - or *

        // Parse each list item
        let mut first = true;
        while self.pos < self.src.len() && self.is_list_item() {
            let marker_start = self.pos;
            
            // Consume marker (- or *)
            self.pos += 1;
            let marker_text = &self.src[marker_start..self.pos];
            
            // Consume space
            let space_start = self.pos;
            self.pos += 1;
            let space_text = &self.src[space_start..self.pos];

            if first {
                // First marker becomes the receiver
                self.builder.start_node(Kind::ApplyReceiver.into());
                self.builder.start_node(Kind::Identifier.into());
                self.builder.token(Kind::Identifier.into(), marker_text);
                self.builder.finish_node();
                self.builder.finish_node();
                
                // Emit space as trivia
                self.builder.token(Kind::Space.into(), space_text);
                
                first = false;
            } else {
                // Subsequent markers emitted as trivia
                self.builder.token(Kind::CommentContent.into(), marker_text);
                self.builder.token(Kind::Space.into(), space_text);
            }

            // Parse list item content as argument
            self.builder.start_node(Kind::ApplyArgument.into());
            let content_start = self.pos;
            
            // Read until end of line
            while self.pos < self.src.len() {
                let ch = self.peek_char().unwrap();
                if ch == '\n' || ch == '\r' {
                    break;
                }
                self.pos += 1;
            }

            let content = &self.src[content_start..self.pos];
            self.builder.start_node(Kind::Literal.into());
            self.builder.start_node(Kind::StringContent.into());
            self.builder.token(Kind::StringContent.into(), content);
            self.builder.finish_node();
            self.builder.finish_node();
            self.builder.finish_node();

            // Skip newline
            if self.peek_char().is_some() {
                self.skip_newline();
            }

            // Check if next line is a list item or blank
            let save_pos = self.pos;
            self.skip_line_whitespace();
            if self.peek_char() == Some('\n') || self.peek_char() == Some('\r') || !self.is_list_item() {
                self.pos = save_pos;
                break;
            }
            self.pos = save_pos;
        }

        self.builder.finish_node(); // End Apply
    }

    fn parse_paragraph(&mut self) {
        // For paragraphs, emit the content directly as StringContent from source
        let content_start = self.pos;
        
        // Read until blank line or special element
        while self.pos < self.src.len() {
            // Check for end conditions at start of line
            if self.is_heading() || self.is_code_fence() {
                break;
            }

            // Read the line
            while self.pos < self.src.len() {
                let ch = self.peek_char().unwrap();
                if ch == '\n' || ch == '\r' {
                    break;
                }
                self.pos += 1;
            }

            // Check if we hit end of source
            if self.pos >= self.src.len() {
                break;
            }

            // Consume newline
            if self.peek_char() == Some('\r') {
                self.pos += 1;
                if self.peek_char() == Some('\n') {
                    self.pos += 1;
                }
            } else if self.peek_char() == Some('\n') {
                self.pos += 1;
            }

            // Check if next line is blank (paragraph break)
            let save_pos = self.pos;
            let mut found_content = false;
            while self.pos < self.src.len() {
                let ch = self.peek_char().unwrap();
                if ch == ' ' || ch == '\t' {
                    self.pos += 1;
                } else if ch == '\n' || ch == '\r' {
                    // Blank line - end of paragraph
                    break;
                } else {
                    // Non-whitespace content - continue paragraph
                    found_content = true;
                    break;
                }
            }
            
            if !found_content {
                // Found blank line or end - restore position and end paragraph
                // Strip the trailing newline from paragraph by backing up
                self.pos = save_pos;
                if self.pos > content_start {
                    // Back up over the newline we just consumed
                    if self.pos >= 2 && &self.src[self.pos-2..self.pos] == "\r\n" {
                        self.pos -= 2;
                    } else if self.pos >= 1 {
                        let prev_char = self.src.as_bytes()[self.pos - 1];
                        if prev_char == b'\n' || prev_char == b'\r' {
                            self.pos -= 1;
                        }
                    }
                }
                break;
            }
            
            // Continue with next line
            self.pos = save_pos;
        }

        // Emit paragraph content from source
        let content = &self.src[content_start..self.pos];
        self.builder.start_node(Kind::Literal.into());
        self.builder.start_node(Kind::StringContent.into());
        self.builder.token(Kind::StringContent.into(), content);
        self.builder.finish_node();
        self.builder.finish_node();
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use cadenza_eval::{BuiltinMacro, Compiler, Env, Type, Value, eval};

    #[test]
    fn test_execute_markdown_with_macros() {
        let markdown = "# Hello\n\nWorld!";

        // Parse Markdown directly into Cadenza AST
        let parse_result = parse(markdown);
        let root = parse_result.ast();

        let mut compiler = Compiler::new();
        let mut env = Env::new();

        // Register h1 macro
        compiler.define_macro(
            "h1".into(),
            Value::BuiltinMacro(BuiltinMacro {
                name: "h1",
                signature: Type::function(vec![Type::String], Type::Nil),
                func: |_args, _ctx| Ok(Value::Nil),
            }),
        );

        // Register p macro
        compiler.define_macro(
            "p".into(),
            Value::BuiltinMacro(BuiltinMacro {
                name: "p",
                signature: Type::function(vec![Type::String], Type::Nil),
                func: |_args, _ctx| Ok(Value::Nil),
            }),
        );

        // Evaluate - eval doesn't care this came from Markdown!
        let results = eval(&root, &mut env, &mut compiler);
        assert_eq!(results.len(), 2);
    }
}
