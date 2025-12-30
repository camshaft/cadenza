//! Type system for Cadenza.
//!
//! This module implements the complete type system described in the design document,
//! including:
//! - Base types (Integer, Float, Rational, String, Bool, Char, Unit)
//! - Compound types (List, Tuple, Record, Struct, Enum)
//! - Function types with lifetimes
//! - Reference types with lifetime tracking
//! - Type variables for inference
//! - Quantified types (forall)
//! - Refined types with contracts
//! - Dimensional types for units
//! - Constrained types with traits
//! - Effect types

use crate::Object;
use cadenza_tree::InternedString;
use std::{collections::BTreeMap, fmt, sync::Arc};

/// The core type representation in Cadenza.
///
/// Types are values that can be inspected at compile time, forming a hierarchy
/// from base types through refined and constrained types.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Type {
    // ===== Base Types =====
    /// Integer type with optional bit width
    Integer { signed: bool, bits: Option<u8> },

    /// Floating point type (64-bit IEEE 754)
    Float,

    /// Rational number type (numerator/denominator)
    Rational { signed: bool, bits: Option<u8> },

    /// UTF-8 string type
    String,

    /// Boolean type
    Bool,

    /// Character type (Unicode scalar value)
    Char,

    /// Unit type (empty tuple)
    Unit,

    // ===== Compound Types =====
    /// List type (dynamically sized, homogeneous)
    List(Box<Type>),

    /// Tuple type (fixed size, heterogeneous)
    Tuple(Box<[Type]>),

    /// Record type (structural with named fields)
    Record {
        fields: Arc<[InternedString]>,
        values: Box<[Type]>,
    },

    /// Struct type (nominal, user-defined)
    Struct {
        name: InternedString,
        fields: Arc<[InternedString]>,
        values: Box<[Type]>,
    },

    /// Enum type (tagged union)
    Enum {
        name: InternedString,
        variants: Arc<[InternedString]>,
        values: Box<[Type]>,
    },

    // ===== Function Types =====
    /// Function type with parameters, return type, and optional lifetimes
    Function {
        parameters: Box<[Type]>,
        return_type: Box<Type>,
        lifetimes: Box<[Lifetime]>,
    },

    // ===== Reference Types =====
    /// Reference/borrow type with lifetime tracking
    Ref {
        target: Box<Type>,
        lifetime: Lifetime,
        mutable: bool,
    },

    // ===== Type Variables =====
    /// Type variable for inference (unbound)
    Var(InternedString),

    // ===== Quantified Types =====
    /// Universally quantified type (polymorphic)
    Forall {
        type_vars: Arc<[InternedString]>,
        body: Box<Type>,
    },

    // ===== Refined Types =====
    /// Type with contract predicates
    Refined {
        base: Box<Type>,
        predicates: Box<[Predicate]>,
    },

    // ===== Dimensional Types =====
    /// Type with physical dimension (for units of measure)
    Dimensional {
        base: Box<Type>,
        dimension: Dimension,
    },

    // ===== Constrained Types =====
    /// Type with trait constraints
    Constrained {
        base: Box<Type>,
        traits: Box<[TraitConstraint]>,
    },

    // ===== Effect Types =====
    /// Type with effect requirements
    Effectful {
        base: Box<Type>,
        effects: Box<[Effect]>,
    },

    /// Unknown/placeholder type
    Unknown,

    /// Error type for malformed types
    Error,
}

/// Lifetime tracking for references.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Lifetime {
    /// Named lifetime variable
    Named(InternedString),

    /// Static lifetime (lives forever)
    Static,

    /// Reference depends on local variable
    InsideFunction(InternedString),

    /// Reference depends on parameter or global
    OutsideFunction,

    /// Inferred lifetime (to be determined)
    Infer,
}

/// Contract predicate for refined types.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Predicate {
    /// The predicate expression (as a string for now)
    pub expression: Object,

    /// Optional error message
    pub message: Option<InternedString>,
}

/// Physical dimension for dimensional analysis.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Dimension {
    /// Base dimensions with their exponents
    /// E.g., velocity = meter^1 * second^-1
    pub components: Arc<BTreeMap<InternedString, i8>>,
}

impl Dimension {
    /// Create a dimensionless quantity
    pub fn dimensionless() -> Self {
        Self {
            components: Default::default(),
        }
    }

    /// Create a base dimension
    pub fn base(name: InternedString) -> Self {
        Self {
            components: Arc::new([(name, 1)].into_iter().collect()),
        }
    }

    /// Multiply two dimensions
    pub fn multiply(&self, other: &Self) -> Self {
        let mut components = (*self.components).clone();

        // Add components from other
        for (name, exp) in other.components.iter() {
            *components.entry(*name).or_insert(0) += exp;
        }

        components.retain(|_, exp| *exp != 0);

        Self {
            components: components.into(),
        }
    }

    /// Divide two dimensions
    pub fn divide(&self, other: &Self) -> Self {
        let mut components = (*self.components).clone();

        // Add components from other
        for (name, exp) in other.components.iter() {
            *components.entry(*name).or_insert(0) -= exp;
        }

        components.retain(|_, exp| *exp != 0);

        Self {
            components: components.into(),
        }
    }

    /// Raise dimension to a power
    pub fn pow(&self, exponent: i8) -> Self {
        let components: BTreeMap<_, _> = self
            .components
            .iter()
            .map(|(name, exp)| (*name, exp * exponent))
            .filter(|(_, exp)| *exp != 0)
            .collect();

        Self {
            components: components.into(),
        }
    }
}

/// Trait constraint for constrained types.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TraitConstraint {
    /// The trait name
    pub name: InternedString,

    /// Optional associated types
    pub associated_types: Arc<[(InternedString, Type)]>,
}

/// Effect requirement for effectful types.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Effect {
    /// The effect name
    pub name: InternedString,

    /// Optional effect parameters
    pub parameters: Arc<[Type]>,
}

impl Type {
    /// Check if this type is linear (requires memory management)
    pub fn is_linear(&self) -> bool {
        match self {
            // Non-linear primitive types
            Type::Integer { .. }
            | Type::Float
            | Type::Rational { .. }
            | Type::Bool
            | Type::Char
            | Type::Unit => false,

            // Linear types (require delete)
            Type::String | Type::List(_) => true,

            // Function closures are linear if they capture values
            Type::Function { .. } => true,

            // References are non-linear (borrowed, not owned)
            Type::Ref { .. } => false,

            // Compound types: linear if any field is linear
            Type::Tuple(types) => types.iter().any(|t| t.is_linear()),
            Type::Record { values, .. } => values.iter().any(|t| t.is_linear()),
            Type::Struct { values, .. } => values.iter().any(|t| t.is_linear()),
            Type::Enum { values, .. } => values.iter().any(|t| t.is_linear()),

            // Type variables: conservatively assume linear
            Type::Var(_) => true,

            // Wrapped types: check the base
            Type::Forall { body, .. }
            | Type::Refined { base: body, .. }
            | Type::Dimensional { base: body, .. }
            | Type::Constrained { base: body, .. }
            | Type::Effectful { base: body, .. } => body.is_linear(),

            // Unknown and Error: conservatively assume linear
            Type::Unknown | Type::Error => true,
        }
    }

    /// Get the base type, unwrapping wrappers like Refined, Dimensional, etc.
    pub fn base_type(&self) -> &Type {
        match self {
            Type::Refined { base, .. }
            | Type::Dimensional { base, .. }
            | Type::Constrained { base, .. }
            | Type::Effectful { base, .. } => base.base_type(),
            _ => self,
        }
    }

    /// Check if two types are structurally equal (ignoring names for nominal types)
    pub fn structurally_equal(&self, other: &Type) -> bool {
        match (self, other) {
            (
                Type::Integer {
                    signed: s1,
                    bits: b1,
                },
                Type::Integer {
                    signed: s2,
                    bits: b2,
                },
            ) => s1 == s2 && b1 == b2,
            (Type::Float, Type::Float) => true,
            (
                Type::Rational {
                    signed: s1,
                    bits: b1,
                },
                Type::Rational {
                    signed: s2,
                    bits: b2,
                },
            ) => s1 == s2 && b1 == b2,
            (Type::String, Type::String) => true,
            (Type::Bool, Type::Bool) => true,
            (Type::Char, Type::Char) => true,
            (Type::Unit, Type::Unit) => true,
            (Type::List(t1), Type::List(t2)) => t1.structurally_equal(t2),
            (Type::Tuple(ts1), Type::Tuple(ts2)) => {
                ts1.len() == ts2.len()
                    && ts1
                        .iter()
                        .zip(ts2.iter())
                        .all(|(t1, t2)| t1.structurally_equal(t2))
            }
            _ => self == other,
        }
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::Integer {
                signed: true,
                bits: None,
            } => write!(f, "Integer"),
            Type::Integer {
                signed: false,
                bits: None,
            } => write!(f, "Natural"),
            Type::Integer {
                signed: true,
                bits: Some(bits),
            } => write!(f, "i{}", bits),
            Type::Integer {
                signed: false,
                bits: Some(bits),
            } => write!(f, "u{}", bits),
            Type::Float => write!(f, "Float"),
            Type::Rational {
                signed: true,
                bits: None,
            } => write!(f, "Rational"),
            Type::Rational {
                signed: false,
                bits: None,
            } => write!(f, "PositiveRational"),
            Type::Rational {
                signed: true,
                bits: Some(bits),
            } => write!(f, "r{}", bits),
            Type::Rational {
                signed: false,
                bits: Some(bits),
            } => write!(f, "ur{}", bits),
            Type::String => write!(f, "String"),
            Type::Bool => write!(f, "Bool"),
            Type::Char => write!(f, "Char"),
            Type::Unit => write!(f, "Unit"),
            Type::List(inner) => write!(f, "List {}", inner),
            Type::Tuple(types) => {
                write!(f, "(")?;
                for (i, ty) in types.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", ty)?;
                }
                write!(f, ")")
            }
            Type::Record { fields, values } => {
                write!(f, "{{")?;
                for (i, (name, ty)) in fields.iter().zip(values.iter()).enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", name, ty)?;
                }
                write!(f, "}}")
            }
            Type::Struct { name, .. } => write!(f, "{}", name),
            Type::Enum { name, .. } => write!(f, "{}", name),
            Type::Function {
                parameters,
                return_type,
                ..
            } => {
                write!(f, "(")?;
                for (i, param) in parameters.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", param)?;
                }
                write!(f, ") -> {}", return_type)
            }
            Type::Ref {
                target,
                mutable: true,
                ..
            } => write!(f, "&mut {}", target),
            Type::Ref {
                target,
                mutable: false,
                ..
            } => write!(f, "&{}", target),
            Type::Var(name) => write!(f, "{}", name),
            Type::Forall { type_vars, body } => {
                write!(f, "forall ")?;
                for (i, var) in type_vars.iter().enumerate() {
                    if i > 0 {
                        write!(f, " ")?;
                    }
                    write!(f, "{}", var)?;
                }
                write!(f, ". {}", body)
            }
            Type::Refined { base, predicates } => {
                write!(f, "{{ {} |", base)?;
                for (i, pred) in predicates.iter().enumerate() {
                    if i > 0 {
                        write!(f, ",")?;
                    }
                    write!(f, " {:?}", pred.expression)?;
                }
                write!(f, " }}")
            }
            Type::Dimensional { base, dimension } => {
                write!(f, "{} ", base)?;
                if dimension.components.is_empty() {
                    write!(f, "dimensionless")
                } else {
                    for (i, (name, exp)) in dimension.components.iter().enumerate() {
                        if i > 0 {
                            write!(f, "·")?;
                        }
                        write!(f, "{}", name)?;
                        if *exp != 1 {
                            write!(f, "^{}", exp)?;
                        }
                    }
                    Ok(())
                }
            }
            Type::Constrained { base, traits } => {
                write!(f, "{} where ", base)?;
                for (i, constraint) in traits.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", constraint.name)?;
                }
                Ok(())
            }
            Type::Effectful { base, effects } => {
                write!(f, "{} with ", base)?;
                for (i, effect) in effects.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", effect.name)?;
                }
                Ok(())
            }
            Type::Unknown => write!(f, "?"),
            Type::Error => write!(f, "<error>"),
        }
    }
}

impl fmt::Display for Lifetime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Lifetime::Named(name) => write!(f, "'{}", name),
            Lifetime::Static => write!(f, "'static"),
            Lifetime::InsideFunction(var) => write!(f, "'inside({})", var),
            Lifetime::OutsideFunction => write!(f, "'outside"),
            Lifetime::Infer => write!(f, "'_"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_linear() {
        // Non-linear types
        assert!(
            !Type::Integer {
                signed: true,
                bits: None
            }
            .is_linear()
        );
        assert!(!Type::Float.is_linear());
        assert!(!Type::Bool.is_linear());
        assert!(!Type::Unit.is_linear());

        // Linear types
        assert!(Type::String.is_linear());
        assert!(
            Type::List(Box::new(Type::Integer {
                signed: true,
                bits: None
            }))
            .is_linear()
        );

        // Compound types
        assert!(
            !Type::Tuple(Box::new([
                Type::Integer {
                    signed: true,
                    bits: None
                },
                Type::Bool
            ]))
            .is_linear()
        );

        assert!(
            Type::Tuple(Box::new([
                Type::Integer {
                    signed: true,
                    bits: None
                },
                Type::String
            ]))
            .is_linear()
        );
    }

    #[test]
    fn test_dimension_operations() {
        let meter = Dimension::base("meter".into());
        let second = Dimension::base("second".into());

        // Velocity = meter / second
        let velocity = meter.divide(&second);
        assert_eq!(velocity.components.len(), 2);

        // Area = meter * meter
        let area = meter.multiply(&meter);
        assert_eq!(area.components.len(), 1);
        assert_eq!(area.components.get(&"meter".into()), Some(&2));

        // Dimensionless
        let dimensionless = meter.divide(&meter);
        assert_eq!(dimensionless.components.len(), 0);
    }

    #[test]
    fn test_type_display() {
        assert_eq!(
            Type::Integer {
                signed: true,
                bits: None
            }
            .to_string(),
            "Integer"
        );
        assert_eq!(Type::Float.to_string(), "Float");
        assert_eq!(Type::String.to_string(), "String");
        assert_eq!(
            Type::List(Box::new(Type::Integer {
                signed: true,
                bits: None
            }))
            .to_string(),
            "List Integer"
        );
    }
}
