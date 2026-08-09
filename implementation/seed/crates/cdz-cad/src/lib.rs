//! cdz-cad — the native mesh driver for Cadenza CAD (GH #400, increment G2).
//!
//! # What this crate does
//!
//! Two halves: (1) THIS module parses a Cadenza program's rendered `Solid` value (canonical s-expr text)
//! into a [`Solid`] CSG tree; (2) [`mesh`] walks that tree into a manifold mesh (`manifold-csg`). A CLI
//! sub-slice (`cdz cad`) wires "run the program → parse → mesh → write 3MF/glTF/STL" together.
//!
//! A Cadenza program built on `implementation/cad`'s EXACT `Solid` model describes a solid as a recursive
//! `Solid` value with `Rational` coordinates. When such a program's single export crosses the component
//! boundary, cdz-run renders the compound value to CANONICAL S-EXPRESSION TEXT (the B1 "render-tree-as-data"
//! seam). For example a plate = cube minus a sphere crosses as:
//!
//! ```text
//! (: (Difference (Cube (: (tuple 50/1 30/1 5/1) Vec3)) (Sphere 127/20)) Solid)
//! ```
//!
//! This module parses that text into a [`Solid`] tree the [`mesh`] backend walks into `manifold-csg`. The
//! render grammar this parser accepts (R4 — the EXACT model; the legacy Float64 `Solid` form is retired):
//!   * the WHOLE value carries an outer type annotation `(: <value> Solid)`; NESTED solids are bare
//!     `(Ctor …)` (no per-node annotation);
//!   * each `Vec3` is annotated `(: (tuple x y z) Vec3)`;
//!   * a sum constructor is `(Ctor arg…)` — `Union`/`Difference`/`Intersection` take two `Solid` args;
//!     `Translate`/`Scale`/`Rotate`/`Mirror` take a `Vec3` then a `Solid`; `Cube` takes a `Vec3` (FULL
//!     size); `Sphere` one Rational (radius); `Cylinder` two Rationals (FULL height, radius). `Rotate`
//!     carries an exact Rational Euler-degree triple (the trig runs at the f64 manifold leaf, like
//!     `Revolve`); `Mirror` a plane normal;
//!   * a NULLARY constructor renders WITH a `unit` payload — `Empty` is `(Empty unit)`;
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
pub mod threemf;

// In-crate pipeline tests over the real CAD library examples (relocated from `tests/examples_pipeline.rs`
// per the no-integration-tests directive — same coverage, compiled with the lib, no separate binary).
#[cfg(test)]
mod examples_pipeline_tests;

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
    /// Reflect across the plane through the origin with the given normal — the Cadenza `Mirror`.
    Mirror(Vec3, Box<Solid>),
    Scale(Vec3, Box<Solid>),
    /// Lift a 2-D profile straight up +z by a full height (a prism) — the Cadenza `ExtrudeLinear`.
    ExtrudeLinear(Profile, f64),
    /// Sweep a 2-D profile about the y-axis by `degrees` — the Cadenza `Revolve`.
    Revolve(Profile, f64),
}

/// A 2-D cross-section the driver lifts into a 3-D `Solid` via `ExtrudeLinear`/`Revolve` (the Cadenza
/// `Profile`). `Rect`/`Circle` map directly onto manifold's `CrossSection::square`/`circle`; `PathProfile`
/// samples its path segments to a polygon (`CrossSection::from_simple_polygon`).
#[derive(Clone, PartialEq, Debug)]
pub enum Profile {
    /// A rectangle of FULL `(w, h)`, centred at the origin.
    Rect(f64, f64),
    /// A disc of the given radius.
    Circle(f64),
    /// A region bounded by a path — an ordered list of segments (sampled to a polygon at mesh time).
    Path(Vec<PathSeg>),
}

/// A 2-D path segment (the Cadenza `PathSeg`) — absolute (`*Abs`) or relative (`*Rel`) to the current point.
/// A `Move` starts a subpath (no edge); a `Line` draws a straight edge; a `Cubic` is a cubic Bézier (end +
/// two control points) the driver samples. Coordinates are already `f64` (evaluated from the exact Rational
/// at the render leaf, like every other driver coordinate).
#[derive(Clone, PartialEq, Debug)]
pub enum PathSeg {
    MoveToAbs([f64; 2]),
    MoveToRel([f64; 2]),
    LineToAbs([f64; 2]),
    LineToRel([f64; 2]),
    /// `(end, start_control, end_control)`.
    CubicToAbs([f64; 2], [f64; 2], [f64; 2]),
    CubicToRel([f64; 2], [f64; 2], [f64; 2]),
}

impl Solid {
    /// The number of primitive leaves (`Cube`/`Sphere`/`Cylinder`) — the geometry the backend emits.
    /// Mirrors the library's `leaf-count`, so a parsed tree can be cross-checked against the program's own
    /// fold. `Empty` contributes none; booleans/transforms are structure.
    pub fn leaf_count(&self) -> usize {
        match self {
            Solid::Empty => 0,
            Solid::Cube(_)
            | Solid::Sphere(_)
            | Solid::Cylinder(_, _)
            | Solid::ExtrudeLinear(_, _)
            | Solid::Revolve(_, _) => 1,
            Solid::Union(a, b) | Solid::Difference(a, b) | Solid::Intersection(a, b) => {
                a.leaf_count() + b.leaf_count()
            }
            Solid::Translate(_, s)
            | Solid::Rotate(_, s)
            | Solid::Mirror(_, s)
            | Solid::Scale(_, s) => s.leaf_count(),
        }
    }

    /// The total node count (the node itself plus every descendant) — mirrors the library's `count-nodes`.
    pub fn node_count(&self) -> usize {
        match self {
            Solid::Empty
            | Solid::Cube(_)
            | Solid::Sphere(_)
            | Solid::Cylinder(_, _)
            | Solid::ExtrudeLinear(_, _)
            | Solid::Revolve(_, _) => 1,
            Solid::Union(a, b) | Solid::Difference(a, b) | Solid::Intersection(a, b) => {
                1 + a.node_count() + b.node_count()
            }
            Solid::Translate(_, s)
            | Solid::Rotate(_, s)
            | Solid::Mirror(_, s)
            | Solid::Scale(_, s) => 1 + s.node_count(),
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

    /// Is the next token an open paren (a nested form is ahead)? Used to loop over a `(list …)`'s elements.
    fn peek_is_open(&self) -> bool {
        matches!(self.toks.get(self.pos), Some(Tok::Open))
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

    /// Parse a bare `(Ctor arg…)` Solid constructor (the EXACT model's variants — R4). The Cadenza model
    /// is now the exact `Solid` (Rational coords), so the render form is `Cube`/`Sphere`/`Cylinder`/
    /// `Union`/`Difference`/`Intersection`/`Translate`/`Scale`/`Rotate`/`Mirror` with `n/d` Rational
    /// numbers. `Rotate` carries an exact Rational Euler-degree triple (trig at the manifold leaf, like
    /// `Revolve`); `Mirror` a plane normal — see exact.cdz. A `Cube(w,d,h)` /
    /// `Cylinder(height, r)` is FULL size (matching solid.cdz's Cube); the mesh backend's centred
    /// `Manifold::cube`/`cylinder` consume full size directly, so no halving is needed here.
    fn parse_solid_node(&mut self) -> Result<Solid, ParseError> {
        self.expect_open()?;
        let head = self.expect_atom()?;
        let node = match head.as_str() {
            "Empty" => {
                let _unit = self.expect_atom()?; // nullary variant renders as `(Empty unit)`
                Solid::Empty
            }
            "Cube" => Solid::Cube(self.parse_vec3()?),
            "Sphere" => Solid::Sphere(self.parse_rational()?),
            "Cylinder" => {
                let h = self.parse_rational()?;
                let r = self.parse_rational()?;
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
            "Scale" => {
                let v = self.parse_vec3()?;
                let s = self.parse_solid_value()?;
                Solid::Scale(v, Box::new(s))
            }
            "Rotate" => {
                // An exact Rational Euler-degree triple; the trig happens at the manifold leaf (see mesh.rs).
                let v = self.parse_vec3()?;
                let s = self.parse_solid_value()?;
                Solid::Rotate(v, Box::new(s))
            }
            "Mirror" => {
                let v = self.parse_vec3()?;
                let s = self.parse_solid_value()?;
                Solid::Mirror(v, Box::new(s))
            }
            "ExtrudeLinear" => {
                let p = self.parse_profile()?;
                let h = self.parse_rational()?;
                Solid::ExtrudeLinear(p, h)
            }
            "Revolve" => {
                let p = self.parse_profile()?;
                let deg = self.parse_rational()?;
                Solid::Revolve(p, deg)
            }
            other => return Err(ParseError(format!("unknown Solid constructor `{other}`"))),
        };
        self.expect_close()?;
        Ok(node)
    }

    /// Parse a `Vec3` value: a `(: (tuple x y z) Vec3)` annotation OR a bare `(tuple x y z)`. Components
    /// are Rational `n/d`.
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
        let x = self.parse_rational()?;
        let y = self.parse_rational()?;
        let z = self.parse_rational()?;
        self.expect_close()?;
        Ok(Vec3::new(x, y, z))
    }

    /// Parse a `Vec2` value: a `(: (tuple x y) Vec2R)` annotation OR a bare `(tuple x y)` (the type-name atom
    /// is discarded, like `parse_vec3`). Components are Rational `n/d`.
    fn parse_vec2(&mut self) -> Result<[f64; 2], ParseError> {
        if self.is_annotation_ahead() {
            self.expect_open()?; // (
            let _colon = self.expect_atom()?; // :
            let v = self.parse_vec2()?; // inner (tuple …)
            let _ty = self.expect_atom()?; // Vec2R
            self.expect_close()?; // )
            return Ok(v);
        }
        self.expect_open()?;
        let head = self.expect_atom()?;
        if head != "tuple" {
            return Err(ParseError(format!(
                "expected a Vec2 `(tuple …)`, found `{head}`"
            )));
        }
        let x = self.parse_rational()?;
        let y = self.parse_rational()?;
        self.expect_close()?;
        Ok([x, y])
    }

    /// Parse a `Profile` — `(Rect <Vec2>)`, `(Circle <r>)`, or `(PathProfile <Path>)`. The type-name atom of
    /// an outer annotation is discarded (like the Solid/Vec parsers).
    fn parse_profile(&mut self) -> Result<Profile, ParseError> {
        if self.is_annotation_ahead() {
            self.expect_open()?;
            let _colon = self.expect_atom()?;
            let p = self.parse_profile()?;
            let _ty = self.expect_atom()?;
            self.expect_close()?;
            return Ok(p);
        }
        self.expect_open()?;
        let head = self.expect_atom()?;
        let prof = match head.as_str() {
            "Rect" => {
                let v = self.parse_vec2()?;
                Profile::Rect(v[0], v[1])
            }
            "Circle" => Profile::Circle(self.parse_rational()?),
            "PathProfile" => Profile::Path(self.parse_path()?),
            other => return Err(ParseError(format!("unknown Profile constructor `{other}`"))),
        };
        self.expect_close()?;
        Ok(prof)
    }

    /// Parse a `Path` — `(: (list <seg…>) PathR)` (or a bare `(list <seg…>)`); the type-name atom is
    /// discarded. Each element is a `PathSeg` constructor.
    fn parse_path(&mut self) -> Result<Vec<PathSeg>, ParseError> {
        if self.is_annotation_ahead() {
            self.expect_open()?;
            let _colon = self.expect_atom()?;
            let segs = self.parse_path()?;
            let _ty = self.expect_atom()?;
            self.expect_close()?;
            return Ok(segs);
        }
        self.expect_open()?;
        let head = self.expect_atom()?;
        if head != "list" {
            return Err(ParseError(format!(
                "expected a Path `(list …)`, found `{head}`"
            )));
        }
        let mut segs = Vec::new();
        while self.peek_is_open() {
            segs.push(self.parse_path_seg()?);
        }
        self.expect_close()?;
        Ok(segs)
    }

    /// Parse one `PathSeg` — `(MoveToAbs <Vec2>)` / `LineToRel` / `(CubicToAbs <Vec2> <Vec2> <Vec2>)` etc.
    fn parse_path_seg(&mut self) -> Result<PathSeg, ParseError> {
        self.expect_open()?;
        let head = self.expect_atom()?;
        let seg = match head.as_str() {
            "MoveToAbs" => PathSeg::MoveToAbs(self.parse_vec2()?),
            "MoveToRel" => PathSeg::MoveToRel(self.parse_vec2()?),
            "LineToAbs" => PathSeg::LineToAbs(self.parse_vec2()?),
            "LineToRel" => PathSeg::LineToRel(self.parse_vec2()?),
            "CubicToAbs" => {
                let e = self.parse_vec2()?;
                let c0 = self.parse_vec2()?;
                let c1 = self.parse_vec2()?;
                PathSeg::CubicToAbs(e, c0, c1)
            }
            "CubicToRel" => {
                let e = self.parse_vec2()?;
                let c0 = self.parse_vec2()?;
                let c1 = self.parse_vec2()?;
                PathSeg::CubicToRel(e, c0, c1)
            }
            other => return Err(ParseError(format!("unknown PathSeg constructor `{other}`"))),
        };
        self.expect_close()?;
        Ok(seg)
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

    // A CURATED render-form s-expr (the `n/d`-rational text `cdz-run` emits for a Solid), hand-built to cover
    // every SolidR constructor `lower` (exact.cdz) can emit — all 13 Solid heads plus a PathProfile that
    // exercises all six PathSeg kinds — so this is a grammar-COMPLETENESS guard: if a new arm is added to the
    // model's `lower` without a matching driver parser arm (the render-blank class), this fails to parse.
    // Left subtree (9 nodes / 3 leaves): Intersection, Difference, Union, Cube, Sphere, Cylinder, Scale,
    // Translate, Empty. Right subtree adds Rotate→ExtrudeLinear(PathProfile) and Mirror→Revolve(Circle) under
    // a Union → +6 nodes / +2 leaves. Total 15 nodes / 5 leaves.
    const ALL_VARIANTS: &str = "(: (Union (Intersection (Difference (Union (Cube (: (tuple 2/1 2/1 2/1) Vec3)) (Sphere 3/2)) (Cylinder 3/1 1/2)) (Scale (: (tuple 2/1 2/1 2/1) Vec3) (Translate (: (tuple 1/1 0/1 0/1) Vec3) (Empty unit)))) (Union (Rotate (: (tuple 0/1 0/1 45/1) Vec3) (ExtrudeLinear (PathProfile (list (MoveToAbs (tuple 0/1 0/1)) (LineToAbs (tuple 4/1 0/1)) (LineToRel (tuple 0/1 2/1)) (MoveToRel (tuple 1/1 1/1)) (CubicToAbs (tuple 6/1 0/1) (tuple 3/1 5/1) (tuple 5/1 0/1)) (CubicToRel (tuple 1/1 0/1) (tuple 0/1 1/1) (tuple 1/1 1/1)))) 4/1)) (Mirror (: (tuple 1/1 0/1 0/1) Vec3) (Revolve (Circle 3/1) 360/1)))) Solid)";

    #[test]
    fn parses_a_single_cube() {
        let s = parse_solid("(: (Cube (: (tuple 2/1 2/1 2/1) Vec3)) Solid)").unwrap();
        assert_eq!(s, Solid::Cube(Vec3::new(2.0, 2.0, 2.0)));
        assert_eq!(s.leaf_count(), 1);
        assert_eq!(s.node_count(), 1);
    }

    #[test]
    fn parses_a_sphere_and_cylinder() {
        assert_eq!(
            parse_solid("(: (Sphere 3/2) Solid)").unwrap(),
            Solid::Sphere(1.5)
        );
        assert_eq!(
            parse_solid("(: (Cylinder 3/1 1/2) Solid)").unwrap(),
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
        let s = parse_solid("(: (Union (Cube (: (tuple 1/1 1/1 1/1) Vec3)) (Sphere 2/1)) Solid)")
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
            parse_solid("(: (Translate (: (tuple 5/1 0/1 0/1) Vec3) (Sphere 1/1)) Solid)").unwrap();
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
        // Leaves: Cube, Sphere, Cylinder + ExtrudeLinear + Revolve = 5 (Empty/booleans/transforms are structure).
        assert_eq!(s.leaf_count(), 5);
        // Nodes: 9 in the left subtree (Intersection, Difference, Union, Cube, Sphere, Cylinder, Scale,
        // Translate, Empty) + the outer Union + Rotate + ExtrudeLinear + Mirror + Revolve + inner Union = 15.
        assert_eq!(s.node_count(), 15);
    }

    #[test]
    fn a_rational_leaf_evaluates_to_its_exact_quotient() {
        // 127/20 = 6.35 exactly (the 1/4-inch hole radius in mm) — parse the fraction, not a float.
        let s = parse_solid("(: (Sphere 127/20) Solid)").unwrap();
        assert_eq!(s, Solid::Sphere(6.35));
    }

    #[test]
    fn negative_and_integral_rationals_parse() {
        let s = parse_solid("(: (Translate (: (tuple -3/2 0/1 0/1) Vec3) (Sphere 2/1)) Solid)")
            .unwrap();
        match s {
            Solid::Translate(v, _) => {
                assert_eq!(v.x, -1.5); // -3/2
                assert_eq!(v.y, 0.0);
                assert_eq!(v.z, 0.0);
            }
            _ => panic!("expected a Translate"),
        }
    }

    #[test]
    fn parses_a_bare_unannotated_form_too() {
        // Defensive: nested solids render bare; the parser must accept a bare top-level node as well.
        let s = parse_solid("(Sphere 3/1)").unwrap();
        assert_eq!(s, Solid::Sphere(3.0));
    }

    #[test]
    fn rejects_an_unknown_constructor() {
        assert!(parse_solid("(: (Torusr 1/1 2/1) Solid)").is_err());
    }

    #[test]
    fn rejects_trailing_junk() {
        assert!(parse_solid("(: (Sphere 1/1) Solid) extra").is_err());
    }

    #[test]
    fn rejects_a_truncated_form() {
        assert!(parse_solid("(: (Union (Sphere 1/1)").is_err());
    }

    #[test]
    fn rejects_a_zero_denominator_rational() {
        assert!(parse_solid("(: (Sphere 1/0) Solid)").is_err());
    }

    #[test]
    fn parser_is_total_on_malformed_input_never_panics() {
        // A driver parses UNTRUSTED rendered text; every malformed shape must be a clean Err, never a panic.
        // (This pins the totality validated by the vertical's robustness probe.)
        for bad in [
            "",                                               // empty
            "(",                                              // lone open
            ")",                                              // lone close
            "()",                                             // empty list (no ctor head)
            "(Cube)",                                         // missing payload
            "(Cube (: (tuple 1/1) Vec3))",                    // Vec3 with too few components
            "(Cube (: (tuple 1/1 2/1 3/1 4/1) Vec3))",        // Vec3 with too many
            "(: (Cube (: (tuple a b c) Vec3)) Solid)",        // non-numeric leaves
            "(Sphere 1/1 2/1 3/1)",                           // wrong arity
            "(Union (Sphere 1/1) (Sphere 2/1) (Sphere 3/1))", // Union with 3 args
            "not-an-sexpr",                                   // a bare atom
            "((((((((((",                                     // runaway opens
            "(Sphere 1/0)",                                   // zero denominator
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
        let mut s = String::from("(Sphere 1/1)");
        for _ in 0..depth {
            s = format!("(Union (Sphere 1/1) {s})");
        }
        let parsed = parse_solid(&format!("(: {s} Solid)")).expect("deep chain parses");
        // depth unions + (depth + 1) spheres = 2*depth + 1 nodes.
        assert_eq!(parsed.node_count(), 2 * depth + 1);
        assert_eq!(parsed.leaf_count(), depth + 1);
    }

    #[test]
    fn an_adversarially_deep_chain_errs_cleanly_instead_of_overflowing() {
        // Past MAX_DEPTH the parser must return an Err (the depth guard), NOT recurse into a stack overflow.
        // Build a chain well beyond the cap; the parse should stop with the depth-limit error.
        let mut s = String::from("(Sphere 1/1)");
        for _ in 0..(MAX_DEPTH + 50) {
            s = format!("(Union (Sphere 1/1) {s})");
        }
        let err = parse_solid(&format!("(: {s} Solid)")).unwrap_err();
        assert!(
            err.0.contains("nests deeper than the limit"),
            "expected the depth-limit error, got: {}",
            err.0
        );
    }
}
