//! The `struct` special form for defining nominally-typed structs.

use crate::{
    Eval,
    context::EvalContext,
    diagnostic::{Diagnostic, Result},
    interner::InternedString,
    ir::{BlockBuilder, IrGenContext, SourceLocation, ValueId},
    special_form::BuiltinSpecialForm,
    value::{Type, Value},
};
use cadenza_syntax::ast::Expr;
use std::sync::OnceLock;

/// Returns the `struct` special form for defining nominally-typed struct types.
///
/// The `struct` special form defines a new struct type with named fields and their types,
/// and creates a constructor function that can be used to create instances of the struct.
///
/// # Evaluation
/// - Takes 2 arguments: struct name (identifier) and field definitions (record expression)
/// - Field definitions are a record where each field maps to its type
/// - Creates a struct type definition
/// - Registers a constructor function with the struct name in the environment
/// - Returns the struct type as a Type value
///
/// # IR Generation
/// - Not yet implemented (returns error)
///
/// # Examples
/// ```cadenza
/// struct Point {
///   x = Integer,
///   y = Integer,
/// }
///
/// let p = Point { x = 1, y = 2 }
/// assert p.x == 1
/// ```
///
/// # Nominal Typing
/// Structs provide nominal typing: two structs with the same field structure but different
/// names are considered different types. This is unlike records which use structural typing.
///
/// ```cadenza
/// struct Point { x = Integer, y = Integer }
/// struct Vector { x = Integer, y = Integer }
///
/// let p = Point { x = 1, y = 2 }
/// let v = Vector { x = 1, y = 2 }
/// # p and v are NOT compatible despite having the same structure
/// ```
pub fn get() -> &'static BuiltinSpecialForm {
    static STRUCT_FORM: OnceLock<BuiltinSpecialForm> = OnceLock::new();
    STRUCT_FORM.get_or_init(|| BuiltinSpecialForm {
        name: "struct",
        // Type signature: struct Name { fields }
        // First arg is Symbol (representing an identifier for the struct name)
        // Second arg is Record (representing the field definitions)
        // Returns Type (the struct type definition)
        signature: Type::function(vec![Type::Symbol, Type::Record(vec![])], Type::Type),
        eval_fn: eval_struct,
        ir_fn: ir_struct,
    })
}

fn eval_struct(args: &[Expr], ctx: &mut EvalContext<'_>) -> Result<Value> {
    // Expect exactly 2 arguments: struct name and field definitions
    if args.len() != 2 {
        return Err(Diagnostic::syntax(
            "struct expects 2 arguments: name and field definitions (e.g., struct Point { x = Integer, y = Integer })",
        ));
    }

    // First argument is the struct name (identifier)
    let struct_name = match &args[0] {
        Expr::Ident(i) => {
            let text = i.syntax().text();
            InternedString::new(&text.to_string())
        }
        _ => {
            return Err(Diagnostic::syntax("struct name must be an identifier"));
        }
    };

    // Second argument is the field definitions (should be a record expression)
    // Each field in the record maps field name -> Type value
    let field_defs_expr = &args[1];
    let field_defs_value = field_defs_expr.eval(ctx)?;

    // Extract field definitions from the record
    let field_types = match field_defs_value {
        Value::Record {
            type_name: _,
            fields,
        } => {
            let mut field_types = Vec::with_capacity(fields.len());
            for (field_name, field_type_value) in fields {
                // Each field value must be a Type
                match field_type_value {
                    Value::Type(ty) => {
                        field_types.push((field_name, ty));
                    }
                    _ => {
                        return Err(Diagnostic::syntax(format!(
                            "field '{}' must have a type value, got {}",
                            &*field_name,
                            field_type_value.type_of()
                        )));
                    }
                }
            }
            field_types
        }
        _ => {
            return Err(Diagnostic::syntax(
                "struct field definitions must be a record (e.g., { x = Integer, y = Integer })",
            ));
        }
    };

    // Create the struct type
    let struct_type = Type::Struct {
        name: struct_name,
        fields: field_types.clone(),
    };

    // Create a constructor function for this struct
    // The constructor is a Value::StructConstructor that can be called to create instances
    let constructor = Value::StructConstructor {
        name: struct_name,
        field_types,
    };

    // Register the constructor in the environment with the struct's name
    ctx.env.define(struct_name, constructor);

    // Return the struct type as a Type value
    Ok(Value::Type(struct_type))
}

fn ir_struct(
    _args: &[Expr],
    _block: &mut BlockBuilder,
    _ctx: &mut IrGenContext,
    _source: SourceLocation,
    _gen_expr: &mut dyn FnMut(&Expr, &mut BlockBuilder, &mut IrGenContext) -> Result<ValueId>,
) -> Result<ValueId> {
    Err(Diagnostic::syntax(
        "struct special form IR generation not yet implemented",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Compiler, Env};
    use cadenza_syntax::parse::parse;

    #[test]
    fn test_struct_definition() {
        let mut env = Env::with_standard_builtins();
        let mut compiler = Compiler::new();

        let input = r#"
struct Point {
  x = Integer,
  y = Integer,
}
"#;
        let parsed = parse(input);
        let root = parsed.ast();

        let results = crate::eval(&root, &mut env, &mut compiler);

        // Check for compiler diagnostics
        let diagnostics = compiler.diagnostics();
        if !diagnostics.is_empty() {
            eprintln!("Compiler diagnostics:");
            for diag in diagnostics {
                eprintln!("  {:?}", diag);
            }
        }

        assert_eq!(results.len(), 1, "Expected 1 result, got {}", results.len());
        match &results[0] {
            Value::Type(Type::Struct { name, fields }) => {
                assert_eq!(&**name, "Point");
                assert_eq!(fields.len(), 2);
                assert_eq!(&*fields[0].0, "x");
                assert_eq!(fields[0].1, Type::Integer);
                assert_eq!(&*fields[1].0, "y");
                assert_eq!(fields[1].1, Type::Integer);
            }
            _ => panic!("Expected struct type, got {:?}", results[0]),
        }

        // Check that the constructor was registered
        let constructor = env.get(InternedString::new("Point")).unwrap();
        match constructor {
            Value::StructConstructor { name, field_types } => {
                assert_eq!(&**name, "Point");
                assert_eq!(field_types.len(), 2);
            }
            _ => panic!("Expected struct constructor in environment"),
        }
    }

    #[test]
    fn test_struct_instantiation() {
        let mut env = Env::with_standard_builtins();
        let mut compiler = Compiler::new();

        let input = r#"
struct Point {
  x = Integer,
  y = Integer,
}

let p = Point { x = 1, y = 2 }
p
"#;
        let parsed = parse(input);
        let root = parsed.ast();

        let results = crate::eval(&root, &mut env, &mut compiler);

        // Should have 3 results: struct definition, let binding, and final value
        assert_eq!(results.len(), 3);

        // Check the final result is a struct instance
        match &results[2] {
            Value::Record {
                type_name: Some(name),
                fields,
            } => {
                assert_eq!(&**name, "Point");
                assert_eq!(fields.len(), 2);
                assert_eq!(&*fields[0].0, "x");
                assert_eq!(fields[0].1, Value::Integer(1));
                assert_eq!(&*fields[1].0, "y");
                assert_eq!(fields[1].1, Value::Integer(2));
            }
            _ => panic!("Expected struct instance, got {:?}", results[2]),
        }
    }

    #[test]
    fn test_struct_field_access() {
        let mut env = Env::with_standard_builtins();
        let mut compiler = Compiler::new();

        let input = r#"
struct Point {
  x = Integer,
  y = Integer,
}

let p = Point { x = 10, y = 20 }
p.x
"#;
        let parsed = parse(input);
        let root = parsed.ast();

        let results = crate::eval(&root, &mut env, &mut compiler);

        // Should return the x field value
        assert_eq!(results.len(), 3);
        assert_eq!(results[2], Value::Integer(10));
    }
}
