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
/// Build a USER-DEFINED SUM program: a TOP-LEVEL `(type …)` declaration + a param-less `main` that
/// CONSTRUCTS one variant and MATCHES it. Two shapes (#5456): a MULTI-variant tagged sum
/// `(type Shape (Circle Int64) (Rect Int64 Int64))` (construct `(Circle a)` / `(Rect a b)`, match both
/// arms), or a SINGLE-variant struct-newtype `(type Pt (Mk Int64 Int64))` that ERASES to its field tuple
/// (construct `(Pt.Mk a b)`, match `(Mk x y)`). Int64 fields + arms returning Int64 keep it type-correct.
/// The type decl MUST be top-level — a local/in-body `(type …)` SKIPs in the oracle; top-level GRADES.
/// Returns `(type_decl, main_body)`.
fn gen_usersum<C: Choice>(c: &mut C) -> (String, String) {
    let (a, b) = (c.int_bounded(0, 9), c.int_bounded(0, 9));
    match c.variant(3) {
        // MULTI-variant tagged sum — construct Circle OR Rect, match both arms (each returns Int64).
        0 => {
            let ctor = if c.variant(2) == 0 {
                format!("(Circle {a})")
            } else {
                format!("(Rect {a} {b})")
            };
            (
                "(type Shape (Circle Int64) (Rect Int64 Int64))".to_string(),
                format!("(match {ctor} ((Circle x) x) ((Rect p q) (+ p q)))"),
            )
        }
        // SINGLE-variant struct-newtype (erases to a field tuple) — construct + destructure.
        1 => (
            "(type Pt (Mk Int64 Int64))".to_string(),
            format!("(match (Pt.Mk {a} {b}) ((Mk x y) (+ x y)))"),
        ),
        // NULLARY-ctor enum — `main` returns a BARE nullary-ctor NAME as its value (the #5589 shape:
        // a bare nullary ctor as a value now grades; a payload-carrying ctor value the other arms never
        // reach the nullary case of).
        _ => (
            "(type Color (Red) (Green) (Blue))".to_string(),
            ["Red", "Green", "Blue"][(a % 3) as usize].to_string(),
        ),
    }
}

/// Short, valid symbol bodies for the Symbol-in-compound shapes — plain lowercase, so `#"<s>"` needs
/// no escaping and always parses.
const SYMS: [&str; 6] = ["a", "foo", "bar", "tag", "x", "key"];

/// Build a param-less `main` body producing a SYMBOL-IN-COMPOUND value — the tag-20 value_codec path
/// landed by v-nix #7710. cdz-smith emitted NO symbols before, so this codec was entirely un-fuzzed.
/// Every shape is a comparable value (the differential normalizes the `(. Symbol of)` vs `(Symbol.of …)`
/// rendering, verified): a Symbol in a tuple / a homogeneous `(List Symbol)` / a record field / a NESTED
/// symbol / structural symbol equality (→ Bool). Self-contained (no top-level defs). Returns the body.
fn gen_symbol_compound_body<C: Choice>(c: &mut C) -> String {
    let shape = c.variant(5);
    let n = c.int_bounded(0, 9);
    let a = SYMS[c.variant(SYMS.len())];
    let b = SYMS[c.variant(SYMS.len())];
    let d = SYMS[c.variant(SYMS.len())];
    match shape {
        // Symbol in a heterogeneous tuple: (Tuple Symbol Int64).
        0 => format!("(tuple #\"{a}\" {n})"),
        // Homogeneous (List Symbol).
        1 => format!("(list #\"{a}\" #\"{b}\" #\"{d}\")"),
        // Symbol in a record field.
        2 => format!("(record (= t #\"{a}\") (= n {n}))"),
        // NESTED symbol — a symbol inside an inner tuple inside an outer tuple.
        3 => format!("(tuple (tuple #\"{a}\" {n}) #\"{b}\")"),
        // Structural symbol equality → Bool (exercises the Symbol compare path).
        _ => format!("(= #\"{a}\" #\"{b}\")"),
    }
}

/// Build a NOMINAL-over-Symbol program — v-nix #7714: a const nominal newtype wrapping a Symbol must
/// recover the `(Symbol.of "…")` value-form, not the bare `String` the erasure would leave. A
/// `(type Tag (T Symbol))` newtype (erases to its Symbol field) constructed as `(Tag.T #"…")`, returned
/// bare or inside a tuple/list. Returns `(type_decl, body)`; the decl MUST be top-level (a local
/// `(type …)` SKIPs in the oracle, and the newtype ctor `Tag.T` must resolve). Pins #7714's value-form.
fn gen_nominal_symbol_program<C: Choice>(c: &mut C) -> (String, String) {
    let a = SYMS[c.variant(SYMS.len())];
    let b = SYMS[c.variant(SYMS.len())];
    let body = match c.variant(3) {
        // Bare nominal-Symbol value (the #7714 shape: a const nominal-over-Symbol value).
        0 => format!("(Tag.T #\"{a}\")"),
        // Nominal-Symbol inside a tuple.
        1 => format!("(tuple (Tag.T #\"{a}\") (Tag.T #\"{b}\"))"),
        // Nominal-Symbol inside a list.
        _ => format!("(list (Tag.T #\"{a}\"))"),
    };
    ("(type Tag (T Symbol))".to_string(), body)
}

/// The NARROW TYPE-FUZZING generator (S194 — the operator #1-for-types false-reject/false-accept lever).
/// Emit a `(do (def (main) <body>) (export main))` whose body is STRICTLY inside the Lean type oracle's
/// modeled fragment — Int64/Bool scalars, arithmetic, comparison, boolean connectives, `if`, `let`, and
/// ascription — so nearly every program is JUDGED (not skipped), unlike the broad text/astgen grammars
/// (~3% judged). Biased ~80% WELL-TYPED (rcdzc accepts + oracle WellTyped ⇒ holds; an rcdzc CODED reject
/// over a well-typed program is a FALSE-REJECT) and ~20% genuinely ILL-TYPED (rcdzc rejects + oracle
/// IllTyped ⇒ holds; an rcdzc ACCEPT of an ill-typed program is a FALSE-ACCEPT / soundness hole). Both
/// directions are the operator's "oracles in both directions". Int64/Bool only in this first slice;
/// tuple/record/fn/sum + match arms are additive follow-ups (v-lean-oracle's fragment widens under them).
pub fn generate_typecheck(entropy: &[u8]) -> Program {
    let mut c = ByteCursorChoice::new(entropy);
    let mut iscope: Vec<String> = Vec::new();
    let mut bscope: Vec<String> = Vec::new();
    let mut fresh = 0usize;
    // ~1/5 a genuinely ill-typed program (false-accept hunt), else a well-typed body — an Int64, a
    // Bool, or a COMPOUND/SUM value (tuple/record construction + Option/Ordering construct — the oracle
    // models tuple/proj + closed records T1.13/14 + sum-construct T1.15, so these judge, not skip).
    let body = if c.variant(5) == 0 {
        gen_typefuzz_illtyped(&mut c, &mut iscope, &mut bscope, &mut fresh)
    } else {
        // Keep the scalar arms dominant (they + their tuple/record PROJECTION sub-arms are near-always
        // judged); the compound/sum VALUE-returning arm skips more (a compound-returning `main` is less
        // fully modeled), so cap it at ~1/5 for construction coverage without over-diluting density.
        match c.variant(5) {
            0 | 1 => gen_typefuzz_int(&mut c, 3, &mut iscope, &mut bscope, &mut fresh),
            2 | 3 => gen_typefuzz_bool(&mut c, 3, &mut iscope, &mut bscope, &mut fresh),
            _ => gen_typefuzz_value(&mut c, &mut iscope, &mut bscope, &mut fresh),
        }
    };
    Program {
        source: format!("(do (def (main) {body}) (export main))"),
    }
}

/// A WELL-TYPED Int64 expression in the modeled fragment (bounded by `depth`).
fn gen_typefuzz_int<C: Choice>(
    c: &mut C,
    depth: u32,
    iscope: &mut Vec<String>,
    bscope: &mut Vec<String>,
    fresh: &mut usize,
) -> String {
    // At depth 0 emit a leaf (literal or an in-scope Int64 var) — bounds recursion + entropy use.
    let arms = if depth == 0 { 2 } else { 8 };
    match c.variant(arms) {
        // Edge-biased Int64 literal.
        0 => {
            let mut s = String::new();
            gen_int_literal(c, &mut s);
            s
        }
        // An in-scope Int64 var (else a literal).
        1 => {
            if iscope.is_empty() {
                let mut s = String::new();
                gen_int_literal(c, &mut s);
                s
            } else {
                iscope[c.int_bounded(0, iscope.len() as i64 - 1) as usize].clone()
            }
        }
        // Arithmetic over two Int64 subexprs (`+`/`-`/`*` — total; `/`/`%` add a zero-divisor trap the
        // VALUE oracle handles, but for the TYPING oracle any Int64→Int64→Int64 op is fine — keep it total).
        2 => {
            let op = ["+", "-", "*"][c.variant(3)];
            let a = gen_typefuzz_int(c, depth - 1, iscope, bscope, fresh);
            let b = gen_typefuzz_int(c, depth - 1, iscope, bscope, fresh);
            format!("({op} {a} {b})")
        }
        // `(if <bool> <int> <int>)`.
        3 => {
            let cnd = gen_typefuzz_bool(c, depth - 1, iscope, bscope, fresh);
            let t = gen_typefuzz_int(c, depth - 1, iscope, bscope, fresh);
            let e = gen_typefuzz_int(c, depth - 1, iscope, bscope, fresh);
            format!("(if {cnd} {t} {e})")
        }
        // `(let ((iN <int>)) <int-using-iN>)` — binds an Int64 var in scope for the body.
        4 => {
            let name = format!("i{}", *fresh);
            *fresh += 1;
            let val = gen_typefuzz_int(c, depth - 1, iscope, bscope, fresh);
            iscope.push(name.clone());
            let body = gen_typefuzz_int(c, depth - 1, iscope, bscope, fresh);
            iscope.pop();
            format!("(let (({name} {val})) {body})")
        }
        // Ascription `(: <int> Int64)` — a constrain-not-contradict that must SOLVE (well-typed).
        5 => {
            let e = gen_typefuzz_int(c, depth - 1, iscope, bscope, fresh);
            format!("(: {e} Int64)")
        }
        // Project an Int64 out of a freshly-built tuple/record (exercises tuple-proj / record-field
        // access — modeled T1.13/14). Both branches yield Int64, keeping the arm well-typed.
        6 => {
            let a = gen_typefuzz_int(c, depth - 1, iscope, bscope, fresh);
            let b = gen_typefuzz_int(c, depth - 1, iscope, bscope, fresh);
            if c.variant(2) == 0 {
                format!("(. (tuple {a} {b}) 0)")
            } else {
                format!("(. (record (= a {a}) (= b {b})) a)")
            }
        }
        // A let-bound lambda applied by NAME: `(let ((fN (fn ((: iM Int64)) <int-body>))) (fN <int-arg>))`.
        // Exercises Fn introduction (T1.11) + CONCRETE-HEAD App (T1.12 — the oracle models application of
        // a NAME, not an inline lambda, so the head must be a bound name to be judged). Returns Int64.
        _ => {
            let f = format!("f{}", *fresh);
            *fresh += 1;
            let p = format!("i{}", *fresh);
            *fresh += 1;
            let arg = gen_typefuzz_int(c, depth - 1, iscope, bscope, fresh);
            iscope.push(p.clone());
            let body = gen_typefuzz_int(c, depth - 1, iscope, bscope, fresh);
            iscope.pop();
            format!("(let (({f} (fn ((: {p} Int64)) {body}))) ({f} {arg}))")
        }
    }
}

/// A WELL-TYPED Bool expression in the modeled fragment (bounded by `depth`).
fn gen_typefuzz_bool<C: Choice>(
    c: &mut C,
    depth: u32,
    iscope: &mut Vec<String>,
    bscope: &mut Vec<String>,
    fresh: &mut usize,
) -> String {
    let arms = if depth == 0 { 2 } else { 8 };
    match c.variant(arms) {
        // Bool literal.
        0 => ["true", "false"][c.variant(2)].to_string(),
        // An in-scope Bool var (else a literal).
        1 => {
            if bscope.is_empty() {
                ["true", "false"][c.variant(2)].to_string()
            } else {
                bscope[c.int_bounded(0, bscope.len() as i64 - 1) as usize].clone()
            }
        }
        // Comparison of two Int64 → Bool.
        2 => {
            let op = ["<", ">", "<=", ">=", "="][c.variant(5)];
            let a = gen_typefuzz_int(c, depth - 1, iscope, bscope, fresh);
            let b = gen_typefuzz_int(c, depth - 1, iscope, bscope, fresh);
            format!("({op} {a} {b})")
        }
        // Boolean connective.
        3 => match c.variant(3) {
            0 => {
                let a = gen_typefuzz_bool(c, depth - 1, iscope, bscope, fresh);
                let b = gen_typefuzz_bool(c, depth - 1, iscope, bscope, fresh);
                format!("(and {a} {b})")
            }
            1 => {
                let a = gen_typefuzz_bool(c, depth - 1, iscope, bscope, fresh);
                let b = gen_typefuzz_bool(c, depth - 1, iscope, bscope, fresh);
                format!("(or {a} {b})")
            }
            _ => {
                let a = gen_typefuzz_bool(c, depth - 1, iscope, bscope, fresh);
                format!("(not {a})")
            }
        },
        // `(if <bool> <bool> <bool>)`.
        4 => {
            let cnd = gen_typefuzz_bool(c, depth - 1, iscope, bscope, fresh);
            let t = gen_typefuzz_bool(c, depth - 1, iscope, bscope, fresh);
            let e = gen_typefuzz_bool(c, depth - 1, iscope, bscope, fresh);
            format!("(if {cnd} {t} {e})")
        }
        // `(let ((bN <bool>)) <bool-using-bN>)`.
        5 => {
            let name = format!("b{}", *fresh);
            *fresh += 1;
            let val = gen_typefuzz_bool(c, depth - 1, iscope, bscope, fresh);
            bscope.push(name.clone());
            let body = gen_typefuzz_bool(c, depth - 1, iscope, bscope, fresh);
            bscope.pop();
            format!("(let (({name} {val})) {body})")
        }
        // Project a Bool out of a freshly-built tuple/record (Bool-typed element/field → Bool).
        6 => {
            let a = gen_typefuzz_bool(c, depth - 1, iscope, bscope, fresh);
            let b = gen_typefuzz_int(c, depth - 1, iscope, bscope, fresh);
            if c.variant(2) == 0 {
                format!("(. (tuple {a} {b}) 0)")
            } else {
                format!("(. (record (= a {a}) (= b {b})) a)")
            }
        }
        // A let-bound lambda applied by NAME returning Bool: `(let ((fN (fn ((: iM Int64)) <bool-body>)))
        // (fN <int-arg>))` — Fn intro + CONCRETE-HEAD App (the head must be a bound name to be judged).
        _ => {
            let f = format!("f{}", *fresh);
            *fresh += 1;
            let p = format!("i{}", *fresh);
            *fresh += 1;
            let arg = gen_typefuzz_int(c, depth - 1, iscope, bscope, fresh);
            iscope.push(p.clone());
            let body = gen_typefuzz_bool(c, depth - 1, iscope, bscope, fresh);
            iscope.pop();
            format!("(let (({f} (fn ((: {p} Int64)) {body}))) ({f} {arg}))")
        }
    }
}

/// A GENUINELY ILL-TYPED body in the modeled fragment — rcdzc must reject (a CODED type fault) and the
/// oracle must infer IllTyped ⇒ holds; an rcdzc ACCEPT here is a FALSE-ACCEPT (soundness hole). Each
/// shape is a real type error the oracle's rules cover: arith-on-Bool, an ascription conflict, a
/// non-Bool `if` condition, a bool-connective-on-Int, a heterogeneous comparison, an unbound name.
fn gen_typefuzz_illtyped<C: Choice>(
    c: &mut C,
    iscope: &mut Vec<String>,
    bscope: &mut Vec<String>,
    fresh: &mut usize,
) -> String {
    let int = |c: &mut C, is: &mut Vec<String>, bs: &mut Vec<String>, f: &mut usize| {
        gen_typefuzz_int(c, 1, is, bs, f)
    };
    let boolean = |c: &mut C, is: &mut Vec<String>, bs: &mut Vec<String>, f: &mut usize| {
        gen_typefuzz_bool(c, 1, is, bs, f)
    };
    match c.variant(10) {
        // Arithmetic with a Bool operand.
        0 => {
            let a = int(c, iscope, bscope, fresh);
            let b = boolean(c, iscope, bscope, fresh);
            format!("(+ {a} {b})")
        }
        // Ascription conflict: an Int64 expr ascribed Bool.
        1 => {
            let e = int(c, iscope, bscope, fresh);
            format!("(: {e} Bool)")
        }
        // Non-Bool `if` condition (an Int64 where a Bool is required).
        2 => {
            let cnd = int(c, iscope, bscope, fresh);
            let t = int(c, iscope, bscope, fresh);
            let e = int(c, iscope, bscope, fresh);
            format!("(if {cnd} {t} {e})")
        }
        // Boolean connective with an Int64 operand.
        3 => {
            let a = boolean(c, iscope, bscope, fresh);
            let b = int(c, iscope, bscope, fresh);
            format!("(and {a} {b})")
        }
        // Heterogeneous comparison (Int64 vs Bool).
        4 => {
            let a = int(c, iscope, bscope, fresh);
            let b = boolean(c, iscope, bscope, fresh);
            format!("(< {a} {b})")
        }
        // Fn ARGUMENT-type mismatch (concrete-head App): a named Int64→Int64 fn applied to a Bool.
        5 => {
            let f = format!("f{}", *fresh);
            *fresh += 1;
            let arg = boolean(c, iscope, bscope, fresh);
            format!("(let (({f} (fn ((: x Int64)) x))) ({f} {arg}))")
        }
        // An Option in an arithmetic position (Int64 vs (Option Int64)).
        6 => {
            let a = int(c, iscope, bscope, fresh);
            let b = int(c, iscope, bscope, fresh);
            format!("(+ (Some {a}) {b})")
        }
        // An Option as an `if` condition (must be Bool).
        7 => {
            let a = int(c, iscope, bscope, fresh);
            let t = int(c, iscope, bscope, fresh);
            let e = int(c, iscope, bscope, fresh);
            format!("(if (Some {a}) {t} {e})")
        }
        // Record field access of a NON-EXISTENT field (row mismatch).
        8 => {
            let a = int(c, iscope, bscope, fresh);
            format!("(. (record (= a {a})) b)")
        }
        // An unbound name (resolution error).
        _ => "zz".to_string(),
    }
}

/// A WELL-TYPED COMPOUND / SUM value in the modeled fragment — exercises CONSTRUCTION typing (not just
/// scalar projection): a tuple, a closed record, an `(Some <int>)` (Option Int64), or an Ordering nullary
/// variant. The oracle models tuple construction, closed records (T1.13/14), and sum-construct (T1.15),
/// so these are JUDGED. (`(Ok …)` / `None` need a type annotation to determine the other Result/Option
/// param — omitted here to keep every shape a clean, fully-determined well-typed value.)
fn gen_typefuzz_value<C: Choice>(
    c: &mut C,
    iscope: &mut Vec<String>,
    bscope: &mut Vec<String>,
    fresh: &mut usize,
) -> String {
    match c.variant(4) {
        // A tuple of an Int64 and a Bool.
        0 => {
            let a = gen_typefuzz_int(c, 1, iscope, bscope, fresh);
            let b = gen_typefuzz_bool(c, 1, iscope, bscope, fresh);
            format!("(tuple {a} {b})")
        }
        // A closed record with an Int64 and a Bool field.
        1 => {
            let a = gen_typefuzz_int(c, 1, iscope, bscope, fresh);
            let b = gen_typefuzz_bool(c, 1, iscope, bscope, fresh);
            format!("(record (= a {a}) (= b {b}))")
        }
        // `(Some <int>)` — Option Int64 construction (fully determined).
        2 => {
            let a = gen_typefuzz_int(c, 1, iscope, bscope, fresh);
            format!("(Some {a})")
        }
        // An Ordering nullary variant.
        _ => ["Less", "Equal", "Greater"][c.variant(3)].to_string(),
    }
}

/// Build a RECURSIVE-PERFORM effect program — the "dynamic-extent, statically-resolved" self-hosting
/// shape: a TOP-LEVEL recursive `loop` def PERFORMS the effect op deep inside itself, and `main`'s
/// `handle` (wrapping the `(loop k)` call) discharges every perform across the recursion. This value-grades
/// effect semantics folded over N performs (deeper than the fixed twice-performed lexical
/// [`gen_effect_body`]) AND reaches the cross-function perform the value-diff never covered. Both defs are
/// TOP-LEVEL because a perform in a LOCALLY-nested def has no enclosing handler (CDZ0401) and a local def
/// SKIPs in the oracle — so this must be a whole-program shape, not a main-body expression. Returns
/// `(defs, main_body)`. Terminating (`n` decrements to the `(<= n 0)` base); all magnitudes stay small so
/// the state folded across the ≤6 performs never overflows; deterministic Int64 result.
fn gen_effect_recursive_body<C: Choice>(c: &mut C) -> (String, String) {
    let s0 = c.int_bounded(0, 9);
    let k = c.int_bounded(2, 6); // loop count — bounded so the fold stays small + terminates
    // resume-value / new-state over the in-scope Int64 params `s` (handler state) and `p` (perform arg = n).
    let rv = ["s", "(+ s p)", "p"][c.variant(3)];
    let ns = ["(+ s p)", "(+ s 1)", "(+ s p)"][c.variant(3)];
    let defs = "(effect E (op o (-> Int64 Int64))) \
                (def (loop (: n Int64)) (if (<= n 0) 0 (+ (E.o n) (loop (- n 1)))))"
        .to_string();
    let body = format!("(handle E {s0} ((o (p) s (resume {rv} {ns}))) (loop {k}))");
    (defs, body)
}

/// Build a CROSS-MODULE program — a top-level inline `(module M …)` that exports a function, and a `main`
/// that calls it, so a value crosses the module import/export boundary (in as an argument, out as the
/// result). Grades cross-module VALUE-correctness (operator seq-22) — a value computed inside a module and
/// marshaled across the link — which the coercing grammar never reached (module-fuzz is crash-only, and a
/// single-module `main` never crosses a link). Must be TOP-LEVEL (an inline `(module …)` is a
/// whole-program shape). Returns `(defs, main_body)`. Deterministic Int64; small `0..=9` args so nothing
/// overflows. Three shapes: scalar→scalar, two-arg arithmetic (crosses the link twice), and a COMPOUND
/// result (a heap list crosses the boundary, consumed by `List.len`).
fn gen_module_body<C: Choice>(c: &mut C) -> (String, String) {
    // Each form exports a fn whose RESULT TYPE crosses the module import/export boundary as a distinct WIT
    // marshal (scalar / tuple / Option / record / sized-int / Bool / heap list / arbitrary-precision
    // BigInt), and `main` consumes it to a deterministic value the wasm-vs-rust diff grades. Operator
    // seq-22: stress import/export.
    let form = c.variant(8);
    let a = c.int_bounded(0, 9);
    let b = c.int_bounded(0, 9);
    match form {
        // scalar in → scalar out: `M.f x = (op x k)`; `main = (M.f a)`.
        0 => {
            let op = ["+", "-", "*"][c.variant(3)];
            let k = c.int_bounded(0, 9);
            (
                format!("(module M (def (f (: x Int64)) ({op} x {k})) (export f))"),
                format!("(M.f {a})"),
            )
        }
        // two args → arithmetic, crossing the link TWICE with swapped args.
        1 => (
            "(module M (def (g (: x Int64) (: y Int64)) (* x y)) (export g))".to_string(),
            format!("(+ (M.g {a} {b}) (M.g {b} {a}))"),
        ),
        // a COMPOUND result — a heap list crosses the module boundary, consumed to a count by `List.len`.
        2 => (
            "(module M (def (mk (: x Int64)) (list x x x)) (export mk))".to_string(),
            format!("(List.len (M.mk {a}))"),
        ),
        // a TUPLE crosses the boundary → projected back to a scalar.
        3 => (
            "(module M (def (tup (: x Int64)) (tuple x (+ x 1))) (export tup))".to_string(),
            format!("(. (M.tup {a}) 0)"),
        ),
        // an OPTION (sum) crosses the boundary → matched back to a scalar.
        4 => (
            "(module M (def (opt (: x Int64)) (Some x)) (export opt))".to_string(),
            format!("(match (M.opt {a}) ((Some v) v) (None 0))"),
        ),
        // a RECORD crosses the boundary → projected back to a scalar.
        5 => (
            "(module M (def (rec (: x Int64)) (record (= a x) (= b (+ x 1)))) (export rec))"
                .to_string(),
            format!("(. (M.rec {a}) a)"),
        ),
        // a BOOL crosses the boundary (the result type is Bool, not Int64).
        6 => (
            "(module M (def (lt (: x Int64)) (< x 5)) (export lt))".to_string(),
            format!("(M.lt {a})"),
        ),
        // an arbitrary-precision BIGINT (a beyond-i64 heap value) crosses the boundary → compared to a
        // Bool. A distinct runtime marshal (the heap bignum codec across the module link), and a positive
        // product `(* (a+1)N (a+1)N)` is always `> 0N` regardless of `a`, so the result is deterministic.
        _ => (
            "(module M (def (big (: x Int64)) (* 99999999999999999999999N 3N)) (export big))"
                .to_string(),
            format!("(> (M.big {a}) 0N)"),
        ),
    }
}

/// Build a RECURSIVE COLLECTION-BUILDER program — a top-level recursive `def` that GROWS a heap
/// collection (List via `List.push`, or Map via `Map.insert`) across its `k` calls, and `main` consumes
/// the built collection to a deterministic scalar (`List.len` / `List.at 0` matched / `Map.len`). Grades
/// the recursion × collection interaction — heap allocation + reference-counting across recursive calls,
/// then the marshaled value — which the coercing grammar never reached (the text crash-oracle's
/// `rec_list_builder` is crash-only). Must be TOP-LEVEL (a local recursive def SKIPs in the oracle).
/// Terminating: `n` decrements to the `(<= n 0)` base; `k` is `2..=6` so the collection stays small.
/// Returns `(defs, main_body)`.
fn gen_recursive_collection_body<C: Choice>(c: &mut C) -> (String, String) {
    let form = c.variant(3);
    let k = c.int_bounded(2, 6);
    let list_builder = "(def (build (: n Int64) (: acc (List Int64))) (if (<= n 0) acc (build (- n 1) (List.push acc n))))";
    match form {
        // recursive List.push builder → element count.
        0 => (
            list_builder.to_string(),
            format!("(List.len (build {k} (list)))"),
        ),
        // recursive List.push builder → the element at index 0 (matched out of the Option).
        1 => (
            list_builder.to_string(),
            format!("(match (List.at (build {k} (list)) 0) ((Some v) v) (None 0))"),
        ),
        // recursive Map.insert builder → distinct-key count.
        _ => (
            "(def (bm (: n Int64) (: acc (Map Int64 Int64))) (if (<= n 0) acc (bm (- n 1) (Map.insert acc n n))))"
                .to_string(),
            format!("(Map.len (bm {k} Map.empty))"),
        ),
    }
}

fn build_program<C: Choice>(c: &mut C) -> Program {
    let mut source = String::from("(do ");
    // Two TOP-LEVEL special shapes, chosen by a SINGLE `variant(6)` (one choice consumed, so the fall-through
    // path's cursor is unchanged). Both are emitted as their OWN top-level shape (bypassing the
    // helper/param/`gen_main_body` path) because they only GRADE when TOP-LEVEL: a `(type …)` decl SKIPs
    // in-body, and the oracle captures a LOCAL fn def's env EAGERLY (excluding itself/later siblings) so a
    // local recursive/mutual call is unbound → SKIP. Gated on NON-ZERO values so an EXHAUSTED cursor
    // (variant → 0) falls through to the base-case path (a bare-literal main), preserving that invariant.
    match c.variant(9) {
        // ~1/9: a USER-DEFINED SUM program — tagged variants + newtype erasure + variant patterns (#5456).
        3 => {
            let (type_decl, body) = gen_usersum(c);
            write!(source, "{type_decl} (def (main) {body}) (export main))").ok();
            return Program { source };
        }
        // ~1/7: a MUTUALLY-RECURSIVE program — two top-level defs that call each other + a param-less `main`
        // calling one (a mutual call graph no single self-recursive helper reaches).
        5 => {
            let (defs, body) = gen_mutual_recursion_body(c);
            write!(source, "{defs} (def (main) {body}) (export main))").ok();
            return Program { source };
        }
        // ~1/7: a RECURSIVE-PERFORM effect program — a top-level recursive `loop` performs the op deep
        // inside itself, discharged by `main`'s enclosing `handle` (the dynamic-extent self-hosting shape).
        6 => {
            let (defs, body) = gen_effect_recursive_body(c);
            write!(source, "{defs} (def (main) {body}) (export main))").ok();
            return Program { source };
        }
        // ~1/8: a CROSS-MODULE program — a top-level inline `(module M …)` exports a function, and `main`
        // calls it across the module import/export boundary (a value crosses the boundary in + out). Grades
        // cross-module VALUE-correctness (operator seq-22), which the value-diff never reached (module-fuzz
        // is crash-only). Must be top-level (an inline module is a whole-program shape, like a `(type …)`).
        7 => {
            let (defs, body) = gen_module_body(c);
            write!(source, "{defs} (def (main) {body}) (export main))").ok();
            return Program { source };
        }
        // ~1/9: a RECURSIVE COLLECTION-BUILDER — a top-level recursive `def` GROWS a heap collection
        // (List/Map) across its calls, and `main` consumes the built collection to a scalar. Grades the
        // recursion × collection interaction (heap allocation + RC across recursive calls, then the
        // marshaled value), which the value-diff never reached (the text crash-oracle's rec_list_builder
        // is crash-only). Must be top-level (a recursive def SKIPs in the oracle when local).
        8 => {
            let (defs, body) = gen_recursive_collection_body(c);
            write!(source, "{defs} (def (main) {body}) (export main))").ok();
            return Program { source };
        }
        // ~1/9: a SYMBOL-IN-COMPOUND value program — a Symbol leaf inside a tuple/list/record (+ nested
        // + structural symbol equality), the tag-20 value_codec path v-nix landed in #7710. cdz-smith
        // emitted no symbols before, so this codec was un-fuzzed; a param-less `main` returns the value
        // directly (self-contained). Grades Symbol-in-compound VALUE correctness — the cadenza-differential
        // renders + round-trips it; the lean type oracle skips Symbol as Unsupported (sound).
        4 => {
            // FLIP between a bare Symbol-in-compound value and a NOMINAL-over-Symbol program (#7714 —
            // a nominal newtype wrapping a Symbol, which must recover the `(Symbol.of …)` value-form).
            if c.variant(2) == 0 {
                let body = gen_symbol_compound_body(c);
                write!(source, "(def (main) {body}) (export main))").ok();
            } else {
                let (type_decl, body) = gen_nominal_symbol_program(c);
                write!(source, "{type_decl} (def (main) {body}) (export main))").ok();
            }
            return Program { source };
        }
        _ => {}
    }
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
    // Weighting: keep PARAM-LESS `main` the MAJORITY (~1/2) so most programs stay VALUE-GRADEABLE (a
    // param'd `main` can't be run 0-arg by the value differential → not-comparable); the heap-param and
    // runtime-`n` variants are coverage (~1/4 each) — ample for the ABI/un-masking paths without drowning
    // the value-differential's comparable yield (an equal-thirds split made 2/3 of programs param'd →
    // ~80% not-comparable; this restores ~1/2 gradeable).
    let mut scope: Vec<String> = Vec::new();
    match c.variant(4) {
        0 => {
            let ty = HEAP_PARAM_TYPES[c.variant(HEAP_PARAM_TYPES.len())];
            write!(source, "(def (main (: v0 {ty})) ").ok();
        }
        1 => {
            source.push_str("(def (main (: n Int64)) ");
            scope.push("n".to_string()); // `n` is a runtime Int64 in scope → runtime-dependent programs
        }
        _ => {
            source.push_str("(def (main) "); // variants 2 & 3 → param-less (1/2, the value-gradeable majority)
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
    match c.variant(34) {
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
        // A `?`/`try` body: a fallible (Result/Option) boundary + a `(try …)` unwrap / short-circuit.
        9 => gen_try_body(c, out),
        // A DESTRUCTURING pattern match: `(match (tuple/record …) ((tuple/record …binders…) <binder>))`.
        10 => gen_pattern_match_body(c, out),
        // A BIGINT / RATIONAL body (`main : BigInt`/`Rational`): arbitrary-precision + exact-rational value
        // / arith lowering — a numeric family the Int64/Float/sized grammar never reached.
        11 => gen_bignum_body(c, out),
        // A PARTIAL-APPLICATION / currying body: a local def applied to FEWER args than its arity yields a
        // closure over the remaining params, later completed — the def-call under-arity + applyClosure
        // currying that #5488 grades (was a skip: "operator/application not modeled").
        12 => gen_partial_application_body(c, out),
        // A HIGHER-ORDER body: a fn value (a `(fn …)` lambda or a named def) passed as an ARGUMENT to
        // another def and applied inside it — the applyClosure over a closure-valued parameter.
        13 => gen_higher_order_body(c, out),
        // A DISCARD body: a non-def LEADING do-statement whose value is computed then DISCARDED, followed
        // by the tail that is the block's value — the sequencing/dead-value drop lowering that #5507 grades.
        14 => gen_discard_body(c, out),
        // A FLOAT-ORDERING body: `main : Bool` from a float comparison `(< f f)` / `> / <= / >=` — the
        // IEEE float ordering (#5519) as the RETURNED value (my float bodies only used it in `if` guards).
        15 => gen_float_ordering_body(c, out),
        // A COMPOUND-KEYED set/map body: a set or map whose KEYS are `(tuple …)` compounds, consumed to
        // Int64 via `Set.len` / `Map.len` / `Set.insert` — the structural total order over compound values
        // that #5540 grades (my set/map arm only used scalar keys via `pick_hashable_ty`).
        16 => gen_compound_keyed_collection_body(c, out),
        // A FLOAT-KEYED set/map body: a set/map whose keys are Float64/Float32 (incl `(Float64.nan)`),
        // consumed via `Set.len` / `Map.len` — the float-carrying keys with canonical-bit order + canonical
        // key equality that #5556 grades (my `pick_hashable_ty` excluded floats as keys).
        17 => gen_float_keyed_collection_body(c, out),
        // A STRING-op body: `String.byte-len` / `String.scalar-at` (→ scalar) or `String.concat` /
        // `String.slice` / a bare string literal (→ String value) over small fixed strings — a whole op
        // family the Int64/compound grammar never reached.
        18 => gen_string_body(c, out),
        // A BYTES-op body: `Bytes.len` / `Bytes.at` (→ scalar) or a `b"…"` literal / `Bytes.of` / concat
        // (→ Bytes value) — the Bytes construct family, distinct from String and the Int64/compound grammar.
        19 => gen_bytes_body(c, out),
        // A NESTED / DEEPER compound body: `List.at` / `List.concat`, or a compound-of-compounds value
        // (tuple-of-lists / list-of-tuples / record with compound fields) — deeper structural shapes the
        // flat single-level compound arms never reach.
        20 => gen_nested_compound_body(c, out),
        // A NESTED-SUM body: a sum value wrapping another sum/compound — `(Some (Some …))`, `(Ok (Some …))`,
        // `(Some (tuple …))`, `(Some (list …))` — deeper sum-wrapping than the flat Some/Ok/Err arms.
        21 => gen_nested_sum_body(c, out),
        // An INT CROSS-WIDTH CONVERSION body: `(<Target>.of (: <v> <Source>))` between any two sized-int
        // types (widen / narrow / cross-sign) — the int-conversion codegen my sized-int arm never reached
        // (it only ascribed literals + `(T.of <Int64>)`).
        22 => gen_int_conversion_body(c, out),
        // A WIDER-ARITY compound body: a 3-/4-tuple, a 3-/4-field record, or a projection out of one —
        // wider construction + projection layouts than the 2-field tuple/record arms.
        23 => gen_wide_compound_body(c, out),
        // A BOOLEAN-LOGIC body: `and` / `or` / `not` over integer comparisons (→ Bool) — short-circuit
        // boolean combinators the `gen_cond` arm (bare comparisons) never composed.
        24 => gen_bool_logic_body(c, out),
        // A SIZED-INT SHIFT body: `<<` / `>>` (and a nested shift+bitwise) on a sized-int-ascribed operand
        // — the narrow-width shift codegen the sized-int arm (which only did `+ * & | ^`) never emitted.
        25 => gen_sized_shift_body(c, out),
        // A QUANTITY (Qty) body: `Qty.value` of a Qty.of magnitude or a same-unit arithmetic combination
        // — Qty magnitude/unit value-lowering, a whole numeric family the coercing grammar never reached
        // (only the text crash-hunt did). Value-comparable (Qty.value → Int64).
        26 => gen_qty_body(c, out),
        // A MAP.LOOKUP body: build a 2-entry const map, look up a PRESENT or ABSENT key, and match the
        // `Option` result to Int64 — the fundamental keyed map READ (→ `Option V`) + the Some/None
        // consumption, absent from the coercing grammar (which only did Map.len). Value-comparable.
        27 => gen_map_lookup_body(c, out),
        // A COLLECTION-OP body: Set.union / Set.remove / Map.remove (consumed by `.len` → Int64) or
        // Set.contains (→ Bool) — set-merge / element-removal / membership lowering the grammar never
        // reached (it only did Set.len/insert + Map.len/lookup). Value-comparable.
        28 => gen_collection_op_body(c, out),
        // A LIST-PRODUCING op body: List.push / Set.to-list / Map.to-list, consumed by List.len (→
        // count) or List.at (→ element) — collection ops that BUILD a list, which the grammar never
        // reached (it only read existing lists via at/concat/len). Value-comparable.
        29 => gen_list_producing_op_body(c, out),
        // An EFFECT body: a stateful handler threads an Int64 state and RESUMES with a computed value; the
        // body performs the op twice. Deterministic Int64 result → the wasm-vs-rust diff grades effect
        // SEMANTICS (perform / handle / resume / state-fold), a family the value-diff never reached (effects
        // were crash-checked only, via the text generator). Value-comparable.
        30 => gen_effect_body(c, out),
        // A MULTI-OP effect body: one effect with TWO ops, each with its own handler arm, the body
        // performing BOTH — the op DISPATCH/selection lowering (which arm discharges which perform),
        // distinct from the single-op handler. Value-comparable (deterministic Int64).
        31 => gen_effect_multiop_body(c, out),
        // A NESTED-HANDLER effect body: two effects, the E2 handle NESTED inside the E1 handle, the body
        // performing BOTH — so the E1 perform resolves ACROSS the intervening E2 handler frame to the
        // outer E1 handler (multi-frame handler-stack resolution), distinct from a single handler.
        32 => gen_effect_nested_body(c, out),
        // An EFFECT × COLLECTION body: the handled body BUILDS a heap collection (list/map) whose elements
        // are PERFORM results, consumed to a scalar — the effect-value × collection-marshal interaction
        // (a feature cross the single-shape effect + collection arms never combine), value-comparable.
        33 => gen_effect_collection_body(c, out),
        // A bare Int64 expression (the base case + exhaustion default).
        _ => gen_expr(c, MAX_DEPTH, scope, fresh, caps, out),
    }
}

/// A LIST-PRODUCING-op body consumed to an Int64 (value-comparable): `List.push` (append), or
/// `Set.to-list` / `Map.to-list` (the ordered projection), fed to `List.len` (→ count) or `List.at`
/// (→ element). Fills the list-BUILDING collection ops the grammar never reached (it only read
/// existing lists). Small `0..=9` literals → a deterministic count/element the wasm-vs-rust diff grades.
fn gen_list_producing_op_body<C: Choice>(c: &mut C, out: &mut String) {
    // Pick the FORM before consuming operand choices — else a short entropy seed exhausts the cursor
    // on the literals and `variant` always defaults to 0 (never reaching Set/Map.to-list).
    let form = c.variant(6);
    let (a, b, x, y) = (
        c.int_bounded(0, 4),
        c.int_bounded(5, 9),
        c.int_bounded(0, 9),
        c.int_bounded(0, 9),
    );
    match form {
        // Append then count: `[a,b]` push `x` → len 3.
        0 => write!(out, "(List.len (List.push (list {a} {b}) {x}))").ok(),
        // Append then read the pushed element at index 2.
        1 => write!(out, "(List.at (List.push (list {a} {b}) {x}) 2)").ok(),
        // Set → ordered list → distinct count.
        2 => write!(out, "(List.len (Set.to-list (Set.of (list {a} {b} {x}))))").ok(),
        // Map → ordered entry list → entry count (keys a,b disjoint so two entries).
        3 => write!(
            out,
            "(List.len (Map.to-list (Map.insert (Map.insert Map.empty {a} {x}) {b} {y})))"
        )
        .ok(),
        // PREPEND then count: `[a,b]` prepend `x` at the FRONT → len 3 (the front-insertion sibling of
        // the push arm; a distinct list-builder lowering push never reaches).
        4 => write!(out, "(List.len (List.prepend (list {a} {b}) {x}))").ok(),
        // Prepend then read the FRONT element at index 0 → the prepended `x` (matched out of the Option).
        _ => write!(
            out,
            "(match (List.at (List.prepend (list {a} {b}) {x}) 0) ((Some v) v) (None {y}))"
        )
        .ok(),
    };
}

/// A COLLECTION-OP body over small const collections, consumed to a scalar/Bool (value-comparable):
/// `Set.union`/`Set.intersection`/`Set.difference`/`Set.remove`/`Map.remove`/`Map.merge` fed to `.len`
/// (→ Int64), or `Set.contains` (→ Bool). Half the remove/contains cases target a PRESENT element, half
/// an ABSENT one,
/// so both outcomes are exercised. The binary set-algebra ops share ONE element (`b`) between the two
/// operand sets so union/intersection/difference each yield a DISTINCT deterministic cardinality (the
/// value the wasm-vs-rust diff grades — intersection={b}=1, difference={a}=1, union=3). Elements/keys are
/// `0..=9`.
fn gen_collection_op_body<C: Choice>(c: &mut C, out: &mut String) {
    let form = c.variant(7);
    let present = c.variant(2) == 0;
    let (a, b, d) = (
        c.int_bounded(0, 4),
        c.int_bounded(5, 9),
        c.int_bounded(0, 4),
    );
    // A present target is an element of the base set/map; an absent one is `99`.
    let target = if present { a } else { 99 };
    match form {
        // Set.union cardinality (dedups the shared element).
        0 => write!(
            out,
            "(Set.len (Set.union (Set.of (list {a} {b})) (Set.of (list {b} {d}))))"
        )
        .ok(),
        // Set.remove cardinality (present target shrinks by one; absent leaves it unchanged).
        1 => write!(
            out,
            "(Set.len (Set.remove (Set.of (list {a} {b} {d})) {target}))"
        )
        .ok(),
        // Set membership → Bool.
        2 => write!(out, "(Set.contains (Set.of (list {a} {b} {d})) {target})").ok(),
        // Map.remove cardinality (present key shrinks by one; absent leaves it unchanged).
        3 => write!(
            out,
            "(Map.len (Map.remove (Map.insert (Map.insert Map.empty {a} {b}) {b} {d}) {target}))"
        )
        .ok(),
        // Set.intersection cardinality — only the shared element `b` survives (sibling of union on the
        // DISTINCT intersect set-algebra lowering the union form never reaches).
        4 => write!(
            out,
            "(Set.len (Set.intersection (Set.of (list {a} {b})) (Set.of (list {b} {d}))))"
        )
        .ok(),
        // Set.difference cardinality — left-only elements survive (here `a`), the shared `b` is removed.
        5 => write!(
            out,
            "(Set.len (Set.difference (Set.of (list {a} {b})) (Set.of (list {b} {d}))))"
        )
        .ok(),
        // Map.merge cardinality — the Map analogue of Set.union: merge two 2-entry maps sharing key `b`
        // (map1 keys {a,b}, map2 keys {b,d}) → distinct-key count, deduping the shared key.
        _ => write!(
            out,
            "(Map.len (Map.merge (Map.insert (Map.insert Map.empty {a} {b}) {b} {d}) \
             (Map.insert (Map.insert Map.empty {b} {a}) {d} {b})))"
        )
        .ok(),
    };
}

/// An EFFECT body: `(do (effect E (op o (-> Int64 Int64))) (handle E <s0> ((o (p) s (resume <rv> <ns>)))
/// (+ (E.o <a>) (E.o <b>))))` — a stateful, single-op handler that RESUMES with a computed value and
/// threads an Int64 state; the handled body PERFORMS the op twice. This grades effect SEMANTICS (perform /
/// handle / tail-resume / state-fold) for VALUE correctness (wasm-vs-rust) — a family the coercing grammar
/// never emitted (effects were crash-checked only, via the text generator). Self-contained fixed names
/// (E/o/p/s); the resume-value / new-state expressions read the in-scope Int64 params `s`/`p` or a small
/// literal, and every combination stays Int64→Int64 so it type-checks. All literals are `0..=9`, so the
/// state threaded across the two performs (each op resumes exactly once → the fold is bounded and always
/// terminates) cannot overflow. The result is a deterministic Int64 the wasm-vs-rust diff grades.
fn gen_effect_body<C: Choice>(c: &mut C, out: &mut String) {
    // Draw the FORM choices BEFORE the operand literals — else a short entropy seed exhausts the cursor on
    // the int_bounded draws and `variant` always defaults to 0 (never reaching the bare-param arm-value
    // forms, or the abort form). Same trap as gen_list_producing_op_body.
    let abort = c.variant(2) == 1;
    // The op-arm value / new-state expressions over the in-scope Int64 params `s` (state) and `p` (perform
    // arg). Each is Int64→Int64, so any pairing type-checks; small ops keep the twice-threaded state small.
    let av = ["(+ s p)", "(+ p 1)", "s", "p"][c.variant(4)];
    let ns = ["(+ s p)", "(+ s 1)", "s", "p"][c.variant(4)];
    let s0 = c.int_bounded(0, 9);
    let a = c.int_bounded(0, 9);
    let b = c.int_bounded(0, 9);
    if abort {
        // ABORT (non-resumptive) form: the op arm returns a value DIRECTLY, never calling `resume`, so the
        // captured continuation (the rest of the handled body — the second perform + the outer `+`) is
        // DISCARDED. The handle's result is the arm value from the FIRST perform (p = a, s = s0) — a
        // distinct effects lowering (continuation drop) from the resume form. Deterministic Int64.
        write!(
            out,
            "(do (effect E (op o (-> Int64 Int64))) \
             (handle E {s0} ((o (p) s {av})) (+ (E.o {a}) (E.o {b}))))"
        )
        .ok();
    } else {
        // RESUME form: a stateful handler that RESUMES with the computed value `av` and threads the new
        // state `ns`; the body performs the op twice, folding the state across both.
        write!(
            out,
            "(do (effect E (op o (-> Int64 Int64))) \
             (handle E {s0} ((o (p) s (resume {av} {ns}))) (+ (E.o {a}) (E.o {b}))))"
        )
        .ok();
    }
}

/// A NESTED-HANDLER effect body: `(do (effect E1 …) (effect E2 …) (handle E1 0 ((o1 (p) s (resume <rv1>
/// s))) (handle E2 0 ((o2 (p) s (resume <rv2> s))) (+ (E1.o1 <a>) (E2.o2 <b>)))))` — TWO effects with the
/// E2 handle NESTED inside the E1 handle, the body performing BOTH. The E1 perform occurs INSIDE the inner
/// E2 handler frame, so it must resolve ACROSS that intervening frame to the OUTER E1 handler — the
/// multi-frame handler-stack resolution, distinct from the single-handler [`gen_effect_body`]. Both arms
/// resume (state unchanged; frame resolution is the focus). Deterministic Int64; small `0..=9` args; each
/// op resumes once so it terminates. Form choices drawn BEFORE the operand literals (cursor-exhaustion).
fn gen_effect_nested_body<C: Choice>(c: &mut C, out: &mut String) {
    let rv1 = ["(+ p 1)", "p", "(+ s p)"][c.variant(3)];
    let rv2 = ["(* p 2)", "p", "(- p 1)"][c.variant(3)];
    let a = c.int_bounded(0, 9);
    let b = c.int_bounded(0, 9);
    write!(
        out,
        "(do (effect E1 (op o1 (-> Int64 Int64))) (effect E2 (op o2 (-> Int64 Int64))) \
         (handle E1 0 ((o1 (p) s (resume {rv1} s))) \
         (handle E2 0 ((o2 (p) s (resume {rv2} s))) (+ (E1.o1 {a}) (E2.o2 {b})))))"
    )
    .ok();
}

/// An EFFECT × COLLECTION body: the handled body BUILDS a heap collection (a `(list …)`) whose elements
/// are PERFORM results, consumed to an Int64 (`List.len`, or `List.at 0` matched). Exercises the
/// effect-value × collection-marshal INTERACTION — a feature cross the single-shape effect + collection
/// arms never combine (a heap collection built from resumed values, crossing the boundary). Single-op
/// tail-resume handler (state unchanged); small `0..=9` perform args; deterministic Int64. Form + resume
/// choices drawn BEFORE the operand literals (cursor-exhaustion trap).
fn gen_effect_collection_body<C: Choice>(c: &mut C, out: &mut String) {
    let form = c.variant(2);
    let rv = ["(+ p 1)", "p", "(* p 2)"][c.variant(3)];
    let a = c.int_bounded(0, 9);
    let b = c.int_bounded(0, 9);
    let k = c.int_bounded(0, 9);
    match form {
        // a LIST of three perform results, consumed by `List.len` → 3 (the state-threaded performs run,
        // the list holds their resume values).
        0 => write!(
            out,
            "(do (effect E (op o (-> Int64 Int64))) (handle E 0 ((o (p) s (resume {rv} s))) \
             (List.len (list (E.o {a}) (E.o {b}) (E.o {k})))))"
        )
        .ok(),
        // a LIST of perform results, read at index 0 (the first perform's resume value), matched out.
        _ => write!(
            out,
            "(do (effect E (op o (-> Int64 Int64))) (handle E 0 ((o (p) s (resume {rv} s))) \
             (match (List.at (list (E.o {a}) (E.o {b})) 0) ((Some v) v) (None {k}))))"
        )
        .ok(),
    };
}

/// A MULTI-OP effect body: `(do (effect E (op o1 (-> Int64 Int64)) (op o2 (-> Int64 Int64))) (handle E 0
/// ((o1 (p) s (resume <rv1> s)) (o2 (p) s (resume <rv2> s))) (+ (E.o1 <a>) (E.o2 <b>))))` — ONE effect
/// declaring TWO operations, a handler with a per-op arm, and a body that performs BOTH. Grades the op
/// DISPATCH/selection lowering (each perform routes to its matching arm) — distinct from the single-op
/// [`gen_effect_body`]. Both arms resume (state unchanged; dispatch, not state-fold, is the focus).
/// Deterministic Int64 result; small `0..=9` args; each op resumes once so it terminates. Form choices
/// drawn BEFORE the operand literals (cursor-exhaustion trap).
fn gen_effect_multiop_body<C: Choice>(c: &mut C, out: &mut String) {
    let rv1 = ["(+ p 1)", "p", "(+ s p)"][c.variant(3)];
    let rv2 = ["(* p 2)", "p", "(- p 1)"][c.variant(3)];
    let a = c.int_bounded(0, 9);
    let b = c.int_bounded(0, 9);
    write!(
        out,
        "(do (effect E (op o1 (-> Int64 Int64)) (op o2 (-> Int64 Int64))) \
         (handle E 0 ((o1 (p) s (resume {rv1} s)) (o2 (p) s (resume {rv2} s))) (+ (E.o1 {a}) (E.o2 {b}))))"
    )
    .ok();
}

/// A `Map.lookup` body: `(match (Map.lookup <2-entry-const-map> <key>) ((Some v) v) (None <dflt>))` —
/// the keyed map read yielding `Option V`, consumed to an Int64 by matching Some/None. Half the time
/// the key is PRESENT (→ `Some` → the stored value), half DEFINITELY-ABSENT (→ `None` → the default),
/// so both arms are exercised. Keys are disjoint (`0..=9` vs `10..=19`) so the map has two distinct
/// entries; values `0..=9`. Value-comparable (the result is an Int64 the wasm-vs-rust diff grades).
fn gen_map_lookup_body<C: Choice>(c: &mut C, out: &mut String) {
    let present = c.variant(2) == 0;
    let (k1, v1) = (c.int_bounded(0, 9), c.int_bounded(0, 9));
    let (k2, v2) = (c.int_bounded(10, 19), c.int_bounded(0, 9));
    let dflt = c.int_bounded(0, 9);
    // A present key is `k1`; an absent one is `99` (outside both key ranges).
    let key = if present { k1 } else { 99 };
    write!(
        out,
        "(match (Map.lookup (Map.insert (Map.insert Map.empty {k1} {v1}) {k2} {v2}) {key}) ((Some v) v) (None {dflt}))"
    )
    .ok();
}

/// A QUANTITY body: `(Qty.value <q>)` where `<q>` is a `Qty.of` magnitude literal or a SAME-UNIT
/// `+`/`-`/`*` combination of two — extracting the magnitude as an Int64 (value-comparable, so the
/// wasm-vs-rust diff + the oracle both grade it). Qty is a whole numeric family absent from the
/// coercing grammar. HALF the magnitudes are PARENTHESIZED `(n)` — a standing regression guard for
/// #7227 (a grouped-literal Qty magnitude must adopt the sibling's fixed width; was an i64/i32
/// invalid-wasm). Same-unit ops keep the result well-typed; the magnitudes are `0..=9` so nothing
/// overflows.
fn gen_qty_body<C: Choice>(c: &mut C, out: &mut String) {
    let form = c.variant(2);
    let op = ["+", "-", "*"][c.variant(3)];
    let unit = ["meter", "second", "gram", "mole"][c.variant(4)];
    let grp = c.variant(2) == 0;
    let (a, b) = (c.int_bounded(0, 9), c.int_bounded(0, 9));
    let mag_a = if grp {
        format!("({a})")
    } else {
        format!("{a}")
    };
    match form {
        // `Qty.value` of a bare `Qty.of` literal → the magnitude.
        0 => write!(out, "(Qty.value (Qty.of {mag_a} (Unit.base #\"{unit}\")))").ok(),
        // `Qty.value` of a same-unit `+`/`-`/`*` combination → the arithmetic result.
        _ => write!(
            out,
            "(Qty.value ({op} (Qty.of {mag_a} (Unit.base #\"{unit}\")) (Qty.of {b} (Unit.base #\"{unit}\"))))"
        )
        .ok(),
    };
}

/// Append a SIZED-integer-typed expression (`main : T` for a `T` in [`SIZED_INT_TYPES`]) — self-contained
/// (no Int64 scope), so it stays type-correct without type-directed generation. One of: an ascribed
/// literal `(: <n> T)`; a checked conversion `(T.of <n>)` from a small in-range Int64 literal; or a
/// width-SAFE binary op `(<op> (: a T) (: b T))` over two `0..=9` operands (see [`SIZED_INT_OPS`] — the
/// result stays in range for every width, so it COMPILES). Reaches narrow-width arith/conversion emit.
/// Max recursion depth for [`gen_sized_expr`]. Bounded at 2 so a nested `*` stays small: with leaves
/// 0..=3, a depth-2 `*`-tree peaks at `(* (* 3 3) (* 3 3))` = 81 < 127 (`Int8`, the tightest width) — so
/// NO sized arithmetic overflows (avoids the pending signed-overflow-wrap 22-0024 divergence) and, with
/// only `+ * & | ^` and non-negative leaves, nothing underflows an unsigned width either.
const SIZED_DEPTH: usize = 2;

/// A RECURSIVE sized-int expression of ONE type `T`: at depth 0 (or ~1/3 above) a leaf — an ascribed
/// literal `(: n T)` or a checked `(T.of n)` — otherwise `(op <sized-expr> <sized-expr>)` for `op` in
/// [`SIZED_INT_OPS`] (`+ * & | ^`), each operand recursing at `depth-1`. Both operands share type `T` so
/// the whole expression is `T` and type-checks. Reaches NESTED narrow-width arithmetic (seq-190: int
/// operators must recurse over sub-expressions, not just leaf literals).
fn gen_sized_expr<C: Choice>(c: &mut C, depth: usize, t: &str, out: &mut String) {
    if depth == 0 || c.variant(3) == 2 {
        if c.variant(2) == 0 {
            write!(out, "(: {} {t})", c.int_bounded(0, 3)).ok();
        } else {
            write!(out, "({t}.of {})", c.int_bounded(0, 3)).ok();
        }
        return;
    }
    // Pick the op BEFORE recursing (variant-ordering).
    let op = SIZED_INT_OPS[c.variant(SIZED_INT_OPS.len())];
    write!(out, "({op} ").ok();
    gen_sized_expr(c, depth - 1, t, out);
    out.push(' ');
    gen_sized_expr(c, depth - 1, t, out);
    out.push(')');
}

fn gen_sized_int_body<C: Choice>(c: &mut C, out: &mut String) {
    let t = SIZED_INT_TYPES[c.variant(SIZED_INT_TYPES.len())];
    gen_sized_expr(c, SIZED_DEPTH, t, out);
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

/// Pick a HASHABLE scalar type for a `#set` element / `#map` key — `Int64`, a sized-int, or `Bool`.
/// EXCLUDES floats (NaN / float-equality make them unsuitable set members + map keys). A map VALUE may be
/// any scalar (no hashing), so it uses [`pick_scalar_ty`] instead.
fn pick_hashable_ty<C: Choice>(c: &mut C) -> ScalarTy {
    match c.variant(3) {
        0 => ScalarTy::Int64,
        1 => ScalarTy::Bool,
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
/// `(List.len …)`, an Option `match`, a `Result` `match`, a sum-match over a COMPOUND payload consumed
/// in-arm, or a native `#set`/`#map` literal + its `.len`. The generator builds compounds elsewhere but
/// rarely CONSUMES them; this exercises the distinct extract / project / list-len / sum-match / set-map
/// consumption emit (where the S52 closure buckets lived). Self-contained + type-correct (payload types
/// drive the leaves; every match arm is the same type). Reaches consumption lowering the construction
/// arms never hit.
fn gen_compound_consume<C: Choice>(c: &mut C, out: &mut String) {
    match c.variant(7) {
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
        3 => {
            let t = pick_scalar_ty(c);
            out.push_str("(match (Some ");
            gen_scalar_leaf(c, t, out);
            out.push_str(") ((Some x) x) ((None) ");
            gen_scalar_leaf(c, t, out);
            out.push_str("))");
        }
        // Result match: (match (: (Ok/Err <t>) (Result t t)) ((Ok x) x) ((Err e) e)) — a two-variant sum
        // (a sum value needs the type annotation), each arm returns its payload of the SAME type `t`, so the
        // join is type-correct. The scrutinee constructor is Ok OR Err, exercising both sum-match arms.
        4 => {
            let t = pick_scalar_ty(c);
            let ctor = if c.variant(2) == 0 { "Ok" } else { "Err" };
            write!(out, "(match (: ({ctor} ").ok();
            gen_scalar_leaf(c, t, out);
            write!(
                out,
                ") (Result {t0} {t0})) ((Ok x) x) ((Err e) e))",
                t0 = t.name()
            )
            .ok();
        }
        // Option match over a COMPOUND payload, CONSUMED to a scalar in the Some arm (project a tuple/record
        // element, or `List.len`); None returns a same-typed scalar default so the arms join. Exercises
        // binding a native compound FROM a sum-match arm + in-arm consumption (tuple-in-Some, record-in-Some,
        // list-in-Some) — the fresh M2 native ctor-leaf codegen the scalar-payload matches never reach.
        5 => match c.variant(3) {
            // (match (Some (tuple <a> <b>)) ((Some x) (. x i)) ((None) <default : elt-ty>))
            0 => {
                let (a, b) = (pick_scalar_ty(c), pick_scalar_ty(c));
                let idx = c.variant(2);
                out.push_str("(match (Some (tuple ");
                gen_scalar_leaf(c, a, out);
                out.push(' ');
                gen_scalar_leaf(c, b, out);
                write!(out, ")) ((Some x) (. x {idx})) ((None) ").ok();
                gen_scalar_leaf(c, if idx == 0 { a } else { b }, out);
                out.push_str("))");
            }
            // (match (Some (record (= a <>) (= b <>))) ((Some x) (. x f)) ((None) <default : field-ty>))
            1 => {
                let (a, b) = (pick_scalar_ty(c), pick_scalar_ty(c));
                let pick_a = c.variant(2) == 0;
                out.push_str("(match (Some (record (= a ");
                gen_scalar_leaf(c, a, out);
                out.push_str(") (= b ");
                gen_scalar_leaf(c, b, out);
                let field = if pick_a { "a" } else { "b" };
                write!(out, "))) ((Some x) (. x {field})) ((None) ").ok();
                gen_scalar_leaf(c, if pick_a { a } else { b }, out);
                out.push_str("))");
            }
            // (match (Some (list <t> <t> <t>)) ((Some x) (List.len x)) ((None) <Int64>)) — List.len : Int64
            _ => {
                let t = pick_scalar_ty(c);
                out.push_str("(match (Some (list ");
                gen_scalar_leaf(c, t, out);
                out.push(' ');
                gen_scalar_leaf(c, t, out);
                out.push(' ');
                gen_scalar_leaf(c, t, out);
                out.push_str(")) ((Some x) (List.len x)) ((None) ");
                gen_scalar_leaf(c, ScalarTy::Int64, out);
                out.push_str("))");
            }
        },
        // A native M2 `#set(…)` / `#map(…)` literal (value) or its `.len` (consume to Int64). Fills the
        // Set/Map codegen gap (leaf kinds 23/24): the generator built tuple/record/list/Option/Result but
        // never a Set or Map. Keys/elements are distinct Int64 literals (hashable, no dup-key/NaN hazard),
        // so the literals always COMPILE. Map values value-GRADE (oracle #5164/#5176); Set values + `.len`
        // currently SKIP in the oracle (a modelled gap, not a bug) but still exercise native set/map
        // construction + codec + emit (and the crash / InvalidWasm oracle).
        // Pick the sub-variant BEFORE consuming element choices — otherwise a short entropy seed exhausts
        // the cursor on the elements and `variant` always defaults to 0 (never reaching map).
        _ => match c.variant(4) {
            // `#set(…)` value or `(Set.len …)` over a homogeneous HASHABLE-typed element list (Int64 /
            // sized-int / Bool; floats excluded). Dedup-safe, so the 3 leaves need not be distinct.
            f @ (0 | 1) => {
                let h = pick_hashable_ty(c);
                if f == 1 {
                    out.push_str("(Set.len ");
                }
                out.push_str("#set(");
                gen_scalar_leaf(c, h, out);
                out.push(' ');
                gen_scalar_leaf(c, h, out);
                out.push(' ');
                gen_scalar_leaf(c, h, out);
                out.push(')');
                if f == 1 {
                    out.push(')');
                }
            }
            // `#map(…)` value or `(Map.len …)` — DISTINCT Int64 keys (disjoint ranges) + a homogeneous
            // value of ANY scalar type (values need no hashing, so `pick_scalar_ty` incl floats/bool).
            f => {
                let (k1, k2) = (c.int_bounded(0, 9), c.int_bounded(20, 29));
                let v = pick_scalar_ty(c);
                if f == 3 {
                    out.push_str("(Map.len ");
                }
                write!(out, "#map((= {k1} ").ok();
                gen_scalar_leaf(c, v, out);
                write!(out, ") (= {k2} ").ok();
                gen_scalar_leaf(c, v, out);
                out.push_str("))");
                if f == 3 {
                    out.push(')');
                }
            }
        },
    }
}

/// A BIGINT (`N`-suffixed) / RATIONAL (`R`-suffixed) body — a literal, a binary op, or a beyond-`i64`
/// BigInt literal. BigInt is arbitrary-precision (`+`/`-`/`*` never overflow → always on the compile
/// path); Rational operands use `1..=9` (a nonzero denominator, so `/` never `/0`-traps). Reaches the
/// bignum/rational value + arith lowering (a distinct numeric family from Int64/Float/sized). NOTE: the
/// Lean oracle does not model BigInt/Rational yet, so these currently SKIP in the value differential —
/// but they COMPILE cleanly (crash-fuzzed via `cdz_smith_gen_never_panics`) and will grade once modelled.
fn gen_bignum_body<C: Choice>(c: &mut C, out: &mut String) {
    // Pick the FORM before consuming the operand choices — else a short entropy seed exhausts the cursor
    // on `a`/`b` and `variant` always defaults to 0 (never reaching the Rational forms).
    let form = c.variant(6);
    let (a, b) = (c.int_bounded(1, 9), c.int_bounded(1, 9));
    match form {
        // A BigInt literal `<n>N`.
        0 => write!(out, "{a}N").ok(),
        // A BigInt binary op — `+`/`-`/`*` only (arbitrary precision: no overflow, no `/0`).
        1 => {
            let op = ["+", "-", "*"][c.variant(3)];
            write!(out, "({op} {a}N {b}N)").ok()
        }
        // A BEYOND-i64 BigInt literal (25 nines > i64::MAX) — exercises the big-magnitude codec + emit.
        2 => write!(out, "{}N", "9".repeat(25)).ok(),
        // A Rational literal `<n>R` (= n/1).
        3 => write!(out, "{a}R").ok(),
        // A BigInt/Rational COMPARISON → Bool: `(= / < / > / <= / >= x y)` over two BigInt (`N`) or
        // two Rational (`R`) values — arbitrary-precision + exact-rational ORDERING (a Bool result),
        // a surface the arith arms never reached (none COMPARED). Rational ordering is graded by the
        // oracle's compareVals fold (#7106); value-comparable (the wasm-vs-rust diff grades the Bool).
        4 => {
            let op = ["=", "<", ">", "<=", ">="][c.variant(5)];
            let suffix = if c.variant(2) == 0 { "N" } else { "R" };
            write!(out, "({op} {a}{suffix} {b}{suffix})").ok()
        }
        // A Rational binary op — `b` in `1..=9` so `/` has a nonzero denominator.
        _ => {
            let op = ["+", "-", "*", "/"][c.variant(4)];
            write!(out, "({op} {a}R {b}R)").ok()
        }
    };
}

/// A PARTIAL-APPLICATION / currying body: a LOCAL def applied to FEWER args than its arity yields a
/// CLOSURE over the remaining params, which is then completed. Reaches the def-call under-arity dispatch
/// and the `applyClosure` currying that #5488 grades (a `(f a)` with `f` under-applied becomes a closure
/// capturing `a`, later completed with the rest). Self-contained (a nested `(do (def …) …)` with small
/// Int64 literals), so it stays type-correct and on the compile path. Verified compiles; #5488 verified
/// the value-shapes GRADE — `(sub 10)` then `(f 3)` is 7; chained `(((add3 1) 2) 3)` is 6; `(add3 1 2)`
/// then `(g 3)` is 6.
fn gen_partial_application_body<C: Choice>(c: &mut C, out: &mut String) {
    // Pick the FORM before consuming the operand choices — else a short entropy seed exhausts the cursor
    // on the args and `variant` always defaults to 0 (never reaching the chained/2-arg forms).
    let form = c.variant(3);
    let (a, b, k) = (
        c.int_bounded(0, 99),
        c.int_bounded(0, 99),
        c.int_bounded(0, 99),
    );
    match form {
        // 2-ary def, partial to 1 arg via `let`, then complete: `(let ((f (pa A))) (f B))`.
        0 => write!(
            out,
            "(do (def (pa a b) (- a b)) (let ((f (pa {a}))) (f {b})))"
        )
        .ok(),
        // 3-ary def, CHAINED currying one arg at a time: `(((pa3 A) B) K)`.
        1 => write!(
            out,
            "(do (def (pa3 a b c) (+ (+ a b) c)) (((pa3 {a}) {b}) {k}))"
        )
        .ok(),
        // 3-ary def, partial to 2 args via `let`, then complete: `(let ((g (pa3 A B))) (g K))`.
        _ => write!(
            out,
            "(do (def (pa3 a b c) (+ (+ a b) c)) (let ((g (pa3 {a} {b}))) (g {k})))"
        )
        .ok(),
    };
}

/// A HIGHER-ORDER body: a function VALUE — either a `(fn …)` lambda or a NAMED def used by name — is
/// passed as an ARGUMENT to another def, which applies it inside its body. Reaches the applyClosure path
/// over a CLOSURE-VALUED parameter (the closure flows through a param, then is called), distinct from the
/// partial-application arm (which under-applies a def directly). Self-contained (a nested `(do (def …) …)`
/// with small Int64 literals), so it stays type-correct and on the compile path. Verified compiles;
/// exact-arity applyClosure is the base closure path the oracle already grades.
fn gen_higher_order_body<C: Choice>(c: &mut C, out: &mut String) {
    // Pick the FORM before consuming the operand choices (variant-ordering: a short seed must still reach
    // the twice/named forms).
    let form = c.variant(3);
    let (a, b) = (c.int_bounded(1, 20), c.int_bounded(0, 20));
    match form {
        // Pass a LAMBDA to a 1-shot applier: `(apply (fn (y) (+ y A)) B)`.
        0 => write!(
            out,
            "(do (def (apply f x) (f x)) (apply (fn (y) (+ y {a})) {b}))"
        )
        .ok(),
        // Pass a LAMBDA to a fn that applies it TWICE: `(twice (fn (y) (* y A)) B)`.
        1 => write!(
            out,
            "(do (def (twice g n) (g (g n))) (twice (fn (y) (* y {a})) {b}))"
        )
        .ok(),
        // Pass a NAMED def by name as a fn value: `(apply inc B)`.
        _ => write!(
            out,
            "(do (def (inc y) (+ y {a})) (def (apply f x) (f x)) (apply inc {b}))"
        )
        .ok(),
    };
}

/// A DISCARD body: `(do <stmt> <tail>)` where `<stmt>` is a non-def LEADING statement whose value is
/// COMPUTED then DISCARDED (CDZ0307 — a non-final block form is evaluated only for effect), and `<tail>`
/// is the block's value. Reaches the sequencing / dead-value drop lowering that #5507 grades (a non-def
/// do-statement is discarded, its trap elided, continue to the tail). Varies the discarded value KIND — a
/// scalar, a heap compound (tuple/list, exercising build-then-drop), a bool — so the drop covers multiple
/// reprs. All discards are NON-trapping constants (a const trapping discard is CDZ0304, a decline), so it
/// stays on the graded compile path; the tail is a small Int64 arithmetic.
fn gen_discard_body<C: Choice>(c: &mut C, out: &mut String) {
    // Pick the discarded-value FORM before consuming the operand choices (variant-ordering).
    let form = c.variant(4);
    let (a, b) = (c.int_bounded(0, 99), c.int_bounded(0, 99));
    match form {
        // Discard a scalar literal.
        0 => write!(out, "(do {a} (+ {a} {b}))").ok(),
        // Discard a heap TUPLE (build-then-drop).
        1 => write!(out, "(do (tuple {a} {b}) (+ {a} {b}))").ok(),
        // Discard a heap LIST (build-then-drop).
        2 => write!(out, "(do (list {a} {b}) (+ {a} {b}))").ok(),
        // Discard a BOOL comparison.
        _ => write!(out, "(do (< {a} {b}) (+ {a} {b}))").ok(),
    };
}

/// A FLOAT-ORDERING body: `main : Bool` = a float ordering comparison `(<rel> <flit> <flit>)` for `<rel>`
/// in `< > <= >=`, over two same-width float literals (Float64 or Float32). Reaches the IEEE float
/// ordering (#5519) as the RETURNED Bool value — my `gen_float` only emitted float relations inside `if`
/// GUARDS (the returned value stayed a float), so the ordering result itself was never the graded value.
fn gen_float_ordering_body<C: Choice>(c: &mut C, out: &mut String) {
    // Pick the width + relation + special-operand choice BEFORE the operand literals (variant-ordering).
    let is_f32 = c.variant(2) == 1;
    let rel = ["<", ">", "<=", ">="][c.variant(4)];
    // For Float64, the LEFT operand is one of three kinds (Float32 has no nan/Infinity constructor → a
    // finite literal): `(Float64.nan)` — every ordering with NaN is UNORDERED (all false, #5519 NaN-
    // unordered); `(Float64.Infinity)` — +inf is ORDERED above every finite (inf > x true, inf < x false;
    // #5563 models it); or a finite literal.
    let left_kind = if is_f32 { 2 } else { c.variant(3) };
    write!(out, "({rel} ").ok();
    match left_kind {
        0 => out.push_str("(Float64.nan)"),
        1 => out.push_str("(Float64.Infinity)"),
        _ => gen_float_lit(c, is_f32, out),
    }
    out.push(' ');
    gen_float_lit(c, is_f32, out);
    out.push(')');
}

/// A COMPOUND-KEYED collection body: a set or map whose KEYS are `(tuple …)` compounds (not scalars),
/// consumed to Int64 via `Set.len` / `Map.len` / `Set.insert`. Reaches the structural total order over
/// COMPOUND values + `Set.insert` that #5540 grades — my scalar-only `pick_hashable_ty` set/map arm never
/// used a compound key. The two keys have DISJOINT first elements (0..=9 vs 20..=29), so they are always
/// distinct compounds → a deterministic `.len` (no dedup ambiguity).
fn gen_compound_keyed_collection_body<C: Choice>(c: &mut C, out: &mut String) {
    // Pick the FORM + key-KIND before consuming the operand choices (variant-ordering).
    let form = c.variant(3);
    let key_kind = c.variant(4);
    let (a, b) = (c.int_bounded(0, 9), c.int_bounded(0, 9));
    let (cc, d) = (c.int_bounded(20, 29), c.int_bounded(20, 29));
    // The compound KEY is a tuple / record / NESTED tuple / list — #5540's structural total order over
    // compound values covers all of them.
    let key = |x: i64, y: i64| match key_kind {
        0 => format!("(record (= a {x}) (= b {y}))"),
        1 => format!("(tuple (tuple {x} {y}) {x})"),
        2 => format!("(list {x} {y})"),
        _ => format!("(tuple {x} {y})"),
    };
    let (k1, k2) = (key(a, b), key(cc, d));
    match form {
        // A set with two DISTINCT compound keys → `Set.len` = 2.
        0 => write!(out, "(Set.len #set({k1} {k2}))").ok(),
        // `Set.insert` a distinct compound key into a one-key set → `Set.len` = 2.
        1 => write!(out, "(Set.len (Set.insert #set({k1}) {k2}))").ok(),
        // A map keyed by two DISTINCT compound keys → `Map.len` = 2.
        _ => write!(out, "(Map.len #map((= {k1} {a}) (= {k2} {cc})))").ok(),
    };
}

/// A FLOAT-KEYED collection body: a set or map whose KEYS are Float64/Float32 values, consumed to Int64
/// via `Set.len` / `Map.len`. Reaches the float-carrying keys with canonical-bit order + canonical key
/// equality that #5556 grades — my scalar `pick_hashable_ty` deliberately EXCLUDED floats as keys. The two
/// keys use DISJOINT magnitudes (0..=9 vs 20..=29), so they are always distinct → deterministic `.len`.
/// Includes a `(Float64.nan)`-key form: #5570 fixed the canonical NaN key (qNaN bits, not 0) so it no
/// longer collides with `0.0` — the earlier oracle dedup bug I (cdz-smith) reported at S157.
fn gen_float_keyed_collection_body<C: Choice>(c: &mut C, out: &mut String) {
    // Pick the FORM before consuming the operand choices (variant-ordering).
    let form = c.variant(4);
    let (a, b) = (c.int_bounded(0, 9), c.int_bounded(20, 29));
    match form {
        // A Float64-keyed set of two distinct keys → `Set.len` = 2.
        0 => write!(out, "(Set.len #set({a}.0 {b}.0))").ok(),
        // A Float64-keyed map of two distinct keys → `Map.len` = 2.
        1 => write!(out, "(Map.len #map((= {a}.0 {a}) (= {b}.0 {b})))").ok(),
        // A set with a `(Float64.nan)` key plus a finite key → `Set.len` = 2 (the canonical qNaN key is
        // distinct from every finite; #5570 fixed the earlier NaN-vs-0.0 dedup collision I reported).
        2 => write!(out, "(Set.len #set((Float64.nan) {a}.0))").ok(),
        // A Float32-keyed set of two distinct keys → `Set.len` = 2.
        _ => write!(out, "(Set.len #set((: {a}.0 Float32) (: {b}.0 Float32)))").ok(),
    };
}

/// A STRING-op body over a small fixed nonempty string: `String.byte-len` (→ Int64) / `String.scalar-at`
/// at index 0 (→ a scalar) / `String.concat` / `String.slice [0,1)` / a bare string literal (→ String
/// value). Reaches the String value + String-op lowering (byte-len/scalar-at/concat/slice/literal codec +
/// emit) — a family the Int64/float/compound grammar never touched. Indices are in-bounds by construction
/// (every pool string is nonempty), so it stays on the compile path.
fn gen_string_body<C: Choice>(c: &mut C, out: &mut String) {
    // Pick the string + FORM before writing (variant-ordering).
    let s = ["a", "ab", "abc", "hello"][c.variant(4)];
    match c.variant(7) {
        // A byte length → Int64.
        0 => write!(out, "(String.byte-len \"{s}\")").ok(),
        // A Unicode scalar at index 0 → a scalar.
        1 => write!(out, "(String.scalar-at \"{s}\" 0)").ok(),
        // Concatenation → a String value.
        2 => write!(out, "(String.concat \"{s}\" \"{s}\")").ok(),
        // A `[0,1)` slice → a String value (valid for any nonempty string).
        3 => write!(out, "(String.slice \"{s}\" 0 1)").ok(),
        // A string COMPARISON → Bool: equality `(= s s2)` or ordering `(< / > / <= / >= s s2)` —
        // string value equality + total-order lowering (a Bool result), a surface the byte/concat/
        // slice ops never reached (none compared TWO strings). Value-comparable (the wasm-vs-rust
        // diff grades the Bool). Both operands from the same small set so shapes repeat + agree.
        4 => {
            let s2 = ["a", "ab", "abc", "hello"][c.variant(4)];
            let op = ["=", "<", ">", "<=", ">="][c.variant(5)];
            write!(out, "({op} \"{s}\" \"{s2}\")").ok()
        }
        // A CHAR (Unicode scalar) COMPARISON → Bool: compare two `String.scalar-at` results with
        // `=`/`<`/`>`/`<=`/`>=` — Char value equality + ordering (a Bool result), graded by the
        // oracle's Char-ordering fold (#7106). Char LITERALS aren't a surface syntax (`'a'` → CDZ0101),
        // so `scalar-at` is the only way to obtain a Char value to compare.
        5 => {
            let s2 = ["a", "ab", "abc", "hello"][c.variant(4)];
            let op = ["=", "<", ">", "<=", ">="][c.variant(5)];
            write!(
                out,
                "({op} (String.scalar-at \"{s}\" 0) (String.scalar-at \"{s2}\" 0))"
            )
            .ok()
        }
        // A bare string literal value.
        _ => write!(out, "\"{s}\"").ok(),
    };
}

/// A BYTES-op body: `Bytes.len` (→ Int64) / `Bytes.at` at index 0 (→ a byte scalar) or a `b"…"` literal /
/// `Bytes.of (list …)` / `Bytes.concat` (→ a Bytes value) over small fixed nonempty byte strings. Reaches
/// the Bytes value + Bytes-op lowering — a construct family distinct from String and the numeric/compound
/// grammar. `Bytes.of` elements are 0..=255 (valid bytes); indices are in-bounds by construction.
fn gen_bytes_body<C: Choice>(c: &mut C, out: &mut String) {
    // Pick the FORM + string + byte operands before writing (variant-ordering).
    let form = c.variant(6);
    let s = ["a", "ab", "abc"][c.variant(3)];
    let (x, y, z) = (
        c.int_bounded(0, 255),
        c.int_bounded(0, 255),
        c.int_bounded(0, 255),
    );
    match form {
        // A byte length → Int64.
        0 => write!(out, "(Bytes.len b\"{s}\")").ok(),
        // A byte at index 0 → a scalar.
        1 => write!(out, "(Bytes.at b\"{s}\" 0)").ok(),
        // A `b"…"` literal → a Bytes value.
        2 => write!(out, "b\"{s}\"").ok(),
        // `Bytes.of` a small byte list → a Bytes value.
        3 => write!(out, "(Bytes.of (list {x} {y} {z}))").ok(),
        // A bytes COMPARISON → Bool: equality `(= b b2)` or ordering `(< / > / <= / >= b b2)` —
        // Bytes value equality + lexicographic total-order lowering (a Bool result), a surface the
        // len/at/of/concat ops never reached (none COMPARED two byte values). Value-comparable, and
        // graded symbolically since v-lean-oracle #7106 models Bytes ordering.
        4 => {
            let s2 = ["a", "ab", "abc"][c.variant(3)];
            let op = ["=", "<", ">", "<=", ">="][c.variant(5)];
            write!(out, "({op} b\"{s}\" b\"{s2}\")").ok()
        }
        // Concatenation → a Bytes value.
        _ => write!(out, "(Bytes.concat b\"{s}\" b\"{s}\")").ok(),
    };
}

/// A NESTED / DEEPER compound body: `List.at` (→ scalar) / `List.concat` (→ a list value), or a
/// compound-of-compounds VALUE — a tuple of two lists, a list of two tuples, or a record whose fields are
/// a tuple and a list. Reaches DEEPER structural shapes than the flat single-level tuple/list/record arms
/// (the oracle grades compound values structurally, #5540). All Int64 leaves; `List.at` index in-bounds.
fn gen_nested_compound_body<C: Choice>(c: &mut C, out: &mut String) {
    // Pick the FORM before consuming operand choices (variant-ordering).
    let form = c.variant(5);
    let (a, b, x, y) = (
        c.int_bounded(0, 99),
        c.int_bounded(0, 99),
        c.int_bounded(0, 99),
        c.int_bounded(0, 99),
    );
    match form {
        // `List.at` a 3-element list at an in-bounds index → an Int64 element.
        0 => {
            let i = c.variant(3);
            write!(out, "(List.at (list {a} {b} {x}) {i})").ok()
        }
        // `List.concat` two lists → a list value.
        1 => write!(out, "(List.concat (list {a} {b}) (list {x} {y}))").ok(),
        // A tuple of two lists → a nested compound value.
        2 => write!(out, "(tuple (list {a} {b}) (list {x} {y}))").ok(),
        // A list of two tuples → a nested compound value.
        3 => write!(out, "(list (tuple {a} {b}) (tuple {x} {y}))").ok(),
        // A record whose fields are a tuple and a list → a nested compound value.
        _ => write!(out, "(record (= a (tuple {a} {b})) (= b (list {x} {y})))").ok(),
    };
}

/// A NESTED-SUM body: a sum value that WRAPS another sum or compound, ascribed to its (nested) type —
/// `(Some (Some n))`, `(Ok (Some n))`, `(Some (tuple a b))`, `(Some (list …))`. Deeper sum-wrapping than
/// the flat Some/Ok/Err arms; the ascription is required (a bare nested `Some`/`Ok` is type-ambiguous).
fn gen_nested_sum_body<C: Choice>(c: &mut C, out: &mut String) {
    // Pick the FORM before consuming operand choices (variant-ordering).
    let form = c.variant(4);
    let (a, b, k) = (
        c.int_bounded(0, 99),
        c.int_bounded(0, 99),
        c.int_bounded(0, 99),
    );
    match form {
        // Option of Option.
        0 => write!(out, "(: (Some (Some {a})) (Option (Option Int64)))").ok(),
        // Result whose Ok payload is an Option.
        1 => write!(out, "(: (Ok (Some {a})) (Result (Option Int64) Int64))").ok(),
        // Option of a tuple.
        2 => write!(
            out,
            "(: (Some (tuple {a} {b})) (Option (Tuple Int64 Int64)))"
        )
        .ok(),
        // Option of a list.
        _ => write!(out, "(: (Some (list {a} {b} {k})) (Option (List Int64)))").ok(),
    };
}

/// An INT CROSS-WIDTH CONVERSION body: `(<Target>.of (: <v> <Source>))` for any Source/Target pair in
/// [`SIZED_INT_TYPES`] (widen, narrow, cross-sign, or identity). The value is 0..=100 — in range for EVERY
/// target (the smallest max is `Int8`'s 127) — so the checked conversion never traps/declines, keeping it
/// on the graded path. Reaches the int-to-int conversion codegen the sized-int arm (literal ascription +
/// `T.of` from Int64 only) never exercised.
fn gen_int_conversion_body<C: Choice>(c: &mut C, out: &mut String) {
    let src = SIZED_INT_TYPES[c.variant(SIZED_INT_TYPES.len())];
    let tgt = SIZED_INT_TYPES[c.variant(SIZED_INT_TYPES.len())];
    let v = c.int_bounded(0, 100);
    write!(out, "({tgt}.of (: {v} {src}))").ok();
}

/// A WIDER-ARITY compound body: a 3-/4-element tuple, a 3-/4-field record, or a projection of one element
/// out of a wide tuple/record. Exercises wider construction + projection LAYOUTS (more fields/offsets) than
/// the 2-field tuple/record arms. All Int64 leaves; the projection index/field is in-bounds.
fn gen_wide_compound_body<C: Choice>(c: &mut C, out: &mut String) {
    // Pick the FORM before consuming operand choices (variant-ordering).
    let form = c.variant(5);
    let (a, b, x, y) = (
        c.int_bounded(0, 99),
        c.int_bounded(0, 99),
        c.int_bounded(0, 99),
        c.int_bounded(0, 99),
    );
    match form {
        // A 3-tuple value.
        0 => write!(out, "(tuple {a} {b} {x})").ok(),
        // A 4-tuple value.
        1 => write!(out, "(tuple {a} {b} {x} {y})").ok(),
        // A 3-field record value.
        2 => write!(out, "(record (= a {a}) (= b {b}) (= c {x}))").ok(),
        // Project the 3rd element of a 3-tuple → a scalar.
        3 => write!(out, "(. (tuple {a} {b} {x}) 2)").ok(),
        // Project field `c` of a 3-field record → a scalar.
        _ => write!(out, "(. (record (= a {a}) (= b {b}) (= c {x})) c)").ok(),
    };
}

/// Max recursion depth for [`gen_bool_expr`] — bounds the deep bool-shape space.
const BOOL_DEPTH: usize = 3;

/// A RECURSIVE bool expression: at depth 0 a leaf comparison `(<rel> a b)`; above that, `and`/`or` over two
/// bool SUB-EXPRESSIONS or `not` of one (each recursing at `depth-1`), or a leaf. Reaches the DEEP bool
/// grammar (nested `and`/`or`/`not` over comparisons + nested bools) instead of a couple of shallow shapes.
fn gen_bool_expr<C: Choice>(c: &mut C, depth: usize, out: &mut String) {
    // At depth 0 (or ~1/4 of the time above it) emit a LEAF comparison.
    if depth == 0 || c.variant(4) == 3 {
        let rels = ["<", ">", "<=", ">=", "="];
        let rel = rels[c.variant(5)];
        let (a, b) = (c.int_bounded(0, 20), c.int_bounded(0, 20));
        write!(out, "({rel} {a} {b})").ok();
        return;
    }
    // Pick the combinator BEFORE recursing (variant-ordering).
    match c.variant(3) {
        0 => {
            out.push_str("(and ");
            gen_bool_expr(c, depth - 1, out);
            out.push(' ');
            gen_bool_expr(c, depth - 1, out);
            out.push(')');
        }
        1 => {
            out.push_str("(or ");
            gen_bool_expr(c, depth - 1, out);
            out.push(' ');
            gen_bool_expr(c, depth - 1, out);
            out.push(')');
        }
        _ => {
            out.push_str("(not ");
            gen_bool_expr(c, depth - 1, out);
            out.push(')');
        }
    }
}

/// A BOOLEAN-LOGIC body: a RECURSIVE `and`/`or`/`not` expression over integer comparisons (depth-bounded by
/// [`BOOL_DEPTH`]) — a Bool result exercising the short-circuit boolean combinator lowering at DEPTH (nested
/// `and`/`or`/`not` over bool sub-expressions), not just a couple of shallow shapes.
fn gen_bool_logic_body<C: Choice>(c: &mut C, out: &mut String) {
    gen_bool_expr(c, BOOL_DEPTH, out);
}

/// A SIZED-INT SHIFT body: `(<< (: a T) s)` / `(>> (: a T) s)` (and a nested `(& (<< …) (: b T))`) for a
/// sized-int type `T`. The shift count `s` is 0..=3 and the operands 0..=3, so no shift is out of range
/// (valid even for the 8-bit widths) and no left-shift overflows — keeping it on the graded path. Reaches
/// the NARROW-WIDTH shift codegen the sized-int arm (which only emitted `+ * & | ^`) never did.
fn gen_sized_shift_body<C: Choice>(c: &mut C, out: &mut String) {
    // Pick the FORM + type before the operands (variant-ordering).
    let form = c.variant(3);
    let t = SIZED_INT_TYPES[c.variant(SIZED_INT_TYPES.len())];
    let (a, b, s) = (
        c.int_bounded(0, 3),
        c.int_bounded(0, 3),
        c.int_bounded(0, 3),
    );
    match form {
        // Shift left.
        0 => write!(out, "(<< (: {a} {t}) {s})").ok(),
        // Shift right.
        1 => write!(out, "(>> (: {a} {t}) {s})").ok(),
        // Nested shift then bitwise-and (same width).
        _ => write!(out, "(& (<< (: {a} {t}) {s}) (: {b} {t}))").ok(),
    };
}

/// A MUTUALLY-RECURSIVE program: two TOP-LEVEL sibling defs that call EACH OTHER, plus a param-less `main`
/// that calls one of them. Returns `(defs, main_body)`. Emitted TOP-LEVEL (via [`build_program`]) — NOT as a
/// nested-`do` body — because the oracle captures a LOCAL fn def's closure env EAGERLY (excluding itself and
/// later siblings), so a local self-/mutually-recursive call is unbound → SKIP (Eval.lean); only TOP-LEVEL
/// defs are name-resolved for recursion, so the mutual call graph GRADES. Both recursions decrement to a
/// `(<= n 0)` base so they terminate. Reaches a mutual call graph no single self-recursive helper produces.
fn gen_mutual_recursion_body<C: Choice>(c: &mut C) -> (String, String) {
    // Pick the FORM before consuming the operand choice (variant-ordering).
    let form = c.variant(2);
    let n = c.int_bounded(0, 12);
    match form {
        // even/odd parity — a Bool result: `ev` calls `od` and vice-versa.
        0 => (
            "(def (ev n) (if (<= n 0) true (od (- n 1)))) \
             (def (od n) (if (<= n 0) false (ev (- n 1))))"
                .to_string(),
            format!("(ev {n})"),
        ),
        // ping/pong with an accumulator — an Int64 result: each hop adds a different amount.
        _ => (
            "(def (pinga n acc) (if (<= n 0) acc (pongb (- n 1) (+ acc 1)))) \
             (def (pongb n acc) (if (<= n 0) acc (pinga (- n 1) (+ acc 2))))"
                .to_string(),
            format!("(pinga {n} 0)"),
        ),
    }
}

/// A `?`/`try` body: `main` is a FALLIBLE fn (a `Result`/`Option` boundary, established by the body
/// ascription), and `(try …)` UNWRAPS a success ctor (`Ok`/`Some` → the payload) or SHORT-CIRCUITS a
/// failure ctor (`Err`/`None` → the fn's own value). Shape `(: (let ((x (try (<ctor> …)))) (<wrap> x))
/// (<sum-ty>))`: the outer ascription is REQUIRED — a bare `(try (Ok 5))` under a non-fallible `main` is
/// CDZ0230 (no boundary to break to); the `let` binds the unwrapped payload; the tail re-wraps it in the
/// SAME sum. Reaches the `?` success-fold + the `Core::Block`/`Break` short-circuit lowering that #5249
/// grades — a surface no other body hits. A CONSTANT operand (a runtime `?` still declines), so it stays
/// on the compile path. Payload type is any scalar (verified: Int64/sized/float/bool all compile).
fn gen_try_body<C: Choice>(c: &mut C, out: &mut String) {
    let t = pick_scalar_ty(c);
    let ty = t.name();
    match c.variant(4) {
        // Result Ok SUCCESS → `(try (Ok <t>))` unwraps to the payload → `(Ok x)`.
        0 => {
            out.push_str("(: (let ((x (try (Ok ");
            gen_scalar_leaf(c, t, out);
            write!(out, ")))) (Ok x)) (Result {ty} {ty}))").ok();
        }
        // Result Err SHORT-CIRCUIT → `(try (Err <t>))` breaks the boundary → the fn's value is `(Err <t>)`.
        1 => {
            out.push_str("(: (let ((x (try (Err ");
            gen_scalar_leaf(c, t, out);
            write!(out, ")))) (Ok x)) (Result {ty} {ty}))").ok();
        }
        // Option Some SUCCESS → `(try (Some <t>))` unwraps → `(Some x)`.
        2 => {
            out.push_str("(: (let ((x (try (Some ");
            gen_scalar_leaf(c, t, out);
            write!(out, ")))) (Some x)) (Option {ty}))").ok();
        }
        // Option None SHORT-CIRCUIT → `(try None)` breaks the boundary → the fn's value is `None`.
        _ => {
            write!(out, "(: (let ((x (try None))) (Some x)) (Option {ty}))").ok();
        }
    };
}

/// A DESTRUCTURING pattern-match body: `match` a native `tuple`/`record` (or a `Some`-wrapped tuple) with
/// a PATTERN that binds its components, returning one bound component (type `t`). Exercises the native
/// compound-PATTERN lowering (#5257 round-trip) + `MatchSum`/binder-extract emit — distinct from the
/// projection (`(. c i)`) and sum-ctor matches. Every element is the SAME scalar type `t` and the arm
/// returns a binder of that type, so it stays type-correct + on the compile path; the destructured
/// tuple/record + the nested `(Some (tuple …))` all GRADE (verified lean:HOLDS). Lists are NOT
/// pattern-destructured (a fixed-arity `(list a b c)` pattern is CDZ0210) — projection/`List.len` cover them.
fn gen_pattern_match_body<C: Choice>(c: &mut C, out: &mut String) {
    let t = pick_scalar_ty(c);
    match c.variant(4) {
        // 2-tuple destructure → return the first binder.
        0 => {
            out.push_str("(match (tuple ");
            gen_scalar_leaf(c, t, out);
            out.push(' ');
            gen_scalar_leaf(c, t, out);
            out.push_str(") ((tuple x y) x))");
        }
        // 3-tuple destructure → return the middle binder.
        1 => {
            out.push_str("(match (tuple ");
            gen_scalar_leaf(c, t, out);
            out.push(' ');
            gen_scalar_leaf(c, t, out);
            out.push(' ');
            gen_scalar_leaf(c, t, out);
            out.push_str(") ((tuple x y z) y))");
        }
        // Record destructure → return the `b` field's binder.
        2 => {
            out.push_str("(match (record (= a ");
            gen_scalar_leaf(c, t, out);
            out.push_str(") (= b ");
            gen_scalar_leaf(c, t, out);
            out.push_str(")) ((record (= a x) (= b y)) y))");
        }
        // NESTED: a `Some`-wrapped 2-tuple, destructured in the `Some` pattern; `None` → a same-typed default.
        _ => {
            out.push_str("(match (Some (tuple ");
            gen_scalar_leaf(c, t, out);
            out.push(' ');
            gen_scalar_leaf(c, t, out);
            out.push_str(")) ((Some (tuple x y)) x) ((None) ");
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

    /// Every SYMBOL-IN-COMPOUND shape (v-nix #7710, tag-20 value_codec) is a well-formed program the
    /// compiler cleanly handles — cdz-smith emitted no symbols before, so this pins the new coverage
    /// and the coercion invariant (never a crash / invalid wasm / parse error) over symbol programs.
    #[test]
    fn symbol_compound_shapes_are_cleanly_handled() {
        for seed in 0u8..40 {
            let bytes = [
                seed,
                seed.wrapping_mul(3),
                seed.wrapping_add(7),
                1,
                2,
                3,
                4,
                5,
            ];
            let mut c = ByteCursorChoice::new(&bytes);
            let body = gen_symbol_compound_body(&mut c);
            assert!(
                body.contains("#\""),
                "a symbol-compound shape must carry a symbol: {body}"
            );
            let prog = format!("(do (def (main) {body}) (export main))");
            match compile_catching(&prog) {
                Verdict::Compiled { .. } | Verdict::Declined { .. } => {}
                other => panic!("symbol-compound program not cleanly handled: {prog}\n{other:?}"),
            }
        }
    }

    /// Every NOMINAL-over-Symbol shape (v-nix #7714 — a nominal newtype wrapping a Symbol) is a
    /// well-formed program the compiler cleanly handles, pinning the `(Symbol.of …)` value-form recovery.
    #[test]
    fn nominal_symbol_shapes_are_cleanly_handled() {
        for seed in 0u8..24 {
            let bytes = [seed, seed.wrapping_mul(5), seed.wrapping_add(3), 2, 4, 6];
            let mut c = ByteCursorChoice::new(&bytes);
            let (type_decl, body) = gen_nominal_symbol_program(&mut c);
            assert!(
                body.contains("Tag.T") && body.contains("#\""),
                "shape carries a nominal symbol: {body}"
            );
            let prog = format!("(do {type_decl} (def (main) {body}) (export main))");
            match compile_catching(&prog) {
                Verdict::Compiled { .. } | Verdict::Declined { .. } => {}
                other => panic!("nominal-symbol program not cleanly handled: {prog}\n{other:?}"),
            }
        }
    }

    /// `generate_coerced` actually REACHES the symbol special-program (variant slot 4) for some entropy —
    /// so the Symbol-in-compound widening is live in the real coercion path, not merely callable directly.
    #[test]
    fn generate_coerced_reaches_a_symbol_program() {
        let hit = (0u64..4000).any(|s| {
            let bytes: Vec<u8> = (0..24)
                .map(|i| ((s.wrapping_mul(0x9E37_79B9).wrapping_add(i)) & 0xff) as u8)
                .collect();
            generate_coerced(&bytes).source.contains("#\"")
        });
        assert!(
            hit,
            "the symbol-in-compound special program (variant slot 4) must be reachable"
        );
    }

    /// The NARROW type-fuzzing grammar (S194): every generated program parses + is cleanly handled by
    /// the compiler (Compiled or a correct coded Declined — the ~20% ill-typed arm), never a crash /
    /// invalid wasm / parse error. A well-formed in-fragment population for the false-reject hunt.
    #[test]
    fn typecheck_grammar_is_cleanly_handled() {
        let mut compiled = 0;
        let mut declined = 0;
        for s in 0u64..120 {
            let bytes: Vec<u8> = (0..64)
                .map(|i| ((s.wrapping_mul(0x9E37_79B9).wrapping_add(i)) & 0xff) as u8)
                .collect();
            let src = generate_typecheck(&bytes).source;
            match compile_catching(&src) {
                Verdict::Compiled { .. } => compiled += 1,
                Verdict::Declined { .. } => declined += 1,
                other => panic!("type-fuzz program not cleanly handled: {src}\n{other:?}"),
            }
        }
        // The 80/20 split means BOTH outcomes must actually occur (well-typed compiles + ill-typed
        // declines) — a witness that the grammar exercises both directions, not just one.
        assert!(
            compiled > 0,
            "the well-typed arm must produce compiled programs"
        );
        assert!(
            declined > 0,
            "the ill-typed arm must produce coded declines"
        );
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
    fn gen_sized_int_body_reaches_nested_and_compiles() {
        let mut saw_nested = false;
        for seed in 0u64..512 {
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(2671);
            let mut bytes = Vec::new();
            for _ in 0..16 {
                x ^= x >> 30;
                x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                bytes.push((x >> 24) as u8);
            }
            let mut body = String::new();
            gen_sized_int_body(&mut ByteCursorChoice::new(&bytes), &mut body);
            // A NESTED sized expr = a sized op whose operand is itself an op (recursion working): >= 2 of
            // the sized ops present.
            let ops = ["(+ ", "(* ", "(& ", "(| ", "(^ "];
            let n: usize = ops.iter().map(|o| body.matches(o).count()).sum();
            saw_nested |= n >= 2;
            let src = format!("(do (def (main) {body}) (export main))");
            assert!(
                matches!(compile_catching(&src), Verdict::Compiled { .. }),
                "recursive sized-int body must COMPILE: {src}"
            );
        }
        assert!(
            saw_nested,
            "recursive sized-int body should reach a NESTED (>=2-op) expression"
        );
    }

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

    /// Every `gen_compound_consume` (tuple/record projection, `List.len`, Option `match`, `Result` `match`)
    /// COMPILES — the build+consume shapes are type-correct by construction and stay on the compile path
    /// (no overflow / div0), so this exercises consumption emit and guards `gen_compound_consume` (a
    /// malformed projection / match would surface as decline/InvalidWasm here). The 256 seeds hit every arm
    /// across all scalar payload types (incl the `Result` Ok/Err ctors added in S97).
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

    /// `gen_compound_consume` REACHES a `Result` match with BOTH the `Ok` and the `Err` scrutinee ctor
    /// (added S97), and every such body COMPILES — guards the sum-match consumption arm against a
    /// regression that would stop generating it (silently shrinking differential reach into sum-match emit).
    #[test]
    fn result_match_consume_reaches_both_ctors_and_compiles() {
        let (mut saw_ok, mut saw_err) = (false, false);
        for seed in 0u64..512 {
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(97);
            let mut bytes = Vec::new();
            for _ in 0..16 {
                x ^= x >> 30;
                x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                bytes.push((x >> 24) as u8);
            }
            let mut body = String::new();
            gen_compound_consume(&mut ByteCursorChoice::new(&bytes), &mut body);
            if !body.contains("(Result ") {
                continue; // a non-Result arm this seed
            }
            saw_ok |= body.contains("(Ok ");
            saw_err |= body.contains("(Err ");
            let src = format!("(do (def (main) {body}) (export main))");
            assert!(
                matches!(compile_catching(&src), Verdict::Compiled { .. }),
                "Result-match consume body must COMPILE: {src}"
            );
        }
        assert!(saw_ok, "Result-match should reach the Ok scrutinee ctor");
        assert!(saw_err, "Result-match should reach the Err scrutinee ctor");
    }

    /// `gen_compound_consume` REACHES a sum-match over a COMPOUND payload (S102: `(Some (tuple/record/list …))`
    /// consumed in-arm) covering all three payload shapes, and every such body COMPILES — guards the
    /// compound-payload consumption arm (binds a native compound from a match arm + projects/List.len in-arm),
    /// the fresh M2 native ctor-leaf codegen the scalar-payload matches never reach.
    #[test]
    fn compound_payload_match_reaches_all_shapes_and_compiles() {
        let (mut saw_tuple, mut saw_record, mut saw_list) = (false, false, false);
        for seed in 0u64..1024 {
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(131);
            let mut bytes = Vec::new();
            for _ in 0..16 {
                x ^= x >> 30;
                x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                bytes.push((x >> 24) as u8);
            }
            let mut body = String::new();
            gen_compound_consume(&mut ByteCursorChoice::new(&bytes), &mut body);
            // The compound-payload arm is the only one that pairs `(Some (` with a native compound head.
            if !(body.contains("(Some (tuple ")
                || body.contains("(Some (record ")
                || body.contains("(Some (list "))
            {
                continue;
            }
            saw_tuple |= body.contains("(Some (tuple ");
            saw_record |= body.contains("(Some (record ");
            saw_list |= body.contains("(Some (list ");
            let src = format!("(do (def (main) {body}) (export main))");
            assert!(
                matches!(compile_catching(&src), Verdict::Compiled { .. }),
                "compound-payload match body must COMPILE: {src}"
            );
        }
        assert!(
            saw_tuple,
            "compound-payload match should reach a tuple payload"
        );
        assert!(
            saw_record,
            "compound-payload match should reach a record payload"
        );
        assert!(
            saw_list,
            "compound-payload match should reach a list payload"
        );
    }

    /// `gen_compound_consume` REACHES native `#set`/`#map` literals (S110: fills the Set/Map codegen gap),
    /// and every such body COMPILES — guards the set/map arm (native leaf kinds 23/24: `#set(…)`, its
    /// `Set.len`, `#map((= k v) …)`, its `Map.len`). A malformed literal / removed op surfaces here.
    #[test]
    fn set_map_literals_are_reached_and_compile() {
        let (mut saw_set, mut saw_map) = (false, false);
        for seed in 0u64..1024 {
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(211);
            let mut bytes = Vec::new();
            for _ in 0..16 {
                x ^= x >> 30;
                x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                bytes.push((x >> 24) as u8);
            }
            let mut body = String::new();
            gen_compound_consume(&mut ByteCursorChoice::new(&bytes), &mut body);
            if !(body.contains("#set(") || body.contains("#map(")) {
                continue;
            }
            saw_set |= body.contains("#set(");
            saw_map |= body.contains("#map(");
            let src = format!("(do (def (main) {body}) (export main))");
            assert!(
                matches!(compile_catching(&src), Verdict::Compiled { .. }),
                "set/map literal body must COMPILE: {src}"
            );
        }
        assert!(saw_set, "should reach a #set literal");
        assert!(saw_map, "should reach a #map literal");
    }

    /// `build_program` REACHES the USER-SUM shape (S140) — a top-level `(type …)` + a construct/match main
    /// — for BOTH the multi-variant tagged sum (`type Shape`) and the single-variant newtype (`type Pt`),
    /// and every such program COMPILES. Guards the user-sum arm (a malformed decl/ctor/pattern would
    /// decline here). Top-level `(type …)` is required to GRADE (a local one SKIPs) — this pins it top-level.
    #[test]
    fn build_program_reaches_user_sum_shapes_and_compiles() {
        let (mut saw_multi, mut saw_newtype, mut saw_nullary) = (false, false, false);
        for seed in 0u64..1024 {
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(719);
            let mut bytes = Vec::new();
            for _ in 0..24 {
                x ^= x >> 30;
                x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                bytes.push((x >> 24) as u8);
            }
            let src = build_program(&mut ByteCursorChoice::new(&bytes)).source;
            if !src.contains("(type ") {
                continue; // not the user-sum shape this seed
            }
            // The top-level type decl must precede `(def (main)` (pins it top-level, not in-body).
            assert!(
                src.find("(type ").unwrap() < src.find("(def (main)").unwrap(),
                "the `(type …)` must be a TOP-LEVEL decl (before main), else it SKIPs: {src}"
            );
            saw_multi |= src.contains("(type Shape ");
            saw_newtype |= src.contains("(type Pt ");
            saw_nullary |= src.contains("(type Color ");
            assert!(
                matches!(compile_catching(&src), Verdict::Compiled { .. }),
                "user-sum program must COMPILE: {src}"
            );
        }
        assert!(
            saw_multi,
            "should reach a multi-variant tagged sum (type Shape)"
        );
        assert!(
            saw_newtype,
            "should reach a single-variant newtype (type Pt)"
        );
        assert!(saw_nullary, "should reach a nullary-ctor enum (type Color)");
    }

    /// `build_program` REACHES the RECURSIVE-PERFORM effect shape — a top-level recursive `loop` that
    /// PERFORMS the op deep inside itself, discharged by `main`'s enclosing `handle` — and every such
    /// program COMPILES. The perform is cross-function (inside `loop`, not lexically in the handle body):
    /// both `(effect …)` and `(def (loop …))` must be TOP-LEVEL (a locally-nested perform has no home =
    /// CDZ0401, and a local def SKIPs in the oracle) — this pins the shape top-level. Also asserts the
    /// resume-value spread reaches both `s` (identity) and `(+ s p)` (fold).
    #[test]
    fn build_program_reaches_recursive_perform_effect_and_compiles() {
        let (mut saw, mut saw_ident, mut saw_fold) = (false, false, false);
        for seed in 0u64..1024 {
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(929);
            let mut bytes = Vec::new();
            for _ in 0..24 {
                x ^= x >> 30;
                x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                bytes.push((x >> 24) as u8);
            }
            let src = build_program(&mut ByteCursorChoice::new(&bytes)).source;
            if !src.contains("(def (loop ") {
                continue; // not the recursive-perform shape this seed
            }
            saw = true;
            // The effect decl + loop def must be TOP-LEVEL (before main), else the perform has no home.
            assert!(
                src.find("(effect E").unwrap() < src.find("(def (main)").unwrap(),
                "the `(effect …)` + `loop` must be TOP-LEVEL (before main): {src}"
            );
            saw_ident |= src.contains("(resume s ");
            saw_fold |= src.contains("(resume (+ s p)");
            assert!(
                matches!(compile_catching(&src), Verdict::Compiled { .. }),
                "recursive-perform effect program must COMPILE: {src}"
            );
        }
        assert!(saw, "should reach the recursive-perform effect shape");
        assert!(
            saw_ident,
            "should reach the identity resume value (resume s …)"
        );
        assert!(
            saw_fold,
            "should reach the folding resume value (resume (+ s p) …)"
        );
    }

    /// `build_program` REACHES the CROSS-MODULE shape — a top-level inline `(module M …)` exporting a
    /// function that `main` calls across the boundary — and every such program COMPILES. The module MUST
    /// be top-level (before main), pinning it a whole-program shape. Asserts all three forms (scalar,
    /// two-arg, compound-result) are reached.
    #[test]
    fn build_program_reaches_cross_module_shape_and_compiles() {
        // Each cross-module form crosses the boundary as a distinct type; every one must compile.
        let markers = [
            "(M.f ", "(M.g ", "(M.mk ", "(M.tup ", "(M.opt ", "(M.rec ", "(M.lt ", "(M.big ",
        ];
        let mut seen = [false; 8];
        let mut saw = false;
        for seed in 0u64..2048 {
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1013);
            let mut bytes = Vec::new();
            for _ in 0..24 {
                x ^= x >> 30;
                x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                bytes.push((x >> 24) as u8);
            }
            let src = build_program(&mut ByteCursorChoice::new(&bytes)).source;
            if !src.contains("(module M ") {
                continue; // not the cross-module shape this seed
            }
            saw = true;
            assert!(
                src.find("(module M ").unwrap() < src.find("(def (main)").unwrap(),
                "the `(module …)` must be a TOP-LEVEL decl (before main): {src}"
            );
            for (i, m) in markers.iter().enumerate() {
                seen[i] |= src.contains(m);
            }
            assert!(
                matches!(compile_catching(&src), Verdict::Compiled { .. }),
                "cross-module program must COMPILE: {src}"
            );
        }
        assert!(saw, "should reach the cross-module shape");
        for (i, m) in markers.iter().enumerate() {
            assert!(seen[i], "should reach the cross-module form {m}");
        }
    }

    /// `build_program` REACHES the RECURSIVE COLLECTION-BUILDER shape — a top-level recursive `def` that
    /// grows a List/Map across its calls, consumed by `main` — and every such program COMPILES. The
    /// builder def must be TOP-LEVEL (a local recursive def SKIPs in the oracle). Asserts both the List
    /// (`build`) and Map (`bm`) builders are reached.
    #[test]
    fn build_program_reaches_recursive_collection_builder_and_compiles() {
        let (mut saw, mut saw_list, mut saw_map) = (false, false, false);
        for seed in 0u64..2048 {
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(2027);
            let mut bytes = Vec::new();
            for _ in 0..24 {
                x ^= x >> 30;
                x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                bytes.push((x >> 24) as u8);
            }
            let src = build_program(&mut ByteCursorChoice::new(&bytes)).source;
            if !src.contains("(def (build ") && !src.contains("(def (bm ") {
                continue; // not the recursive-collection-builder shape this seed
            }
            saw = true;
            let builder = if src.contains("(def (build ") {
                "(def (build "
            } else {
                "(def (bm "
            };
            assert!(
                src.find(builder).unwrap() < src.find("(def (main)").unwrap(),
                "the recursive builder def must be TOP-LEVEL (before main): {src}"
            );
            saw_list |= src.contains("(def (build ");
            saw_map |= src.contains("(def (bm ");
            assert!(
                matches!(compile_catching(&src), Verdict::Compiled { .. }),
                "recursive collection-builder program must COMPILE: {src}"
            );
        }
        assert!(saw, "should reach the recursive collection-builder shape");
        assert!(saw_list, "should reach the List builder (build)");
        assert!(saw_map, "should reach the Map builder (bm)");
    }

    /// `gen_bignum_body` REACHES both BigInt (`N`) and Rational (`R`) forms and every body COMPILES (S132:
    /// fills the BigInt/Rational numeric-family gap). BigInt `+`/`-`/`*` never overflow; Rational `/` uses a
    /// nonzero denominator — so all stay on the compile path (they SKIP in the value oracle for now).
    #[test]
    fn gen_bignum_body_reaches_bigint_and_rational_and_compiles() {
        let (mut saw_n, mut saw_r, mut saw_cmp) = (false, false, false);
        for seed in 0u64..512 {
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(613);
            let mut bytes = Vec::new();
            for _ in 0..16 {
                x ^= x >> 30;
                x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                bytes.push((x >> 24) as u8);
            }
            let mut body = String::new();
            gen_bignum_body(&mut ByteCursorChoice::new(&bytes), &mut body);
            saw_n |= body.contains('N');
            saw_r |= body.contains('R');
            // A comparison body begins with a comparison op head (arith ops are `+ - * /`).
            saw_cmp |= body.starts_with("(=") || body.starts_with("(<") || body.starts_with("(>");
            let src = format!("(do (def (main) {body}) (export main))");
            assert!(
                matches!(compile_catching(&src), Verdict::Compiled { .. }),
                "bignum body must COMPILE: {src}"
            );
        }
        assert!(saw_n, "should reach a BigInt (N) form");
        assert!(saw_r, "should reach a Rational (R) form");
        assert!(saw_cmp, "should reach a BigInt/Rational comparison");
    }

    /// `gen_qty_body` REACHES both the bare `Qty.of` literal form and the same-unit arithmetic form,
    /// exercises a PARENTHESIZED magnitude (the #7227 regression guard), and every body COMPILES —
    /// filling the Qty numeric-family gap (Qty was absent from the coercing/value-comparable grammar).
    #[test]
    fn gen_qty_body_reaches_all_forms_and_compiles() {
        let (mut saw_lit, mut saw_arith, mut saw_grouped) = (false, false, false);
        for seed in 0u64..512 {
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(937);
            let mut bytes = Vec::new();
            for _ in 0..16 {
                x ^= x >> 30;
                x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                bytes.push((x >> 24) as u8);
            }
            let mut body = String::new();
            gen_qty_body(&mut ByteCursorChoice::new(&bytes), &mut body);
            // A bare-literal body is `(Qty.value (Qty.of …))`; an arithmetic body wraps an op.
            saw_arith |= body.starts_with("(Qty.value (+")
                || body.starts_with("(Qty.value (-")
                || body.starts_with("(Qty.value (*");
            saw_lit |= body.starts_with("(Qty.value (Qty.of");
            // A parenthesized magnitude renders `(Qty.of (n) …)` (grouped literal — #7227 guard).
            saw_grouped |= body.contains("(Qty.of (");
            let src = format!("(do (def (main) {body}) (export main))");
            assert!(
                matches!(compile_catching(&src), Verdict::Compiled { .. }),
                "qty body must COMPILE: {src}"
            );
        }
        assert!(saw_lit, "should reach a bare Qty.of literal");
        assert!(
            saw_arith,
            "should reach a Qty same-unit arithmetic combination"
        );
        assert!(
            saw_grouped,
            "should reach a parenthesized (grouped) magnitude"
        );
    }

    /// `gen_map_lookup_body` REACHES both a PRESENT-key lookup (the `Some` arm, key `0..=9`) and an
    /// ABSENT-key lookup (the `None` arm, key `99`), and every body COMPILES — filling the Map.lookup
    /// (keyed read → `Option V`) gap the coercing grammar's Map.len-only coverage never reached.
    #[test]
    fn gen_map_lookup_body_reaches_present_and_absent_and_compiles() {
        let (mut saw_present, mut saw_absent) = (false, false);
        for seed in 0u64..512 {
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1049);
            let mut bytes = Vec::new();
            for _ in 0..16 {
                x ^= x >> 30;
                x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                bytes.push((x >> 24) as u8);
            }
            let mut body = String::new();
            gen_map_lookup_body(&mut ByteCursorChoice::new(&bytes), &mut body);
            // The absent lookup uses the sentinel key ` 99)`; a present lookup uses a `0..=9` key.
            saw_absent |= body.contains(" 99) ((Some");
            saw_present |= !body.contains(" 99) ((Some") && body.contains("(Map.lookup");
            let src = format!("(do (def (main) {body}) (export main))");
            assert!(
                matches!(compile_catching(&src), Verdict::Compiled { .. }),
                "map-lookup body must COMPILE: {src}"
            );
        }
        assert!(saw_present, "should reach a present-key lookup (Some arm)");
        assert!(saw_absent, "should reach an absent-key lookup (None arm)");
    }

    /// `gen_collection_op_body` REACHES all six op forms (Set.union, Set.remove, Set.contains,
    /// Map.remove, Set.intersection, Set.difference) and every body COMPILES — filling the set-merge /
    /// element-removal / membership / set-algebra gap the coercing grammar's Set.len/insert +
    /// Map.len/lookup coverage never reached.
    #[test]
    fn gen_collection_op_body_reaches_all_forms_and_compiles() {
        let (
            mut saw_union,
            mut saw_sremove,
            mut saw_contains,
            mut saw_mremove,
            mut saw_intersection,
            mut saw_difference,
            mut saw_merge,
        ) = (false, false, false, false, false, false, false);
        for seed in 0u64..512 {
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1163);
            let mut bytes = Vec::new();
            for _ in 0..16 {
                x ^= x >> 30;
                x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                bytes.push((x >> 24) as u8);
            }
            let mut body = String::new();
            gen_collection_op_body(&mut ByteCursorChoice::new(&bytes), &mut body);
            saw_union |= body.contains("Set.union");
            saw_sremove |= body.contains("Set.remove");
            saw_contains |= body.contains("Set.contains");
            saw_mremove |= body.contains("Map.remove");
            saw_intersection |= body.contains("Set.intersection");
            saw_difference |= body.contains("Set.difference");
            saw_merge |= body.contains("Map.merge");
            let src = format!("(do (def (main) {body}) (export main))");
            assert!(
                matches!(compile_catching(&src), Verdict::Compiled { .. }),
                "collection-op body must COMPILE: {src}"
            );
        }
        assert!(saw_union, "should reach Set.union");
        assert!(saw_sremove, "should reach Set.remove");
        assert!(saw_contains, "should reach Set.contains");
        assert!(saw_mremove, "should reach Map.remove");
        assert!(saw_intersection, "should reach Set.intersection");
        assert!(saw_difference, "should reach Set.difference");
        assert!(saw_merge, "should reach Map.merge");
    }

    /// `gen_effect_body` emits a well-formed value-comparable EFFECT program (effect decl + stateful
    /// handler + tail-resume + twice-performed op) and every body COMPILES — the effect-SEMANTICS
    /// value-coverage the coercing grammar never reached (effects were crash-checked only). Also asserts
    /// the resume-value form spread reaches both the state-folding `(+ s p)` and a bare param/literal.
    #[test]
    fn gen_effect_body_is_well_formed_and_compiles() {
        let (mut saw_handle, mut saw_resume, mut saw_statefold, mut saw_bare, mut saw_abort) =
            (false, false, false, false, false);
        for seed in 0u64..512 {
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1481);
            let mut bytes = Vec::new();
            for _ in 0..16 {
                x ^= x >> 30;
                x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                bytes.push((x >> 24) as u8);
            }
            let mut body = String::new();
            gen_effect_body(&mut ByteCursorChoice::new(&bytes), &mut body);
            saw_handle |= body.contains("(handle E");
            saw_resume |= body.contains("(resume ");
            saw_statefold |= body.contains("(resume (+ s p)");
            saw_bare |= body.contains("(resume s ") || body.contains("(resume p ");
            // An ABORT body is a (handle …) with NO (resume …) — the arm returns a value directly.
            saw_abort |= body.contains("(handle E") && !body.contains("(resume ");
            let src = format!("(do (def (main) {body}) (export main))");
            assert!(
                matches!(compile_catching(&src), Verdict::Compiled { .. }),
                "effect body must COMPILE: {src}"
            );
        }
        assert!(saw_handle, "effect body should emit a (handle E …)");
        assert!(saw_resume, "effect body should emit a (resume …)");
        assert!(
            saw_statefold,
            "effect body should reach the state-folding resume value (+ s p)"
        );
        assert!(
            saw_bare,
            "effect body should reach a bare param resume value"
        );
        assert!(
            saw_abort,
            "effect body should reach the ABORT (non-resumptive) form"
        );
    }

    /// `gen_effect_multiop_body` emits a well-formed TWO-op effect program (one effect declaring `o1`+`o2`,
    /// a per-op handler arm, a body performing BOTH) and every body COMPILES — the op-dispatch value
    /// coverage the single-op handler never reached. Asserts both ops + both arms are present.
    #[test]
    fn gen_effect_multiop_body_is_well_formed_and_compiles() {
        let (mut saw_two_ops, mut saw_both_performs) = (false, false);
        for seed in 0u64..512 {
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1601);
            let mut bytes = Vec::new();
            for _ in 0..16 {
                x ^= x >> 30;
                x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                bytes.push((x >> 24) as u8);
            }
            let mut body = String::new();
            gen_effect_multiop_body(&mut ByteCursorChoice::new(&bytes), &mut body);
            saw_two_ops |= body.contains("(op o1 ") && body.contains("(op o2 ");
            saw_both_performs |= body.contains("(E.o1 ") && body.contains("(E.o2 ");
            let src = format!("(do (def (main) {body}) (export main))");
            assert!(
                matches!(compile_catching(&src), Verdict::Compiled { .. }),
                "multi-op effect body must COMPILE: {src}"
            );
        }
        assert!(saw_two_ops, "should declare two ops (o1 + o2)");
        assert!(saw_both_performs, "should perform both ops (E.o1 + E.o2)");
    }

    /// `gen_effect_collection_body` emits a well-formed EFFECT × COLLECTION program (the handled body
    /// builds a `(list …)` of PERFORM results, consumed by `List.len`/`List.at`) and every body COMPILES —
    /// the effect-value × collection-marshal interaction the single-shape arms never combine. Asserts both
    /// forms (List.len / List.at) are reached.
    #[test]
    fn gen_effect_collection_body_is_well_formed_and_compiles() {
        let (mut saw_len, mut saw_at) = (false, false);
        for seed in 0u64..512 {
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1867);
            let mut bytes = Vec::new();
            for _ in 0..16 {
                x ^= x >> 30;
                x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                bytes.push((x >> 24) as u8);
            }
            let mut body = String::new();
            gen_effect_collection_body(&mut ByteCursorChoice::new(&bytes), &mut body);
            // Every one builds a list of performs inside a handle.
            assert!(
                body.contains("(handle E") && body.contains("(list (E.o "),
                "effect-collection body must build a list of performs: {body}"
            );
            saw_len |= body.contains("(List.len (list (E.o ");
            saw_at |= body.contains("(List.at (list (E.o ");
            let src = format!("(do (def (main) {body}) (export main))");
            assert!(
                matches!(compile_catching(&src), Verdict::Compiled { .. }),
                "effect-collection body must COMPILE: {src}"
            );
        }
        assert!(saw_len, "should reach the List.len form");
        assert!(saw_at, "should reach the List.at form");
    }

    /// `gen_effect_nested_body` emits a well-formed NESTED-HANDLER effect program (two effects, the E2
    /// handle nested inside the E1 handle, both performed) and every body COMPILES — the multi-frame
    /// handler-stack resolution the single-handler shapes never reached. Asserts both effects + a nested
    /// (two-`handle`) structure are present.
    #[test]
    fn gen_effect_nested_body_is_well_formed_and_compiles() {
        let (mut saw_two_effects, mut saw_nested) = (false, false);
        for seed in 0u64..512 {
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1733);
            let mut bytes = Vec::new();
            for _ in 0..16 {
                x ^= x >> 30;
                x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                bytes.push((x >> 24) as u8);
            }
            let mut body = String::new();
            gen_effect_nested_body(&mut ByteCursorChoice::new(&bytes), &mut body);
            saw_two_effects |= body.contains("(effect E1 ") && body.contains("(effect E2 ");
            saw_nested |= body.matches("(handle ").count() >= 2;
            let src = format!("(do (def (main) {body}) (export main))");
            assert!(
                matches!(compile_catching(&src), Verdict::Compiled { .. }),
                "nested-handler effect body must COMPILE: {src}"
            );
        }
        assert!(saw_two_effects, "should declare two effects (E1 + E2)");
        assert!(saw_nested, "should nest two (handle …) frames");
    }

    /// `gen_list_producing_op_body` REACHES all forms (List.push, List.prepend, Set.to-list, Map.to-list)
    /// and every body COMPILES — filling the list-BUILDING collection ops the coercing grammar never
    /// reached.
    #[test]
    fn gen_list_producing_op_body_reaches_all_forms_and_compiles() {
        let (mut saw_push, mut saw_settolist, mut saw_maptolist, mut saw_prepend) =
            (false, false, false, false);
        for seed in 0u64..512 {
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1277);
            let mut bytes = Vec::new();
            for _ in 0..16 {
                x ^= x >> 30;
                x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                bytes.push((x >> 24) as u8);
            }
            let mut body = String::new();
            gen_list_producing_op_body(&mut ByteCursorChoice::new(&bytes), &mut body);
            saw_push |= body.contains("List.push");
            saw_settolist |= body.contains("Set.to-list");
            saw_maptolist |= body.contains("Map.to-list");
            saw_prepend |= body.contains("List.prepend");
            let src = format!("(do (def (main) {body}) (export main))");
            assert!(
                matches!(compile_catching(&src), Verdict::Compiled { .. }),
                "list-producing-op body must COMPILE: {src}"
            );
        }
        assert!(saw_push, "should reach List.push");
        assert!(saw_prepend, "should reach List.prepend");
        assert!(saw_settolist, "should reach Set.to-list");
        assert!(saw_maptolist, "should reach Map.to-list");
    }

    /// `gen_partial_application_body` REACHES all three currying forms (2-ary `let`-partial, 3-ary chained,
    /// 3-ary 2-arg `let`-partial) and every body COMPILES (S143: fills the partial-application gap that
    /// #5488 now grades — a local def under-applied → a closure over the remaining params, later completed).
    #[test]
    fn gen_partial_application_body_reaches_all_forms_and_compiles() {
        let (mut saw_2ary, mut saw_chain, mut saw_2arg) = (false, false, false);
        for seed in 0u64..512 {
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(827);
            let mut bytes = Vec::new();
            for _ in 0..16 {
                x ^= x >> 30;
                x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                bytes.push((x >> 24) as u8);
            }
            let mut body = String::new();
            gen_partial_application_body(&mut ByteCursorChoice::new(&bytes), &mut body);
            saw_2ary |= body.contains("(def (pa a b)");
            saw_chain |= body.contains("(((pa3 ");
            saw_2arg |= body.contains("((g (pa3 ");
            let src = format!("(do (def (main) {body}) (export main))");
            assert!(
                matches!(compile_catching(&src), Verdict::Compiled { .. }),
                "partial-application body must COMPILE: {src}"
            );
        }
        assert!(saw_2ary, "should reach a 2-ary `let`-partial form");
        assert!(saw_chain, "should reach a 3-ary chained-currying form");
        assert!(saw_2arg, "should reach a 3-ary 2-arg `let`-partial form");
    }

    /// `gen_higher_order_body` REACHES all three higher-order forms (lambda-applied-once, lambda-applied-
    /// twice, named-def-as-value) and every body COMPILES (S146: a fn value passed as an argument and
    /// applied inside another def — the applyClosure-over-a-closure-valued-param path).
    #[test]
    fn gen_higher_order_body_reaches_all_forms_and_compiles() {
        let (mut saw_once, mut saw_twice, mut saw_named) = (false, false, false);
        for seed in 0u64..512 {
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(941);
            let mut bytes = Vec::new();
            for _ in 0..16 {
                x ^= x >> 30;
                x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                bytes.push((x >> 24) as u8);
            }
            let mut body = String::new();
            gen_higher_order_body(&mut ByteCursorChoice::new(&bytes), &mut body);
            saw_once |= body.contains("(def (apply f x) (f x)) (apply (fn ");
            saw_twice |= body.contains("(def (twice g n)");
            saw_named |= body.contains("(apply inc ");
            let src = format!("(do (def (main) {body}) (export main))");
            assert!(
                matches!(compile_catching(&src), Verdict::Compiled { .. }),
                "higher-order body must COMPILE: {src}"
            );
        }
        assert!(saw_once, "should reach a lambda-applied-once form");
        assert!(saw_twice, "should reach a lambda-applied-twice form");
        assert!(saw_named, "should reach a named-def-as-value form");
    }

    /// `gen_discard_body` REACHES all four discarded-value kinds (scalar, tuple, list, bool) and every body
    /// COMPILES (S148: a non-def leading do-statement is computed then discarded — the sequencing/dead-value
    /// drop lowering #5507 grades).
    #[test]
    fn gen_discard_body_reaches_all_kinds_and_compiles() {
        let (mut saw_scalar, mut saw_tuple, mut saw_list, mut saw_bool) =
            (false, false, false, false);
        for seed in 0u64..512 {
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1187);
            let mut bytes = Vec::new();
            for _ in 0..16 {
                x ^= x >> 30;
                x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                bytes.push((x >> 24) as u8);
            }
            let mut body = String::new();
            gen_discard_body(&mut ByteCursorChoice::new(&bytes), &mut body);
            saw_tuple |= body.contains("(do (tuple ");
            saw_list |= body.contains("(do (list ");
            saw_bool |= body.contains("(do (< ");
            saw_scalar |= body.starts_with("(do ")
                && !body.contains("(do (tuple ")
                && !body.contains("(do (list ")
                && !body.contains("(do (< ");
            let src = format!("(do (def (main) {body}) (export main))");
            assert!(
                matches!(compile_catching(&src), Verdict::Compiled { .. }),
                "discard body must COMPILE: {src}"
            );
        }
        assert!(saw_scalar, "should reach a discarded-scalar form");
        assert!(saw_tuple, "should reach a discarded-tuple form");
        assert!(saw_list, "should reach a discarded-list form");
        assert!(saw_bool, "should reach a discarded-bool form");
    }

    /// `gen_float_ordering_body` REACHES both widths (Float64, Float32) and all four ordering relations
    /// (`< > <= >=`), and every body COMPILES (S149: float ordering as the returned Bool value — #5519).
    #[test]
    fn gen_float_ordering_body_reaches_both_widths_and_all_rels_and_compiles() {
        let (mut saw_f64, mut saw_f32, mut saw_nan, mut saw_inf) = (false, false, false, false);
        let mut rels_seen = std::collections::BTreeSet::new();
        for seed in 0u64..512 {
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1291);
            let mut bytes = Vec::new();
            for _ in 0..16 {
                x ^= x >> 30;
                x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                bytes.push((x >> 24) as u8);
            }
            let mut body = String::new();
            gen_float_ordering_body(&mut ByteCursorChoice::new(&bytes), &mut body);
            saw_nan |= body.contains("Float64.nan");
            saw_inf |= body.contains("Float64.Infinity");
            if body.contains("Float32") {
                saw_f32 = true;
            } else {
                saw_f64 = true;
            }
            for rel in ["<=", ">=", "<", ">"] {
                if body.starts_with(&format!("({rel} ")) {
                    rels_seen.insert(rel);
                    break;
                }
            }
            let src = format!("(do (def (main) {body}) (export main))");
            assert!(
                matches!(compile_catching(&src), Verdict::Compiled { .. }),
                "float-ordering body must COMPILE: {src}"
            );
        }
        assert!(saw_f64, "should reach a Float64 ordering");
        assert!(saw_f32, "should reach a Float32 ordering");
        assert!(saw_nan, "should reach a NaN-operand (Float64.nan) ordering");
        assert!(
            saw_inf,
            "should reach an Infinity-operand (Float64.Infinity) ordering"
        );
        assert_eq!(
            rels_seen.len(),
            4,
            "should reach all four ordering relations"
        );
    }

    /// `gen_compound_keyed_collection_body` REACHES all three forms (compound-keyed set, `Set.insert`,
    /// compound-keyed map) and every body COMPILES (S154: sets/maps keyed by `(tuple …)` compounds — the
    /// structural total order over compound values #5540 grades).
    #[test]
    fn gen_compound_keyed_collection_body_reaches_all_forms_and_compiles() {
        let (mut saw_set, mut saw_insert, mut saw_map) = (false, false, false);
        let (mut saw_tuple_key, mut saw_record_key, mut saw_nested_key, mut saw_list_key) =
            (false, false, false, false);
        for seed in 0u64..512 {
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1409);
            let mut bytes = Vec::new();
            for _ in 0..16 {
                x ^= x >> 30;
                x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                bytes.push((x >> 24) as u8);
            }
            let mut body = String::new();
            gen_compound_keyed_collection_body(&mut ByteCursorChoice::new(&bytes), &mut body);
            saw_insert |= body.contains("Set.insert");
            saw_set |= body.starts_with("(Set.len #set(");
            saw_map |= body.contains("Map.len");
            saw_record_key |= body.contains("(record ");
            saw_nested_key |= body.contains("(tuple (tuple ");
            saw_list_key |= body.contains("(list ");
            saw_tuple_key |= body.contains("(tuple ") && !body.contains("(tuple (tuple ");
            let src = format!("(do (def (main) {body}) (export main))");
            assert!(
                matches!(compile_catching(&src), Verdict::Compiled { .. }),
                "compound-keyed collection body must COMPILE: {src}"
            );
        }
        assert!(saw_set, "should reach a compound-keyed set (Set.len #set)");
        assert!(saw_insert, "should reach a Set.insert form");
        assert!(saw_map, "should reach a compound-keyed map (Map.len)");
        assert!(saw_tuple_key, "should reach a flat-tuple-keyed form");
        assert!(saw_record_key, "should reach a record-keyed form");
        assert!(saw_nested_key, "should reach a nested-tuple-keyed form");
        assert!(saw_list_key, "should reach a list-keyed form");
    }

    /// `gen_float_keyed_collection_body` REACHES all four forms (Float64 set, Float64 map, NaN-key set,
    /// Float32 set) and every body COMPILES (S157: float-carrying set/map keys — canonical-bit order +
    /// canonical key equality + NaN keys, #5556).
    #[test]
    fn gen_float_keyed_collection_body_reaches_all_forms_and_compiles() {
        let (mut saw_f64_set, mut saw_f64_map, mut saw_f32, mut saw_nan) =
            (false, false, false, false);
        for seed in 0u64..512 {
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1523);
            let mut bytes = Vec::new();
            for _ in 0..16 {
                x ^= x >> 30;
                x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                bytes.push((x >> 24) as u8);
            }
            let mut body = String::new();
            gen_float_keyed_collection_body(&mut ByteCursorChoice::new(&bytes), &mut body);
            saw_f32 |= body.contains("Float32");
            saw_nan |= body.contains("Float64.nan");
            saw_f64_map |= body.starts_with("(Map.len");
            saw_f64_set |= body.starts_with("(Set.len #set(")
                && !body.contains("Float32")
                && !body.contains("Float64.nan");
            let src = format!("(do (def (main) {body}) (export main))");
            assert!(
                matches!(compile_catching(&src), Verdict::Compiled { .. }),
                "float-keyed collection body must COMPILE: {src}"
            );
        }
        assert!(saw_f64_set, "should reach a Float64-keyed set");
        assert!(saw_f64_map, "should reach a Float64-keyed map");
        assert!(saw_f32, "should reach a Float32-keyed set");
        assert!(saw_nan, "should reach a NaN-key set");
    }

    /// `gen_string_body` REACHES all five String-op forms (byte-len, scalar-at, concat, slice, bare literal)
    /// and every body COMPILES (S166: a String-op family the Int64/float/compound grammar never reached).
    #[test]
    fn gen_string_body_reaches_all_forms_and_compiles() {
        let (
            mut saw_len,
            mut saw_at,
            mut saw_concat,
            mut saw_slice,
            mut saw_lit,
            mut saw_cmp,
            mut saw_char_cmp,
        ) = (false, false, false, false, false, false, false);
        for seed in 0u64..512 {
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1657);
            let mut bytes = Vec::new();
            for _ in 0..16 {
                x ^= x >> 30;
                x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                bytes.push((x >> 24) as u8);
            }
            let mut body = String::new();
            gen_string_body(&mut ByteCursorChoice::new(&bytes), &mut body);
            saw_len |= body.contains("String.byte-len");
            saw_at |= body.contains("String.scalar-at");
            saw_concat |= body.contains("String.concat");
            saw_slice |= body.contains("String.slice");
            // A comparison body begins with an op head; a CHAR comparison compares two
            // `String.scalar-at` results (contains scalar-at), a STRING comparison two literals.
            let is_cmp = body.starts_with("(=") || body.starts_with("(<") || body.starts_with("(>");
            saw_char_cmp |= is_cmp && body.contains("String.scalar-at");
            saw_cmp |= is_cmp && !body.contains("String.scalar-at");
            saw_lit |= body.starts_with('"');
            let src = format!("(do (def (main) {body}) (export main))");
            assert!(
                matches!(compile_catching(&src), Verdict::Compiled { .. }),
                "string-op body must COMPILE: {src}"
            );
        }
        assert!(saw_len, "should reach String.byte-len");
        assert!(saw_at, "should reach String.scalar-at");
        assert!(saw_concat, "should reach String.concat");
        assert!(saw_slice, "should reach String.slice");
        assert!(saw_lit, "should reach a bare string literal");
        assert!(saw_cmp, "should reach a string comparison");
        assert!(saw_char_cmp, "should reach a char (scalar-at) comparison");
    }

    /// `gen_bytes_body` REACHES all five Bytes-op forms (len, at, literal, of-list, concat) and every body
    /// COMPILES (S167: the Bytes construct family — distinct from String and numeric/compound grammar).
    #[test]
    fn gen_bytes_body_reaches_all_forms_and_compiles() {
        let (mut saw_len, mut saw_at, mut saw_lit, mut saw_of, mut saw_concat, mut saw_cmp) =
            (false, false, false, false, false, false);
        for seed in 0u64..512 {
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1789);
            let mut bytes = Vec::new();
            for _ in 0..16 {
                x ^= x >> 30;
                x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                bytes.push((x >> 24) as u8);
            }
            let mut body = String::new();
            gen_bytes_body(&mut ByteCursorChoice::new(&bytes), &mut body);
            saw_len |= body.contains("Bytes.len");
            saw_at |= body.contains("Bytes.at");
            saw_of |= body.contains("Bytes.of");
            saw_concat |= body.contains("Bytes.concat");
            saw_lit |= body.starts_with("b\"");
            // A bytes COMPARISON body begins with an op head over two b"…" byte values.
            saw_cmp |= body.starts_with("(=") || body.starts_with("(<") || body.starts_with("(>");
            let src = format!("(do (def (main) {body}) (export main))");
            assert!(
                matches!(compile_catching(&src), Verdict::Compiled { .. }),
                "bytes-op body must COMPILE: {src}"
            );
        }
        assert!(saw_len, "should reach Bytes.len");
        assert!(saw_at, "should reach Bytes.at");
        assert!(saw_lit, "should reach a b\"…\" literal");
        assert!(saw_of, "should reach Bytes.of");
        assert!(saw_concat, "should reach Bytes.concat");
        assert!(saw_cmp, "should reach a bytes comparison");
    }

    /// `gen_nested_compound_body` REACHES all five forms (List.at, List.concat, tuple-of-lists,
    /// list-of-tuples, record-of-compounds) and every body COMPILES (S168: deeper structural shapes than
    /// the flat single-level compound arms).
    #[test]
    fn gen_nested_compound_body_reaches_all_forms_and_compiles() {
        let (mut saw_at, mut saw_concat, mut saw_tol, mut saw_lot, mut saw_rec) =
            (false, false, false, false, false);
        for seed in 0u64..512 {
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1913);
            let mut bytes = Vec::new();
            for _ in 0..16 {
                x ^= x >> 30;
                x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                bytes.push((x >> 24) as u8);
            }
            let mut body = String::new();
            gen_nested_compound_body(&mut ByteCursorChoice::new(&bytes), &mut body);
            saw_at |= body.contains("List.at");
            saw_concat |= body.contains("List.concat");
            saw_tol |= body.starts_with("(tuple (list ");
            saw_lot |= body.starts_with("(list (tuple ");
            saw_rec |= body.starts_with("(record (= a (tuple ");
            let src = format!("(do (def (main) {body}) (export main))");
            assert!(
                matches!(compile_catching(&src), Verdict::Compiled { .. }),
                "nested-compound body must COMPILE: {src}"
            );
        }
        assert!(saw_at, "should reach List.at");
        assert!(saw_concat, "should reach List.concat");
        assert!(saw_tol, "should reach a tuple-of-lists");
        assert!(saw_lot, "should reach a list-of-tuples");
        assert!(saw_rec, "should reach a record-of-compounds");
    }

    /// `gen_nested_sum_body` REACHES all four forms (Option-of-Option, Result-of-Option, Option-of-tuple,
    /// Option-of-list) and every body COMPILES (S169: deeper sum-wrapping than the flat Some/Ok/Err arms).
    #[test]
    fn gen_nested_sum_body_reaches_all_forms_and_compiles() {
        let (mut saw_oo, mut saw_ro, mut saw_ot, mut saw_ol) = (false, false, false, false);
        for seed in 0u64..512 {
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(2039);
            let mut bytes = Vec::new();
            for _ in 0..16 {
                x ^= x >> 30;
                x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                bytes.push((x >> 24) as u8);
            }
            let mut body = String::new();
            gen_nested_sum_body(&mut ByteCursorChoice::new(&bytes), &mut body);
            saw_oo |= body.contains("(Some (Some ");
            saw_ro |= body.contains("(Ok (Some ");
            saw_ot |= body.contains("(Some (tuple ");
            saw_ol |= body.contains("(Some (list ");
            let src = format!("(do (def (main) {body}) (export main))");
            assert!(
                matches!(compile_catching(&src), Verdict::Compiled { .. }),
                "nested-sum body must COMPILE: {src}"
            );
        }
        assert!(saw_oo, "should reach Option-of-Option");
        assert!(saw_ro, "should reach Result-of-Option");
        assert!(saw_ot, "should reach Option-of-tuple");
        assert!(saw_ol, "should reach Option-of-list");
    }

    /// `gen_int_conversion_body` reaches a BREADTH of Source/Target int-type pairs (≥4 distinct targets +
    /// ≥4 distinct sources) and every `(<Target>.of (: <v> <Source>))` body COMPILES (S170: int cross-width
    /// conversion codegen — widen/narrow/cross-sign).
    #[test]
    fn gen_int_conversion_body_reaches_breadth_and_compiles() {
        let mut targets = std::collections::BTreeSet::new();
        let mut sources = std::collections::BTreeSet::new();
        for seed in 0u64..512 {
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(2161);
            let mut bytes = Vec::new();
            for _ in 0..16 {
                x ^= x >> 30;
                x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                bytes.push((x >> 24) as u8);
            }
            let mut body = String::new();
            gen_int_conversion_body(&mut ByteCursorChoice::new(&bytes), &mut body);
            for t in SIZED_INT_TYPES {
                if body.starts_with(&format!("({t}.of ")) {
                    targets.insert(*t);
                }
                if body.contains(&format!(" {t}))")) {
                    sources.insert(*t);
                }
            }
            let src = format!("(do (def (main) {body}) (export main))");
            assert!(
                matches!(compile_catching(&src), Verdict::Compiled { .. }),
                "int-conversion body must COMPILE: {src}"
            );
        }
        assert!(targets.len() >= 4, "should reach >=4 distinct target types");
        assert!(sources.len() >= 4, "should reach >=4 distinct source types");
    }

    /// `gen_wide_compound_body` REACHES all five wider-arity forms (3-tuple, 4-tuple, 3-record, 3-tuple
    /// projection, 3-record projection) and every body COMPILES (S171: wider construction/projection).
    #[test]
    fn gen_wide_compound_body_reaches_all_forms_and_compiles() {
        let (mut saw_t3, mut saw_t4, mut saw_r3, mut saw_pt, mut saw_pr) =
            (false, false, false, false, false);
        for seed in 0u64..512 {
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(2293);
            let mut bytes = Vec::new();
            for _ in 0..16 {
                x ^= x >> 30;
                x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                bytes.push((x >> 24) as u8);
            }
            let mut body = String::new();
            gen_wide_compound_body(&mut ByteCursorChoice::new(&bytes), &mut body);
            saw_pt |= body.starts_with("(. (tuple ");
            saw_pr |= body.starts_with("(. (record ");
            saw_t4 |= body.starts_with("(tuple ") && body.matches(' ').count() >= 4;
            saw_t3 |= body.starts_with("(tuple ") && body.matches(' ').count() == 3;
            saw_r3 |= body.starts_with("(record ");
            let src = format!("(do (def (main) {body}) (export main))");
            assert!(
                matches!(compile_catching(&src), Verdict::Compiled { .. }),
                "wide-compound body must COMPILE: {src}"
            );
        }
        assert!(saw_t3, "should reach a 3-tuple");
        assert!(saw_t4, "should reach a 4-tuple");
        assert!(saw_r3, "should reach a 3-field record");
        assert!(saw_pt, "should reach a tuple projection");
        assert!(saw_pr, "should reach a record projection");
    }

    /// `gen_bool_logic_body` REACHES all four forms (and, or, not, nested) and every body COMPILES
    /// (S174: short-circuit boolean combinators over comparisons).
    #[test]
    fn gen_bool_logic_body_reaches_all_forms_and_compiles() {
        let (mut saw_and, mut saw_or, mut saw_not, mut saw_nested) = (false, false, false, false);
        for seed in 0u64..512 {
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(2417);
            let mut bytes = Vec::new();
            for _ in 0..16 {
                x ^= x >> 30;
                x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                bytes.push((x >> 24) as u8);
            }
            let mut body = String::new();
            gen_bool_logic_body(&mut ByteCursorChoice::new(&bytes), &mut body);
            saw_and |= body.contains("(and ");
            saw_or |= body.contains("(or ");
            saw_not |= body.contains("(not ");
            // A DEEP shape: >= 2 boolean combinators = a bool op nested inside another (recursion working).
            let combinators = body.matches("(and ").count()
                + body.matches("(or ").count()
                + body.matches("(not ").count();
            saw_nested |= combinators >= 2;
            let src = format!("(do (def (main) {body}) (export main))");
            assert!(
                matches!(compile_catching(&src), Verdict::Compiled { .. }),
                "bool-logic body must COMPILE: {src}"
            );
        }
        assert!(saw_and, "should reach an `and`");
        assert!(saw_or, "should reach an `or`");
        assert!(saw_not, "should reach a `not`");
        assert!(
            saw_nested,
            "should reach a DEEP (recursively-nested) bool shape"
        );
    }

    /// `gen_sized_shift_body` REACHES all three forms (shift-left, shift-right, nested shift+and) over a
    /// breadth of sized-int types, and every body COMPILES (S175: narrow-width shift codegen).
    #[test]
    fn gen_sized_shift_body_reaches_all_forms_and_compiles() {
        let (mut saw_shl, mut saw_shr, mut saw_nested) = (false, false, false);
        let mut types = std::collections::BTreeSet::new();
        for seed in 0u64..512 {
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(2549);
            let mut bytes = Vec::new();
            for _ in 0..16 {
                x ^= x >> 30;
                x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                bytes.push((x >> 24) as u8);
            }
            let mut body = String::new();
            gen_sized_shift_body(&mut ByteCursorChoice::new(&bytes), &mut body);
            saw_nested |= body.starts_with("(& (<< ");
            saw_shl |= body.starts_with("(<< ");
            saw_shr |= body.starts_with("(>> ");
            for t in SIZED_INT_TYPES {
                if body.contains(&format!(" {t})")) {
                    types.insert(*t);
                }
            }
            let src = format!("(do (def (main) {body}) (export main))");
            assert!(
                matches!(compile_catching(&src), Verdict::Compiled { .. }),
                "sized-shift body must COMPILE: {src}"
            );
        }
        assert!(saw_shl, "should reach a shift-left");
        assert!(saw_shr, "should reach a shift-right");
        assert!(saw_nested, "should reach a nested shift+and");
        assert!(
            types.len() >= 4,
            "should reach >=4 distinct sized-int types"
        );
    }

    /// `gen_mutual_recursion_body` REACHES both forms (even/odd Bool parity, ping/pong Int accumulator), the
    /// defs are TOP-LEVEL (assembled before `main`, so they GRADE — a local recursive def SKIPs), and every
    /// program COMPILES (S147: two top-level defs calling each other — a mutual call graph no single
    /// self-recursive helper reaches).
    #[test]
    fn gen_mutual_recursion_body_reaches_both_forms_and_compiles() {
        let (mut saw_parity, mut saw_pingpong) = (false, false);
        for seed in 0u64..512 {
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1063);
            let mut bytes = Vec::new();
            for _ in 0..16 {
                x ^= x >> 30;
                x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                bytes.push((x >> 24) as u8);
            }
            let (defs, body) = gen_mutual_recursion_body(&mut ByteCursorChoice::new(&bytes));
            saw_parity |= defs.contains("(def (ev n)");
            saw_pingpong |= defs.contains("(def (pinga n acc)");
            let src = format!("(do {defs} (def (main) {body}) (export main))");
            // The mutually-recursive defs must precede `(def (main)` (top-level, so they resolve + GRADE).
            assert!(
                src.find("(def (main)").unwrap()
                    > src
                        .find("(def (ev n)")
                        .or_else(|| src.find("(def (pinga n acc)"))
                        .unwrap(),
                "mutual-recursion defs must be TOP-LEVEL (before main): {src}"
            );
            assert!(
                matches!(compile_catching(&src), Verdict::Compiled { .. }),
                "mutual-recursion program must COMPILE: {src}"
            );
        }
        assert!(saw_parity, "should reach the even/odd parity form");
        assert!(saw_pingpong, "should reach the ping/pong accumulator form");
    }

    /// `gen_try_body` REACHES all four `?`/`try` forms (Ok/Err success+short-circuit for Result, Some/None
    /// for Option) and every body COMPILES (S118: fills the `?`/try codegen gap #5249 unlocked). Guards the
    /// try arm — a malformed ascription/boundary would decline/CDZ0230 here.
    #[test]
    fn gen_try_body_reaches_all_forms_and_compiles() {
        let (mut saw_ok, mut saw_err, mut saw_some, mut saw_none) = (false, false, false, false);
        for seed in 0u64..1024 {
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(307);
            let mut bytes = Vec::new();
            for _ in 0..16 {
                x ^= x >> 30;
                x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                bytes.push((x >> 24) as u8);
            }
            let mut body = String::new();
            gen_try_body(&mut ByteCursorChoice::new(&bytes), &mut body);
            saw_ok |= body.contains("(try (Ok ");
            saw_err |= body.contains("(try (Err ");
            saw_some |= body.contains("(try (Some ");
            saw_none |= body.contains("(try None)");
            let src = format!("(do (def (main) {body}) (export main))");
            assert!(
                matches!(compile_catching(&src), Verdict::Compiled { .. }),
                "try body must COMPILE: {src}"
            );
        }
        assert!(saw_ok, "should reach a Result Ok-success try");
        assert!(saw_err, "should reach a Result Err-short-circuit try");
        assert!(saw_some, "should reach an Option Some-success try");
        assert!(saw_none, "should reach an Option None-short-circuit try");
    }

    /// `gen_pattern_match_body` REACHES all destructuring-pattern forms (tuple-2 / tuple-3 / record /
    /// nested Some-tuple) and every body COMPILES (S119: fills the compound-PATTERN gap #5257 round-trips).
    /// A malformed pattern (or an unsupported list pattern → CDZ0210) would surface here.
    #[test]
    fn gen_pattern_match_body_reaches_all_forms_and_compiles() {
        let (mut saw_t2, mut saw_t3, mut saw_rec, mut saw_nested) = (false, false, false, false);
        for seed in 0u64..1024 {
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(409);
            let mut bytes = Vec::new();
            for _ in 0..16 {
                x ^= x >> 30;
                x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                bytes.push((x >> 24) as u8);
            }
            let mut body = String::new();
            gen_pattern_match_body(&mut ByteCursorChoice::new(&bytes), &mut body);
            saw_t2 |= body.contains("((tuple x y) x)");
            saw_t3 |= body.contains("((tuple x y z) y)");
            saw_rec |= body.contains("((record (= a x) (= b y)) y)");
            saw_nested |= body.contains("((Some (tuple x y)) x)");
            let src = format!("(do (def (main) {body}) (export main))");
            assert!(
                matches!(compile_catching(&src), Verdict::Compiled { .. }),
                "pattern-match body must COMPILE: {src}"
            );
        }
        assert!(saw_t2, "should reach a 2-tuple destructure pattern");
        assert!(saw_t3, "should reach a 3-tuple destructure pattern");
        assert!(saw_rec, "should reach a record destructure pattern");
        assert!(
            saw_nested,
            "should reach a nested Some-tuple destructure pattern"
        );
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
