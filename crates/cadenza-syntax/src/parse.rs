use crate::{
    SyntaxNode,
    iter::Peek2,
    lexer::Lexer,
    span::Span,
    token::{Kind, Token},
};
use rowan::{GreenNode, GreenNodeBuilder};

#[derive(Debug)]
pub struct ParseError {
    pub span: Span,
    pub message: String,
}

pub struct Parse {
    pub green: GreenNode,
    pub errors: Vec<ParseError>,
}

impl Parse {
    pub fn syntax(&self) -> SyntaxNode {
        rowan::SyntaxNode::new_root(self.green.clone())
    }

    pub fn ast(&self) -> crate::ast::Root {
        crate::ast::Root::cast(self.syntax()).unwrap()
    }
}

pub fn parse(src: &str) -> Parse {
    Parser::new(src).parse()
}

struct Parser<'src> {
    src: &'src str,
    tokens: Peek2<Lexer<'src>>,
    builder: GreenNodeBuilder<'static>,
    errors: Vec<ParseError>,
    whitespace: Whitespace,
}

impl<'src> Parser<'src> {
    fn new(src: &'src str) -> Self {
        Self {
            src,
            tokens: Peek2::new(Lexer::new(src)),
            builder: GreenNodeBuilder::new(),
            errors: Vec::new(),
            whitespace: Default::default(),
        }
    }

    fn parse(mut self) -> Parse {
        self.builder.start_node(Kind::Root.into());

        while self.current() != Kind::Eof {
            // Skip leading trivia before creating the marker, so the marker
            // reflects the actual starting position of the expression.
            // This fixes the bug where `\nfoo 123 456` was parsed as
            // `[foo, [123, 456]]` (two separate expressions) instead of
            // the correct `[[foo, 123], 456]` (one nested expression).
            self.skip_trivia();
            let marker = self.whitespace.marker();
            self.parse_expression(marker);
        }

        self.builder.finish_node();

        Parse {
            green: self.builder.finish(),
            errors: self.errors,
        }
    }

    fn parse_expression(&mut self, marker: impl Marker) {
        self.parse_expression_bp(0, marker);
    }

    /// Parse expression with binding power (Pratt parsing)
    fn parse_expression_bp(&mut self, min_bp: u8, marker: impl Marker) {
        marker.start(self);

        // Parse the left-hand side (check for prefix operators first)
        self.skip_trivia();

        // Take checkpoint before parsing primary (for the Apply node)
        let apply_checkpoint = self.builder.checkpoint();

        // Take checkpoint for content only (excludes leading trivia)
        let content_checkpoint = self.builder.checkpoint();

        // Check for prefix operators
        if let Some(prefix_bp) = self.current().prefix_binding_power() {
            // Create a unary Apply node: operator(operand)
            self.builder
                .start_node_at(apply_checkpoint, Kind::Apply.into());

            // The operator is the receiver
            self.builder.start_node(Kind::ApplyReceiver.into());
            self.bump(); // the prefix operator
            self.builder.finish_node();

            // Parse the operand with appropriate binding power
            let child_marker = self.whitespace.marker();
            self.skip_trivia();
            self.builder.start_node(Kind::ApplyArgument.into());
            self.parse_expression_bp(prefix_bp, child_marker);
            self.builder.finish_node();

            self.builder.finish_node();
        } else {
            self.parse_primary();
        }
        // Checkpoint now captures just the primary (or prefix expression), before trailing trivia

        // Loop to handle sequences of operators and function application
        loop {
            // Check what comes next
            self.skip_trivia();

            if !marker.should_continue(self) {
                // Nothing more - we're done
                marker.finish(self);
                return;
            }

            let op = self.current();

            // Check if this is a postfix operator first
            if let Some(l_bp) = op.postfix_binding_power() {
                // Stop if binding power is too low
                if l_bp < min_bp {
                    marker.finish(self);
                    return;
                }

                // Create a unary Apply node: operator(operand)
                self.builder
                    .start_node_at(apply_checkpoint, Kind::Apply.into());

                // The left side becomes the argument (without trailing trivia)
                self.builder
                    .start_node_at(content_checkpoint, Kind::ApplyArgument.into());
                self.builder.finish_node();

                // The operator is the receiver
                self.builder.start_node(Kind::ApplyReceiver.into());
                self.bump(); // the operator
                self.builder.finish_node();

                self.builder.finish_node();
                // Continue the outer loop to check for more operators
            } else if let Some((l_bp, r_bp)) = op.infix_binding_power() {
                // Check if this is an explicit infix operator
                // Stop if binding power is too low
                if l_bp < min_bp {
                    marker.finish(self);
                    return;
                }

                // Create a binary Apply node: operator(left, right)
                self.builder
                    .start_node_at(apply_checkpoint, Kind::Apply.into());

                // The left side becomes the first argument (without trailing trivia)
                self.builder
                    .start_node_at(content_checkpoint, Kind::ApplyArgument.into());
                self.builder.finish_node();

                // The operator is the receiver
                self.builder.start_node(Kind::ApplyReceiver.into());
                self.bump(); // the operator
                self.builder.finish_node();

                // Parse the right side with appropriate binding power
                let child_marker = self.whitespace.marker();
                self.skip_trivia();
                self.builder.start_node(Kind::ApplyArgument.into());
                self.parse_expression_bp(r_bp, child_marker);
                self.builder.finish_node();

                self.builder.finish_node();
                // Continue the outer loop to check for more operators
            } else {
                // Function application (implicit operator via juxtaposition)
                let (l_bp, r_bp) = Kind::juxtaposition_binding_power();

                if l_bp < min_bp {
                    marker.finish(self);
                    return;
                }

                // Create a binary Apply node: receiver(argument)
                self.builder
                    .start_node_at(apply_checkpoint, Kind::Apply.into());

                // The left side becomes the receiver (without trailing trivia)
                self.builder
                    .start_node_at(content_checkpoint, Kind::ApplyReceiver.into());
                self.builder.finish_node();

                // Parse the right side
                let child_marker = self.whitespace.marker();
                self.builder.start_node(Kind::ApplyArgument.into());
                self.parse_expression_bp(r_bp, child_marker);
                self.builder.finish_node();

                self.builder.finish_node();
                // Continue the outer loop for more application or operators
            }
        }
    }

    fn parse_primary(&mut self) {
        match self.current() {
            Kind::Identifier => {
                self.bump();
            }
            Kind::Integer | Kind::Float => {
                self.parse_literal();
            }
            Kind::StringStart => {
                self.parse_string();
            }
            Kind::LParen => {
                self.parse_expression(ParenMarker::new(self));
            }
            Kind::LBracket => {
                self.parse_array();
            }
            Kind::LBrace => {
                self.parse_record();
            }
            _ => {
                // Other tokens
                self.bump();
            }
        }
    }

    /// Parse an array literal: [elem1, elem2, ...]
    /// Represented as Apply(SyntheticList, [elem1, elem2, ...])
    fn parse_array(&mut self) {
        self.builder.start_node(Kind::Apply.into());

        // Create marker before consuming opening bracket
        let bracket_marker = BracketMarker::new(self);

        // Use marker's start to consume opening bracket and skip trivia
        bracket_marker.start(self);

        // Create a synthetic receiver node - the AST layer will provide the identifier
        self.builder.start_node(Kind::ApplyReceiver.into());
        self.builder.start_node(Kind::SyntheticList.into());
        self.builder.finish_node();
        self.builder.finish_node();

        // Parse comma-separated elements - delegate continue logic to marker
        while bracket_marker.should_continue(self) {
            // Check for empty element (comma without content)
            if self.current() == Kind::Comma {
                // Emit an error node in the CST
                self.builder.start_node(Kind::Error.into());
                self.error("expected expression before comma");
                self.builder.finish_node();
                self.bump();
                self.skip_trivia();
                continue;
            }

            self.builder.start_node(Kind::ApplyArgument.into());
            let child_marker = CommaMarker::new(bracket_marker, self);
            self.parse_expression_bp(0, child_marker);
            self.builder.finish_node();

            self.skip_trivia();

            // Handle comma separator
            if self.current() == Kind::Comma {
                self.bump();
                self.skip_trivia();
            } else {
                // No comma - we should be at the closing bracket or an error
                break;
            }
        }

        // Use marker's finish to consume closing bracket and handle errors
        bracket_marker.finish(self);

        self.builder.finish_node();
    }

    /// Parse a record literal: { elem1, elem2, ... }
    /// Represented as Apply(SyntheticRecord, [elem1, elem2, ...])
    fn parse_record(&mut self) {
        self.builder.start_node(Kind::Apply.into());

        // Create marker before consuming opening brace
        let brace_marker = BraceMarker::new(self);

        // Use marker's start to consume opening brace and skip trivia
        brace_marker.start(self);

        // Create a synthetic receiver node - the AST layer will provide the identifier
        self.builder.start_node(Kind::ApplyReceiver.into());
        self.builder.start_node(Kind::SyntheticRecord.into());
        self.builder.finish_node();
        self.builder.finish_node();

        // Parse comma-separated elements - delegate continue logic to marker
        while brace_marker.should_continue(self) {
            // Check for empty element (comma without content)
            if self.current() == Kind::Comma {
                // Emit an error node in the CST
                self.builder.start_node(Kind::Error.into());
                self.error("expected expression before comma");
                self.builder.finish_node();
                self.bump();
                self.skip_trivia();
                continue;
            }

            self.builder.start_node(Kind::ApplyArgument.into());
            let child_marker = CommaMarker::new(brace_marker, self);
            self.parse_expression_bp(0, child_marker);
            self.builder.finish_node();

            self.skip_trivia();

            // Handle comma separator
            if self.current() == Kind::Comma {
                self.bump();
                self.skip_trivia();
            } else {
                // No comma - we should be at the closing brace or an error
                break;
            }
        }

        // Use marker's finish to consume closing brace and handle errors
        brace_marker.finish(self);

        self.builder.finish_node();
    }

    fn parse_literal(&mut self) {
        // Check if there's an immediate identifier after the number (no whitespace)
        // If so, create an Apply node with reversed order: unit(number)

        // Get the current number token info
        let number_token = self
            .tokens
            .peek()
            .expect("parse_literal called without a token");
        let number_kind = number_token.kind;
        let number_span = number_token.span;
        let number_end = number_span.end;

        // Consume the number
        self.tokens.next();
        self.whitespace.on_token(&Token {
            kind: number_kind,
            span: number_span,
        });

        // Now check if the next token is an identifier with no whitespace
        if let Some(next_token) = self.tokens.peek() {
            if next_token.kind == Kind::Identifier && next_token.span.start == number_end {
                // Create an Apply node: unit(number)
                // Note: We emit the argument (number) before the receiver (unit) to maintain
                // correct CST offsets, as the number appears first in the source text.
                // The AST doesn't care about the order, similar to infix operators.
                self.builder.start_node(Kind::Apply.into());

                // The number is the argument - emit it first since it appears first in source
                self.builder.start_node(Kind::ApplyArgument.into());
                self.builder.start_node(Kind::Literal.into());
                self.builder.start_node(number_kind.into());
                self.builder
                    .token(number_kind.into(), self.text(number_span));
                self.builder.finish_node(); // Close Integer/Float node
                self.builder.finish_node(); // Close Literal
                self.builder.finish_node(); // Close ApplyArgument

                // The identifier is the receiver - emit it second
                self.builder.start_node(Kind::ApplyReceiver.into());
                self.bump(); // Consume the identifier
                self.builder.finish_node();

                self.builder.finish_node(); // Close Apply
                return;
            }
        }

        // Default case: no unit suffix, add the number as a literal
        self.builder.start_node(Kind::Literal.into());
        self.builder.start_node(number_kind.into());
        self.builder
            .token(number_kind.into(), self.text(number_span));
        self.builder.finish_node(); // Close Integer/Float node
        self.builder.finish_node(); // Close Literal
    }

    fn parse_string(&mut self) {
        self.builder.start_node(Kind::Literal.into());
        self.bump(); // StringStart
        debug_assert!(
            [Kind::StringContent, Kind::StringContentWithEscape].contains(&self.current())
        );
        self.bump();

        if self.current() == Kind::StringEnd {
            self.bump();
        } else {
            self.error("expected closing quote");
        }

        self.builder.finish_node();
    }

    fn current(&mut self) -> Kind {
        self.tokens
            .peek()
            .map(|token| token.kind)
            .unwrap_or(Kind::Eof)
    }

    fn bump(&mut self) {
        let Some(token) = self.tokens.next() else {
            return;
        };

        self.whitespace.on_token(&token);

        let is_implicit_node = token.kind.is_node();

        if is_implicit_node {
            self.builder.start_node(token.kind.into());
        }

        self.builder.token(token.kind.into(), self.text(token.span));

        if is_implicit_node {
            self.builder.finish_node();
        }
    }

    /// Skip all of the whitespace
    #[expect(dead_code)]
    fn skip_ws(&mut self) {
        while self.current().is_whitespace() {
            self.bump();
        }
    }

    fn skip_trivia(&mut self) {
        while self.current().is_trivia() {
            self.bump();
        }
    }

    fn text(&self, span: Span) -> &'src str {
        &self.src[span.start..span.end]
    }

    fn error(&mut self, message: &str) {
        let span = self
            .tokens
            .peek()
            .map(|t| t.span)
            .unwrap_or(Span { start: 0, end: 0 });

        self.errors.push(ParseError {
            span,
            message: message.to_string(),
        });
    }
}

trait Marker: Copy {
    fn start(&self, parser: &mut Parser) {
        let _ = parser;
    }

    fn should_continue(&self, parser: &mut Parser) -> bool;

    fn finish(&self, parser: &mut Parser) {
        let _ = parser;
    }
}

/// Generic marker for delimited expressions (parens, brackets, etc.)
/// Uses const generics for the closing delimiter Kind discriminant for optimization.
#[derive(Clone, Copy, Debug)]
struct DelimiterMarker<const CLOSE: u16> {
    saved_whitespace: WhitespaceMarker,
}

impl<const CLOSE: u16> DelimiterMarker<CLOSE> {
    fn new(parser: &Parser) -> Self {
        Self {
            saved_whitespace: parser.whitespace.marker(),
        }
    }

    const fn close_kind() -> Kind {
        // Use the generated try_from_u16 and panic at compile time if invalid
        // Wrap in const block to ensure compile-time evaluation
        const {
            match Kind::try_from_u16(CLOSE) {
                Some(kind) => kind,
                None => panic!("Invalid Kind discriminant"),
            }
        }
    }
}

/// Type alias for parenthesis marker
type ParenMarker = DelimiterMarker<{ Kind::RParen as u16 }>;
/// Type alias for bracket marker
type BracketMarker = DelimiterMarker<{ Kind::RBracket as u16 }>;
/// Type alias for brace marker
type BraceMarker = DelimiterMarker<{ Kind::RBrace as u16 }>;

impl<const CLOSE: u16> Marker for DelimiterMarker<CLOSE> {
    fn start(&self, parser: &mut Parser) {
        parser.bump(); // Opening delimiter
        parser.skip_trivia();
    }

    fn should_continue(&self, parser: &mut Parser) -> bool {
        let current = parser.current();
        if current == Self::close_kind() || current == Kind::Eof {
            return false;
        }
        // Delegate to saved whitespace marker for indentation checking
        // It already handles commas specially (as it handles infix operators)
        self.saved_whitespace.should_continue(parser)
    }

    fn finish(&self, parser: &mut Parser) {
        if parser.current() == Self::close_kind() {
            parser.bump();
        } else {
            // Use the generated display_name for human-readable error message
            let msg = Self::close_kind().display_name();
            parser.error(&format!("expected {}", msg));
        }
        // Restore whitespace state from before the delimiter block
        parser.whitespace.span = self.saved_whitespace.span;
        parser.whitespace.len = self.saved_whitespace.len;
        parser.whitespace.line = self.saved_whitespace.line;
    }
}

/// A marker that wraps an inner marker and also stops at commas.
/// This is used for parsing comma-separated elements within delimiters.
/// Note: This marker does NOT call inner.finish() - it only uses the inner
/// marker's should_continue logic and restores whitespace state directly.
#[derive(Clone, Copy, Debug)]
struct CommaMarker<M: Marker> {
    inner: M,
    saved_whitespace: WhitespaceMarker,
}

impl<M: Marker> CommaMarker<M> {
    fn new(inner: M, parser: &Parser) -> Self {
        Self {
            inner,
            saved_whitespace: parser.whitespace.marker(),
        }
    }
}

impl<M: Marker> Marker for CommaMarker<M> {
    fn start(&self, parser: &mut Parser) {
        // Don't call inner.start() - we handle our own start behavior
        let _ = parser;
    }

    fn should_continue(&self, parser: &mut Parser) -> bool {
        // Stop at comma in addition to whatever the inner marker stops at
        if parser.current() == Kind::Comma {
            return false;
        }
        self.inner.should_continue(parser)
    }

    fn finish(&self, parser: &mut Parser) {
        // Don't call inner.finish() - we just restore whitespace state
        parser.whitespace.span = self.saved_whitespace.span;
        parser.whitespace.len = self.saved_whitespace.len;
        parser.whitespace.line = self.saved_whitespace.line;
    }
}

#[derive(Debug)]
struct Whitespace {
    span: Span,
    len: usize,
    line: usize,
}

impl Default for Whitespace {
    fn default() -> Self {
        Self {
            span: Span::new(0, 0),
            len: 0,
            line: 0,
        }
    }
}

impl Whitespace {
    #[expect(dead_code)]
    pub fn len(&self) -> usize {
        self.len
    }

    #[expect(dead_code)]
    pub fn line(&self) -> usize {
        self.line
    }

    #[expect(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn marker(&self) -> WhitespaceMarker {
        WhitespaceMarker {
            span: self.span,
            line: self.line,
            len: self.len,
        }
    }

    pub fn on_token(&mut self, token: &Token) {
        let span = token.span;

        if token.kind == Kind::Newline {
            self.span = span;
            self.len = 0;
            self.line += 1;
            return;
        }

        if ![Kind::Space, Kind::Tab].contains(&token.kind) {
            return;
        }

        if self.span.end != span.start {
            return;
        }

        self.span.end = span.end;
        if token.kind == Kind::Tab {
            self.len += span.len() * 4;
        } else {
            self.len += span.len();
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct WhitespaceMarker {
    span: Span,
    line: usize,
    len: usize,
}

impl Marker for WhitespaceMarker {
    fn should_continue(&self, parser: &mut Parser) -> bool {
        if parser.current() == Kind::Eof {
            return false;
        }

        if self.line == parser.whitespace.line {
            return true;
        }

        let current = parser.current();
        // Infix, postfix operators and comma are allowed to start continuation lines
        // at same indentation level (for comma-first style like: [ 1\n, 2\n, 3])
        if current.is_infix() || current.is_postfix() || current == Kind::Comma {
            return parser.whitespace.len >= self.len;
        }

        // We should continue as long as the indentation is greater than the marker line
        parser.whitespace.len > self.len
    }
}
