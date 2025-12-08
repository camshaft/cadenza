use std::collections::HashMap;

pub fn tokens() -> String {
    Tokens::default()
        .with_punctuation()
        .with_content()
        .with_literal()
        .with_node()
        .with_synthetic()
        .finish()
}

#[derive(Default)]
struct Tokens {
    variants: Vec<String>,
    whitespace: Vec<String>,
    nodes: Vec<String>,
    trivia: Vec<String>,
    with_values: HashMap<String, &'static str>,
}

impl Tokens {
    fn with_punctuation(mut self) -> Self {
        self.variants
            .extend(Punctuation::ALL.iter().map(|p| p.name.to_string()));

        self.whitespace.extend(
            Punctuation::ALL
                .iter()
                .filter(|p| p.whitespace)
                .map(|p| p.name.to_string()),
        );

        self.trivia.extend(
            Punctuation::ALL
                .iter()
                .filter(|p| p.trivia)
                .map(|p| p.name.to_string()),
        );

        self.with_values.extend(
            Punctuation::ALL
                .iter()
                .filter(|p| !p.duplicate)
                .map(|p| (p.name.to_string(), p.value)),
        );

        self.nodes.extend(
            Punctuation::ALL
                .iter()
                .filter(|p| p.binding_power.is_some())
                .map(|p| p.name.to_string()),
        );

        self
    }

    fn with_content(mut self) -> Self {
        self.variants
            .extend(Content::ALL.iter().map(|c| c.name.to_string()));

        self.trivia.extend(
            Content::ALL
                .iter()
                .filter(|c| c.trivia)
                .map(|c| c.name.to_string()),
        );

        self.nodes.extend(
            Content::ALL
                .iter()
                .filter(|c| !c.trivia)
                .map(|c| c.name.to_string()),
        );

        self
    }

    fn with_literal(mut self) -> Self {
        self.variants
            .extend(Literal::ALL.iter().map(|l| l.name.to_string()));

        self.nodes
            .extend(Literal::ALL.iter().map(|l| l.name.to_string()));

        self
    }

    fn with_node(mut self) -> Self {
        self.variants
            .extend(Node::ALL.iter().map(|n| n.name.to_string()));

        self.nodes
            .extend(Node::ALL.iter().map(|n| n.name.to_string()));

        self
    }

    fn with_synthetic(mut self) -> Self {
        self.variants
            .extend(Synthetic::ALL.iter().map(|s| s.name.to_string()));

        self.nodes
            .extend(Synthetic::ALL.iter().map(|s| s.name.to_string()));

        self
    }

    fn finish(mut self) -> String {
        self.variants.push("Eof".to_string());

        let mut out = String::new();
        macro_rules! w {
            ($($tt:tt)*) => {
                out.push_str(&format!($($tt)*));
                out.push('\n');
            };
        }

        w!("use serde::{{Serialize, Deserialize}};");
        w!(
            "#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]"
        );
        w!("#[repr(u16)]");
        w!("pub enum Kind {{");
        for variant in &self.variants {
            if let Some(value) = self.with_values.get(variant.as_str()) {
                w!("    /// {value:?}");
            }
            w!("    {variant},");
        }
        w!("}}");

        w!("impl Kind {{");
        w!("    pub const ALL: &'static [Self] = &[");
        for variant in &self.variants {
            w!("        Self::{variant},");
        }
        w!("    ];");
        w!("");

        let kinds = [
            ("is_whitespace", &self.whitespace),
            ("is_trivia", &self.trivia),
            ("is_node", &self.nodes),
        ];

        for (name, variants) in kinds {
            let c = name.strip_prefix("is_").unwrap().to_uppercase();
            w!("    pub const {c}: &'static [Self] = &[");
            for variant in variants {
                w!("        Self::{variant}, ");
            }
            w!("    ];");
            w!("");

            w!("    pub const fn {name}(self) -> bool {{");
            let variant_list = variants
                .iter()
                .map(|v| format!("Self::{v}"))
                .collect::<Vec<_>>()
                .join(" | ");
            w!("        matches!(self, {variant_list})");
            w!("    }}");
            w!("");
        }

        w!("    pub const fn as_str(self) -> Option<&'static str> {{");
        w!("        match self {{");
        for (variant, value) in self
            .variants
            .iter()
            .filter_map(|name| self.with_values.get(name.as_str()).map(|v| (name, v)))
        {
            w!("            Self::{variant} => Some({value:?}),");
        }
        w!("            _ => None, ");
        w!("        }}");
        w!("    }}");
        w!("");

        // Generate try_from_u16 method
        w!("    /// Try to convert a u16 discriminant to a Kind");
        w!("    pub const fn try_from_u16(value: u16) -> Option<Self> {{");
        w!("        if value < {} {{", self.variants.len());
        w!("            // SAFETY: value is within valid discriminant range");
        w!("            Some(unsafe {{ core::mem::transmute::<u16, Kind>(value) }})");
        w!("        }} else {{");
        w!("            None");
        w!("        }}");
        w!("    }}");
        w!("");

        // Generate human-readable display_name method
        w!("    /// Returns a human-readable name for this token kind");
        w!("    pub const fn display_name(self) -> &'static str {{");
        w!("        match self {{");
        for variant in &self.variants {
            // Use the value if it exists (for punctuation), otherwise use the variant name
            if let Some(value) = self.with_values.get(variant.as_str()) {
                w!("            Self::{variant} => {value:?},");
            } else {
                // Convert PascalCase to readable form
                let readable = variant
                    .chars()
                    .enumerate()
                    .map(|(i, c)| {
                        if c.is_uppercase() && i > 0 {
                            format!(" {}", c.to_lowercase())
                        } else if i == 0 {
                            c.to_lowercase().to_string()
                        } else {
                            c.to_string()
                        }
                    })
                    .collect::<String>();
                w!("            Self::{variant} => {readable:?},");
            }
        }
        w!("        }}");
        w!("    }}");
        w!("");

        w!("    pub const fn as_op(self) -> Option<Op> {{");
        w!("        match self {{");
        for p in Punctuation::ALL
            .iter()
            .filter(|p| p.binding_power.is_some())
        {
            let name = p.name;
            w!("            Self::{name} => Some(Op::{name}), ");
        }
        w!("            _ => None, ");
        w!("        }}");
        w!("    }}");
        w!("");

        // Generate prefix_binding_power method
        w!("    pub const fn prefix_binding_power(self) -> Option<u8> {{");
        w!("        match self {{");
        for p in Punctuation::ALL.iter() {
            if let Some(BindingPower::Prefix(bp)) = p.binding_power {
                let name = p.name;
                let bp_value = bp as u8;
                w!("            Self::{name} => Some({bp_value}),");
            }
        }
        w!("            _ => None,");
        w!("        }}");
        w!("    }}");
        w!("");

        // Generate infix_binding_power method
        w!("    pub const fn infix_binding_power(self) -> Option<(u8, u8)> {{");
        w!("        match self {{");
        for p in Punctuation::ALL.iter() {
            if let Some(BindingPower::Infix(bp)) = p.binding_power {
                let name = p.name;
                let (left_bp, right_bp) = bp.binding_power();
                w!("            Self::{name} => Some(({left_bp}, {right_bp})),");
            }
        }
        w!("            _ => None,");
        w!("        }}");
        w!("    }}");
        w!("");

        // Generate postfix_binding_power method
        w!("    pub const fn postfix_binding_power(self) -> Option<u8> {{");
        w!("        match self {{");
        for p in Punctuation::ALL.iter() {
            if let Some(BindingPower::Postfix(bp)) = p.binding_power {
                let name = p.name;
                let bp_value = bp as u8;
                w!("            Self::{name} => Some({bp_value}),");
            }
        }
        w!("            _ => None,");
        w!("        }}");
        w!("    }}");
        w!("");

        // Generate juxtaposition_binding_power method
        let (jux_left, jux_right) = InfixBindingPower::Juxtaposition.binding_power();
        w!("    /// Returns the binding power for juxtaposition (function application)");
        w!("    /// This is calculated from the Juxtaposition infix binding power group");
        w!("    pub const fn juxtaposition_binding_power() -> (u8, u8) {{");
        w!("        ({jux_left}, {jux_right})");
        w!("    }}");
        w!("");

        // Generate array_index_binding_power method
        // Array indexing should have the same precedence as path access (highest)
        let (path_left, _path_right) = InfixBindingPower::PathAccess.binding_power();
        w!("    /// Returns the left binding power for array indexing");
        w!("    /// Array indexing has the same precedence as path access (::)");
        w!("    pub const fn array_index_binding_power() -> u8 {{");
        w!("        {path_left}");
        w!("    }}");
        w!("");

        // Generate is_infix method
        w!("    /// Returns true if this token kind has infix binding power");
        w!("    pub const fn is_infix(self) -> bool {{");
        w!("        self.infix_binding_power().is_some()");
        w!("    }}");
        w!("");

        // Generate is_postfix method
        w!("    /// Returns true if this token kind has postfix binding power");
        w!("    pub const fn is_postfix(self) -> bool {{");
        w!("        self.postfix_binding_power().is_some()");
        w!("    }}");
        w!("");

        // Generate is_prefix method
        w!("    /// Returns true if this token kind has prefix binding power");
        w!("    pub const fn is_prefix(self) -> bool {{");
        w!("        self.prefix_binding_power().is_some()");
        w!("    }}");
        w!("");

        // Generate synthetic_identifier method
        w!("    /// Returns the identifier for synthetic nodes");
        w!("    pub const fn synthetic_identifier(self) -> Option<&'static str> {{");
        w!("        match self {{");
        for s in Synthetic::ALL.iter() {
            let name = s.name;
            let identifier = s.identifier;
            w!("            Self::{name} => Some({identifier:?}),");
        }
        w!("            _ => None,");
        w!("        }}");
        w!("    }}");

        w!("}}");
        w!("");

        w!(
            "#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]"
        );
        w!("#[repr(u16)]");
        w!("pub enum Op {{");
        for p in Punctuation::ALL
            .iter()
            .filter(|p| p.binding_power.is_some())
        {
            w!("    /// {:?}", p.value);
            w!("    {}, ", p.name);
        }
        w!("}}");

        out
    }
}

#[derive(Clone, Copy)]
enum BindingPower {
    Prefix(PrefixBindingPower),
    Infix(InfixBindingPower),
    Postfix(PostfixBindingPower),
}

#[derive(Clone, Copy)]
#[repr(u8)]
enum PrefixBindingPower {
    /// Attribute operator: @expr
    Attribute = 0,
    /// Prefix operators: -x, !x, ~x
    Unary = 26,
}

#[derive(Clone, Copy)]
#[repr(u8)]
enum InfixBindingPower {
    /// Pipe operators: |>
    Pipe,
    /// Range operators: .., ..=
    Range,
    /// Assignment operators: =, +=, -=, *=, /=, %=, &=, |=, ^=, <<=, >>=, ->, <-
    Assignment,
    /// Juxtaposition (function application)
    Juxtaposition,
    /// Match arm separator: =>
    MatchArm,
    /// Logical OR: ||
    LogicalOr,
    /// Logical AND: &&
    LogicalAnd,
    /// Equality: ==, !=
    Equality,
    /// Comparison: <, <=, >, >=
    Comparison,
    /// Bitwise OR: |
    BitwiseOr,
    /// Bitwise XOR: ^
    BitwiseXor,
    /// Bitwise AND: &
    BitwiseAnd,
    /// Shift operators: <<, >>
    Shift,
    /// Additive: +, -
    Additive,
    /// Multiplicative: *, /, %
    Multiplicative,
    /// Exponentiation: **
    Exponentiation,
    /// Field access: .
    FieldAccess,
    /// Path separator: ::
    PathAccess,
}

#[derive(Clone, Copy)]
enum Associativity {
    Left,
    Right,
}

impl InfixBindingPower {
    /// Returns the associativity of this operator group
    const fn associativity(&self) -> Associativity {
        match self {
            // Right-associative operators
            Self::Assignment | Self::Exponentiation => Associativity::Right,
            // Everything else is left-associative
            _ => Associativity::Left,
        }
    }

    /// Returns (left_bp, right_bp) for this operator
    /// Calculated from base value and associativity:
    /// - Left-associative: (base, base + 1)
    /// - Right-associative: (base, base - 1)
    const fn binding_power(&self) -> (u8, u8) {
        let base = *self as u8 * 2;
        match self.associativity() {
            Associativity::Left => (base, base + 1),
            Associativity::Right => (base + 1, base),
        }
    }
}

#[derive(Clone, Copy)]
#[repr(u8)]
enum PostfixBindingPower {
    /// Pipe try operator: |?
    PipeTry = 0,
    /// Try operator: ? (lower than FieldAccess=30 so field resolves first)
    Try = 29,
    /// Array indexing: arr[0] (highest precedence, same as PathAccess)
    /// Note: This is handled specially in the parser, not through the binding power system
    #[allow(dead_code)]
    ArrayIndex = 34,
}

struct Punctuation {
    name: &'static str,
    value: &'static str,
    duplicate: bool,
    whitespace: bool,
    trivia: bool,
    // Pratt parser binding powers
    binding_power: Option<BindingPower>,
}

impl Punctuation {
    const ALL: &[Self] = &const {
        const fn p(name: &'static str, value: &'static str) -> Punctuation {
            Punctuation {
                name,
                value,
                duplicate: false,
                whitespace: false,
                trivia: false,
                binding_power: None,
            }
        }

        [
            // Prefix operators
            p("At", "@").prefix(PrefixBindingPower::Attribute),
            p("Bang", "!").prefix(PrefixBindingPower::Unary),
            p("Tilde", "~").prefix(PrefixBindingPower::Unary),
            p("Dollar", "$").prefix(PrefixBindingPower::Unary),
            p("DotDotDot", "...").prefix(PrefixBindingPower::Unary),
            // Postfix operators
            p("Question", "?").postfix(PostfixBindingPower::Try),
            p("PipeQuestion", "|?").postfix(PostfixBindingPower::PipeTry),
            // Infix operators
            p("PipeGreater", "|>").infix(InfixBindingPower::Pipe),
            p("DotDot", "..").infix(InfixBindingPower::Range),
            p("DotDotEqual", "..=").infix(InfixBindingPower::Range),
            p("Equal", "=").infix(InfixBindingPower::Assignment),
            p("RightArrow", "->").infix(InfixBindingPower::Assignment),
            p("FatArrow", "=>").infix(InfixBindingPower::MatchArm),
            p("LeftArrow", "<-").infix(InfixBindingPower::Assignment),
            p("PlusEqual", "+=").infix(InfixBindingPower::Assignment),
            p("MinusEqual", "-=").infix(InfixBindingPower::Assignment),
            p("StarEqual", "*=").infix(InfixBindingPower::Assignment),
            p("SlashEqual", "/=").infix(InfixBindingPower::Assignment),
            p("PercentEqual", "%=").infix(InfixBindingPower::Assignment),
            p("AmpersandEqual", "&=").infix(InfixBindingPower::Assignment),
            p("PipeEqual", "|=").infix(InfixBindingPower::Assignment),
            p("CaretEqual", "^=").infix(InfixBindingPower::Assignment),
            p("LessLessEqual", "<<=").infix(InfixBindingPower::Assignment),
            p("GreaterGreaterEqual", ">>=").infix(InfixBindingPower::Assignment),
            p("PipePipe", "||").infix(InfixBindingPower::LogicalOr),
            p("AmpersandAmpersand", "&&").infix(InfixBindingPower::LogicalAnd),
            p("EqualEqual", "==").infix(InfixBindingPower::Equality),
            p("BangEqual", "!=").infix(InfixBindingPower::Equality),
            p("Less", "<").infix(InfixBindingPower::Comparison),
            p("LessEqual", "<=").infix(InfixBindingPower::Comparison),
            p("Greater", ">").infix(InfixBindingPower::Comparison),
            p("GreaterEqual", ">=").infix(InfixBindingPower::Comparison),
            p("Pipe", "|").infix(InfixBindingPower::BitwiseOr),
            p("Caret", "^").infix(InfixBindingPower::BitwiseXor),
            p("Ampersand", "&").infix(InfixBindingPower::BitwiseAnd),
            p("LessLess", "<<").infix(InfixBindingPower::Shift),
            p("GreaterGreater", ">>").infix(InfixBindingPower::Shift),
            p("Plus", "+").infix(InfixBindingPower::Additive),
            p("Minus", "-").infix(InfixBindingPower::Additive),
            p("Star", "*").infix(InfixBindingPower::Multiplicative),
            p("Slash", "/").infix(InfixBindingPower::Multiplicative),
            p("Percent", "%").infix(InfixBindingPower::Multiplicative),
            p("StarStar", "**").infix(InfixBindingPower::Exponentiation),
            p("Dot", ".").infix(InfixBindingPower::FieldAccess),
            p("ColonColon", "::").infix(InfixBindingPower::PathAccess),
            // Non-operator punctuation
            p("Backslash", "\\"),
            p("Backtick", "`"),
            p("SingleQuote", "'"),
            p("Comma", ","),
            p("Colon", ":"),
            p("Semicolon", ";"),
            p("LParen", "("),
            p("RParen", ")"),
            p("LDollarBrace", "${"),
            p("LBrace", "{"),
            p("RBrace", "}"),
            p("LBracket", "["),
            p("RBracket", "]"),
            p("StringStart", "\""),
            p("StringEnd", "\"").dup(),
            p("CommentStart", "#").trivia(),
            p("DocCommentStart", "##"),
            p("Space", " ").ws(),
            p("Tab", "\t").ws(),
            p("Newline", "\n").ws(),
        ]
    };

    const fn dup(self) -> Self {
        Self {
            duplicate: true,
            ..self
        }
    }

    const fn ws(self) -> Self {
        Self {
            whitespace: true,
            trivia: true,
            ..self
        }
    }

    const fn trivia(self) -> Self {
        Self {
            trivia: true,
            ..self
        }
    }

    const fn prefix(self, binding_power: PrefixBindingPower) -> Self {
        Self {
            binding_power: Some(BindingPower::Prefix(binding_power)),
            ..self
        }
    }

    const fn infix(self, binding_power: InfixBindingPower) -> Self {
        Self {
            binding_power: Some(BindingPower::Infix(binding_power)),
            ..self
        }
    }

    const fn postfix(self, binding_power: PostfixBindingPower) -> Self {
        Self {
            binding_power: Some(BindingPower::Postfix(binding_power)),
            ..self
        }
    }
}

struct Content {
    name: &'static str,
    trivia: bool,
}

impl Content {
    const ALL: &[Self] = &const {
        const fn c(name: &'static str) -> Content {
            Content {
                name,
                trivia: false,
            }
        }

        [
            c("StringContent"),
            c("StringContentWithEscape"),
            c("CommentContent").trivia(),
            c("DocCommentContent"),
        ]
    };

    const fn trivia(self) -> Self {
        Self {
            trivia: true,
            ..self
        }
    }
}

struct Literal {
    name: &'static str,
}

impl Literal {
    const ALL: &[Self] = &const {
        const fn l(name: &'static str) -> Literal {
            Literal { name }
        }

        [l("Integer"), l("Float"), l("Identifier")]
    };
}

struct Node {
    name: &'static str,
}

impl Node {
    const ALL: &[Self] = &const {
        const fn n(name: &'static str) -> Node {
            Node { name }
        }

        [
            n("Root"),
            n("Apply"),
            n("ApplyArgument"),
            n("ApplyReceiver"),
            n("Attr"),
            n("Literal"),
            n("Error"),
        ]
    };
}

/// Synthetic nodes that don't correspond to source text but provide
/// semantic meaning for the AST layer. Each has an associated identifier.
struct Synthetic {
    name: &'static str,
    /// The identifier that this synthetic node represents in the AST
    identifier: &'static str,
}

impl Synthetic {
    const ALL: &[Self] = &const {
        const fn s(name: &'static str, identifier: &'static str) -> Synthetic {
            Synthetic { name, identifier }
        }

        [
            s("SyntheticList", "__list__"),
            s("SyntheticRecord", "__record__"),
            s("SyntheticBlock", "__block__"),
            s("SyntheticIndex", "__index__"),
            // Markdown elements
            s("SyntheticMarkdownH1", "h1"),
            s("SyntheticMarkdownH2", "h2"),
            s("SyntheticMarkdownH3", "h3"),
            s("SyntheticMarkdownH4", "h4"),
            s("SyntheticMarkdownH5", "h5"),
            s("SyntheticMarkdownH6", "h6"),
            s("SyntheticMarkdownParagraph", "p"),
            s("SyntheticMarkdownList", "ul"),
            s("SyntheticMarkdownCode", "code"),
            // Markdown inline elements
            s("SyntheticMarkdownEmphasis", "em"),
            s("SyntheticMarkdownStrong", "strong"),
            s("SyntheticMarkdownCodeInline", "code_inline"),
        ]
    };
}
