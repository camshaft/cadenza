//! Unit system for dimensional analysis.
//!
//! This module implements a unit system with automatic conversions and
//! dimensional analysis. Units can be defined with conversions to other
//! units, creating bidirectional links between units in the same dimension.
//!
//! # Example
//!
//! ```ignore
//! // Define a base unit
//! measure meter
//!
//! // Define a unit with a conversion
//! measure inch = 25.4mm  // Creates bidirectional link: inch <-> millimeter
//!
//! // Derived units from operations
//! distance / time => velocity
//! ```

use crate::{interner::InternedString, map::Map};
use std::fmt;

/// A dimension in the unit system (e.g., length, time, mass).
///
/// Dimensions are identified by their base unit. Units with the same base
/// unit are part of the same dimension and can be converted to each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Dimension {
    /// The base unit that defines this dimension.
    pub base_unit: InternedString,
}

impl Dimension {
    /// Creates a new dimension with the given base unit.
    pub fn new(base_unit: InternedString) -> Self {
        Self { base_unit }
    }
}

/// A unit with its conversion information.
///
/// Units can have conversions to other units in the same dimension.
/// Conversions are stored as a scale factor and offset.
#[derive(Debug, Clone)]
pub struct Unit {
    /// The name of this unit.
    pub name: InternedString,
    /// The dimension this unit belongs to.
    pub dimension: Dimension,
    /// Conversion to the base unit: base = this * scale + offset
    pub scale: f64,
    pub offset: f64,
}

impl Unit {
    /// Creates a new base unit (no conversion needed, scale=1.0, offset=0.0).
    pub fn base(name: InternedString) -> Self {
        Self {
            name,
            dimension: Dimension::new(name),
            scale: 1.0,
            offset: 0.0,
        }
    }

    /// Creates a derived unit with a conversion to a base unit.
    ///
    /// The scale represents how many base units equal one of this unit.
    /// For example: inch with scale 25.4 and base millimeter means 1 inch = 25.4 mm.
    pub fn derived(name: InternedString, dimension: Dimension, scale: f64, offset: f64) -> Self {
        Self {
            name,
            dimension,
            scale,
            offset,
        }
    }

    /// Converts a value from this unit to another unit in the same dimension.
    ///
    /// Returns None if the units are not in the same dimension.
    pub fn convert_to(&self, value: f64, target: &Unit) -> Option<f64> {
        if self.dimension != target.dimension {
            return None;
        }

        // Convert to base unit first: base = this * scale + offset
        let base_value = value * self.scale + self.offset;

        // Convert from base unit to target: target = (base - target.offset) / target.scale
        let target_value = (base_value - target.offset) / target.scale;

        Some(target_value)
    }
}

/// A derived dimension created from operations on other dimensions.
///
/// For example, velocity is length/time, acceleration is length/time^2.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DerivedDimension {
    /// The numerator dimensions with their powers.
    pub numerator: Vec<(Dimension, i32)>,
    /// The denominator dimensions with their powers.
    pub denominator: Vec<(Dimension, i32)>,
}

impl DerivedDimension {
    /// Creates a new derived dimension from a single dimension.
    pub fn from_dimension(dim: Dimension) -> Self {
        Self {
            numerator: vec![(dim, 1)],
            denominator: vec![],
        }
    }

    /// Multiplies two derived dimensions.
    pub fn multiply(&self, other: &DerivedDimension) -> DerivedDimension {
        let mut numerator = self.numerator.clone();
        let mut denominator = self.denominator.clone();

        // Add other's numerator to our numerator
        for (dim, power) in &other.numerator {
            if let Some(pos) = numerator.iter().position(|(d, _)| d == dim) {
                numerator[pos].1 += power;
            } else {
                numerator.push((*dim, *power));
            }
        }

        // Add other's denominator to our denominator
        for (dim, power) in &other.denominator {
            if let Some(pos) = denominator.iter().position(|(d, _)| d == dim) {
                denominator[pos].1 += power;
            } else {
                denominator.push((*dim, *power));
            }
        }

        // Simplify by moving negative powers to denominator and vice versa
        Self::simplify(numerator, denominator)
    }

    /// Divides two derived dimensions.
    pub fn divide(&self, other: &DerivedDimension) -> DerivedDimension {
        let mut numerator = self.numerator.clone();
        let mut denominator = self.denominator.clone();

        // Add other's denominator to our numerator
        for (dim, power) in &other.denominator {
            if let Some(pos) = numerator.iter().position(|(d, _)| d == dim) {
                numerator[pos].1 += power;
            } else {
                numerator.push((*dim, *power));
            }
        }

        // Add other's numerator to our denominator
        for (dim, power) in &other.numerator {
            if let Some(pos) = denominator.iter().position(|(d, _)| d == dim) {
                denominator[pos].1 += power;
            } else {
                denominator.push((*dim, *power));
            }
        }

        // Simplify by moving negative powers to denominator and vice versa
        Self::simplify(numerator, denominator)
    }

    /// Simplifies a derived dimension by canceling terms and removing zero powers.
    fn simplify(
        mut numerator: Vec<(Dimension, i32)>,
        mut denominator: Vec<(Dimension, i32)>,
    ) -> DerivedDimension {
        // Cancel common dimensions between numerator and denominator
        let mut i = 0;
        while i < numerator.len() {
            let (num_dim, num_power) = numerator[i];
            if let Some(j) = denominator.iter().position(|(d, _)| *d == num_dim) {
                let den_power = denominator[j].1;
                let min_power = num_power.min(den_power);
                numerator[i].1 -= min_power;
                denominator[j].1 -= min_power;
            }
            i += 1;
        }

        // Remove zero powers
        numerator.retain(|(_, power)| *power != 0);
        denominator.retain(|(_, power)| *power != 0);

        DerivedDimension {
            numerator,
            denominator,
        }
    }

    /// Returns true if this is a dimensionless quantity (no dimensions).
    pub fn is_dimensionless(&self) -> bool {
        self.numerator.is_empty() && self.denominator.is_empty()
    }
}

impl fmt::Display for DerivedDimension {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_dimensionless() {
            return write!(f, "dimensionless");
        }

        let mut first = true;
        for (dim, power) in &self.numerator {
            if !first {
                write!(f, "·")?;
            }
            first = false;
            write!(f, "{}", &*dim.base_unit)?;
            if *power != 1 {
                write!(f, "^{}", power)?;
            }
        }

        if !self.denominator.is_empty() {
            write!(f, "/")?;
            let mut first = true;
            for (dim, power) in &self.denominator {
                if !first {
                    write!(f, "·")?;
                }
                first = false;
                write!(f, "{}", &*dim.base_unit)?;
                if *power != 1 {
                    write!(f, "^{}", power)?;
                }
            }
        }

        Ok(())
    }
}

/// The global unit registry.
///
/// This stores all defined units and provides lookup and conversion services.
#[derive(Debug, Default)]
pub struct UnitRegistry {
    /// All defined units, indexed by name.
    units: Map<Unit>,
}

impl UnitRegistry {
    /// Creates a new empty unit registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a new unit.
    pub fn register(&mut self, unit: Unit) {
        self.units.insert(unit.name, unit);
    }

    /// Looks up a unit by name.
    pub fn get(&self, name: InternedString) -> Option<&Unit> {
        self.units.get(&name)
    }

    /// Returns all registered units.
    pub fn all_units(&self) -> impl Iterator<Item = &Unit> {
        self.units.values()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_unit_conversion() {
        let meter: InternedString = "meter".into();
        let meter_unit = Unit::base(meter);

        // Converting a base unit to itself should work
        let result = meter_unit.convert_to(10.0, &meter_unit);
        assert_eq!(result, Some(10.0));
    }

    #[test]
    fn derived_unit_conversion() {
        let millimeter: InternedString = "millimeter".into();
        let inch: InternedString = "inch".into();

        let mm_unit = Unit::base(millimeter);
        let mm_dim = mm_unit.dimension;

        // 1 inch = 25.4 mm
        let inch_unit = Unit::derived(inch, mm_dim, 25.4, 0.0);

        // Convert 1 inch to mm
        let result = inch_unit.convert_to(1.0, &mm_unit);
        assert_eq!(result, Some(25.4));

        // Convert 25.4 mm to inches
        let result = mm_unit.convert_to(25.4, &inch_unit);
        assert_eq!(result, Some(1.0));
    }

    #[test]
    fn different_dimensions_cannot_convert() {
        let meter: InternedString = "meter".into();
        let second: InternedString = "second".into();

        let meter_unit = Unit::base(meter);
        let second_unit = Unit::base(second);

        // Cannot convert between different dimensions
        let result = meter_unit.convert_to(10.0, &second_unit);
        assert_eq!(result, None);
    }

    #[test]
    fn derived_dimension_multiply() {
        let length: InternedString = "meter".into();
        let time: InternedString = "second".into();

        let length_dim = Dimension::new(length);
        let time_dim = Dimension::new(time);

        let length_dd = DerivedDimension::from_dimension(length_dim);
        let time_dd = DerivedDimension::from_dimension(time_dim);

        // length * time
        let result = length_dd.multiply(&time_dd);
        assert_eq!(result.numerator.len(), 2);
        assert!(result.numerator.contains(&(length_dim, 1)));
        assert!(result.numerator.contains(&(time_dim, 1)));
        assert_eq!(result.denominator.len(), 0);
    }

    #[test]
    fn derived_dimension_divide() {
        let length: InternedString = "meter".into();
        let time: InternedString = "second".into();

        let length_dim = Dimension::new(length);
        let time_dim = Dimension::new(time);

        let length_dd = DerivedDimension::from_dimension(length_dim);
        let time_dd = DerivedDimension::from_dimension(time_dim);

        // length / time = velocity
        let velocity = length_dd.divide(&time_dd);
        assert_eq!(velocity.numerator, vec![(length_dim, 1)]);
        assert_eq!(velocity.denominator, vec![(time_dim, 1)]);
    }

    #[test]
    fn derived_dimension_simplify() {
        let length: InternedString = "meter".into();
        let length_dim = Dimension::new(length);

        let length_dd = DerivedDimension::from_dimension(length_dim);

        // (length / length) should simplify to dimensionless
        let result = length_dd.divide(&length_dd);
        assert!(result.is_dimensionless());
    }

    #[test]
    fn unit_registry() {
        let meter: InternedString = "meter".into();
        let inch: InternedString = "inch".into();

        let mut registry = UnitRegistry::new();

        let meter_unit = Unit::base(meter);
        let inch_unit = Unit::derived(inch, meter_unit.dimension, 0.0254, 0.0);

        registry.register(meter_unit.clone());
        registry.register(inch_unit.clone());

        assert!(registry.get(meter).is_some());
        assert!(registry.get(inch).is_some());

        let missing: InternedString = "missing".into();
        assert!(registry.get(missing).is_none());
    }
}
