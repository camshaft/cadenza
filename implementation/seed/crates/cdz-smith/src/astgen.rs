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
//! Grammar: `(do [ (def (r n) …) ] [ (def (t n acc) …) ] [ (def (f a b) …) ] (def (main) <body>)
//! (export main))` where `<body>` is an Int64 expression — edge-biased literal | in-scope var |
//! arithmetic (10 ops) | `(if <bool-cond> … …)` | `let` | a SELF-operation reusing an in-scope var
//! `(op v v)` / `(rel v v)` (stresses self-identity folds + thunk sharing) | non-recursive helper call `(f e e)` |
//! non-tail recursive-helper call `(r <small-fuel>)` | tail-recursive-helper call `(t <small-fuel>
//! <seed>)` — a compound value `(tuple e e)` / `(list e e e)` of Int64 elements, or a Bool value (a
//! relation incl. `=`, boolean connectives, or a STRUCTURAL `(= <compound> <compound>)` equality). Kept
//! type-correct throughout, so a generated program is cleanly HANDLED (it compiles, or cleanly declines
//! e.g. on a const-folded overflow) — never a crash / invalid wasm / non-terminating run (both recursive
//! helpers are fuel-bounded + structurally decreasing).

use core::fmt::Write as _;

use crate::generator::Program;

/// The recursion budget for a generated expression (bounds program size + guarantees termination). At 5
/// this yields deeper nesting — more const-fold / CSE / sharing INTERACTIONS per program (the surface the
/// self-identity-fold miscompile lives in) — while staying small enough to compile+run fast. (Runtime
/// recursion via the `r`/`t` helpers is bounded SEPARATELY by their fuel literal, so depth ≠ run time.)
const MAX_DEPTH: usize = 5;

/// Int64 → Int64 → Int64 binary operators: arithmetic, division/modulo, and bitwise/shift. All
/// type-check as Int64; runtime edges (a const `*` overflow → CDZ decline; a `/`/`%` by zero → a
/// trap; a large/negative `<<`/`>>` count → shift-count masking) are runnable outcomes the compiler
/// handles cleanly, and the bitwise/shift lowering is a known Wasm-vs-Rust disagreement surface.
const OPS: [&str; 10] = ["+", "-", "*", "/", "%", "&", "|", "^", "<<", ">>"];

/// Int64 → Int64 → Bool relational operators for an `if` condition. Includes `=` (equality) alongside
/// the orderings; `=` also drives the reflexive-equality fold path via the `(= v v)` self-op. (`!=` is
/// not a valid Cadenza form — CDZ0101 — so it is deliberately absent.)
const RELS: [&str; 5] = ["<=", "<", ">=", ">", "="];

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

/// Which optional helpers are in scope for an expression, so the call arms (`gen_expr`) know what they
/// may emit. A `Copy` struct threaded by value — cheaper to extend with a new helper than a positional
/// `bool` per generator function.
#[derive(Clone, Copy)]
struct Caps {
    /// `(def (f a b) …)` — a NON-recursive 2-arg helper is in scope (call arm `(f <e> <e>)`).
    f: bool,
    /// `(def (r n) …)` — the NON-tail recursive helper is in scope (call arm `(r <fuel>)`).
    r: bool,
    /// `(def (t n acc) …)` — the TAIL-recursive accumulator helper is in scope (arm `(t <fuel> <seed>)`).
    t: bool,
}

/// Build a `(do [helpers] (def (main) <body>) (export main))` program by making choices via `c`. Shared
/// by [`generate_coerced`] (byte cursor) and the bolero `ValueGenerator` (a `Driver`→`Choice` adapter).
fn build_program<C: Choice>(c: &mut C) -> Program {
    let mut source = String::from("(do ");
    let mut fresh = 0usize;
    let caps = Caps {
        f: c.variant(2) == 1,
        r: c.variant(2) == 1,
        t: c.variant(2) == 1,
    };
    // NON-tail recursive helper. Shape is FIXED — `(def (r n) (if (<= n 0) <base> (<op> n (r (- n 1)))))`
    // — so the only recursive call is `(r (- n 1))`: structurally decreasing on `n`, `(<= n 0)`
    // base-guarded, hence total for ANY argument. Callers (`gen_expr`'s r-arm) pass only a SMALL bounded
    // fuel literal, so runtime depth stays tiny. Reaches SELF-recursive call lowering (corpus §"a do-local
    // function declaration is recursive": `(def (fac n) (if (= n 0) 1 (* n (fac (- n 1)))))`). `r`'s body
    // is self-contained (never calls `f`/`gen_expr` arms), so termination cannot be broken by the driver.
    if caps.r {
        let op = OPS[c.variant(OPS.len())];
        source.push_str("(def (r n) (if (<= n 0) ");
        gen_int_literal(c, &mut source);
        write!(source, " ({op} n (r (- n 1))))) ").ok();
    }
    // TAIL-recursive accumulator helper `(def (t n acc) (if (<= n 0) acc (t (- n 1) (<op> acc n))))`. The
    // recursive `(t …)` sits in TAIL position of the else-branch, so it reaches tail-call / loop lowering
    // (corpus "a tail-recursive counted loop") — a surface DISTINCT from `r`'s non-tail call. Also
    // structurally decreasing on `n` + `(<= n 0)` base-guarded → total. In the else-branch `n >= 1`, so
    // the accumulator op `(<op> acc n)` NEVER divides by zero — `t` is trap-free (all comparable values).
    if caps.t {
        let op = OPS[c.variant(OPS.len())];
        write!(
            source,
            "(def (t n acc) (if (<= n 0) acc (t (- n 1) ({op} acc n)))) "
        )
        .ok();
    }
    // NON-recursive 2-arg helper `(def (f a b) <body>)`: body uses `a`/`b` but CANNOT call `f` (total);
    // it MAY call `r`/`t` (bounded fuel → total). `main` may then call `(f <e> <e>)` — multi-arg
    // function-def + call lowering.
    if caps.f {
        source.push_str("(def (f a b) ");
        let mut fscope = vec!["a".to_string(), "b".to_string()];
        gen_expr(
            c,
            MAX_DEPTH,
            &mut fscope,
            &mut fresh,
            Caps { f: false, ..caps },
            &mut source,
        );
        source.push_str(") ");
    }
    source.push_str("(def (main) ");
    let mut scope: Vec<String> = Vec::new();
    gen_main_body(c, &mut scope, &mut fresh, caps, &mut source);
    source.push_str(") (export main))");
    Program { source }
}

/// Append one Int64 literal — biased toward boundary values (where width/overflow/wrap miscompiles
/// cluster), else a bounded random int. Both are valid Int64 literals. Shared by the leaf case and the
/// recursive helper's base.
fn gen_int_literal<C: Choice>(c: &mut C, out: &mut String) {
    let n = if c.variant(2) == 1 {
        INT_BOUNDARIES[c.variant(INT_BOUNDARIES.len())]
    } else {
        c.int_bounded(-1_000_000, 1_000_000)
    };
    write!(out, "{n}").ok();
}

/// Append one COMPOUND value of Int64 elements: a `(tuple <e> <e>)` (`is_list=false`) or a homogeneous
/// `(list <e> <e> <e>)` (`is_list=true`). Shared by `main`'s compound body and the structural
/// `(= <compound> <compound>)` equality arm (both sides built with the SAME `is_list` → type-correct).
fn gen_compound<C: Choice>(
    c: &mut C,
    is_list: bool,
    depth: usize,
    scope: &mut Vec<String>,
    fresh: &mut usize,
    caps: Caps,
    out: &mut String,
) {
    if is_list {
        out.push_str("(list ");
        gen_expr(c, depth, scope, fresh, caps, out);
        out.push(' ');
        gen_expr(c, depth, scope, fresh, caps, out);
        out.push(' ');
        gen_expr(c, depth, scope, fresh, caps, out);
        out.push(')');
    } else {
        out.push_str("(tuple ");
        gen_expr(c, depth, scope, fresh, caps, out);
        out.push(' ');
        gen_expr(c, depth, scope, fresh, caps, out);
        out.push(')');
    }
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
    caps: Caps,
    out: &mut String,
) {
    match c.variant(4) {
        // A BOOL-typed body: `main : Bool`. Reaches bool return-value lowering (bool-as-i32 result +
        // the bool value codec), a surface a scalar/compound Int64 body never hits.
        3 => gen_cond(c, MAX_DEPTH, scope, fresh, caps, out),
        // (tuple <e> <e>) — a 2-tuple of Int64.
        1 => gen_compound(c, false, MAX_DEPTH - 1, scope, fresh, caps, out),
        // (list <e> <e> <e>) — a homogeneous Int64 list.
        2 => gen_compound(c, true, MAX_DEPTH - 1, scope, fresh, caps, out),
        // A bare Int64 expression (the base case + exhaustion default).
        _ => gen_expr(c, MAX_DEPTH, scope, fresh, caps, out),
    }
}

/// Append one coerced `Int64` expression: at `depth == 0` (or when the base variant is picked) an
/// integer literal / variable reference; otherwise arithmetic, an `if`, a `let`, or a helper call.
fn gen_expr<C: Choice>(
    c: &mut C,
    depth: usize,
    scope: &mut Vec<String>,
    fresh: &mut usize,
    caps: Caps,
    out: &mut String,
) {
    // At `depth == 0` force the base case (0); otherwise `variant` biases toward it — so generation
    // always terminates within the depth budget. The helper-call arms (`f`, then `r`, then `t`) are only
    // offered when the respective helper is in scope; when present they occupy the arms ABOVE the fixed
    // 0..3, in that order. Exhaustion → base case (0).
    let f_arm = if caps.f { Some(4) } else { None };
    let r_arm = if caps.r {
        Some(4 + caps.f as usize)
    } else {
        None
    };
    let t_arm = if caps.t {
        Some(4 + caps.f as usize + caps.r as usize)
    } else {
        None
    };
    let arms = 4 + caps.f as usize + caps.r as usize + caps.t as usize;
    let variant = if depth == 0 { 0 } else { c.variant(arms) };
    // Call the in-scope helper `(f <e> <e>)` — `f: Int64,Int64 -> Int64` is non-recursive + total, so
    // the call terminates. Reaches multi-arg function-call lowering.
    if Some(variant) == f_arm {
        out.push_str("(f ");
        gen_expr(c, depth - 1, scope, fresh, caps, out);
        out.push(' ');
        gen_expr(c, depth - 1, scope, fresh, caps, out);
        out.push(')');
        return;
    }
    // Call the NON-tail recursive helper `(r <fuel>)` with a SMALL bounded fuel LITERAL (0..12), so it
    // recurses at most ~12 deep and terminates fast — NEVER an arbitrary expr (which could recurse a
    // million deep). Reaches self-recursive call lowering.
    if Some(variant) == r_arm {
        let fuel = c.int_bounded(0, 12);
        write!(out, "(r {fuel})").ok();
        return;
    }
    // Call the TAIL-recursive helper `(t <fuel> <seed>)`: a small bounded fuel LITERAL (0..12) + an Int64
    // seed literal for the accumulator. Reaches tail-call / loop lowering (distinct from the `r` arm).
    if Some(variant) == t_arm {
        let fuel = c.int_bounded(0, 12);
        write!(out, "(t {fuel} ").ok();
        gen_int_literal(c, out);
        out.push(')');
        return;
    }
    match variant {
        // Binary arithmetic `(op <e> <e>)`.
        1 => {
            let op = OPS[c.variant(OPS.len())];
            out.push('(');
            out.push_str(op);
            out.push(' ');
            gen_expr(c, depth - 1, scope, fresh, caps, out);
            out.push(' ');
            gen_expr(c, depth - 1, scope, fresh, caps, out);
            out.push(')');
        }
        // Conditional `(if <cond> <e> <e>)` — `<cond>` is a Bool (relations + boolean connectives),
        // both branches Int64, so the whole `if` is Int64 and type-checks.
        2 => {
            out.push_str("(if ");
            gen_cond(c, depth - 1, scope, fresh, caps, out);
            out.push(' ');
            // Sometimes emit IDENTICAL branches — `(if C a a)`. If the compiler folds an identical-branch
            // `if` to `a` WITHOUT evaluating `C`, then a TRAPPING condition's effect is ELIDED — the same
            // fold-soundness family as the self-identity-fold miscompile (#4870), but on the if-CONDITION
            // path, and `C` here is a DIRECT expr (a distinct `is_trap_free` path from a LocalRef). The
            // branch is generated ONCE and duplicated verbatim so the two arms are syntactically equal.
            if c.variant(3) == 0 {
                let mut branch = String::new();
                gen_expr(c, depth - 1, scope, fresh, caps, &mut branch);
                out.push_str(&branch);
                out.push(' ');
                out.push_str(&branch);
            } else {
                gen_expr(c, depth - 1, scope, fresh, caps, out);
                out.push(' ');
                gen_expr(c, depth - 1, scope, fresh, caps, out);
            }
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
            gen_expr(c, depth - 1, scope, fresh, caps, out);
            out.push_str(")) ");
            scope.push(name);
            gen_expr(c, depth - 1, scope, fresh, caps, out);
            scope.pop();
            out.push(')');
        }
        // Base case: an in-scope Int64 variable reference (when any is bound and the driver picks it) —
        // which keeps the expression Int64 — else a bounded Int64 literal.
        _ => {
            if !scope.is_empty() && c.variant(2) == 1 {
                let idx = c.variant(scope.len());
                // Sometimes emit a SELF-operation `(op v v)` REUSING the same in-scope var, rather than a
                // bare `v`. No recursion (operands are the var), so it stays a depth-0-safe leaf. This
                // densely stresses the const-fold-soundness surface — self-identity folds (`v - v`→0,
                // `v ^ v`→0, …), CSE, and thunk SHARING of the bound value: if the binding traps on force,
                // a fold that drops the operand must not elide the trap (cf. the self-identity-fold
                // miscompile the L2 differential found).
                if c.variant(2) == 1 {
                    let op = OPS[c.variant(OPS.len())];
                    let v = &scope[idx];
                    write!(out, "({op} {v} {v})").ok();
                } else {
                    out.push_str(&scope[idx]);
                }
            } else {
                gen_int_literal(c, out);
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
    caps: Caps,
    out: &mut String,
) {
    let variant = if depth == 0 { 0 } else { c.variant(5) };
    match variant {
        // Structural EQUALITY of two same-shaped COMPOUND values — `(= (tuple e e) (tuple e e))` or
        // `(= (list e e e) (list e e e))`. Reaches the structural (recursive, heap) equality lowering —
        // a surface scalar relations never hit. Both sides share `is_list` so the comparison type-checks.
        4 => {
            let is_list = c.variant(2) == 1;
            out.push_str("(= ");
            gen_compound(c, is_list, depth - 1, scope, fresh, caps, out);
            out.push(' ');
            gen_compound(c, is_list, depth - 1, scope, fresh, caps, out);
            out.push(')');
        }
        // `(and <c> <c>)` — short-circuit conjunction.
        1 => {
            out.push_str("(and ");
            gen_cond(c, depth - 1, scope, fresh, caps, out);
            out.push(' ');
            gen_cond(c, depth - 1, scope, fresh, caps, out);
            out.push(')');
        }
        // `(or <c> <c>)` — short-circuit disjunction.
        2 => {
            out.push_str("(or ");
            gen_cond(c, depth - 1, scope, fresh, caps, out);
            out.push(' ');
            gen_cond(c, depth - 1, scope, fresh, caps, out);
            out.push(')');
        }
        // `(not <c>)` — negation.
        3 => {
            out.push_str("(not ");
            gen_cond(c, depth - 1, scope, fresh, caps, out);
            out.push(')');
        }
        // Base case: a relation `(<rel> <e> <e>)` over Int64 → Bool. `saturating_sub` because `gen_cond`
        // can be entered at depth 0 (the `if` arm always emits a condition), where `depth - 1` would
        // underflow — the operand exprs just bottom out at their own base case.
        _ => {
            let rel = RELS[c.variant(RELS.len())];
            // Sometimes compare the SAME in-scope var to itself — `(rel v v)` — the exact shape that
            // exposes self-identity RELATIONAL folds (`(< v v)`→false, `(<= v v)`→true, …). If `v`'s
            // binding traps on force, folding the relation to a constant that drops `v` must NOT elide
            // the trap (the confirmed self-identity-fold miscompile). Else two independent operands.
            if !scope.is_empty() && c.variant(3) == 0 {
                let v = &scope[c.variant(scope.len())];
                write!(out, "({rel} {v} {v})").ok();
            } else {
                out.push('(');
                out.push_str(rel);
                out.push(' ');
                gen_expr(c, depth.saturating_sub(1), scope, fresh, caps, out);
                out.push(' ');
                gen_expr(c, depth.saturating_sub(1), scope, fresh, caps, out);
                out.push(')');
            }
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

    /// The terminating recursive-helper shapes the generator can emit compile to VALID wasm: a
    /// `(def (r n) (if (<= n 0) <base> (<op> n (r (- n 1)))))` called with a small fuel literal. Mirrors
    /// the corpus §"a do-local function declaration is recursive" shape. Pins that SELF-recursive call
    /// lowering is valid Cadenza (a surface no other generator arm reaches).
    #[test]
    fn recursive_helper_shape_compiles() {
        for src in [
            // + accumulation — a plain counted sum, cannot trap.
            "(do (def (r n) (if (<= n 0) 0 (+ n (r (- n 1))))) (def (main) (r 5)) (export main))",
            // The exact corpus `fac` shape (multiply), called with a small fuel.
            "(do (def (r n) (if (<= n 0) 1 (* n (r (- n 1))))) (def (main) (r 5)) (export main))",
            // Called with fuel 0 — hits the base case immediately.
            "(do (def (r n) (if (<= n 0) 7 (- n (r (- n 1))))) (def (main) (r 0)) (export main))",
        ] {
            assert!(
                matches!(
                    compile_catching(src),
                    Verdict::Compiled { .. } | Verdict::Declined { .. }
                ),
                "recursive helper shape must be cleanly handled: {src}"
            );
        }
    }

    /// Sweeping varied entropy: the recursive-helper arm (`(r <fuel>)`) is REACHABLE, and every coerced
    /// program that emits it is cleanly handled (compiles / declines — never a crash / invalid wasm /
    /// parse error, and never a non-terminating run). Guards the terminating-recursion widening.
    #[test]
    fn recursive_arm_is_reachable_and_cleanly_handled() {
        let mut saw_rec = false;
        for seed in 0u64..400 {
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
            let mut bytes = Vec::new();
            for _ in 0..24 {
                x ^= x >> 30;
                x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                bytes.push((x >> 24) as u8);
            }
            let program = gen_from(&bytes);
            if program.source.contains("(def (r n)") {
                saw_rec = true;
                let verdict = compile_catching(&program.source);
                assert!(
                    matches!(verdict, Verdict::Compiled { .. } | Verdict::Declined { .. }),
                    "recursive-helper program must be cleanly handled, got {verdict:?} for: {}",
                    program.source
                );
            }
        }
        assert!(
            saw_rec,
            "the recursive-helper arm should be reachable across 400 varied entropy inputs"
        );
    }

    /// The TAIL-recursive accumulator helper shapes compile to VALID wasm: `(def (t n acc) (if (<= n 0)
    /// acc (t (- n 1) (<op> acc n))))` called with a small fuel + seed. Pins that tail-position recursive
    /// call lowering (the corpus "tail-recursive counted loop" surface) is valid Cadenza — distinct from
    /// the non-tail `r`. In the else-branch `n >= 1`, so `/`/`%` never divide by zero (trap-free).
    #[test]
    fn tail_recursive_shape_compiles() {
        for src in [
            "(do (def (t n acc) (if (<= n 0) acc (t (- n 1) (+ acc n)))) (def (main) (t 5 0)) (export main))",
            "(do (def (t n acc) (if (<= n 0) acc (t (- n 1) (* acc n)))) (def (main) (t 5 1)) (export main))",
            // `/` in the accumulator — safe because n >= 1 in the recursive branch (no div-by-zero).
            "(do (def (t n acc) (if (<= n 0) acc (t (- n 1) (/ acc n)))) (def (main) (t 4 100)) (export main))",
            // fuel 0 → base case immediately, returns the seed.
            "(do (def (t n acc) (if (<= n 0) acc (t (- n 1) (- acc n)))) (def (main) (t 0 9)) (export main))",
        ] {
            assert!(
                matches!(compile_catching(src), Verdict::Compiled { .. }),
                "tail-recursive helper shape must compile to valid wasm: {src}"
            );
        }
    }

    /// Sweeping varied entropy: the tail-recursive arm (`(t <fuel> <seed>)`) is REACHABLE, and every
    /// coerced program that emits it is cleanly handled (compiles / declines — never crash / invalid wasm
    /// / parse error, and never a non-terminating run). Guards the tail-recursion widening.
    #[test]
    fn tail_recursive_arm_is_reachable_and_cleanly_handled() {
        let mut saw_tail = false;
        for seed in 0u64..400 {
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
            let mut bytes = Vec::new();
            for _ in 0..24 {
                x ^= x >> 30;
                x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                bytes.push((x >> 24) as u8);
            }
            let program = gen_from(&bytes);
            if program.source.contains("(def (t n acc)") {
                saw_tail = true;
                let verdict = compile_catching(&program.source);
                assert!(
                    matches!(verdict, Verdict::Compiled { .. } | Verdict::Declined { .. }),
                    "tail-recursive program must be cleanly handled, got {verdict:?} for: {}",
                    program.source
                );
            }
        }
        assert!(
            saw_tail,
            "the tail-recursive arm should be reachable across 400 varied entropy inputs"
        );
    }

    /// Self-operations on a bound var — `(op v v)` / `(rel v v)`, the same in-scope name reused for both
    /// operands — are REACHABLE across varied entropy, and every program that emits one is cleanly
    /// handled. Guards the variable-reuse widening that stresses the const-fold-soundness surface.
    #[test]
    fn self_operations_on_bound_vars_are_reachable_and_cleanly_handled() {
        // A doubled var token `vK vK` (same name, space-separated) is the self-operation signature.
        fn has_self_op(src: &str) -> bool {
            let toks: Vec<&str> = src.split(['(', ')', ' ']).collect();
            toks.windows(2).any(|w| {
                let t = w[0];
                !t.is_empty()
                    && t == w[1]
                    && t.starts_with('v')
                    && t[1..].chars().all(|c| c.is_ascii_digit())
                    && t.len() > 1
            })
        }
        let mut saw_self_op = false;
        for seed in 0u64..400 {
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
            let mut bytes = Vec::new();
            for _ in 0..24 {
                x ^= x >> 30;
                x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                bytes.push((x >> 24) as u8);
            }
            let program = gen_from(&bytes);
            if has_self_op(&program.source) {
                saw_self_op = true;
                let verdict = compile_catching(&program.source);
                assert!(
                    matches!(verdict, Verdict::Compiled { .. } | Verdict::Declined { .. }),
                    "self-op program must be cleanly handled, got {verdict:?} for: {}",
                    program.source
                );
            }
        }
        assert!(
            saw_self_op,
            "a self-operation `(op v v)`/`(rel v v)` should be reachable across 400 varied inputs"
        );
    }

    /// The identical-branch `if` shapes the generator can emit are cleanly handled — a plain `(if C a a)`
    /// and one whose condition can TRAP at runtime (`(if (< (r 2) 5) a a)`, r divide-by-zero). Pins that
    /// the identical-branch fold surface (which must preserve a trapping condition's effect) is valid
    /// Cadenza the compiler handles (compiles or a correct trap-decline), never a crash / invalid wasm.
    #[test]
    fn identical_branch_if_shapes_are_cleanly_handled() {
        for src in [
            "(do (def (main) (if (< 1 2) 5 5)) (export main))",
            "(do (def (r n) (if (<= n 0) -9223372036854775808 (/ n (r (- n 1))))) (def (main) (if (< (r 2) 5) 7 7)) (export main))",
        ] {
            assert!(
                matches!(
                    compile_catching(src),
                    Verdict::Compiled { .. } | Verdict::Declined { .. }
                ),
                "identical-branch if must be cleanly handled: {src}"
            );
        }
    }

    /// Scalar `=` and STRUCTURAL compound `=` shapes the generator can emit compile: Int64 equality as an
    /// `if` condition, and `(= (tuple …) (tuple …))` / `(= (list …) (list …))` structural equality. Pins
    /// that equality (incl. the recursive/heap compound-equality lowering) is valid Cadenza the compiler
    /// handles — `!=` is deliberately NOT generated (invalid form, CDZ0101).
    #[test]
    fn equality_and_compound_equality_shapes_compile() {
        for src in [
            "(do (def (main) (if (= 3 3) 1 0)) (export main))",
            "(do (def (main) (if (= (tuple 1 2) (tuple 3 4)) 1 0)) (export main))",
            "(do (def (main) (if (= (list 1 2 3) (list 1 2 3)) 1 0)) (export main))",
        ] {
            assert!(
                matches!(compile_catching(src), Verdict::Compiled { .. }),
                "equality shape must compile: {src}"
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
