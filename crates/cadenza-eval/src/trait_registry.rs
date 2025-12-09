//! Trait registry for storing and looking up trait definitions and implementations.

use crate::{
    interner::InternedString,
    map::Map,
    value::{MethodSignature, Type, Value},
};

/// A trait definition.
///
/// Traits define a set of methods that types can implement. They are similar to
/// type classes in Haskell or traits in Rust.
#[derive(Debug, Clone, PartialEq)]
pub struct TraitDef {
    /// The name of the trait.
    pub name: InternedString,
    /// The method signatures defined by this trait.
    pub methods: Vec<MethodSignature>,
    /// Type parameters for generic traits (currently unused, reserved for future).
    pub type_params: Vec<InternedString>,
}

impl TraitDef {
    /// Creates a new trait definition.
    pub fn new(name: InternedString, methods: Vec<MethodSignature>) -> Self {
        Self {
            name,
            methods,
            type_params: Vec::new(),
        }
    }

    /// Creates a new trait definition with type parameters.
    pub fn with_type_params(
        name: InternedString,
        methods: Vec<MethodSignature>,
        type_params: Vec<InternedString>,
    ) -> Self {
        Self {
            name,
            methods,
            type_params,
        }
    }
}

/// A trait implementation.
///
/// Connects a type to a trait and provides the method implementations.
#[derive(Debug, Clone, PartialEq)]
pub struct TraitImpl {
    /// The name of the trait being implemented.
    pub trait_name: InternedString,
    /// The type for which the trait is being implemented.
    pub for_type: Type,
    /// The method implementations.
    /// Maps method name to the implementation (a function value).
    pub methods: Map<Value>,
}

impl TraitImpl {
    /// Creates a new trait implementation.
    pub fn new(trait_name: InternedString, for_type: Type, methods: Map<Value>) -> Self {
        Self {
            trait_name,
            for_type,
            methods,
        }
    }
}

/// A registry for trait definitions and implementations.
///
/// The trait registry stores all trait definitions and implementations in the program.
/// It provides lookup methods to find traits and their implementations for specific types.
pub struct TraitRegistry {
    /// All defined traits, indexed by trait name.
    traits: Map<TraitDef>,
    /// All trait implementations.
    /// Key: (type, trait name) -> implementation
    /// Using a Vec instead of a Map because the key is composite
    implementations: Vec<TraitImpl>,
}

impl TraitRegistry {
    /// Creates a new empty trait registry.
    pub fn new() -> Self {
        Self {
            traits: Map::default(),
            implementations: Vec::new(),
        }
    }

    /// Registers a new trait definition.
    ///
    /// # Errors
    /// Returns an error if a trait with the same name already exists.
    pub fn define_trait(&mut self, trait_def: TraitDef) -> Result<(), String> {
        if self.traits.contains_key(&trait_def.name) {
            return Err(format!("Trait {} already defined", &*trait_def.name));
        }
        self.traits.insert(trait_def.name, trait_def);
        Ok(())
    }

    /// Gets a trait definition by name.
    pub fn get_trait(&self, name: InternedString) -> Option<&TraitDef> {
        self.traits.get(&name)
    }

    /// Registers a new trait implementation.
    ///
    /// Multiple implementations for the same (type, trait) pair are not allowed (coherence).
    ///
    /// # Errors
    /// Returns an error if:
    /// - The trait does not exist
    /// - An implementation for this (type, trait) pair already exists
    /// - The implementation methods do not match the trait's method signatures
    pub fn implement_trait(&mut self, trait_impl: TraitImpl) -> Result<(), String> {
        // Check that the trait exists
        let trait_def = self
            .get_trait(trait_impl.trait_name)
            .ok_or_else(|| format!("Trait {} not found", &*trait_impl.trait_name))?;

        // Check for duplicate implementation
        if self
            .find_implementation(&trait_impl.for_type, trait_impl.trait_name)
            .is_some()
        {
            return Err(format!(
                "Trait {} already implemented for type {}",
                &*trait_impl.trait_name, trait_impl.for_type
            ));
        }

        // Validate that all methods are implemented
        for method_sig in &trait_def.methods {
            if !trait_impl.methods.contains_key(&method_sig.name) {
                return Err(format!(
                    "Implementation missing method {}",
                    &*method_sig.name
                ));
            }
        }

        // TODO: Validate that method signatures match (requires type checking)

        self.implementations.push(trait_impl);
        Ok(())
    }

    /// Finds a trait implementation for a specific type and trait.
    pub fn find_implementation(
        &self,
        for_type: &Type,
        trait_name: InternedString,
    ) -> Option<&TraitImpl> {
        self.implementations
            .iter()
            .find(|impl_| impl_.trait_name == trait_name && &impl_.for_type == for_type)
    }

    /// Gets all trait implementations for a specific type.
    pub fn get_implementations_for_type(&self, for_type: &Type) -> Vec<&TraitImpl> {
        self.implementations
            .iter()
            .filter(|impl_| &impl_.for_type == for_type)
            .collect()
    }

    /// Gets all trait implementations for a specific trait.
    pub fn get_implementations_for_trait(&self, trait_name: InternedString) -> Vec<&TraitImpl> {
        self.implementations
            .iter()
            .filter(|impl_| impl_.trait_name == trait_name)
            .collect()
    }

    /// Returns the number of registered traits.
    pub fn num_traits(&self) -> usize {
        self.traits.len()
    }

    /// Returns the number of trait implementations.
    pub fn num_implementations(&self) -> usize {
        self.implementations.len()
    }
}

impl Default for TraitRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::Type;

    #[test]
    fn test_define_trait() {
        let mut registry = TraitRegistry::new();

        let trait_name = InternedString::new("Numeric");
        let add_method = MethodSignature::new(
            InternedString::new("add"),
            vec![Type::Integer, Type::Integer],
            Type::Integer,
        );

        let trait_def = TraitDef::new(trait_name, vec![add_method]);

        assert!(registry.define_trait(trait_def).is_ok());
        assert!(registry.get_trait(trait_name).is_some());
        assert_eq!(registry.num_traits(), 1);
    }

    #[test]
    fn test_duplicate_trait_definition() {
        let mut registry = TraitRegistry::new();

        let trait_name = InternedString::new("Numeric");
        let trait_def1 = TraitDef::new(trait_name, vec![]);
        let trait_def2 = TraitDef::new(trait_name, vec![]);

        assert!(registry.define_trait(trait_def1).is_ok());
        assert!(registry.define_trait(trait_def2).is_err());
    }

    #[test]
    fn test_implement_trait() {
        let mut registry = TraitRegistry::new();

        // Define trait
        let trait_name = InternedString::new("Show");
        let show_method = MethodSignature::new(
            InternedString::new("show"),
            vec![Type::Integer],
            Type::String,
        );
        let trait_def = TraitDef::new(trait_name, vec![show_method]);
        registry.define_trait(trait_def).unwrap();

        // Implement trait for Integer
        let mut methods = Map::default();
        methods.insert(InternedString::new("show"), Value::Nil); // Placeholder
        let trait_impl = TraitImpl::new(trait_name, Type::Integer, methods);

        assert!(registry.implement_trait(trait_impl).is_ok());
        assert_eq!(registry.num_implementations(), 1);

        // Check we can find the implementation
        let found = registry.find_implementation(&Type::Integer, trait_name);
        assert!(found.is_some());
    }

    #[test]
    fn test_implement_nonexistent_trait() {
        let mut registry = TraitRegistry::new();

        let trait_name = InternedString::new("NonExistent");
        let methods = Map::default();
        let trait_impl = TraitImpl::new(trait_name, Type::Integer, methods);

        assert!(registry.implement_trait(trait_impl).is_err());
    }

    #[test]
    fn test_duplicate_implementation() {
        let mut registry = TraitRegistry::new();

        // Define trait
        let trait_name = InternedString::new("Show");
        let trait_def = TraitDef::new(trait_name, vec![]);
        registry.define_trait(trait_def).unwrap();

        // First implementation
        let methods = Map::default();
        let trait_impl1 = TraitImpl::new(trait_name, Type::Integer, methods.clone());
        assert!(registry.implement_trait(trait_impl1).is_ok());

        // Duplicate implementation should fail
        let trait_impl2 = TraitImpl::new(trait_name, Type::Integer, methods);
        assert!(registry.implement_trait(trait_impl2).is_err());
    }
}
