//! Scoped environment for variable bindings.
//!
//! The environment is a stack of scopes, where each scope maps interned
//! identifiers to values. Closures capture the environment by reference.

use crate::{
    interner::InternedString,
    map::Map,
    special_form,
    value::Value,
};
use std::{collections::HashSet, rc::Rc};

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
///
/// Uses Rc for cheap cloning - useful for closures that capture their environment.
#[derive(Debug, Clone, Default)]
pub struct Env {
    scopes: Rc<Vec<Scope>>,
}

impl Env {
    /// Creates a new environment with an empty global scope.
    pub fn new() -> Self {
        Self {
            scopes: Rc::new(vec![Scope::new()]),
        }
    }

    /// Creates a new environment with standard built-in forms and functions.
    ///
    /// This registers all standard built-ins including:
    /// - `let` - Variable declaration macro
    /// - `=` - Assignment macro
    /// - `fn` - Function definition macro
    /// - `assert` - Assertion macro for runtime checks
    /// - `measure` - Unit definition macro for dimensional analysis
    /// - `|>` - Pipeline operator macro
    /// - `__block__` - Block expression macro (automatically emitted by parser)
    /// - `__list__` - List literal macro (automatically emitted by parser)
    /// - `__record__` - Record literal macro (automatically emitted by parser)
    /// - `__index__` - Array indexing macro (automatically emitted by parser)
    /// - Arithmetic operators: `+`, `-`, `*`, `/`
    /// - Comparison operators: `==`, `!=`, `<`, `<=`, `>`, `>=`
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
    /// - `let` - Variable declaration macro
    /// - `=` - Assignment macro
    /// - `fn` - Function definition macro
    /// - `match` - Pattern matching macro for booleans
    /// - `assert` - Assertion macro for runtime checks
    /// - `typeof` - Type query macro (returns type as string)
    /// - `measure` - Unit definition macro for dimensional analysis
    /// - `|>` - Pipeline operator macro
    /// - `__block__` - Block expression macro (automatically emitted by parser)
    /// - `__list__` - List literal macro (automatically emitted by parser)
    /// - `__record__` - Record literal macro (automatically emitted by parser)
    /// - `__index__` - Array indexing macro (automatically emitted by parser)
    /// - Arithmetic operators: `+`, `-`, `*`, `/`
    /// - Comparison operators: `==`, `!=`, `<`, `<=`, `>`, `>=`
    ///
    /// This can be called on an existing environment to add the standard built-ins.
    pub fn register_standard_builtins(&mut self) {
        // Macros
        let let_id: InternedString = "let".into();
        let assign_id: InternedString = "=".into();
        let fn_id: InternedString = "fn".into();
        let match_id: InternedString = "match".into();
        let assert_id: InternedString = "assert".into();
        let typeof_id: InternedString = "typeof".into();
        let measure_id: InternedString = "measure".into();
        let pipeline_id: InternedString = "|>".into();
        let block_id: InternedString = "__block__".into();
        let list_id: InternedString = "__list__".into();
        let record_id: InternedString = "__record__".into();
        let index_id: InternedString = "__index__".into();

        self.define(let_id, Value::SpecialForm(special_form::let_form::get()));
        self.define(
            assign_id,
            Value::SpecialForm(special_form::assign_form::get()),
        );
        self.define(fn_id, Value::SpecialForm(special_form::fn_form::get()));
        self.define(
            match_id,
            Value::SpecialForm(special_form::match_form::get()),
        );
        self.define(
            assert_id,
            Value::SpecialForm(special_form::assert_form::get()),
        );
        self.define(
            typeof_id,
            Value::SpecialForm(special_form::typeof_form::get()),
        );
        self.define(
            measure_id,
            Value::SpecialForm(special_form::measure_form::get()),
        );
        self.define(
            pipeline_id,
            Value::SpecialForm(special_form::pipeline_form::get()),
        );
        self.define(
            block_id,
            Value::SpecialForm(special_form::block_form::get()),
        );
        self.define(list_id, Value::SpecialForm(special_form::list_form::get()));
        self.define(
            record_id,
            Value::SpecialForm(special_form::record_form::get()),
        );
        self.define(
            index_id,
            Value::SpecialForm(special_form::index_form::get()),
        );

        // Arithmetic operators
        let add_id: InternedString = "+".into();
        let sub_id: InternedString = "-".into();
        let mul_id: InternedString = "*".into();
        let div_id: InternedString = "/".into();

        self.define(add_id, Value::SpecialForm(special_form::add_form::get()));
        self.define(sub_id, Value::SpecialForm(special_form::sub_form::get()));
        self.define(mul_id, Value::SpecialForm(special_form::mul_form::get()));
        self.define(div_id, Value::SpecialForm(special_form::div_form::get()));

        // Comparison operators
        let eq_id: InternedString = "==".into();
        let ne_id: InternedString = "!=".into();
        let lt_id: InternedString = "<".into();
        let lte_id: InternedString = "<=".into();
        let gt_id: InternedString = ">".into();
        let gte_id: InternedString = ">=".into();

        self.define(eq_id, Value::SpecialForm(special_form::eq_form::get()));
        self.define(ne_id, Value::SpecialForm(special_form::ne_form::get()));
        self.define(lt_id, Value::SpecialForm(special_form::lt_form::get()));
        self.define(lte_id, Value::SpecialForm(special_form::le_form::get()));
        self.define(gt_id, Value::SpecialForm(special_form::gt_form::get()));
        self.define(gte_id, Value::SpecialForm(special_form::ge_form::get()));

        // Boolean constants
        let true_id: InternedString = "true".into();
        let false_id: InternedString = "false".into();
        self.define(true_id, Value::Bool(true));
        self.define(false_id, Value::Bool(false));

        // Field access operator
        let dot_id: InternedString = ".".into();
        self.define(
            dot_id,
            Value::SpecialForm(special_form::field_access_form::get()),
        );
    }

    /// Pushes a new empty scope onto the stack.
    pub fn push_scope(&mut self) {
        Rc::make_mut(&mut self.scopes).push(Scope::new());
    }

    /// Pops the top scope from the stack.
    ///
    /// # Panics
    ///
    /// Panics if there is only one scope (the global scope).
    pub fn pop_scope(&mut self) {
        assert!(self.scopes.len() > 1, "Cannot pop the global scope");
        Rc::make_mut(&mut self.scopes).pop();
    }

    /// Defines a binding in the current (top) scope.
    pub fn define(&mut self, name: InternedString, value: Value) {
        if let Some(scope) = Rc::make_mut(&mut self.scopes).last_mut() {
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
        for scope in Rc::make_mut(&mut self.scopes).iter_mut().rev() {
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
        if let Some(scope) = Rc::make_mut(&mut self.scopes).first_mut() {
            scope.define(name, value);
        }
    }

    /// Iterates over all bindings in all scopes, from top to bottom.
    ///
    /// If a name is shadowed, only the innermost binding is yielded.
    /// This is useful for building a type environment from the current runtime environment.
    pub fn iter(&self) -> impl Iterator<Item = (InternedString, &Value)> {
        // Collect bindings from all scopes, top to bottom, skipping shadowed names
        let mut seen = HashSet::new();
        let mut bindings = Vec::new();

        for scope in self.scopes.iter().rev() {
            for (name, value) in scope.bindings.iter() {
                if seen.insert(*name) {
                    bindings.push((*name, value));
                }
            }
        }

        bindings.into_iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

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

        // Should have `let` and `=` macros registered
        let let_id: InternedString = "let".into();
        let assign_id: InternedString = "=".into();

        assert!(
            matches!(env.get(let_id), Some(Value::SpecialForm(_))),
            "Expected `let` to be registered as a SpecialForm"
        );
        assert!(
            matches!(env.get(assign_id), Some(Value::SpecialForm(_))),
            "Expected `=` to be registered as a SpecialForm"
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
            matches!(env.get(let_id), Some(Value::SpecialForm(_))),
            "Expected `let` to be registered as a SpecialForm"
        );
        assert!(
            matches!(env.get(assign_id), Some(Value::SpecialForm(_))),
            "Expected `=` to be registered as a SpecialForm"
        );
    }

    #[test]
    fn env_iter() {
        let mut env = Env::new();
        let x: InternedString = "x".into();
        let y: InternedString = "y".into();

        env.define(x, Value::Integer(1));
        env.define(y, Value::Integer(2));

        // Collect all bindings
        let bindings: HashMap<_, _> = env.iter().collect();
        assert_eq!(bindings.len(), 2);
        assert_eq!(bindings.get(&x), Some(&&Value::Integer(1)));
        assert_eq!(bindings.get(&y), Some(&&Value::Integer(2)));
    }

    #[test]
    fn env_iter_with_shadowing() {
        let mut env = Env::new();
        let x: InternedString = "x".into();

        env.define(x, Value::Integer(1));
        env.push_scope();
        env.define(x, Value::Integer(2));

        // Should only return the innermost binding
        let bindings: HashMap<_, _> = env.iter().collect();
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings.get(&x), Some(&&Value::Integer(2)));
    }
}
