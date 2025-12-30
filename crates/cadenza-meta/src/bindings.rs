//! Binding-based intermediate representation for code generation.
//!
//! Adapts ISLE's approach: patterns are converted to bindings (values) and
//! constraints (tests), then compiled into decision trees.

use crate::types::*;
use std::collections::HashMap;

/// A binding represents anything that can be bound to a variable in Rust
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Binding {
    /// Input argument to the query
    Input,

    /// Constant value
    Constant(Value),

    /// Variable captured from pattern
    Captured(String),

    /// Field extracted from a parent binding
    Extract {
        source: BindingId,
        kind: ExtractKind,
    },

    /// Call to another query
    QueryCall { query: String, args: Vec<BindingId> },

    /// Constructor call
    Construct {
        constructor: String,
        fields: Vec<BindingId>,
    },

    /// Let-bound expression
    LetBound {
        name: String,
        value: Box<CompiledExpr>,
    },
}

/// How to extract a value from a parent binding
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ExtractKind {
    /// Apply.callee
    ApplyCallee,
    /// Apply.args[i]
    ApplyArg(usize),
    /// Apply.args (the whole vec)
    ApplyArgs,
    /// Tuple[i]
    TupleField(usize),
    /// Record.field
    RecordField(String),
    /// Struct.field
    StructField(String),
}

/// A constraint tests whether a binding matches a pattern
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Constraint {
    /// Must be this literal integer
    ConstInt(i128),

    /// Must be this literal bool
    ConstBool(bool),

    /// Must be this literal string
    ConstString(String),

    /// Must be Apply with this structure
    IsApply,

    /// Must be Symbol with this name
    IsSymbol(String),

    /// Must be Integer
    IsInteger,

    /// Must be this tuple size
    IsTuple(usize),

    /// Must be this record structure
    IsRecord { fields: Vec<String> },

    /// Must be this struct
    IsStruct { name: String },

    /// Args vec must be this length
    ArgsLength(usize),
}

/// Unique ID for a binding
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BindingId(pub usize);

/// A compiled rule in binding-based form
#[derive(Clone, Debug)]
pub struct CompiledRule {
    /// The original pattern and result
    pub original: Rule,

    /// All bindings created for this rule
    pub bindings: Vec<Binding>,

    /// Constraints that must be satisfied
    pub constraints: Vec<(BindingId, Constraint)>,

    /// The result expression
    pub result: CompiledExpr,
}

/// A compiled expression (right-hand side)
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum CompiledExpr {
    /// Reference a binding
    Binding(BindingId),

    /// Constant
    Const(Value),

    /// Query call
    Call {
        query: String,
        args: Vec<CompiledExpr>,
    },

    /// Constructor
    Construct {
        constructor: String,
        fields: Vec<CompiledExpr>,
    },

    /// Let expression
    Let {
        bindings: Vec<(String, Box<CompiledExpr>)>,
        body: Box<CompiledExpr>,
    },

    /// Ok wrapper
    Ok(Box<CompiledExpr>),
}

/// Compile a pattern into bindings and constraints
/// Returns (bindings, constraints, var_env) where var_env maps variable names to BindingIds
/// The returned bindings should be appended after the Input binding (which is assumed to be at index 0)
pub fn compile_pattern(
    pattern: &Pattern,
    source: BindingId,
) -> (
    Vec<Binding>,
    Vec<(BindingId, Constraint)>,
    HashMap<String, BindingId>,
) {
    // Start with Input at index 0
    let mut bindings = vec![Binding::Input];
    let mut constraints = Vec::new();
    let mut var_env = HashMap::new();

    compile_pattern_helper(
        pattern,
        source,
        &mut bindings,
        &mut constraints,
        &mut var_env,
    );

    // Return bindings WITHOUT the Input (caller will add it)
    let pattern_bindings = bindings.into_iter().skip(1).collect();

    (pattern_bindings, constraints, var_env)
}

fn compile_pattern_helper(
    pattern: &Pattern,
    source: BindingId,
    bindings: &mut Vec<Binding>,
    constraints: &mut Vec<(BindingId, Constraint)>,
    var_env: &mut HashMap<String, BindingId>,
) {
    match pattern {
        Pattern::Wildcard | Pattern::Any => {
            // No constraints
        }

        Pattern::Capture(name) => {
            // If source is already a binding (not Input), just map the name to it
            // Don't create a new Captured binding
            var_env.insert(name.clone(), source);
        }

        Pattern::Integer(inner) => {
            constraints.push((source, Constraint::IsInteger));
            match &**inner {
                Pattern::Capture(name) => {
                    let id = BindingId(bindings.len());
                    bindings.push(Binding::Captured(name.clone()));
                    var_env.insert(name.clone(), id);
                }
                Pattern::Wildcard => {}
                _ => {
                    // Nested pattern - not yet supported
                }
            }
        }

        Pattern::SymbolLit(name) => {
            constraints.push((source, Constraint::IsSymbol(name.clone())));
        }

        Pattern::Apply { callee, args } => {
            constraints.push((source, Constraint::IsApply));

            // Extract callee
            let callee_id = BindingId(bindings.len());
            bindings.push(Binding::Extract {
                source,
                kind: ExtractKind::ApplyCallee,
            });
            compile_pattern_helper(callee, callee_id, bindings, constraints, var_env);

            // Constrain args length
            if !args.is_empty() {
                constraints.push((source, Constraint::ArgsLength(args.len())));
            }

            // Extract each arg
            for (i, arg) in args.iter().enumerate() {
                let arg_id = BindingId(bindings.len());
                bindings.push(Binding::Extract {
                    source,
                    kind: ExtractKind::ApplyArg(i),
                });
                compile_pattern_helper(arg, arg_id, bindings, constraints, var_env);
            }
        }

        Pattern::Tuple(patterns) => {
            constraints.push((source, Constraint::IsTuple(patterns.len())));

            // Extract each tuple element
            for (i, pattern) in patterns.iter().enumerate() {
                let field_id = BindingId(bindings.len());
                bindings.push(Binding::Extract {
                    source,
                    kind: ExtractKind::TupleField(i),
                });
                compile_pattern_helper(pattern, field_id, bindings, constraints, var_env);
            }
        }

        Pattern::Bool(inner) => {
            // For boolean patterns, we need to check if it's a specific bool value or just match any bool
            // For now, just extract the boolean value
            match &**inner {
                Pattern::Capture(name) => {
                    var_env.insert(name.clone(), source);
                }
                Pattern::Wildcard => {}
                _ => {
                    // Nested pattern - not yet supported
                }
            }
        }

        Pattern::String(inner) => {
            // Similar to Bool - for now just handle Capture and Wildcard
            match &**inner {
                Pattern::Capture(name) => {
                    var_env.insert(name.clone(), source);
                }
                Pattern::Wildcard => {}
                _ => {
                    // Nested pattern - not yet supported
                }
            }
        }

        Pattern::Float(inner) => {
            // Similar to Integer
            match &**inner {
                Pattern::Capture(name) => {
                    var_env.insert(name.clone(), source);
                }
                Pattern::Wildcard => {}
                _ => {
                    // Nested pattern - not yet supported
                }
            }
        }

        Pattern::Symbol(inner) => {
            // Symbol with inner pattern
            match &**inner {
                Pattern::Capture(name) => {
                    var_env.insert(name.clone(), source);
                }
                Pattern::Wildcard => {}
                _ => {
                    // Nested pattern - not yet supported
                }
            }
        }

        Pattern::Value(value) => {
            // Match a specific literal value
            match value {
                Value::Integer(i) => {
                    constraints.push((source, Constraint::ConstInt(*i)));
                }
                Value::Bool(b) => {
                    constraints.push((source, Constraint::ConstBool(*b)));
                }
                Value::String(s) => {
                    constraints.push((source, Constraint::ConstString(s.clone())));
                }
                _ => {
                    // TODO: Other literal types
                }
            }
        }

        _ => {
            // TODO: Implement remaining patterns (Record, Struct, Enum, Function, etc.)
        }
    }
}

/// Compile an expression into a compiled form
pub fn compile_expr(expr: &Expr, var_env: &HashMap<String, BindingId>) -> CompiledExpr {
    match expr {
        Expr::Var(name) => {
            // Look up the binding for this variable
            if let Some(&id) = var_env.get(name) {
                CompiledExpr::Binding(id)
            } else {
                // Variable not found - this is an error, but for now return a placeholder
                CompiledExpr::Const(Value::Error)
            }
        }

        Expr::Const(value) => CompiledExpr::Const(value.clone()),

        Expr::Call { query, args } => CompiledExpr::Call {
            query: query.clone(),
            args: args.iter().map(|e| compile_expr(e, var_env)).collect(),
        },

        Expr::Construct {
            constructor,
            fields,
        } => CompiledExpr::Construct {
            constructor: constructor.clone(),
            fields: fields.iter().map(|e| compile_expr(e, var_env)).collect(),
        },

        Expr::Let { bindings, body } => {
            // Create a new environment with the let-bound variables
            let new_env = var_env.clone();
            let compiled_bindings: Vec<_> = bindings
                .iter()
                .map(|(name, expr)| {
                    let compiled = compile_expr(expr, &new_env);
                    // Note: We don't add to new_env here because let bindings
                    // don't create BindingIds, they're handled in code generation
                    (name.clone(), Box::new(compiled))
                })
                .collect();

            CompiledExpr::Let {
                bindings: compiled_bindings,
                body: Box::new(compile_expr(body, &new_env)),
            }
        }

        Expr::Ok(inner) => CompiledExpr::Ok(Box::new(compile_expr(inner, var_env))),

        Expr::CurrentNode => {
            // CurrentNode refers to the input being matched
            CompiledExpr::Binding(BindingId(0))
        }

        Expr::TupleExpr(exprs) => {
            // Tuple expressions compile to Construct with "Tuple" constructor
            CompiledExpr::Construct {
                constructor: "Value::Tuple".to_string(),
                fields: exprs.iter().map(|e| compile_expr(e, var_env)).collect(),
            }
        }

        Expr::Do(exprs) => {
            // Do is a sequence - for now, just return the last expression
            if exprs.is_empty() {
                CompiledExpr::Const(Value::Error)
            } else {
                assert_eq!(exprs.len(), 1, "only one expression supported right now");
                compile_expr(exprs.last().unwrap(), var_env)
            }
        }

        Expr::RecordExpr { fields } | Expr::StructExpr { name: _, fields } => {
            // For now, treat records and structs similarly
            // TODO: Distinguish between structural and nominal
            CompiledExpr::Construct {
                constructor: "Value::Record".to_string(),
                fields: fields
                    .iter()
                    .map(|(_, e)| compile_expr(e, var_env))
                    .collect(),
            }
        }

        _ => {
            // TODO: Implement remaining expressions (TryLet, Match, If, ForEach, etc.)
            CompiledExpr::Const(Value::Error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builders::*;

    #[test]
    fn test_compile_integer_pattern() {
        let pattern = integer(capture("x"));
        let (bindings, constraints, _var_env) = compile_pattern(&pattern, BindingId(0));

        // Should have a captured binding
        assert_eq!(bindings.len(), 1);
        assert!(matches!(bindings[0], Binding::Captured(ref name) if name == "x"));

        // Should have an integer constraint
        assert_eq!(constraints.len(), 1);
        assert!(matches!(
            constraints[0],
            (BindingId(0), Constraint::IsInteger)
        ));
    }

    #[test]
    fn test_compile_apply_pattern() {
        let rule = apply(symbol("+"), [capture("lhs"), capture("rhs")]).then(var("result"));
        let (bindings, constraints, _var_env) = compile_pattern(&rule.pattern, BindingId(0));

        // Should extract callee and args
        assert!(!bindings.is_empty());

        // Should have IsApply and ArgsLength constraints
        assert!(
            constraints
                .iter()
                .any(|(_, c)| matches!(c, Constraint::IsApply))
        );
        assert!(
            constraints
                .iter()
                .any(|(_, c)| matches!(c, Constraint::ArgsLength(2)))
        );
    }

    #[test]
    fn test_compile_literal_in_apply() {
        // Test pattern: (+ x 0) where 0 is a literal
        let rule = apply(symbol("+"), [capture("x"), value(Value::Integer(0))]).then(var("x"));
        let (bindings, constraints, _var_env) = compile_pattern(&rule.pattern, BindingId(0));

        // Debug: print out all bindings and constraints
        eprintln!("Bindings:");
        for (i, b) in bindings.iter().enumerate() {
            eprintln!("  {}: {:?}", i, b);
        }
        eprintln!("Constraints:");
        for (source, c) in &constraints {
            eprintln!("  source={:?}: {:?}", source, c);
        }

        // Find the ConstInt constraint
        let const_int_constraint = constraints
            .iter()
            .find(|(_, c)| matches!(c, Constraint::ConstInt(0)));

        assert!(
            const_int_constraint.is_some(),
            "Should have ConstInt(0) constraint"
        );

        // The ConstInt constraint should be on binding_3 (the second extracted arg)
        if let Some((source, _)) = const_int_constraint {
            eprintln!("ConstInt(0) constraint is on binding {:?}", source);
        }
    }
}
