//! Special forms - built-in language constructs that provide evaluation and IR generation.
//!
//! Special forms are the base layer for interacting with the evaluator, type system, and IR builder.
//! Unlike macros (which are syntax tree to syntax tree transformations), special forms define
//! fundamental language constructs that have both evaluation semantics and IR generation logic.
//!
//! Each special form implements two key functions:
//! - `eval`: Evaluates the form with unevaluated AST arguments, returns a runtime Value
//! - `build_ir`: Generates IR instructions from the form's AST arguments
//!
//! Examples of special forms include: `let`, `fn`, `if`, `__block__`, etc.
//!
//! ## Integration Pattern
//!
//! Currently, special forms must be manually integrated in two places:
//! 1. **Environment Registration**: Register the special form in `Env::register_standard_builtins()`
//!    so it can be looked up and called during evaluation.
//! 2. **IR Generator Integration**: Add explicit handling in `IrGenerator::gen_apply()` to detect
//!    the special form by name and call its IR generation logic.
//!
//! This dual registration is temporary and follows the existing pattern for built-in macros like
//! `__block__` and `__list__`. Future refactoring could introduce a special form registry that
//! both the environment and IR generator use.

use crate::{
    context::EvalContext,
    diagnostic::Result,
    ir::{BlockBuilder, IrGenContext, SourceLocation, ValueId},
    value::{Type, Value},
};
use cadenza_syntax::ast::Expr;

/// A special form that provides both evaluation and IR generation.
///
/// Special forms are built-in language constructs that:
/// - Receive unevaluated AST expressions (unlike regular functions)
/// - Provide custom evaluation semantics
/// - Provide custom IR generation logic
///
/// This replaces the previous `BuiltinMacro` concept, which only handled evaluation.
pub trait SpecialForm: Send + Sync {
    /// Returns the name of this special form for display/debugging.
    fn name(&self) -> &'static str;

    /// Returns the type signature of this special form.
    fn signature(&self) -> Type;

    /// Evaluates this special form with unevaluated AST arguments.
    ///
    /// The special form receives the raw AST expressions and can choose which
    /// to evaluate and when. This is what makes special forms "special" - they
    /// control evaluation rather than receiving pre-evaluated arguments.
    ///
    /// # Arguments
    /// - `args`: Unevaluated AST expressions passed to the special form
    /// - `ctx`: Evaluation context providing access to environment and compiler
    ///
    /// # Returns
    /// The evaluated result as a runtime Value, or a diagnostic error.
    fn eval(&self, args: &[Expr], ctx: &mut EvalContext<'_>) -> Result<Value>;

    /// Generates IR instructions for this special form.
    ///
    /// The special form receives the raw AST expressions and generates appropriate
    /// IR instructions. It can generate IR for sub-expressions by calling the
    /// provided gen_expr function.
    ///
    /// # Arguments
    /// - `args`: Unevaluated AST expressions passed to the special form
    /// - `block`: IR block builder for emitting instructions
    /// - `ctx`: IR generation context for variable bindings and types
    /// - `source`: Source location information for debugging
    /// - `gen_expr`: Function to generate IR for a sub-expression
    ///
    /// # Returns
    /// The ValueId of the result in the IR, or an error message.
    fn build_ir(
        &self,
        args: &[Expr],
        block: &mut BlockBuilder,
        ctx: &mut IrGenContext,
        source: SourceLocation,
        gen_expr: &mut dyn FnMut(&Expr, &mut BlockBuilder, &mut IrGenContext) -> std::result::Result<ValueId, String>,
    ) -> std::result::Result<ValueId, String>;
}

/// A special form implementation with function pointers.
///
/// This is the concrete type used for most built-in special forms.
/// It stores function pointers for both evaluation and IR generation.
pub struct BuiltinSpecialForm {
    /// The special form name for display/debugging.
    pub name: &'static str,
    /// The type signature of this special form.
    pub signature: Type,
    /// The evaluation function (receives unevaluated AST expressions).
    pub eval_fn: fn(&[Expr], &mut EvalContext<'_>) -> Result<Value>,
    /// The IR generation function (generates IR from AST expressions).
    pub ir_fn: fn(
        &[Expr],
        &mut BlockBuilder,
        &mut IrGenContext,
        SourceLocation,
        &mut dyn FnMut(&Expr, &mut BlockBuilder, &mut IrGenContext) -> std::result::Result<ValueId, String>,
    ) -> std::result::Result<ValueId, String>,
}

impl SpecialForm for BuiltinSpecialForm {
    fn name(&self) -> &'static str {
        self.name
    }

    fn signature(&self) -> Type {
        self.signature.clone()
    }

    fn eval(&self, args: &[Expr], ctx: &mut EvalContext<'_>) -> Result<Value> {
        (self.eval_fn)(args, ctx)
    }

    fn build_ir(
        &self,
        args: &[Expr],
        block: &mut BlockBuilder,
        ctx: &mut IrGenContext,
        source: SourceLocation,
        gen_expr: &mut dyn FnMut(&Expr, &mut BlockBuilder, &mut IrGenContext) -> std::result::Result<ValueId, String>,
    ) -> std::result::Result<ValueId, String> {
        (self.ir_fn)(args, block, ctx, source, gen_expr)
    }
}

impl Clone for BuiltinSpecialForm {
    fn clone(&self) -> Self {
        Self {
            name: self.name,
            signature: self.signature.clone(),
            eval_fn: self.eval_fn,
            ir_fn: self.ir_fn,
        }
    }
}
