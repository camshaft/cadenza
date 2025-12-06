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

type SyntaxToken = rowan::SyntaxToken<cadenza_syntax::Lang>;

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
                if self.peek_char() == Some('\n')
                    || self.peek_char() == Some('\r')
                    || self.peek_char().is_none()
                {
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
        // Note: This method treats bytes as chars, which is safe because it's only used
        // to check for ASCII markdown syntax (like #, `, -, etc.). Content is always
        // handled as &str slices which properly preserve UTF-8.
        if self.pos < self.src.len() {
            Some(self.src.as_bytes()[self.pos] as char)
        } else {
            None
        }
    }

    fn peek_ahead(&self, offset: usize) -> Option<char> {
        // Note: See peek_char comment about ASCII-only usage
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
        (self.peek_char() == Some('`')
            && self.peek_ahead(1) == Some('`')
            && self.peek_ahead(2) == Some('`'))
            || (self.peek_char() == Some('~')
                && self.peek_ahead(1) == Some('~')
                && self.peek_ahead(2) == Some('~'))
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

        // Capture the hash markers and emit as trivia
        let hash_text = &self.src[hash_start..self.pos];
        self.builder.token(Kind::CommentContent.into(), hash_text);

        // Skip and emit the space after #
        let space_start = self.pos;
        self.pos += 1;
        let space_text = &self.src[space_start..self.pos];
        self.builder.token(Kind::Space.into(), space_text);

        // Create Apply node with synthetic heading token
        self.builder.start_node(Kind::Apply.into());

        // Use synthetic token based on heading level
        self.builder.start_node(Kind::ApplyReceiver.into());
        let synthetic_kind = match level {
            1 => Kind::SyntheticMarkdownH1,
            2 => Kind::SyntheticMarkdownH2,
            3 => Kind::SyntheticMarkdownH3,
            4 => Kind::SyntheticMarkdownH4,
            5 => Kind::SyntheticMarkdownH5,
            6 => Kind::SyntheticMarkdownH6,
            _ => Kind::SyntheticMarkdownH1, // fallback
        };
        self.builder.start_node(synthetic_kind.into());
        self.builder.finish_node();
        self.builder.finish_node();

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
        // Parse inline elements in the heading content
        self.parse_inline_content(content, content_start);

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

        // Skip opening fence (``` or ~~~) and emit as trivia
        self.pos += 3;
        let fence_text = &self.src[fence_start..self.pos];
        self.builder.token(Kind::CommentContent.into(), fence_text);

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
        let language = &self.src[lang_start..lang_end];

        // Parse parameters (space-separated tokens after language)
        let mut parameters = Vec::new();
        while self.pos < self.src.len() {
            let ch = self.peek_char();
            if ch == Some('\n') || ch == Some('\r') {
                break;
            }
            if ch == Some(' ') || ch == Some('\t') {
                self.pos += 1;
                continue;
            }

            // Found a parameter
            let param_start = self.pos;
            while self.pos < self.src.len() {
                match self.peek_char() {
                    Some(ch) if ch == ' ' || ch == '\t' || ch == '\n' || ch == '\r' => break,
                    Some(_) => self.pos += 1,
                    None => break,
                }
            }
            let param_end = self.pos;
            if param_end > param_start {
                parameters.push(&self.src[param_start..param_end]);
            }
        }

        let line_end = self.pos;

        // Emit language and rest of first line as trivia
        let first_line_rest = &self.src[lang_start..line_end];
        self.builder
            .token(Kind::CommentContent.into(), first_line_rest);

        self.skip_newline();

        // Read code content until closing fence
        let code_start = self.pos;

        while self.pos < self.src.len() {
            // Check if we're at the closing fence
            if self.peek_char() == Some(fence_char)
                && self.peek_ahead(1) == Some(fence_char)
                && self.peek_ahead(2) == Some(fence_char)
            {
                break;
            }

            self.pos += 1;
        }

        let code_end = self.pos;
        let code_content = &self.src[code_start..code_end];

        // Skip closing fence if present
        let close_fence_start = self.pos;
        if self.pos < self.src.len() {
            self.pos += 3;
        }

        // Skip to end of line (but don't consume newline yet)
        while self.pos < self.src.len()
            && self.peek_char() != Some('\n')
            && self.peek_char() != Some('\r')
        {
            self.pos += 1;
        }

        let fence_end = self.pos;

        // Emit closing fence and rest of line as trivia
        let closing_text = &self.src[close_fence_start..fence_end];
        self.builder
            .token(Kind::CommentContent.into(), closing_text);

        // Now consume the newline if present
        if self.peek_char().is_some() {
            self.skip_newline();
        }

        // Create Apply node with synthetic code token
        self.builder.start_node(Kind::Apply.into());

        // Use synthetic code token as receiver
        self.builder.start_node(Kind::ApplyReceiver.into());
        self.builder.start_node(Kind::SyntheticMarkdownCode.into());
        self.builder.finish_node();
        self.builder.finish_node();

        // Language as first argument
        self.builder.start_node(Kind::ApplyArgument.into());
        self.builder.start_node(Kind::Literal.into());
        self.builder.start_node(Kind::StringContent.into());
        self.builder.token(Kind::StringContent.into(), language);
        self.builder.finish_node();
        self.builder.finish_node();
        self.builder.finish_node();

        // Code content as second argument
        // Special case: if language is empty or "cadenza", parse as Cadenza code
        self.builder.start_node(Kind::ApplyArgument.into());
        if language.is_empty() || language == "cadenza" {
            // Parse the code content as Cadenza
            let cadenza_parse = cadenza_syntax::parse::parse(code_content);
            // Get the root node from the parsed Cadenza code
            let cadenza_root = cadenza_parse.syntax();

            // Wrap the Cadenza statements in a synthetic block
            self.builder.start_node(Kind::Apply.into());
            self.builder.start_node(Kind::ApplyReceiver.into());
            self.builder.start_node(Kind::SyntheticBlock.into());
            self.builder.finish_node();
            self.builder.finish_node();

            // Add each statement as an argument to the block
            for child in cadenza_root.children_with_tokens() {
                self.builder.start_node(Kind::ApplyArgument.into());
                self.add_element_recursive(child);
                self.builder.finish_node();
            }

            self.builder.finish_node(); // End Apply (block)
        } else {
            // Non-Cadenza code: emit as string content
            self.builder.start_node(Kind::Literal.into());
            self.builder.start_node(Kind::StringContent.into());
            self.builder.token(Kind::StringContent.into(), code_content);
            self.builder.finish_node();
            self.builder.finish_node();
        }
        self.builder.finish_node();

        // Add parameters as additional arguments
        for param in parameters {
            self.builder.start_node(Kind::ApplyArgument.into());
            self.builder.start_node(Kind::Literal.into());
            self.builder.start_node(Kind::StringContent.into());
            self.builder.token(Kind::StringContent.into(), param);
            self.builder.finish_node();
            self.builder.finish_node();
            self.builder.finish_node();
        }

        self.builder.finish_node();
    }

    // Helper to recursively add elements from another parsed tree
    fn add_element_recursive(
        &mut self,
        element: rowan::NodeOrToken<cadenza_syntax::SyntaxNode, SyntaxToken>,
    ) {
        match element {
            rowan::NodeOrToken::Node(node) => {
                self.builder.start_node(node.kind().into());
                for child in node.children_with_tokens() {
                    self.add_element_recursive(child);
                }
                self.builder.finish_node();
            }
            rowan::NodeOrToken::Token(token) => {
                // Just use the token's text content - Rowan will handle positions automatically
                self.builder.token(token.kind().into(), token.text());
            }
        }
    }

    fn parse_list(&mut self) {
        // Start list Apply node with synthetic ul token
        self.builder.start_node(Kind::Apply.into());

        let _marker_char = self.peek_char().unwrap(); // - or *

        // Use synthetic list token as receiver
        self.builder.start_node(Kind::ApplyReceiver.into());
        self.builder.start_node(Kind::SyntheticMarkdownList.into());
        self.builder.finish_node();
        self.builder.finish_node();

        // Parse each list item
        while self.pos < self.src.len() && self.is_list_item() {
            let marker_start = self.pos;

            // Consume marker (- or *)
            self.pos += 1;
            let marker_text = &self.src[marker_start..self.pos];

            // Emit marker as trivia
            self.builder.token(Kind::CommentContent.into(), marker_text);

            // Consume space
            let space_start = self.pos;
            self.pos += 1;
            let space_text = &self.src[space_start..self.pos];

            // Emit space as trivia
            self.builder.token(Kind::Space.into(), space_text);

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
            // Parse inline elements in list item content
            self.parse_inline_content(content, content_start);
            self.builder.finish_node();

            // Skip newline
            if self.peek_char().is_some() {
                self.skip_newline();
            }

            // Check if next line is a list item or blank
            let save_pos = self.pos;
            self.skip_line_whitespace();
            if self.peek_char() == Some('\n')
                || self.peek_char() == Some('\r')
                || !self.is_list_item()
            {
                self.pos = save_pos;
                break;
            }
            self.pos = save_pos;
        }

        self.builder.finish_node(); // End Apply
    }

    fn parse_paragraph(&mut self) {
        // For paragraphs, wrap in an Apply node with synthetic p token
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
                    if self.pos >= 2 && &self.src[self.pos - 2..self.pos] == "\r\n" {
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

        // Wrap paragraph in Apply node with synthetic p token
        self.builder.start_node(Kind::Apply.into());

        // Use synthetic paragraph token as receiver
        self.builder.start_node(Kind::ApplyReceiver.into());
        self.builder
            .start_node(Kind::SyntheticMarkdownParagraph.into());
        self.builder.finish_node();
        self.builder.finish_node();

        // Emit paragraph content as argument
        self.builder.start_node(Kind::ApplyArgument.into());
        let content = &self.src[content_start..self.pos];
        // Parse inline elements in the paragraph content
        self.parse_inline_content(content, content_start);
        self.builder.finish_node();

        self.builder.finish_node();
    }

    /// Parse inline elements within a text content range.
    /// Returns the content as either a simple string or a list of mixed inline elements.
    fn parse_inline_content(&mut self, content: &str, content_start: usize) {
        // Check if there are any inline elements in the content
        if !self.has_inline_elements(content) {
            // No inline elements, just emit as string
            self.builder.start_node(Kind::Literal.into());
            self.builder.start_node(Kind::StringContent.into());
            self.builder.token(Kind::StringContent.into(), content);
            self.builder.finish_node();
            self.builder.finish_node();
        } else {
            // Has inline elements, parse them
            self.parse_inline_elements(content, content_start);
        }
    }

    /// Check if content has any inline elements
    fn has_inline_elements(&self, content: &str) -> bool {
        content.contains('*') || content.contains('`')
    }

    /// Parse inline elements and emit as a list structure
    fn parse_inline_elements(&mut self, content: &str, _base_offset: usize) {
        // Emit as a synthetic list to hold mixed content
        self.builder.start_node(Kind::Apply.into());
        self.builder.start_node(Kind::ApplyReceiver.into());
        self.builder.start_node(Kind::SyntheticList.into());
        self.builder.finish_node();
        self.builder.finish_node();

        let mut pos = 0;
        let bytes = content.as_bytes();

        while pos < bytes.len() {
            // Check for inline code first (highest priority)
            if bytes[pos] == b'`' {
                let code_start = pos;
                pos += 1;

                // Find closing backtick
                let mut found_close = false;
                let code_content_start = pos;
                while pos < bytes.len() {
                    if bytes[pos] == b'`' {
                        found_close = true;
                        break;
                    }
                    pos += 1;
                }

                if found_close {
                    let code_content = &content[code_content_start..pos];

                    // Emit opening backtick as trivia
                    self.builder.token(Kind::CommentContent.into(), "`");

                    // Emit inline code as Apply node
                    self.builder.start_node(Kind::ApplyArgument.into());
                    self.builder.start_node(Kind::Apply.into());
                    self.builder.start_node(Kind::ApplyReceiver.into());
                    self.builder
                        .start_node(Kind::SyntheticMarkdownCodeInline.into());
                    self.builder.finish_node();
                    self.builder.finish_node();

                    self.builder.start_node(Kind::ApplyArgument.into());
                    self.builder.start_node(Kind::Literal.into());
                    self.builder.start_node(Kind::StringContent.into());
                    self.builder.token(Kind::StringContent.into(), code_content);
                    self.builder.finish_node();
                    self.builder.finish_node();
                    self.builder.finish_node();

                    self.builder.finish_node();
                    self.builder.finish_node();

                    // Skip closing backtick and emit as trivia
                    pos += 1;
                    self.builder.token(Kind::CommentContent.into(), "`");
                    continue;
                } else {
                    // No closing backtick, treat as regular text
                    pos = code_start;
                }
            }

            // Check for emphasis (** or *)
            if bytes[pos] == b'*' {
                let star_start = pos;

                // Check if it's bold (**) or italic (*)
                let is_bold = pos + 1 < bytes.len() && bytes[pos + 1] == b'*';
                let marker_len = if is_bold { 2 } else { 1 };
                let marker = if is_bold { "**" } else { "*" };

                pos += marker_len;

                // Find closing marker
                let content_start = pos;
                let mut found_close = false;

                while pos < bytes.len() {
                    if bytes[pos] == b'*'
                        && (!is_bold || (pos + 1 < bytes.len() && bytes[pos + 1] == b'*'))
                    {
                        found_close = true;
                        break;
                    }
                    pos += 1;
                }

                if found_close {
                    let emphasis_content = &content[content_start..pos];

                    // Emit opening marker as trivia
                    self.builder.token(Kind::CommentContent.into(), marker);

                    // Emit emphasis as Apply node
                    self.builder.start_node(Kind::ApplyArgument.into());
                    self.builder.start_node(Kind::Apply.into());
                    self.builder.start_node(Kind::ApplyReceiver.into());

                    if is_bold {
                        self.builder
                            .start_node(Kind::SyntheticMarkdownStrong.into());
                    } else {
                        self.builder
                            .start_node(Kind::SyntheticMarkdownEmphasis.into());
                    }
                    self.builder.finish_node();
                    self.builder.finish_node();

                    // Parse the emphasis content recursively to support nested inline elements
                    self.builder.start_node(Kind::ApplyArgument.into());
                    self.parse_inline_content(emphasis_content, 0);
                    self.builder.finish_node();

                    self.builder.finish_node();
                    self.builder.finish_node();

                    // Skip closing marker and emit as trivia
                    pos += marker_len;
                    self.builder.token(Kind::CommentContent.into(), marker);
                    continue;
                } else {
                    // No closing marker, treat as regular text
                    pos = star_start;
                }
            }

            // Regular text - collect until next special character
            let text_start = pos;
            while pos < bytes.len() && bytes[pos] != b'*' && bytes[pos] != b'`' {
                pos += 1;
            }

            if pos > text_start {
                let text = &content[text_start..pos];
                self.builder.start_node(Kind::ApplyArgument.into());
                self.builder.start_node(Kind::Literal.into());
                self.builder.start_node(Kind::StringContent.into());
                self.builder.token(Kind::StringContent.into(), text);
                self.builder.finish_node();
                self.builder.finish_node();
                self.builder.finish_node();
            }

            // If we didn't advance (e.g., malformed marker), advance by 1 to avoid infinite loop
            if pos == text_start && pos < bytes.len() {
                let char = &content[pos..pos + 1];
                self.builder.start_node(Kind::ApplyArgument.into());
                self.builder.start_node(Kind::Literal.into());
                self.builder.start_node(Kind::StringContent.into());
                self.builder.token(Kind::StringContent.into(), char);
                self.builder.finish_node();
                self.builder.finish_node();
                self.builder.finish_node();
                pos += 1;
            }
        }

        self.builder.finish_node();
    }
}
