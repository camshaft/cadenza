//! Compiler API surface for the evaluator.
//!
//! The `Compiler` struct is the shared mutable state that accumulates
//! definitions during evaluation. Macros call back into the compiler
//! API to register definitions, emit IR, etc.

use crate::interner::InternedId;
use crate::value::Value;
use std::collections::HashMap;

/// The compiler state that accumulates definitions during evaluation.
///
/// This is the explicit API the language uses to build the module.
/// All internal compiler tables use `HashMap` with `InternedId` keys.
#[derive(Debug, Default)]
pub struct Compiler {
    /// Variable and function definitions.
    defs: HashMap<InternedId, Value>,
    /// Macro definitions.
    macros: HashMap<InternedId, Value>,
}

impl Compiler {
    /// Creates a new empty compiler state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Defines a variable or function.
    pub fn define_var(&mut self, name: InternedId, value: Value) {
        self.defs.insert(name, value);
    }

    /// Defines a macro.
    ///
    /// The value must be a `Value::Macro` or `Value::BuiltinMacro`.
    pub fn define_macro(&mut self, name: InternedId, expander: Value) {
        self.macros.insert(name, expander);
    }

    /// Looks up a variable or function definition.
    pub fn get_var(&self, name: InternedId) -> Option<&Value> {
        self.defs.get(&name)
    }

    /// Looks up a macro definition.
    pub fn get_macro(&self, name: InternedId) -> Option<&Value> {
        self.macros.get(&name)
    }

    /// Returns all variable/function definitions.
    pub fn defs(&self) -> &HashMap<InternedId, Value> {
        &self.defs
    }

    /// Returns all macro definitions.
    pub fn macros(&self) -> &HashMap<InternedId, Value> {
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
    use crate::interner::Interner;

    #[test]
    fn define_and_get_var() {
        let mut interner = Interner::new();
        let name = interner.intern("x");
        let mut compiler = Compiler::new();

        compiler.define_var(name, Value::Integer(42));
        assert_eq!(compiler.get_var(name), Some(&Value::Integer(42)));
    }

    #[test]
    fn define_and_get_macro() {
        let mut interner = Interner::new();
        let name = interner.intern("my_macro");
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
        let mut interner = Interner::new();
        let mut compiler = Compiler::new();

        assert_eq!(compiler.num_defs(), 0);

        compiler.define_var(interner.intern("a"), Value::Integer(1));
        assert_eq!(compiler.num_defs(), 1);

        compiler.define_var(interner.intern("b"), Value::Integer(2));
        assert_eq!(compiler.num_defs(), 2);
    }
}
