//! Runtime value representation for the Cadenza evaluator.
//!
//! Values can be symbols, lists, functions, macros, or built-in operations.

use crate::{diagnostic::Result, interner::InternedString};
use cadenza_syntax::{ast::Expr, span::Span};
use std::fmt;

/// A runtime type in the Cadenza evaluator.
///
/// Types are first-class values that can be inspected and operated on at runtime.
/// This allows for type-level programming and better error messages.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Type {
    /// The type of nil/unit values.
    Nil,
    /// The type of boolean values.
    Bool,
    /// The type of symbol values.
    Symbol,
    /// The type of integer values.
    Integer,
    /// The type of floating-point values.
    Float,
    /// The type of string values.
    String,
    /// The type of list values with element type.
    List(Box<Type>),
    /// The type of type values.
    Type,
    /// A function type with argument types and return type (last element).
    /// For example, `Fn(vec![Integer, Integer, Integer])` represents `(Integer, Integer) -> Integer`.
    Fn(Vec<Type>),
    /// A record type with field names and types.
    Record(Vec<(InternedString, Type)>),
    /// A nominally-typed struct with a name and field definitions.
    /// Unlike records (structural typing), structs are nominally typed:
    /// two structs with the same fields but different names are different types.
    Struct {
        /// The name of the struct type.
        name: InternedString,
        /// The field names and types.
        fields: Vec<(InternedString, Type)>,
    },
    /// A tuple type with a list of element types.
    Tuple(Vec<Type>),
    /// An enum type with variant names and their associated types.
    Enum(Vec<(InternedString, Type)>),
    /// A union type representing one of several possible types.
    Union(Vec<Type>),
    /// An unknown or unresolved type (used when type inference is incomplete).
    Unknown,
}

impl Type {
    /// Creates a function type from argument types and a return type.
    pub fn function(args: Vec<Type>, ret: Type) -> Self {
        let mut types = args;
        types.push(ret);
        Type::Fn(types)
    }

    /// Creates a union type from a list of types.
    ///
    /// # Panics
    /// Panics if the types vector is empty (unions must have at least one type).
    pub fn union(types: Vec<Type>) -> Self {
        assert!(!types.is_empty(), "union type must have at least one type");
        Type::Union(types)
    }

    /// Creates a list type with the given element type.
    pub fn list(element: Type) -> Self {
        Type::List(Box::new(element))
    }

    /// Returns the string representation of this type.
    pub fn as_str(&self) -> &'static str {
        match self {
            Type::Nil => "nil",
            Type::Bool => "bool",
            Type::Symbol => "symbol",
            Type::Integer => "integer",
            Type::Float => "float",
            Type::String => "string",
            Type::List(_) => "list",
            Type::Type => "type",
            Type::Fn(_) => "fn",
            Type::Record(_) => "record",
            Type::Struct { .. } => "struct",
            Type::Tuple(_) => "tuple",
            Type::Enum(_) => "enum",
            Type::Union(_) => "union",
            Type::Unknown => "unknown",
        }
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::Nil => write!(f, "nil"),
            Type::Bool => write!(f, "bool"),
            Type::Symbol => write!(f, "symbol"),
            Type::Integer => write!(f, "integer"),
            Type::Float => write!(f, "float"),
            Type::String => write!(f, "string"),
            Type::List(elem) => write!(f, "list[{elem}]"),
            Type::Type => write!(f, "type"),
            Type::Fn(types) => {
                if types.is_empty() {
                    write!(f, "fn() -> nil")
                } else {
                    write!(f, "fn(")?;
                    let (args, ret) = types.split_at(types.len() - 1);
                    for (i, t) in args.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{t}")?;
                    }
                    write!(f, ") -> {}", ret[0])
                }
            }
            Type::Record(fields) => {
                write!(f, "{{")?;
                for (i, (name, t)) in fields.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", &**name, t)?;
                }
                write!(f, "}}")
            }
            Type::Struct { name, fields } => {
                write!(f, "struct {} {{", &**name)?;
                for (i, (field_name, t)) in fields.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", &**field_name, t)?;
                }
                write!(f, "}}")
            }
            Type::Tuple(types) => {
                write!(f, "(")?;
                for (i, t) in types.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{t}")?;
                }
                write!(f, ")")
            }
            Type::Enum(variants) => {
                write!(f, "enum[")?;
                for (i, (name, t)) in variants.iter().enumerate() {
                    if i > 0 {
                        write!(f, " | ")?;
                    }
                    write!(f, "{}({})", &**name, t)?;
                }
                write!(f, "]")
            }
            Type::Union(types) => {
                if types.is_empty() {
                    write!(f, "never")
                } else {
                    for (i, t) in types.iter().enumerate() {
                        if i > 0 {
                            write!(f, " | ")?;
                        }
                        write!(f, "{t}")?;
                    }
                    Ok(())
                }
            }
            Type::Unknown => write!(f, "unknown"),
        }
    }
}

/// Source location information for a value.
///
/// This tracks where a value was created in the source code,
/// enabling better error messages and debugging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceInfo {
    /// The source file where this value originated, if known.
    pub file: Option<InternedString>,
    /// The source span where this value was created.
    pub span: Span,
}

impl SourceInfo {
    /// Creates new source info with a file and span.
    pub fn new(file: Option<InternedString>, span: Span) -> Self {
        Self { file, span }
    }

    /// Creates source info from just a span (no file information).
    pub fn from_span(span: Span) -> Self {
        Self { file: None, span }
    }

    /// Creates a synthetic/unknown source info with a zero-length span at position 0.
    ///
    /// Used for values that don't have a meaningful source location (e.g., built-in values,
    /// synthesized values, or values created at runtime).
    pub fn synthetic() -> Self {
        Self {
            file: None,
            span: Span::new(0, 0),
        }
    }
}

/// A runtime value in the Cadenza evaluator.
#[derive(Clone)]
pub enum Value {
    /// The nil/unit value, typically returned from side-effecting operations.
    Nil,

    /// A boolean value.
    Bool(bool),

    /// A symbol (interned identifier).
    Symbol(InternedString),

    /// An integer value (for now using i64, will be rational later).
    Integer(i64),

    /// A floating-point value.
    Float(f64),

    /// A string value.
    String(String),

    /// A list of values.
    List(Vec<Value>),

    /// A record value with named fields.
    ///
    /// When `type_name` is None, this is a structurally-typed record.
    /// When `type_name` is Some, this is a nominally-typed struct instance.
    ///
    /// Structural records are equal if they have the same fields and values.
    /// Nominal structs are equal only if they have the same type name, fields, and values.
    /// Field order is preserved from construction.
    Record {
        /// The type name for nominally-typed structs, None for structural records.
        type_name: Option<InternedString>,
        /// The field values.
        fields: Vec<(InternedString, Value)>,
    },

    /// A struct constructor function.
    ///
    /// When a struct is defined, a constructor function with the struct's name is created.
    /// This constructor takes field assignments and creates struct instances.
    /// Example: `Point { x = 1, y = 2 }` where `Point` is a StructConstructor.
    StructConstructor {
        /// The name of the struct type.
        name: InternedString,
        /// The field definitions (field name and type).
        field_types: Vec<(InternedString, Type)>,
    },

    /// A type value (types are first-class values).
    Type(Type),

    /// A quantity with a unit (for dimensional analysis).
    ///
    /// Represents a numeric value with an associated unit and dimension.
    /// Used for automatic unit conversions and dimensional analysis.
    Quantity {
        /// The numeric value.
        value: f64,
        /// The unit of this quantity.
        unit: crate::unit::Unit,
        /// The derived dimension (for operations that create new dimensions).
        dimension: crate::unit::DerivedDimension,
    },

    /// A unit constructor that creates quantities when applied to numbers.
    ///
    /// When a unit name is used as a function (e.g., `meter 5`), it creates a quantity.
    UnitConstructor(crate::unit::Unit),

    /// A built-in function implemented in Rust.
    BuiltinFn(BuiltinFn),

    /// A built-in macro implemented in Rust.
    /// Macros receive unevaluated AST expressions and return values directly.
    /// This unified type replaces both the old BuiltinMacro and BuiltinSpecialForm.
    ///
    /// DEPRECATED: Use SpecialForm instead. This is kept for backward compatibility.
    BuiltinMacro(BuiltinMacro),

    /// A special form that provides both evaluation and IR generation.
    ///
    /// Special forms are built-in language constructs that define the base layer
    /// for interacting with the evaluator, type system, and IR builder.
    /// Unlike macros (syntax tree to syntax tree), special forms provide both
    /// evaluation semantics and IR generation logic.
    SpecialForm(&'static crate::special_form::BuiltinSpecialForm),

    /// A user-defined function with parameter names and body expression.
    UserFunction(UserFunction),
}

/// A built-in function type with type signature.
///
/// The function receives both the evaluated arguments and an [`EvalContext`]
/// providing access to the environment and compiler.
#[derive(Clone)]
pub struct BuiltinFn {
    /// The function name for display/debugging.
    pub name: &'static str,
    /// The type signature of this function (argument types + return type).
    pub signature: Type,
    /// The function implementation.
    ///
    /// Takes the evaluated arguments and an evaluation context that provides
    /// access to the environment and compiler.
    pub func: fn(&[Value], &mut crate::context::EvalContext<'_>) -> Result<Value>,
}

/// A built-in macro type that receives unevaluated AST expressions.
///
/// Macros receive unevaluated AST nodes and return values directly.
/// This unified type now takes `&[Expr]` arguments (previously used `&[GreenNode]`)
/// and returns `Value` directly, replacing the old separation between BuiltinMacro
/// (which returned GreenNode) and BuiltinSpecialForm (which returned Value).
#[derive(Clone)]
pub struct BuiltinMacro {
    /// The macro name for display/debugging.
    pub name: &'static str,
    /// The type signature of this macro (argument types + return type).
    pub signature: Type,
    /// The macro implementation (receives unevaluated AST expressions).
    ///
    /// Takes unevaluated Expr AST nodes and an evaluation context that provides
    /// access to the environment and compiler. Returns a Value directly.
    pub func: fn(&[Expr], &mut crate::context::EvalContext<'_>) -> Result<Value>,
}

/// A user-defined function with captured environment.
///
/// User functions are closures that capture their lexical environment at definition time.
/// When called, they evaluate their body in a new scope that extends the captured environment
/// with bindings for the parameters.
#[derive(Clone)]
pub struct UserFunction {
    /// The function name for display/debugging.
    pub name: InternedString,
    /// The parameter names (in order).
    pub params: Vec<InternedString>,
    /// The function body expression.
    pub body: Expr,
    /// The captured environment at function definition time.
    /// This enables closure semantics - the function uses this environment
    /// when called, not the caller's environment.
    pub captured_env: crate::env::Env,
}

impl Value {
    /// Returns true if this value is nil.
    pub fn is_nil(&self) -> bool {
        matches!(self, Value::Nil)
    }

    /// Tries to convert this value to a boolean.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Tries to convert this value to an integer.
    pub fn as_integer(&self) -> Option<i64> {
        match self {
            Value::Integer(n) => Some(*n),
            _ => None,
        }
    }

    /// Tries to convert this value to a symbol ID.
    pub fn as_symbol(&self) -> Option<InternedString> {
        match self {
            Value::Symbol(id) => Some(*id),
            _ => None,
        }
    }

    /// Returns the runtime type of this value.
    pub fn type_of(&self) -> Type {
        match self {
            Value::Nil => Type::Nil,
            Value::Bool(_) => Type::Bool,
            Value::Symbol(_) => Type::Symbol,
            Value::Integer(_) => Type::Integer,
            Value::Float(_) => Type::Float,
            Value::String(_) => Type::String,
            // For lists, we use Unknown since we don't track element types at runtime yet
            Value::List(_) => Type::list(Type::Unknown),
            // For records, extract field types
            // Structural records (type_name = None) use Unknown for field types
            // Nominal structs (type_name = Some) return Struct type with field types
            Value::Record { type_name, fields } => {
                let field_types = fields
                    .iter()
                    .map(|(name, val)| (*name, val.type_of()))
                    .collect();
                match type_name {
                    None => Type::Record(field_types),
                    Some(name) => Type::Struct {
                        name: *name,
                        fields: field_types,
                    },
                }
            }
            // For struct constructors, return a function type that takes the struct type
            // and returns a struct instance
            Value::StructConstructor { name, field_types } => {
                Type::function(
                    vec![Type::Record(field_types.clone())],
                    Type::Struct {
                        name: *name,
                        fields: field_types.clone(),
                    },
                )
            }
            Value::Type(_) => Type::Type,
            Value::Quantity { .. } => Type::Float, // Quantities are numeric
            Value::UnitConstructor(_) => Type::function(vec![Type::Float], Type::Float),
            Value::BuiltinFn(bf) => bf.signature.clone(),
            Value::BuiltinMacro(bm) => bm.signature.clone(),
            Value::SpecialForm(sf) => sf.signature(),
            Value::UserFunction(uf) => {
                // Create a function type with Unknown parameter types and return type
                // since we don't track types at compile time yet
                let param_types = vec![Type::Unknown; uf.params.len()];
                Type::function(param_types, Type::Unknown)
            }
        }
    }

    /// Wraps this value with source location information.
    pub fn with_source(self, source: SourceInfo) -> TrackedValue {
        TrackedValue {
            value: self,
            source,
        }
    }

    /// Wraps this value with source information extracted from an expression.
    ///
    /// Extracts the text range from the expression's syntax node and converts it
    /// to a Span. This conversion is necessary as rowan's TextRange uses a different
    /// representation than our Span type.
    pub fn from_expr(self, expr: &Expr) -> TrackedValue {
        let syntax = match expr {
            Expr::Literal(lit) => lit.syntax(),
            Expr::Ident(ident) => ident.syntax(),
            Expr::Apply(apply) => apply.syntax(),
            Expr::Attr(attr) => attr.syntax(),
            Expr::Op(op) => op.syntax(),
            Expr::Synthetic(syn) => syn.syntax(),
            Expr::Error(err) => err.syntax(),
        };
        let span = Span::new(
            syntax.text_range().start().into(),
            syntax.text_range().end().into(),
        );
        TrackedValue {
            value: self,
            source: SourceInfo::from_span(span),
        }
    }

    /// Wraps this value with synthetic (unknown) source information.
    pub fn without_source(self) -> TrackedValue {
        TrackedValue {
            value: self,
            source: SourceInfo::synthetic(),
        }
    }
}

/// A value paired with source location information.
///
/// This allows values to carry information about where they were created
/// in the source code, which is useful for error reporting and debugging.
#[derive(Clone)]
pub struct TrackedValue {
    /// The actual runtime value.
    pub value: Value,
    /// Source location information.
    pub source: SourceInfo,
}

impl TrackedValue {
    /// Creates a new tracked value with synthetic (unknown) source information.
    pub fn new(value: Value) -> Self {
        Self {
            value,
            source: SourceInfo::synthetic(),
        }
    }

    /// Creates a tracked value with source information.
    pub fn with_source(value: Value, source: SourceInfo) -> Self {
        Self { value, source }
    }

    /// Returns a reference to the underlying value.
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// Consumes self and returns the underlying value.
    pub fn into_value(self) -> Value {
        self.value
    }

    /// Returns the source information.
    pub fn source(&self) -> SourceInfo {
        self.source
    }
}

impl From<Value> for TrackedValue {
    fn from(value: Value) -> Self {
        Self::new(value)
    }
}

impl fmt::Debug for TrackedValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Only show source info if it's not synthetic (has a non-zero span)
        if self.source.span.start != 0 || self.source.span.end != 0 {
            write!(f, "{:?}@{:?}", self.value, self.source.span)
        } else {
            write!(f, "{:?}", self.value)
        }
    }
}

impl fmt::Display for TrackedValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.value)
    }
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Nil => write!(f, "nil"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Symbol(id) => write!(f, "Symbol({id:?})"),
            Value::Integer(n) => write!(f, "{n}"),
            Value::Float(n) => write!(f, "{n}"),
            Value::String(s) => write!(f, "{s:?}"),
            Value::List(items) => f.debug_list().entries(items).finish(),
            Value::Record { type_name, fields } => {
                match type_name {
                    None => {
                        // Structural record
                        write!(f, "{{")?;
                        for (i, (name, value)) in fields.iter().enumerate() {
                            if i > 0 {
                                write!(f, ", ")?;
                            }
                            write!(f, "{}: {:?}", &**name, value)?;
                        }
                        write!(f, "}}")
                    }
                    Some(name) => {
                        // Nominal struct
                        write!(f, "Struct({} {{", &**name)?;
                        for (i, (field_name, value)) in fields.iter().enumerate() {
                            if i > 0 {
                                write!(f, ", ")?;
                            }
                            write!(f, "{}: {:?}", &**field_name, value)?;
                        }
                        write!(f, "}})")
                    }
                }
            }
            Value::StructConstructor { name, field_types } => {
                write!(f, "StructConstructor({}, {} fields)", &**name, field_types.len())
            }
            Value::Type(t) => write!(f, "Type({t})"),
            Value::Quantity {
                value,
                unit,
                dimension,
            } => write!(f, "Quantity({} {} [{}])", value, &*unit.name, dimension),
            Value::UnitConstructor(unit) => write!(f, "<unit-constructor {}>", &*unit.name),
            Value::BuiltinFn(bf) => write!(f, "<builtin-fn {}>", bf.name),
            Value::BuiltinMacro(bm) => write!(f, "<builtin-macro {}>", bm.name),
            Value::SpecialForm(sf) => write!(f, "<special-form {}>", sf.name()),
            Value::UserFunction(uf) => write!(f, "<fn {}>", &*uf.name),
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Nil => write!(f, "nil"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Symbol(id) => write!(f, "#{}", &**id),
            Value::Integer(n) => write!(f, "{n}"),
            Value::Float(n) => write!(f, "{n}"),
            Value::String(s) => write!(f, "{s}"),
            Value::List(items) => {
                write!(f, "[")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{item}")?;
                }
                write!(f, "]")
            }
            Value::Record { type_name, fields } => {
                match type_name {
                    None => {
                        // Structural record
                        write!(f, "{{")?;
                        for (i, (name, value)) in fields.iter().enumerate() {
                            if i > 0 {
                                write!(f, ", ")?;
                            }
                            write!(f, "{} = {}", &**name, value)?;
                        }
                        write!(f, "}}")
                    }
                    Some(type_name) => {
                        // Nominal struct
                        write!(f, "{} {{", &**type_name)?;
                        for (i, (name, value)) in fields.iter().enumerate() {
                            if i > 0 {
                                write!(f, ", ")?;
                            }
                            write!(f, "{} = {}", &**name, value)?;
                        }
                        write!(f, "}}")
                    }
                }
            }
            Value::StructConstructor { name, .. } => {
                write!(f, "<struct-constructor {}>", &**name)
            }
            Value::Type(t) => write!(f, "{t}"),
            Value::Quantity {
                value,
                unit,
                dimension: _,
            } => write!(f, "{}{}", value, &*unit.name),
            Value::UnitConstructor(unit) => write!(f, "<unit-constructor {}>", &*unit.name),
            Value::BuiltinFn(bf) => write!(f, "<builtin-fn {}>", bf.name),
            Value::BuiltinMacro(bm) => write!(f, "<builtin-macro {}>", bm.name),
            Value::SpecialForm(sf) => write!(f, "<special-form {}>", sf.name()),
            Value::UserFunction(uf) => write!(f, "<fn {}>", &*uf.name),
        }
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Nil, Value::Nil) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Symbol(a), Value::Symbol(b)) => a == b,
            (Value::Integer(a), Value::Integer(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => a == b,
            (Value::String(a), Value::String(b)) => a == b,
            (Value::List(a), Value::List(b)) => a == b,
            (Value::Record { type_name: n1, fields: f1 }, Value::Record { type_name: n2, fields: f2 }) => {
                // Structural records (type_name = None) are equal if fields match
                // Nominal structs (type_name = Some) must also have matching type names
                match (n1, n2) {
                    (None, None) => f1 == f2,  // Structural equality
                    (Some(name1), Some(name2)) => name1 == name2 && f1 == f2,  // Nominal equality
                    _ => false,  // Structural vs nominal are never equal
                }
            }
            (Value::StructConstructor { name: n1, .. }, Value::StructConstructor { name: n2, .. }) => {
                // Struct constructors are equal if they construct the same struct type
                n1 == n2
            }
            (Value::Type(a), Value::Type(b)) => a == b,
            (
                Value::Quantity {
                    value: v1,
                    unit: u1,
                    dimension: d1,
                },
                Value::Quantity {
                    value: v2,
                    unit: u2,
                    dimension: d2,
                },
            ) => {
                // Quantities are equal if they're in the same dimension and convert to the same value
                if d1 != d2 {
                    return false;
                }
                // Convert v2 to u1's units and compare
                if let Some(converted) = u2.convert_to(*v2, u1) {
                    (v1 - converted).abs() < 1e-10 // Use epsilon for floating point comparison
                } else {
                    false
                }
            }
            // Functions, macros, and special forms are compared by identity (they never compare equal)
            (Value::BuiltinFn(_), _) | (_, Value::BuiltinFn(_)) => false,
            (Value::BuiltinMacro(_), _) | (_, Value::BuiltinMacro(_)) => false,
            (Value::SpecialForm(_), _) | (_, Value::SpecialForm(_)) => false,
            (Value::UnitConstructor(_), _) | (_, Value::UnitConstructor(_)) => false,
            (Value::UserFunction(_), _) | (_, Value::UserFunction(_)) => false,
            // All other combinations are false (different types)
            _ => false,
        }
    }
}

impl Eq for Value {}

#[cfg(test)]
mod tests {
    use super::*;
    use cadenza_syntax::span::Span;

    #[test]
    fn type_of_returns_correct_types() {
        assert_eq!(Value::Nil.type_of(), Type::Nil);
        assert_eq!(Value::Bool(true).type_of(), Type::Bool);
        assert_eq!(Value::Integer(42).type_of(), Type::Integer);
        assert_eq!(Value::Float(2.5).type_of(), Type::Float);
        assert_eq!(Value::String("hello".to_string()).type_of(), Type::String);
        assert_eq!(Value::List(vec![]).type_of(), Type::list(Type::Unknown));
        assert_eq!(Value::Type(Type::Integer).type_of(), Type::Type);
    }

    #[test]
    fn type_display() {
        assert_eq!(Type::Nil.to_string(), "nil");
        assert_eq!(Type::Bool.to_string(), "bool");
        assert_eq!(Type::Integer.to_string(), "integer");
        assert_eq!(Type::Float.to_string(), "float");
        assert_eq!(Type::String.to_string(), "string");
        assert_eq!(Type::list(Type::Integer).to_string(), "list[integer]");
        assert_eq!(Type::Type.to_string(), "type");
        assert_eq!(Type::Unknown.to_string(), "unknown");
    }

    #[test]
    fn type_display_uses_display_impl() {
        // Use Display impl (via to_string) instead of type_name()
        assert_eq!(Value::Nil.type_of().to_string(), "nil");
        assert_eq!(Value::Bool(true).type_of().to_string(), "bool");
        assert_eq!(Value::Integer(1).type_of().to_string(), "integer");
        assert_eq!(Value::List(vec![]).type_of().to_string(), "list[unknown]");
        assert_eq!(Value::Type(Type::Integer).type_of().to_string(), "type");
    }

    #[test]
    fn empty_union_displays_as_never() {
        // Empty union represents an impossible/never type
        let empty_union = Type::Union(vec![]);
        assert_eq!(empty_union.to_string(), "never");
    }

    #[test]
    fn fn_type_display() {
        // (Integer, Integer) -> Integer
        let fn_type = Type::function(vec![Type::Integer, Type::Integer], Type::Integer);
        assert_eq!(fn_type.to_string(), "fn(integer, integer) -> integer");

        // () -> Bool
        let thunk_type = Type::function(vec![], Type::Bool);
        assert_eq!(thunk_type.to_string(), "fn() -> bool");
    }

    #[test]
    fn union_type_display() {
        let union_type = Type::union(vec![Type::Integer, Type::Float]);
        assert_eq!(union_type.to_string(), "integer | float");
    }

    #[test]
    fn record_type_display() {
        let name: InternedString = "name".into();
        let age: InternedString = "age".into();
        let record_type = Type::Record(vec![(name, Type::String), (age, Type::Integer)]);
        assert_eq!(record_type.to_string(), "{name: string, age: integer}");
    }

    #[test]
    fn tuple_type_display() {
        let tuple_type = Type::Tuple(vec![Type::Integer, Type::String, Type::Bool]);
        assert_eq!(tuple_type.to_string(), "(integer, string, bool)");
    }

    #[test]
    fn type_value_display_and_debug() {
        let type_val = Value::Type(Type::Integer);
        assert_eq!(format!("{type_val}"), "integer");
        assert_eq!(format!("{type_val:?}"), "Type(integer)");
    }

    #[test]
    fn type_values_are_equal() {
        assert_eq!(Value::Type(Type::Integer), Value::Type(Type::Integer));
        assert_ne!(Value::Type(Type::Integer), Value::Type(Type::Float));
    }

    #[test]
    fn tracked_value_debug_with_source() {
        // Verifies that debug output includes both value and source information
        // Expected format: "42@Span { start: 5, end: 7 }"
        let value = Value::Integer(42);
        let span = Span::new(5, 7);
        let source = SourceInfo::from_span(span);
        let tracked = TrackedValue::with_source(value, source);
        let debug_str = format!("{tracked:?}");
        assert!(debug_str.contains("42"));
        assert!(debug_str.contains("5"));
        assert!(debug_str.contains("7"));
    }

    #[test]
    fn tracked_value_display_shows_value_only() {
        // Verifies that Display output shows only the value, not source information
        // Debug shows "42@Span{...}" while Display shows just "123"
        // This allows values with source info to be printed cleanly in normal output
        let value = Value::Integer(123);
        let span = Span::new(10, 13);
        let source = SourceInfo::from_span(span);
        let tracked = TrackedValue::with_source(value, source);
        assert_eq!(format!("{tracked}"), "123");
    }
}
