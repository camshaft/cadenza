//! Decision tree construction for efficient pattern matching.
//!
//! Adapts ISLE's serialize.rs approach: build a tree of control flow constructs
//! from bindings and constraints, which can then be translated to efficient
//! Rust code.

use crate::bindings::*;
use std::collections::HashSet;

/// A block of sequential evaluation steps
#[derive(Clone, Debug)]
pub struct Block {
    /// Steps to evaluate in order
    pub steps: Vec<EvalStep>,
}

/// A single evaluation step
#[derive(Clone, Debug)]
pub struct EvalStep {
    /// Bindings to emit before this step
    pub let_bindings: Vec<BindingId>,

    /// The control flow at this step
    pub control: ControlFlow,
}

/// Control flow construct
#[derive(Clone, Debug)]
pub enum ControlFlow {
    /// Check a constraint and branch
    CheckConstraint {
        /// Which binding to test
        source: BindingId,

        /// The constraint to test
        constraint: Constraint,

        /// Bindings introduced if constraint succeeds
        bindings: Vec<BindingId>,

        /// Block to execute if constraint matches
        body: Box<Block>,
    },

    /// Check if a binding matches a value pattern
    MatchValue {
        /// Which binding to match
        source: BindingId,

        /// Arms to try
        arms: Vec<MatchArm>,
    },

    /// Return a result
    Return {
        /// The result expression
        result: CompiledExpr,
    },
}

/// A match arm
#[derive(Clone, Debug)]
pub struct MatchArm {
    /// The constraint for this arm
    pub constraint: Constraint,

    /// Bindings introduced by this arm
    pub bindings: Vec<BindingId>,

    /// Body if this arm matches
    pub body: Block,
}

/// Build a decision tree from compiled rules
pub fn build_decision_tree(rules: &[CompiledRule]) -> Block {
    if rules.is_empty() {
        return Block { steps: Vec::new() };
    }

    // Build nested constraint checks for each rule
    let mut steps = Vec::new();

    for rule in rules {
        // Build nested checks from constraints
        let nested_check = build_nested_constraints(rule);
        steps.push(nested_check);
    }

    Block { steps }
}

/// Build nested constraint checks from a rule's constraints
fn build_nested_constraints(rule: &CompiledRule) -> EvalStep {
    if rule.constraints.is_empty() {
        // No constraints - just return
        return EvalStep {
            let_bindings: Vec::new(),
            control: ControlFlow::Return {
                result: rule.result.clone(),
            },
        };
    }

    // Build from innermost (result) outward
    let mut body = Block {
        steps: vec![EvalStep {
            let_bindings: Vec::new(),
            control: ControlFlow::Return {
                result: rule.result.clone(),
            },
        }],
    };

    // Process constraints in reverse order to build nesting from inside out
    for (source, constraint) in rule.constraints.iter().rev() {
        let introduced = find_introduced_bindings(&rule.bindings, *source, constraint);

        body = Block {
            steps: vec![EvalStep {
                let_bindings: Vec::new(),
                control: ControlFlow::CheckConstraint {
                    source: *source,
                    constraint: constraint.clone(),
                    bindings: introduced,
                    body: Box::new(body),
                },
            }],
        };
    }

    // Return the outermost step
    body.steps.into_iter().next().unwrap()
}

/// Find which bindings are introduced by matching a constraint
fn find_introduced_bindings(
    bindings: &[Binding],
    _source: BindingId,
    constraint: &Constraint,
) -> Vec<BindingId> {
    // For now, return all Extract bindings
    // TODO: Filter to only those introduced by this specific constraint
    bindings
        .iter()
        .enumerate()
        .filter_map(|(i, b)| {
            if matches!(b, Binding::Extract { .. }) {
                Some(BindingId(i))
            } else {
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::*;
    use insta::assert_debug_snapshot;

    #[test]
    fn test_build_simple_tree() {
        let rule = CompiledRule {
            original: integer(capture("x")).then(var("x")),
            bindings: vec![Binding::Input, Binding::Captured("x".to_string())],
            constraints: vec![(BindingId(0), Constraint::IsInteger)],
            result: CompiledExpr::Binding(BindingId(1)),
        };

        let tree = build_decision_tree(&[rule]);

        assert_debug_snapshot!(tree);
    }

    #[test]
    fn test_build_tree_with_multiple_rules() {
        let rule1 = CompiledRule {
            original: integer(capture("x")).then(var("x")),
            bindings: vec![Binding::Input],
            constraints: vec![(BindingId(0), Constraint::IsInteger)],
            result: CompiledExpr::Const(Value::Integer(1)),
        };

        let rule2 = CompiledRule {
            original: symbol("y").then(var("y")),
            bindings: vec![Binding::Input],
            constraints: vec![(BindingId(0), Constraint::IsSymbol("test".to_string()))],
            result: CompiledExpr::Const(Value::Integer(2)),
        };

        let tree = build_decision_tree(&[rule1, rule2]);

        assert_debug_snapshot!(tree);
    }
}
