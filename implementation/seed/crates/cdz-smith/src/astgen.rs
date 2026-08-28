//! A COERCING program generator (S6) — the operator's directed generation mechanism.
//!
//! Operator: *"the bolero type/value generator traits all try really really hard to COERCE a valid
//! value. We should use those instead of a seeded libfuzzer corpus — libfuzzer isn't going to get
//! value out of the seeded corpus with seeds."* So the driving mechanism maps ARBITRARY entropy → a
//! VALID Cadenza program (always succeeds, by construction), rather than mutating a corpus of seed
//! bytes the strict decode-gate mostly rejects.
//!
//! The grammar is written against a small [`Choice`] abstraction (`variant` + `int_bounded`), so it is
//! driver-agnostic and LIB-available: [`generate_coerced`] drives it from a plain `&[u8]` (usable by
//! the `lean-differential` subcommand and any non-test caller), while the coverage-guided bolero
//! `ValueGenerator` (behind `#[cfg(test)]`, since `bolero` is a dev-dependency) drives the SAME grammar
//! via a `Driver`→`Choice` adapter for the `cdz_smith_gen_never_panics` target. One grammar, two drivers.
//!
//! Grammar: `(do [ (def (f a b) …) ] (def (main) <body>) (export main))` where `<body>` is an Int64
//! expression — edge-biased literal | in-scope var | arithmetic (10 ops) | `(if <bool-cond> … …)` |
//! `let` | non-recursive helper call `(f e e)` — or a compound value `(tuple e e)` / `(list e e e)` of
//! Int64 elements. Kept type-correct Int64 throughout, so a generated program is cleanly HANDLED (it
//! compiles, or cleanly declines e.g. on a const-folded overflow) — never a crash / invalid wasm.

use core::fmt::Write as _;

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

/// The coercion the grammar reads from: pick a variant in `0..n` (arm 0 is the base/simplest, so
/// exhausted entropy bottoms out there) and a bounded `i64`. Infallible — an implementation must always
/// return a value (falling back to the simplest choice on exhaustion), which is what makes the generator
/// "coerce ANY entropy → a valid program".
pub trait Choice {
    /// A variant index in `0..n` (returns `0` when `n == 0`), biased toward `0` on exhaustion.
    fn variant(&mut self, n: usize) -> usize;
    /// An `i64` in `[min, max]` (returns `min` on exhaustion / an empty range).
    fn int_bounded(&mut self, min: i64, max: i64) -> i64;
}

/// A plain byte-cursor [`Choice`]: consumes the entropy `&[u8]` left to right, yielding `0` once spent
/// (so every choice bottoms out at its base and generation terminates). The LIB driver — no bolero.
struct ByteCursorChoice<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> ByteCursorChoice<'a> {
    fn new(bytes: &'a [u8]) -> ByteCursorChoice<'a> {
        ByteCursorChoice { bytes, pos: 0 }
    }
    fn byte(&mut self) -> u8 {
        let v = self.bytes.get(self.pos).copied().unwrap_or(0);
        self.pos = self.pos.saturating_add(1);
        v
    }
}

impl Choice for ByteCursorChoice<'_> {
    fn variant(&mut self, n: usize) -> usize {
        // Every `n` the grammar uses is < 256, so a single byte suffices.
        if n == 0 {
            0
        } else {
            (self.byte() as usize) % n
        }
    }
    fn int_bounded(&mut self, min: i64, max: i64) -> i64 {
        if min >= max {
            return min;
        }
        let mut v: u64 = 0;
        for _ in 0..8 {
            v = (v << 8) | self.byte() as u64;
        }
        let span = (max as i128 - min as i128 + 1) as u128;
        (min as i128 + (v as u128 % span) as i128) as i64
    }
}

/// Coerce an arbitrary entropy `&[u8]` into a valid Cadenza program (always succeeds). This is the
/// LIB entry the `lean-differential` subcommand and any non-test caller use.
pub fn generate_coerced(entropy: &[u8]) -> Program {
    build_program(&mut ByteCursorChoice::new(entropy))
}

/// Build a `(do [helper] (def (main) <body>) (export main))` program by making choices via `c`. Shared
/// by [`generate_coerced`] (byte cursor) and the bolero `ValueGenerator` (a `Driver`→`Choice` adapter).
fn build_program<C: Choice>(c: &mut C) -> Program {
    let mut source = String::from("(do ");
    let mut fresh = 0usize;
    // Optionally emit a NON-RECURSIVE helper `(def (f a b) <body>)`: its body uses the Int64 params
    // `a`/`b` but CANNOT call `f` (so it always terminates), and `main`'s body may then call
    // `(f <e> <e>)` — reaching multi-arg function-def + call lowering, kept total.
    let has_helper = c.variant(2) == 1;
    if has_helper {
        source.push_str("(def (f a b) ");
        let mut fscope = vec!["a".to_string(), "b".to_string()];
        gen_expr(c, MAX_DEPTH, &mut fscope, &mut fresh, false, &mut source);
        source.push_str(") ");
    }
    source.push_str("(def (main) ");
    let mut scope: Vec<String> = Vec::new();
    gen_main_body(c, &mut scope, &mut fresh, has_helper, &mut source);
    source.push_str(") (export main))");
    Program { source }
}

/// `main`'s body: an Int64 expression, a COMPOUND value built from Int64 sub-expressions — a
/// `(tuple <e> <e>)` or `(list <e> <e> <e>)` — or a BOOL value. Keeping the elements Int64 stays
/// type-safe without full type-directed generation, while reaching product/collection construction +
/// the compound value codec (a lowering surface a bare scalar body never exercises). The bool arm
/// returns a `Bool` from `main` (via [`gen_cond`]), exercising bool RETURN-value lowering + the bool
/// value codec — distinct from bool-as-`if`-condition, the only place a Bool appears otherwise.
fn gen_main_body<C: Choice>(
    c: &mut C,
    scope: &mut Vec<String>,
    fresh: &mut usize,
    can_call_f: bool,
    out: &mut String,
) {
    match c.variant(4) {
        // A BOOL-typed body: `main : Bool`. Reaches bool return-value lowering (bool-as-i32 result +
        // the bool value codec), a surface a scalar/compound Int64 body never hits.
        3 => gen_cond(c, MAX_DEPTH, scope, fresh, can_call_f, out),
        // (tuple <e> <e>) — a 2-tuple of Int64.
        1 => {
            out.push_str("(tuple ");
            gen_expr(c, MAX_DEPTH - 1, scope, fresh, can_call_f, out);
            out.push(' ');
            gen_expr(c, MAX_DEPTH - 1, scope, fresh, can_call_f, out);
            out.push(')');
        }
        // (list <e> <e> <e>) — a homogeneous Int64 list.
        2 => {
            out.push_str("(list ");
            gen_expr(c, MAX_DEPTH - 1, scope, fresh, can_call_f, out);
            out.push(' ');
            gen_expr(c, MAX_DEPTH - 1, scope, fresh, can_call_f, out);
            out.push(' ');
            gen_expr(c, MAX_DEPTH - 1, scope, fresh, can_call_f, out);
            out.push(')');
        }
        // A bare Int64 expression (the base case + exhaustion default).
        _ => gen_expr(c, MAX_DEPTH, scope, fresh, can_call_f, out),
    }
}

/// Append one coerced `Int64` expression: at `depth == 0` (or when the base variant is picked) an
/// integer literal / variable reference; otherwise arithmetic, an `if`, a `let`, or a helper call.
fn gen_expr<C: Choice>(
    c: &mut C,
    depth: usize,
    scope: &mut Vec<String>,
    fresh: &mut usize,
    can_call_f: bool,
    out: &mut String,
) {
    // At `depth == 0` force the base case (0); otherwise `variant` biases toward it — so generation
    // always terminates within the depth budget. The `(f …)` call arm is only offered when a helper is
    // in scope (`can_call_f`). Exhaustion → base case (0).
    let arms = if can_call_f { 5 } else { 4 };
    let variant = if depth == 0 { 0 } else { c.variant(arms) };
    match variant {
        // Binary arithmetic `(op <e> <e>)`.
        1 => {
            let op = OPS[c.variant(OPS.len())];
            out.push('(');
            out.push_str(op);
            out.push(' ');
            gen_expr(c, depth - 1, scope, fresh, can_call_f, out);
            out.push(' ');
            gen_expr(c, depth - 1, scope, fresh, can_call_f, out);
            out.push(')');
        }
        // Conditional `(if <cond> <e> <e>)` — `<cond>` is a Bool (relations + boolean connectives),
        // both branches Int64, so the whole `if` is Int64 and type-checks.
        2 => {
            out.push_str("(if ");
            gen_cond(c, depth - 1, scope, fresh, can_call_f, out);
            out.push(' ');
            gen_expr(c, depth - 1, scope, fresh, can_call_f, out);
            out.push(' ');
            gen_expr(c, depth - 1, scope, fresh, can_call_f, out);
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
            gen_expr(c, depth - 1, scope, fresh, can_call_f, out);
            out.push_str(")) ");
            scope.push(name);
            gen_expr(c, depth - 1, scope, fresh, can_call_f, out);
            scope.pop();
            out.push(')');
        }
        // Call the in-scope helper `(f <e> <e>)` — `f: Int64,Int64 -> Int64` is non-recursive + total, so
        // the call terminates. Only reachable when `can_call_f`. Reaches function-call lowering.
        4 => {
            out.push_str("(f ");
            gen_expr(c, depth - 1, scope, fresh, can_call_f, out);
            out.push(' ');
            gen_expr(c, depth - 1, scope, fresh, can_call_f, out);
            out.push(')');
        }
        // Base case: an in-scope Int64 variable reference (when any is bound and the driver picks it) —
        // which keeps the expression Int64 — else a bounded Int64 literal.
        _ => {
            if !scope.is_empty() && c.variant(2) == 1 {
                let idx = c.variant(scope.len());
                out.push_str(&scope[idx]);
            } else {
                // Bias toward boundary values (where width/overflow/wrap miscompiles cluster); else a
                // bounded random int. Both are valid Int64 literals.
                let n = if c.variant(2) == 1 {
                    INT_BOUNDARIES[c.variant(INT_BOUNDARIES.len())]
                } else {
                    c.int_bounded(-1_000_000, 1_000_000)
                };
                write!(out, "{n}").ok();
            }
        }
    }
}

/// Append one coerced Bool CONDITION (for an `if`): a base relation `(<rel> <e> <e>)` over Int64
/// sub-expressions, or a boolean connective `(and <c> <c>)` / `(or <c> <c>)` / `(not <c>)`. Reaches
/// boolean-connective + short-circuit lowering. Depth-bounded (base = a relation) so it terminates.
fn gen_cond<C: Choice>(
    c: &mut C,
    depth: usize,
    scope: &mut Vec<String>,
    fresh: &mut usize,
    can_call_f: bool,
    out: &mut String,
) {
    let variant = if depth == 0 { 0 } else { c.variant(4) };
    match variant {
        // `(and <c> <c>)` — short-circuit conjunction.
        1 => {
            out.push_str("(and ");
            gen_cond(c, depth - 1, scope, fresh, can_call_f, out);
            out.push(' ');
            gen_cond(c, depth - 1, scope, fresh, can_call_f, out);
            out.push(')');
        }
        // `(or <c> <c>)` — short-circuit disjunction.
        2 => {
            out.push_str("(or ");
            gen_cond(c, depth - 1, scope, fresh, can_call_f, out);
            out.push(' ');
            gen_cond(c, depth - 1, scope, fresh, can_call_f, out);
            out.push(')');
        }
        // `(not <c>)` — negation.
        3 => {
            out.push_str("(not ");
            gen_cond(c, depth - 1, scope, fresh, can_call_f, out);
            out.push(')');
        }
        // Base case: a relation `(<rel> <e> <e>)` over Int64 → Bool. `saturating_sub` because `gen_cond`
        // can be entered at depth 0 (the `if` arm always emits a condition), where `depth - 1` would
        // underflow — the operand exprs just bottom out at their own base case.
        _ => {
            let rel = RELS[c.variant(RELS.len())];
            out.push('(');
            out.push_str(rel);
            out.push(' ');
            gen_expr(c, depth.saturating_sub(1), scope, fresh, can_call_f, out);
            out.push(' ');
            gen_expr(c, depth.saturating_sub(1), scope, fresh, can_call_f, out);
            out.push(')');
        }
    }
}

// ── the bolero coverage-guided driver (dev-dependency, so behind `#[cfg(test)]`) ───────────────────
// A `Driver`→`Choice` adapter lets the bolero `ValueGenerator` drive the SAME grammar as
// `generate_coerced`, so `cargo bolero test cdz_smith_gen_never_panics` gets coverage-guided coercion.

#[cfg(test)]
use bolero::generator::ValueGenerator;
#[cfg(test)]
use bolero::generator::bolero_generator::Driver;

/// Adapts a bolero [`Driver`] to [`Choice`] (variant + bounded-int), so the grammar is driven by
/// bolero's coercing entropy under `cargo bolero` / `cargo test`.
#[cfg(test)]
struct BoleroChoice<'a, D: Driver>(&'a mut D);

#[cfg(test)]
impl<D: Driver> Choice for BoleroChoice<'_, D> {
    fn variant(&mut self, n: usize) -> usize {
        self.0.gen_variant(n, 0).unwrap_or(0)
    }
    fn int_bounded(&mut self, min: i64, max: i64) -> i64 {
        self.0
            .gen_i64(
                core::ops::Bound::Included(&min),
                core::ops::Bound::Included(&max),
            )
            .unwrap_or(min)
    }
}

/// A bolero [`ValueGenerator`] that coerces the driver's entropy into a valid program via the shared
/// grammar. Wire it with `check!().with_generator(ProgramGen)`.
#[cfg(test)]
pub struct ProgramGen;

#[cfg(test)]
impl ValueGenerator for ProgramGen {
    type Output = Program;

    fn generate<D: Driver>(&self, driver: &mut D) -> Option<Program> {
        // Infallible: the grammar always produces a valid program (Choice falls back to base choices on
        // exhaustion), so this never returns `None`.
        Some(build_program(&mut BoleroChoice(driver)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oracle::{Verdict, compile_catching};
    // `bolero::generator` re-exports the `bolero_generator` crate (`pub use bolero_generator::self`),
    // so the byte-slice test driver lives at this path.
    use bolero::generator::bolero_generator::driver::{ByteSliceDriver, Options};

    /// Generate a program by coercing a fixed byte string through the BOLERO driver (deterministic) —
    /// exercises the `Driver`→`Choice` adapter + the shared grammar, matching the `cargo bolero` path.
    fn gen_from(bytes: &[u8]) -> Program {
        let options = Options::default();
        let mut driver = ByteSliceDriver::new(bytes, &options);
        ProgramGen
            .generate(&mut driver)
            .expect("ProgramGen always produces a program")
    }

    /// The coercion invariant: ANY entropy → a valid, well-formed program the compiler CLEANLY handles
    /// (compiles, or a correct decline like a const-folded overflow) — never a crash / invalid wasm /
    /// parse error. Every input reaches the compiler.
    #[test]
    fn any_entropy_coerces_to_a_cleanly_handled_program() {
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
            let verdict = compile_catching(&program.source);
            assert!(
                matches!(verdict, Verdict::Compiled { .. } | Verdict::Declined { .. }),
                "coerced program must be cleanly handled (Compiled/Declined), got {verdict:?} for: {}",
                program.source
            );
        }
    }

    /// `generate_coerced` (the LIB byte-cursor path) coerces ANY entropy into a cleanly-handled program
    /// too — the same invariant as the bolero path, exercised through `ByteCursorChoice`.
    #[test]
    fn generate_coerced_lib_path_is_cleanly_handled() {
        for seed in 0u64..64 {
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
            let mut bytes = Vec::new();
            for _ in 0..24 {
                x ^= x >> 30;
                x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                bytes.push((x >> 24) as u8);
            }
            let program = generate_coerced(&bytes);
            assert!(
                program.source.starts_with("(do ") && program.source.ends_with("(export main))"),
                "shape: {}",
                program.source
            );
            let verdict = compile_catching(&program.source);
            assert!(
                matches!(verdict, Verdict::Compiled { .. } | Verdict::Declined { .. }),
                "generate_coerced program must be cleanly handled, got {verdict:?} for: {}",
                program.source
            );
        }
        // Empty entropy still yields a valid program (all choices bottom out).
        assert!(generate_coerced(&[]).source.ends_with("(export main))"));
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

    /// The compound `main`-body shapes the generator can emit compile: a `(tuple …)` and a `(list …)`
    /// of Int64 elements. Pins that product/collection construction from the coercing generator is valid.
    #[test]
    fn compound_main_body_shapes_compile() {
        for src in [
            "(do (def (main) (tuple 1 2)) (export main))",
            "(do (def (main) (list 1 2 3)) (export main))",
        ] {
            assert!(
                matches!(compile_catching(src), Verdict::Compiled { .. }),
                "compound main body must compile: {src}"
            );
        }
    }

    /// The bool-valued `main`-body shapes the generator can emit compile to VALID wasm: `main : Bool`
    /// from a relation and from boolean connectives. Pins that bool RETURN-value lowering (bool-as-i32
    /// result + the bool value codec) — distinct from bool-as-`if`-condition — is valid Cadenza.
    #[test]
    fn bool_main_body_shapes_compile() {
        for src in [
            "(do (def (main) (< 1 2)) (export main))",
            "(do (def (main) (and (< 1 2) (not (>= 3 4)))) (export main))",
        ] {
            assert!(
                matches!(compile_catching(src), Verdict::Compiled { .. }),
                "bool main body must compile to valid wasm: {src}"
            );
        }
    }

    /// The base case (empty entropy) coerces to the simplest program — a single bounded literal main —
    /// which COMPILES, proving the generator reaches the backend, not just the parser.
    #[test]
    fn base_case_entropy_compiles() {
        let program = gen_from(&[]);
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

    /// Sweeping varied entropy: the `if` and `let` arms are REACHABLE, and every coerced program is
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

    /// A recursive-entropy input coerces into a NESTED program (exercises the recursive arms), cleanly
    /// handled.
    #[test]
    fn recursive_entropy_builds_a_nested_arith_program() {
        let program = gen_from(&[1u8; 24]);
        assert!(
            program.source.matches('(').count() >= 2,
            "expected a nested node: {}",
            program.source
        );
        assert!(matches!(
            compile_catching(&program.source),
            Verdict::Compiled { .. } | Verdict::Declined { .. }
        ));
    }
}
