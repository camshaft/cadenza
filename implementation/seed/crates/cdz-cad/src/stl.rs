//! stl.rs — serialize a [`crate::Mesh`] to binary STL (GH #400, increment G2c).
//!
//! STL is the universal printer/viewer interchange: a flat list of triangles, each with a face normal and
//! three vertices. It carries no topology or units (manifold's own guidance prefers 3MF/glTF for a
//! watertight round-trip), but it is the zero-dependency, everyone-reads-it format — the right first export
//! target for `cdz cad` (a later sub-slice can add 3MF/glTF). Binary STL, not ASCII: compact and exact.
//!
//! Binary STL layout (little-endian):
//!   * 80-byte header (free-form; we stamp a short identifier)
//!   * u32 triangle count
//!   * per triangle: 12×f32 (normal xyz, then v1/v2/v3 xyz) + u16 "attribute byte count" (0)
//!
//! The normal is computed from the winding (manifold emits outward-facing, CCW triangles), so a viewer
//! that trusts the stored normal and one that recomputes from winding agree.

use crate::Mesh;

/// Serialize `mesh` to a binary-STL byte buffer.
pub fn to_binary_stl(mesh: &Mesh) -> Vec<u8> {
    let tri_count = mesh.triangle_count();
    // 80 header + 4 count + 50 bytes/triangle (12 f32 = 48, + 2 attr).
    let mut out = Vec::with_capacity(84 + tri_count * 50);

    // 80-byte header — a fixed identifier, zero-padded. (Must NOT begin with "solid", which would make some
    // readers parse it as ASCII STL.)
    let mut header = [0u8; 80];
    let tag = b"cdz-cad binary STL (manifold)";
    header[..tag.len()].copy_from_slice(tag);
    out.extend_from_slice(&header);

    out.extend_from_slice(&(tri_count as u32).to_le_bytes());

    for t in 0..tri_count {
        let ia = mesh.indices[t * 3] as usize;
        let ib = mesh.indices[t * 3 + 1] as usize;
        let ic = mesh.indices[t * 3 + 2] as usize;
        let a = vertex(mesh, ia);
        let b = vertex(mesh, ib);
        let c = vertex(mesh, ic);
        let n = face_normal(a, b, c);
        for v in [n, a, b, c] {
            out.extend_from_slice(&v[0].to_le_bytes());
            out.extend_from_slice(&v[1].to_le_bytes());
            out.extend_from_slice(&v[2].to_le_bytes());
        }
        out.extend_from_slice(&0u16.to_le_bytes()); // attribute byte count
    }
    out
}

/// Serialize `mesh` to ASCII STL (a UTF-8 string). Human-readable and required by some legacy tools;
/// larger than binary STL (verbose text), so binary is the default and this is opt-in (`--ascii`). Same
/// geometry + face normals as [`to_binary_stl`], just the text encoding of the format.
pub fn to_ascii_stl(mesh: &Mesh) -> String {
    let name = "cdz_cad";
    let mut out = String::new();
    out.push_str(&format!("solid {name}\n"));
    for t in 0..mesh.triangle_count() {
        let a = vertex(mesh, mesh.indices[t * 3] as usize);
        let b = vertex(mesh, mesh.indices[t * 3 + 1] as usize);
        let c = vertex(mesh, mesh.indices[t * 3 + 2] as usize);
        let n = face_normal(a, b, c);
        out.push_str(&format!("  facet normal {} {} {}\n", n[0], n[1], n[2]));
        out.push_str("    outer loop\n");
        for v in [a, b, c] {
            out.push_str(&format!("      vertex {} {} {}\n", v[0], v[1], v[2]));
        }
        out.push_str("    endloop\n");
        out.push_str("  endfacet\n");
    }
    out.push_str(&format!("endsolid {name}\n"));
    out
}

/// The i-th vertex position `[x, y, z]` from the interleaved buffer.
fn vertex(mesh: &Mesh, i: usize) -> [f32; 3] {
    [
        mesh.positions[i * 3],
        mesh.positions[i * 3 + 1],
        mesh.positions[i * 3 + 2],
    ]
}

/// The unit face normal of triangle `(a, b, c)` via the normalized cross product `(b-a) × (c-a)`. A
/// degenerate (zero-area) triangle yields a zero normal rather than NaN — some meshes carry slivers, and a
/// zero normal is a benign value STL readers tolerate.
fn face_normal(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> [f32; 3] {
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let cross = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    let len = (cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2]).sqrt();
    if len == 0.0 {
        [0.0, 0.0, 0.0]
    } else {
        [cross[0] / len, cross[1] / len, cross[2] / len]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{mesh, parse_solid};

    /// Read the triangle count out of a binary-STL buffer (bytes 80..84).
    fn stl_tri_count(bytes: &[u8]) -> u32 {
        u32::from_le_bytes([bytes[80], bytes[81], bytes[82], bytes[83]])
    }

    #[test]
    fn a_cube_stl_has_the_right_size_and_count() {
        let m = mesh(&parse_solid("(: (Cuber (: (tuple 2.0 2.0 2.0) Vec3r)) Solidr)").unwrap());
        let stl = to_binary_stl(&m);
        // 12 triangles → header(80) + count(4) + 12×50 = 684 bytes.
        assert_eq!(stl.len(), 84 + 12 * 50);
        assert_eq!(stl_tri_count(&stl), 12);
        // the header must not start with "solid" (that marks ASCII STL to some readers).
        assert_ne!(&stl[..5], b"solid");
    }

    #[test]
    fn the_byte_length_always_matches_the_triangle_count() {
        // property: len == 84 + 50 * tris, for any mesh.
        let m = mesh(
            &parse_solid(
                "(: (Differencer (Cuber (: (tuple 4.0 4.0 4.0) Vec3r)) (Spherer 1.5)) Solidr)",
            )
            .unwrap(),
        );
        let stl = to_binary_stl(&m);
        assert_eq!(stl.len(), 84 + 50 * m.triangle_count());
        assert_eq!(stl_tri_count(&stl) as usize, m.triangle_count());
    }

    #[test]
    fn an_empty_mesh_writes_a_valid_zero_triangle_stl() {
        let m = mesh(&parse_solid("(: (Emptyr unit) Solidr)").unwrap());
        let stl = to_binary_stl(&m);
        assert_eq!(stl.len(), 84); // header + count, no triangles
        assert_eq!(stl_tri_count(&stl), 0);
    }

    #[test]
    fn normals_are_unit_length_for_a_cube() {
        let m = mesh(&parse_solid("(: (Cuber (: (tuple 2.0 2.0 2.0) Vec3r)) Solidr)").unwrap());
        let stl = to_binary_stl(&m);
        // first triangle's normal is at bytes 84..96 (3 f32).
        let nx = f32::from_le_bytes([stl[84], stl[85], stl[86], stl[87]]);
        let ny = f32::from_le_bytes([stl[88], stl[89], stl[90], stl[91]]);
        let nz = f32::from_le_bytes([stl[92], stl[93], stl[94], stl[95]]);
        let len = (nx * nx + ny * ny + nz * nz).sqrt();
        assert!(
            (len - 1.0).abs() < 1e-5,
            "a cube face normal is unit length"
        );
    }

    #[test]
    fn ascii_stl_is_well_formed_with_one_facet_per_triangle() {
        let m = mesh(&parse_solid("(: (Cuber (: (tuple 2.0 2.0 2.0) Vec3r)) Solidr)").unwrap());
        let s = to_ascii_stl(&m);
        assert!(s.starts_with("solid cdz_cad"), "opens with a solid header");
        assert!(
            s.trim_end().ends_with("endsolid cdz_cad"),
            "closes the solid"
        );
        // one `facet normal` per triangle (a cube = 12), and a matching endfacet count.
        assert_eq!(s.matches("facet normal").count(), 12);
        assert_eq!(s.matches("endfacet").count(), 12);
        // each facet has exactly 3 vertices → 36 vertex lines.
        assert_eq!(s.matches("vertex ").count(), 36);
    }

    #[test]
    fn ascii_stl_of_empty_has_no_facets() {
        let m = mesh(&parse_solid("(: (Emptyr unit) Solidr)").unwrap());
        let s = to_ascii_stl(&m);
        assert_eq!(s.matches("facet normal").count(), 0);
        assert!(s.contains("solid cdz_cad") && s.contains("endsolid cdz_cad"));
    }
}
