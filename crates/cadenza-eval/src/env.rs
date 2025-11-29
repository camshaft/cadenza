//! Scoped environment for variable bindings.
//!
//! The environment is a stack of scopes, where each scope maps interned
//! identifiers to values. Closures capture the environment by reference.

use crate::interner::InternedId;
use crate::value::Value;
use std::collections::HashMap;

/// A single scope in the environment.
#[derive(Debug, Clone, Default)]
pub struct Scope {
    bindings: HashMap<InternedId, Value>,
}

impl Scope {
    /// Creates a new empty scope.
    pub fn new() -> Self {
        Self::default()
    }

    /// Defines a binding in this scope.
    pub fn define(&mut self, name: InternedId, value: Value) {
        self.bindings.insert(name, value);
    }

    /// Looks up a binding in this scope.
    pub fn get(&self, name: InternedId) -> Option<&Value> {
        self.bindings.get(&name)
    }

    /// Returns true if this scope contains a binding for the given name.
    pub fn contains(&self, name: InternedId) -> bool {
        self.bindings.contains_key(&name)
    }
}

/// A scoped environment as a stack of scopes.
///
/// Variable lookup searches from the top scope to the bottom.
/// New scopes are pushed for function calls and let bindings.
#[derive(Debug, Clone, Default)]
pub struct Env {
    scopes: Vec<Scope>,
}

impl Env {
    /// Creates a new environment with an empty global scope.
    pub fn new() -> Self {
        Self {
            scopes: vec![Scope::new()],
        }
    }

    /// Pushes a new empty scope onto the stack.
    pub fn push_scope(&mut self) {
        self.scopes.push(Scope::new());
    }

    /// Pops the top scope from the stack.
    ///
    /// # Panics
    ///
    /// Panics if there is only one scope (the global scope).
    pub fn pop_scope(&mut self) {
        assert!(
            self.scopes.len() > 1,
            "Cannot pop the global scope"
        );
        self.scopes.pop();
    }

    /// Defines a binding in the current (top) scope.
    pub fn define(&mut self, name: InternedId, value: Value) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.define(name, value);
        }
    }

    /// Looks up a binding, searching from the top scope to the bottom.
    pub fn get(&self, name: InternedId) -> Option<&Value> {
        for scope in self.scopes.iter().rev() {
            if let Some(value) = scope.get(name) {
                return Some(value);
            }
        }
        None
    }

    /// Returns true if any scope contains a binding for the given name.
    pub fn contains(&self, name: InternedId) -> bool {
        self.scopes.iter().any(|scope| scope.contains(name))
    }

    /// Returns the number of scopes in the environment.
    pub fn depth(&self) -> usize {
        self.scopes.len()
    }

    /// Defines a binding in the global (bottom) scope.
    pub fn define_global(&mut self, name: InternedId, value: Value) {
        if let Some(scope) = self.scopes.first_mut() {
            scope.define(name, value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interner::Interner;

    #[test]
    fn define_and_get() {
        let mut interner = Interner::new();
        let name = interner.intern("x");
        let mut env = Env::new();

        env.define(name, Value::Integer(42));
        assert_eq!(env.get(name), Some(&Value::Integer(42)));
    }

    #[test]
    fn shadowing_in_nested_scope() {
        let mut interner = Interner::new();
        let name = interner.intern("x");
        let mut env = Env::new();

        env.define(name, Value::Integer(1));
        env.push_scope();
        env.define(name, Value::Integer(2));

        assert_eq!(env.get(name), Some(&Value::Integer(2)));

        env.pop_scope();
        assert_eq!(env.get(name), Some(&Value::Integer(1)));
    }

    #[test]
    fn lookup_in_parent_scope() {
        let mut interner = Interner::new();
        let name = interner.intern("x");
        let mut env = Env::new();

        env.define(name, Value::Integer(42));
        env.push_scope();

        // Should still be accessible from child scope
        assert_eq!(env.get(name), Some(&Value::Integer(42)));
    }

    #[test]
    fn undefined_variable_returns_none() {
        let mut interner = Interner::new();
        let name = interner.intern("undefined");
        let env = Env::new();

        assert_eq!(env.get(name), None);
    }

    #[test]
    fn define_global() {
        let mut interner = Interner::new();
        let name = interner.intern("x");
        let mut env = Env::new();

        env.push_scope();
        env.define_global(name, Value::Integer(42));
        env.pop_scope();

        // Should be accessible in global scope
        assert_eq!(env.get(name), Some(&Value::Integer(42)));
    }
}
