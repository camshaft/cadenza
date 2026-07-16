/// stl.ts — serialize a browser CAD mesh (the flat MeshResult buffers from index.ts) to BINARY STL, the
/// browser twin of the native cdz-cad `stl.rs` writer (P3, req #5: "export to STL … in the browser").
///
/// This is the mesh→bytes SERIALIZATION half only. v-cad owns this (mesh → format bytes); v-guide-infra
/// owns the /cad download UI (the button + the file-save). The two meet at `meshToBinaryStl(mesh)`, which
/// hands back a `Uint8Array` a download handler saves as `<name>.stl`.
///
/// The byte layout MIRRORS the native binary STL (cdz-cad/src/stl.rs) so a model exported from either
/// surface is byte-compatible:
///   * 80-byte header (free-form; must NOT begin with "solid", or a reader treats it as ASCII STL)
///   * u32 little-endian triangle count
///   * per triangle: 12 × f32 LE (face normal xyz, then v1/v2/v3 xyz) + u16 "attribute byte count" (0)
/// The face normal is computed from the winding (manifold emits outward-facing CCW triangles), so a viewer
/// trusting the stored normal and one recomputing from winding agree.

type Vec3 = [number, number, number];

/// The flat mesh buffers a binary-STL write needs: XYZ vertex positions (length a multiple of 3) and
/// triangle indices into them (length a multiple of 3). This is the success shape of index.ts's
/// `MeshResult`; kept structural so the writer does not depend on the parser module.
export interface StlMesh {
  positions: Float32Array;
  indices: Uint32Array;
}

const HEADER_TAG = "cdz-cad binary STL (manifold)";

/// A vertex (its XYZ) by index into the flat positions array.
function vertexAt(mesh: StlMesh, i: number): Vec3 {
  const b = i * 3;
  return [mesh.positions[b], mesh.positions[b + 1], mesh.positions[b + 2]];
}

/// The (un-normalized-then-normalized) face normal of triangle (a, b, c) from its winding: (b−a)×(c−a),
/// normalized. A degenerate triangle (zero-area) yields a zero vector — STL permits a zero normal (a
/// reader then recomputes from winding), so we do not force a unit here.
function faceNormal(a: Vec3, b: Vec3, c: Vec3): Vec3 {
  const ux = b[0] - a[0], uy = b[1] - a[1], uz = b[2] - a[2];
  const vx = c[0] - a[0], vy = c[1] - a[1], vz = c[2] - a[2];
  const nx = uy * vz - uz * vy;
  const ny = uz * vx - ux * vz;
  const nz = ux * vy - uy * vx;
  const len = Math.hypot(nx, ny, nz);
  if (len === 0) return [0, 0, 0];
  return [nx / len, ny / len, nz / len];
}

/// Serialize `mesh` to a binary-STL byte buffer (a `Uint8Array`), byte-compatible with the native writer.
export function meshToBinaryStl(mesh: StlMesh): Uint8Array {
  const triCount = Math.floor(mesh.indices.length / 3);
  const out = new ArrayBuffer(84 + triCount * 50);
  const view = new DataView(out);
  const bytes = new Uint8Array(out);

  // 80-byte header: the fixed tag, ASCII, zero-padded (the rest of the 80 bytes stays zero).
  for (let i = 0; i < HEADER_TAG.length; i++) bytes[i] = HEADER_TAG.charCodeAt(i);

  view.setUint32(80, triCount, true); // little-endian triangle count

  let off = 84;
  for (let t = 0; t < triCount; t++) {
    const a = vertexAt(mesh, mesh.indices[t * 3]);
    const b = vertexAt(mesh, mesh.indices[t * 3 + 1]);
    const c = vertexAt(mesh, mesh.indices[t * 3 + 2]);
    const n = faceNormal(a, b, c);
    for (const v of [n, a, b, c]) {
      view.setFloat32(off, v[0], true); off += 4;
      view.setFloat32(off, v[1], true); off += 4;
      view.setFloat32(off, v[2], true); off += 4;
    }
    view.setUint16(off, 0, true); off += 2; // attribute byte count
  }
  return bytes;
}
