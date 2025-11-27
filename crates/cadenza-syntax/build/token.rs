use std::collections::HashMap;

pub fn tokens() -> String {
    Tokens::default()
        .with_punctuation()
        .with_content()
        .with_literal()
        .with_node()
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
                .filter(|p| p.op)
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

        w!("    pub const fn as_op(self) -> Option<Op> {{");
        w!("        match self {{");
        for p in Punctuation::ALL.iter().filter(|p| p.op) {
            let name = p.name;
            w!("            Self::{name} => Some(Op::{name}), ");
        }
        w!("            _ => None, ");
        w!("        }}");
        w!("    }}");

        w!("}}");
        w!("");

        w!(
            "#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]"
        );
        w!("#[repr(u16)]");
        w!("pub enum Op {{");
        for p in Punctuation::ALL.iter().filter(|p| p.op) {
            w!("    /// {:?}", p.value);
            w!("    {}, ", p.name);
        }
        w!("}}");

        out
    }
}

struct Punctuation {
    name: &'static str,
    value: &'static str,
    duplicate: bool,
    whitespace: bool,
    trivia: bool,
    op: bool,
    // TODO infix binding
    // TODO prefix binding
    // TODO suffix binding
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
                op: true,
            }
        }

        [
            p("At", "@"),
            p("Equal", "="),
            p("EqualEqual", "=="),
            p("Less", "<"),
            p("LessEqual", "<="),
            p("LessLess", "<<"),
            p("Greater", ">"),
            p("GreaterEqual", ">="),
            p("GreaterGreater", ">>"),
            p("Plus", "+"),
            p("PlusEqual", "+="),
            p("Minus", "-"),
            p("MinusEqual", "-="),
            p("Arrow", "->"),
            p("Star", "*"),
            p("StarEqual", "*="),
            p("Slash", "/"),
            p("SlashEqual", "/="),
            p("Backslash", "\\"),
            p("Percent", "%"),
            p("PercentEqual", "%="),
            p("Bang", "!"),
            p("BangEqual", "!="),
            p("Ampersand", "&"),
            p("AmpersandAmpersand", "&&"),
            p("AmpersandEqual", "&="),
            p("Backtick", "`"),
            p("SingleQuote", "'"),
            p("Pipe", "|"),
            p("PipePipe", "||"),
            p("PipeEqual", "|="),
            p("PipeGreater", "|>"),
            p("PipeQuestion", "|?"),
            p("Caret", "^"),
            p("CaretEqual", "^="),
            p("Tilde", "~"),
            p("Dot", "."),
            p("DotDot", ".."),
            p("DotEqual", ".="),
            p("Dollar", "$"),
            p("Question", "?"),
            p("Comma", ","),
            p("Colon", ":"),
            p("ColonColon", "::"),
            p("Semicolon", ";"),
            p("LParen", "(").not_op(),
            p("RParen", ")").not_op(),
            p("LBrace", "{"),
            p("RBrace", "}").not_op(),
            p("LBracket", "["),
            p("RBracket", "]").not_op(),
            p("StringStart", "\"").not_op(),
            p("StringEnd", "\"").not_op().dup(),
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
            op: false,
            ..self
        }
    }

    const fn trivia(self) -> Self {
        Self {
            trivia: true,
            op: false,
            ..self
        }
    }

    const fn not_op(self) -> Self {
        Self { op: false, ..self }
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
