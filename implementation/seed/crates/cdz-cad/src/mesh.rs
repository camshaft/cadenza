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

use crate::{Solid, Vec3};
use manifold_csg::manifold::Manifold;

/// The tessellation quality for curved primitives (sphere/cylinder) — the number of segments around the
/// circumference. Higher = smoother + more triangles. A reasonable interactive default; `mesh_with_segments`
/// overrides it.
pub const DEFAULT_SEGMENTS: i32 = 32;

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
        Solid::Scale(Vec3 { x, y, z }, of) => {
            to_manifold_with_segments(of, segments).scale(*x, *y, *z)
        }
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
        let s = parse_solid("(: (Cuber (: (tuple 2.0 2.0 2.0) Vec3r)) Solidr)").unwrap();
        let m = mesh(&s);
        assert_eq!(m.triangle_count(), 12);
        assert!(!m.is_empty());
        assert_eq!(m.positions.len() % 3, 0);
        assert_eq!(m.indices.len() % 3, 0);
    }

    #[test]
    fn empty_meshes_to_nothing() {
        let s = parse_solid("(: (Emptyr unit) Solidr)").unwrap();
        let m = mesh(&s);
        assert!(m.is_empty());
        assert_eq!(m.triangle_count(), 0);
    }

    #[test]
    fn a_difference_produces_a_watertight_hole() {
        // cube minus a sphere → more triangles than the bare cube (the boolean carved a cavity).
        let s = parse_solid(
            "(: (Differencer (Cuber (: (tuple 4.0 4.0 4.0) Vec3r)) (Spherer 1.5)) Solidr)",
        )
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
        let plate = "(: (Differencer (Differencer (Cuber (: (tuple 10.0 4.0 1.0) Vec3r)) (Translater (: (tuple 2.5 2.0 0.0) Vec3r) (Cylinderr 1.0 0.5))) (Translater (: (tuple 7.5 2.0 0.0) Vec3r) (Cylinderr 1.0 0.5))) Solidr)";
        let m = mesh(&parse_solid(plate).unwrap());
        assert!(!m.is_empty());
        assert!(m.triangle_count() > 12);
    }

    #[test]
    fn a_transform_chain_meshes() {
        // scale ∘ translate ∘ cube — the transform arms all evaluate (the exact model has no Rotate).
        let s = parse_solid(
            "(: (Scaler (: (tuple 2/1 2/1 2/1) Vec3r) (Translater (: (tuple 1/1 0/1 0/1) Vec3r) (Cuber (: (tuple 1/1 1/1 1/1) Vec3r)))) Solidr)",
        )
        .unwrap();
        let m = mesh(&s);
        assert_eq!(m.triangle_count(), 12); // a transformed cube is still 12 triangles
    }

    #[test]
    fn segment_count_controls_sphere_tessellation() {
        let s = parse_solid("(: (Spherer 1.0) Solidr)").unwrap();
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
        let one = mesh(&parse_solid("(: (Spherer 1.0) Solidr)").unwrap());
        let two = mesh(
            &parse_solid(
                "(: (Unionr (Spherer 1.0) (Translater (: (tuple 10.0 0.0 0.0) Vec3r) (Spherer 1.0))) Solidr)",
            )
            .unwrap(),
        );
        assert!(two.triangle_count() > one.triangle_count());
    }
}
