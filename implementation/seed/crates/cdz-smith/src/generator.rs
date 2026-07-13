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
const TYPES: &[&str] = &[
    "Int64", "Int32", "Int8", "UInt64", "UInt8", "Bool", "String", "Float64",
];

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
    Op::Rel("<"),
    Op::Rel(">"),
    Op::Rel("<="),
    Op::Rel(">="),
    Op::Rel("="),
    Op::Logic("and"),
    Op::Logic("or"),
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
        match self.cur.choice(14) {
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
        // A spread of magnitudes incl. boundary-ish values, occasionally negated.
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
        let ty = if want == Kind::Num {
            TYPES[self.cur.choice(6)] // the numeric-ish subset (Int*/UInt*)
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
        "Float64" => Kind::Num,
        _ => Kind::Num, // Int*/UInt*
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
