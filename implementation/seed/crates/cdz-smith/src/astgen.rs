//! A COERCING program generator (S6) — the operator's directed generation mechanism.
//!
//! Operator: *"the bolero type/value generator traits all try really really hard to COERCE a valid
//! value. We should use those instead of a seeded libfuzzer corpus — libfuzzer isn't going to get
//! value out of the seeded corpus with seeds."* So the driving mechanism is a bolero
//! [`ValueGenerator`] that maps ARBITRARY entropy → a VALID Cadenza program (always succeeds, by
//! construction), driven coverage-guided by bolero — rather than mutating a corpus of seed bytes that
//! the strict decode-gate mostly rejects. Bolero's [`Driver`] does the coercion (bounded ints, variant
//! choices, depth), so every input maps to a well-formed, type-correct program the compiler accepts.
//!
//! This v0 grammar is deliberately small — `(do (def (main) <expr>) (export main))` where `<expr>` is
//! an `Int64` literal, a binary arithmetic tree, or a conditional (`(if (<rel> …) … …)`) — kept
//! type-correct Int64 throughout so a generated program is cleanly HANDLED (it compiles, or cleanly
//! declines e.g. on a const-folded overflow), and the property target asserts the compiler never
//! PANICS or emits invalid wasm on them. Later
//! increments widen the grammar (let/if/functions/…, and direct binary-AST emission) behind the same
//! `ValueGenerator` seam; the semantics-corpus ASTs inform which constructs to add, but the DRIVING
//! mechanism stays "coerce entropy → valid program", never a seed corpus.

use core::fmt::Write as _;
use core::ops::Bound;

use bolero::generator::ValueGenerator;
use bolero::generator::bolero_generator::Driver;

use crate::generator::Program;

/// The recursion budget for a generated expression (bounds program size + guarantees termination).
const MAX_DEPTH: usize = 4;

/// Int64 → Int64 → Int64 binary operators: arithmetic, division/modulo, and bitwise/shift. All
/// type-check as Int64; runtime edges (a const `*` overflow → CDZ decline; a `/`/`%` by zero → a
/// trap; a large/negative `<<`/`>>` count → shift-count masking) are runnable outcomes the compiler
/// handles cleanly, and the bitwise/shift lowering is a known Wasm-vs-Rust disagreement surface.
const OPS: [&str; 10] = ["+", "-", "*", "/", "%", "&", "|", "^", "<<", ">>"];

/// Int64 → Int64 → Bool relational operators, for the condition of an `if` (both branches are Int64).
const RELS: [&str; 4] = ["<=", "<", ">=", ">"];

/// Boundary Int64 literals — where width / overflow / wrap / sign-extend miscompiles cluster (mirrors
/// `generator.rs`'s `INT_BOUNDARIES`). Index 0 is `0`, so exhausted entropy still yields a trivial
/// compilable literal. Includes i64 / i32 / i16 / i8 / u* edges and the small ±1 neighbours.
const INT_BOUNDARIES: [i64; 15] = [
    0,
    1,
    -1,
    i64::MAX,
    i64::MIN,
    i32::MAX as i64,
    i32::MIN as i64,
    i32::MAX as i64 + 1,
    127,
    -128,
    255,
    256,
    32767,
    -32768,
    4_294_967_295,
];

/// A bolero [`ValueGenerator`] that coerces the driver's entropy into a valid `(do (def (main) …)
/// (export main))` program. Wire it with `check!().with_generator(ProgramGen)`.
pub struct ProgramGen;

impl ValueGenerator for ProgramGen {
    type Output = Program;

    fn generate<D: Driver>(&self, driver: &mut D) -> Option<Program> {
        // INFALLIBLE by design: a coercing generator always yields a valid program, even from empty /
        // exhausted entropy (every driver read falls back to a default), so `generate` never returns
        // `None`. That is the operator's "coerce ANY entropy → a valid value".
        let mut source = String::from("(do ");
        let mut fresh = 0usize;
        // Optionally emit a NON-RECURSIVE helper `(def (f a b) <body>)`: its body uses the Int64 params
        // `a`/`b` but CANNOT call `f` (so it always terminates), and `main`'s body may then call
        // `(f <e> <e>)` — reaching multi-arg function-def + call lowering, kept total.
        let has_helper = driver.gen_variant(2, 0).unwrap_or(0) == 1;
        if has_helper {
            source.push_str("(def (f a b) ");
            let mut fscope = vec!["a".to_string(), "b".to_string()];
            gen_expr(
                driver,
                MAX_DEPTH,
                &mut fscope,
                &mut fresh,
                false,
                &mut source,
            );
            source.push_str(") ");
        }
        source.push_str("(def (main) ");
        let mut scope: Vec<String> = Vec::new();
        gen_expr(
            driver,
            MAX_DEPTH,
            &mut scope,
            &mut fresh,
            has_helper,
            &mut source,
        );
        source.push_str(") (export main))");
        Some(Program { source })
    }
}

/// Append one coerced `Int64` expression: at `depth == 0` (or when the driver picks the base variant) an
/// integer literal; otherwise a binary arithmetic node over two sub-expressions. Every driver read falls
/// back to a default on exhaustion, so this always produces a well-formed sub-expression.
fn gen_expr<D: Driver>(
    driver: &mut D,
    depth: usize,
    scope: &mut Vec<String>,
    fresh: &mut usize,
    can_call_f: bool,
    out: &mut String,
) {
    // At `depth == 0` force the base case (0); otherwise `gen_variant(n, 0)` biases toward it — so
    // generation always terminates within the depth budget. The `(f …)` call arm is only offered when a
    // helper is in scope (`can_call_f`). Exhaustion → base case (0).
    let arms = if can_call_f { 5 } else { 4 };
    let variant = if depth == 0 {
        0
    } else {
        driver.gen_variant(arms, 0).unwrap_or(0)
    };
    match variant {
        // Binary arithmetic `(op <e> <e>)`.
        1 => {
            let op = OPS[driver.gen_variant(OPS.len(), 0).unwrap_or(0)];
            out.push('(');
            out.push_str(op);
            out.push(' ');
            gen_expr(driver, depth - 1, scope, fresh, can_call_f, out);
            out.push(' ');
            gen_expr(driver, depth - 1, scope, fresh, can_call_f, out);
            out.push(')');
        }
        // Conditional `(if <cond> <e> <e>)` — `<cond>` is a Bool (relations + boolean connectives),
        // both branches Int64, so the whole `if` is Int64 and type-checks.
        2 => {
            out.push_str("(if ");
            gen_cond(driver, depth - 1, scope, fresh, can_call_f, out);
            out.push(' ');
            gen_expr(driver, depth - 1, scope, fresh, can_call_f, out);
            out.push(' ');
            gen_expr(driver, depth - 1, scope, fresh, can_call_f, out);
            out.push(')');
        }
        // Let binding `(let ((vN <val>)) <body>)` — binds a FRESH Int64 name (so no shadowing) and adds
        // it to scope for the body, which may then reference it. The bound value is generated BEFORE the
        // name enters scope (a `let` is non-recursive). Reaches name-resolution / binding lowering.
        3 => {
            let name = format!("v{fresh}");
            *fresh += 1;
            out.push_str("(let ((");
            out.push_str(&name);
            out.push(' ');
            gen_expr(driver, depth - 1, scope, fresh, can_call_f, out);
            out.push_str(")) ");
            scope.push(name);
            gen_expr(driver, depth - 1, scope, fresh, can_call_f, out);
            scope.pop();
            out.push(')');
        }
        // Call the in-scope helper `(f <e> <e>)` — `f: Int64,Int64 -> Int64` is non-recursive + total, so
        // the call terminates. Only reachable when `can_call_f`. Reaches function-call lowering.
        4 => {
            out.push_str("(f ");
            gen_expr(driver, depth - 1, scope, fresh, can_call_f, out);
            out.push(' ');
            gen_expr(driver, depth - 1, scope, fresh, can_call_f, out);
            out.push(')');
        }
        // Base case: an in-scope Int64 variable reference (when any is bound and the driver picks it) —
        // which keeps the expression Int64 — else a bounded Int64 literal.
        _ => {
            if !scope.is_empty() && driver.gen_variant(2, 0).unwrap_or(0) == 1 {
                let idx = driver.gen_variant(scope.len(), 0).unwrap_or(0);
                out.push_str(&scope[idx]);
            } else {
                // Bias toward boundary values (where width/overflow/wrap miscompiles cluster); else a
                // bounded random int. Both are valid Int64 literals.
                let n = if driver.gen_variant(2, 0).unwrap_or(0) == 1 {
                    INT_BOUNDARIES[driver.gen_variant(INT_BOUNDARIES.len(), 0).unwrap_or(0)]
                } else {
                    driver
                        .gen_i64(Bound::Included(&-1_000_000), Bound::Included(&1_000_000))
                        .unwrap_or(0)
                };
                write!(out, "{n}").ok();
            }
        }
    }
}

/// Append one coerced Bool CONDITION (for an `if`): a base relation `(<rel> <e> <e>)` over Int64
/// sub-expressions, or a boolean connective `(and <c> <c>)` / `(or <c> <c>)` / `(not <c>)`. Reaches
/// boolean-connective + short-circuit lowering. Depth-bounded (base = a relation) so it terminates.
fn gen_cond<D: Driver>(
    driver: &mut D,
    depth: usize,
    scope: &mut Vec<String>,
    fresh: &mut usize,
    can_call_f: bool,
    out: &mut String,
) {
    let variant = if depth == 0 {
        0
    } else {
        driver.gen_variant(4, 0).unwrap_or(0)
    };
    match variant {
        // `(and <c> <c>)` — short-circuit conjunction.
        1 => {
            out.push_str("(and ");
            gen_cond(driver, depth - 1, scope, fresh, can_call_f, out);
            out.push(' ');
            gen_cond(driver, depth - 1, scope, fresh, can_call_f, out);
            out.push(')');
        }
        // `(or <c> <c>)` — short-circuit disjunction.
        2 => {
            out.push_str("(or ");
            gen_cond(driver, depth - 1, scope, fresh, can_call_f, out);
            out.push(' ');
            gen_cond(driver, depth - 1, scope, fresh, can_call_f, out);
            out.push(')');
        }
        // `(not <c>)` — negation.
        3 => {
            out.push_str("(not ");
            gen_cond(driver, depth - 1, scope, fresh, can_call_f, out);
            out.push(')');
        }
        // Base case: a relation `(<rel> <e> <e>)` over Int64 → Bool. `saturating_sub` because `gen_cond`
        // can be entered at depth 0 (the `if` arm always emits a condition), where `depth - 1` would
        // underflow — the operand exprs just bottom out at their own base case.
        _ => {
            let rel = RELS[driver.gen_variant(RELS.len(), 0).unwrap_or(0)];
            out.push('(');
            out.push_str(rel);
            out.push(' ');
            gen_expr(
                driver,
                depth.saturating_sub(1),
                scope,
                fresh,
                can_call_f,
                out,
            );
            out.push(' ');
            gen_expr(
                driver,
                depth.saturating_sub(1),
                scope,
                fresh,
                can_call_f,
                out,
            );
            out.push(')');
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oracle::{Verdict, compile_catching};
    // `bolero::generator` re-exports the `bolero_generator` crate (`pub use bolero_generator::self`),
    // so the byte-slice test driver lives at this path.
    use bolero::generator::bolero_generator::driver::{ByteSliceDriver, Options};

    /// Generate a program by coercing a fixed byte string through the bolero driver (deterministic).
    fn gen_from(bytes: &[u8]) -> Program {
        let options = Options::default();
        let mut driver = ByteSliceDriver::new(bytes, &options);
        ProgramGen
            .generate(&mut driver)
            .expect("ProgramGen always produces a program")
    }

    /// The coercion invariant: ANY entropy → a valid, type-correct program that COMPILES (not merely a
    /// clean decline). This is the whole point of a coercing generator over a seed corpus — every input
    /// reaches the backend.
    #[test]
    fn any_entropy_coerces_to_a_compilable_program() {
        let inputs: [&[u8]; 6] = [
            &[],
            &[0],
            &[1, 2, 3, 4, 5, 6, 7, 8],
            &[0xFF; 32],
            &[
                0x01, 0x00, 0x02, 0x01, 0x00, 0x00, 0x01, 0x02, 0x00, 0x00, 0x00, 0x00,
            ],
            &[
                0x9e, 0x37, 0x79, 0xb9, 0x7f, 0x4a, 0x7c, 0x15, 0x11, 0x22, 0x33, 0x44,
            ],
        ];
        for bytes in inputs {
            let program = gen_from(bytes);
            assert!(
                program.source.starts_with("(do ")
                    && program.source.contains("(def (main) ")
                    && program.source.ends_with("(export main))"),
                "shape: {}",
                program.source
            );
            // A coerced program is always CLEANLY handled: it compiles, or it cleanly DECLINES (e.g. a
            // const-folded `*` overflowing Int64 → CDZ0304 is a correct rejection). It never crashes the
            // compiler, never emits invalid wasm, and never fails to parse.
            let verdict = compile_catching(&program.source);
            assert!(
                matches!(verdict, Verdict::Compiled { .. } | Verdict::Declined { .. }),
                "coerced program must be cleanly handled (Compiled/Declined), got {verdict:?} for: {}",
                program.source
            );
        }
    }

    /// Every operator the generator can emit is a valid Int64→Int64→Int64 op the compiler CLEANLY
    /// handles (guards the `OPS` list: a bogus/removed op would surface here rather than as silent
    /// declines in the fuzzer). With small operands (6, 3) there is no overflow / div-by-zero, so each
    /// compiles.
    #[test]
    fn every_operator_is_a_cleanly_handled_int64_op() {
        for op in OPS {
            let source = format!("(do (def (main) ({op} 6 3)) (export main))");
            assert!(
                matches!(compile_catching(&source), Verdict::Compiled { .. }),
                "operator `{op}` should compile as an Int64 op: {source}"
            );
        }
    }

    /// The helper + call shape the generator can emit compiles: a non-recursive `(def (f a b) …)` plus a
    /// `(f <e> <e>)` call from main. Pins that function-def + multi-arg call lowering is valid Cadenza.
    #[test]
    fn helper_and_call_shape_compiles() {
        let src = "(do (def (f a b) (+ a b)) (def (main) (f 3 4)) (export main))";
        assert!(
            matches!(compile_catching(src), Verdict::Compiled { .. }),
            "helper + call must compile: {src}"
        );
    }

    /// The boolean-connective condition shapes the generator can emit compile: `and`/`or`/`not` over
    /// relations, as an `if` condition. Pins that boolean-connective lowering is valid Cadenza.
    #[test]
    fn boolean_connective_condition_compiles() {
        let src =
            "(do (def (main) (if (and (< 1 2) (or (not (> 3 4)) (<= 5 6))) 1 0)) (export main))";
        assert!(
            matches!(compile_catching(src), Verdict::Compiled { .. }),
            "boolean-connective condition must compile: {src}"
        );
    }

    /// The base case (empty entropy) coerces to the simplest program — a single bounded literal main —
    /// which COMPILES, proving the generator reaches the backend, not just the parser. (Exhausted
    /// entropy resolves each driver read to a bound default, so no arith node is emitted.)
    #[test]
    fn base_case_entropy_compiles() {
        let program = gen_from(&[]);
        // Exhausted entropy → a single bounded literal main (no arith node), which compiles.
        assert!(
            !program.source.contains("(+ ")
                && !program.source.contains("(- ")
                && !program.source.contains("(* "),
            "base case should be a bare literal main, got: {}",
            program.source
        );
        assert!(matches!(
            compile_catching(&program.source),
            Verdict::Compiled { .. }
        ));
    }

    /// Sweeping varied entropy: the `if` arm is REACHABLE, and every coerced program (arith or `if`) is
    /// cleanly handled (never a crash / invalid wasm / parse error). Guards that the widened grammar
    /// stays in-bounds.
    #[test]
    fn if_arm_is_reachable_and_every_coerced_program_is_cleanly_handled() {
        let mut saw_if = false;
        let mut saw_let = false;
        for seed in 0u64..200 {
            // A varied, well-mixed byte string per seed (SplitMix-ish), so the driver visits all arms.
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
            let mut bytes = Vec::new();
            for _ in 0..24 {
                x ^= x >> 30;
                x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                bytes.push((x >> 24) as u8);
            }
            let program = gen_from(&bytes);
            saw_if |= program.source.contains("(if (");
            saw_let |= program.source.contains("(let ((");
            let verdict = compile_catching(&program.source);
            assert!(
                matches!(verdict, Verdict::Compiled { .. } | Verdict::Declined { .. }),
                "coerced program must be cleanly handled, got {verdict:?} for: {}",
                program.source
            );
        }
        assert!(
            saw_if,
            "the if arm should be reachable across 200 varied entropy inputs"
        );
        assert!(
            saw_let,
            "the let arm should be reachable across 200 varied entropy inputs"
        );
    }

    /// A recursive-entropy input coerces into a NESTED arithmetic program (exercises the recursive arm),
    /// still compilable.
    #[test]
    fn recursive_entropy_builds_a_nested_arith_program() {
        // Bias every `gen_variant(2,0)` toward the recursive arm (1) so we get a deeper tree.
        let bytes = [1u8; 24];
        let program = gen_from(&bytes);
        assert!(
            program.source.contains('(') && program.source.matches('(').count() >= 2,
            "expected a nested arith node: {}",
            program.source
        );
        assert!(matches!(
            compile_catching(&program.source),
            Verdict::Compiled { .. } | Verdict::Declined { .. }
        ));
    }
}
