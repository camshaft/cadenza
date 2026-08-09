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

use crate::{bounds, mesh, parse_solid};

fn approx(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-4
}

/// `plate(4, 2, 1, 0.3)` — a 4×2×1 plate with two Ø0.6 bolt holes. Box = the base cube (holes are internal
/// cutouts, so they don't extend the extent): centred 4×2×1.
const PLATE: &str = "(: (Difference (Difference (Cube (: (tuple 4.0 2.0 1.0) Vec3)) (Translate (: (tuple 1.0 1.0 0.0) Vec3) (Cylinder 1.0 0.3))) (Translate (: (tuple 3.0 1.0 0.0) Vec3) (Cylinder 1.0 0.3))) Solid)";

/// `washer(1, 2, 1)` — a thickness-1 ring, outer radius 2, bore radius 1. Box = the outer cylinder: 4×4×1.
const WASHER: &str = "(: (Difference (Cylinder 1.0 2.0) (Cylinder 1.0 1.0)) Solid)";

/// `tube(3, 2, 1)` — a length-3 pipe, outer radius 2, bore 1. Box = the outer cylinder: 4×4×3.
const TUBE: &str = "(: (Difference (Cylinder 3.0 2.0) (Cylinder 3.0 1.0)) Solid)";

/// `mixed-block()` from the UNITS-TYPED library (`examples-typed.cdz`): a box authored `box-len(meters(1),
/// inches(12), inches(6))` — 1 m wide, 12-inch deep, 6-inch high. The units-everywhere path: each dimension
/// is a length QUANTITY (a bare number is a type error at `box-len`), authored in mixed units and converted
/// to EXACT Rational METERS (the model's internal unit) before rendering — 12 inch = 381/1250 m, 6 inch =
/// 381/2500 m EXACTLY (no float rounding). So the model renders the same `n/d`-Rational grammar the driver
/// already parses, and this pins that a UNITS-AUTHORED model flows end-to-end (library → render → driver →
/// mesh) — the payoff that makes units-everywhere real at the render edge, not just unit-tested in Cadenza.
const MIXED_BLOCK: &str = "(: (Cube (: (tuple 1/1 381/1250 381/2500) Vec3)) Solid)";

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
fn units_typed_mixed_block_meshes_and_bounds_are_exact_meters() {
    // The units-everywhere end-to-end pin: a model authored in MIXED units (1 m + 12 inch + 6 inch) renders
    // to exact Rational METERS and flows through the native driver. Box = the full-size cube: 1 × 0.3048 ×
    // 0.1524 m (12 inch = 381/1250 = 0.3048 m; 6 inch = 381/2500 = 0.1524 m — exact, no float rounding).
    let s = parse_solid(MIXED_BLOCK).expect("the units-typed mixed block parses");
    let m = mesh(&s);
    assert!(!m.is_empty(), "the mixed-unit block has geometry");
    assert_eq!(m.triangle_count(), 12, "a box is 12 triangles");
    let b = bounds(&s).expect("the mixed block has bounds");
    let d = b.dimensions();
    assert!(
        approx(d[0], 1.0) && approx(d[1], 0.3048) && approx(d[2], 0.1524),
        "mixed-block box is 1 m × 12in(0.3048 m) × 6in(0.1524 m), got {d:?}"
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
    let cube = "(: (Cube (: (tuple 2.0 2.0 2.0) Vec3)) Solid)";
    let union_left = "(: (Union (Empty unit) (Cube (: (tuple 2.0 2.0 2.0) Vec3))) Solid)";
    let union_right = "(: (Union (Cube (: (tuple 2.0 2.0 2.0) Vec3)) (Empty unit)) Solid)";
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
    let cube = "(: (Cube (: (tuple 2.0 2.0 2.0) Vec3)) Solid)";
    let diff = "(: (Difference (Cube (: (tuple 2.0 2.0 2.0) Vec3)) (Empty unit)) Solid)";
    assert_eq!(
        tri_count(diff),
        tri_count(cube),
        "difference(cube, Empty) ≡ cube"
    );
}

#[test]
fn intersection_with_empty_meshes_to_nothing() {
    let inter = "(: (Intersection (Cube (: (tuple 2.0 2.0 2.0) Vec3)) (Empty unit)) Solid)";
    assert_eq!(tri_count(inter), 0, "intersection(x, Empty) is empty");
}

// ── The P5 host-render mirror: SolidR/Vec3R type-name atoms (GH #400) ──────────────────────────────
// The authored CAD model is GENERIC (`Solid(a)`/`Vec3(a)`), but the compiler cannot yet emit a host
// value-form walker for a GENERIC recursive sum instantiated at a type — so a program returns a MONOMORPHIC
// `SolidR`/`Vec3R` mirror (via `exact.cdz`'s `lower : Solid(Rational) → SolidR`), which renders with type-name
// atoms `SolidR`/`Vec3R` instead of `Solid`/`Vec3`. The driver parses by CONSTRUCTOR name and DISCARDS the
// trailing type-name atom (see `parse_solid_value`: it `expect_atom()`s the type name without checking it), so
// a SolidR-rendered model parses + meshes IDENTICALLY to the Solid form. This pin locks that tolerance in: a
// future change that started VALIDATING the type-name atom (rejecting anything not literally `Solid`/`Vec3`)
// would silently break the /cad preload path — this test would catch it. The render below is the EXACT string
// `lower(Difference(Cube(v3r 4 4 4), Sphere 5/2))` produces (captured from `cdz run`).
const SOLIDR_RENDER: &str =
    "(: (Difference (Cube (: (tuple 4/1 4/1 4/1) Vec3R)) (Sphere 5/2)) SolidR)";

#[test]
fn a_solidr_rendered_model_parses_and_meshes_like_its_solid_twin() {
    // the P5 monomorphic render (SolidR/Vec3R type names) must parse + mesh exactly as the equivalent Solid
    // form — the type-name atom is cosmetic to the driver.
    let solidr = parse_solid(SOLIDR_RENDER)
        .expect("a SolidR-rendered model parses (type-name atom ignored)");
    let twin =
        parse_solid("(: (Difference (Cube (: (tuple 4/1 4/1 4/1) Vec3)) (Sphere 5/2)) Solid)")
            .expect("the Solid twin parses");
    let m = mesh(&solidr);
    assert!(
        !m.is_empty(),
        "the SolidR model has geometry (a 4³ cube minus a Ø2.5 sphere)"
    );
    assert_eq!(
        mesh(&solidr).triangle_count(),
        mesh(&twin).triangle_count(),
        "SolidR and Solid renders of the same model mesh identically — the type name is discarded"
    );
    // and the bounds match the base cube (4×4×4; the sphere is an internal cut that doesn't grow the box).
    let d = bounds(&solidr)
        .expect("the SolidR model has bounds")
        .dimensions();
    assert!(
        approx(d[0], 4.0) && approx(d[1], 4.0) && approx(d[2], 4.0),
        "SolidR model box is its 4×4×4 cube, got {d:?}"
    );
}

// ── P-D: profile/extrude/revolve/spline pipeline pins (parse → mesh → bounds on the real render forms) ──
// These pin the WHOLE native chain for the P-D geometry (examples-profiles.cdz's worked models), on the
// EXACT s-expr the exact-model renders (captured from `cdz run`), with geometrically-derived bounds as an
// oracle — a regression in the library's render OR the driver's parse/mesh/bounds is caught.

/// `plinth()` — a 40×20 Rect profile extruded 10 tall → box 40×20×10 (a prism).
const PLINTH: &str = "(: (ExtrudeLinear (Rect (: (tuple 40/1 20/1) Vec2R)) 10/1) SolidR)";
/// `bead()` — a 4×2 Rect revolved 360° about the y-axis → the conservative envelope box (4, 2, 4).
const BEAD: &str = "(: (Revolve (Rect (: (tuple 4/1 2/1) Vec2R)) 360/1) SolidR)";
/// A cubic-Bézier SPLINE outline (arch: base (0,0)→(8,0), cubic top back to (0,0)) extruded 2 thick.
const ARCH: &str = "(: (ExtrudeLinear (PathProfile (: (list (MoveToAbs (: (tuple 0/1 0/1) Vec2R)) (LineToAbs (: (tuple 8/1 0/1) Vec2R)) (CubicToAbs (: (tuple 0/1 0/1) Vec2R) (: (tuple 8/1 10/1) Vec2R) (: (tuple 0/1 10/1) Vec2R))) PathR)) 2/1) SolidR)";

#[test]
fn plinth_extrude_meshes_and_bounds_are_the_prism() {
    let s = parse_solid(PLINTH).expect("plinth parses");
    let m = mesh(&s);
    assert!(!m.is_empty(), "the extruded plinth has geometry");
    assert_eq!(
        m.triangle_count(),
        12,
        "an extruded rectangle is a 12-triangle prism"
    );
    let d = bounds(&s).expect("plinth has bounds").dimensions();
    assert!(
        approx(d[0], 40.0) && approx(d[1], 20.0) && approx(d[2], 10.0),
        "plinth box is its 40×20×10 prism, got {d:?}"
    );
}

#[test]
fn bead_revolve_meshes_a_finite_ring() {
    let s = parse_solid(BEAD).expect("bead parses");
    assert!(!mesh(&s).is_empty(), "the revolved bead has geometry");
    // a full 360° revolve of an origin-centred rect sweeps a finite solid; its bounds are finite + well-formed.
    let d = bounds(&s).expect("bead has bounds").dimensions();
    assert!(
        d[0].is_finite() && d[1].is_finite() && d[2].is_finite() && d[0] > 0.0 && d[2] > 0.0,
        "revolved bead has a finite, positive-volume box, got {d:?}"
    );
}

#[test]
fn spline_arch_path_profile_meshes_a_curved_wall() {
    // the cubic-Bézier arch samples to a polygon → an extruded curved fin. Real geometry, and (since the
    // curved wall samples to many segments) well over a bare box's 12 triangles.
    let s = parse_solid(ARCH).expect("spline arch parses");
    let m = mesh(&s);
    assert!(!m.is_empty(), "the extruded spline arch has geometry");
    assert!(
        m.triangle_count() > 12,
        "a sampled cubic-Bézier wall has many triangles, got {}",
        m.triangle_count()
    );
}
