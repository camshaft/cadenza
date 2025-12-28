//! Core data structures for the Cadenza compiler.
//!
//! The central type is `Object`, which represents a value throughout the compilation pipeline.
//! Each phase progressively annotates the Object with additional information.

use crate::error::Error;
use cadenza_syntax::ast::Expr;
use cadenza_tree::InternedString;
use hashbrown::HashMap;
use rustc_hash::FxBuildHasher;
use std::{fmt, hash::Hash, sync::Arc};

/// The central data structure throughout the compilation pipeline.
///
/// An `Object` wraps a `Value` and progressively accumulates metadata through each
/// compilation phase:
/// - Parse: Creates initial Objects from syntax
/// - Evaluate: Adds evaluation context
/// - Type Check: Adds type information
/// - Ownership: Adds memory management metadata
/// - Monomorphize: Adds specialization information
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Object {
    /// The actual value/expression
    pub value: Value,

    /// Type information (added during type checking)
    pub ty: Option<Type>,

    /// Ownership and memory management metadata (added during ownership analysis)
    pub ownership: Option<OwnershipMetadata>,

    /// Source location information
    pub source: Option<Expr>,

    /// Contract metadata (preconditions, postconditions, invariants)
    pub attributes: Attributes,

    /// Documentation string
    pub documentation: Option<Arc<str>>,

    /// Monomorphization metadata (polymorphic suffix, specialized versions)
    pub monomorphization: Option<MonomorphizationMetadata>,
}

impl Object {
    /// Create a new Object with just a value
    pub fn new(value: Value) -> Self {
        Self {
            value,
            ty: None,
            ownership: None,
            source: None,
            attributes: Default::default(),
            documentation: None,
            monomorphization: None,
        }
    }

    /// Create an Object from a syntax expression
    pub fn from_expr(scope: &Scope, expr: Expr) -> Self {
        let value = Value::from_expr(scope, expr.clone());
        Self::new(value).with_source(expr)
    }

    /// Set the type annotation
    pub fn with_type(mut self, ty: Type) -> Self {
        self.ty = Some(ty);
        self
    }

    /// Set the ownership metadata
    pub fn with_ownership(mut self, ownership: OwnershipMetadata) -> Self {
        self.ownership = Some(ownership);
        self
    }

    /// Set the source file
    pub fn with_source(mut self, source: Expr) -> Self {
        self.source = Some(source);
        self
    }

    /// Add an attribute
    pub fn with_attribute(mut self, name: InternedString, arguments: Arc<[Object]>) -> Self {
        self.attributes.0.entry(name).or_default().push(arguments);
        self
    }

    /// Set the monomorphization metadata
    pub fn with_monomorphization(mut self, mono: MonomorphizationMetadata) -> Self {
        self.monomorphization = Some(mono);
        self
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Number {
    Integer(i128),
    Float(f64),
    /// Rational number (numerator, denominator)
    Rational(i128, i128),
}

impl PartialEq for Number {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Integer(l0), Self::Integer(r0)) => l0 == r0,
            (Self::Float(l0), Self::Float(r0)) => l0.to_bits() == r0.to_bits(),
            (Self::Rational(l0, l1), Self::Rational(r0, r1)) => l0 == r0 && l1 == r1,
            _ => false,
        }
    }
}

impl Eq for Number {}

impl Hash for Number {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            Self::Integer(i) => i.hash(state),
            Self::Float(f) => f.to_bits().hash(state),
            Self::Rational(n, d) => {
                n.hash(state);
                d.hash(state);
            }
        }
    }
}

impl From<i128> for Number {
    fn from(value: i128) -> Self {
        Self::Integer(value)
    }
}

impl From<f64> for Number {
    fn from(value: f64) -> Self {
        Self::Float(value)
    }
}

pub type Scope = Arc<[InternedString]>;

/// The core value types in Cadenza.
///
/// This enum represents all possible values that can exist during compilation,
/// from literals to complex structures.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum Value {
    Number(Number),

    /// String literal
    LiteralString(InternedString),

    /// Dynamic string
    String(String),

    /// Boolean literal
    Bool(bool),

    /// Character literal
    Char(char),

    /// Unit value (empty tuple)
    Unit,

    /// Symbol/identifier
    Symbol {
        scope: Scope,
        name: InternedString,
    },

    /// Function application: receiver (first element) and arguments
    Apply {
        arguments: Vec<Object>,
    },

    /// List of objects
    List(Vec<Object>),

    /// Array/vector of objects
    Vec(Vec<Object>),

    /// Dictionary/map of key-value pairs
    Dict(Dict),

    Struct {
        name: Option<Box<Object>>,
        fields: Arc<[Object]>,
        values: Vec<Object>,
    },

    Tuple {
        name: Option<Box<Object>>,
        values: Vec<Object>,
    },

    EnumVariant {
        name: Option<Box<Object>>,
        variant: InternedString,
        value: Box<Object>,
    },

    /// Function definition
    Function {
        name: Option<Box<Object>>,
        parameters: Vec<Object>,
        body: Box<Object>,
        captures: Vec<Object>, // For closures
    },

    /// Let binding
    Let {
        bindings: Vec<(Object, Object)>,
    },

    /// If conditional
    If {
        condition: Box<Object>,
        then_branch: Box<Object>,
        else_branch: Option<Box<Object>>,
    },

    /// Match expression
    Match {
        scrutinee: Box<Object>,
        arms: Vec<MatchArm>,
    },

    /// While loop
    While {
        condition: Box<Object>,
        body: Box<Object>,
    },

    /// Do block (sequence of expressions)
    Do(Vec<Object>),

    /// Reference/borrow (&x)
    Ref(Box<Object>),

    /// Dereference/copy (*x)
    Deref(Box<Object>),

    /// Compile-time type value
    Type(Type),

    /// Type annotation (the x : Type)
    TypeInscription {
        value: Box<Object>,
        ty: Box<Object>,
    },

    /// Macro definition
    Macro {
        name: InternedString,
        parameters: Vec<InternedString>,
        body: Box<Object>,
    },

    /// Quote (prevent evaluation)
    Quote(Box<Object>),

    /// Unquote (evaluate in quote context)
    Unquote(Box<Object>),

    /// Error placeholder for malformed expressions
    Error(Error),
}

impl Value {
    /// Convert a syntax expression to a Value
    pub fn from_expr(scope: &Scope, expr: Expr) -> Self {
        use cadenza_syntax::ast::*;

        match expr {
            Expr::Literal(lit) => match lit.value() {
                Some(LiteralValue::Integer(int)) => {
                    // Parse the integer text
                    if let Ok(value) = int.parse() {
                        Value::Number(value.into())
                    } else {
                        Value::Error(format!("Invalid integer literal: {int:?}").into())
                    }
                }
                Some(LiteralValue::Float(float)) => {
                    // Parse the float text
                    if let Ok(value) = float.parse() {
                        Value::Number(value.into())
                    } else {
                        Value::Error(format!("Invalid float literal: {float:?}").into())
                    }
                }
                Some(LiteralValue::String(string)) => {
                    Value::String(string.syntax().text().to_string())
                }
                Some(LiteralValue::StringWithEscape(string)) => match string.unescaped() {
                    Ok(unescaped) => Value::String(unescaped),
                    Err(span) => {
                        Value::Error(crate::Error::from("Invalid escape sequence").with_span(span))
                    }
                },
                None => Value::Error("Missing literal value".into()),
            },
            Expr::Ident(ident) => {
                let scope = scope.clone();
                let name = ident.syntax().text().interned();
                Value::Symbol { scope, name }
            }
            Expr::Apply(apply) => {
                let receiver = apply
                    .receiver()
                    .and_then(|r| r.value())
                    .map(|e| Object::from_expr(scope, e))
                    .unwrap_or_else(|| Object::new(Value::Error("Missing receiver".into())));

                let all_arguments = apply.all_arguments();
                let count = 1 + all_arguments.len();

                let mut arguments = Vec::with_capacity(count);
                arguments.push(receiver);
                arguments.extend(
                    all_arguments
                        .into_iter()
                        .map(|argument| Object::from_expr(scope, argument)),
                );

                Value::Apply { arguments }
            }
            Expr::Op(op) => {
                let scope = scope.clone();
                let name = op.syntax().text().interned();
                Value::Symbol { scope, name }
            }
            Expr::Synthetic(syn) => {
                let scope = scope.clone();
                let name = syn.identifier().into();
                Value::Symbol { scope, name }
            }
            Expr::Error(err) => Value::Error(crate::error::Error::invalid_syntax(err)),
        }
    }
}

/// Type information for Objects.
///
/// This will be expanded to include all the type system features described
/// in the design document (generics, traits, effects, dimensions, etc.).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Type {
    /// Integer type
    Integer { signed: bool, bits: u8 },

    /// Float type
    Float,

    /// Rational type
    Rational { signed: bool, bits: u8 },

    /// String type
    String,

    /// Boolean type
    Bool,

    /// Character type
    Char,

    /// Unit type
    Unit,

    /// Function type
    Function { parameters_and_return: Vec<Type> },

    /// Type variable (for inference)
    Var(InternedString),

    /// Placeholder for future type system features
    Unknown,
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::Integer { .. } => write!(f, "Integer"),
            Type::Float => write!(f, "Float"),
            Type::Rational { .. } => write!(f, "Rational"),
            Type::String => write!(f, "String"),
            Type::Bool => write!(f, "Bool"),
            Type::Char => write!(f, "Char"),
            Type::Unit => write!(f, "Unit"),
            Type::Function {
                parameters_and_return,
            } => {
                write!(f, "(")?;
                let len = parameters_and_return.len();
                if len == 0 {
                    return write!(f, ")");
                }

                if len == 1 {
                    write!(f, "Unit")?;
                }

                for (i, param) in parameters_and_return.iter().enumerate() {
                    if i > 0 && i < len - 1 {
                        write!(f, ", ")?;
                    } else if i == len - 1 {
                        write!(f, " -> ")?;
                    }
                    write!(f, "{}", param)?;
                    if i == len - 1 {
                        write!(f, ")")?;
                    }
                }

                Ok(())
            }
            Type::Var(name) => write!(f, "{}", name),
            Type::Unknown => write!(f, "?"),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Dict(HashMap<Object, Object, FxBuildHasher>);

impl PartialEq for Dict {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Eq for Dict {}

impl Hash for Dict {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // Hash entries in iteration order (deterministic with FxBuildHasher)
        for (k, v) in &self.0 {
            k.hash(state);
            v.hash(state);
        }
    }
}

pub type Attribute = Arc<[Object]>;

#[derive(Clone, Debug, Default)]
pub struct Attributes(HashMap<InternedString, Vec<Attribute>, FxBuildHasher>);

impl PartialEq for Attributes {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Eq for Attributes {}

impl Hash for Attributes {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // Hash entries in iteration order (deterministic with FxBuildHasher)
        for (k, v) in &self.0 {
            k.hash(state);
            v.hash(state);
        }
    }
}

/// Ownership and memory management metadata.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct OwnershipMetadata {
    /// Ownership status
    pub status: OwnershipStatus,

    /// Deleters to call at this point
    pub deleters: Vec<Deleter>,

    /// Lifetime information
    pub lifetime: Option<Lifetime>,
}

/// Ownership status of a value.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum OwnershipStatus {
    /// Value is owned by this binding
    Owned,

    /// Value has been moved away
    Moved,

    /// Value is borrowed (reference)
    Borrowed,
}

/// A deleter marks cleanup operations.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Deleter {
    /// Call actual delete function
    Proper {
        path: InternedString,
        variable: InternedString,
    },

    /// Marks ownership without actual cleanup
    Fake { variable: InternedString },

    /// Non-managed type (no cleanup, just documentation)
    Primitive { variable: InternedString },

    /// Borrowed value (no cleanup)
    Reference { variable: InternedString },
}

/// Lifetime of a reference.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Lifetime {
    /// Reference depends on local variable
    InsideFunction(InternedString),

    /// Reference depends on something beyond current function
    OutsideFunction,

    /// Static lifetime
    Static,
}

/// Monomorphization metadata.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MonomorphizationMetadata {
    /// Polymorphic suffix for specialized functions
    pub suffix: InternedString,

    /// Type arguments used for specialization
    pub type_args: Vec<Type>,
}

/// Match arm for match expressions.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MatchArm {
    /// Pattern to match
    pub pattern: Object,

    /// Guard condition (optional)
    pub guard: Option<Object>,

    /// Body to execute if matched
    pub body: Object,
}
