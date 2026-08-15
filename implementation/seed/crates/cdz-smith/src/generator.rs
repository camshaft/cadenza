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
    /// `(do (def (main <params>) <body>) (export main))`
    fn program(&mut self) {
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

    /// Emit one expression of the requested (hint) kind, within the depth budget.
    fn expr(&mut self, depth: u32, want: Kind) {
        self.nodes += 1;
        if depth == 0 || self.nodes >= self.node_cap {
            self.leaf(want);
            return;
        }
        // Weighted toward leaves + operators + control flow (the shapes most likely to type and
        // reach codegen); the tail arms exercise ctors, access, ascription, and match.
        match self.cur.choice(15) {
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
    /// an `expr` arm shifts the modulus and would break hand-tuned fixed seeds).
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
        let hit = (0u16..=255).any(|b| {
            let seed = [b as u8; 48];
            let src = generate(&seed).source;
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
