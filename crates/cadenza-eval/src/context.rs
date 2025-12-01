//! Evaluation context for the Cadenza evaluator.
//!
//! The `EvalContext` consolidates all the standard eval arguments (env, compiler)
//! into a single struct that is passed to evaluation functions and built-in functions/macros.
//! This makes it easy to add new fields in the future without changing function signatures.

use crate::{compiler::Compiler, diagnostic::Result, env::Env, value::Value};

/// The evaluation context containing all state needed during evaluation.
///
/// This struct consolidates:
/// - The scoped environment for variable bindings
/// - The compiler state that accumulates definitions
///
/// Future extensions may include:
/// - Stack trace maintenance
/// - Source file tracking
/// - Evaluation limits/quotas
pub struct EvalContext<'a> {
    /// The scoped environment for variable bindings.
    pub env: &'a mut Env,
    /// The compiler state that accumulates definitions.
    pub compiler: &'a mut Compiler,
}

impl<'a> EvalContext<'a> {
    /// Creates a new evaluation context with the given environment and compiler.
    pub fn new(env: &'a mut Env, compiler: &'a mut Compiler) -> Self {
        Self { env, compiler }
    }

    /// Creates a borrowed evaluation context from the current context.
    ///
    /// This is useful for passing the context to nested evaluations while
    /// maintaining the lifetime constraints.
    pub fn reborrow(&mut self) -> EvalContext<'_> {
        EvalContext {
            env: self.env,
            compiler: self.compiler,
        }
    }
}

/// A trait for types that can be evaluated to produce a [`Value`].
///
/// This trait provides a unified interface for evaluation across different
/// expression types. Types implementing this trait can be evaluated using
/// an [`EvalContext`] that provides access to the environment and compiler.
///
/// # Example
///
/// ```ignore
/// use cadenza_eval::{Eval, EvalContext, Env, Compiler, Value};
///
/// // Expressions implement Eval
/// let expr: Expr = /* ... */;
/// let mut env = Env::new();
/// let mut compiler = Compiler::new();
/// let mut ctx = EvalContext::new(&mut env, &mut compiler);
/// let result = expr.eval(&mut ctx);
/// ```
pub trait Eval {
    /// Evaluates this expression using the given context.
    ///
    /// Returns `Ok(Value)` on success, or a boxed `Diagnostic` on error.
    fn eval(&self, ctx: &mut EvalContext<'_>) -> Result<Value>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_reborrow() {
        let mut env = Env::new();
        let mut compiler = Compiler::new();
        let mut ctx = EvalContext::new(&mut env, &mut compiler);

        // Test that reborrow works
        {
            let ctx2 = ctx.reborrow();
            ctx2.env.push_scope();
            ctx2.env.pop_scope();
        }

        // Original context is still usable
        assert_eq!(ctx.env.depth(), 1);
    }
}
