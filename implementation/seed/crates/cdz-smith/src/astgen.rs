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
//! Grammar: `(do [ (def (r n) …) ] [ (def (t n acc) …) ] [ (def (f a b) …) ] (def (main [ (: v0 <heap>) |
//! (: n Int64) ]) <body>) (export main))` — `main` OPTIONALLY takes either one heap/reference-typed param
//! (`String`/`Bytes`/`(List Int64)`/`(Option Int64)`, left unused) to exercise the exported-entry heap-param
//! ABI path (the bucket-1 emit miscompile fixed by rcdzc #4961), OR a RUNTIME `(: n Int64)` param put
//! IN SCOPE so the Int64 body depends on a non-const-foldable input (runtime-dependent programs; an `if`/
//! `match` on `n` keeps both arms live — no dead-branch-elim masking the join/emit surface). `<body>` is an Int64 expression — edge-biased literal | in-scope var |
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
    // Optionally give the EXPORTED entry `main` a HEAP/reference-typed parameter (`v0`). This exercises
    // the exported-entry heap-param ABI lowering path (lift-op import appends + def-index shifting) —
    // where the bucket-1 miscompile lived (a heap-param entry + a reachable recursive fn emitted the
    // recursive call's result at the wrong width; fixed by rcdzc #4961). Generating this shape gives the
    // coercing generator an ONGOING regression guard for that path + broader entry-ABI coverage. The
    // param is left UNUSED (not added to `scope`) so the body stays well-typed regardless (an in-scope
    // heap var used in the Int64 body would be a type conflict); only the accepted entry-param types are
    // drawn (String/Bytes/(List Int64)/(Option Int64) cross the plain export boundary — Tuple/Record/Unit
    // decline there, which is still "cleanly handled" but pointless), so the program COMPILES (post-#4961)
    // or cleanly declines.
    // Also: sometimes give `main` a RUNTIME `(: n Int64)` param and put `n` IN SCOPE. `n` is a genuine
    // entry input the compiler cannot const-fold, so the Int64 grammar (`gen_expr`/`gen_cond`/`gen_compound`
    // reference it) becomes RUNTIME-DEPENDENT — programs whose values depend on `n` are NOT collapsed by
    // const-folding, and an `if`/`match` on `n` keeps BOTH arms live (no dead-branch-elim). That un-masks
    // the join/emit surface: a const condition/scrutinee lets dead-branch-elim ELIDE the mismatched arm
    // (the masking that hid the float-widen class in probes) — a runtime `n` keeps the join actually
    // emitted. Broader emit coverage + the runtime-scrutinee foundation the aggressive mixed-type mode needs.
    let mut scope: Vec<String> = Vec::new();
    match c.variant(3) {
        0 => {
            let ty = HEAP_PARAM_TYPES[c.variant(HEAP_PARAM_TYPES.len())];
            write!(source, "(def (main (: v0 {ty})) ").ok();
        }
        1 => {
            source.push_str("(def (main (: n Int64)) ");
            scope.push("n".to_string()); // `n` is a runtime Int64 in scope → runtime-dependent programs
        }
        _ => {
            source.push_str("(def (main) ");
        }
    }
    gen_main_body(c, &mut scope, &mut fresh, caps, &mut source);
    source.push_str(") (export main))");
    Program { source }
}

/// Heap/reference-typed entry-parameter types that cross the plain export boundary (so `main` with one of
/// these + a reachable recursive fn reaches the entry-ABI path that #4961 fixed). Scalars (Int64/Bool)
/// don't perturb the path; Tuple/Record/Unit decline at the boundary.
const HEAP_PARAM_TYPES: &[&str] = &["String", "Bytes", "(List Int64)", "(Option Int64)"];

/// SIZED integer types (the generator is otherwise Int64-only). A `main : T` body over one of these
/// reaches narrow-width value lowering + the checked-conversion ops — a surface an Int64 body never hits.
const SIZED_INT_TYPES: &[&str] = &[
    "Int8", "Int16", "Int32", "Int64", "UInt8", "UInt16", "UInt32", "UInt64",
];

/// Width-SAFE binary ops for the sized-int body: with two `0..=9` operands the result stays in range for
/// EVERY width (incl. UInt8) — no const overflow/underflow — so the program COMPILES (reaching narrow
/// arith emit) rather than declining CDZ0304. Deliberately excludes `-` (unsigned underflow), `/`/`%`
/// (div-by-zero), and shifts (overflow) to keep the coverage arm on the compile path.
const SIZED_INT_OPS: &[&str] = &["+", "*", "&", "|", "^"];

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
/// `(tuple <e> <e>)` or `(list <e> <e> <e>)` — a BOOL value, or a SIZED-int value. Keeping the elements
/// Int64 stays type-safe without full type-directed generation, while reaching product/collection
/// construction + the compound value codec (a lowering surface a bare scalar body never exercises). The
/// bool arm returns a `Bool` from `main` (via [`gen_cond`]), exercising bool RETURN-value lowering + the
/// bool value codec — distinct from bool-as-`if`-condition, the only place a Bool appears otherwise. The
/// sized-int arm ([`gen_sized_int_body`]) returns an `Int8`/…/`UInt64` value, reaching narrow-width value
/// lowering + the checked-conversion ops that the Int64-only expression grammar never hits; the float arm
/// ([`gen_float_body`]) returns a `Float64`/`Float32` value, reaching float value/arith/compare/if-join/let
/// lowering (uniform-width, so it stays on the compile path); the type-diverse-compound arm
/// ([`gen_typed_compound`]) returns a HETEROGENEOUS tuple / non-Int64 list / heterogeneous record /
/// `Option` / annotated `Result` — the type-directed step past the Int64-element compounds `gen_compound`
/// builds (named-record + Option/Result sum value lowering); the typed-fn arm ([`gen_typed_fn_call_body`])
/// defines + calls a locally-typed function (typed param/return/call across scalar types).
fn gen_main_body<C: Choice>(
    c: &mut C,
    scope: &mut Vec<String>,
    fresh: &mut usize,
    caps: Caps,
    out: &mut String,
) {
    match c.variant(9) {
        // A BOOL-typed body: `main : Bool`. Reaches bool return-value lowering (bool-as-i32 result +
        // the bool value codec), a surface a scalar/compound Int64 body never hits.
        3 => gen_cond(c, MAX_DEPTH, scope, fresh, caps, out),
        // (tuple <e> <e>) — a 2-tuple of Int64.
        1 => gen_compound(c, false, MAX_DEPTH - 1, scope, fresh, caps, out),
        // (list <e> <e> <e>) — a homogeneous Int64 list.
        2 => gen_compound(c, true, MAX_DEPTH - 1, scope, fresh, caps, out),
        // A SIZED-int-typed body (`main : Int8`/…): narrow-width value lowering + checked conversions.
        4 => gen_sized_int_body(c, out),
        // A FLOAT-typed body (`main : Float64`/`Float32`): float value/arith/compare/if-join/let lowering.
        5 => gen_float_body(c, fresh, out),
        // A TYPE-DIVERSE compound: a heterogeneous tuple / a non-Int64 homogeneous list (type-directed).
        6 => gen_typed_compound(c, COMPOUND_DEPTH, fresh, out),
        // A TYPED local function def + call `(do (def (g (: x T)) …) (g <T-expr>))`: typed param/return/call.
        7 => gen_typed_fn_call_body(c, fresh, out),
        // BUILD + CONSUME a compound (projection / List.len / Option match): consumption emit.
        8 => gen_compound_consume(c, out),
        // A bare Int64 expression (the base case + exhaustion default).
        _ => gen_expr(c, MAX_DEPTH, scope, fresh, caps, out),
    }
}

/// Append a SIZED-integer-typed expression (`main : T` for a `T` in [`SIZED_INT_TYPES`]) — self-contained
/// (no Int64 scope), so it stays type-correct without type-directed generation. One of: an ascribed
/// literal `(: <n> T)`; a checked conversion `(T.of <n>)` from a small in-range Int64 literal; or a
/// width-SAFE binary op `(<op> (: a T) (: b T))` over two `0..=9` operands (see [`SIZED_INT_OPS`] — the
/// result stays in range for every width, so it COMPILES). Reaches narrow-width arith/conversion emit.
fn gen_sized_int_body<C: Choice>(c: &mut C, out: &mut String) {
    let t = SIZED_INT_TYPES[c.variant(SIZED_INT_TYPES.len())];
    match c.variant(3) {
        0 => write!(out, "(: {} {t})", c.int_bounded(0, 9)).ok(),
        1 => write!(out, "({t}.of {})", c.int_bounded(0, 9)).ok(),
        _ => {
            let op = SIZED_INT_OPS[c.variant(SIZED_INT_OPS.len())];
            write!(
                out,
                "({op} (: {} {t}) (: {} {t}))",
                c.int_bounded(0, 9),
                c.int_bounded(0, 9)
            )
            .ok()
        }
    };
}

/// Float binary operators — all TOTAL on floats (a `/ 0.0` is `inf`/`nan`, never a trap), so a generated
/// float expression stays on the COMPILE path.
const FLOAT_OPS: [&str; 4] = ["+", "-", "*", "/"];

/// Append one non-negative float LITERAL of a single width: a bare `N.0` for Float64, an ascribed
/// `(: N.0 Float32)` for Float32 (a bare float literal defaults to Float64, so an f32 slot must ascribe).
fn gen_float_lit<C: Choice>(c: &mut C, is_f32: bool, out: &mut String) {
    let whole = c.int_bounded(0, 1000);
    if is_f32 {
        write!(out, "(: {whole}.0 Float32)").ok();
    } else {
        write!(out, "{whole}.0").ok();
    }
}

/// Append a UNIFORM float expression of ONE width (`is_f32` → Float32, else Float64): a literal, a binary
/// op, an `if` (float-relation condition + two same-width arms), or a `let`. Type-correct by construction
/// — EVERY leaf and arm is the same float type — so an `if`/`match` join never mixes widths (which would
/// hit the open match-emit-widen gap); it COMPILES, exercising float value / arith / compare / if-join /
/// let lowering that the Int64-only expression grammar never reached.
fn gen_float<C: Choice>(
    c: &mut C,
    is_f32: bool,
    depth: usize,
    fresh: &mut usize,
    out: &mut String,
) {
    if depth == 0 {
        gen_float_lit(c, is_f32, out);
        return;
    }
    match c.variant(4) {
        // A binary op over two same-width float sub-expressions.
        1 => {
            let op = FLOAT_OPS[c.variant(FLOAT_OPS.len())];
            write!(out, "({op} ").ok();
            gen_float(c, is_f32, depth - 1, fresh, out);
            out.push(' ');
            gen_float(c, is_f32, depth - 1, fresh, out);
            out.push(')');
        }
        // (if (<rel> <float> <float>) <float> <float>) — a float-relation condition + two same-width arms.
        2 => {
            let rel = RELS[c.variant(RELS.len())];
            write!(out, "(if ({rel} ").ok();
            gen_float(c, is_f32, depth - 1, fresh, out);
            out.push(' ');
            gen_float(c, is_f32, depth - 1, fresh, out);
            out.push_str(") ");
            gen_float(c, is_f32, depth - 1, fresh, out);
            out.push(' ');
            gen_float(c, is_f32, depth - 1, fresh, out);
            out.push(')');
        }
        // (let ((vN <float>)) (<op> vN <float>)) — a float-typed binding used in a float op.
        3 => {
            let v = *fresh;
            *fresh += 1;
            let op = FLOAT_OPS[c.variant(FLOAT_OPS.len())];
            write!(out, "(let ((v{v} ").ok();
            gen_float(c, is_f32, depth - 1, fresh, out);
            write!(out, ")) ({op} v{v} ").ok();
            gen_float(c, is_f32, depth - 1, fresh, out);
            out.push_str("))");
        }
        // Base case + exhaustion default: a literal.
        _ => gen_float_lit(c, is_f32, out),
    }
}

/// A FLOAT-typed body (`main : Float64`/`Float32`) via [`gen_float`] — the coercing generator was
/// otherwise Int64/Bool/compound only. Reaches float value/arith/compare/if-join/let lowering. Kept
/// uniform-width (see [`gen_float`]) so it stays on the compile path (the mixed-width match-arm widen is
/// v-rb's open emit bug; a later increment adds mixed-width arms once that lands + surfaces as findings).
fn gen_float_body<C: Choice>(c: &mut C, fresh: &mut usize, out: &mut String) {
    let is_f32 = c.variant(2) == 1;
    gen_float(c, is_f32, MAX_DEPTH, fresh, out);
}

/// A scalar type the TYPE-DIRECTED generator can emit a value of (increment 1 of the type-directed
/// program: the coercing grammar was Int64-centric, so compounds were Int64-only). `Sized` carries a
/// [`SIZED_INT_TYPES`] name (Int8/…/UInt64).
#[derive(Clone, Copy)]
enum ScalarTy {
    Int64,
    Float64,
    Float32,
    Sized(&'static str),
    Bool,
}

impl ScalarTy {
    /// The Cadenza type name — for a type annotation (e.g. a `Result` payload) that a bare value needs.
    fn name(self) -> &'static str {
        match self {
            ScalarTy::Int64 => "Int64",
            ScalarTy::Float64 => "Float64",
            ScalarTy::Float32 => "Float32",
            ScalarTy::Bool => "Bool",
            ScalarTy::Sized(t) => t,
        }
    }
}

/// Pick a scalar type uniformly (Int64 / Float64 / Float32 / Bool / a sized int).
fn pick_scalar_ty<C: Choice>(c: &mut C) -> ScalarTy {
    match c.variant(5) {
        0 => ScalarTy::Int64,
        1 => ScalarTy::Float64,
        2 => ScalarTy::Float32,
        3 => ScalarTy::Bool,
        _ => ScalarTy::Sized(SIZED_INT_TYPES[c.variant(SIZED_INT_TYPES.len())]),
    }
}

/// Append a LEAF value of a given scalar type — type-correct by construction (a Float32/sized leaf is
/// ascribed; a bare int/float literal is Int64/Float64). The type-directed building block: it lets a
/// compound hold elements of ARBITRARY (independently-chosen) scalar types, not just Int64.
fn gen_scalar_leaf<C: Choice>(c: &mut C, ty: ScalarTy, out: &mut String) {
    match ty {
        ScalarTy::Int64 => gen_int_literal(c, out),
        ScalarTy::Float64 => {
            write!(out, "{}.0", c.int_bounded(0, 1000)).ok();
        }
        ScalarTy::Float32 => {
            write!(out, "(: {}.0 Float32)", c.int_bounded(0, 1000)).ok();
        }
        ScalarTy::Bool => out.push_str(if c.variant(2) == 0 { "true" } else { "false" }),
        ScalarTy::Sized(t) => {
            write!(out, "(: {} {t})", c.int_bounded(0, 9)).ok();
        }
    }
}

/// Int64 binary ops kept on the COMPILE path for the type-directed expression generator: no `/`/`%` (a
/// const zero divisor traps/declines) and no shifts (a large const count → CDZ0304). `*` can still const-
/// overflow to a clean CDZ0304 decline — cleanly-handled, so it stays.
const INT_SAFE_OPS: &[&str] = &["+", "-", "*", "&", "|", "^"];

/// Recursive TYPED expression generator — a type-correct expression of `ty`: at `depth == 0` a leaf, else
/// a type-appropriate binary op, a uniform-arm `if`, or (for Bool) a relation. The CORE of the
/// type-directed program: it lets compounds / fn args / fn bodies hold typed EXPRESSIONS, not just leaves,
/// exercising typed-expression emit in those positions. UNIFORM by construction (an `if`'s two arms share
/// `ty`, a Float32 stays Float32) — no join mixes widths (which would hit v-rb's open match-emit-widen) —
/// so it stays cleanly handled: it COMPILES, or cleanly DECLINES on a const overflow / non-finite float.
fn gen_of_ty<C: Choice>(
    c: &mut C,
    ty: ScalarTy,
    depth: usize,
    fresh: &mut usize,
    out: &mut String,
) {
    if depth == 0 {
        gen_scalar_leaf(c, ty, out);
        return;
    }
    match ty {
        ScalarTy::Float64 => gen_float(c, false, depth, fresh, out),
        ScalarTy::Float32 => gen_float(c, true, depth, fresh, out),
        ScalarTy::Bool => match c.variant(3) {
            0 => gen_scalar_leaf(c, ScalarTy::Bool, out),
            // (<rel> <int> <int>) — a Bool from two Int64 sub-expressions.
            1 => {
                let rel = RELS[c.variant(RELS.len())];
                write!(out, "({rel} ").ok();
                gen_of_ty(c, ScalarTy::Int64, depth - 1, fresh, out);
                out.push(' ');
                gen_of_ty(c, ScalarTy::Int64, depth - 1, fresh, out);
                out.push(')');
            }
            _ => gen_if_of(c, ScalarTy::Bool, depth, fresh, out),
        },
        ScalarTy::Int64 => match c.variant(3) {
            0 => gen_int_literal(c, out),
            1 => {
                let op = INT_SAFE_OPS[c.variant(INT_SAFE_OPS.len())];
                write!(out, "({op} ").ok();
                gen_of_ty(c, ScalarTy::Int64, depth - 1, fresh, out);
                out.push(' ');
                gen_of_ty(c, ScalarTy::Int64, depth - 1, fresh, out);
                out.push(')');
            }
            _ => gen_if_of(c, ScalarTy::Int64, depth, fresh, out),
        },
        // Sized: a leaf or a width-safe op over two ascribed 0..=9 operands (no overflow at any width).
        ScalarTy::Sized(t) => match c.variant(2) {
            0 => gen_scalar_leaf(c, ScalarTy::Sized(t), out),
            _ => {
                let op = SIZED_INT_OPS[c.variant(SIZED_INT_OPS.len())];
                write!(
                    out,
                    "({op} (: {} {t}) (: {} {t}))",
                    c.int_bounded(0, 9),
                    c.int_bounded(0, 9)
                )
                .ok();
            }
        },
    }
}

/// `(if <bool-cond> <ty> <ty>)` with a generated Bool condition and two UNIFORM `ty` arms (same type, so
/// the join never mixes widths). Shared by [`gen_of_ty`]'s Int64/Bool if-arms.
fn gen_if_of<C: Choice>(
    c: &mut C,
    ty: ScalarTy,
    depth: usize,
    fresh: &mut usize,
    out: &mut String,
) {
    out.push_str("(if ");
    gen_of_ty(c, ScalarTy::Bool, depth - 1, fresh, out);
    out.push(' ');
    gen_of_ty(c, ty, depth - 1, fresh, out);
    out.push(' ');
    gen_of_ty(c, ty, depth - 1, fresh, out);
    out.push(')');
}

/// A TYPE-DIVERSE compound body over independently-typed scalar leaves — the type-directed step past the
/// Int64-element compounds [`gen_compound`] builds. One of: a heterogeneous `(tuple …)` (2–3 mixed-type
/// leaves); a homogeneous non-Int64 `(list …)`; a heterogeneous `(record (= a …) …)`; an `(Some …)`
/// (Option, inferable bare); or an annotated `(: (Ok/Err …) (Result T E))` (a sum needs the annotation).
/// Reaches heterogeneous-product / non-Int64-list / named-record / Option+Result sum value+codec+emit
/// lowering. Tuple/record elements MAY themselves NEST a compound (bounded by `depth`) — reaching
/// nested-structure emit (tuple-of-record, record-with-Option, …); list/Result payloads stay scalar.
fn gen_typed_compound<C: Choice>(c: &mut C, depth: usize, fresh: &mut usize, out: &mut String) {
    match c.variant(5) {
        // Heterogeneous tuple of 2 or 3 independently-typed elements (each may NEST a compound).
        0 => {
            let n = 2 + c.variant(2);
            out.push_str("(tuple");
            for _ in 0..n {
                out.push(' ');
                gen_compound_element(c, depth, fresh, out);
            }
            out.push(')');
        }
        // Homogeneous list of one chosen scalar type (3 elements, all `ty`).
        1 => {
            let ty = pick_scalar_ty(c);
            out.push_str("(list");
            for _ in 0..3 {
                out.push(' ');
                gen_of_ty(c, ty, ELEM_DEPTH, fresh, out);
            }
            out.push(')');
        }
        // Heterogeneous record of 2 or 3 named fields (a/b/c), each independently-typed (may NEST a compound).
        2 => {
            let n = 2 + c.variant(2);
            out.push_str("(record");
            for i in 0..n {
                let field = ["a", "b", "c"][i];
                write!(out, " (= {field} ").ok();
                gen_compound_element(c, depth, fresh, out);
                out.push(')');
            }
            out.push(')');
        }
        // (Some <expr>) — Option, inferable bare from its payload.
        3 => {
            out.push_str("(Some ");
            let ty = pick_scalar_ty(c);
            gen_of_ty(c, ty, ELEM_DEPTH, fresh, out);
            out.push(')');
        }
        // (: (Ok/Err <expr>) (Result T E)) — a sum needs a type annotation to infer both arms.
        _ => {
            let ok = pick_scalar_ty(c);
            let err = pick_scalar_ty(c);
            let (ctor, payload) = if c.variant(2) == 0 {
                ("Ok", ok)
            } else {
                ("Err", err)
            };
            write!(out, "(: ({ctor} ").ok();
            gen_of_ty(c, payload, ELEM_DEPTH, fresh, out);
            write!(out, ") (Result {} {}))", ok.name(), err.name()).ok();
        }
    }
}

/// A tuple/record element: at `depth > 0` it MAY be a NESTED compound (recurse, bounded by `depth`),
/// otherwise a typed scalar EXPRESSION. Only heterogeneous positions (tuple/record) nest — a homogeneous
/// list needs all elements the same type, and Result payloads need a nameable type for the annotation, so
/// those stay scalar. Reaches NESTED-structure value/codec/emit (tuple-of-record, record-with-Option, …).
fn gen_compound_element<C: Choice>(c: &mut C, depth: usize, fresh: &mut usize, out: &mut String) {
    if depth > 0 && c.variant(3) == 0 {
        gen_typed_compound(c, depth - 1, fresh, out);
    } else {
        let ty = pick_scalar_ty(c);
        gen_of_ty(c, ty, ELEM_DEPTH, fresh, out);
    }
}

/// Depth of a typed sub-expression in a compound element / fn arg — shallow (structured, but bounds size).
const ELEM_DEPTH: usize = 2;

/// Max NESTING depth of a type-diverse compound (tuple/record elements may be compounds) — bounds size.
const COMPOUND_DEPTH: usize = 2;

/// A body that DEFINES and CALLS a locally-TYPED function: `(do (def (g (: x T)) <body>) (g <T-leaf>))`.
/// The generator's helpers (`f`/`r`/`t`) are Int64-only; this exercises a typed function PARAM `T`, a
/// typed RETURN, and a typed CALL/arg-pass across arbitrary scalar types. The body is either the identity
/// `x` (return type `T` — a typed round-trip THROUGH the param) or an independently-typed leaf `U` (the
/// `T` param ignored, a `U` return — distinct in/out types). Half the time the param is instead a
/// COMPOUND type (identity over a `(Tuple …)`/`(List …)`/`(Option …)`/`(Record …)`, see
/// [`gen_compound_ty`]) — a compound value marshalled IN as an arg + OUT as the return, exercising
/// compound-typed function ABI. Type-correct + bounded (typed function args/returns, scalar + compound).
fn gen_typed_fn_call_body<C: Choice>(c: &mut C, fresh: &mut usize, out: &mut String) {
    // 1/2 the time pass a COMPOUND-typed param through the call (compound-value marshalling across a call
    // frame) — otherwise a scalar param (scalar/independent-U return).
    if c.variant(2) == 0 {
        let (ty, val) = gen_compound_ty(c);
        // Identity over a compound: `(def (g (: x <compound-ty>)) x) (g <compound-val>)` — the compound
        // flows IN as an arg and OUT as the return, exercising compound-typed function ABI.
        write!(out, "(do (def (g (: x {ty})) x) (g {val}))").ok();
        return;
    }
    let pty = pick_scalar_ty(c);
    write!(out, "(do (def (g (: x {})) ", pty.name()).ok();
    if c.variant(2) == 0 {
        out.push('x'); // identity — return type T (the param flows to the return)
    } else {
        let uty = pick_scalar_ty(c);
        gen_of_ty(c, uty, ELEM_DEPTH, fresh, out); // independent U-typed expression (param ignored)
    }
    out.push_str(") (g ");
    gen_of_ty(c, pty, ELEM_DEPTH, fresh, out); // call argument of type T
    out.push_str("))");
}

/// Generate a COMPOUND type + a matching value, as `(type_string, value_string)` — a `(Tuple T U)` /
/// `(List T)` / `(Option T)` / `(Record (: a T) (: b U))` over scalar-leaf elements. The element types
/// drive BOTH the type annotation and the value, so they agree by construction. Used for a compound-typed
/// function param (the compound crosses a call boundary). Scalar LEAVES keep the value simple + inferable.
fn gen_compound_ty<C: Choice>(c: &mut C) -> (String, String) {
    let mut ty = String::new();
    let mut val = String::new();
    match c.variant(4) {
        // (Tuple T U) / (tuple <T> <U>)
        0 => {
            let (a, b) = (pick_scalar_ty(c), pick_scalar_ty(c));
            write!(ty, "(Tuple {} {})", a.name(), b.name()).ok();
            val.push_str("(tuple ");
            gen_scalar_leaf(c, a, &mut val);
            val.push(' ');
            gen_scalar_leaf(c, b, &mut val);
            val.push(')');
        }
        // (List T) / (list <T> <T> <T>)
        1 => {
            let a = pick_scalar_ty(c);
            write!(ty, "(List {})", a.name()).ok();
            val.push_str("(list ");
            gen_scalar_leaf(c, a, &mut val);
            val.push(' ');
            gen_scalar_leaf(c, a, &mut val);
            val.push(' ');
            gen_scalar_leaf(c, a, &mut val);
            val.push(')');
        }
        // (Option T) / (Some <T>)
        2 => {
            let a = pick_scalar_ty(c);
            write!(ty, "(Option {})", a.name()).ok();
            val.push_str("(Some ");
            gen_scalar_leaf(c, a, &mut val);
            val.push(')');
        }
        // (Record (: a T) (: b U)) / (record (= a <T>) (= b <U>))
        _ => {
            let (a, b) = (pick_scalar_ty(c), pick_scalar_ty(c));
            write!(ty, "(Record (: a {}) (: b {}))", a.name(), b.name()).ok();
            val.push_str("(record (= a ");
            gen_scalar_leaf(c, a, &mut val);
            val.push_str(") (= b ");
            gen_scalar_leaf(c, b, &mut val);
            val.push_str("))");
        }
    }
    (ty, val)
}

/// A body that BUILDS a compound and immediately CONSUMES it — tuple/record projection (`(. c i/field)`),
/// `(List.len …)`, or an Option `match`. The generator builds compounds elsewhere but rarely CONSUMES
/// them; this exercises the distinct extract / project / list-len / match consumption emit (where the S52
/// closure buckets lived). Self-contained + type-correct (payload types drive the leaves; a match's
/// None arm defaults to the payload type). Reaches consumption lowering the construction arms never hit.
fn gen_compound_consume<C: Choice>(c: &mut C, out: &mut String) {
    match c.variant(4) {
        // Tuple projection: (. (tuple <a> <b>) <0|1>).
        0 => {
            let (a, b) = (pick_scalar_ty(c), pick_scalar_ty(c));
            let idx = c.variant(2);
            out.push_str("(. (tuple ");
            gen_scalar_leaf(c, a, out);
            out.push(' ');
            gen_scalar_leaf(c, b, out);
            write!(out, ") {idx})").ok();
        }
        // Record field access: (. (record (= a <>) (= b <>)) <a|b>).
        1 => {
            let (a, b) = (pick_scalar_ty(c), pick_scalar_ty(c));
            let field = if c.variant(2) == 0 { "a" } else { "b" };
            out.push_str("(. (record (= a ");
            gen_scalar_leaf(c, a, out);
            out.push_str(") (= b ");
            gen_scalar_leaf(c, b, out);
            write!(out, ")) {field})").ok();
        }
        // List length: (List.len (list <t> <t> <t>)).
        2 => {
            let t = pick_scalar_ty(c);
            out.push_str("(List.len (list ");
            gen_scalar_leaf(c, t, out);
            out.push(' ');
            gen_scalar_leaf(c, t, out);
            out.push(' ');
            gen_scalar_leaf(c, t, out);
            out.push_str("))");
        }
        // Option match: (match (Some <t>) ((Some x) x) ((None) <t-default>)) — both arms type `t`.
        _ => {
            let t = pick_scalar_ty(c);
            out.push_str("(match (Some ");
            gen_scalar_leaf(c, t, out);
            out.push_str(") ((Some x) x) ((None) ");
            gen_scalar_leaf(c, t, out);
            out.push_str("))");
        }
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
                    // `main` is either param-less `(def (main) …)` or a heap-param entry `(def (main (: v0 …
                    && program.source.contains("(def (main")
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

    /// REGRESSION GUARD for the bucket-1 emit miscompile (rcdzc #4961): an EXPORTED entry with a
    /// heap/reference-typed param + a reachable RECURSIVE fn once emitted the recursive call's result at
    /// the wrong wasm width (i32 vs i64) → InvalidWasm. Each `HEAP_PARAM_TYPES` entry, wired to the exact
    /// minimal shape the fuzzer found + bisected, must COMPILE (not merely be cleanly handled) — so a
    /// re-introduction of the def-index-shift bug fails here rather than silently in a campaign.
    #[test]
    fn heap_param_entry_over_a_recursive_fn_compiles() {
        for ty in HEAP_PARAM_TYPES {
            let src = format!(
                "(do (def (main (: v0 {ty})) (do (def (v1 v2) (if (<= v2 0) v2 (v1 (- v2 1)))) (v1 2))) (export main))"
            );
            assert!(
                matches!(compile_catching(&src), Verdict::Compiled { .. }),
                "heap-param entry ({ty}) over a recursive fn must COMPILE (bucket-1 #4961 regression): {src}"
            );
        }
    }

    /// The generator REACHES the heap-param-entry shape (the #4961 regression-guard path) across varied
    /// entropy — so the coercing fuzzer actually exercises the exported-entry heap-param ABI lowering, not
    /// only param-less `main`.
    #[test]
    fn generator_reaches_a_heap_param_entry() {
        let mut saw = false;
        for seed in 0u64..128 {
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(5);
            let mut bytes = Vec::new();
            for _ in 0..24 {
                x ^= x >> 30;
                x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                bytes.push((x >> 24) as u8);
            }
            if generate_coerced(&bytes)
                .source
                .contains("(def (main (: v0 ")
            {
                saw = true;
                break;
            }
        }
        assert!(
            saw,
            "the coercing generator should reach a heap-param entry `(def (main (: v0 …)) …)`"
        );
    }

    /// The generator REACHES a runtime `(: n Int64)` entry that actually REFERENCES `n` — a
    /// runtime-dependent program (not const-foldable) that keeps `if`/`match` joins live (no dead-branch
    /// elim). Guards that the runtime-`n` branch produces `n`-using bodies, not just an unused param.
    #[test]
    fn generator_reaches_a_runtime_n_entry() {
        let mut saw = false;
        for seed in 0u64..256 {
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(61);
            let mut bytes = Vec::new();
            for _ in 0..24 {
                x ^= x >> 30;
                x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                bytes.push((x >> 24) as u8);
            }
            let src = generate_coerced(&bytes).source;
            // A runtime-n entry that USES n: after the `(: n Int64))` param, the body has a standalone
            // `n` token (a var reference) — tokenize on non-identifier chars so `main` doesn't false-match.
            if let Some((_, after)) = src.split_once("(def (main (: n Int64)) ") {
                let body = after.strip_suffix(") (export main))").unwrap_or(after);
                if body
                    .split(|c: char| !c.is_alphanumeric() && c != '_')
                    .any(|tok| tok == "n")
                {
                    saw = true;
                    break;
                }
            }
        }
        assert!(
            saw,
            "the coercing generator should reach a runtime `(: n Int64)` entry that references `n`"
        );
    }

    /// Every form the sized-int body arm emits — an ascribed literal `(: n T)`, a checked conversion
    /// `(T.of n)`, and a width-safe binary op `(<op> (: a T) (: b T))` — must COMPILE (not merely be
    /// cleanly handled) for EVERY `T` in `SIZED_INT_TYPES`: the arm is deliberately kept on the compile
    /// path (small 0..=9 operands + no-overflow ops) so the coverage actually reaches narrow-width emit.
    /// Guards `SIZED_INT_TYPES`/`SIZED_INT_OPS` (a bad type/op name would decline/parse-error here).
    #[test]
    fn every_sized_int_body_form_compiles() {
        for t in SIZED_INT_TYPES {
            for src in [
                format!("(do (def (main) (: 5 {t})) (export main))"),
                format!("(do (def (main) ({t}.of 9)) (export main))"),
                format!("(do (def (main) (+ (: 9 {t}) (: 9 {t}))) (export main))"),
                format!("(do (def (main) (* (: 9 {t}) (: 9 {t}))) (export main))"),
                format!("(do (def (main) (& (: 9 {t}) (: 3 {t}))) (export main))"),
                format!("(do (def (main) (| (: 5 {t}) (: 2 {t}))) (export main))"),
                format!("(do (def (main) (^ (: 6 {t}) (: 3 {t}))) (export main))"),
            ] {
                assert!(
                    matches!(compile_catching(&src), Verdict::Compiled { .. }),
                    "sized-int body form must COMPILE: {src}"
                );
            }
        }
    }

    /// The generator REACHES the sized-int body arm across varied entropy — so the coverage (narrow-width
    /// value/arith/conversion emit) is actually exercised by the coercing fuzzer, not dead.
    #[test]
    fn generator_reaches_a_sized_int_body() {
        let mut saw = false;
        for seed in 0u64..256 {
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(9);
            let mut bytes = Vec::new();
            for _ in 0..24 {
                x ^= x >> 30;
                x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                bytes.push((x >> 24) as u8);
            }
            let src = generate_coerced(&bytes).source;
            if SIZED_INT_TYPES
                .iter()
                .any(|t| src.contains(&format!(": {t})")) || src.contains(&format!("{t}.of ")))
            {
                saw = true;
                break;
            }
        }
        assert!(saw, "the coercing generator should reach a sized-int body");
    }

    /// Every `gen_float` expression (both widths, across depths/arms) is CLEANLY HANDLED — it COMPILES or
    /// cleanly DECLINES (e.g. a const-folded non-finite `/ 0.0` or an overflow-to-`inf` `*`), never a
    /// crash / invalid wasm / parse error. It is uniform-width by construction (Float32 leaves ascribed,
    /// if/match arms same width) so no join mixes widths (which would hit v-rb's open match-emit-widen
    /// bug) — a width leak would surface as InvalidWasm here. Also asserts the arm REACHES the compile
    /// path (some body COMPILES), so the float value/arith/emit coverage is real, not all const-declines.
    #[test]
    fn gen_float_body_is_cleanly_handled_and_reaches_emit() {
        let mut saw_compiled = false;
        for seed in 0u64..256 {
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(13);
            let mut bytes = Vec::new();
            for _ in 0..32 {
                x ^= x >> 30;
                x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                bytes.push((x >> 24) as u8);
            }
            let mut fresh = 0usize;
            for is_f32 in [false, true] {
                let mut src = String::from("(do (def (main) ");
                gen_float(
                    &mut ByteCursorChoice::new(&bytes),
                    is_f32,
                    MAX_DEPTH,
                    &mut fresh,
                    &mut src,
                );
                src.push_str(") (export main))");
                let v = compile_catching(&src);
                assert!(
                    matches!(v, Verdict::Compiled { .. } | Verdict::Declined { .. }),
                    "uniform-width float body must be cleanly handled (is_f32={is_f32}), got {v:?}: {src}"
                );
                saw_compiled |= matches!(v, Verdict::Compiled { .. });
            }
        }
        assert!(
            saw_compiled,
            "the float body arm should REACH the compile path (float value/arith emit), not only const-declines"
        );
    }

    /// The generator REACHES a float-typed body across varied entropy — so float value/arith/if-join/let
    /// lowering is actually exercised by the coercing fuzzer, not dead.
    #[test]
    fn generator_reaches_a_float_body() {
        let mut saw = false;
        for seed in 0u64..256 {
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(21);
            let mut bytes = Vec::new();
            for _ in 0..24 {
                x ^= x >> 30;
                x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                bytes.push((x >> 24) as u8);
            }
            let src = generate_coerced(&bytes).source;
            // A float body shows up as a bare `N.0` (Float64) or an ascribed `Float32` literal.
            if src.contains(".0)") || src.contains(".0 ") || src.contains("Float32)") {
                saw = true;
                break;
            }
        }
        assert!(
            saw,
            "the coercing generator should reach a float-typed body"
        );
    }

    /// Every `gen_typed_compound` — a heterogeneous tuple (independently-typed leaves) or a non-Int64
    /// homogeneous list — is CLEANLY HANDLED (leaf elements are type-correct by construction, so these
    /// COMPILE); guards `gen_scalar_leaf`/`pick_scalar_ty` (a bad type name / ill-typed leaf would surface
    /// as decline/InvalidWasm here). Also asserts a non-Int64-element compound is REACHED (real coverage
    /// past the Int64-only `gen_compound`).
    #[test]
    fn gen_typed_compound_is_cleanly_handled_and_diverse() {
        let mut saw_non_int64 = false;
        for seed in 0u64..256 {
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(37);
            let mut bytes = Vec::new();
            for _ in 0..16 {
                x ^= x >> 30;
                x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                bytes.push((x >> 24) as u8);
            }
            let mut src = String::from("(do (def (main) ");
            gen_typed_compound(
                &mut ByteCursorChoice::new(&bytes),
                COMPOUND_DEPTH,
                &mut 0,
                &mut src,
            );
            src.push_str(") (export main))");
            assert!(
                matches!(
                    compile_catching(&src),
                    Verdict::Compiled { .. } | Verdict::Declined { .. }
                ),
                "type-diverse compound must be cleanly handled: {src}"
            );
            // Reached a non-Int64 element (float / bool / sized) → genuinely past Int64-only compounds.
            if src.contains(".0")
                || src.contains("true")
                || src.contains("false")
                || src.contains(": ")
            {
                saw_non_int64 = true;
            }
        }
        assert!(
            saw_non_int64,
            "type-diverse compounds should reach non-Int64 element types"
        );
    }

    /// Every `gen_typed_fn_call_body` (a typed local `(def (g (: x T)) …)` + `(g <T-leaf>)`) is CLEANLY
    /// HANDLED across scalar param/return types, and REACHES a non-Int64 param type — so typed function
    /// param/return/call ABI is genuinely exercised, not just the Int64 helpers.
    #[test]
    fn gen_typed_fn_call_body_is_cleanly_handled_and_diverse() {
        let mut saw_non_int64_param = false;
        for seed in 0u64..256 {
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(41);
            let mut bytes = Vec::new();
            for _ in 0..16 {
                x ^= x >> 30;
                x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                bytes.push((x >> 24) as u8);
            }
            let mut src = String::from("(do (def (main) ");
            gen_typed_fn_call_body(&mut ByteCursorChoice::new(&bytes), &mut 0, &mut src);
            src.push_str(") (export main))");
            assert!(
                matches!(
                    compile_catching(&src),
                    Verdict::Compiled { .. } | Verdict::Declined { .. }
                ),
                "typed fn def+call must be cleanly handled: {src}"
            );
            // A non-Int64 param shows up as `(: x Float…/Int8/…/Bool)` — i.e. `(: x ` not followed by Int64.
            if src.contains("(: x ") && !src.contains("(: x Int64)") {
                saw_non_int64_param = true;
            }
        }
        assert!(
            saw_non_int64_param,
            "typed fn bodies should reach a non-Int64 param type"
        );
    }

    /// Every `gen_compound_consume` (tuple/record projection, `List.len`, Option `match`) COMPILES — the
    /// build+consume shapes are type-correct by construction and stay on the compile path (no overflow /
    /// div0), so this exercises consumption emit and guards `gen_compound_consume` (a malformed
    /// projection / match would surface as decline/InvalidWasm here).
    #[test]
    fn gen_compound_consume_compiles() {
        for seed in 0u64..256 {
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(53);
            let mut bytes = Vec::new();
            for _ in 0..16 {
                x ^= x >> 30;
                x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                bytes.push((x >> 24) as u8);
            }
            let mut src = String::from("(do (def (main) ");
            gen_compound_consume(&mut ByteCursorChoice::new(&bytes), &mut src);
            src.push_str(") (export main))");
            assert!(
                matches!(compile_catching(&src), Verdict::Compiled { .. }),
                "compound-consume body must COMPILE: {src}"
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
