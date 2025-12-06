//! IR Generator - converts evaluated values to IR.
//!
//! This module generates target-independent IR from Cadenza's evaluated values.
//! The IR is in SSA form and can be used for optimization passes and code generation.
//!
//! Note: IR generation happens after evaluation, not directly from AST. This means
//! we work with Values rather than AST nodes, making the transformation simpler.

use super::{IrBuilder, IrConst};
use crate::{interner::InternedString, value::Value};

/// IR Generator - converts evaluated values to IR.
pub struct IrGenerator {
    builder: IrBuilder,
}

impl IrGenerator {
    /// Create a new IR generator.
    pub fn new() -> Self {
        Self {
            builder: IrBuilder::new(),
        }
    }

    /// Generate IR for a constant value.
    ///
    /// Converts a Cadenza `Value` to an `IrConst`.
    pub fn value_to_const(&self, value: &Value) -> Option<IrConst> {
        match value {
            Value::Nil => Some(IrConst::Nil),
            Value::Bool(b) => Some(IrConst::Bool(*b)),
            Value::Integer(i) => Some(IrConst::Integer(*i)),
            Value::Float(f) => Some(IrConst::Float(*f)),
            Value::String(s) => Some(IrConst::String(InternedString::new(s))),
            Value::Quantity {
                value,
                unit: _,
                dimension,
            } => {
                // Convert DerivedDimension to a single Dimension for IR
                // For now, use the base dimension if it exists, otherwise skip
                // TODO: Properly represent derived dimensions in IR
                if let Some((dim, _power)) = dimension.numerator.first() {
                    Some(IrConst::Quantity {
                        value: *value,
                        dimension: dim.clone(),
                    })
                } else {
                    // No base dimension, treat as plain float
                    Some(IrConst::Float(*value))
                }
            }
            // Non-constant values
            _ => None,
        }
    }

    /// Build and return the final IR module.
    pub fn build(self) -> super::IrModule {
        self.builder.build()
    }

    /// Get a mutable reference to the underlying IrBuilder.
    pub fn builder_mut(&mut self) -> &mut IrBuilder {
        &mut self.builder
    }
}

impl Default for IrGenerator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_value_to_const() {
        let generator = IrGenerator::new();

        assert_eq!(generator.value_to_const(&Value::Nil), Some(IrConst::Nil));
        assert_eq!(
            generator.value_to_const(&Value::Bool(true)),
            Some(IrConst::Bool(true))
        );
        assert_eq!(
            generator.value_to_const(&Value::Integer(42)),
            Some(IrConst::Integer(42))
        );
        assert_eq!(
            generator.value_to_const(&Value::Float(3.14)),
            Some(IrConst::Float(3.14))
        );

        let s = String::from("hello");
        let expected_s = InternedString::new("hello");
        assert_eq!(
            generator.value_to_const(&Value::String(s)),
            Some(IrConst::String(expected_s))
        );
    }
}
