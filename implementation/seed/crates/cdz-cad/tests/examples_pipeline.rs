//! End-to-end pipeline tests over the REAL `implementation/cad` library examples (GH #400).
//!
//! Each string here is the EXACT canonical s-expr a worked example from `implementation/cad/src/examples.cdz`
//! renders to when run (captured from `cdz run`). These pin the WHOLE native driver chain —
//! `parse_solid` → `mesh` → `bounds` — against the real models a user writes, so a regression in EITHER the
//! Cadenza library's output OR the driver is caught. Bounds are geometrically derived (a plate's box is its
//! base cube; a washer/tube's is its outer cylinder), so they double as a correctness oracle.
//!
//! If the library's render form drifts (a constructor changes, the printer changes), these strings go stale
//! and the parse fails loudly — a signal to re-capture, exactly what an integration pin is for.

use cdz_cad::{bounds, mesh, parse_solid};

fn approx(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-4
}

/// `plate(4, 2, 1, 0.3)` — a 4×2×1 plate with two Ø0.6 bolt holes. Box = the base cube (holes are internal
/// cutouts, so they don't extend the extent): centred 4×2×1.
const PLATE: &str = "(: (Differencer (Differencer (Cuber (: (tuple 4.0 2.0 1.0) Vec3r)) (Translater (: (tuple 1.0 1.0 0.0) Vec3r) (Cylinderr 1.0 0.3))) (Translater (: (tuple 3.0 1.0 0.0) Vec3r) (Cylinderr 1.0 0.3))) Solidr)";

/// `washer(1, 2, 1)` — a thickness-1 ring, outer radius 2, bore radius 1. Box = the outer cylinder: 4×4×1.
const WASHER: &str = "(: (Differencer (Cylinderr 1.0 2.0) (Cylinderr 1.0 1.0)) Solidr)";

/// `tube(3, 2, 1)` — a length-3 pipe, outer radius 2, bore 1. Box = the outer cylinder: 4×4×3.
const TUBE: &str = "(: (Differencer (Cylinderr 3.0 2.0) (Cylinderr 3.0 1.0)) Solidr)";

#[test]
fn plate_meshes_and_bounds_match_the_base_cube() {
    let s = parse_solid(PLATE).expect("plate parses");
    let m = mesh(&s);
    assert!(!m.is_empty(), "the plate has geometry");
    let b = bounds(&s).expect("the plate has bounds");
    let d = b.dimensions();
    assert!(
        approx(d[0], 4.0) && approx(d[1], 2.0) && approx(d[2], 1.0),
        "plate box is its base cube 4×2×1, got {d:?}"
    );
}

#[test]
fn washer_bounds_match_the_outer_cylinder() {
    let s = parse_solid(WASHER).expect("washer parses");
    assert!(!mesh(&s).is_empty(), "the washer has geometry");
    let b = bounds(&s).expect("the washer has bounds");
    let d = b.dimensions();
    // outer radius 2 → diameter 4 in x/y; thickness 1 in z.
    assert!(
        approx(d[0], 4.0) && approx(d[1], 4.0) && approx(d[2], 1.0),
        "washer box is Ø4 × 1 thick, got {d:?}"
    );
}

#[test]
fn tube_bounds_match_the_outer_cylinder() {
    let s = parse_solid(TUBE).expect("tube parses");
    assert!(!mesh(&s).is_empty(), "the tube has geometry");
    let b = bounds(&s).expect("the tube has bounds");
    let d = b.dimensions();
    // outer radius 2 → Ø4; length 3 in z.
    assert!(
        approx(d[0], 4.0) && approx(d[1], 4.0) && approx(d[2], 3.0),
        "tube box is Ø4 × 3 long, got {d:?}"
    );
}

#[test]
fn a_washer_is_a_watertight_hollow_shell_not_empty() {
    // the bore is fully through, but the ring itself is solid — a difference that does NOT cancel to empty.
    let s = parse_solid(WASHER).unwrap();
    let m = mesh(&s);
    // a hollow ring has both an outer and inner wall → more triangles than a bare cylinder's ~sides.
    assert!(
        m.triangle_count() > 20,
        "a washer shell has real surface area"
    );
}

#[test]
fn every_example_centers_on_the_origin() {
    // all three models are built from origin-centred primitives + symmetric transforms, so their box center
    // is the origin (a sanity pin: the transform arithmetic didn't shift anything unexpectedly).
    for src in [PLATE, WASHER, TUBE] {
        let b = bounds(&parse_solid(src).unwrap()).unwrap();
        let c = b.center();
        assert!(
            approx(c[0], 0.0) && approx(c[1], 0.0) && approx(c[2], 0.0),
            "example centered at origin, got {c:?} for {src}"
        );
    }
}

// ── The manifold `Empty` identity, cross-checked against the library's `simplify` algebra ──────────
// The library's `simplify` absorbs `Empty` (union Empty x = x; intersection _ Empty = Empty; …). The
// driver must MESH consistently: `Empty` is the boolean identity in manifold too. These pin that the mesh
// backend agrees with the library's algebra, so a model and its `simplify`d form mesh to the same thing —
// a driver that mis-handled `Empty` (e.g. treated it as a unit cube) would fail here.

fn tri_count(src: &str) -> usize {
    mesh(&parse_solid(src).unwrap()).triangle_count()
}

#[test]
fn union_with_empty_meshes_like_the_operand_alone() {
    let cube = "(: (Cuber (: (tuple 2.0 2.0 2.0) Vec3r)) Solidr)";
    let union_left = "(: (Unionr (Emptyr unit) (Cuber (: (tuple 2.0 2.0 2.0) Vec3r))) Solidr)";
    let union_right = "(: (Unionr (Cuber (: (tuple 2.0 2.0 2.0) Vec3r)) (Emptyr unit)) Solidr)";
    assert_eq!(
        tri_count(union_left),
        tri_count(cube),
        "union(Empty, cube) ≡ cube"
    );
    assert_eq!(
        tri_count(union_right),
        tri_count(cube),
        "union(cube, Empty) ≡ cube"
    );
}

#[test]
fn difference_of_empty_tool_meshes_like_the_base() {
    // subtracting nothing leaves the base unchanged.
    let cube = "(: (Cuber (: (tuple 2.0 2.0 2.0) Vec3r)) Solidr)";
    let diff = "(: (Differencer (Cuber (: (tuple 2.0 2.0 2.0) Vec3r)) (Emptyr unit)) Solidr)";
    assert_eq!(
        tri_count(diff),
        tri_count(cube),
        "difference(cube, Empty) ≡ cube"
    );
}

#[test]
fn intersection_with_empty_meshes_to_nothing() {
    let inter = "(: (Intersectionr (Cuber (: (tuple 2.0 2.0 2.0) Vec3r)) (Emptyr unit)) Solidr)";
    assert_eq!(tri_count(inter), 0, "intersection(x, Empty) is empty");
}
