//! The program generator: a byte seed → a canonical s-expr Cadenza program.
//!
//! # Why byte-cursor-driven
//!
//! The generator is a total function of a `&[u8]` seed. That single interface is what lets the
//! PRNG driver, the `bolero` property test, and a subprocess worker all drive the exact same
//! code: the driver feeds PRNG bytes, `bolero` feeds bytes it mutates + shrinks, and the shrinker
//! truncates/edits the byte string. Choices are drawn by consuming bytes from a [`Cursor`]; when
//! the seed is exhausted the cursor yields `0`, which every choice maps to its SIMPLEST arm (a
//! leaf). Combined with a hard depth + node budget, that guarantees termination on any seed,
//! however short or adversarial.
//!
//! # What it aims for
//!
//! Syntactic validity is a hard requirement — every emitted string must parse (`sexpr::read`), or
//! we never reach the compiler at all. Beyond that the grammar is biased toward things that TYPE
//! and reach codegen (in-scope variable references, typed `main` parameters, well-shaped operator
//! applications), because the densest crash clusters live behind the backend seam, past the type
//! checker. It does NOT guarantee well-typedness — an ill-typed program is a clean decline, not a
//! bug, so emitting some is harmless and still exercises the resolver/typer. The bound-name
//! environment ([`Env`]) that already tracks in-scope names and their types is the seam along
//! which this grows toward a fully type-directed generator.

use std::fmt::Write as _;

/// A generated program, ready to hand to the oracle.
#[derive(Clone, Debug)]
pub struct Program {
    /// The full canonical s-expr source, in the runnable export shape
    /// `(do (def (main <params>) <body>) (export main))`.
    pub source: String,
}

/// A deterministic reader over the byte seed. Exhaustion yields `0` (biasing every choice toward
/// its simplest arm), so generation always terminates.
struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Cursor { bytes, pos: 0 }
    }

    /// The next seed byte, or `0` once the seed is spent.
    fn byte(&mut self) -> u8 {
        let b = self.bytes.get(self.pos).copied().unwrap_or(0);
        // Saturate rather than wrap: a spent seed stays spent (yields 0 forever), so a short seed
        // deterministically collapses to a small leaf-heavy program instead of looping the seed.
        self.pos = self.pos.saturating_add(1);
        b
    }

    /// A choice in `0..n` (n >= 1). `0` is always reachable and is each site's simplest arm.
    fn choice(&mut self, n: usize) -> usize {
        debug_assert!(n >= 1);
        (self.byte() as usize) % n
    }

    /// A byte-valued integer in `lo..=hi`.
    fn range(&mut self, lo: u8, hi: u8) -> u8 {
        debug_assert!(lo <= hi);
        lo + (self.byte() % (hi - lo + 1))
    }

    fn flip(&mut self) -> bool {
        self.byte() & 1 == 1
    }
}

/// A small closed set of type expressions, for `main` parameters and `:` ascriptions. Kept to
/// types the front end fully models so an ascription constrains rather than declines.
/// Numeric types come FIRST (indices `0..NUM_TYPES`) so the numeric-ish ascription subset is a clean
/// prefix slice — `Bool`/`String` trail. All fixed-width int widths (8/16/32/64, signed + unsigned)
/// plus both float widths are here so an ascription can constrain a boundary literal to its MATCHING
/// narrow type (the 16- and 32-bit widths were previously absent, leaving those width-fit seams
/// unreachable).
const TYPES: &[&str] = &[
    "Int64", "Int32", "Int16", "Int8", "UInt64", "UInt32", "UInt16", "UInt8", "Float64", "Float32",
    "Bool", "String",
];

/// Count of leading numeric entries in [`TYPES`] (the `Int*`/`UInt*`/`Float*` prefix). Used to slice
/// the numeric-ish subset for a `want == Num` ascription without accidentally reaching `Bool`.
const NUM_TYPES: usize = 10;

/// Infix operator heads, paired with a coarse arity/kind so the generator can pick operands that
/// have a chance of typing. `Bool` in/out vs numeric in/out is the only distinction tracked.
#[derive(Clone, Copy)]
enum Op {
    /// numeric × numeric → numeric
    Arith(&'static str),
    /// numeric × numeric → Bool
    Rel(&'static str),
    /// Bool × Bool → Bool
    Logic(&'static str),
}

const OPS: &[Op] = &[
    Op::Arith("+"),
    Op::Arith("-"),
    Op::Arith("*"),
    Op::Arith("/"),
    Op::Arith("%"),
    // Bitwise + shift: integer-only, total, and width-sensitive (shift-count masking, sign
    // extension, wrapping) — the lowering where a Wasm-vs-Rust const-fold-vs-runtime disagreement
    // is most plausible, and where a large/negative shift operand drawn from INT_BOUNDARIES lands
    // straight on the count-out-of-range edge. Classed Arith (numeric x numeric -> numeric); a
    // float operand is a clean decline, same as `%`.
    Op::Arith("&"),
    Op::Arith("|"),
    Op::Arith("^"),
    Op::Arith("<<"),
    Op::Arith(">>"),
    Op::Rel("<"),
    Op::Rel(">"),
    Op::Rel("<="),
    Op::Rel(">="),
    Op::Rel("="),
    Op::Logic("and"),
    Op::Logic("or"),
];

/// Exact fixed-width integer-type boundaries (and their immediate neighbours) rendered as decimal
/// literals. Emitted verbatim so a program's numeric operand lands ON a min/max/overflow edge — the
/// dense cluster for the overflow-detect (CDZ0304) and width-fit (CDZ0301) diagnostics and for a
/// Wasm-vs-Rust const-fold-vs-runtime disagreement. Values past i64/u64 range are intentional: the
/// front end reads integer literals as arbitrary-precision `BigInt`, so an out-of-range literal is a
/// clean width DECLINE (still exercising the checker), never a lexer failure.
const INT_BOUNDARIES: &[&str] = &[
    // Int8 / UInt8
    "127",
    "128",
    "-128",
    "-129",
    "255",
    "256",
    // Int16 / UInt16
    "32767",
    "32768",
    "-32768",
    "-32769",
    "65535",
    "65536",
    // Int32 / UInt32
    "2147483647",
    "2147483648",
    "-2147483648",
    "-2147483649",
    "4294967295",
    "4294967296",
    // Int64 / UInt64
    "9223372036854775807",
    "9223372036854775808",
    "-9223372036854775808",
    "-9223372036854775809",
    "18446744073709551615",
    "18446744073709551616",
];

/// The fixed-width integer type names, for a boundary-literal ascription (`(: 256 Int8)`). Pairing an
/// `INT_BOUNDARIES` magnitude with a possibly-narrower declared width drives the width-fit (CDZ0301) /
/// const-fold-overflow (CDZ0304) decision at a KNOWN operand — and where the value DOES fit, both
/// backends must agree, so it also feeds the differential oracle.
const INT_FIXED_TYPES: &[&str] = &[
    "Int8", "Int16", "Int32", "Int64", "UInt8", "UInt16", "UInt32", "UInt64",
];

/// Known base units for `Qty`/`Unit` generation (the SI base set + gram) — an UNKNOWN unit declines
/// (`unknown unit …`), so the generator sticks to built-ins that reach the quantity/unit lowering.
const QTY_UNITS: &[&str] = &[
    "meter", "second", "kilogram", "gram", "ampere", "kelvin", "mole", "candela",
];

/// Exact float boundaries, each a well-formed float token (starts with a digit, has a `.`/`e`, valid
/// chars) so `cadenza-syntax::parse_float` accepts it. Covers signed zero, the f32/f64 magnitude
/// extremes, tiny subnormal-scale values, and out-of-f64-range magnitudes (a clean range/const-fold
/// DECLINE — floats parse as an EXACT `Decimal`, so a huge exponent never fails the lexer). These are
/// the operands where a float compare/round/const-fold Wasm-vs-Rust disagreement would surface. No
/// NaN/inf entries: there is no such literal syntax (they would classify as a `Name` and reject).
const FLOAT_BOUNDARIES: &[&str] = &[
    "0.0",
    "-0.0",
    "1.0",
    "-1.0",
    // f32 extremes
    "3.4028235e38",  // ~f32::MAX
    "-3.4028235e38", // ~f32::MIN
    "1.1754944e-38", // ~f32::MIN_POSITIVE (smallest normal)
    "1.0e-45",       // ~f32 smallest subnormal
    // f64 extremes
    "1.7976931348623157e308",  // ~f64::MAX
    "-1.7976931348623157e308", // ~f64::MIN
    "2.2250738585072014e-308", // ~f64::MIN_POSITIVE
    "5.0e-324",                // ~f64 smallest subnormal
    // out-of-f64-range magnitudes (clean range/const-fold edge)
    "1.0e309",
    "-1.0e309",
    "1.0e-400",
    // rounding / representability hazards
    "0.1",
    "0.2",
    "0.3",
];

/// A crude value-kind lattice used only to bias operand generation toward well-typedness. It is a
/// hint, never a guarantee — the compiler is the real type authority.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Num,
    Bool,
    Str,
    /// unknown / any (a bound name whose type we didn't track, a compound, …)
    Any,
}

/// A bound name and our best guess at its kind.
struct Binding {
    name: String,
    kind: Kind,
}

/// The lexical environment threaded through generation: the names in scope (with kind hints) and a
/// monotonic counter for minting fresh, always-valid identifiers.
struct Env {
    scope: Vec<Binding>,
    next_id: u32,
}

impl Env {
    fn new() -> Self {
        Env {
            scope: Vec::new(),
            next_id: 0,
        }
    }

    fn fresh(&mut self) -> String {
        let n = self.next_id;
        self.next_id += 1;
        format!("v{n}")
    }

    /// Push a binding; returns the scope depth to truncate back to when it goes out of scope.
    fn push(&mut self, name: String, kind: Kind) -> usize {
        let mark = self.scope.len();
        self.scope.push(Binding { name, kind });
        mark
    }

    fn truncate(&mut self, mark: usize) {
        self.scope.truncate(mark);
    }

    /// A random in-scope name, preferring one whose kind matches `want` (or any if none match).
    fn pick(&self, cur: &mut Cursor, want: Kind) -> Option<&str> {
        if self.scope.is_empty() {
            return None;
        }
        let matches: Vec<&Binding> = self
            .scope
            .iter()
            .filter(|b| want == Kind::Any || b.kind == want || b.kind == Kind::Any)
            .collect();
        let pool = if matches.is_empty() {
            self.scope.iter().collect()
        } else {
            matches
        };
        Some(&pool[cur.choice(pool.len())].name)
    }
}

/// Generate a program from a byte seed. Total and terminating for any input.
pub fn generate(seed: &[u8]) -> Program {
    let mut g = Gen {
        cur: Cursor::new(seed),
        env: Env::new(),
        nodes: 0,
        node_cap: 220,
        out: String::new(),
    };
    g.program();
    Program { source: g.out }
}

struct Gen<'a> {
    cur: Cursor<'a>,
    env: Env,
    nodes: usize,
    node_cap: usize,
    out: String,
}

impl Gen<'_> {
    /// `(do (def (main <params>) <body>) (export main))` — or, ~1/4 of the time, a USER-SUM program
    /// (a top-level `(type …)` declaration + a `main` that constructs and matches it).
    fn program(&mut self) {
        // Occasionally emit a user-defined-sum program: this reaches the sum TYPE-DECLARATION + tag
        // layout + user-ctor construct/match emit — a path the built-in Option/Result arm and the rest
        // of the grammar never touch (a `(type …)` decl must be TOP-LEVEL). Operator directive
        // 2026-08-30: keep expanding the shapes of programs.
        if self.cur.choice(4) == 0 {
            // A whole-program special shape needing top-level structure the body dispatch can't build:
            // a user-sum type decl, or a try/`?` fallible-boundary program. (The sub-choice here only
            // affects this ~1/4 branch — the normal-path crafted-seed tests take choice(4) != 0.)
            if self.cur.flip() {
                self.user_sum_program();
            } else {
                self.try_program();
            }
            return;
        }
        self.out.push_str("(do (def (main");
        // 0..=2 typed parameters, each seeding a typed binding into the body's scope.
        let nparams = self.cur.choice(3);
        for _ in 0..nparams {
            let name = self.env.fresh();
            let ty = TYPES[self.cur.choice(TYPES.len())];
            let kind = kind_of_type(ty);
            let _ = write!(self.out, " (: {name} {ty})");
            self.env.push(name, kind);
        }
        self.out.push_str(") ");
        let depth = 5;
        self.expr(depth, Kind::Any);
        self.out.push_str(") (export main))");
    }

    /// A TRY/`?` fallible-boundary program: a helper `(def (f) (: (Ok …) (Result Int64 Int64)))` whose
    /// body threads one or two `(try (Ok <num>))` unwraps, + a `main` that matches `f`'s Result. Reaches
    /// the try-operator / `?` desugaring — the fallible-boundary propagation lowering (a Result-returning
    /// fn where `try` unwraps Ok and would early-return Err), which nothing else generates (a bare `(try …)`
    /// declines: it needs an enclosing Result/Option boundary — the `(: … (Result Int64 Int64))` ascription).
    /// Operator directive 2026-08-30: keep expanding generated program shapes. Ok-wrapped operands only (a
    /// bare `(try (Err …))` declines on Err-type unification, so the Ok path is what reaches emit).
    fn try_program(&mut self) {
        let depth = 4;
        self.out.push_str("(do (def (f) (: (Ok ");
        if self.cur.flip() {
            self.out.push_str("(try (Ok ");
            self.expr(depth, Kind::Num);
            self.out.push_str("))");
        } else {
            // Two try-unwraps combined — exercises multiple `?` boundaries in one body.
            self.out.push_str("(+ (try (Ok ");
            self.expr(depth, Kind::Num);
            self.out.push_str(")) (try (Ok ");
            self.expr(depth, Kind::Num);
            self.out.push_str(")))");
        }
        self.out.push_str(
            ") (Result Int64 Int64))) (def (main) (match (f) ((Ok v) v) ((Err e) e))) (export main))",
        );
    }

    /// A USER-DEFINED-SUM program: a top-level monomorphic `(type …)` declaration + a param-less `main`
    /// that CONSTRUCTS a variant (with generated numeric payloads for variety) and MATCHES all arms.
    /// Reaches the sum type-decl / tag-layout / user-ctor construct + match-dispatch emit that no other
    /// path exercises (built-in Option/Result don't declare a type; a `(type …)` must be top-level).
    /// Three shapes mirror the corpus: a multi-variant sum with payloads, nullary variants (an enum),
    /// and a single-variant struct-newtype (which erases to its field tuple). Int64 fields so it types.
    fn user_sum_program(&mut self) {
        let depth = 4;
        match self.cur.choice(3) {
            0 => {
                // Multi-variant with payloads: construct one variant, match both (bind + use payloads).
                self.out.push_str(
                    "(do (type Shape (Circle Int64) (Rect Int64 Int64)) (def (main) (match ",
                );
                if self.cur.flip() {
                    self.out.push_str("(Circle ");
                    self.expr(depth, Kind::Num);
                    self.out.push(')');
                } else {
                    self.out.push_str("(Rect ");
                    self.expr(depth, Kind::Num);
                    self.out.push(' ');
                    self.expr(depth, Kind::Num);
                    self.out.push(')');
                }
                self.out
                    .push_str(" ((Circle a) a) ((Rect a b) (+ a b)))) (export main))");
            }
            1 => {
                // Nullary variants (an enum) — construct one, match all three arms.
                self.out
                    .push_str("(do (type Color (Red) (Green) (Blue)) (def (main) (match ");
                self.out
                    .push_str(["(Red)", "(Green)", "(Blue)"][self.cur.choice(3)]);
                self.out
                    .push_str(" ((Red) 1) ((Green) 2) ((Blue) 3))) (export main))");
            }
            _ => {
                // Single-variant struct-newtype (erases to a field tuple) — construct + destructure.
                self.out
                    .push_str("(do (type Pt (Mk Int64 Int64)) (def (main) (match (Mk ");
                self.expr(depth, Kind::Num);
                self.out.push(' ');
                self.expr(depth, Kind::Num);
                self.out.push_str(") ((Mk a b) (+ a b)))) (export main))");
            }
        }
    }

    /// Emit one expression of the requested (hint) kind, within the depth budget.
    fn expr(&mut self, depth: u32, want: Kind) {
        self.nodes += 1;
        if depth == 0 || self.nodes >= self.node_cap {
            self.leaf(want);
            return;
        }
        // Weighted toward leaves + operators + control flow (the shapes most likely to type and
        // reach codegen); the tail arms exercise ctors, access, ascription, and match.
        match self.cur.choice(24) {
            0..=2 => self.leaf(want),
            3 => self.if_expr(depth, want),
            4 => self.let_expr(depth, want),
            5 => self.op_expr(depth, want),
            6 => self.op_expr(depth, want),
            7 => self.fn_expr(depth),
            8 => self.app_expr(depth),
            9 => self.ctor("list", depth),
            10 => self.ctor("tuple", depth),
            11 => self.access_expr(depth),
            12 => self.ascribe_expr(depth, want),
            13 => self.list_builtin_expr(depth),
            14 => self.string_builtin_expr(depth),
            15 => self.rec_def_expr(depth),
            16 => self.effect_handler_expr(depth, want),
            17 => self.effect_multiop_expr(depth),
            18 => self.map_set_builtin_expr(depth),
            19 => self.record_expr(depth),
            20 => self.sum_expr(depth),
            21 => self.char_expr(depth),
            22 => self.qty_expr(depth),
            _ => self.match_expr(depth, want),
        }
    }

    fn leaf(&mut self, want: Kind) {
        // Prefer an in-scope name of the wanted kind (deepens data flow) about half the time.
        if self.cur.flip()
            && let Some(name) = self.env.pick(&mut self.cur, want)
        {
            self.out.push_str(name);
            return;
        }
        match want {
            Kind::Bool => self.bool_lit(),
            Kind::Str => self.string_lit(),
            Kind::Num => self.num_lit(),
            Kind::Any => match self.cur.choice(4) {
                0 => self.num_lit(),
                1 => self.bool_lit(),
                2 => self.string_lit(),
                _ => self.float_lit(),
            },
        }
    }

    fn num_lit(&mut self) {
        // A spread of magnitudes incl. boundary-ish values, occasionally negated. About a quarter
        // of the time emit an EXACT type-width boundary instead — the min/max of each fixed-width
        // int type and its ±1 neighbours (up to u64::MAX and beyond, safe because integer literals
        // are arbitrary-precision `BigInt` at parse time). These are the operands where the
        // overflow-detect (CDZ0304) and width-fit (CDZ0301) checks flip, and where a Wasm-vs-Rust
        // const-fold-vs-runtime disagreement would surface, so feeding them densely aims the
        // differential oracle straight at that seam rather than random mid-range magnitudes.
        if self.cur.choice(4) == 0 {
            self.out
                .push_str(INT_BOUNDARIES[self.cur.choice(INT_BOUNDARIES.len())]);
            return;
        }
        let n = self.cur.range(0, 200) as i64;
        let v = match self.cur.choice(4) {
            0 => n,
            1 => n * 256,
            2 => -(n),
            _ => n.saturating_mul(n),
        };
        let _ = write!(self.out, "{v}");
    }

    fn float_lit(&mut self) {
        // About a third of the time emit an EXACT float boundary (see FLOAT_BOUNDARIES) instead of a
        // random mid-range decimal — signed zero, f32/f64 magnitude extremes, tiny subnormal-scale
        // values, and out-of-f64-range magnitudes. Floats are parsed as an EXACT `Decimal`
        // (significand·10^exp), so a huge exponent is a clean range/const-fold edge, not a lexer
        // failure; there is no NaN/inf literal syntax to emit. These are where a float compare/
        // round/const-fold Wasm-vs-Rust disagreement would surface.
        if self.cur.choice(3) == 0 {
            self.out
                .push_str(FLOAT_BOUNDARIES[self.cur.choice(FLOAT_BOUNDARIES.len())]);
            return;
        }
        let whole = self.cur.range(0, 200);
        let frac = self.cur.range(0, 99);
        let _ = write!(self.out, "{whole}.{frac:02}");
    }

    fn bool_lit(&mut self) {
        self.out
            .push_str(if self.cur.flip() { "true" } else { "false" });
    }

    fn string_lit(&mut self) {
        // Short a-z strings only — no escaping hazards, always lexes.
        let len = self.cur.choice(4);
        self.out.push('"');
        for _ in 0..len {
            self.out.push((b'a' + self.cur.range(0, 25)) as char);
        }
        self.out.push('"');
    }

    fn if_expr(&mut self, depth: u32, want: Kind) {
        self.out.push_str("(if ");
        self.expr(depth.saturating_sub(1), Kind::Bool);
        self.out.push(' ');
        self.expr(depth.saturating_sub(1), want);
        self.out.push(' ');
        self.expr(depth.saturating_sub(1), want);
        self.out.push(')');
    }

    fn let_expr(&mut self, depth: u32, want: Kind) {
        let name = self.env.fresh();
        // Bind to an arbitrary value; we don't know its kind precisely → Any.
        self.out.push_str("(let ((");
        self.out.push_str(&name);
        self.out.push(' ');
        self.expr(depth.saturating_sub(1), Kind::Any);
        self.out.push_str(")) ");
        let mark = self.env.push(name, Kind::Any);
        self.expr(depth.saturating_sub(1), want);
        self.env.truncate(mark);
        self.out.push(')');
    }

    fn fn_expr(&mut self, depth: u32) {
        let name = self.env.fresh();
        self.out.push_str("(fn (");
        self.out.push_str(&name);
        self.out.push_str(") ");
        let mark = self.env.push(name, Kind::Any);
        self.expr(depth.saturating_sub(1), Kind::Any);
        self.env.truncate(mark);
        self.out.push(')');
    }

    fn op_expr(&mut self, depth: u32, want: Kind) {
        // Pick an operator whose result kind matches `want` when we can, else anything.
        let op = self.pick_op(want);
        let (head, operand) = match op {
            Op::Arith(h) => (h, Kind::Num),
            Op::Rel(h) => (h, Kind::Num),
            Op::Logic(h) => (h, Kind::Bool),
        };
        self.out.push('(');
        self.out.push_str(head);
        self.out.push(' ');
        self.expr(depth.saturating_sub(1), operand);
        self.out.push(' ');
        self.expr(depth.saturating_sub(1), operand);
        self.out.push(')');
    }

    fn pick_op(&mut self, want: Kind) -> Op {
        let candidates: Vec<Op> = OPS
            .iter()
            .copied()
            .filter(|op| {
                matches!(
                    (op, want),
                    (_, Kind::Any)
                        | (Op::Arith(_), Kind::Num)
                        | (Op::Rel(_) | Op::Logic(_), Kind::Bool)
                )
            })
            .collect();
        let pool = if candidates.is_empty() {
            OPS.to_vec()
        } else {
            candidates
        };
        pool[self.cur.choice(pool.len())]
    }

    fn app_expr(&mut self, depth: u32) {
        // Apply an in-scope name (could be a function) to 1..=2 args. If nothing is in scope, fall
        // back to a small inline lambda applied to an argument — still a real application node.
        if let Some(name) = self.env.pick(&mut self.cur, Kind::Any) {
            let name = name.to_string();
            self.out.push('(');
            self.out.push_str(&name);
            let nargs = 1 + self.cur.choice(2);
            for _ in 0..nargs {
                self.out.push(' ');
                self.expr(depth.saturating_sub(1), Kind::Any);
            }
            self.out.push(')');
        } else {
            self.out.push('(');
            self.fn_expr(depth.saturating_sub(1));
            self.out.push(' ');
            self.expr(depth.saturating_sub(1), Kind::Any);
            self.out.push(')');
        }
    }

    fn ctor(&mut self, head: &str, depth: u32) {
        self.out.push('(');
        self.out.push_str(head);
        let n = self.cur.choice(4);
        for _ in 0..n {
            self.out.push(' ');
            self.expr(depth.saturating_sub(1), Kind::Any);
        }
        self.out.push(')');
    }

    /// A homogeneous, non-empty numeric list `(list <num> <num> ...)` — the operand a `List.*`
    /// builtin needs to actually TYPE (a heterogeneous list declines) and reach the list runtime
    /// lowering. Non-empty so an index op has something to hit.
    fn num_list(&mut self, depth: u32) {
        let n = 1 + self.cur.choice(3); // 1..=3 elements
        self.out.push_str("(list");
        for _ in 0..n {
            self.out.push(' ');
            self.expr(depth.saturating_sub(1), Kind::Num);
        }
        self.out.push(')');
    }

    /// A `List`-module builtin over a freshly-built numeric list — `len`/`at`/`update`. These reach
    /// the list runtime indexing/update lowering (the width-partition-index-scratch class the breaker
    /// mined: ListAt/ListUpdate) where a value actually executes, feeding the differential oracle. A
    /// boundary/negative index drawn from `num_lit` lands on the out-of-bounds edge (a clean
    /// `None`/no-op, not a hang). The result type (Int / Option / List) governs the node, so a
    /// mismatch with the outer `want` is a clean decline.
    fn list_builtin_expr(&mut self, depth: u32) {
        match self.cur.choice(3) {
            0 => {
                self.out.push_str("(List.len ");
                self.num_list(depth);
                self.out.push(')');
            }
            1 => {
                self.out.push_str("(List.at ");
                self.num_list(depth);
                self.out.push(' ');
                self.expr(depth.saturating_sub(1), Kind::Num);
                self.out.push(')');
            }
            _ => {
                self.out.push_str("(List.update ");
                self.num_list(depth);
                self.out.push(' ');
                self.expr(depth.saturating_sub(1), Kind::Num);
                self.out.push(' ');
                self.expr(depth.saturating_sub(1), Kind::Num);
                self.out.push(')');
            }
        }
    }

    /// A `String`-module builtin over string literals — `at`/`slice`/`concat`/`byte-len`/`to-bytes`.
    /// These reach the string runtime indexing/slice/length lowering (the StrAt/BytesAt width-alias
    /// class the breaker mined) where a value executes, feeding the differential oracle. A
    /// boundary/negative index drawn from `num_lit` lands on the out-of-bounds edge (a clean
    /// `None`, never a hang). Result type (Option String / String / Int / Bytes) governs the node,
    /// so a mismatch with the outer `want` is a clean decline. String operands are literals so the
    /// call always types and reaches codegen.
    fn string_builtin_expr(&mut self, depth: u32) {
        match self.cur.choice(5) {
            0 => {
                self.out.push_str("(String.at ");
                self.string_lit();
                self.out.push(' ');
                self.expr(depth.saturating_sub(1), Kind::Num);
                self.out.push(')');
            }
            1 => {
                self.out.push_str("(String.slice ");
                self.string_lit();
                self.out.push(' ');
                self.expr(depth.saturating_sub(1), Kind::Num);
                self.out.push(' ');
                self.expr(depth.saturating_sub(1), Kind::Num);
                self.out.push(')');
            }
            2 => {
                self.out.push_str("(String.concat ");
                self.string_lit();
                self.out.push(' ');
                self.string_lit();
                self.out.push(')');
            }
            3 => {
                self.out.push_str("(String.byte-len ");
                self.string_lit();
                self.out.push(')');
            }
            _ => {
                self.out.push_str("(String.to-bytes ");
                self.string_lit();
                self.out.push(')');
            }
        }
    }

    /// A homogeneous non-empty numeric `Map` built by chained `(Map.insert Map.empty k v)` — the operand
    /// shape a `Map.*` builtin needs to TYPE (numeric keys+values) and reach the MAP runtime lowering
    /// (hash-map alloc / insert / rehash). Non-empty so a lookup/remove has a live entry to hit. Nested
    /// inserts start from the bare `Map.empty` atom (not `(Map.empty)`), matching the corpus surface.
    fn num_map(&mut self, depth: u32) {
        let n = 1 + self.cur.choice(3); // 1..=3 entries
        for _ in 0..n {
            self.out.push_str("(Map.insert ");
        }
        self.out.push_str("Map.empty");
        for _ in 0..n {
            self.out.push(' ');
            self.expr(depth.saturating_sub(1), Kind::Num); // key
            self.out.push(' ');
            self.expr(depth.saturating_sub(1), Kind::Num); // value
            self.out.push(')');
        }
    }

    /// A homogeneous non-empty numeric `Set` — `(Set.of (list <num> ...))` (Set.of takes a LIST operand,
    /// not variadic, per the corpus surface). Reaches the SET runtime lowering (hash-set alloc / dedup /
    /// membership). Non-empty so a `contains`/`remove` has a live element.
    fn num_set(&mut self, depth: u32) {
        self.out.push_str("(Set.of ");
        self.num_list(depth);
        self.out.push(')');
    }

    /// A `Map`/`Set`-module builtin over a freshly-built numeric map/set — the HEAP-COLLECTION lowering
    /// (hash map/set: alloc, insert, lookup, membership, remove, union, cardinality) that the crash/
    /// invalid-wasm hunt otherwise NEVER reaches (the generator only emitted `list`/`tuple` ctors +
    /// `List.*`/`String.*` builtins; maps/sets are a distinct runtime subsystem — operator directive
    /// 2026-08-30 to keep expanding generated inputs). Result kind (Int len / Option lookup / Bool
    /// membership / Map|Set|List value) governs the node, so a mismatch with the outer `want` is a clean
    /// decline, never a false finding.
    fn map_set_builtin_expr(&mut self, depth: u32) {
        match self.cur.choice(9) {
            0 => {
                self.out.push_str("(Map.len ");
                self.num_map(depth);
                self.out.push(')');
            }
            1 => {
                self.out.push_str("(Map.lookup ");
                self.num_map(depth);
                self.out.push(' ');
                self.expr(depth.saturating_sub(1), Kind::Num); // key (may miss → None)
                self.out.push(')');
            }
            2 => {
                self.out.push_str("(Map.remove ");
                self.num_map(depth);
                self.out.push(' ');
                self.expr(depth.saturating_sub(1), Kind::Num);
                self.out.push(')');
            }
            3 => {
                self.out.push_str("(Map.to-list ");
                self.num_map(depth);
                self.out.push(')');
            }
            4 => {
                self.out.push_str("(Set.len ");
                self.num_set(depth);
                self.out.push(')');
            }
            5 => {
                self.out.push_str("(Set.contains ");
                self.num_set(depth);
                self.out.push(' ');
                self.expr(depth.saturating_sub(1), Kind::Num);
                self.out.push(')');
            }
            6 => {
                self.out.push_str("(Set.insert ");
                self.num_set(depth);
                self.out.push(' ');
                self.expr(depth.saturating_sub(1), Kind::Num);
                self.out.push(')');
            }
            7 => {
                self.out.push_str("(Set.union ");
                self.num_set(depth);
                self.out.push(' ');
                self.num_set(depth);
                self.out.push(')');
            }
            _ => {
                self.out.push_str("(Set.to-list ");
                self.num_set(depth);
                self.out.push(')');
            }
        }
    }

    /// A `record` value in GENERAL position — `(record (= a e) (= b e) …)` over 1..=3 numeric fields
    /// (fixed labels a/b/c so it always types), sometimes wrapped in a field PROJECTION `(. <record>
    /// <label>)`. Reaches record CONSTRUCTION + projection lowering in ordinary expression position —
    /// which the crash hunt otherwise never exercised (records were emitted ONLY as effect-handler
    /// state before; operator directive 2026-08-30 to keep expanding generated inputs). Numeric fields
    /// so the value types + reaches codegen; a projected label is always one that is present.
    fn record_expr(&mut self, depth: u32) {
        let n = 1 + self.cur.choice(3); // 1..=3 fields
        const LABELS: [&str; 3] = ["a", "b", "c"];
        let project = self.cur.flip();
        if project {
            self.out.push_str("(. ");
        }
        self.out.push_str("(record");
        for &label in LABELS.iter().take(n) {
            let _ = write!(self.out, " (= {label} ");
            self.expr(depth.saturating_sub(1), Kind::Num);
            self.out.push(')');
        }
        self.out.push(')');
        if project {
            // Project a field that is definitely present (index < n).
            let _ = write!(self.out, " {})", LABELS[self.cur.choice(n)]);
        }
    }

    /// A built-in SUM value — `Option` (`(Some e)`/`(None)`) or `Result` (`(Ok e)`/`(Err e)`) — either
    /// bare or wrapped in an exhaustive `match` that DESTRUCTURES it and binds the payload. Reaches the
    /// SUM lowering the crash hunt never exercised in general position: tagged-variant CONSTRUCTION +
    /// match dispatch on a user-visible sum + payload binding (only effect-state used `(Some …)` before;
    /// operator directive 2026-08-30 to keep expanding generated inputs). Numeric payloads so it types.
    fn sum_expr(&mut self, depth: u32) {
        let is_result = self.cur.flip();
        if self.cur.flip() {
            // Bare construction (no match) — reaches sum-value construction + boxing.
            if is_result {
                if self.cur.flip() {
                    self.out.push_str("(Ok ");
                    self.expr(depth.saturating_sub(1), Kind::Num);
                    self.out.push(')');
                } else {
                    self.out.push_str("(Err ");
                    self.expr(depth.saturating_sub(1), Kind::Num);
                    self.out.push(')');
                }
            } else if self.cur.flip() {
                self.out.push_str("(Some ");
                self.expr(depth.saturating_sub(1), Kind::Num);
                self.out.push(')');
            } else {
                self.out.push_str("(None)");
            }
            return;
        }
        // Construct-then-MATCH: exhaustive destructuring that binds the payload into the arm body.
        let v = self.env.fresh();
        if is_result {
            // scrutinee: (Ok e) or (Err e)
            if self.cur.flip() {
                self.out.push_str("(match (Ok ");
            } else {
                self.out.push_str("(match (Err ");
            }
            self.expr(depth.saturating_sub(1), Kind::Num);
            let _ = write!(self.out, ") ((Ok {v}) ");
            let mark = self.env.push(v.clone(), Kind::Num);
            self.expr(depth.saturating_sub(1), Kind::Num);
            self.env.truncate(mark);
            let e = self.env.fresh();
            let _ = write!(self.out, ") ((Err {e}) ");
            let mark = self.env.push(e, Kind::Num);
            self.expr(depth.saturating_sub(1), Kind::Num);
            self.env.truncate(mark);
            self.out.push_str("))");
        } else {
            // scrutinee: (Some e) or (None)
            if self.cur.flip() {
                self.out.push_str("(match (Some ");
                self.expr(depth.saturating_sub(1), Kind::Num);
                self.out.push(')');
            } else {
                self.out.push_str("(match (None)");
            }
            let _ = write!(self.out, " ((Some {v}) ");
            let mark = self.env.push(v.clone(), Kind::Num);
            self.expr(depth.saturating_sub(1), Kind::Num);
            self.env.truncate(mark);
            self.out.push_str(") ((None) ");
            self.expr(depth.saturating_sub(1), Kind::Num);
            self.out.push_str("))");
        }
    }

    /// A `Char` expression — `(Char.from-int <num>)` yields `(Option Char)` (fallible: an int may not be
    /// a valid codepoint), either returned bare or MATCHED and round-tripped back to an Int via
    /// `(Char.to-int c)`. Reaches the Char scalar (codepoint) lowering + `Char.from-int`/`Char.to-int` +
    /// the Option-of-Char match — a value domain the crash hunt never touched (operator directive
    /// 2026-08-30: keep expanding generated program shapes). Numeric arg so it types.
    fn char_expr(&mut self, depth: u32) {
        if self.cur.flip() {
            // Bare (Option Char) construction.
            self.out.push_str("(Char.from-int ");
            self.expr(depth.saturating_sub(1), Kind::Num);
            self.out.push(')');
        } else {
            // Construct-then-match: bind the Char and convert back to Int (exhaustive with a None arm).
            self.out.push_str("(match (Char.from-int ");
            self.expr(depth.saturating_sub(1), Kind::Num);
            let c = self.env.fresh();
            let _ = write!(self.out, ") ((Some {c}) (Char.to-int {c})) ((None) 0))");
        }
    }

    /// A `Qty` (dimensioned quantity) expression, reduced to its magnitude via `Qty.value` so it types
    /// as a scalar in any position. Reaches the QUANTITY/UNIT lowering — `Qty.of <num> <unit>`,
    /// `Unit.base`/`Unit.of`, `Qty.value`, `Qty.pow`, and Qty ARITHMETIC (unit algebra: `*` composes
    /// units, `+` requires matching units) — a value domain the crash hunt never touched (operator
    /// directive 2026-08-30). Only KNOWN units (an unknown unit declines); numeric magnitudes.
    fn qty_expr(&mut self, depth: u32) {
        self.out.push_str("(Qty.value ");
        let nunits = QTY_UNITS.len();
        match self.cur.choice(4) {
            0 => {
                let u = self.cur.choice(nunits);
                self.qty_of(depth, u);
            }
            1 => {
                // (Qty.pow <qty> <small-exp>)
                self.out.push_str("(Qty.pow ");
                let u = self.cur.choice(nunits);
                self.qty_of(depth, u);
                let _ = write!(self.out, " {})", self.cur.range(0, 3));
            }
            2 => {
                // Product — units may differ (compound unit result), Qty.value takes its magnitude.
                self.out.push_str("(* ");
                let u1 = self.cur.choice(nunits);
                self.qty_of(depth, u1);
                self.out.push(' ');
                let u2 = self.cur.choice(nunits);
                self.qty_of(depth, u2);
                self.out.push(')');
            }
            _ => {
                // Sum — operands MUST share a unit (dimension-checked), so pick ONE unit for both.
                let u = self.cur.choice(QTY_UNITS.len());
                self.out.push_str("(+ ");
                self.qty_of(depth, u);
                self.out.push(' ');
                self.qty_of(depth, u);
                self.out.push(')');
            }
        }
        self.out.push(')'); // close (Qty.value …)
    }

    /// `(Qty.of <num> (Unit.base|Unit.of #"<known-unit>"))` for the unit at `QTY_UNITS[unit_idx]`.
    fn qty_of(&mut self, depth: u32, unit_idx: usize) {
        self.out.push_str("(Qty.of ");
        self.expr(depth.saturating_sub(1), Kind::Num);
        let ctor = if self.cur.flip() {
            "Unit.base"
        } else {
            "Unit.of"
        };
        let _ = write!(self.out, " ({ctor} #\"{}\"))", QTY_UNITS[unit_idx]);
    }

    /// A GUARANTEED-TERMINATING recursive top-level def, exercising the self-call / recursion
    /// lowering (the F24 tail-resumptive-continuation + sft1 recursive-fold crash-cluster territory)
    /// that the generator otherwise never reaches. Two shapes, chosen by a coin flip. The TAIL-
    /// accumulator form `(do (def (f n acc) (if (<= n 0) acc (f (- n 1) (<arith> acc OPERAND)))) (f K 0))`
    /// puts the SOLE self-call in TAIL position with a SECOND (accumulator) argument, reaching the
    /// MULTI-ARG tail-call lowering (the `return_call` / differing-result-valtype path where invalid-
    /// wasm finding 7529f6901 lived) — a shape the non-tail form never produces, since there the self-
    /// call feeds into an arith op. The non-tail form is
    /// `(do (def (f n) (if (<= n 0) BASE (<arith> (f (- n 1)) OPERAND))) (f K))`.
    /// Termination is structural for BOTH: base case `(<= n 0)`, the SOLE recursive call passes the
    /// strictly-decreasing `(- n 1)`, and the initial argument K is a small non-negative literal — so
    /// the differential run always halts (no false hang). CRITICAL SAFETY INVARIANT: `f` is NEVER
    /// pushed into scope, so the generated BASE/OPERAND sub-exprs cannot introduce a SECOND,
    /// non-decreasing call to `f` (which would be unbounded recursion); only `n` (and, in the
    /// accumulator form, `acc`) are in scope for them.
    fn rec_def_expr(&mut self, depth: u32) {
        let f = self.env.fresh(); // deliberately NOT pushed into scope
        let n = self.env.fresh();
        if self.cur.flip() {
            // TAIL-accumulator form: the self-call is the direct else-branch, in tail position.
            let acc = self.env.fresh();
            let _ = write!(
                self.out,
                "(do (def ({f} {n} {acc}) (if (<= {n} 0) {acc} ({f} (- {n} 1) "
            );
            let mark = self.env.push(n.clone(), Kind::Num); // n + acc visible to OPERAND; f never is
            self.env.push(acc.clone(), Kind::Num);
            let op = match self.cur.choice(3) {
                0 => "+",
                1 => "-",
                _ => "*",
            };
            let _ = write!(self.out, "({op} {acc} ");
            self.expr(depth.saturating_sub(1), Kind::Num); // OPERAND (numeric; `f` not in scope → safe)
            self.out.push_str("))))");
            self.env.truncate(mark);
            // initial call: K iterations, accumulator seeded at 0; K small bounds the tail recursion.
            let _ = write!(self.out, " ({f} {} 0))", self.cur.range(0, 6));
            return;
        }
        let _ = write!(self.out, "(do (def ({f} {n}) (if (<= {n} 0) ");
        let mark = self.env.push(n.clone(), Kind::Num); // only `n` visible to sub-exprs, never `f`
        self.expr(depth.saturating_sub(1), Kind::Num); // BASE (numeric)
        let op = match self.cur.choice(3) {
            0 => "+",
            1 => "-",
            _ => "*",
        };
        let _ = write!(self.out, " ({op} ({f} (- {n} 1)) ");
        self.expr(depth.saturating_sub(1), Kind::Num); // OPERAND (numeric; `f` not in scope → safe)
        self.out.push_str(")))");
        self.env.truncate(mark);
        // initial call with a small non-negative literal bounds the recursion depth for the diff run
        let _ = write!(self.out, " ({f} {}))", self.cur.range(0, 6));
    }

    /// A self-contained, GUARANTEED-TERMINATING one-op effect + intra-program handler, reaching the
    /// perform / handle / resume lowering — the effects crash-cluster territory (F24 tail-resumptive,
    /// cmb1/pom5 sharing-aware core-emit, xhs1 cross-handler) the generator otherwise never produces.
    /// Declared in a NESTED `do` (as main's body expr) so no top-level `program()` surgery is needed:
    /// `(do (effect E (op o (-> Int64 Int64))) (handle E <init> ((o (p) s (resume <val> <newstate>))) <body>))`
    /// where `<body>` performs `E.o` a FIXED 1..=2 times. TERMINATION is structural: the single arm
    /// body is ALWAYS a bare `(resume <val> <newstate>)` — it resumes EXACTLY once — and the handled
    /// body performs a fixed, NON-recursive number of times, so the perform/resume fold is bounded.
    /// SAFETY: the effect name is NOT a value binding in [`Env`], so a generated sub-expr can never
    /// re-perform `E.o` into an unbounded loop; only the numeric param `p` (and, in the scalar variant,
    /// the scalar state `s`) is visible. The handler state is one of four shapes (choice of 4): a scalar
    /// Int64; a 2-TUPLE whose arm projects both fields and rebuilds the pair through the resume (the
    /// tuple-projection-through-handler-fold / cmb1-pom5 sharing-aware core-emit seam); an
    /// `(Option Int64)` SUM the arm DESTRUCTURES with a `match` on `Some`/`None`, threading a `(Some …)`
    /// (the `Core::SumPayload`-extraction-across-the-arm / cmb1 re-descent seam); or a RECORD with a
    /// scalar field and a HEAP list field, read-modify-written (record projection + `List.push`) through
    /// the fold (the spec "AST-node accumulator"). Int64 op/result/field/payload types so it type-checks;
    /// a mismatch with the outer `want` is a clean decline. The perform
    /// args and initial state are small literals so most programs RUN (feed the diff oracle); the resume
    /// value/state carry richer numeric sub-exprs (where a backend divergence surfaces).
    fn effect_handler_expr(&mut self, depth: u32, want: Kind) {
        let ename = {
            let v = self.env.fresh();
            format!("E{}", &v[1..]) // capitalized effect name from the fresh `v{n}` suffix, e.g. E7
        };
        let oname = self.env.fresh(); // lowercase op name, e.g. v8 — a valid operation identifier
        let p = self.env.fresh();
        let s = self.env.fresh();
        // The four Int64-state variants (scalar/tuple/sum/record) RETURN Int64; the fifth (Bool state)
        // RETURNS Bool and the sixth (String state) RETURNS String. Pick the state shape from a modulus
        // keyed on `want` so the handler's result type matches its hole and the program type-checks (a
        // mismatch is a clean decline, but a wasted seed): an Any hole may take any of the six (Bool +
        // String reachable); a Num hole stays on the four Int64 shapes; a Bool/Str hole forces its matching
        // non-Int shape (the only handler that types in that hole).
        let shape = match want {
            Kind::Bool => 4,
            Kind::Str => 5,
            Kind::Any => self.cur.choice(6),
            _ => self.cur.choice(4),
        };
        match shape {
            4 => {
                // BOOL-state variant: the effect op is `(-> Bool Bool)` and the handler threads a BOOL
                // state — reaching the Bool value codec on the handler resume/fold path that the Int64
                // state variants (scalar/tuple/sum/record) never exercise. The active effects-lowering
                // frontier (pyth1/pyce1/pyad1/pymf1 resume+replay fixes) is all Int64-state; a non-Int
                // handler state is the complementary differential coverage. Guaranteed well-typed and
                // terminating: the resume value is `(and|or A B)` and the new state is `(not C)`, where
                // A/B/C are in-scope Bool params (s, p) or Bool literals (a `leaf(Bool)` is terminal, so
                // no depth recursion); the body performs the op a FIXED 1..=2 times combined with a Bool
                // operator. Emits the whole program and RETURNS — the shared numeric tail body below would
                // feed the Bool op integer (0..9) perform args and a numeric combining op.
                let init = if self.cur.flip() { "true" } else { "false" };
                let rop = if self.cur.flip() { "and" } else { "or" };
                let _ = write!(
                    self.out,
                    "(do (effect {ename} (op {oname} (-> Bool Bool))) (handle {ename} {init} (({oname} ({p}) {s} (resume ({rop} "
                );
                let mark = self.env.push(p.clone(), Kind::Bool); // p and s are both Bool here
                self.env.push(s.clone(), Kind::Bool);
                self.leaf(Kind::Bool); // resume-value operand A (may reference s/p)
                self.out.push(' ');
                self.leaf(Kind::Bool); // resume-value operand B
                self.out.push_str(") (not "); // close (rop A B); new state is (not C)
                self.leaf(Kind::Bool); // new-state operand C
                self.env.truncate(mark);
                self.out.push_str(")))) "); // close (not C), (resume …), the op arm, the arm-list
                // handled body: perform the Bool op a FIXED 1..=2 times combined with a Bool operator.
                if self.cur.flip() {
                    let barg = if self.cur.flip() { "true" } else { "false" };
                    let _ = write!(self.out, "({ename}.{oname} {barg})");
                } else {
                    let bop = if self.cur.flip() { "and" } else { "or" };
                    let a = if self.cur.flip() { "true" } else { "false" };
                    let b = if self.cur.flip() { "true" } else { "false" };
                    let _ = write!(
                        self.out,
                        "({bop} ({ename}.{oname} {a}) ({ename}.{oname} {b}))"
                    );
                }
                self.out.push_str("))"); // close (handle …), close (do …)
                return;
            }
            5 => {
                // STRING-state variant: the effect op is `(-> String String)` and the handler threads a
                // STRING state — reaching the String value codec (heap-allocated / length-prefixed) on the
                // handler resume/fold path, the heap-value analogue of the Bool/Int scalar-state arms and
                // the differential complement to the breaker's String-state pins (e.g. pystr1). Guaranteed
                // well-typed and terminating: the resume value and new state are `(String.concat …)` over
                // the in-scope String params (s, p) and string literals (a `leaf(Str)` is terminal, so no
                // depth recursion); the body performs the op a FIXED 1..=2 times concatenated. Emits the
                // whole program and RETURNS — the shared numeric tail body below would feed the String op
                // integer (0..9) perform args and a numeric combining op.
                let _ = write!(
                    self.out,
                    "(do (effect {ename} (op {oname} (-> String String))) (handle {ename} "
                );
                self.string_lit(); // initial String state
                let _ = write!(self.out, " (({oname} ({p}) {s} (resume (String.concat ");
                let mark = self.env.push(p.clone(), Kind::Str); // p and s are both String here
                self.env.push(s.clone(), Kind::Str);
                self.leaf(Kind::Str); // resume-value operand A (may reference s/p)
                self.out.push(' ');
                self.leaf(Kind::Str); // resume-value operand B
                self.out.push_str(") (String.concat "); // close (String.concat A B); new state is another concat
                self.leaf(Kind::Str); // new-state operand C
                self.out.push(' ');
                self.leaf(Kind::Str); // new-state operand D
                self.env.truncate(mark);
                self.out.push_str(")))) "); // close 2nd (String.concat …), (resume …), the op arm, the arm-list
                // handled body: perform the String op a FIXED 1..=2 times concatenated.
                if self.cur.flip() {
                    let _ = write!(self.out, "({ename}.{oname} ");
                    self.string_lit();
                    self.out.push(')');
                } else {
                    let _ = write!(self.out, "(String.concat ({ename}.{oname} ");
                    self.string_lit();
                    let _ = write!(self.out, ") ({ename}.{oname} ");
                    self.string_lit();
                    self.out.push_str("))");
                }
                self.out.push_str("))"); // close (handle …), close (do …)
                return;
            }
            1 => {
                // TUPLE-state variant: the arm projects BOTH fields (`(. s 0)`/`(. s 1)`) and REBUILDS
                // the pair threaded through the resume — reaching the tuple-projection-through-handler-
                // fold lowering (the cmb1/pom5 sharing-aware core-emit seam: a handler STATE-TUPLE read
                // across the resume) that a scalar state never exercises. Resume value reads both fields;
                // new state advances field 0 by a generated numeric operand (over `p`) and holds field 1.
                let advance = match self.cur.choice(3) {
                    0 => "+",
                    1 => "-",
                    _ => "*",
                };
                let _ = write!(
                    self.out,
                    "(do (effect {ename} (op {oname} (-> Int64 Int64))) (handle {ename} (tuple {} {}) (({oname} ({p}) {s} (resume (+ (. {s} 0) (. {s} 1)) (tuple ({advance} (. {s} 0) ",
                    self.cur.range(0, 9),
                    self.cur.range(0, 9),
                );
                let mark = self.env.push(p, Kind::Num); // only p is scalar; s is a tuple, read via projections
                self.expr(depth.saturating_sub(1), Kind::Num); // field-0 advance operand (may reference p)
                self.env.truncate(mark);
                let _ = write!(self.out, ") (. {s} 1))))) "); // hold field 1; close tuple/resume/arm/arm-list
            }
            2 => {
                // SUM-state variant: state is an `(Option Int64)`; the arm DESTRUCTURES it with a `match`
                // on the `Some`/`None` constructors and threads a `(Some …)` — reaching the
                // `Core::SumPayload` extraction across the handler arm (the cmb1 re-descent seam: a
                // handler state read via SumPayload) that neither scalar nor tuple state hits. Seeded
                // `(Some k)`, and every resume threads `(Some …)`, so the `None` arm (kept for match
                // exhaustiveness/typing) is unreached at runtime; the payload `n` is numeric.
                let advance = match self.cur.choice(3) {
                    0 => "+",
                    1 => "-",
                    _ => "*",
                };
                let n = self.env.fresh();
                let _ = write!(
                    self.out,
                    "(do (effect {ename} (op {oname} (-> Int64 Int64))) (handle {ename} (Some {}) (({oname} ({p}) {s} (match {s} ((Some {n}) (resume (+ {n} {p}) (Some ({advance} {n} ",
                    self.cur.range(0, 9),
                );
                let mark = self.env.push(p, Kind::Num); // p and the payload n are numeric; s (Option) is not pushed
                self.env.push(n, Kind::Num);
                self.expr(depth.saturating_sub(1), Kind::Num); // Some-payload advance operand (may use p / n)
                self.env.truncate(mark);
                self.out.push_str("))))"); // close advance-op, (Some …), (resume …), Some-arm
                self.out.push_str(" (None (resume 0 (Some 0)))"); // None arm (balanced; unreached at runtime)
                self.out.push_str(")))"); // close (match …), the op arm, the arm-list
                self.out.push(' ');
            }
            3 => {
                // RECORD-state variant: state is a `(record (= n …) (= xs (list)))` combining a scalar
                // field and a HEAP (list) field; the arm PROJECTS both, advances the scalar, and rebuilds
                // the record with the param PUSHED onto the list — reaching record construction +
                // projection + heap-field read-modify-write threaded through the handler fold (the spec
                // "AST-node accumulator" shape; the compound-with-heap-field companion of tuple/sum state).
                // Resume value reads the scalar field; only the numeric `p` is pushed (the record `s` is
                // read via projections), so it type-checks (Int64 scalar field / op / result).
                let advance = match self.cur.choice(3) {
                    0 => "+",
                    1 => "-",
                    _ => "*",
                };
                let _ = write!(
                    self.out,
                    "(do (effect {ename} (op {oname} (-> Int64 Int64))) (handle {ename} (record (= n {}) (= xs (list))) (({oname} ({p}) {s} (resume (. {s} n) (record (= n ({advance} (. {s} n) ",
                    self.cur.range(0, 9),
                );
                let mark = self.env.push(p.clone(), Kind::Num); // p visible to the advance operand; s (record) read via projections
                self.expr(depth.saturating_sub(1), Kind::Num); // scalar-field advance operand (may reference p)
                self.env.truncate(mark);
                self.out.push_str("))"); // close advance-op and the (= n …) field
                let _ = write!(self.out, " (= xs ((. List push) (. {s} xs) {p}))"); // heap field: push p onto the list
                self.out.push_str("))))"); // close (record …), (resume …), the op arm, the arm-list
                self.out.push(' ');
            }
            _ => {
                // Scalar Int64 state. The arm body is one of four (choice of 4): a bare single
                // `(resume …)`; a CONDITIONAL resume — an `if` on the state with a `(resume …)` in BOTH
                // branches, a MULTI-RESUME-POINT arm (the F24-sibling / two-hole-refold territory the
                // pyr3 post-resume fixes touched); a DISCARD-resume — the arm returns a plain value and
                // NEVER resumes (zero-shot / tombstone: the handler drops the captured continuation and
                // the `(handle …)` evaluates to the arm value directly); or a MULTI-SHOT double-resume —
                // the arm resumes TWICE, combining the two continuation runs (the active replay/multi-
                // shot territory). All terminate: conditional runs one resume per perform, discard none,
                // and double-resume re-runs the continuation at most 2x (bounded — performs capped at 2,
                // depth ≤ 5, so ≤ 2^depth, no unbounded recursion).
                let init = self.cur.range(0, 9);
                let _ = write!(
                    self.out,
                    "(do (effect {ename} (op {oname} (-> Int64 Int64))) (handle {ename} {init} (({oname} ({p}) {s} "
                );
                let mark = self.env.push(p.clone(), Kind::Num); // p and s are both scalar Int64 here
                self.env.push(s.clone(), Kind::Num);
                match self.cur.choice(4) {
                    0 => {
                        let rel = match self.cur.choice(4) {
                            0 => "<",
                            1 => ">",
                            2 => "<=",
                            _ => ">=",
                        };
                        let _ = write!(
                            self.out,
                            "(if ({rel} {s} {}) (resume ",
                            self.cur.range(0, 9)
                        );
                        self.expr(depth.saturating_sub(1), Kind::Num); // branch-1 resume VALUE
                        let _ = write!(self.out, " (+ {s} {p})) (resume ");
                        self.expr(depth.saturating_sub(1), Kind::Num); // branch-2 resume VALUE
                        let _ = write!(self.out, " {s}))");
                    }
                    1 => {
                        // DISCARD: arm returns a plain numeric value, NEVER resumes (zero-shot/tombstone).
                        self.expr(depth.saturating_sub(1), Kind::Num);
                    }
                    2 => {
                        // MULTI-SHOT double-resume: the arm resumes TWICE (both threading the SAME state
                        // `s`), combining the two continuation runs — `(+ (resume V1 s) (resume V2 s))`.
                        // The active replay/multi-shot territory (14c double-resume/triple-replay pins).
                        // Bounded: re-runs the continuation at most 2x per perform, performs are capped at
                        // 2 and depth ≤ 5, so the blowup is ≤ 2^depth (small) — no unbounded recursion.
                        self.out.push_str("(+ (resume ");
                        self.expr(depth.saturating_sub(1), Kind::Num); // resume-1 VALUE
                        let _ = write!(self.out, " {s}) (resume ");
                        self.expr(depth.saturating_sub(1), Kind::Num); // resume-2 VALUE
                        let _ = write!(self.out, " {s}))");
                    }
                    _ => {
                        self.out.push_str("(resume ");
                        self.expr(depth.saturating_sub(1), Kind::Num); // resume VALUE
                        self.out.push(' ');
                        self.expr(depth.saturating_sub(1), Kind::Num); // new STATE
                        self.out.push(')');
                    }
                }
                self.env.truncate(mark);
                self.out.push_str(")) "); // close the op arm and the arm-list
            }
        }
        // handled body: perform the op a FIXED 1..=2 times (no recursion → the fold is bounded).
        if self.cur.flip() {
            let _ = write!(self.out, "({ename}.{oname} {})", self.cur.range(0, 9));
        } else {
            let op = match self.cur.choice(3) {
                0 => "+",
                1 => "-",
                _ => "*",
            };
            let _ = write!(
                self.out,
                "({op} ({ename}.{oname} {}) ({ename}.{oname} {}))",
                self.cur.range(0, 9),
                self.cur.range(0, 9)
            );
        }
        self.out.push_str("))"); // close (handle …), close (do …)
    }

    /// A self-contained, GUARANTEED-TERMINATING TWO-op effect + handler, reaching multi-op DISPATCH —
    /// the handler routes each perform to the matching op arm and threads ONE shared scalar state across
    /// the two heterogeneous ops (the "continuation destructuring across arm branches" cmb1 territory)
    /// that the single-op `effect_handler_expr` never exercises. Shape (nested `do`, no program surgery):
    /// `(do (effect E (op o1 (-> Int64 Int64)) (op o2 (-> Int64 Int64)))
    ///      (handle E <init> ((o1 (p) s (resume s (+ s <operand>))) (o2 (p) s (resume (+ s p) s)))
    ///        (<op> (E.o1 a) (E.o2 b))))`.
    /// TERMINATION/SAFETY as `effect_handler_expr`: each arm body is a bare single `(resume …)`, the body
    /// performs a FIXED (non-recursive) count, and neither effect nor op names are value bindings, so a
    /// generated operand can't re-perform into a loop. Int64 throughout so it type-checks.
    fn effect_multiop_expr(&mut self, depth: u32) {
        let e = {
            let v = self.env.fresh();
            format!("E{}", &v[1..]) // capitalized effect name, e.g. E7
        };
        let o1 = self.env.fresh();
        let o2 = self.env.fresh();
        let p1 = self.env.fresh();
        let p2 = self.env.fresh();
        let s = self.env.fresh();
        let _ = write!(
            self.out,
            "(do (effect {e} (op {o1} (-> Int64 Int64)) (op {o2} (-> Int64 Int64))) (handle {e} {} (({o1} ({p1}) {s} (resume {s} (+ {s} ",
            self.cur.range(0, 9),
        );
        let mark = self.env.push(p1, Kind::Num); // p1 and the shared scalar state s visible to the operand
        self.env.push(s.clone(), Kind::Num);
        self.expr(depth.saturating_sub(1), Kind::Num); // o1 new-state operand (may reference p1 / s)
        self.env.truncate(mark);
        self.out.push_str(")))"); // close (+ …), (resume …), the o1 arm
        let _ = write!(self.out, " ({o2} ({p2}) {s} (resume (+ {s} {p2}) {s}))"); // o2 arm (balanced)
        self.out.push(')'); // close the arm-list
        let bodyop = match self.cur.choice(3) {
            0 => "+",
            1 => "-",
            _ => "*",
        };
        let _ = write!(
            self.out,
            " ({bodyop} ({e}.{o1} {}) ({e}.{o2} {}))", // handled body: perform BOTH ops (balanced)
            self.cur.range(0, 9),
            self.cur.range(0, 9),
        );
        self.out.push_str("))"); // close (handle …), (do …)
    }

    fn access_expr(&mut self, depth: u32) {
        // (. <expr> <name-or-index>)
        self.out.push_str("(. ");
        self.expr(depth.saturating_sub(1), Kind::Any);
        self.out.push(' ');
        if self.cur.flip() {
            let _ = write!(self.out, "{}", self.cur.range(0, 5));
        } else {
            let _ = write!(self.out, "f{}", self.cur.range(0, 5));
        }
        self.out.push(')');
    }

    fn ascribe_expr(&mut self, depth: u32, want: Kind) {
        // About a third of the time, ascribe an exact int-width BOUNDARY directly to a fixed-width
        // int type (`(: 256 Int8)`). This deliberately pairs a boundary magnitude with a possibly-
        // NARROWER declared width, hitting the width-fit (CDZ0301) / const-fold-overflow (CDZ0304)
        // decision at a known operand — and where the value fits, both backends must agree (a
        // differential check). Independent of `want`: an ascription's own type governs its result,
        // so a mismatch with `want` is just a clean outer decline, not a lost input.
        if self.cur.choice(3) == 0 {
            self.out.push_str("(: ");
            self.out
                .push_str(INT_BOUNDARIES[self.cur.choice(INT_BOUNDARIES.len())]);
            self.out.push(' ');
            self.out
                .push_str(INT_FIXED_TYPES[self.cur.choice(INT_FIXED_TYPES.len())]);
            self.out.push(')');
            return;
        }
        let ty = if want == Kind::Num {
            TYPES[self.cur.choice(NUM_TYPES)] // the numeric-ish subset (Int*/UInt*/Float*)
        } else {
            TYPES[self.cur.choice(TYPES.len())]
        };
        self.out.push_str("(: ");
        self.expr(depth.saturating_sub(1), kind_of_type(ty));
        self.out.push(' ');
        self.out.push_str(ty);
        self.out.push(')');
    }

    fn match_expr(&mut self, depth: u32, want: Kind) {
        // About a third of the time (with depth to spare), emit a STRUCTURED tuple match: a tuple
        // scrutinee of known arity paired with a binding tuple PATTERN that destructures it. Unlike
        // the literal-pattern arms below, this reaches product-destructuring lowering AND binds fresh
        // names into the arm body (deepening data flow) — a match shape the generator otherwise never
        // produces. Arity-matched scrutinee+pattern guarantee it types and reaches codegen; a trailing
        // wildcard keeps it exhaustive.
        if depth > 1 && self.cur.choice(3) == 0 {
            let arity = 2 + self.cur.choice(2); // 2 or 3 elements
            self.out.push_str("(match (tuple");
            for _ in 0..arity {
                self.out.push(' ');
                self.expr(depth.saturating_sub(1), Kind::Any);
            }
            self.out.push_str(") ((tuple");
            let mark = self.env.scope.len();
            for _ in 0..arity {
                let name = self.env.fresh();
                self.out.push(' ');
                self.out.push_str(&name);
                self.env.push(name, Kind::Any);
            }
            self.out.push_str(") ");
            self.expr(depth.saturating_sub(1), want);
            self.env.truncate(mark);
            self.out.push_str(") (_ ");
            self.expr(depth.saturating_sub(1), want);
            self.out.push_str("))");
            return;
        }
        self.out.push_str("(match ");
        self.expr(depth.saturating_sub(1), Kind::Any);
        // 1..=2 arms; a wildcard arm guarantees exhaustiveness so more match nodes reach codegen.
        let arms = 1 + self.cur.choice(2);
        for i in 0..arms {
            self.out.push_str(" (");
            if i + 1 == arms {
                self.out.push('_'); // final arm is a catch-all
            } else {
                // a literal pattern
                let _ = write!(self.out, "{}", self.cur.range(0, 9));
            }
            self.out.push(' ');
            self.expr(depth.saturating_sub(1), want);
            self.out.push(')');
        }
        self.out.push(')');
    }
}

fn kind_of_type(ty: &str) -> Kind {
    match ty {
        "Bool" => Kind::Bool,
        "String" => Kind::Str,
        "Float64" | "Float32" => Kind::Num,
        _ => Kind::Num, // Int*/UInt*
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A varied (non-uniform) byte seed derived from `n`, for reachability sweeps. Uniform `[b; N]`
    /// seeds drive every choice site with the SAME byte, which can make an arm gated behind two
    /// successive `choice`s structurally unreachable; a varied seed exercises every arm combination
    /// over a large-enough sweep, so these tests stay robust to grammar-distribution changes (adding
    /// an `expr` arm shifts the modulus and would break hand-tuned fixed seeds). NOTE: this sweep's byte
    /// distribution does not reliably reach every DEEP arm-body residue (e.g. the scalar handler's
    /// discard/double-resume body choices) — those are guarded by CRAFTED seeds instead (see
    /// [`some_seed_emits_a_discard_resume_arm`] / [`some_seed_emits_a_double_resume_arm`]).
    fn varied_seed(n: u32) -> Vec<u8> {
        (0..24u32)
            .map(|i| {
                (n.wrapping_mul(2_654_435_761)
                    .wrapping_add(i.wrapping_mul(40_503))
                    >> 11) as u8
            })
            .collect()
    }

    /// Any seed — including empty, all-zero, and adversarial — produces parseable s-expr.
    #[test]
    fn every_seed_parses() {
        let seeds: &[&[u8]] = &[
            &[],
            &[0],
            &[0; 64],
            &[255; 64],
            b"the quick brown fox",
            &[3, 1, 4, 1, 5, 9, 2, 6, 5, 3, 5, 8, 9, 7, 9],
        ];
        for seed in seeds {
            let p = generate(seed);
            assert!(
                cadenza_syntax::sexpr::read(&p.source).is_ok(),
                "generated program did not parse:\n{}",
                p.source
            );
        }
    }

    /// Every width-boundary literal we feed the generator lexes as a standalone program — an
    /// out-of-range magnitude (past i64/u64) must still PARSE (arbitrary-precision literal), so the
    /// checker gets to width-decline it rather than the reader choking upstream.
    #[test]
    fn int_boundaries_all_parse() {
        for lit in INT_BOUNDARIES {
            let src = format!("(do (def (main) {lit}) (export main))");
            assert!(
                cadenza_syntax::sexpr::read(&src).is_ok(),
                "boundary literal did not parse: {lit}"
            );
        }
    }

    /// The boundary arm is actually reachable: some seed makes `num_lit` emit an exact boundary.
    /// Guards against a future refactor silently dropping the boundary bias.
    #[test]
    fn some_seed_emits_a_boundary() {
        let hit = (0u16..=255).any(|b| {
            let seed = [b as u8; 32];
            let src = generate(&seed).source;
            INT_BOUNDARIES.iter().any(|lit| src.contains(*lit))
        });
        assert!(
            hit,
            "no seed in the sweep emitted an int-width boundary literal"
        );
    }

    /// Every float boundary is a GENUINE float literal — it classifies as a `Float` leaf, not a
    /// `Name` (which would silently reject upstream and never exercise the float path we're aiming
    /// at). Uses the same classifier the front end uses so an out-of-f64-range magnitude still
    /// classifies as a float (exact `Decimal`) rather than falling through.
    #[test]
    fn float_boundaries_are_float_literals() {
        use cadenza_syntax::ast::Leaf;
        for lit in FLOAT_BOUNDARIES {
            assert!(
                matches!(cadenza_syntax::literal::classify_word(lit), Leaf::Float(_)),
                "float boundary did not classify as a Float literal: {lit}"
            );
        }
    }

    /// The float-boundary arm is reachable: some seed makes `float_lit` emit an exact boundary.
    #[test]
    fn some_seed_emits_a_float_boundary() {
        // Uses the VARIED-seed sweep (not uniform `[b; N]` seeds, which the varied_seed doc notes can
        // make an arm structurally unreachable + are fragile to grammar-modulus shifts as arms are added).
        let hit = (0u32..8000).any(|n| {
            let src = generate(&varied_seed(n)).source;
            FLOAT_BOUNDARIES.iter().any(|lit| src.contains(*lit))
        });
        assert!(hit, "no seed in the sweep emitted a float boundary literal");
    }

    /// The boundary-ascription arm is reachable and every program it can emit still parses (a
    /// `(: <boundary> <IntType>)` node is a width-fit/overflow probe — it must reach the checker,
    /// not choke the reader).
    #[test]
    fn some_seed_emits_a_boundary_ascription() {
        let mut hit = false;
        for b0 in 0u16..=255 {
            for b1 in 0u16..=255 {
                let seed = [b0 as u8, b1 as u8, b0 as u8, b1 as u8, 0, 0, 0, 0];
                let src = generate(&seed).source;
                assert!(
                    cadenza_syntax::sexpr::read(&src).is_ok(),
                    "generated program did not parse:\n{src}"
                );
                if INT_FIXED_TYPES
                    .iter()
                    .any(|t| src.contains(&format!(" {t})")))
                    && INT_BOUNDARIES.iter().any(|lit| src.contains(*lit))
                {
                    hit = true;
                }
            }
        }
        assert!(hit, "no seed in the sweep emitted a boundary ascription");
    }

    /// Every fixed-width int type we ascribe boundaries to pairs with every boundary literal into a
    /// program that PARSES — so the 16- and 32-bit width-fit/overflow seams (`Int16`/`UInt16`/`UInt32`)
    /// are actually reachable via ascription, not just the 8/64-bit ones. A parse failure here would
    /// mean a boundary/type token the reader chokes on before the width checker ever sees it.
    #[test]
    fn every_fixed_type_boundary_ascription_parses() {
        for t in INT_FIXED_TYPES {
            for lit in INT_BOUNDARIES {
                let src = format!("(do (def (main) (: {lit} {t})) (export main))");
                assert!(
                    cadenza_syntax::sexpr::read(&src).is_ok(),
                    "boundary ascription did not parse: (: {lit} {t})"
                );
            }
        }
    }

    /// The numeric-ish ascription prefix ([`NUM_TYPES`] leading entries of [`TYPES`]) is exactly the
    /// `Int*`/`UInt*`/`Float*` types — no `Bool`/`String` leaks into the slice a `want == Num`
    /// ascription draws from. Guards the prefix invariant against a future `TYPES` reordering.
    #[test]
    fn numeric_type_prefix_is_well_formed() {
        assert!(NUM_TYPES <= TYPES.len());
        for ty in &TYPES[..NUM_TYPES] {
            assert!(
                kind_of_type(ty) == Kind::Num,
                "non-numeric type {ty} inside the numeric prefix"
            );
        }
        for ty in &TYPES[NUM_TYPES..] {
            assert!(
                kind_of_type(ty) != Kind::Num,
                "numeric type {ty} outside the numeric prefix"
            );
        }
    }

    /// Every operator head (incl. the bitwise/shift additions `&`/`|`/`^`/`<<`/`>>`) forms a
    /// parseable binary application `(<head> 1 2)` — a malformed head string would silently reject
    /// upstream and never reach the arithmetic/bitwise lowering the differential oracle targets.
    #[test]
    fn every_op_head_forms_a_parseable_application() {
        for op in OPS {
            let head = match op {
                Op::Arith(h) | Op::Rel(h) | Op::Logic(h) => *h,
            };
            let src = format!("(do (def (main) ({head} 1 2)) (export main))");
            assert!(
                cadenza_syntax::sexpr::read(&src).is_ok(),
                "operator application did not parse: ({head} 1 2)"
            );
        }
    }

    /// The structured tuple-destructuring match arm is reachable — some seed emits a binding tuple
    /// pattern (`(match (tuple ...) ((tuple ...) ...) (_ ...))`) — and every program that path can
    /// emit still parses. Guards the product-destructuring reach against a refactor that drops it or
    /// emits an unparseable pattern.
    #[test]
    fn some_seed_emits_a_tuple_destructuring_match() {
        let mut hit = false;
        for n in 0..8000u32 {
            let seed = varied_seed(n);
            let src = generate(&seed).source;
            assert!(
                cadenza_syntax::sexpr::read(&src).is_ok(),
                "generated program did not parse:\n{src}"
            );
            if src.contains("(match (tuple") && src.contains("((tuple") {
                hit = true;
            }
        }
        assert!(
            hit,
            "no seed in the sweep emitted a tuple-destructuring match"
        );
    }

    /// The List-builtin arm is reachable — some seed emits a `(List.len|at|update ...)` call over a
    /// list — and every program that path can emit still parses. Guards the list-runtime reach
    /// (ListAt/ListUpdate width-alias territory) against a refactor that drops it.
    #[test]
    fn some_seed_emits_a_list_builtin() {
        let mut hit = false;
        for n in 0..8000u32 {
            let seed = varied_seed(n);
            let src = generate(&seed).source;
            assert!(
                cadenza_syntax::sexpr::read(&src).is_ok(),
                "generated program did not parse:\n{src}"
            );
            if src.contains("(List.len ")
                || src.contains("(List.at ")
                || src.contains("(List.update ")
            {
                hit = true;
            }
        }
        assert!(hit, "no seed in the sweep emitted a List builtin");
    }

    /// The String-builtin arm is reachable — some seed emits a `(String.at|slice|concat|byte-len|
    /// to-bytes ...)` call — and every program that path can emit still parses. Guards the string
    /// runtime reach (StrAt/BytesAt width-alias territory) against a refactor that drops it.
    #[test]
    fn some_seed_emits_a_string_builtin() {
        let mut hit = false;
        for n in 0..8000u32 {
            let seed = varied_seed(n);
            let src = generate(&seed).source;
            assert!(
                cadenza_syntax::sexpr::read(&src).is_ok(),
                "generated program did not parse:\n{src}"
            );
            if src.contains("(String.") {
                hit = true;
            }
        }
        assert!(hit, "no seed in the sweep emitted a String builtin");
    }

    /// The Map/Set-builtin arm is reachable — some seed emits a `(Map.*|Set.* ...)` call over a
    /// freshly-built numeric map/set — and every program that path can emit still parses. Guards the
    /// HEAP-COLLECTION runtime reach (hash map/set lowering — a subsystem the crash hunt never touched
    /// before this arm; operator directive 2026-08-30 to keep expanding generated inputs).
    #[test]
    fn some_seed_emits_a_map_set_builtin() {
        let mut hit = false;
        for n in 0..8000u32 {
            let seed = varied_seed(n);
            let src = generate(&seed).source;
            assert!(
                cadenza_syntax::sexpr::read(&src).is_ok(),
                "generated program did not parse:\n{src}"
            );
            if src.contains("(Map.") || src.contains("(Set.") {
                // A map/set op must be built over a non-empty operand (Map.insert Map.empty … / Set.of).
                assert!(
                    src.contains("Map.empty") || src.contains("(Set.of "),
                    "map/set builtin without a constructed operand:\n{src}"
                );
                hit = true;
            }
        }
        assert!(hit, "no seed in the sweep emitted a Map/Set builtin");
    }

    /// The record arm is reachable — some seed emits a `(record (= a …) …)` in GENERAL position (not just
    /// effect-handler state), and every program it can emit parses. Guards the record construction +
    /// projection reach (operator directive 2026-08-30 to keep expanding generated inputs).
    #[test]
    fn some_seed_emits_a_record() {
        let mut hit = false;
        let mut saw_projection = false;
        for n in 0..8000u32 {
            let seed = varied_seed(n);
            let src = generate(&seed).source;
            assert!(
                cadenza_syntax::sexpr::read(&src).is_ok(),
                "generated program did not parse:\n{src}"
            );
            if src.contains("(record (= a ") {
                hit = true;
                if src.contains("(. (record (= a ") {
                    saw_projection = true;
                }
            }
        }
        assert!(hit, "no seed in the sweep emitted a record");
        assert!(saw_projection, "no seed projected a field from a record");
    }

    /// The sum arm is reachable — some seed emits an Option/Result CONSTRUCTION and some seed a
    /// construct-then-MATCH destructuring — and every program it can emit parses. Guards the sum
    /// construction + match-dispatch reach in general position (operator directive 2026-08-30).
    #[test]
    fn some_seed_emits_a_sum() {
        // `(Ok …)`/`(Err …)` (Result) are emitted ONLY by `sum_expr` — effect-handler state uses
        // Option (`(Some …)`) — so a Result ctor + a Result-match isolate the new arm specifically.
        let mut saw_result_ctor = false;
        let mut saw_result_match = false;
        for n in 0..8000u32 {
            let seed = varied_seed(n);
            let src = generate(&seed).source;
            assert!(
                cadenza_syntax::sexpr::read(&src).is_ok(),
                "generated program did not parse:\n{src}"
            );
            if src.contains("(Ok ") || src.contains("(Err ") {
                saw_result_ctor = true;
            }
            if src.contains("((Ok ") || src.contains("((Err ") {
                saw_result_match = true;
            }
        }
        assert!(
            saw_result_ctor,
            "no seed in the sweep emitted a Result constructor (sum_expr arm)"
        );
        assert!(
            saw_result_match,
            "no seed in the sweep emitted a Result-destructuring match (sum_expr arm)"
        );
    }

    /// The user-sum program shape is reachable — some seed emits a TOP-LEVEL `(type …)` declaration with
    /// a matching `main` that constructs + matches a user ctor — and every such program parses. Guards
    /// the sum type-decl / user-ctor construct + match emit reach (operator directive 2026-08-30).
    #[test]
    fn some_seed_emits_a_user_sum_type_decl() {
        let mut hit = false;
        for n in 0..8000u32 {
            let seed = varied_seed(n);
            let src = generate(&seed).source;
            assert!(
                cadenza_syntax::sexpr::read(&src).is_ok(),
                "generated program did not parse:\n{src}"
            );
            if src.contains("(do (type ") {
                assert!(
                    src.contains("((Circle a)")
                        || src.contains("((Red) ")
                        || src.contains("((Mk a b)"),
                    "user-sum program declared a type without the expected match arms:\n{src}"
                );
                hit = true;
            }
        }
        assert!(
            hit,
            "no seed in the sweep emitted a user-sum type declaration"
        );
    }

    /// The Char arm is reachable — some seed emits `(Char.from-int …)` and some a matched round-trip via
    /// `Char.to-int` — and every such program parses. Guards the Char scalar (codepoint) lowering reach
    /// (operator directive 2026-08-30 to keep expanding generated inputs).
    #[test]
    fn some_seed_emits_a_char() {
        let mut saw_ctor = false;
        let mut saw_roundtrip = false;
        for n in 0..8000u32 {
            let seed = varied_seed(n);
            let src = generate(&seed).source;
            assert!(
                cadenza_syntax::sexpr::read(&src).is_ok(),
                "generated program did not parse:\n{src}"
            );
            if src.contains("(Char.from-int ") {
                saw_ctor = true;
            }
            if src.contains("(Char.to-int ") {
                saw_roundtrip = true;
            }
        }
        assert!(saw_ctor, "no seed in the sweep emitted (Char.from-int …)");
        assert!(
            saw_roundtrip,
            "no seed in the sweep emitted a Char.to-int round-trip"
        );
    }

    /// The Qty arm is reachable — some seed emits `(Qty.of … (Unit.base|Unit.of #"<known>"))` reduced via
    /// `Qty.value` — and every such program parses, using ONLY known units. Guards the quantity/unit
    /// lowering reach (operator directive 2026-08-30 to keep expanding generated inputs).
    #[test]
    fn some_seed_emits_a_qty() {
        let mut hit = false;
        for n in 0..8000u32 {
            let seed = varied_seed(n);
            let src = generate(&seed).source;
            assert!(
                cadenza_syntax::sexpr::read(&src).is_ok(),
                "generated program did not parse:\n{src}"
            );
            if src.contains("(Qty.of ") {
                assert!(
                    src.contains("(Qty.value "),
                    "Qty built without reducing via Qty.value:\n{src}"
                );
                // Every emitted unit is a known one (an unknown unit would decline).
                assert!(
                    QTY_UNITS.iter().any(|u| src.contains(&format!("#\"{u}\""))),
                    "Qty emitted with no known unit:\n{src}"
                );
                hit = true;
            }
        }
        assert!(hit, "no seed in the sweep emitted a Qty");
    }

    /// The try/`?` program shape is reachable — some seed emits a `(try (Ok …))` inside a Result-returning
    /// helper matched by main — and every such program parses. Guards the try/`?` fallible-boundary
    /// desugaring reach (operator directive 2026-08-30 to keep expanding generated program shapes).
    #[test]
    fn some_seed_emits_a_try_operator() {
        let mut hit = false;
        for n in 0..8000u32 {
            let seed = varied_seed(n);
            let src = generate(&seed).source;
            assert!(
                cadenza_syntax::sexpr::read(&src).is_ok(),
                "generated program did not parse:\n{src}"
            );
            if src.contains("(try (Ok ") {
                assert!(
                    src.contains("(Result Int64 Int64)") && src.contains("((Err e)"),
                    "try program without its Result fallible boundary + match:\n{src}"
                );
                hit = true;
            }
        }
        assert!(hit, "no seed in the sweep emitted a try/? program");
    }

    /// The recursive-def arm is reachable — some seed emits a NESTED `(def (...` helper (main is the
    /// only other def) — and every program that path can emit still parses. Also asserts the
    /// TERMINATION STRUCTURE holds on every such program: the helper carries a `(if (<= ` base-case
    /// guard and its SOLE self-application decrements via `(- `, so the differential run can't hang.
    #[test]
    fn some_seed_emits_a_terminating_recursive_def() {
        let mut hit = false;
        for n in 0..8000u32 {
            let seed = varied_seed(n);
            let src = generate(&seed).source;
            assert!(
                cadenza_syntax::sexpr::read(&src).is_ok(),
                "generated program did not parse:\n{src}"
            );
            // `main` is the only top-level def; a SECOND `(def (` is a recursive helper — EXCEPT in the
            // special whole-program shapes, whose top-level helper is not a rec_def_expr helper: a
            // try/`?` program has a sibling `(def (f) …)` boundary fn (no recursion), and a user-sum
            // program declares a type. Exclude those so this only checks genuine normal-path rec-defs.
            let is_special = src.contains("(try (Ok ") || src.contains("(do (type ");
            if !is_special && src.matches("(def (").count() >= 2 {
                hit = true;
                assert!(
                    src.contains("(if (<= "),
                    "recursive helper is missing its base-case guard `(if (<= `:\n{src}"
                );
            }
        }
        assert!(
            hit,
            "no seed in the sweep emitted a recursive def (nested `(def (`)"
        );
    }

    /// The TAIL-accumulator recursive form is reachable — some seed emits a THREE-parameter helper
    /// def `(def (vA vB vC) ...)` (only the accumulator arm mints a 3-name param list; `main` takes
    /// 0..=2 params and no other def is generated) — and every program that path can emit still
    /// parses. Also asserts the termination structure holds: the helper carries a `(if (<= ` base-case
    /// guard so the multi-arg tail recursion can't hang the differential run.
    #[test]
    fn some_seed_emits_a_tail_accumulator_recursive_def() {
        // A `(def (` whose parenthesized param list holds exactly three `v<n>` identifiers.
        fn has_three_param_def(src: &str) -> bool {
            let mut rest = src;
            while let Some(i) = rest.find("(def (") {
                let after = &rest[i + "(def (".len()..];
                if let Some(close) = after.find(')') {
                    let params = &after[..close];
                    let toks: Vec<&str> = params.split_whitespace().collect();
                    if toks.len() == 3 && toks.iter().all(|t| t.starts_with('v')) {
                        return true;
                    }
                }
                rest = &rest[i + "(def (".len()..];
            }
            false
        }
        let mut hit = false;
        for n in 0..8000u32 {
            let seed = varied_seed(n);
            let src = generate(&seed).source;
            assert!(
                cadenza_syntax::sexpr::read(&src).is_ok(),
                "generated program did not parse:\n{src}"
            );
            if has_three_param_def(&src) {
                hit = true;
                assert!(
                    src.contains("(if (<= "),
                    "tail-accumulator helper is missing its base-case guard `(if (<= `:\n{src}"
                );
            }
        }
        assert!(
            hit,
            "no seed in the sweep emitted a tail-accumulator recursive def (3-param helper)"
        );
    }

    /// The effect-handler arm is reachable — some seed emits an `(effect …)` declaration paired with
    /// a `(handle …)` — and every program that path can emit still parses. (A handler need NOT carry a
    /// `(resume …)`: the discard/tombstone arm is total by dropping its continuation — see
    /// [`some_seed_emits_a_discard_resume_arm`]. Resume-bearing arms resume at most once per perform,
    /// so the fold stays bounded either way.)
    #[test]
    fn some_seed_emits_an_effect_handler() {
        let mut hit = false;
        let mut hit_tuple_state = false;
        let mut hit_sum_state = false;
        let mut hit_record_state = false;
        for n in 0..8000u32 {
            let seed = varied_seed(n);
            let src = generate(&seed).source;
            assert!(
                cadenza_syntax::sexpr::read(&src).is_ok(),
                "generated program did not parse:\n{src}"
            );
            if src.contains("(effect ") && src.contains("(handle ") {
                hit = true;
                // The tuple-state variant threads a `(tuple …)` state read back via `(. s N)` projections.
                if src.contains("(tuple ") && src.contains("(. ") {
                    hit_tuple_state = true;
                }
                // The sum-state variant threads a `(Some …)` state destructured by a `(match …)` arm.
                if src.contains("(Some ") && src.contains("(match ") {
                    hit_sum_state = true;
                }
                // The record-state variant threads a `(record …)` state with a heap field via `List push`.
                if src.contains("(record ") && src.contains("(. List push)") {
                    hit_record_state = true;
                }
            }
        }
        assert!(hit, "no seed in the sweep emitted an effect handler");
        assert!(
            hit_tuple_state,
            "no seed in the sweep emitted a TUPLE-state effect handler"
        );
        assert!(
            hit_sum_state,
            "no seed in the sweep emitted a SUM-state (Option) effect handler"
        );
        assert!(
            hit_record_state,
            "no seed in the sweep emitted a RECORD-state effect handler"
        );
    }

    /// The conditional-resume (MULTI-RESUME-POINT) scalar arm is reachable — some seed emits a handler
    /// arm whose body is an `(if …)` with a `(resume …)` in BOTH branches — and every program that path
    /// can emit still parses. The branch-1 `(resume V (+ s p))` immediately followed by branch-2
    /// ` (resume …)` yields the distinctive `)) (resume ` substring no other arm produces. Guards the
    /// two-hole-refold / F24-sibling reach against a refactor that drops it.
    #[test]
    fn some_seed_emits_a_conditional_resume_arm() {
        let mut hit = false;
        for n in 0..8000u32 {
            let seed = varied_seed(n);
            let src = generate(&seed).source;
            assert!(
                cadenza_syntax::sexpr::read(&src).is_ok(),
                "generated program did not parse:\n{src}"
            );
            if src.contains("(handle ") && src.contains(")) (resume ") {
                hit = true;
            }
        }
        assert!(hit, "no seed in the sweep emitted a conditional-resume arm");
    }

    /// The discard-resume (zero-shot / tombstone) arm is reachable — driven by a CRAFTED seed that forces
    /// main-body → effect_handler (b1 % 19 == 16) → scalar state (b2 % 4 == 0) → body choice discard
    /// (b4 % 4 == 1). The whole program then contains a `(handle …)` yet NO `(resume …)` (the discard arm
    /// abandons its continuation; every other arm shape emits a `(resume …)`), and it parses. (The generic
    /// `varied_seed` sweep's byte distribution does not reach this deep body residue.)
    #[test]
    fn some_seed_emits_a_discard_resume_arm() {
        // Leading `1` consumes program()'s user-sum-vs-normal `choice(4)` (1 % 4 != 0 → normal main),
        // then the original bytes align: b1 % 22 == 16 → effect_handler, b2 % 4 == 0 → scalar state, …
        let seed = [1u8, 0, 16, 0, 3, 1, 2, 2, 2, 2, 2, 2, 2];
        let src = generate(&seed).source;
        assert!(
            cadenza_syntax::sexpr::read(&src).is_ok(),
            "generated program did not parse:\n{src}"
        );
        assert!(
            src.contains("(handle ") && !src.contains("(resume "),
            "crafted seed did not emit a discard-resume arm (handler without resume):\n{src}"
        );
    }

    /// The multi-shot double-resume arm is reachable — driven by a CRAFTED seed that forces
    /// main-body → effect_handler (b1 % 19 == 16) → scalar state (b2 % 4 == 0) → body choice double
    /// (b4 % 4 == 2). Asserts the arm emits the distinctive `(+ (resume … ) (resume … ))` (two resume
    /// sites; a generated resume value never emits `(resume …)`) and that the program parses. (The
    /// generic `varied_seed` sweep's byte distribution does not reach this deep body residue.)
    #[test]
    fn some_seed_emits_a_double_resume_arm() {
        // Leading `1` consumes program()'s user-sum-vs-normal `choice(4)` (1 % 4 != 0 → normal main),
        // then the original bytes align: b1 % 22 == 16 → effect_handler, b2 % 4 == 0 → scalar, b4 → double.
        let seed = [1u8, 0, 16, 0, 3, 2, 2, 2, 2, 2, 2, 2, 2];
        let src = generate(&seed).source;
        assert!(
            cadenza_syntax::sexpr::read(&src).is_ok(),
            "generated program did not parse:\n{src}"
        );
        assert!(
            src.contains("(+ (resume "),
            "crafted seed did not emit a double-resume arm:\n{src}"
        );
    }

    /// The two-op effect variant is reachable — some seed emits an `(effect …)` declaring TWO `(op …)`
    /// clauses — and every program that path can emit still parses. Guards the multi-op dispatch reach
    /// (two handler arms routing distinct performs) against a refactor that drops it.
    #[test]
    fn some_seed_emits_a_two_op_effect() {
        let mut hit = false;
        for n in 0..8000u32 {
            let seed = varied_seed(n);
            let src = generate(&seed).source;
            assert!(
                cadenza_syntax::sexpr::read(&src).is_ok(),
                "generated program did not parse:\n{src}"
            );
            // A two-op effect declaration carries two `(op ` clauses inside a single `(effect …)`.
            if let Some(decl) = src.find("(effect ")
                && let Some(end) = src[decl..].find("(handle ")
                && src[decl..decl + end].matches("(op ").count() >= 2
            {
                hit = true;
                assert!(
                    src.contains("(resume "),
                    "two-op effect handler is missing a `(resume `:\n{src}"
                );
            }
        }
        assert!(hit, "no seed in the sweep emitted a two-op effect");
    }

    /// The BOOL-state effect handler is reachable — some seed emits an effect op typed `(-> Bool Bool)`
    /// discharged by a `(handle …)`, threading a non-Int (Bool) handler state. This exercises the Bool
    /// value codec on the handler resume/fold path that the Int64 state variants never reach (the
    /// complementary differential coverage for the active effects-lowering frontier). Every program that
    /// path can emit still parses. Guards the non-Int handler-state reach against a refactor that drops it.
    #[test]
    fn some_seed_emits_a_bool_state_effect_handler() {
        let mut hit = false;
        for n in 0..8000u32 {
            let seed = varied_seed(n);
            let src = generate(&seed).source;
            assert!(
                cadenza_syntax::sexpr::read(&src).is_ok(),
                "generated program did not parse:\n{src}"
            );
            if src.contains("(-> Bool Bool)") && src.contains("(handle ") {
                hit = true;
            }
        }
        assert!(
            hit,
            "no seed in the sweep emitted a BOOL-state effect handler"
        );
    }

    /// The STRING-state effect handler is reachable — some seed emits an effect op typed
    /// `(-> String String)` discharged by a `(handle …)`, threading a non-Int (String) handler state.
    /// This exercises the heap String value codec on the handler resume/fold path (the heap-value
    /// analogue of the Bool-state reach; complementary differential coverage for the effects frontier).
    /// Every program that path can emit still parses. Guards the String handler-state reach against a
    /// refactor that drops it.
    #[test]
    fn some_seed_emits_a_string_state_effect_handler() {
        let mut hit = false;
        for n in 0..8000u32 {
            let seed = varied_seed(n);
            let src = generate(&seed).source;
            assert!(
                cadenza_syntax::sexpr::read(&src).is_ok(),
                "generated program did not parse:\n{src}"
            );
            if src.contains("(-> String String)") && src.contains("(handle ") {
                hit = true;
            }
        }
        assert!(
            hit,
            "no seed in the sweep emitted a STRING-state effect handler"
        );
    }

    /// Generation is deterministic in the seed (required for reproducing + shrinking a finding).
    #[test]
    fn deterministic() {
        let seed = b"deterministic?";
        assert_eq!(generate(seed).source, generate(seed).source);
    }

    /// Termination on a long random-ish seed (budget actually bounds it).
    #[test]
    fn terminates_and_is_bounded() {
        let seed: Vec<u8> = (0..10_000u32)
            .map(|i| (i.wrapping_mul(2654435761) >> 13) as u8)
            .collect();
        let p = generate(&seed);
        assert!(cadenza_syntax::sexpr::read(&p.source).is_ok());
        // node_cap bounds the tree; the string can't be enormous.
        assert!(
            p.source.len() < 100_000,
            "unbounded output: {} bytes",
            p.source.len()
        );
    }
}
