//! Runtime value representation for the Cadenza evaluator.
//!
//! Values can be symbols, lists, functions, macros, or built-in operations.

use crate::{diagnostic::Result, interner::InternedString};
use std::fmt;

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

    /// A built-in function implemented in Rust.
    BuiltinFn(BuiltinFn),

    /// A built-in macro implemented in Rust.
    BuiltinMacro(BuiltinMacro),
}

/// A built-in function type.
#[derive(Clone)]
pub struct BuiltinFn {
    /// The function name for display/debugging.
    pub name: &'static str,
    /// The function implementation.
    pub func: fn(&[Value]) -> Result<Value>,
}

/// A built-in macro type that receives unevaluated syntax nodes.
#[derive(Clone)]
pub struct BuiltinMacro {
    /// The macro name for display/debugging.
    pub name: &'static str,
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

    /// Returns the type name of this value.
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Nil => "nil",
            Value::Bool(_) => "bool",
            Value::Symbol(_) => "symbol",
            Value::Integer(_) => "integer",
            Value::Float(_) => "float",
            Value::String(_) => "string",
            Value::List(_) => "list",
            Value::BuiltinFn(_) => "builtin-fn",
            Value::BuiltinMacro(_) => "builtin-macro",
        }
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
    }
}
