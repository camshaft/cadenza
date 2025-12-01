//! Scoped environment for variable bindings.
//!
//! The environment is a stack of scopes, where each scope maps interned
//! identifiers to values. Closures capture the environment by reference.

use crate::{
    eval::{builtin_assign, builtin_let},
    interner::InternedString,
    map::Map,
    value::Value,
};

/// A single scope in the environment.
#[derive(Debug, Clone, Default)]
pub struct Scope {
    bindings: Map<Value>,
}

impl Scope {
    /// Creates a new empty scope.
    pub fn new() -> Self {
        Self::default()
    }

    /// Defines a binding in this scope.
    pub fn define(&mut self, name: InternedString, value: Value) {
        self.bindings.insert(name, value);
    }

    /// Looks up a binding in this scope.
    pub fn get(&self, name: InternedString) -> Option<&Value> {
        self.bindings.get(&name)
    }

    /// Looks up a mutable binding in this scope.
    pub fn get_mut(&mut self, name: InternedString) -> Option<&mut Value> {
        self.bindings.get_mut(&name)
    }

    /// Returns true if this scope contains a binding for the given name.
    pub fn contains(&self, name: InternedString) -> bool {
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

    /// Creates a new environment with standard built-in forms and functions.
    ///
    /// This registers all standard built-ins including:
    /// - `let` - Variable declaration special form
    /// - `=` - Assignment special form
    ///
    /// Use this when you want an environment ready for typical evaluation.
    pub fn with_standard_builtins() -> Self {
        let mut env = Self::new();
        env.register_standard_builtins();
        env
    }

    /// Registers all standard built-in forms and functions in the current environment.
    ///
    /// This registers:
    /// - `let` - Variable declaration special form
    /// - `=` - Assignment special form
    ///
    /// This can be called on an existing environment to add the standard built-ins.
    pub fn register_standard_builtins(&mut self) {
        let let_id: InternedString = "let".into();
        let assign_id: InternedString = "=".into();

        self.define(let_id, Value::BuiltinSpecialForm(builtin_let()));
        self.define(assign_id, Value::BuiltinSpecialForm(builtin_assign()));
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
        assert!(self.scopes.len() > 1, "Cannot pop the global scope");
        self.scopes.pop();
    }

    /// Defines a binding in the current (top) scope.
    pub fn define(&mut self, name: InternedString, value: Value) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.define(name, value);
        }
    }

    /// Looks up a binding, searching from the top scope to the bottom.
    pub fn get(&self, name: InternedString) -> Option<&Value> {
        for scope in self.scopes.iter().rev() {
            if let Some(value) = scope.get(name) {
                return Some(value);
            }
        }
        None
    }

    /// Looks up a mutable binding, searching from the top scope to the bottom.
    /// Used by the `=` operator to update values.
    pub fn get_mut(&mut self, name: InternedString) -> Option<&mut Value> {
        for scope in self.scopes.iter_mut().rev() {
            if let Some(value) = scope.get_mut(name) {
                return Some(value);
            }
        }
        None
    }

    /// Returns true if any scope contains a binding for the given name.
    pub fn contains(&self, name: InternedString) -> bool {
        self.scopes.iter().any(|scope| scope.contains(name))
    }

    /// Returns the number of scopes in the environment.
    pub fn depth(&self) -> usize {
        self.scopes.len()
    }

    /// Defines a binding in the global (bottom) scope.
    pub fn define_global(&mut self, name: InternedString, value: Value) {
        if let Some(scope) = self.scopes.first_mut() {
            scope.define(name, value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn define_and_get() {
        let name: InternedString = "x".into();
        let mut env = Env::new();

        env.define(name, Value::Integer(42));
        assert_eq!(env.get(name), Some(&Value::Integer(42)));
    }

    #[test]
    fn shadowing_in_nested_scope() {
        let name: InternedString = "x".into();
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
        let name: InternedString = "x".into();
        let mut env = Env::new();

        env.define(name, Value::Integer(42));
        env.push_scope();

        // Should still be accessible from child scope
        assert_eq!(env.get(name), Some(&Value::Integer(42)));
    }

    #[test]
    fn undefined_variable_returns_none() {
        let name: InternedString = "undefined".into();
        let env = Env::new();

        assert_eq!(env.get(name), None);
    }

    #[test]
    fn define_global() {
        let name: InternedString = "x".into();
        let mut env = Env::new();

        env.push_scope();
        env.define_global(name, Value::Integer(42));
        env.pop_scope();

        // Should be accessible in global scope
        assert_eq!(env.get(name), Some(&Value::Integer(42)));
    }

    #[test]
    fn with_standard_builtins() {
        let env = Env::with_standard_builtins();

        // Should have `let` and `=` special forms registered
        let let_id: InternedString = "let".into();
        let assign_id: InternedString = "=".into();

        assert!(
            matches!(env.get(let_id), Some(Value::BuiltinSpecialForm(_))),
            "Expected `let` to be registered as a BuiltinSpecialForm"
        );
        assert!(
            matches!(env.get(assign_id), Some(Value::BuiltinSpecialForm(_))),
            "Expected `=` to be registered as a BuiltinSpecialForm"
        );
    }

    #[test]
    fn register_standard_builtins_on_existing_env() {
        let mut env = Env::new();

        // Define a custom variable first
        let custom_id: InternedString = "custom".into();
        env.define(custom_id, Value::Integer(42));

        // Register standard builtins
        env.register_standard_builtins();

        // Should have both the custom variable and the standard builtins
        assert_eq!(env.get(custom_id), Some(&Value::Integer(42)));

        let let_id: InternedString = "let".into();
        let assign_id: InternedString = "=".into();
        assert!(
            matches!(env.get(let_id), Some(Value::BuiltinSpecialForm(_))),
            "Expected `let` to be registered as a BuiltinSpecialForm"
        );
        assert!(
            matches!(env.get(assign_id), Some(Value::BuiltinSpecialForm(_))),
            "Expected `=` to be registered as a BuiltinSpecialForm"
        );
    }
}
