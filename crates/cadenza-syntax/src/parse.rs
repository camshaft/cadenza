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
            self.parse_item();
        }

        self.builder.finish_node();

        Parse {
            green: self.builder.finish(),
            errors: self.errors,
        }
    }

    fn parse_item(&mut self) {
        match self.current() {
            Kind::At => {
                self.parse_attr();
            }
            Kind::Identifier => {
                let marker = self.whitespace.marker();
                self.parse_expression(marker);
            }
            Kind::Integer | Kind::Float => {
                self.parse_literal();
            }
            Kind::StringStart => {
                self.parse_string();
            }
            Kind::Eof => {}
            _ => {
                // For now, just consume other tokens
                self.bump();
            }
        }
    }

    fn parse_expression(&mut self, marker: impl Marker) {
        self.parse_expression_bp(0, marker);
    }

    /// Parse expression with binding power (Pratt parsing)
    fn parse_expression_bp(&mut self, min_bp: u8, marker: impl Marker) {
        marker.start(self);

        // Parse the left-hand side (prefix)
        self.skip_trivia();

        // Take checkpoint before parsing primary (for the Apply node)
        let apply_checkpoint = self.builder.checkpoint();

        // Take checkpoint for content only (excludes leading trivia)
        let content_checkpoint = self.builder.checkpoint();
        self.parse_primary();
        // Checkpoint now captures just the primary, before trailing trivia

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

            // Check if this is an explicit infix operator
            if let Some((l_bp, r_bp)) = Self::infix_binding_power(op) {
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
                let (l_bp, r_bp) = Self::juxtaposition_binding_power();

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
                self.parse_expression(LParenMarker::new(self));
            }
            _ => {
                // Other tokens
                self.bump();
            }
        }
    }

    fn parse_attr(&mut self) {
        self.builder.start_node(Kind::Attr.into());
        debug_assert!(self.current() == Kind::At);
        self.bump(); // @

        let marker = self.whitespace.marker();
        self.parse_expression(marker);

        self.builder.finish_node();
    }

    fn parse_literal(&mut self) {
        self.builder.start_node(Kind::Literal.into());
        self.bump(); // Integer or Float
        self.builder.finish_node();
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

    fn juxtaposition_binding_power() -> (u8, u8) {
        // Between Equal (3, 2) and PipePipe (4, 5)
        // Left-associative
        (3, 4)
    }

    /// Returns (left_bp, right_bp) for infix operators
    /// Higher binding power = higher precedence
    fn infix_binding_power(op: Kind) -> Option<(u8, u8)> {
        use Kind::*;
        Some(match op {
            // Pipe operator (lowest precedence, left-associative)
            // Should capture complete expressions: a + b |> f means (a + b) |> f
            PipeGreater => (1, 2),

            // Assignment-like (right-associative)
            Equal => (3, 2),

            // Logical OR
            PipePipe => (4, 5),

            // Logical AND
            AmpersandAmpersand => (6, 7),

            // Equality/Comparison
            EqualEqual | BangEqual => (8, 9),
            Less | LessEqual | Greater | GreaterEqual => (8, 9),

            // Additive
            Plus | Minus => (10, 11),

            // Multiplicative
            Star | Slash | Percent => (12, 13),

            _ => return None,
        })
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

#[derive(Clone, Copy, Debug)]
struct LParenMarker {
    saved_whitespace: WhitespaceMarker,
}

impl LParenMarker {
    fn new(parser: &Parser) -> Self {
        Self {
            saved_whitespace: parser.whitespace.marker(),
        }
    }
}

impl Marker for LParenMarker {
    fn start(&self, parser: &mut Parser) {
        parser.bump(); // LParen
        parser.skip_trivia();
    }

    fn should_continue(&self, parser: &mut Parser) -> bool {
        ![Kind::RParen, Kind::Eof].contains(&parser.current())
    }

    fn finish(&self, parser: &mut Parser) {
        if parser.current() == Kind::RParen {
            parser.bump(); // RParen
        } else {
            parser.error("expected closing parenthesis");
        }
        // Restore whitespace state from before the paren block
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
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn line(&self) -> usize {
        self.line
    }

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

    pub fn should_continue(&self, marker: &WhitespaceMarker) -> bool {
        self.line == marker.line || self.len > marker.len
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

        // If the current token is an operator, continue even if we're on a new line
        // This allows operators to start continuation lines
        if Parser::infix_binding_power(parser.current()).is_some() {
            return true;
        }

        parser.whitespace.should_continue(self)
    }
}
