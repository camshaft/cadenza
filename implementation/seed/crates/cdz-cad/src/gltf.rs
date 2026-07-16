//! gltf.rs — serialize a [`crate::Mesh`] to binary glTF (`.glb`) (GH #400, increment G2/export).
//!
//! glTF is a design-PRIMARY export target (DESIGN §5: "3MF + glTF primary … STL as a printer convenience")
//! — a modern, watertight-preserving mesh format every web/3D viewer reads. The BINARY container (`.glb`) is
//! self-describing and hand-writable from the neutral [`Mesh`] (positions + indices) with ZERO extra
//! dependencies — unlike 3MF (a ZIP of XML, which would pull a zip + xml dep). So `.glb` is the natural
//! second writer after STL.
//!
//! GLB layout (little-endian), per the glTF 2.0 spec:
//!   * 12-byte header: magic `glTF` (0x46546C67), u32 version = 2, u32 total file length
//!   * chunk 0 (JSON): u32 byteLength, u32 type `JSON` (0x4E4F534A), then the glTF JSON, padded with SPACES
//!     to a 4-byte boundary
//!   * chunk 1 (BIN):  u32 byteLength, u32 type `BIN\0` (0x004E4942), then the binary buffer, padded with
//!     zero bytes to a 4-byte boundary
//!
//! The JSON describes ONE mesh with one primitive: an index accessor (SCALAR u32) and a POSITION accessor
//! (VEC3 f32, with the spec-required per-axis min/max). Both index and vertex data live in one buffer,
//! addressed by two bufferViews. Indices come first, then positions (each already 4-byte aligned).

use crate::Mesh;

/// glTF component type for `u32` indices (`UNSIGNED_INT`).
const COMPONENT_U32: u32 = 5125;
/// glTF component type for `f32` positions (`FLOAT`).
const COMPONENT_F32: u32 = 5126;
/// bufferView target `ELEMENT_ARRAY_BUFFER` (indices).
const TARGET_ELEMENT_ARRAY: u32 = 34963;
/// bufferView target `ARRAY_BUFFER` (vertex attributes).
const TARGET_ARRAY: u32 = 34962;

const GLB_MAGIC: u32 = 0x4654_6C67; // "glTF"
const CHUNK_JSON: u32 = 0x4E4F_534A; // "JSON"
const CHUNK_BIN: u32 = 0x004E_4942; // "BIN\0"

/// Serialize `mesh` to a binary-glTF (`.glb`) byte buffer.
pub fn to_glb(mesh: &Mesh) -> Vec<u8> {
    // ---- BIN buffer: indices (u32 LE) then positions (f32 LE) ----
    let mut bin: Vec<u8> = Vec::with_capacity(mesh.indices.len() * 4 + mesh.positions.len() * 4);
    let idx_offset = 0usize;
    let idx_len = mesh.indices.len() * 4;
    for &i in &mesh.indices {
        bin.extend_from_slice(&i.to_le_bytes());
    }
    // u32 indices keep 4-byte alignment, so positions start already aligned.
    let pos_offset = bin.len();
    let pos_len = mesh.positions.len() * 4;
    for &p in &mesh.positions {
        bin.extend_from_slice(&p.to_le_bytes());
    }
    pad_to_4(&mut bin, 0);

    let (min, max) = position_bounds(&mesh.positions);
    let vcount = mesh.vertex_count();
    let icount = mesh.indices.len();

    // ---- JSON chunk ----
    let json = build_json(
        icount,
        vcount,
        min,
        max,
        idx_offset,
        idx_len,
        pos_offset,
        pos_len,
        bin.len(),
    );
    let mut json_bytes = json.into_bytes();
    pad_to_4(&mut json_bytes, b' '); // JSON chunk padded with SPACES per spec

    // ---- assemble ----
    let total = 12 + 8 + json_bytes.len() + 8 + bin.len();
    let mut glb = Vec::with_capacity(total);
    glb.extend_from_slice(&GLB_MAGIC.to_le_bytes());
    glb.extend_from_slice(&2u32.to_le_bytes());
    glb.extend_from_slice(&(total as u32).to_le_bytes());
    // JSON chunk
    glb.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
    glb.extend_from_slice(&CHUNK_JSON.to_le_bytes());
    glb.extend_from_slice(&json_bytes);
    // BIN chunk
    glb.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    glb.extend_from_slice(&CHUNK_BIN.to_le_bytes());
    glb.extend_from_slice(&bin);
    glb
}

/// Pad `buf` up to a 4-byte boundary with `fill`.
fn pad_to_4(buf: &mut Vec<u8>, fill: u8) {
    while !buf.len().is_multiple_of(4) {
        buf.push(fill);
    }
}

/// Per-axis (min, max) over the interleaved xyz positions — required by the glTF POSITION accessor. An
/// empty mesh yields all-zero bounds (a valid degenerate box).
fn position_bounds(positions: &[f32]) -> ([f32; 3], [f32; 3]) {
    if positions.is_empty() {
        return ([0.0; 3], [0.0; 3]);
    }
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for v in positions.chunks_exact(3) {
        for a in 0..3 {
            min[a] = min[a].min(v[a]);
            max[a] = max[a].max(v[a]);
        }
    }
    (min, max)
}

/// Build the glTF 2.0 JSON document for a single indexed triangle mesh. Hand-formatted (no serde dep) —
/// the shape is fixed, so a format string is clearest + dependency-free.
#[allow(clippy::too_many_arguments)]
fn build_json(
    icount: usize,
    vcount: usize,
    min: [f32; 3],
    max: [f32; 3],
    idx_offset: usize,
    idx_len: usize,
    pos_offset: usize,
    pos_len: usize,
    buffer_len: usize,
) -> String {
    format!(
        concat!(
            r#"{{"asset":{{"version":"2.0","generator":"cdz-cad"}},"#,
            r#""scene":0,"scenes":[{{"nodes":[0]}}],"nodes":[{{"mesh":0}}],"#,
            r#""meshes":[{{"primitives":[{{"attributes":{{"POSITION":1}},"indices":0}}]}}],"#,
            r#""accessors":["#,
            r#"{{"bufferView":0,"componentType":{ict},"count":{ic},"type":"SCALAR"}},"#,
            r#"{{"bufferView":1,"componentType":{fct},"count":{vc},"type":"VEC3","#,
            r#""min":[{mnx},{mny},{mnz}],"max":[{mxx},{mxy},{mxz}]}}],"#,
            r#""bufferViews":["#,
            r#"{{"buffer":0,"byteOffset":{io},"byteLength":{il},"target":{tea}}},"#,
            r#"{{"buffer":0,"byteOffset":{po},"byteLength":{pl},"target":{ta}}}],"#,
            r#""buffers":[{{"byteLength":{bl}}}]}}"#,
        ),
        ict = COMPONENT_U32,
        ic = icount,
        fct = COMPONENT_F32,
        vc = vcount,
        mnx = f(min[0]),
        mny = f(min[1]),
        mnz = f(min[2]),
        mxx = f(max[0]),
        mxy = f(max[1]),
        mxz = f(max[2]),
        io = idx_offset,
        il = idx_len,
        tea = TARGET_ELEMENT_ARRAY,
        po = pos_offset,
        pl = pos_len,
        ta = TARGET_ARRAY,
        bl = buffer_len,
    )
}

/// Render an `f32` as valid JSON (finite → its decimal; non-finite coerced to 0 — glTF JSON has no NaN/Inf,
/// and a bound is metadata a viewer only uses for culling).
fn f(x: f32) -> f32 {
    if x.is_finite() {
        x
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{mesh, parse_solid};

    fn u32_at(b: &[u8], off: usize) -> u32 {
        u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
    }

    #[test]
    fn a_cube_glb_has_a_valid_header_and_chunks() {
        let m = mesh(&parse_solid("(: (Cube (: (tuple 2.0 2.0 2.0) Vec3)) Solid)").unwrap());
        let glb = to_glb(&m);
        assert_eq!(u32_at(&glb, 0), GLB_MAGIC, "magic is glTF");
        assert_eq!(u32_at(&glb, 4), 2, "version 2");
        assert_eq!(
            u32_at(&glb, 8) as usize,
            glb.len(),
            "header length == file length"
        );
        // JSON chunk header
        let json_len = u32_at(&glb, 12) as usize;
        assert_eq!(u32_at(&glb, 16), CHUNK_JSON);
        assert!(json_len.is_multiple_of(4), "JSON chunk is 4-byte aligned");
        // BIN chunk header follows the JSON chunk
        let bin_off = 20 + json_len;
        assert_eq!(u32_at(&glb, bin_off + 4), CHUNK_BIN);
        let bin_len = u32_at(&glb, bin_off) as usize;
        assert!(bin_len.is_multiple_of(4), "BIN chunk is 4-byte aligned");
        // total accounting
        assert_eq!(12 + 8 + json_len + 8 + bin_len, glb.len());
    }

    #[test]
    fn the_json_is_well_formed_and_declares_the_mesh() {
        let m = mesh(&parse_solid("(: (Sphere 1.0) Solid)").unwrap());
        let glb = to_glb(&m);
        let json_len = u32_at(&glb, 12) as usize;
        let json = std::str::from_utf8(&glb[20..20 + json_len]).unwrap();
        assert!(json.contains(r#""version":"2.0""#));
        assert!(json.contains(r#""POSITION":1"#));
        assert!(json.contains(r#""indices":0"#));
        assert!(json.contains(r#""componentType":5125"#)); // u32 indices
        assert!(json.contains(r#""componentType":5126"#)); // f32 positions
        assert!(json.contains(r#""min":["#) && json.contains(r#""max":["#));
    }

    #[test]
    fn bin_buffer_length_matches_indices_plus_positions() {
        let m = mesh(&parse_solid("(: (Cube (: (tuple 2.0 2.0 2.0) Vec3)) Solid)").unwrap());
        let glb = to_glb(&m);
        let json_len = u32_at(&glb, 12) as usize;
        let bin_off = 20 + json_len;
        let bin_len = u32_at(&glb, bin_off) as usize;
        // indices (u32) + positions (f32), padded to 4 — both already multiples of 4, so exact.
        assert_eq!(bin_len, m.indices.len() * 4 + m.positions.len() * 4);
    }

    #[test]
    fn an_empty_mesh_still_writes_a_valid_glb() {
        let m = mesh(&parse_solid("(: (Empty unit) Solid)").unwrap());
        let glb = to_glb(&m);
        assert_eq!(u32_at(&glb, 0), GLB_MAGIC);
        assert_eq!(u32_at(&glb, 8) as usize, glb.len());
        // zero triangles → zero-length bin buffer (still 4-aligned).
        let json_len = u32_at(&glb, 12) as usize;
        let bin_off = 20 + json_len;
        assert_eq!(u32_at(&glb, bin_off) as usize, 0);
    }
}
