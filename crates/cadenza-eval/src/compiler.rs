//! Compiler API surface for the evaluator.
//!
//! The `Compiler` struct is the shared mutable state that accumulates
//! definitions during evaluation. Macros call back into the compiler
//! API to register definitions, emit IR, etc.

use crate::{interner::InternedString, map::Map, value::Value};

/// The compiler state that accumulates definitions during evaluation.
///
/// This is the explicit API the language uses to build the module.
/// All internal compiler tables use `Map` with `InternedString` keys and FxHash.
#[derive(Debug, Default)]
pub struct Compiler {
    /// Variable and function definitions.
    defs: Map<Value>,
    /// Macro definitions.
    macros: Map<Value>,
}

impl Compiler {
    /// Creates a new empty compiler state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Defines a variable or function.
    pub fn define_var(&mut self, name: InternedString, value: Value) {
        self.defs.insert(name, value);
    }

    /// Defines a macro.
    ///
    /// The value must be a `Value::Macro` or `Value::BuiltinMacro`.
    pub fn define_macro(&mut self, name: InternedString, expander: Value) {
        self.macros.insert(name, expander);
    }

    /// Looks up a variable or function definition.
    pub fn get_var(&self, name: InternedString) -> Option<&Value> {
        self.defs.get(&name)
    }

    /// Looks up a macro definition.
    pub fn get_macro(&self, name: InternedString) -> Option<&Value> {
        self.macros.get(&name)
    }

    /// Returns all variable/function definitions.
    pub fn defs(&self) -> &Map<Value> {
        &self.defs
    }

    /// Returns all macro definitions.
    pub fn macros(&self) -> &Map<Value> {
        &self.macros
    }

    /// Returns the number of variable/function definitions.
    pub fn num_defs(&self) -> usize {
        self.defs.len()
    }

    /// Returns the number of macro definitions.
    pub fn num_macros(&self) -> usize {
        self.macros.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn define_and_get_var() {
        let name: InternedString = "x".into();
        let mut compiler = Compiler::new();

        compiler.define_var(name, Value::Integer(42));
        assert_eq!(compiler.get_var(name), Some(&Value::Integer(42)));
    }

    #[test]
    fn define_and_get_macro() {
        let name: InternedString = "my_macro".into();
        let mut compiler = Compiler::new();

        // Using a builtin macro as a placeholder
        let macro_value = Value::BuiltinMacro(crate::value::BuiltinMacro {
            name: "my_macro",
            func: |_| {
                // Return the green node for a simple identifier "x"
                let parsed = cadenza_syntax::parse::parse("x");
                Ok(parsed.green)
            },
        });

        compiler.define_macro(name, macro_value);
        assert!(compiler.get_macro(name).is_some());
    }

    #[test]
    fn num_defs_is_tracked() {
        let mut compiler = Compiler::new();

        assert_eq!(compiler.num_defs(), 0);

        let a: InternedString = "a".into();
        let b: InternedString = "b".into();
        compiler.define_var(a, Value::Integer(1));
        assert_eq!(compiler.num_defs(), 1);

        compiler.define_var(b, Value::Integer(2));
        assert_eq!(compiler.num_defs(), 2);
    }
}
