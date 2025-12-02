use crate::{SyntaxNode, token::Kind};
use core::fmt;

macro_rules! ast_node {
    ($name:ident) => {
        ast_node!($name, $name);
    };
    ($name:ident, $kind:ident) => {
        #[derive(Clone, PartialEq, Eq, Hash)]
        #[repr(transparent)]
        pub struct $name(SyntaxNode);

        impl $name {
            pub fn cast(node: SyntaxNode) -> Option<Self> {
                if node.kind() == Kind::$kind {
                    Some(Self(node))
                } else {
                    None
                }
            }

            pub fn syntax(&self) -> &SyntaxNode {
                &self.0
            }

            pub fn into_syntax(self) -> SyntaxNode {
                self.0
            }
        }
    };
}

struct DebugIter<F, I: Iterator>(F)
where
    F: Fn() -> I;

impl<F, I> fmt::Debug for DebugIter<F, I>
where
    F: Fn() -> I,
    I: Iterator,
    I::Item: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.0()).finish()
    }
}

ast_node!(Root);

impl fmt::Debug for Root {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.items()).finish()
    }
}

impl Root {
    pub fn items(&self) -> impl Iterator<Item = Expr> {
        self.0.children().filter_map(Expr::cast)
    }
}

ast_node!(Apply);

impl fmt::Debug for Apply {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut list = f.debug_list();
        if let Some(receiver) = self.receiver() {
            list.entry(&receiver);
        } else {
            list.entry(&format_args!(
                "<MISSING APPLY RECEIVER in {} {:?}>",
                self.0.text(),
                DebugIter(|| self.0.children())
            ));
        }
        list.entries(self.arguments()).finish()
    }
}

impl Apply {
    pub fn receiver(&self) -> Option<ApplyReceiver> {
        self.0.children().find_map(ApplyReceiver::cast)
    }

    pub fn arguments(&self) -> impl Iterator<Item = ApplyArgument> {
        self.0.children().filter_map(ApplyArgument::cast)
    }
}

ast_node!(ApplyArgument);

impl fmt::Debug for ApplyArgument {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(value) = self.value() {
            write!(f, "{value:?}")
        } else {
            write!(
                f,
                "<MISSING APPLY ARGUMENT EXPR in {:?}>",
                DebugIter(|| self.0.children())
            )
        }
    }
}

impl ApplyArgument {
    pub fn value(&self) -> Option<Expr> {
        self.0.children().find_map(Expr::cast)
    }
}

ast_node!(ApplyReceiver);

impl fmt::Debug for ApplyReceiver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(value) = self.value() {
            write!(f, "{value:?}")
        } else {
            write!(
                f,
                "<MISSING APPLY RECEIVER EXPR in {:?}>",
                DebugIter(|| self.0.children())
            )
        }
    }
}

impl ApplyReceiver {
    pub fn value(&self) -> Option<Expr> {
        self.0.children().find_map(Expr::cast)
    }
}

ast_node!(Attr);

impl fmt::Debug for Attr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(value) = self.value() {
            write!(f, "@{value:?}")
        } else {
            write!(
                f,
                "<MISSING ATTR VALUE in {:?}>",
                DebugIter(|| self.0.children())
            )
        }
    }
}

impl Attr {
    pub fn value(&self) -> Option<Expr> {
        self.0.children().find_map(Expr::cast)
    }
}

ast_node!(Ident, Identifier);

impl fmt::Debug for Ident {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.text())
    }
}

ast_node!(Literal);

impl fmt::Debug for Literal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(value) = self.value() {
            write!(f, "{value:?}")
        } else {
            write!(
                f,
                "<MISSING LITERAL VALUE in {:?}>",
                DebugIter(|| self.0.children())
            )
        }
    }
}

impl Literal {
    pub fn value(&self) -> Option<LiteralValue> {
        self.0.children().find_map(LiteralValue::cast)
    }
}

#[derive(Clone)]
pub enum LiteralValue {
    Integer(IntegerValue),
    Float(FloatValue),
    String(StringValue),
    StringWithEscape(StringValueWithEscape),
}

impl fmt::Debug for LiteralValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Integer(value) => write!(f, "{value:?}"),
            Self::Float(value) => write!(f, "{value:?}"),
            Self::String(value) => write!(f, "{value:?}"),
            Self::StringWithEscape(value) => write!(f, "{value:?}"),
        }
    }
}

impl LiteralValue {
    fn cast(node: SyntaxNode) -> Option<Self> {
        match node.kind() {
            Kind::Integer => Some(Self::Integer(IntegerValue::cast(node)?)),
            Kind::Float => Some(Self::Float(FloatValue::cast(node)?)),
            Kind::StringContent => Some(Self::String(StringValue::cast(node)?)),
            Kind::StringContentWithEscape => {
                Some(Self::StringWithEscape(StringValueWithEscape::cast(node)?))
            }
            _ => None,
        }
    }
}

ast_node!(IntegerValue, Integer);

impl fmt::Debug for IntegerValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.text())
    }
}

ast_node!(FloatValue, Float);

impl fmt::Debug for FloatValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.text())
    }
}

ast_node!(StringValue, StringContent);

impl fmt::Debug for StringValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.0.text())
    }
}

ast_node!(StringValueWithEscape, StringContentWithEscape);

impl fmt::Debug for StringValueWithEscape {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.0.text())
    }
}

ast_node!(Error);

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Error").field(&self.0).finish()
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct Op(SyntaxNode);

impl Op {
    fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind().as_op().is_some() {
            Some(Self(node))
        } else {
            None
        }
    }

    /// Returns a reference to the underlying syntax node.
    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }
}

impl fmt::Debug for Op {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.text())
    }
}

/// A synthetic node that represents a semantic concept not directly in source.
/// The identifier is provided by the Kind's synthetic_identifier method.
#[derive(Clone, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct Synthetic(SyntaxNode);

impl Synthetic {
    /// Cast a SyntaxNode to a Synthetic if the node kind has a synthetic identifier.
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind().synthetic_identifier().is_some() {
            Some(Self(node))
        } else {
            None
        }
    }

    /// Returns a reference to the underlying syntax node.
    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// Returns the identifier for this synthetic node.
    ///
    /// This method is guaranteed to succeed because `cast` only succeeds
    /// for nodes with a synthetic identifier.
    pub fn identifier(&self) -> &'static str {
        self.0
            .kind()
            .synthetic_identifier()
            .expect("Synthetic node must have a synthetic_identifier")
    }
}

impl fmt::Debug for Synthetic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.identifier())
    }
}

#[derive(Clone)]
pub enum Expr {
    Apply(Apply),
    Attr(Attr),
    Ident(Ident),
    Error(Error),
    Literal(Literal),
    Op(Op),
    Synthetic(Synthetic),
}

impl fmt::Debug for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Apply(expr) => write!(f, "{expr:?}"),
            Self::Attr(expr) => write!(f, "{expr:?}"),
            Self::Ident(expr) => write!(f, "{expr:?}"),
            Self::Error(expr) => write!(f, "{expr:?}"),
            Self::Literal(expr) => write!(f, "{expr:?}"),
            Self::Op(expr) => write!(f, "{expr:?}"),
            Self::Synthetic(expr) => write!(f, "{expr:?}"),
        }
    }
}

impl Expr {
    fn cast(node: SyntaxNode) -> Option<Self> {
        match node.kind() {
            Kind::Apply => Some(Self::Apply(Apply::cast(node)?)),
            Kind::Attr => Some(Self::Attr(Attr::cast(node)?)),
            Kind::Identifier => Some(Self::Ident(Ident::cast(node)?)),
            Kind::Error => Some(Self::Error(Error::cast(node)?)),
            Kind::Literal => Some(Self::Literal(Literal::cast(node)?)),
            kind if kind.synthetic_identifier().is_some() => {
                Some(Self::Synthetic(Synthetic::cast(node)?))
            }
            _ => Some(Self::Op(Op::cast(node)?)),
        }
    }

    /// Cast a SyntaxNode to an Expr (public API).
    pub fn cast_syntax_node(node: &SyntaxNode) -> Option<Self> {
        Self::cast(node.clone())
    }
}
