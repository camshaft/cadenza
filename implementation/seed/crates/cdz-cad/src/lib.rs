//! cdz-cad — the native mesh driver for Cadenza CAD (GH #400, increment G2).
//!
//! # What this crate does
//!
//! Two halves: (1) THIS module parses a Cadenza program's rendered `Solid` value (canonical s-expr text)
//! into a [`Solid`] CSG tree; (2) [`mesh`] walks that tree into a manifold mesh (`manifold-csg`). A CLI
//! sub-slice (`cdz cad`) wires "run the program → parse → mesh → write 3MF/glTF/STL" together.
//!
//! A Cadenza program built on `implementation/cad`'s `Solid` library describes a solid as a recursive
//! `Solid` value. When such a program's single export crosses the component boundary, cdz-run renders the
//! compound value to CANONICAL S-EXPRESSION TEXT (the B1 "render-tree-as-data" seam). For example the
//! program returning `union (cube 2 2 2) (translate (v3 5 0 0) (sphere 1))` crosses as:
//!
//! ```text
//! (: (Union (Cube (: (tuple 2.0 2.0 2.0) Vec3)) (Translate (: (tuple 5.0 0.0 0.0) Vec3) (Sphere 1.0))) Solid)
//! ```
//!
//! This module parses that text into a [`Solid`] tree the mesh backend (a later sub-slice) walks into
//! `manifold-csg`. The render grammar this parser accepts, read off the live compiler output:
//!   * the WHOLE value carries an outer type annotation `(: <value> Solid)`; NESTED solids are bare
//!     `(Ctor …)` (no per-node annotation);
//!   * each `Vec3` is annotated `(: (tuple x y z) Vec3)`;
//!   * a sum constructor is `(Ctor arg…)` — `Union`/`Difference`/`Intersection` take two `Solid` args;
//!     `Translate`/`Rotate`/`Scale` take a `Vec3` then a `Solid`; `Cube` takes a `Vec3`; `Sphere` one float;
//!     `Cylinder` two floats;
//!   * a NULLARY constructor renders WITH a `unit` payload — `Empty` is `(Empty unit)`;
//!   * a float is a decimal with a fractional part (`2.0`, `1.5`, `-0.0`, `NaN`), matching cdz-run's
//!     `display_float`.
//!
//! The parser unwraps a `(: v T)` annotation transparently wherever a value is expected, so it also accepts
//! a fully-annotated form (belt-and-suspenders — it never depends on WHICH positions are annotated).

use std::fmt;

pub mod mesh;
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

struct Parser<'a> {
    toks: &'a [Tok],
    pos: usize,
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
    /// (top-level `(: … Solid)`, and defensively any nested one), then the bare `(Ctor …)` node.
    fn parse_solid_value(&mut self) -> Result<Solid, ParseError> {
        if self.is_annotation_ahead() {
            self.expect_open()?; // (
            let _colon = self.expect_atom()?; // :
            let inner = self.parse_solid_value()?; // the annotated value
            let _ty = self.expect_atom()?; // Type name (e.g. Solid)
            self.expect_close()?; // )
            return Ok(inner);
        }
        self.parse_solid_node()
    }

    /// Parse a bare `(Ctor arg…)` Solid constructor.
    fn parse_solid_node(&mut self) -> Result<Solid, ParseError> {
        self.expect_open()?;
        let head = self.expect_atom()?;
        let node = match head.as_str() {
            "Empty" => {
                let _unit = self.expect_atom()?; // nullary variant renders as `(Empty unit)`
                Solid::Empty
            }
            "Cube" => Solid::Cube(self.parse_vec3()?),
            "Sphere" => Solid::Sphere(self.parse_float()?),
            "Cylinder" => {
                let h = self.parse_float()?;
                let r = self.parse_float()?;
                Solid::Cylinder(h, r)
            }
            "Union" => {
                let a = self.parse_solid_value()?;
                let b = self.parse_solid_value()?;
                Solid::Union(Box::new(a), Box::new(b))
            }
            "Difference" => {
                let a = self.parse_solid_value()?;
                let b = self.parse_solid_value()?;
                Solid::Difference(Box::new(a), Box::new(b))
            }
            "Intersection" => {
                let a = self.parse_solid_value()?;
                let b = self.parse_solid_value()?;
                Solid::Intersection(Box::new(a), Box::new(b))
            }
            "Translate" => {
                let v = self.parse_vec3()?;
                let s = self.parse_solid_value()?;
                Solid::Translate(v, Box::new(s))
            }
            "Rotate" => {
                let v = self.parse_vec3()?;
                let s = self.parse_solid_value()?;
                Solid::Rotate(v, Box::new(s))
            }
            "Scale" => {
                let v = self.parse_vec3()?;
                let s = self.parse_solid_value()?;
                Solid::Scale(v, Box::new(s))
            }
            other => return Err(ParseError(format!("unknown Solid constructor `{other}`"))),
        };
        self.expect_close()?;
        Ok(node)
    }

    /// Parse a `Vec3` value: a `(: (tuple x y z) Vec3)` annotation OR a bare `(tuple x y z)`.
    fn parse_vec3(&mut self) -> Result<Vec3, ParseError> {
        if self.is_annotation_ahead() {
            self.expect_open()?; // (
            let _colon = self.expect_atom()?; // :
            let v = self.parse_vec3()?; // inner (tuple …)
            let _ty = self.expect_atom()?; // Vec3
            self.expect_close()?; // )
            return Ok(v);
        }
        self.expect_open()?;
        let head = self.expect_atom()?;
        if head != "tuple" {
            return Err(ParseError(format!(
                "expected a Vec3 `(tuple …)`, found `{head}`"
            )));
        }
        let x = self.parse_float()?;
        let y = self.parse_float()?;
        let z = self.parse_float()?;
        self.expect_close()?;
        Ok(Vec3::new(x, y, z))
    }

    fn parse_float(&mut self) -> Result<f64, ParseError> {
        let a = self.expect_atom()?;
        // cdz-run renders NaN as `NaN` and integral floats as `N.0`; parse both.
        if a == "NaN" {
            return Ok(f64::NAN);
        }
        a.parse::<f64>()
            .map_err(|_| ParseError(format!("expected a float, found `{a}`")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The EXACT text cdz-run renders for a Solid (captured from the live compiler), covering every variant.
    const ALL_VARIANTS: &str = "(: (Intersection (Difference (Union (Cube (: (tuple 2.0 2.0 2.0) Vec3)) (Sphere 1.5)) (Cylinder 3.0 0.5)) (Scale (: (tuple 2.0 2.0 2.0) Vec3) (Rotate (: (tuple 0.0 0.0 90.0) Vec3) (Translate (: (tuple 1.0 0.0 0.0) Vec3) (Empty unit))))) Solid)";

    #[test]
    fn parses_a_single_cube() {
        let s = parse_solid("(: (Cube (: (tuple 2.0 2.0 2.0) Vec3)) Solid)").unwrap();
        assert_eq!(s, Solid::Cube(Vec3::new(2.0, 2.0, 2.0)));
        assert_eq!(s.leaf_count(), 1);
        assert_eq!(s.node_count(), 1);
    }

    #[test]
    fn parses_a_sphere_and_cylinder() {
        assert_eq!(
            parse_solid("(: (Sphere 1.5) Solid)").unwrap(),
            Solid::Sphere(1.5)
        );
        assert_eq!(
            parse_solid("(: (Cylinder 3.0 0.5) Solid)").unwrap(),
            Solid::Cylinder(3.0, 0.5)
        );
    }

    #[test]
    fn parses_empty_with_its_unit_payload() {
        assert_eq!(parse_solid("(: (Empty unit) Solid)").unwrap(), Solid::Empty);
        assert_eq!(
            parse_solid("(: (Empty unit) Solid)").unwrap().leaf_count(),
            0
        );
    }

    #[test]
    fn parses_a_union_of_two_leaves() {
        let s = parse_solid("(: (Union (Cube (: (tuple 1.0 1.0 1.0) Vec3)) (Sphere 2.0)) Solid)")
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
        let s =
            parse_solid("(: (Translate (: (tuple 5.0 0.0 0.0) Vec3) (Sphere 1.0)) Solid)").unwrap();
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
        // Cube, Sphere, Cylinder (3 leaves) + Empty (0) = 3 primitive leaves.
        assert_eq!(s.leaf_count(), 3);
        // Intersection, Difference, Union, Cube, Sphere, Cylinder, Scale, Rotate, Translate, Empty = 10.
        assert_eq!(s.node_count(), 10);
    }

    #[test]
    fn negative_and_integral_floats_parse() {
        let s = parse_solid("(: (Translate (: (tuple -1.5 0.0 -0.0) Vec3) (Sphere 2.0)) Solid)")
            .unwrap();
        match s {
            Solid::Translate(v, _) => {
                assert_eq!(v.x, -1.5);
                assert_eq!(v.y, 0.0);
                assert!(v.z == 0.0); // -0.0 == 0.0
            }
            _ => panic!("expected a Translate"),
        }
    }

    #[test]
    fn parses_a_bare_unannotated_form_too() {
        // Defensive: nested solids render bare; the parser must accept a bare top-level node as well.
        let s = parse_solid("(Sphere 3.0)").unwrap();
        assert_eq!(s, Solid::Sphere(3.0));
    }

    #[test]
    fn rejects_an_unknown_constructor() {
        assert!(parse_solid("(: (Torus 1.0 2.0) Solid)").is_err());
    }

    #[test]
    fn rejects_trailing_junk() {
        assert!(parse_solid("(: (Sphere 1.0) Solid) extra").is_err());
    }

    #[test]
    fn rejects_a_truncated_form() {
        assert!(parse_solid("(: (Union (Sphere 1.0)").is_err());
    }
}
