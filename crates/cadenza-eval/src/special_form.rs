//! Special forms - built-in language constructs that provide evaluation and IR generation.
//!
//! Special forms are the base layer for interacting with the evaluator, type system, and IR builder.
//! Unlike macros (which are syntax tree to syntax tree transformations), special forms define
//! fundamental language constructs that have both evaluation semantics and IR generation logic.
//!
//! Each special form implements two key functions:
//! - `eval_fn`: Evaluates the form with unevaluated AST arguments, returns a runtime Value
//! - `ir_fn`: Generates IR instructions from the form's AST arguments
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

pub mod add_form;
pub mod assert_form;
pub mod assign_form;
pub mod block_form;
pub mod div_form;
pub mod eq_form;
pub mod field_access_form;
pub mod fn_form;
pub mod ge_form;
pub mod gt_form;
pub mod index_form;
pub mod le_form;
pub mod let_form;
pub mod list_form;
pub mod lt_form;
pub mod match_form;
pub mod measure_form;
pub mod mul_form;
pub mod ne_form;
pub mod pipeline_form;
pub mod record_form;
pub mod sub_form;
pub mod typeof_form;

use crate::{
    context::EvalContext,
    diagnostic::Result,
    ir::{BlockBuilder, IrGenContext, SourceLocation, ValueId},
    value::{Type, Value},
};
use cadenza_syntax::ast::Expr;

/// Type alias for special form evaluation functions.
pub type SpecialFormEvalFn = fn(&[Expr], &mut EvalContext<'_>) -> Result<Value>;

/// Type alias for special form IR generation functions.
pub type SpecialFormIrFn = fn(
    &[Expr],
    &mut BlockBuilder,
    &mut IrGenContext,
    SourceLocation,
    &mut dyn FnMut(&Expr, &mut BlockBuilder, &mut IrGenContext) -> Result<ValueId>,
) -> Result<ValueId>;

/// A special form implementation with function pointers.
///
/// This is the concrete type used for all built-in special forms.
/// It stores function pointers for both evaluation and IR generation.
pub struct BuiltinSpecialForm {
    /// The special form name for display/debugging.
    pub name: &'static str,
    /// The type signature of this special form.
    pub signature: Type,
    /// The evaluation function (receives unevaluated AST expressions).
    pub eval_fn: SpecialFormEvalFn,
    /// The IR generation function (generates IR from AST expressions).
    pub ir_fn: SpecialFormIrFn,
}

impl BuiltinSpecialForm {
    /// Returns the name of this special form for display/debugging.
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// Returns the type signature of this special form.
    pub fn signature(&self) -> Type {
        self.signature.clone()
    }

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
    pub fn eval(&self, args: &[Expr], ctx: &mut EvalContext<'_>) -> Result<Value> {
        (self.eval_fn)(args, ctx)
    }

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
    /// The ValueId of the result in the IR, or a diagnostic error.
    pub fn build_ir(
        &self,
        args: &[Expr],
        block: &mut BlockBuilder,
        ctx: &mut IrGenContext,
        source: SourceLocation,
        gen_expr: &mut dyn FnMut(&Expr, &mut BlockBuilder, &mut IrGenContext) -> Result<ValueId>,
    ) -> Result<ValueId> {
        (self.ir_fn)(args, block, ctx, source, gen_expr)
    }
}
