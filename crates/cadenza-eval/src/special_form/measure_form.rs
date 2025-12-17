//! The `measure` special form for defining units and conversions.

use crate::{
    Eval,
    context::EvalContext,
    diagnostic::{Diagnostic, Result},
    interner::InternedString,
    ir::{BlockBuilder, IrGenContext, SourceLocation, ValueId},
    special_form::BuiltinSpecialForm,
    unit::Unit,
    value::{Type, Value},
};
use cadenza_syntax::ast::Expr;
use std::sync::OnceLock;

/// Returns the `measure` special form for defining units and conversions.
///
/// The `measure` special form defines a unit for dimensional analysis.
///
/// # Evaluation
/// - Takes 1 or 2 arguments:
///   1. Base unit definition: `measure meter` - creates a new base unit
///   2. Derived unit: `measure inch = millimeter 25.4` - creates unit with conversion
/// - Registers the unit in the compiler's unit registry
/// - Returns Nil
///
/// # IR Generation
/// - Not yet implemented (returns error)
///
/// # Examples
/// ```cadenza
/// measure meter                      // Base unit
/// measure inch = millimeter 25.4     // 1 inch = 25.4 millimeters
/// measure foot = inch 12             // 1 foot = 12 inches
/// ```
pub fn get() -> &'static BuiltinSpecialForm {
    static MEASURE_FORM: OnceLock<BuiltinSpecialForm> = OnceLock::new();
    MEASURE_FORM.get_or_init(|| BuiltinSpecialForm {
        name: "measure",
        signature: Type::function(vec![Type::Symbol], Type::Nil),
        eval_fn: eval_measure,
        ir_fn: ir_measure,
    })
}

fn eval_measure(args: &[Expr], ctx: &mut EvalContext<'_>) -> Result<Value> {
    if args.is_empty() {
        return Err(Diagnostic::syntax("measure requires at least one argument"));
    }

    // Case 1: Base unit definition - measure name
    if args.len() == 1 {
        match &args[0] {
            Expr::Ident(ident) => {
                let text = ident.syntax().text();
                let name: InternedString = text.to_string().as_str().into();
                let unit = Unit::base(name);
                ctx.compiler.units_mut().register(unit);
                return Ok(Value::Nil);
            }
            _ => {
                return Err(Diagnostic::syntax(
                    "measure requires an identifier as unit name",
                ));
            }
        }
    }

    // Case 2: Derived unit definition - measure name = base scale
    // When called from `=`, we receive [name, rhs] where rhs is `base scale`
    if args.len() == 2 {
        // Get the unit name
        let name = match &args[0] {
            Expr::Ident(ident) => {
                let text = ident.syntax().text();
                let name: InternedString = text.to_string().as_str().into();
                name
            }
            _ => {
                return Err(Diagnostic::syntax(
                    "measure unit name must be an identifier",
                ));
            }
        };

        // The RHS should be: base scale (an Apply node)
        let rhs_expr = &args[1];
        match rhs_expr {
            Expr::Apply(rhs_apply) => {
                let base_expr = rhs_apply.callee();
                let scale_args: Vec<Expr> = rhs_apply
                    .arguments()
                    .filter_map(|arg| arg.value())
                    .collect();

                if scale_args.len() != 1 {
                    return Err(Diagnostic::syntax(
                        "measure conversion requires: base scale (e.g., millimeter 25.4)",
                    ));
                }

                // Get base unit name
                let base_unit_name = match base_expr {
                    Some(Expr::Ident(ident)) => {
                        let text = ident.syntax().text();
                        let name: InternedString = text.to_string().as_str().into();
                        name
                    }
                    _ => {
                        return Err(Diagnostic::syntax(
                            "measure conversion base must be a unit name",
                        ));
                    }
                };

                // Get scale
                let scale_value = ctx.eval_child(&scale_args[0])?;
                let scale = match scale_value {
                    Value::Integer(n) => n as f64,
                    Value::Float(f) => f,
                    _ => {
                        return Err(Diagnostic::syntax(
                            "measure conversion scale must be a number",
                        ));
                    }
                };

                // Look up the base unit
                let base_unit = ctx.compiler.units().get(base_unit_name).ok_or_else(|| {
                    Diagnostic::syntax(format!("undefined unit '{}'", &*base_unit_name))
                })?;

                // Create derived unit: 1 new_unit = scale base_units
                let derived_unit = Unit::derived(name, base_unit.dimension, scale, 0.0);

                ctx.compiler.units_mut().register(derived_unit);
                Ok(Value::Nil)
            }
            _ => Err(Diagnostic::syntax(
                "measure conversion requires: base scale (e.g., millimeter 25.4)",
            )),
        }
    } else {
        Err(Diagnostic::syntax(
            "measure expects 1 or 2 arguments (e.g., measure meter, or measure inch = millimeter 25.4)",
        ))
    }
}

fn ir_measure(
    _args: &[Expr],
    _block: &mut BlockBuilder,
    _ctx: &mut IrGenContext,
    _source: SourceLocation,
    _gen_expr: &mut dyn FnMut(&Expr, &mut BlockBuilder, &mut IrGenContext) -> Result<ValueId>,
) -> Result<ValueId> {
    Err(Diagnostic::syntax(
        "measure special form IR generation not yet implemented",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Compiler, Env};
    use cadenza_syntax::parse::parse;

    #[test]
    fn test_measure_base_unit() {
        let mut env = Env::with_standard_builtins();
        let mut compiler = Compiler::new();

        let input = "measure meter";
        let parsed = parse(input);
        let root = parsed.ast();

        let results = crate::eval(&root, &mut env, &mut compiler);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0], Value::Nil);

        // Verify the unit was registered
        let meter: InternedString = "meter".into();
        assert!(compiler.units().get(meter).is_some());
    }

    #[test]
    fn test_measure_derived_unit() {
        let mut env = Env::with_standard_builtins();
        let mut compiler = Compiler::new();

        let input = r#"
measure meter
measure inch = meter 0.0254
"#;
        let parsed = parse(input);
        let root = parsed.ast();

        let results = crate::eval(&root, &mut env, &mut compiler);

        assert_eq!(results.len(), 2);

        // Verify both units were registered
        let meter: InternedString = "meter".into();
        let inch: InternedString = "inch".into();
        assert!(compiler.units().get(meter).is_some());
        assert!(compiler.units().get(inch).is_some());
    }
}
