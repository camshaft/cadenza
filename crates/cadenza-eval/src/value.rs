//! Runtime value representation for the Cadenza evaluator.
//!
//! Values can be symbols, lists, functions, macros, or built-in operations.

use crate::{diagnostic::Result, interner::InternedString};
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
}

impl Type {
    /// Creates a function type from argument types and a return type.
    pub fn function(args: Vec<Type>, ret: Type) -> Self {
        let mut types = args;
        types.push(ret);
        Type::Fn(types)
    }

    /// Creates a union type from a list of types.
    pub fn union(types: Vec<Type>) -> Self {
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
                for (i, t) in types.iter().enumerate() {
                    if i > 0 {
                        write!(f, " | ")?;
                    }
                    write!(f, "{t}")?;
                }
                Ok(())
            }
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

    /// A type value (types are first-class values).
    Type(Type),

    /// A built-in function implemented in Rust.
    BuiltinFn(BuiltinFn),

    /// A built-in macro implemented in Rust.
    BuiltinMacro(BuiltinMacro),
}

/// A built-in function type with type signature.
#[derive(Clone)]
pub struct BuiltinFn {
    /// The function name for display/debugging.
    pub name: &'static str,
    /// The type signature of this function (argument types + return type).
    pub signature: Type,
    /// The function implementation.
    pub func: fn(&[Value]) -> Result<Value>,
}

/// A built-in macro type that receives unevaluated syntax nodes.
#[derive(Clone)]
pub struct BuiltinMacro {
    /// The macro name for display/debugging.
    pub name: &'static str,
    /// The type signature of this macro (argument types + return type).
    pub signature: Type,
    /// The macro implementation (receives unevaluated syntax nodes).
    pub func: fn(&[rowan::GreenNode]) -> Result<rowan::GreenNode>,
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
            // For lists, we use a dynamic type since we don't track element types at runtime yet
            Value::List(_) => Type::list(Type::Nil), // TODO: infer element type
            Value::Type(_) => Type::Type,
            Value::BuiltinFn(bf) => bf.signature.clone(),
            Value::BuiltinMacro(bm) => bm.signature.clone(),
        }
    }

    /// Returns the type name of this value as a string.
    ///
    /// This is a convenience method that returns `self.type_of().as_str()`.
    pub fn type_name(&self) -> &'static str {
        self.type_of().as_str()
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

    #[test]
    fn type_names_are_correct() {
        assert_eq!(Value::Nil.type_name(), "nil");
        assert_eq!(Value::Bool(true).type_name(), "bool");
        assert_eq!(Value::Integer(1).type_name(), "integer");
        assert_eq!(Value::List(vec![]).type_name(), "list");
        assert_eq!(Value::Type(Type::Integer).type_name(), "type");
    }

    #[test]
    fn type_of_returns_correct_types() {
        assert_eq!(Value::Nil.type_of(), Type::Nil);
        assert_eq!(Value::Bool(true).type_of(), Type::Bool);
        assert_eq!(Value::Integer(42).type_of(), Type::Integer);
        assert_eq!(Value::Float(2.5).type_of(), Type::Float);
        assert_eq!(Value::String("hello".to_string()).type_of(), Type::String);
        assert_eq!(Value::List(vec![]).type_of(), Type::list(Type::Nil));
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
}
