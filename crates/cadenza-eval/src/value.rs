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

    /// A type value (types are first-class values).
    Type(Type),

    /// A built-in function implemented in Rust.
    BuiltinFn(BuiltinFn),

    /// A built-in macro implemented in Rust.
    /// Macros receive unevaluated AST expressions and return values directly.
    /// This unified type replaces both the old BuiltinMacro and BuiltinSpecialForm.
    BuiltinMacro(BuiltinMacro),
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
            Value::Type(_) => Type::Type,
            Value::BuiltinFn(bf) => bf.signature.clone(),
            Value::BuiltinMacro(bm) => bm.signature.clone(),
        }
    }

    /// Wraps this value with source location information.
    pub fn with_source(self, source: SourceInfo) -> TrackedValue {
        TrackedValue {
            value: self,
            source: Some(source),
        }
    }

    /// Wraps this value with source information extracted from an expression.
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
            source: Some(SourceInfo::from_span(span)),
        }
    }

    /// Wraps this value without source information.
    pub fn without_source(self) -> TrackedValue {
        TrackedValue {
            value: self,
            source: None,
        }
    }
}

/// A value paired with optional source location information.
///
/// This allows values to carry information about where they were created
/// in the source code, which is useful for error reporting and debugging.
#[derive(Clone)]
pub struct TrackedValue {
    /// The actual runtime value.
    pub value: Value,
    /// Source location information, if available.
    pub source: Option<SourceInfo>,
}

impl TrackedValue {
    /// Creates a new tracked value without source information.
    pub fn new(value: Value) -> Self {
        Self {
            value,
            source: None,
        }
    }

    /// Creates a tracked value with source information.
    pub fn with_source(value: Value, source: SourceInfo) -> Self {
        Self {
            value,
            source: Some(source),
        }
    }

    /// Creates a tracked value with source from an expression.
    pub fn from_expr(value: Value, expr: &Expr) -> Self {
        value.from_expr(expr)
    }

    /// Returns a reference to the underlying value.
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// Consumes self and returns the underlying value.
    pub fn into_value(self) -> Value {
        self.value
    }

    /// Returns the source information, if available.
    pub fn source(&self) -> Option<SourceInfo> {
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
        if let Some(source) = self.source {
            write!(f, "{:?}@{:?}", self.value, source.span)
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
            Value::Type(t) => write!(f, "Type({t})"),
            Value::BuiltinFn(bf) => write!(f, "<builtin-fn {}>", bf.name),
            Value::BuiltinMacro(bm) => write!(f, "<builtin-macro {}>", bm.name),
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
            Value::Type(t) => write!(f, "{t}"),
            Value::BuiltinFn(bf) => write!(f, "<builtin-fn {}>", bf.name),
            Value::BuiltinMacro(bm) => write!(f, "<builtin-macro {}>", bm.name),
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
            (Value::Type(a), Value::Type(b)) => a == b,
            // Functions and macros are compared by identity (they never compare equal)
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
    fn source_info_creation() {
        let file: InternedString = "test.cdz".into();
        let span = Span::new(10, 20);
        let source = SourceInfo::new(Some(file), span);
        assert_eq!(source.file, Some(file));
        assert_eq!(source.span, span);
    }

    #[test]
    fn source_info_from_span() {
        let span = Span::new(5, 15);
        let source = SourceInfo::from_span(span);
        assert_eq!(source.file, None);
        assert_eq!(source.span, span);
    }

    #[test]
    fn tracked_value_without_source() {
        let value = Value::Integer(42);
        let tracked = TrackedValue::new(value.clone());
        assert_eq!(tracked.value, value);
        assert_eq!(tracked.source, None);
    }

    #[test]
    fn tracked_value_with_source() {
        let value = Value::Integer(42);
        let span = Span::new(0, 2);
        let source = SourceInfo::from_span(span);
        let tracked = TrackedValue::with_source(value.clone(), source);
        assert_eq!(tracked.value, value);
        assert_eq!(tracked.source, Some(source));
    }

    #[test]
    fn value_with_source_helper() {
        let value = Value::String("hello".to_string());
        let span = Span::new(10, 15);
        let source = SourceInfo::from_span(span);
        let tracked = value.clone().with_source(source);
        assert_eq!(tracked.value, value);
        assert_eq!(tracked.source, Some(source));
    }

    #[test]
    fn tracked_value_from_value() {
        let value = Value::Bool(true);
        let tracked: TrackedValue = value.clone().into();
        assert_eq!(tracked.value, value);
        assert_eq!(tracked.source, None);
    }

    #[test]
    fn tracked_value_into_value() {
        let value = Value::Float(3.15);
        let tracked = TrackedValue::new(value.clone());
        let extracted = tracked.into_value();
        assert_eq!(extracted, value);
    }

    #[test]
    fn tracked_value_display() {
        let value = Value::Integer(123);
        let tracked = TrackedValue::new(value);
        assert_eq!(format!("{tracked}"), "123");
    }

    #[test]
    fn tracked_value_debug_without_source() {
        let value = Value::Integer(42);
        let tracked = TrackedValue::new(value);
        assert_eq!(format!("{tracked:?}"), "42");
    }

    #[test]
    fn tracked_value_debug_with_source() {
        let value = Value::Integer(42);
        let span = Span::new(5, 7);
        let source = SourceInfo::from_span(span);
        let tracked = TrackedValue::with_source(value, source);
        let debug_str = format!("{tracked:?}");
        assert!(debug_str.contains("42"));
        assert!(debug_str.contains("5"));
        assert!(debug_str.contains("7"));
    }
}
