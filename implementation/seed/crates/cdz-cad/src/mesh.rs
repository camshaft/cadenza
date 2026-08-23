//! mesh.rs — walk a parsed [`Solid`] CSG tree into a manifold mesh (GH #400, increment G2b).
//!
//! This is the geometry half of the native driver: the [`crate::Solid`] tree (parsed from a Cadenza
//! program's rendered value) is evaluated node-by-node into a `manifold_csg::manifold::Manifold`, whose
//! booleans produce a guaranteed-watertight mesh. Each variant maps 1:1 onto a manifold call — the CSG tree
//! IS the operation tree manifold evaluates. Segment count (the tessellation of curved primitives) is a
//! render parameter; a sensible default is exposed and overridable.
//!
//! The mesh is returned as a plain [`Mesh`] (interleaved f32 vertex positions + u32 triangle indices) — the
//! neutral form a 3MF/glTF/STL writer (a later sub-slice) serializes and a preview uploads to the GPU.

use crate::{PathSeg, Profile, Solid, Vec3};
use manifold_csg::cross_section::CrossSection;
use manifold_csg::manifold::Manifold;

/// The tessellation quality for curved primitives (sphere/cylinder) — the number of segments around the
/// circumference. Higher = smoother + more triangles. A reasonable interactive default; `mesh_with_segments`
/// overrides it.
pub const DEFAULT_SEGMENTS: i32 = 32;

/// The floor on a tessellation segment count: fewer than 3 sides can't close a curved loop. A `Detail`
/// override (or any threaded count) below this is clamped up rather than producing a degenerate mesh. Mirrors
/// the browser driver's `MIN_SEGMENTS` and the CLI's `--segments` minimum.
pub const MIN_SEGMENTS: i32 = 3;

/// A triangle mesh in the neutral form every export target wants: vertex POSITIONS (x, y, z per vertex, so
/// `positions.len()` is a multiple of 3) and triangle INDICES (three vertex indices per triangle, so
/// `indices.len()` is a multiple of 3). Produced from a manifold `MeshGL`; consumed by the 3MF/glTF/STL
/// writers and the browser preview.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Mesh {
    pub positions: Vec<f32>,
    pub indices: Vec<u32>,
}

impl Mesh {
    /// The number of triangles (`indices.len() / 3`).
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// The number of vertices (`positions.len() / 3`).
    pub fn vertex_count(&self) -> usize {
        self.positions.len() / 3
    }

    /// Whether the mesh has no geometry (an `Empty` solid, or a boolean that cancelled to nothing).
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }
}

/// Evaluate a [`Solid`] into a manifold at the default tessellation.
pub fn to_manifold(solid: &Solid) -> Manifold {
    to_manifold_with_segments(solid, DEFAULT_SEGMENTS)
}

/// Evaluate a [`Solid`] into a manifold at a given tessellation (`segments` = circumference divisions for
/// curved primitives). The recursive tree-walk — each node is one manifold operation.
pub fn to_manifold_with_segments(solid: &Solid, segments: i32) -> Manifold {
    match solid {
        Solid::Empty => Manifold::empty(),
        // manifold's cube is centred when the `center` flag is set — matches the library's origin-centred
        // primitives (the model's `cube w d h` is an axis-aligned box centred at the origin, DESIGN §1).
        Solid::Cube(Vec3 { x, y, z }) => Manifold::cube(*x, *y, *z, true),
        Solid::Sphere(r) => Manifold::sphere(*r, segments),
        // The library's `Cylinder(height, radius)` is a constant-radius cylinder → manifold's
        // `cylinder(height, radius_low, radius_high, …)` with both radii equal, centred on the origin.
        Solid::Cylinder(h, r) => Manifold::cylinder(*h, *r, *r, segments, true),
        Solid::Union(a, b) => {
            to_manifold_with_segments(a, segments).union(&to_manifold_with_segments(b, segments))
        }
        Solid::Difference(a, b) => to_manifold_with_segments(a, segments)
            .difference(&to_manifold_with_segments(b, segments)),
        Solid::Intersection(a, b) => to_manifold_with_segments(a, segments)
            .intersection(&to_manifold_with_segments(b, segments)),
        Solid::Translate(Vec3 { x, y, z }, of) => {
            to_manifold_with_segments(of, segments).translate(*x, *y, *z)
        }
        // manifold's rotate takes DEGREES per axis — matching the library's `Rotate(deg, …)` convention.
        Solid::Rotate(Vec3 { x, y, z }, of) => {
            to_manifold_with_segments(of, segments).rotate(*x, *y, *z)
        }
        // manifold's mirror reflects across the plane through the origin with the given normal — matching the
        // library's `Mirror(normal, …)`. (An axis-aligned normal like [1,0,0] is the exact-in-model case.)
        Solid::Mirror(Vec3 { x, y, z }, of) => {
            to_manifold_with_segments(of, segments).mirror([*x, *y, *z])
        }
        Solid::Scale(Vec3 { x, y, z }, of) => {
            to_manifold_with_segments(of, segments).scale(*x, *y, *z)
        }
        // Lift a 2-D profile straight up +z by `height` — manifold extrudes the CrossSection; center it in z
        // so it matches the origin-centred primitives (extrude runs 0..height, then shift down by height/2).
        Solid::ExtrudeLinear(p, height) => profile_to_cross_section(p, segments)
            .extrude(*height)
            .translate(0.0, 0.0, -*height / 2.0),
        // Sweep a 2-D profile about the y-axis by `degrees` — manifold's revolve (circular_segments for the
        // sweep tessellation). A full 360° sweep passes the profile's circular resolution.
        Solid::Revolve(p, degrees) => {
            Manifold::revolve(&profile_to_cross_section(p, segments), segments, *degrees)
        }
        // An OpenSCAD-`$fn`-style resolution override: mesh the child with the node's LOCAL segment count
        // instead of the inherited one, clamped to a closable loop. A deeper `Detail` overrides again
        // (dynamic scoping — the innermost enclosing `Detail` wins), and unwrapped geometry outside any
        // `Detail` keeps the ambient `segments`. A mesh hint only — the child's geometry is unchanged.
        Solid::Detail(n, of) => to_manifold_with_segments(of, (*n).max(MIN_SEGMENTS)),
    }
}

/// Build a manifold 2-D `CrossSection` from a [`Profile`] — the input an extrude/revolve lifts. `Rect`/
/// `Circle` map onto manifold's centred `square`/`circle`; a `Path` is SAMPLED to a polygon (line segments
/// are exact vertices; a cubic Bézier is sampled at `segments` points) then built as a simple polygon.
fn profile_to_cross_section(p: &Profile, segments: i32) -> CrossSection {
    match p {
        Profile::Rect(w, h) => CrossSection::square(*w, *h, true),
        Profile::Circle(r) => CrossSection::circle(*r, segments),
        Profile::Path(segs) => CrossSection::from_simple_polygon(&sample_path(segs, segments)),
    }
}

/// Sample a path's segments into a flat polygon point list (`[x, y]` each), walking the cursor. A
/// Line/Move contributes its endpoint; a cubic Bézier is sampled at `segments` interior points via the
/// standard Bernstein form. Relative segments offset the current cursor. The starting `MoveTo` seeds the
/// cursor (its point is included so the outline closes back to it).
fn sample_path(segs: &[PathSeg], segments: i32) -> Vec<[f64; 2]> {
    let mut pts: Vec<[f64; 2]> = Vec::new();
    let mut cur = [0.0f64, 0.0];
    let abs = |cur: [f64; 2], d: [f64; 2]| [cur[0] + d[0], cur[1] + d[1]];
    for seg in segs {
        match seg {
            PathSeg::MoveToAbs(p) => {
                cur = *p;
                pts.push(cur);
            }
            PathSeg::MoveToRel(d) => {
                cur = abs(cur, *d);
                pts.push(cur);
            }
            PathSeg::LineToAbs(p) => {
                cur = *p;
                pts.push(cur);
            }
            PathSeg::LineToRel(d) => {
                cur = abs(cur, *d);
                pts.push(cur);
            }
            PathSeg::CubicToAbs(e, c0, c1) => {
                sample_cubic(&mut pts, cur, *c0, *c1, *e, segments);
                cur = *e;
            }
            PathSeg::CubicToRel(e, c0, c1) => {
                let (e, c0, c1) = (abs(cur, *e), abs(cur, *c0), abs(cur, *c1));
                sample_cubic(&mut pts, cur, c0, c1, e, segments);
                cur = e;
            }
        }
    }
    pts
}

/// Push `n` sample points of the cubic Bézier `p0→p3` (controls `p1`, `p2`) for `t` in (0, 1] — the start
/// point `p0` is assumed already emitted by the prior segment, so we sample the interior + endpoint.
fn sample_cubic(
    pts: &mut Vec<[f64; 2]>,
    p0: [f64; 2],
    p1: [f64; 2],
    p2: [f64; 2],
    p3: [f64; 2],
    n: i32,
) {
    let n = n.max(1);
    for i in 1..=n {
        let t = i as f64 / n as f64;
        let u = 1.0 - t;
        // Bernstein: u³p0 + 3u²t·p1 + 3ut²·p2 + t³p3 (per component).
        let b0 = u * u * u;
        let b1 = 3.0 * u * u * t;
        let b2 = 3.0 * u * t * t;
        let b3 = t * t * t;
        pts.push([
            b0 * p0[0] + b1 * p1[0] + b2 * p2[0] + b3 * p3[0],
            b0 * p0[1] + b1 * p1[1] + b2 * p2[1] + b3 * p3[1],
        ]);
    }
}

/// Mesh a [`Solid`] into the neutral [`Mesh`] form at the default tessellation.
pub fn mesh(solid: &Solid) -> Mesh {
    mesh_with_segments(solid, DEFAULT_SEGMENTS)
}

/// Mesh a [`Solid`] at a given tessellation.
pub fn mesh_with_segments(solid: &Solid, segments: i32) -> Mesh {
    let m = to_manifold_with_segments(solid, segments);
    let gl = m.to_meshgl();
    // MeshGL packs vertex PROPERTIES as [x, y, z, (extra props…)] per vertex; with no extra properties the
    // stride is 3, so `vert_properties` IS the interleaved position array. `tri_verts` is the flat u32
    // index list (3 per triangle).
    Mesh {
        positions: gl.vert_properties().to_vec(),
        indices: gl.tri_verts().to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_solid;

    #[test]
    fn a_cube_meshes_to_twelve_triangles() {
        // an axis-aligned box has 6 faces × 2 triangles = 12.
        let s = parse_solid("(: (Cube (: (tuple 2.0 2.0 2.0) Vec3)) Solid)").unwrap();
        let m = mesh(&s);
        assert_eq!(m.triangle_count(), 12);
        assert!(!m.is_empty());
        assert_eq!(m.positions.len() % 3, 0);
        assert_eq!(m.indices.len() % 3, 0);
    }

    #[test]
    fn empty_meshes_to_nothing() {
        let s = parse_solid("(: (Empty unit) Solid)").unwrap();
        let m = mesh(&s);
        assert!(m.is_empty());
        assert_eq!(m.triangle_count(), 0);
    }

    #[test]
    fn a_negative_dimension_cube_meshes_to_empty() {
        // Cross-surface consistency guard: the exact model (exact.cdz) normalizes a negative-dimension box to a
        // well-formed ABSOLUTE extent, and manifold documents that any negative (or all-zero) dimension yields
        // an EMPTY manifold — so a negative-size Cube meshes to NOTHING here (never garbage/degenerate geometry
        // that could crash a downstream STL/glTF writer). Pins that the native driver agrees with manifold's
        // documented negative-dimension behavior. (A model would `simplify-r`/normalize upstream; this is the
        // driver's own safety net.)
        let s = parse_solid("(: (Cube (: (tuple -2.0 2.0 2.0) Vec3)) Solid)").unwrap();
        let m = mesh(&s);
        assert!(
            m.is_empty(),
            "a negative-dimension cube must mesh to empty, not degenerate geometry"
        );
        assert_eq!(m.triangle_count(), 0);
    }

    #[test]
    fn a_difference_produces_a_watertight_hole() {
        // cube minus a sphere → more triangles than the bare cube (the boolean carved a cavity).
        let s =
            parse_solid("(: (Difference (Cube (: (tuple 4.0 4.0 4.0) Vec3)) (Sphere 1.5)) Solid)")
                .unwrap();
        let m = mesh(&s);
        assert!(
            m.triangle_count() > 12,
            "the difference should add geometry"
        );
        assert!(!m.is_empty());
    }

    #[test]
    fn the_plate_example_meshes_non_empty() {
        // the DESIGN marquee: a 10×4×1 plate with two Ø1 bolt holes.
        let plate = "(: (Difference (Difference (Cube (: (tuple 10.0 4.0 1.0) Vec3)) (Translate (: (tuple 2.5 2.0 0.0) Vec3) (Cylinder 1.0 0.5))) (Translate (: (tuple 7.5 2.0 0.0) Vec3) (Cylinder 1.0 0.5))) Solid)";
        let m = mesh(&parse_solid(plate).unwrap());
        assert!(!m.is_empty());
        assert!(m.triangle_count() > 12);
    }

    #[test]
    fn a_transform_chain_meshes() {
        // scale ∘ translate ∘ cube — the transform arms all evaluate.
        let s = parse_solid(
            "(: (Scale (: (tuple 2/1 2/1 2/1) Vec3) (Translate (: (tuple 1/1 0/1 0/1) Vec3) (Cube (: (tuple 1/1 1/1 1/1) Vec3)))) Solid)",
        )
        .unwrap();
        let m = mesh(&s);
        assert_eq!(m.triangle_count(), 12); // a transformed cube is still 12 triangles
    }

    #[test]
    fn rotate_of_a_cube_still_meshes_a_cube() {
        // Rotate carries an exact Rational Euler-degree triple; the trig runs at the manifold leaf. A rotated
        // cube is still a watertight 12-triangle box (rotation preserves topology).
        let s = parse_solid(
            "(: (Rotate (: (tuple 0/1 0/1 45/1) Vec3) (Cube (: (tuple 2/1 2/1 2/1) Vec3))) Solid)",
        )
        .unwrap();
        let m = mesh(&s);
        assert_eq!(m.triangle_count(), 12);
        assert!(!m.is_empty());
    }

    #[test]
    fn mirror_of_a_cube_still_meshes_a_cube() {
        // Mirror reflects across the plane with the given normal; a mirrored cube is still a 12-triangle box.
        let s = parse_solid(
            "(: (Mirror (: (tuple 1/1 0/1 0/1) Vec3) (Translate (: (tuple 5/1 0/1 0/1) Vec3) (Cube (: (tuple 2/1 2/1 2/1) Vec3)))) Solid)",
        )
        .unwrap();
        let m = mesh(&s);
        assert_eq!(m.triangle_count(), 12);
        assert!(!m.is_empty());
    }

    #[test]
    fn six_fold_rotational_array_unions_into_more_geometry() {
        // The snowflake idiom: union a bar with a rotated copy → more geometry than one bar (the copies don't
        // fully overlap). Pins that Rotate + Union compose the way the 6-fold symmetry relies on.
        let one = mesh(
            &parse_solid(
                "(: (Translate (: (tuple 5/1 0/1 0/1) Vec3) (Cube (: (tuple 8/1 1/1 1/1) Vec3))) Solid)",
            )
            .unwrap(),
        );
        let two = mesh(
            &parse_solid(
                "(: (Union (Translate (: (tuple 5/1 0/1 0/1) Vec3) (Cube (: (tuple 8/1 1/1 1/1) Vec3))) (Rotate (: (tuple 0/1 0/1 60/1) Vec3) (Translate (: (tuple 5/1 0/1 0/1) Vec3) (Cube (: (tuple 8/1 1/1 1/1) Vec3))))) Solid)",
            )
            .unwrap(),
        );
        assert!(two.triangle_count() > one.triangle_count());
    }

    #[test]
    fn the_l_bracket_assembly_meshes_with_the_arm_standing_above_the_base() {
        // Mesh the /cad L-bracket ASSEMBLY example end-to-end (the exact rendered sexpr the browser gets) and
        // verify via the ACTUAL meshed geometry (manifold bounding_box — unaffected by the Cadenza-side bbox
        // fold): the base plate is a 40×30×4 slab on z∈[0,4] with a bolt hole; the arm is a 30×25×4 box rotated
        // +90° about x (so it rises in +z) and lifted onto the base top. So the assembly's meshed z-max must be
        // well ABOVE the base's 4mm thickness (the arm stands up), and the whole thing is a single watertight
        // solid. This is the mesh-verify v-guide-infra couldn't run (no playwright) — the native driver uses
        // the SAME manifold .rotate/.union ops as the browser, so a correct native mesh proves the shape.
        let base = "(Difference (Translate (: (tuple 0/1 0/1 2/1) Vec3) (Cube (: (tuple 40/1 30/1 4/1) Vec3))) (Translate (: (tuple 10/1 0/1 2/1) Vec3) (Cylinder 8/1 3/1)))";
        let arm = "(Translate (: (tuple 0/1 0/1 4/1) Vec3) (Rotate (: (tuple 90/1 0/1 0/1) Vec3) (Translate (: (tuple 0/1 0/1 2/1) Vec3) (Cube (: (tuple 30/1 25/1 4/1) Vec3)))))";
        let m = to_manifold(&parse_solid(&format!("(: (Union {base} {arm}) Solid)")).unwrap());
        let mesh = mesh(&parse_solid(&format!("(: (Union {base} {arm}) Solid)")).unwrap());
        assert!(
            !mesh.is_empty(),
            "the L-bracket assembly meshes to real geometry"
        );
        let bb = m
            .bounding_box()
            .expect("a non-empty assembly has a bounding box");
        // z-max must clear the base's 4mm thickness by a wide margin — the arm (25 tall) stands up on the base.
        assert!(
            bb.max()[2] > 10.0,
            "the arm should stand well above the base (meshed z-max = {}, base is only 4mm thick)",
            bb.max()[2]
        );
    }

    #[test]
    fn segment_count_controls_sphere_tessellation() {
        let s = parse_solid("(: (Sphere 1.0) Solid)").unwrap();
        let coarse = mesh_with_segments(&s, 8);
        let fine = mesh_with_segments(&s, 64);
        assert!(
            fine.triangle_count() > coarse.triangle_count(),
            "more segments → more triangles"
        );
    }

    #[test]
    fn union_of_disjoint_solids_keeps_both() {
        // two spheres far apart → the union keeps both shells (roughly double a single sphere's tris).
        let one = mesh(&parse_solid("(: (Sphere 1.0) Solid)").unwrap());
        let two = mesh(
            &parse_solid(
                "(: (Union (Sphere 1.0) (Translate (: (tuple 10.0 0.0 0.0) Vec3) (Sphere 1.0))) Solid)",
            )
            .unwrap(),
        );
        assert!(two.triangle_count() > one.triangle_count());
    }

    #[test]
    fn intersection_of_disjoint_solids_meshes_to_nothing() {
        // The mesh-side counterpart of the exact library's inverted-box invariant (exact.cdz's
        // `intersection-of-disjoint-solids-is-an-inverted-empty-box`): two NON-overlapping real solids
        // intersect to NO geometry. Two unit cubes 10 apart in x share no volume → the manifold boolean is
        // empty (0 triangles), NOT a degenerate sliver. This pins that the driver agrees with the model that
        // a disjoint intersection is empty — a driver that clamped an inverted box to a zero/positive sliver
        // (rather than emptying it) would fail here.
        let inter = mesh(
            &parse_solid(
                "(: (Intersection (Cube (: (tuple 2.0 2.0 2.0) Vec3)) (Translate (: (tuple 10.0 0.0 0.0) Vec3) (Cube (: (tuple 2.0 2.0 2.0) Vec3)))) Solid)",
            )
            .unwrap(),
        );
        assert_eq!(
            inter.triangle_count(),
            0,
            "intersection of two disjoint cubes is empty geometry"
        );
    }

    // ── Detail: the OpenSCAD-`$fn`-style tessellation-resolution override node ────────────────────────

    #[test]
    fn detail_overrides_the_inherited_segment_count() {
        // `(Detail n child)` meshes the child at n segments regardless of the ambient count threaded in: a
        // Detail-64 sphere is finer than a Detail-8 one even when BOTH are meshed at the same ambient default,
        // because the inner override wins.
        let coarse = mesh_with_segments(
            &parse_solid("(: (Detail 8 (Sphere 5/1)) SolidR)").unwrap(),
            32,
        );
        let fine = mesh_with_segments(
            &parse_solid("(: (Detail 64 (Sphere 5/1)) SolidR)").unwrap(),
            32,
        );
        assert!(
            fine.triangle_count() > coarse.triangle_count(),
            "the Detail override sets the child's tessellation, not the ambient count"
        );
    }

    #[test]
    fn detail_is_dynamically_scoped_innermost_wins() {
        // A nested Detail overrides an outer one for its subtree: Detail 8 (Detail 64 (Sphere)) meshes the
        // sphere at 64 (the innermost), identical to a bare Detail 64.
        let nested = mesh(&parse_solid("(: (Detail 8 (Detail 64 (Sphere 5/1))) SolidR)").unwrap());
        let inner = mesh(&parse_solid("(: (Detail 64 (Sphere 5/1)) SolidR)").unwrap());
        assert_eq!(
            nested.triangle_count(),
            inner.triangle_count(),
            "the innermost Detail wins (dynamic scoping)"
        );
    }

    #[test]
    fn detail_count_below_min_is_clamped_not_degenerate() {
        // A count under 3 can't close a curved loop; the mesh clamps up to MIN_SEGMENTS rather than emptying.
        // Detail 0 and Detail 3 mesh the same real geometry.
        let zero = mesh(&parse_solid("(: (Detail 0 (Sphere 5/1)) SolidR)").unwrap());
        let floor = mesh(&parse_solid("(: (Detail 3 (Sphere 5/1)) SolidR)").unwrap());
        assert!(!zero.is_empty(), "a clamped-up Detail meshes real geometry");
        assert_eq!(
            zero.triangle_count(),
            floor.triangle_count(),
            "Detail 0 clamps to the same mesh as Detail 3 (MIN_SEGMENTS)"
        );
    }

    #[test]
    fn detail_is_a_mesh_hint_and_does_not_change_a_polyhedral_child() {
        // Detail is a MESH HINT, not geometry: wrapping a cube (no curved leaves) in any Detail meshes the
        // SAME 12-triangle box — the tessellation count is irrelevant to a polyhedral child, so the shape is
        // unchanged. (For a curved child the shape is likewise the same solid, only tessellated finer.)
        let bare = mesh(&parse_solid("(: (Cube (: (tuple 2/1 2/1 2/1) Vec3)) SolidR)").unwrap());
        let detailed = mesh(
            &parse_solid("(: (Detail 128 (Cube (: (tuple 2/1 2/1 2/1) Vec3))) SolidR)").unwrap(),
        );
        assert_eq!(detailed.triangle_count(), bare.triangle_count());
        assert_eq!(detailed.triangle_count(), 12);
    }

    // ── P-D: extrude / revolve / path profiles (the exact.cdz Profile+Path render forms) ─────────────

    #[test]
    fn extrude_a_rect_profile_meshes() {
        // (ExtrudeLinear (Rect (: (tuple 4/1 2/1) Vec2R)) 6/1) — a 4×2 rectangle lifted to height 6 → a
        // 4×2×6 prism (a box = 12 triangles). Pins the extrude of a Rect profile meshes end-to-end.
        let s =
            parse_solid("(: (ExtrudeLinear (Rect (: (tuple 4/1 2/1) Vec2R)) 6/1) SolidR)").unwrap();
        let m = mesh(&s);
        assert!(!m.is_empty(), "an extruded rect has geometry");
        assert_eq!(
            m.triangle_count(),
            12,
            "an extruded rectangle is a 12-triangle box"
        );
    }

    #[test]
    fn extrude_a_circle_profile_meshes() {
        // (ExtrudeLinear (Circle 3/1) 5/1) — a disc r=3 lifted to height 5 → a cylinder-like solid with real
        // surface area (well over a box's 12 triangles).
        let s = parse_solid("(: (ExtrudeLinear (Circle 3/1) 5/1) SolidR)").unwrap();
        let m = mesh(&s);
        assert!(!m.is_empty(), "an extruded circle has geometry");
        assert!(
            m.triangle_count() > 12,
            "an extruded disc has curved-wall triangles"
        );
    }

    #[test]
    fn revolve_a_profile_sweeps_a_solid() {
        // (Revolve (Rect …) 360/1), offset in x so the sweep encloses volume — pins the revolve API works
        // end-to-end (a full sweep of a profile about the y-axis produces real geometry).
        let s = parse_solid(
            "(: (Translate (: (tuple 5/1 0/1 0/1) Vec3) (Revolve (Rect (: (tuple 2/1 4/1) Vec2R)) 360/1)) SolidR)",
        )
        .unwrap();
        assert!(!mesh(&s).is_empty(), "a revolved profile has geometry");
    }

    #[test]
    fn extrude_a_path_profile_polygon_meshes() {
        // (ExtrudeLinear (PathProfile (: (list (MoveToAbs …) (LineToAbs …)…) PathR)) 3/1) — a triangular
        // path outline (0,0)→(4,0)→(2,3) extruded to height 3. The path samples to a polygon (line segments
        // are exact vertices); the extrude meshes to real geometry — pins the PathProfile → polygon → extrude
        // chain end-to-end.
        let s = parse_solid(
            "(: (ExtrudeLinear (PathProfile (: (list (MoveToAbs (: (tuple 0/1 0/1) Vec2R)) (LineToAbs (: (tuple 4/1 0/1) Vec2R)) (LineToAbs (: (tuple 2/1 3/1) Vec2R))) PathR)) 3/1) SolidR)",
        )
        .unwrap();
        assert!(
            !mesh(&s).is_empty(),
            "an extruded path-profile triangle has geometry"
        );
    }

    #[test]
    fn relative_path_segments_sample_the_same_points_as_the_equivalent_absolute_path() {
        // sample_path threads the cursor through RELATIVE segments (`abs(cur, delta)`); a regression that
        // forgot to add `cur` would sample the raw deltas and land the polygon in the wrong place. Pin it by
        // building the SAME outline two ways — once with absolute segments, once with the equivalent relative
        // deltas — and asserting the sampled point lists are identical. Covers LineToRel + CubicToRel (the
        // `cubic-by` builder's driver-side twin), which no prior test exercised.
        let seg = 4;
        // Absolute: (0,0) → line (10,0) → cubic to (10,6) via controls (8,4),(12,4).
        let absolute = [
            PathSeg::MoveToAbs([0.0, 0.0]),
            PathSeg::LineToAbs([10.0, 0.0]),
            PathSeg::CubicToAbs([10.0, 6.0], [8.0, 4.0], [12.0, 4.0]),
        ];
        // Relative deltas producing the identical absolute points (cursor threaded: 0→10, then +0/+6 etc.).
        let relative = [
            PathSeg::MoveToAbs([0.0, 0.0]),
            PathSeg::LineToRel([10.0, 0.0]),
            PathSeg::CubicToRel([0.0, 6.0], [-2.0, 4.0], [2.0, 4.0]),
        ];
        let a = sample_path(&absolute, seg);
        let r = sample_path(&relative, seg);
        assert_eq!(
            a, r,
            "relative segments must thread the cursor → identical sampled polygon to the absolute form"
        );
        // And the shared endpoint really is the threaded absolute (10,6), not the raw delta (0,6).
        assert_eq!(
            *r.last().unwrap(),
            [10.0, 6.0],
            "the relative cubic's endpoint resolves to absolute (10,6) via the cursor"
        );
    }
}
