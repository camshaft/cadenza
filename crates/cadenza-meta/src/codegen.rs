//! Code generation for semantic definitions.
//!
//! This module generates Rust code from semantic definitions using the
//! quote and proc_macro2 crates.

use crate::{analysis::Analysis, bindings::BindingId, tree::*, types::*};
use proc_macro2::{Ident, TokenStream};
use quote::{format_ident, quote};
use std::collections::HashSet;

/// Generate Rust code for a semantic definition
pub fn generate(semantics: &Semantics, _analysis: &Analysis) -> TokenStream {
    let queries = semantics.queries.iter().map(|query| {
        let query_fn = generate_query(query);
        quote! { #query_fn }
    });

    // Add imports prelude
    quote! {
        use crate::prelude::*;

        #(#queries)*
    }
}

/// Generate code for a single query
fn generate_query(query: &Query) -> TokenStream {
    let name = format_ident!("{}", query.name);
    let input_ty = generate_type(&query.input);
    let output_ty = generate_type(&query.output);

    if query.external {
        // External query - just generate the signature
        quote! {
            pub fn #name(db: &dyn Database, input: #input_ty) -> #output_ty {
                unimplemented!("External query must be implemented by user")
            }
        }
    } else {
        // Compile rules to binding-based IR
        let compiled_rules: Vec<crate::bindings::CompiledRule> = query
            .rules
            .iter()
            .map(|rule| {
                // compile_pattern now returns bindings with Input already at index 0
                let (pattern_bindings, constraints, var_env) =
                    crate::bindings::compile_pattern(&rule.pattern, BindingId(0));

                // Build complete bindings list: Input at 0, then pattern bindings
                let mut all_bindings = vec![crate::bindings::Binding::Input];
                all_bindings.extend(pattern_bindings);

                // Compile result expression using the var_env
                let result = crate::bindings::compile_expr(&rule.result, &var_env);

                crate::bindings::CompiledRule {
                    original: rule.clone(),
                    bindings: all_bindings,
                    constraints,
                    result,
                }
            })
            .collect();

        // Build decision tree
        let tree = crate::tree::build_decision_tree(&compiled_rules);

        // Generate code from tree
        let tree_code = generate_block(&tree, &compiled_rules);

        quote! {
            pub fn #name(db: &dyn Database, input: #input_ty) -> #output_ty {
                #tree_code
            }
        }
    }
}

/// Generate code for a type
fn generate_type(ty: &Type) -> TokenStream {
    match ty {
        Type::NodeId => quote! { NodeId },
        Type::Value => quote! { Value },
        Type::Type => quote! { Type },
        Type::String => quote! { String },
        Type::Bool => quote! { bool },
        Type::Diagnostics => quote! { Diagnostics },
        Type::Symbol => quote! { Symbol },
        Type::EnvId => quote! { EnvId },
        Type::Option(inner) => {
            let inner_ty = generate_type(inner);
            quote! { Option<#inner_ty> }
        }
        Type::Result(ok, err) => {
            let ok_ty = generate_type(ok);
            let err_ty = generate_type(err);
            quote! { Result<#ok_ty, #err_ty> }
        }
        Type::Array(inner) => {
            let inner_ty = generate_type(inner);
            quote! { Vec<#inner_ty> }
        }
        Type::HashMap(k, v) => {
            let key_ty = generate_type(k);
            let val_ty = generate_type(v);
            quote! { HashMap<#key_ty, #val_ty> }
        }
        Type::Spanned(inner) => {
            let inner_ty = generate_type(inner);
            quote! { Spanned<#inner_ty> }
        }
        Type::List(inner) => {
            let inner_ty = generate_type(inner);
            quote! { Vec<#inner_ty> }
        }
        Type::Tuple(types) => {
            let type_codes = types.iter().map(generate_type);
            quote! { (#(#type_codes),*) }
        }
        _ => {
            // For now, unimplemented types become ()
            // This allows build to succeed while we add support incrementally
            quote! { () }
        }
    }
}

/// Generate code for a value
fn generate_value(value: &Value) -> TokenStream {
    match value {
        Value::Integer(i) => quote! { #i },
        Value::Bool(b) => quote! { #b },
        Value::String(s) => quote! { #s },
        Value::Type(ty) => {
            let ty_code = generate_type(ty);
            quote! { Value::Type(#ty_code) }
        }
        _ => quote! { todo!("other values") },
    }
}

/// Generate code for a decision tree block
fn generate_block(block: &Block, rules: &[crate::bindings::CompiledRule]) -> TokenStream {
    generate_block_with_emitted(block, rules, &mut HashSet::new())
}

/// Generate code for a decision tree block, tracking already-emitted bindings
fn generate_block_with_emitted(
    block: &Block,
    rules: &[crate::bindings::CompiledRule],
    emitted: &mut HashSet<BindingId>,
) -> TokenStream {
    let steps = block
        .steps
        .iter()
        .map(|step| generate_step_with_emitted(step, rules, emitted));

    quote! {
        #(#steps)*
    }
}

/// Generate code for a single evaluation step, tracking already-emitted bindings
fn generate_step_with_emitted(
    step: &EvalStep,
    rules: &[crate::bindings::CompiledRule],
    emitted: &mut HashSet<BindingId>,
) -> TokenStream {
    // Generate let bindings for this step (only if not already emitted)
    let mut let_stmts = Vec::new();

    for &binding_id in &step.let_bindings {
        if !emitted.contains(&binding_id) {
            // Find the binding in one of the rules
            if let Some(rule) = rules.first()
                && let Some(binding) = rule.bindings.get(binding_id.0) {
                    let stmt = generate_binding_statement(binding_id, binding);
                    let_stmts.push(stmt);
                    emitted.insert(binding_id);
                }
        }
    }

    let control = generate_control_flow_with_emitted(&step.control, rules, emitted);

    if let_stmts.is_empty() {
        quote! {
            #control
        }
    } else {
        quote! {
            #(#let_stmts)*
            #control
        }
    }
}

/// Generate code for control flow, tracking already-emitted bindings
fn generate_control_flow_with_emitted(
    control: &ControlFlow,
    rules: &[crate::bindings::CompiledRule],
    emitted: &mut HashSet<BindingId>,
) -> TokenStream {
    match control {
        ControlFlow::CheckConstraint {
            source,
            constraint,
            bindings: introduced_bindings,
            body,
        } => {
            // Generate binding statements for introduced bindings that are safe after this constraint
            let mut binding_stmts = Vec::new();
            if let Some(rule) = rules.first() {
                for &binding_id in introduced_bindings {
                    if !emitted.contains(&binding_id)
                        && let Some(binding) = rule.bindings.get(binding_id.0) {
                            // Only emit if this binding is safe after the current constraint
                            if is_binding_safe_after_constraint(binding, constraint) {
                                let stmt = generate_binding_statement(binding_id, binding);
                                binding_stmts.push(stmt);
                                emitted.insert(binding_id);
                            }
                        }
                }
            }

            let body_code = generate_block_with_emitted(body, rules, emitted);

            // For IsApply, generate if-let instead of matches!
            if matches!(constraint, crate::bindings::Constraint::IsApply) {
                let source_name = format_ident!("binding_{}", source.0);
                // If source is input (0), use input directly
                let source_expr = if source.0 == 0 {
                    quote! { input }
                } else {
                    quote! { #source_name }
                };

                quote! {
                    if let Value::Apply { callee, args } = #source_expr {
                        #(#binding_stmts)*
                        #body_code
                    }
                }
            } else {
                // For other constraints, use the old approach
                let check = generate_constraint_check(*source, constraint);

                if binding_stmts.is_empty() {
                    quote! {
                        if #check {
                            #body_code
                        }
                    }
                } else {
                    quote! {
                        if #check {
                            #(#binding_stmts)*
                            #body_code
                        }
                    }
                }
            }
        }

        ControlFlow::Return { result } => {
            let result_code = generate_compiled_expr(result, rules);
            quote! {
                return #result_code;
            }
        }

        ControlFlow::MatchValue { .. } => {
            quote! { todo!("MatchValue") }
        }
    }
}

/// Generate code to check a constraint
fn generate_constraint_check(
    source: BindingId,
    constraint: &crate::bindings::Constraint,
) -> TokenStream {
    // Get source expression
    let source_expr = if source.0 == 0 {
        quote! { input }
    } else {
        let binding_name = format_ident!("binding_{}", source.0);
        quote! { #binding_name }
    };

    match constraint {
        crate::bindings::Constraint::IsInteger => {
            quote! { matches!(#source_expr, Value::Integer(_)) }
        }
        crate::bindings::Constraint::IsApply => {
            quote! { matches!(#source_expr, Value::Apply { .. }) }
        }
        crate::bindings::Constraint::IsSymbol(name) => {
            quote! { matches!(#source_expr, Value::Symbol(ref s) if s == #name) }
        }
        crate::bindings::Constraint::IsTuple(size) => {
            quote! { matches!(#source_expr, Value::Tuple(ref t) if t.len() == #size) }
        }
        crate::bindings::Constraint::ConstInt(value) => {
            quote! { matches!(#source_expr, Value::Integer(v) if v == #value) }
        }
        crate::bindings::Constraint::ConstBool(value) => {
            quote! { matches!(#source_expr, Value::Bool(b) if b == #value) }
        }
        crate::bindings::Constraint::ConstString(value) => {
            quote! { matches!(#source_expr, Value::String(ref s) if s == #value) }
        }
        crate::bindings::Constraint::ArgsLength(len) => {
            quote! { args.len() == #len }
        }
        _ => quote! { todo!("other constraints") },
    }
}

/// Generate code for a compiled expression
fn generate_compiled_expr(
    expr: &crate::bindings::CompiledExpr,
    rules: &[crate::bindings::CompiledRule],
) -> TokenStream {
    match expr {
        crate::bindings::CompiledExpr::Binding(id) => {
            // Look up what this binding is
            // For now, if it's the captured value, just use input
            if id.0 == 1 {
                // Binding 1 is typically the first capture after Input
                // For integer pattern, this would be the integer value itself
                quote! { input }
            } else {
                let binding_name = format_ident!("binding_{}", id.0);
                quote! { #binding_name }
            }
        }
        crate::bindings::CompiledExpr::Const(value) => generate_value(value),
        crate::bindings::CompiledExpr::Ok(inner) => {
            let inner_code = generate_compiled_expr(inner, rules);
            quote! { Ok(#inner_code) }
        }
        crate::bindings::CompiledExpr::Call { query, args } => {
            let query_fn = format_ident!("{query}");
            let arg_codes = args.iter().map(|arg| generate_compiled_expr(arg, rules));
            quote! { #query_fn(db, #(#arg_codes),*) }
        }
        crate::bindings::CompiledExpr::Construct {
            constructor,
            fields,
        } => {
            let ctor = parse_constructor_path(constructor);
            let field_codes = fields.iter().map(|f| generate_compiled_expr(f, rules));
            quote! { #ctor(#(#field_codes),*) }
        }
        crate::bindings::CompiledExpr::Let { bindings, body } => {
            let binding_stmts = bindings.iter().map(|(name, value)| {
                let ident = format_ident!("{name}");
                let value_code = generate_compiled_expr(value, rules);
                quote! { let #ident = #value_code; }
            });
            let body_code = generate_compiled_expr(body, rules);
            quote! {
                {
                    #(#binding_stmts)*
                    #body_code
                }
            }
        }
    }
}

/// Generate a let statement for a binding
fn generate_binding_statement(
    binding_id: BindingId,
    binding: &crate::bindings::Binding,
) -> TokenStream {
    let binding_name = format_ident!("binding_{}", binding_id.0);

    match binding {
        crate::bindings::Binding::Input => {
            quote! { let #binding_name = input; }
        }
        crate::bindings::Binding::Captured(_) => {
            // Captured bindings are handled by pattern matching
            quote! {}
        }
        crate::bindings::Binding::Extract { source: _, kind } => match kind {
            crate::bindings::ExtractKind::ApplyCallee => {
                quote! { let #binding_name = callee; }
            }
            crate::bindings::ExtractKind::ApplyArg(idx) => {
                quote! { let #binding_name = &args[#idx]; }
            }
            crate::bindings::ExtractKind::ApplyArgs => {
                quote! { let #binding_name = args; }
            }
            crate::bindings::ExtractKind::TupleField(idx) => {
                quote! { let #binding_name = &tuple[#idx]; }
            }
            crate::bindings::ExtractKind::RecordField(field_name) => {
                quote! { let #binding_name = &record.get(#field_name).unwrap(); }
            }
            crate::bindings::ExtractKind::StructField(field_name) => {
                quote! { let #binding_name = &struct_val.#field_name; }
            }
        },
        crate::bindings::Binding::Constant(value) => {
            let value_code = generate_value(value);
            quote! { let #binding_name = #value_code; }
        }
        crate::bindings::Binding::QueryCall { query, args } => {
            let query_fn = format_ident!("{}", query);
            let arg_names: Vec<_> = args
                .iter()
                .map(|id| format_ident!("binding_{}", id.0))
                .collect();
            quote! { let #binding_name = #query_fn(db, #(#arg_names),*); }
        }
        _ => {
            // Captured and LetBound are handled elsewhere
            quote! {}
        }
    }
}

/// Check if a binding is safe to emit after a given constraint
fn is_binding_safe_after_constraint(
    binding: &crate::bindings::Binding,
    constraint: &crate::bindings::Constraint,
) -> bool {
    match binding {
        crate::bindings::Binding::Extract { kind, .. } => match kind {
            // Callee is safe after IsApply
            crate::bindings::ExtractKind::ApplyCallee => {
                matches!(constraint, crate::bindings::Constraint::IsApply)
            }
            // Arg extractions are only safe after ArgsLength check
            crate::bindings::ExtractKind::ApplyArg(_) => {
                matches!(constraint, crate::bindings::Constraint::ArgsLength(_))
            }
            // ApplyArgs (the whole vec) is safe after IsApply
            crate::bindings::ExtractKind::ApplyArgs => {
                matches!(constraint, crate::bindings::Constraint::IsApply)
            }
            // TODO: Add safety rules for other extract kinds
            _ => true, // Default to safe for now
        },
        // Other binding types are always safe
        _ => true,
    }
}

/// Parse a constructor path like "Value::Integer" into tokens
fn parse_constructor_path(path: &str) -> TokenStream {
    let parts: Vec<&str> = path.split("::").collect();
    let idents: Vec<Ident> = parts.iter().map(|part| format_ident!("{}", part)).collect();

    quote! { #(#idents)::* }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builders::*;
    use insta::assert_snapshot;

    #[test]
    fn test_generate_simple_query() {
        let semantics = Semantics::new().add_query(
            query("eval")
                .input(node_id())
                .output(value_type())
                .rule(integer(capture("x")).then(construct("Value::Integer", [var("x")])))
                .build(),
        );

        let analysis = crate::analysis::analyze(&semantics);
        let code = generate(&semantics, &analysis);

        // Should generate a function named eval
        // TODO rustfmt this
        let code_str = code.to_string();
        assert_snapshot!(code_str);
    }

    #[test]
    fn test_generate_apply_pattern() {
        let semantics = Semantics::new().add_query(
            query("eval")
                .input(node_id())
                .output(value_type())
                .rule(
                    apply(symbol("+"), [capture("lhs"), capture("rhs")])
                        .then(construct("Value::Integer", [var("lhs")])),
                )
                .build(),
        );

        let analysis = crate::analysis::analyze(&semantics);
        let code = generate(&semantics, &analysis);

        // Should generate if-let for Apply pattern with callee/args extraction
        let code_str = code.to_string();
        assert_snapshot!(code_str);
    }

    #[test]
    fn test_generate_literal_matching() {
        let semantics = Semantics::new().add_query(
            query("simplify")
                .input(node_id())
                .output(value_type())
                .rule(
                    // (+ x 0) -> x
                    apply(symbol("+"), [capture("x"), value(Value::Integer(0))]).then(var("x")),
                )
                .build(),
        );

        let analysis = crate::analysis::analyze(&semantics);
        let code = generate(&semantics, &analysis);

        // Should generate literal matching for the 0
        let code_str = code.to_string();
        assert_snapshot!(code_str);
    }
}
