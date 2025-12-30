//! Analysis phase for semantic definitions.
//!
//! This module validates semantic definitions, builds dependency graphs,
//! detects cycles, checks for pattern overlaps, and identifies optimization
//! opportunities.

use crate::types::*;
use std::collections::{HashMap, HashSet};

/// Result of analyzing a semantic definition
#[derive(Clone, Debug)]
pub struct Analysis {
    /// Query dependency graph (which queries call which)
    pub dependencies: QueryDependencies,

    /// Pattern overlaps within queries
    pub overlaps: Vec<PatternOverlap>,

    /// Validation errors
    pub errors: Vec<AnalysisError>,

    /// Validation warnings
    pub warnings: Vec<AnalysisWarning>,
}

/// Dependency graph showing which queries call which others
#[derive(Clone, Debug, Default)]
pub struct QueryDependencies {
    /// Map from query name to the queries it depends on
    pub dependencies: HashMap<String, HashSet<String>>,
}

impl QueryDependencies {
    /// Add a dependency edge
    pub fn add_dependency(&mut self, from: String, to: String) {
        self.dependencies.entry(from).or_default().insert(to);
    }

    /// Get all queries that a given query depends on
    pub fn get_dependencies(&self, query: &str) -> Option<&HashSet<String>> {
        self.dependencies.get(query)
    }

    /// Check if there's a dependency path from `from` to `to`
    pub fn has_path(&self, from: &str, to: &str) -> bool {
        let mut visited = HashSet::new();
        self.has_path_helper(from, to, &mut visited)
    }

    fn has_path_helper(&self, from: &str, to: &str, visited: &mut HashSet<String>) -> bool {
        if from == to {
            return true;
        }

        if visited.contains(from) {
            return false;
        }

        visited.insert(from.to_string());

        if let Some(deps) = self.dependencies.get(from) {
            for dep in deps {
                if self.has_path_helper(dep, to, visited) {
                    return true;
                }
            }
        }

        false
    }
}

/// Pattern overlap within a query
#[derive(Clone, Debug)]
pub struct PatternOverlap {
    /// The query containing the overlap
    pub query: String,

    /// The indices of the overlapping rules
    pub rule_indices: (usize, usize),

    /// Description of the overlap
    pub description: String,
}

/// Analysis error
#[derive(Clone, Debug)]
pub struct AnalysisError {
    /// Error message
    pub message: String,

    /// Related query name
    pub query: Option<String>,
}

/// Analysis warning
#[derive(Clone, Debug)]
pub struct AnalysisWarning {
    /// Warning message
    pub message: String,

    /// Related query name
    pub query: Option<String>,
}

/// Analyze a semantic definition
pub fn analyze(semantics: &Semantics) -> Analysis {
    let mut analysis = Analysis {
        dependencies: QueryDependencies::default(),
        overlaps: Vec::new(),
        errors: Vec::new(),
        warnings: Vec::new(),
    };

    // Build dependency graph
    build_dependency_graph(semantics, &mut analysis);

    // Check for undefined queries
    check_undefined_queries(semantics, &mut analysis);

    // Check for pattern overlaps
    check_pattern_overlaps(semantics, &mut analysis);

    // Validate query signatures
    validate_query_signatures(semantics, &mut analysis);

    analysis
}

/// Build the dependency graph by analyzing query calls in expressions
fn build_dependency_graph(semantics: &Semantics, analysis: &mut Analysis) {
    for query in &semantics.queries {
        for rule in &query.rules {
            collect_expr_dependencies(&query.name, &rule.result, &mut analysis.dependencies);

            if let Some(guard) = &rule.guard {
                collect_guard_dependencies(&query.name, guard, &mut analysis.dependencies);
            }
        }
    }
}

/// Recursively collect query calls from an expression
fn collect_expr_dependencies(from_query: &str, expr: &Expr, deps: &mut QueryDependencies) {
    match expr {
        Expr::Call { query, args } => {
            deps.add_dependency(from_query.to_string(), query.clone());
            for arg in args {
                collect_expr_dependencies(from_query, arg, deps);
            }
        }
        Expr::Let { bindings, body } | Expr::TryLet { bindings, body, .. } => {
            for (_, binding_expr) in bindings {
                collect_expr_dependencies(from_query, binding_expr, deps);
            }
            collect_expr_dependencies(from_query, body, deps);
        }
        Expr::Do(exprs) => {
            for e in exprs {
                collect_expr_dependencies(from_query, e, deps);
            }
        }
        Expr::Match { scrutinee, arms } => {
            collect_expr_dependencies(from_query, scrutinee, deps);
            for arm in arms {
                collect_expr_dependencies(from_query, &arm.body, deps);
            }
        }
        Expr::If {
            condition,
            then_branch,
            else_branch,
        } => {
            collect_expr_dependencies(from_query, condition, deps);
            collect_expr_dependencies(from_query, then_branch, deps);
            collect_expr_dependencies(from_query, else_branch, deps);
        }
        Expr::Construct { fields, .. } => {
            for field in fields {
                collect_expr_dependencies(from_query, field, deps);
            }
        }
        Expr::Array(items) => {
            for item in items {
                collect_expr_dependencies(from_query, item, deps);
            }
        }
        Expr::Spanned { value, .. } => {
            collect_expr_dependencies(from_query, value, deps);
        }
        Expr::ErrorAndReturn { fallback, .. } => {
            collect_expr_dependencies(from_query, fallback, deps);
        }
        _ => {
            // Other expression types don't contain query calls
        }
    }
}

/// Collect dependencies from guards
fn collect_guard_dependencies(from_query: &str, guard: &Guard, deps: &mut QueryDependencies) {
    match guard {
        Guard::Match { expr, .. } => {
            collect_expr_dependencies(from_query, expr, deps);
        }
        Guard::Call { args, .. } => {
            for arg in args {
                collect_expr_dependencies(from_query, arg, deps);
            }
        }
        Guard::Eq(left, right) => {
            collect_expr_dependencies(from_query, left, deps);
            collect_expr_dependencies(from_query, right, deps);
        }
        Guard::And(guards) => {
            for g in guards {
                collect_guard_dependencies(from_query, g, deps);
            }
        }
    }
}

/// Check for undefined query references
fn check_undefined_queries(semantics: &Semantics, analysis: &mut Analysis) {
    let defined_queries: HashSet<String> =
        semantics.queries.iter().map(|q| q.name.clone()).collect();

    for (from_query, to_queries) in &analysis.dependencies.dependencies {
        for to_query in to_queries {
            if !defined_queries.contains(to_query) {
                analysis.errors.push(AnalysisError {
                    message: format!(
                        "Query '{}' references undefined query '{}'",
                        from_query, to_query
                    ),
                    query: Some(from_query.clone()),
                });
            }
        }
    }
}

/// Check for pattern overlaps within queries
fn check_pattern_overlaps(_semantics: &Semantics, _analysis: &mut Analysis) {
    // TODO: Implement pattern overlap detection
    // This requires checking if two patterns can match the same syntax
}

/// Validate query signatures
fn validate_query_signatures(semantics: &Semantics, analysis: &mut Analysis) {
    for query in &semantics.queries {
        // Check that external queries have no rules
        if query.external && !query.rules.is_empty() {
            analysis.errors.push(AnalysisError {
                message: format!(
                    "External query '{}' should not have rules (implementation provided externally)",
                    query.name
                ),
                query: Some(query.name.clone()),
            });
        }

        // Check that non-external queries have at least one rule
        if !query.external && query.rules.is_empty() {
            analysis.warnings.push(AnalysisWarning {
                message: format!("Query '{}' has no rules defined", query.name),
                query: Some(query.name.clone()),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builders::*;

    #[test]
    fn test_dependency_graph() {
        let semantics = Semantics::new()
            .add_query(
                query("eval")
                    .input(node_id())
                    .output(value_type())
                    .rule(integer(capture("x")).then(call("type_of", [var("x")])))
                    .build(),
            )
            .add_query(
                query("type_of")
                    .input(node_id())
                    .output(type_type())
                    .rule(integer(wildcard()).then(const_val(Value::Type(Type::NodeId))))
                    .build(),
            );

        let analysis = analyze(&semantics);

        // eval depends on type_of
        assert!(
            analysis
                .dependencies
                .get_dependencies("eval")
                .unwrap()
                .contains("type_of")
        );
    }

    #[test]
    fn test_undefined_query_detection() {
        let semantics = Semantics::new().add_query(
            query("eval")
                .input(node_id())
                .output(value_type())
                .rule(integer(capture("x")).then(call("undefined", vec![var("x")])))
                .build(),
        );

        let analysis = analyze(&semantics);

        // Should detect undefined query reference
        assert!(!analysis.errors.is_empty());
        assert!(
            analysis.errors[0]
                .message
                .contains("undefined query 'undefined'")
        );
    }

    #[test]
    fn test_external_query_validation() {
        let semantics = Semantics::new().add_query(
            query("external")
                .input(node_id())
                .output(value_type())
                .extern_impl()
                .rule(integer(capture("x")).then(var("x")))
                .build(),
        );

        let analysis = analyze(&semantics);

        // External queries shouldn't have rules
        assert!(!analysis.errors.is_empty());
        assert!(analysis.errors[0].message.contains("should not have rules"));
    }
}
