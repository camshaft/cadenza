//! Compiler API surface for the evaluator.
//!
//! The `Compiler` struct is the shared mutable state that accumulates
//! definitions during evaluation. Macros call back into the compiler
//! API to register definitions, emit IR, etc.

use crate::{diagnostic::Diagnostic, interner::InternedString, map::Map, value::Value};

/// The compiler state that accumulates definitions during evaluation.
///
/// This is the explicit API the language uses to build the module.
/// All internal compiler tables use `Map` with `InternedString` keys and FxHash.
///
/// The compiler also collects diagnostics during evaluation, allowing for
/// multi-error reporting instead of bailing on the first error.
#[derive(Debug, Default)]
pub struct Compiler {
    /// Variable and function definitions.
    defs: Map<Value>,
    /// Macro definitions.
    macros: Map<Value>,
    /// Accumulated diagnostics (errors, warnings, hints).
    diagnostics: Vec<Diagnostic>,
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

    /// Records a diagnostic (error, warning, or hint).
    ///
    /// This allows the evaluator to collect multiple diagnostics instead of
    /// bailing on the first error.
    pub fn record_diagnostic(&mut self, diagnostic: Diagnostic) {
        self.diagnostics.push(diagnostic);
    }

    /// Returns all accumulated diagnostics.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Takes ownership of all accumulated diagnostics, leaving the compiler's
    /// diagnostic list empty.
    pub fn take_diagnostics(&mut self) -> Vec<Diagnostic> {
        std::mem::take(&mut self.diagnostics)
    }

    /// Returns the number of accumulated diagnostics.
    pub fn num_diagnostics(&self) -> usize {
        self.diagnostics.len()
    }

    /// Returns true if any error-level diagnostics have been recorded.
    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(|d| d.is_error())
    }

    /// Clears all accumulated diagnostics.
    pub fn clear_diagnostics(&mut self) {
        self.diagnostics.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::DiagnosticLevel;

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

    #[test]
    fn record_and_get_diagnostics() {
        let x_id: InternedString = "x".into();
        let mut compiler = Compiler::new();

        assert_eq!(compiler.num_diagnostics(), 0);
        assert!(!compiler.has_errors());

        compiler.record_diagnostic(Diagnostic::undefined_variable(x_id));
        assert_eq!(compiler.num_diagnostics(), 1);
        assert!(compiler.has_errors());

        let diagnostics = compiler.diagnostics();
        assert_eq!(diagnostics.len(), 1);
        assert!(matches!(
            &diagnostics[0].kind,
            crate::diagnostic::DiagnosticKind::UndefinedVariable(_)
        ));
    }

    #[test]
    fn take_diagnostics_empties_list() {
        let x_id: InternedString = "x".into();
        let mut compiler = Compiler::new();

        compiler.record_diagnostic(Diagnostic::undefined_variable(x_id));
        assert_eq!(compiler.num_diagnostics(), 1);

        let taken = compiler.take_diagnostics();
        assert_eq!(taken.len(), 1);
        assert_eq!(compiler.num_diagnostics(), 0);
    }

    #[test]
    fn clear_diagnostics() {
        let x_id: InternedString = "x".into();
        let mut compiler = Compiler::new();

        compiler.record_diagnostic(Diagnostic::undefined_variable(x_id));
        compiler.record_diagnostic(Diagnostic::type_error("number", "string"));
        assert_eq!(compiler.num_diagnostics(), 2);

        compiler.clear_diagnostics();
        assert_eq!(compiler.num_diagnostics(), 0);
        assert!(!compiler.has_errors());
    }

    #[test]
    fn has_errors_distinguishes_levels() {
        let x_id: InternedString = "x".into();
        let mut compiler = Compiler::new();

        // Add a warning - should not count as error
        let warning = Diagnostic::undefined_variable(x_id).set_level(DiagnosticLevel::Warning);
        compiler.record_diagnostic(warning);
        assert!(!compiler.has_errors());

        // Add an error
        compiler.record_diagnostic(Diagnostic::undefined_variable(x_id));
        assert!(compiler.has_errors());
    }
}
