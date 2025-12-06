//! Compiler API surface for the evaluator.
//!
//! The `Compiler` struct is the shared mutable state that accumulates
//! definitions during evaluation. Macros call back into the compiler
//! API to register definitions, emit IR, etc.

use crate::{
    diagnostic::Diagnostic,
    interner::InternedString,
    ir::IrGenerator,
    map::Map,
    typeinfer::TypeInferencer,
    unit::UnitRegistry,
    value::Value,
};

/// The compiler state that accumulates definitions during evaluation.
///
/// This is the explicit API the language uses to build the module.
/// All internal compiler tables use `Map` with `InternedString` keys and FxHash.
///
/// The compiler also collects diagnostics during evaluation, allowing for
/// multi-error reporting instead of bailing on the first error.
///
/// Additionally, the compiler includes a type inferencer for lazy type checking.
/// Type checking is not performed during evaluation by default, but can be
/// triggered on-demand for specific expressions or for LSP integration.
///
/// The compiler also includes an IR generator that converts evaluated functions
/// into a target-independent intermediate representation suitable for optimization
/// and code generation.
pub struct Compiler {
    /// Variable and function definitions.
    defs: Map<Value>,
    /// Macro definitions.
    macros: Map<Value>,
    /// Accumulated diagnostics (errors, warnings, hints).
    diagnostics: Vec<Diagnostic>,
    /// Unit registry for dimensional analysis.
    units: UnitRegistry,
    /// Type inferencer for lazy type checking.
    type_inferencer: TypeInferencer,
    /// IR generator for code generation.
    ir_generator: IrGenerator,
}

impl Default for Compiler {
    fn default() -> Self {
        Self::new()
    }
}

impl Compiler {
    /// Creates a new empty compiler state.
    pub fn new() -> Self {
        Self {
            defs: Map::default(),
            macros: Map::default(),
            diagnostics: Vec::new(),
            units: UnitRegistry::new(),
            type_inferencer: TypeInferencer::new(),
            ir_generator: IrGenerator::new(),
        }
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

    /// Returns a reference to the unit registry.
    pub fn units(&self) -> &UnitRegistry {
        &self.units
    }

    /// Returns a mutable reference to the unit registry.
    pub fn units_mut(&mut self) -> &mut UnitRegistry {
        &mut self.units
    }

    /// Returns a reference to the type inferencer.
    ///
    /// This allows lazy type checking - types can be inferred on-demand
    /// for specific expressions without blocking evaluation.
    pub fn type_inferencer(&self) -> &TypeInferencer {
        &self.type_inferencer
    }

    /// Returns a mutable reference to the type inferencer.
    ///
    /// This allows macros and other code to perform type inference
    /// for metaprogramming purposes.
    pub fn type_inferencer_mut(&mut self) -> &mut TypeInferencer {
        &mut self.type_inferencer
    }

    /// Generates IR for a user function.
    ///
    /// This should be called when a function is defined to generate its IR representation.
    /// Returns the function ID on success, or an error message on failure.
    ///
    /// # Errors
    ///
    /// Returns an error if IR generation fails (e.g., unsupported expressions).
    pub fn generate_ir_for_function(
        &mut self,
        func: &crate::value::UserFunction,
    ) -> Result<crate::ir::FunctionId, String> {
        self.ir_generator.gen_function(func)
    }

    /// Returns a reference to the IR generator.
    ///
    /// This allows direct access to the IR generator for advanced use cases.
    pub fn ir_generator(&self) -> &IrGenerator {
        &self.ir_generator
    }

    /// Returns a mutable reference to the IR generator.
    ///
    /// This allows direct manipulation of the IR generator.
    pub fn ir_generator_mut(&mut self) -> &mut IrGenerator {
        &mut self.ir_generator
    }

    /// Builds and returns the generated IR module.
    ///
    /// This consumes the IR generator and returns the final IR module.
    /// After calling this, the compiler will have a fresh IR generator.
    pub fn build_ir_module(&mut self) -> crate::ir::IrModule {
        let mut new_generator = IrGenerator::new();
        std::mem::swap(&mut self.ir_generator, &mut new_generator);
        new_generator.build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{diagnostic::DiagnosticLevel, value::Type};

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
            signature: crate::value::Type::function(vec![], crate::value::Type::Nil),
            func: |_args, _ctx| {
                // Return nil as a placeholder
                Ok(Value::Nil)
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

        compiler.record_diagnostic(*Diagnostic::undefined_variable(x_id));
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

        compiler.record_diagnostic(*Diagnostic::undefined_variable(x_id));
        assert_eq!(compiler.num_diagnostics(), 1);

        let taken = compiler.take_diagnostics();
        assert_eq!(taken.len(), 1);
        assert_eq!(compiler.num_diagnostics(), 0);
    }

    #[test]
    fn clear_diagnostics() {
        let x_id: InternedString = "x".into();
        let mut compiler = Compiler::new();

        compiler.record_diagnostic(*Diagnostic::undefined_variable(x_id));
        // Use union type to express "number" (integer | float)
        let number_type = Type::union(vec![Type::Integer, Type::Float]);
        compiler.record_diagnostic(*Diagnostic::type_error(number_type, Type::String));
        assert_eq!(compiler.num_diagnostics(), 2);

        compiler.clear_diagnostics();
        assert_eq!(compiler.num_diagnostics(), 0);
        assert!(!compiler.has_errors());
    }

    #[test]
    fn has_errors_distinguishes_levels() {
        use crate::diagnostic::BoxedDiagnosticExt;

        let x_id: InternedString = "x".into();
        let mut compiler = Compiler::new();

        // Add a warning - should not count as error
        let warning = Diagnostic::undefined_variable(x_id).set_level(DiagnosticLevel::Warning);
        compiler.record_diagnostic(*warning);
        assert!(!compiler.has_errors());

        // Add an error
        compiler.record_diagnostic(*Diagnostic::undefined_variable(x_id));
        assert!(compiler.has_errors());
    }
}
