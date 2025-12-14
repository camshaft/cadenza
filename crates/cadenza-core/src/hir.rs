//! High-level Intermediate Representation (HIR) for Cadenza.
//!
//! The HIR is a desugared, simplified representation of the source code that:
//! - Preserves all source span information for error reporting
//! - Is easier to analyze and transform than the raw AST
//! - Serves as the input to evaluation, macro expansion, and type inference
//!
//! ## Design Principles
//!
//! 1. **Span Tracking**: Every HIR node has a source span
//! 2. **Simplified Structure**: Complex syntax is desugared during lowering
//! 3. **Evaluation-Ready**: HIR can be directly evaluated/expanded
//! 4. **Type-Inference-Ready**: HIR structure supports type checking
//!
//! ## HIR Pipeline
//!
//! ```text
//! AST (from parser)
//!   → [Lower] → HIR (with spans)
//!   → [Eval/Expand] → Expanded HIR (with spans)
//!   → [Type Inference] → Typed HIR
//!   → [LSP Queries] → IDE features
//! ```

use cadenza_syntax::span::Span;

/// A unique identifier for an HIR node.
///
/// This allows referencing HIR nodes without holding references.
/// Useful for building graphs of HIR nodes (e.g., for control flow).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HirId(u32);

impl HirId {
    /// Create a new HIR ID from a raw integer.
    pub fn new(id: u32) -> Self {
        Self(id)
    }

    /// Get the raw integer ID.
    pub fn raw(self) -> u32 {
        self.0
    }
}

/// An HIR expression with source span information.
///
/// Every expression in the HIR tracks its source location for error reporting
/// and IDE features (hover, go-to-definition, etc.).
#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    /// The expression kind (what type of expression this is).
    pub kind: ExprKind,
    /// The source span of this expression.
    pub span: Span,
    /// Optional unique identifier for this expression.
    pub id: Option<HirId>,
}

impl Expr {
    /// Create a new expression with a span.
    pub fn new(kind: ExprKind, span: Span) -> Self {
        Self {
            kind,
            span,
            id: None,
        }
    }

    /// Create a new expression with a span and ID.
    pub fn with_id(kind: ExprKind, span: Span, id: HirId) -> Self {
        Self {
            kind,
            span,
            id: Some(id),
        }
    }
}

/// The kind of HIR expression.
///
/// This enum represents the different types of expressions in the HIR.
/// Complex syntax from the AST is desugared into these simpler forms.
#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    /// A literal value (integer, float, string, bool).
    Literal(Literal),

    /// An identifier (variable reference).
    Ident(String),

    /// A let binding: `let name = value`.
    Let {
        name: String,
        value: Box<Expr>,
    },

    /// A function definition: `fn name params = body`.
    Fn {
        name: String,
        params: Vec<String>,
        body: Box<Expr>,
    },

    /// A function call: `f(arg1, arg2, ...)`.
    Call {
        func: Box<Expr>,
        args: Vec<Expr>,
    },

    /// A binary operation: `left op right`.
    BinOp {
        op: BinOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },

    /// A unary operation: `op expr`.
    UnaryOp {
        op: UnaryOp,
        expr: Box<Expr>,
    },

    /// A block of expressions: `{ expr1; expr2; ... }`.
    Block(Vec<Expr>),

    /// A list literal: `[elem1, elem2, ...]`.
    List(Vec<Expr>),

    /// A record literal: `{ field1 = val1, field2 = val2, ... }`.
    Record(Vec<(String, Expr)>),

    /// Field access: `expr.field`.
    FieldAccess {
        expr: Box<Expr>,
        field: String,
    },

    /// An if expression: `if cond then else`.
    If {
        cond: Box<Expr>,
        then_branch: Box<Expr>,
        else_branch: Option<Box<Expr>>,
    },

    /// A match expression (pattern matching).
    Match {
        expr: Box<Expr>,
        arms: Vec<MatchArm>,
    },

    /// An error node (produced when lowering fails).
    Error,
}

/// A literal value.
#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    /// Integer literal.
    Integer(i64),
    /// Float literal.
    Float(f64),
    /// String literal.
    String(String),
    /// Boolean literal.
    Bool(bool),
    /// Nil/unit literal.
    Nil,
}

/// Binary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BinOp {
    /// Addition: `+`
    Add,
    /// Subtraction: `-`
    Sub,
    /// Multiplication: `*`
    Mul,
    /// Division: `/`
    Div,
    /// Equality: `==`
    Eq,
    /// Inequality: `!=`
    Ne,
    /// Less than: `<`
    Lt,
    /// Less than or equal: `<=`
    Le,
    /// Greater than: `>`
    Gt,
    /// Greater than or equal: `>=`
    Ge,
    /// Logical AND: `&&`
    And,
    /// Logical OR: `||`
    Or,
}

/// Unary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnaryOp {
    /// Negation: `-expr`
    Neg,
    /// Logical NOT: `!expr`
    Not,
}

/// A match arm in a match expression.
#[derive(Debug, Clone, PartialEq)]
pub struct MatchArm {
    /// The pattern to match against.
    pub pattern: Pattern,
    /// The expression to evaluate if the pattern matches.
    pub body: Expr,
}

/// A pattern for pattern matching.
#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    /// Wildcard pattern: `_`
    Wildcard,
    /// Literal pattern: matches a specific literal value.
    Literal(Literal),
    /// Identifier pattern: binds to a variable.
    Ident(String),
    /// List pattern: `[pat1, pat2, ...]`
    List(Vec<Pattern>),
    /// Record pattern: `{ field1 = pat1, ... }`
    Record(Vec<(String, Pattern)>),
}

/// A complete HIR module (lowered from a source file).
///
/// This represents the top-level structure of a Cadenza file after
/// lowering from AST to HIR.
#[derive(Debug, Clone, PartialEq)]
pub struct Module {
    /// The list of top-level expressions in the module.
    pub items: Vec<Expr>,
}

impl Module {
    /// Create an empty module.
    pub fn empty() -> Self {
        Self { items: Vec::new() }
    }

    /// Create a module with the given items.
    pub fn new(items: Vec<Expr>) -> Self {
        Self { items }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hir_id() {
        let id1 = HirId::new(1);
        let id2 = HirId::new(1);
        let id3 = HirId::new(2);

        assert_eq!(id1, id2);
        assert_ne!(id1, id3);
        assert_eq!(id1.raw(), 1);
    }

    #[test]
    fn test_expr_creation() {
        let span = Span::new(0, 5);
        let expr = Expr::new(ExprKind::Literal(Literal::Integer(42)), span);

        assert_eq!(expr.span, span);
        assert!(expr.id.is_none());
        assert!(matches!(expr.kind, ExprKind::Literal(Literal::Integer(42))));
    }

    #[test]
    fn test_expr_with_id() {
        let span = Span::new(0, 5);
        let id = HirId::new(1);
        let expr = Expr::with_id(ExprKind::Literal(Literal::Integer(42)), span, id);

        assert_eq!(expr.span, span);
        assert_eq!(expr.id, Some(id));
    }

    #[test]
    fn test_module_creation() {
        let module = Module::empty();
        assert_eq!(module.items.len(), 0);

        let span = Span::new(0, 1);
        let expr = Expr::new(ExprKind::Literal(Literal::Nil), span);
        let module = Module::new(vec![expr]);
        assert_eq!(module.items.len(), 1);
    }
}
