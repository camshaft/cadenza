//! cdz-cad — the native mesh driver for Cadenza CAD (GH #400, increment G2).
//!
//! # What this crate does
//!
//! Two halves: (1) THIS module parses a Cadenza program's rendered `Solid` value (canonical s-expr text)
//! into a [`Solid`] CSG tree; (2) [`mesh`] walks that tree into a manifold mesh (`manifold-csg`). A CLI
//! sub-slice (`cdz cad`) wires "run the program → parse → mesh → write 3MF/glTF/STL" together.
//!
//! A Cadenza program built on `implementation/cad`'s EXACT `Solidr` model describes a solid as a recursive
//! `Solidr` value with `Rational` coordinates. When such a program's single export crosses the component
//! boundary, cdz-run renders the compound value to CANONICAL S-EXPRESSION TEXT (the B1 "render-tree-as-data"
//! seam). For example a plate = cube minus a sphere crosses as:
//!
//! ```text
//! (: (Differencer (Cuber (: (tuple 50/1 30/1 5/1) Vec3r)) (Spherer 127/20)) Solidr)
//! ```
//!
//! This module parses that text into a [`Solid`] tree the [`mesh`] backend walks into `manifold-csg`. The
//! render grammar this parser accepts (R4 — the EXACT model; the legacy Float64 `Solid` form is retired):
//!   * the WHOLE value carries an outer type annotation `(: <value> Solidr)`; NESTED solids are bare
//!     `(Ctor …)` (no per-node annotation);
//!   * each `Vec3r` is annotated `(: (tuple x y z) Vec3r)`;
//!   * a sum constructor is `(Ctor arg…)` — `Unionr`/`Differencer`/`Intersectionr` take two `Solidr` args;
//!     `Translater`/`Scaler` take a `Vec3r` then a `Solidr`; `Cuber` takes a `Vec3r` (FULL size);
//!     `Spherer` one Rational (radius); `Cylinderr` two Rationals (FULL height, radius). There is NO
//!     `Rotater` (a general rotation has no exact Rational form);
//!   * a NULLARY constructor renders WITH a `unit` payload — `Emptyr` is `(Emptyr unit)`;
//!   * a number leaf is a RATIONAL `n/d` (`50/1`, `127/20`), evaluated to `f64` (`n/d`) at the mesh leaf —
//!     the MODEL stays exact; the geometry kernel works in float. (Bare int / `N.0` accepted defensively.)
//!
//! The parser unwraps a `(: v T)` annotation transparently wherever a value is expected, so it also accepts
//! a fully-annotated form (belt-and-suspenders — it never depends on WHICH positions are annotated).

use std::fmt;

pub mod bounds;
pub mod gltf;
pub mod mesh;
pub mod stl;
pub use bounds::{bounds, bounds_with_segments, Bounds};
pub use mesh::{
    mesh, mesh_with_segments, to_manifold, to_manifold_with_segments, Mesh, DEFAULT_SEGMENTS,
};

/// A 3-D vector / point — the parsed form of the library's `Vec3` (a `(tuple x y z)`), all `f64`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Vec3 {
    pub fn new(x: f64, y: f64, z: f64) -> Self {
        Vec3 { x, y, z }
    }
}

/// The parsed CSG tree — the driver's in-memory mirror of the Cadenza `Solid` sum. The mesh backend walks
/// this into geometry; keeping it a plain owned tree (children boxed) means the walk is an ordinary match.
#[derive(Clone, PartialEq, Debug)]
pub enum Solid {
    Empty,
    Cube(Vec3),
    Sphere(f64),
    /// `(height, radius)`.
    Cylinder(f64, f64),
    Union(Box<Solid>, Box<Solid>),
    Difference(Box<Solid>, Box<Solid>),
    Intersection(Box<Solid>, Box<Solid>),
    Translate(Vec3, Box<Solid>),
    Rotate(Vec3, Box<Solid>),
    Scale(Vec3, Box<Solid>),
}

impl Solid {
    /// The number of primitive leaves (`Cube`/`Sphere`/`Cylinder`) — the geometry the backend emits.
    /// Mirrors the library's `leaf-count`, so a parsed tree can be cross-checked against the program's own
    /// fold. `Empty` contributes none; booleans/transforms are structure.
    pub fn leaf_count(&self) -> usize {
        match self {
            Solid::Empty => 0,
            Solid::Cube(_) | Solid::Sphere(_) | Solid::Cylinder(_, _) => 1,
            Solid::Union(a, b) | Solid::Difference(a, b) | Solid::Intersection(a, b) => {
                a.leaf_count() + b.leaf_count()
            }
            Solid::Translate(_, s) | Solid::Rotate(_, s) | Solid::Scale(_, s) => s.leaf_count(),
        }
    }

    /// The total node count (the node itself plus every descendant) — mirrors the library's `count-nodes`.
    pub fn node_count(&self) -> usize {
        match self {
            Solid::Empty | Solid::Cube(_) | Solid::Sphere(_) | Solid::Cylinder(_, _) => 1,
            Solid::Union(a, b) | Solid::Difference(a, b) | Solid::Intersection(a, b) => {
                1 + a.node_count() + b.node_count()
            }
            Solid::Translate(_, s) | Solid::Rotate(_, s) | Solid::Scale(_, s) => 1 + s.node_count(),
        }
    }
}

/// A parse failure, with a human-readable reason. The driver surfaces this on stderr with a non-zero exit;
/// it should never fire on well-formed cdz-run output, so a failure means the render form drifted.
#[derive(Clone, PartialEq, Debug)]
pub struct ParseError(pub String);

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cad parse error: {}", self.0)
    }
}

impl std::error::Error for ParseError {}

/// Parse a rendered `Solid` s-expression (cdz-run's canonical text) into a [`Solid`] tree.
pub fn parse_solid(text: &str) -> Result<Solid, ParseError> {
    let toks = tokenize(text);
    let mut p = Parser {
        toks: &toks,
        pos: 0,
        depth: 0,
    };
    let s = p.parse_solid_value()?;
    if p.pos != p.toks.len() {
        return Err(ParseError(format!(
            "trailing tokens after the solid (at token {} of {})",
            p.pos,
            p.toks.len()
        )));
    }
    Ok(s)
}

/// One s-expression token: a paren or an atom (a bareword like `Union`/`tuple`/`unit`/`:`, or a number).
#[derive(Clone, PartialEq, Debug)]
enum Tok {
    Open,
    Close,
    Atom(String),
}

/// Split the text into tokens. Parens are single-char tokens; everything else is whitespace-delimited atoms
/// (`:` is its own atom, the head of a `(: v T)` annotation). No strings appear in a `Solid`.
fn tokenize(text: &str) -> Vec<Tok> {
    let mut toks = Vec::new();
    let mut cur = String::new();
    fn flush(cur: &mut String, toks: &mut Vec<Tok>) {
        if !cur.is_empty() {
            toks.push(Tok::Atom(std::mem::take(cur)));
        }
    }
    for c in text.chars() {
        match c {
            '(' => {
                flush(&mut cur, &mut toks);
                toks.push(Tok::Open);
            }
            ')' => {
                flush(&mut cur, &mut toks);
                toks.push(Tok::Close);
            }
            c if c.is_whitespace() => flush(&mut cur, &mut toks),
            c => cur.push(c),
        }
    }
    flush(&mut cur, &mut toks);
    toks
}

/// The maximum `Solid` nesting depth the recursive-descent parser accepts. The parser (and the tree walks
/// that follow: `mesh`, `node_count`) recurse one stack frame per level, so an ADVERSARIAL deeply-nested
/// input could otherwise overflow the thread stack (empirically ~400 levels on a 2 MiB stack). A real CAD
/// model never nests remotely this deep (a gear/bracket is a handful of levels), so a generous cap turns a
/// crash into a clean `Err` with no practical cost. 256 is far above any real model, far below the overflow.
const MAX_DEPTH: usize = 256;

struct Parser<'a> {
    toks: &'a [Tok],
    pos: usize,
    /// Current `Solid` nesting depth (guards against a stack-overflowing adversarial input — see MAX_DEPTH).
    depth: usize,
}

impl Parser<'_> {
    fn bump(&mut self) -> Option<&Tok> {
        let t = self.toks.get(self.pos);
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn expect_open(&mut self) -> Result<(), ParseError> {
        match self.bump() {
            Some(Tok::Open) => Ok(()),
            other => Err(ParseError(format!("expected `(`, found {other:?}"))),
        }
    }

    fn expect_close(&mut self) -> Result<(), ParseError> {
        match self.bump() {
            Some(Tok::Close) => Ok(()),
            other => Err(ParseError(format!("expected `)`, found {other:?}"))),
        }
    }

    fn expect_atom(&mut self) -> Result<String, ParseError> {
        match self.bump() {
            Some(Tok::Atom(a)) => Ok(a.clone()),
            other => Err(ParseError(format!("expected an atom, found {other:?}"))),
        }
    }

    /// Is the upcoming form a `(: … …)` type annotation?
    fn is_annotation_ahead(&self) -> bool {
        matches!(self.toks.get(self.pos), Some(Tok::Open))
            && matches!(self.toks.get(self.pos + 1), Some(Tok::Atom(a)) if a == ":")
    }

    /// Parse a value at a `Solid` position, transparently unwrapping any `(: <value> Type>)` annotations
    /// (top-level `(: … Solid)`, and defensively any nested one), then the bare `(Ctor …)` node. Bounds the
    /// recursion depth (MAX_DEPTH) so an adversarial deeply-nested input Errs cleanly instead of overflowing.
    fn parse_solid_value(&mut self) -> Result<Solid, ParseError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(ParseError(format!(
                "solid nests deeper than the limit ({MAX_DEPTH}) — refusing to recurse further"
            )));
        }
        let result = if self.is_annotation_ahead() {
            self.expect_open()?; // (
            let _colon = self.expect_atom()?; // :
            let inner = self.parse_solid_value()?; // the annotated value
            let _ty = self.expect_atom()?; // Type name (e.g. Solid)
            self.expect_close()?; // )
            Ok(inner)
        } else {
            self.parse_solid_node()
        };
        self.depth -= 1;
        result
    }

    /// Parse a bare `(Ctor arg…)` Solidr constructor (the EXACT model's variants — R4). The Cadenza model
    /// is now the exact `Solidr` (Rational coords), so the render form is `Cuber`/`Spherer`/`Cylinderr`/
    /// `Unionr`/`Differencer`/`Intersectionr`/`Translater`/`Scaler` with `n/d` Rational numbers. There is no
    /// `Rotater` (a general rotation has no exact Rational form — see exact.cdz). A `Cuber(w,d,h)` /
    /// `Cylinderr(height, r)` is FULL size (matching solid.cdz's Cube); the mesh backend's centred
    /// `Manifold::cube`/`cylinder` consume full size directly, so no halving is needed here.
    fn parse_solid_node(&mut self) -> Result<Solid, ParseError> {
        self.expect_open()?;
        let head = self.expect_atom()?;
        let node = match head.as_str() {
            "Emptyr" => {
                let _unit = self.expect_atom()?; // nullary variant renders as `(Emptyr unit)`
                Solid::Empty
            }
            "Cuber" => Solid::Cube(self.parse_vec3()?),
            "Spherer" => Solid::Sphere(self.parse_rational()?),
            "Cylinderr" => {
                let h = self.parse_rational()?;
                let r = self.parse_rational()?;
                Solid::Cylinder(h, r)
            }
            "Unionr" => {
                let a = self.parse_solid_value()?;
                let b = self.parse_solid_value()?;
                Solid::Union(Box::new(a), Box::new(b))
            }
            "Differencer" => {
                let a = self.parse_solid_value()?;
                let b = self.parse_solid_value()?;
                Solid::Difference(Box::new(a), Box::new(b))
            }
            "Intersectionr" => {
                let a = self.parse_solid_value()?;
                let b = self.parse_solid_value()?;
                Solid::Intersection(Box::new(a), Box::new(b))
            }
            "Translater" => {
                let v = self.parse_vec3()?;
                let s = self.parse_solid_value()?;
                Solid::Translate(v, Box::new(s))
            }
            "Scaler" => {
                let v = self.parse_vec3()?;
                let s = self.parse_solid_value()?;
                Solid::Scale(v, Box::new(s))
            }
            other => return Err(ParseError(format!("unknown Solidr constructor `{other}`"))),
        };
        self.expect_close()?;
        Ok(node)
    }

    /// Parse a `Vec3r` value: a `(: (tuple x y z) Vec3r)` annotation OR a bare `(tuple x y z)`. Components
    /// are Rational `n/d`.
    fn parse_vec3(&mut self) -> Result<Vec3, ParseError> {
        if self.is_annotation_ahead() {
            self.expect_open()?; // (
            let _colon = self.expect_atom()?; // :
            let v = self.parse_vec3()?; // inner (tuple …)
            let _ty = self.expect_atom()?; // Vec3r
            self.expect_close()?; // )
            return Ok(v);
        }
        self.expect_open()?;
        let head = self.expect_atom()?;
        if head != "tuple" {
            return Err(ParseError(format!(
                "expected a Vec3r `(tuple …)`, found `{head}`"
            )));
        }
        let x = self.parse_rational()?;
        let y = self.parse_rational()?;
        let z = self.parse_rational()?;
        self.expect_close()?;
        Ok(Vec3::new(x, y, z))
    }

    /// Parse a RATIONAL number leaf `n/d` (the exact model renders coordinates as normalized fractions,
    /// e.g. `50/1`, `127/20`) and evaluate it to the `f64` the mesh kernel uses — division at the leaf
    /// keeps the MODEL exact while the geometry backend works in float. A bare integer `n` (no `/`) or the
    /// float forms (`N.0`/`NaN`) are accepted defensively so a mixed/legacy render still parses.
    fn parse_rational(&mut self) -> Result<f64, ParseError> {
        let a = self.expect_atom()?;
        if a == "NaN" {
            return Ok(f64::NAN);
        }
        if let Some((num, den)) = a.split_once('/') {
            let n: f64 = num
                .parse()
                .map_err(|_| ParseError(format!("bad rational numerator `{num}` in `{a}`")))?;
            let d: f64 = den
                .parse()
                .map_err(|_| ParseError(format!("bad rational denominator `{den}` in `{a}`")))?;
            if d == 0.0 {
                return Err(ParseError(format!("rational `{a}` has a zero denominator")));
            }
            return Ok(n / d);
        }
        // no `/` — a bare integer or a float (N.0); parse directly.
        a.parse::<f64>()
            .map_err(|_| ParseError(format!("expected a rational `n/d`, found `{a}`")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The EXACT text cdz-run renders for a Solidr (captured from the live compiler), covering every variant.
    // Rational leaves `n/d`; no Rotater (no exact rotation).
    const ALL_VARIANTS: &str = "(: (Intersectionr (Differencer (Unionr (Cuber (: (tuple 2/1 2/1 2/1) Vec3r)) (Spherer 3/2)) (Cylinderr 3/1 1/2)) (Scaler (: (tuple 2/1 2/1 2/1) Vec3r) (Translater (: (tuple 1/1 0/1 0/1) Vec3r) (Emptyr unit)))) Solidr)";

    #[test]
    fn parses_a_single_cube() {
        let s = parse_solid("(: (Cuber (: (tuple 2/1 2/1 2/1) Vec3r)) Solidr)").unwrap();
        assert_eq!(s, Solid::Cube(Vec3::new(2.0, 2.0, 2.0)));
        assert_eq!(s.leaf_count(), 1);
        assert_eq!(s.node_count(), 1);
    }

    #[test]
    fn parses_a_sphere_and_cylinder() {
        assert_eq!(
            parse_solid("(: (Spherer 3/2) Solidr)").unwrap(),
            Solid::Sphere(1.5)
        );
        assert_eq!(
            parse_solid("(: (Cylinderr 3/1 1/2) Solidr)").unwrap(),
            Solid::Cylinder(3.0, 0.5)
        );
    }

    #[test]
    fn parses_empty_with_its_unit_payload() {
        assert_eq!(
            parse_solid("(: (Emptyr unit) Solidr)").unwrap(),
            Solid::Empty
        );
        assert_eq!(
            parse_solid("(: (Emptyr unit) Solidr)")
                .unwrap()
                .leaf_count(),
            0
        );
    }

    #[test]
    fn parses_a_union_of_two_leaves() {
        let s =
            parse_solid("(: (Unionr (Cuber (: (tuple 1/1 1/1 1/1) Vec3r)) (Spherer 2/1)) Solidr)")
                .unwrap();
        assert_eq!(
            s,
            Solid::Union(
                Box::new(Solid::Cube(Vec3::new(1.0, 1.0, 1.0))),
                Box::new(Solid::Sphere(2.0)),
            )
        );
        assert_eq!(s.leaf_count(), 2);
        assert_eq!(s.node_count(), 3);
    }

    #[test]
    fn parses_a_transform_wrapping_a_leaf() {
        let s = parse_solid("(: (Translater (: (tuple 5/1 0/1 0/1) Vec3r) (Spherer 1/1)) Solidr)")
            .unwrap();
        assert_eq!(
            s,
            Solid::Translate(Vec3::new(5.0, 0.0, 0.0), Box::new(Solid::Sphere(1.0)))
        );
        assert_eq!(s.leaf_count(), 1);
        assert_eq!(s.node_count(), 2);
    }

    #[test]
    fn parses_every_variant_and_counts_match() {
        let s = parse_solid(ALL_VARIANTS).unwrap();
        // Cuber, Spherer, Cylinderr (3 leaves) + Emptyr (0) = 3 primitive leaves.
        assert_eq!(s.leaf_count(), 3);
        // Intersectionr, Differencer, Unionr, Cuber, Spherer, Cylinderr, Scaler, Translater, Emptyr = 9.
        assert_eq!(s.node_count(), 9);
    }

    #[test]
    fn a_rational_leaf_evaluates_to_its_exact_quotient() {
        // 127/20 = 6.35 exactly (the 1/4-inch hole radius in mm) — parse the fraction, not a float.
        let s = parse_solid("(: (Spherer 127/20) Solidr)").unwrap();
        assert_eq!(s, Solid::Sphere(6.35));
    }

    #[test]
    fn negative_and_integral_rationals_parse() {
        let s = parse_solid("(: (Translater (: (tuple -3/2 0/1 0/1) Vec3r) (Spherer 2/1)) Solidr)")
            .unwrap();
        match s {
            Solid::Translate(v, _) => {
                assert_eq!(v.x, -1.5); // -3/2
                assert_eq!(v.y, 0.0);
                assert_eq!(v.z, 0.0);
            }
            _ => panic!("expected a Translater"),
        }
    }

    #[test]
    fn parses_a_bare_unannotated_form_too() {
        // Defensive: nested solids render bare; the parser must accept a bare top-level node as well.
        let s = parse_solid("(Spherer 3/1)").unwrap();
        assert_eq!(s, Solid::Sphere(3.0));
    }

    #[test]
    fn rejects_an_unknown_constructor() {
        assert!(parse_solid("(: (Torusr 1/1 2/1) Solidr)").is_err());
    }

    #[test]
    fn rejects_trailing_junk() {
        assert!(parse_solid("(: (Spherer 1/1) Solidr) extra").is_err());
    }

    #[test]
    fn rejects_a_truncated_form() {
        assert!(parse_solid("(: (Unionr (Spherer 1/1)").is_err());
    }

    #[test]
    fn rejects_a_zero_denominator_rational() {
        assert!(parse_solid("(: (Spherer 1/0) Solidr)").is_err());
    }

    #[test]
    fn parser_is_total_on_malformed_input_never_panics() {
        // A driver parses UNTRUSTED rendered text; every malformed shape must be a clean Err, never a panic.
        // (This pins the totality validated by the vertical's robustness probe.)
        for bad in [
            "",                                                   // empty
            "(",                                                  // lone open
            ")",                                                  // lone close
            "()",                                                 // empty list (no ctor head)
            "(Cuber)",                                            // missing payload
            "(Cuber (: (tuple 1/1) Vec3r))",                      // Vec3r with too few components
            "(Cuber (: (tuple 1/1 2/1 3/1 4/1) Vec3r))",          // Vec3r with too many
            "(: (Cuber (: (tuple a b c) Vec3r)) Solidr)",         // non-numeric leaves
            "(Spherer 1/1 2/1 3/1)",                              // wrong arity
            "(Unionr (Spherer 1/1) (Spherer 2/1) (Spherer 3/1))", // Unionr with 3 args
            "not-an-sexpr",                                       // a bare atom
            "((((((((((",                                         // runaway opens
            "(Spherer 1/0)",                                      // zero denominator
        ] {
            assert!(
                parse_solid(bad).is_err(),
                "malformed input should Err (not panic / not Ok): {bad:?}"
            );
        }
    }

    #[test]
    fn parser_handles_a_reasonably_deep_chain() {
        // A moderately deep Union chain (100) — well beyond any real CAD model's nesting — parses + counts.
        let depth = 100;
        let mut s = String::from("(Spherer 1/1)");
        for _ in 0..depth {
            s = format!("(Unionr (Spherer 1/1) {s})");
        }
        let parsed = parse_solid(&format!("(: {s} Solidr)")).expect("deep chain parses");
        // depth unions + (depth + 1) spheres = 2*depth + 1 nodes.
        assert_eq!(parsed.node_count(), 2 * depth + 1);
        assert_eq!(parsed.leaf_count(), depth + 1);
    }

    #[test]
    fn an_adversarially_deep_chain_errs_cleanly_instead_of_overflowing() {
        // Past MAX_DEPTH the parser must return an Err (the depth guard), NOT recurse into a stack overflow.
        // Build a chain well beyond the cap; the parse should stop with the depth-limit error.
        let mut s = String::from("(Spherer 1/1)");
        for _ in 0..(MAX_DEPTH + 50) {
            s = format!("(Unionr (Spherer 1/1) {s})");
        }
        let err = parse_solid(&format!("(: {s} Solidr)")).unwrap_err();
        assert!(
            err.0.contains("nests deeper than the limit"),
            "expected the depth-limit error, got: {}",
            err.0
        );
    }
}
