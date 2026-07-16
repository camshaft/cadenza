/// The CAD mesh boundary between the /cad route (this vertical's territory) and v-cad's Solid parser +
/// manifold-3d mesh driver. THIS FILE IS A STUB placeholder — v-cad owns the real implementation (a TS
/// port of their Rust cdz-cad: parse the Solid s-expr → build a manifold-3d solid → triangulate). The
/// route consumes only this typed interface, so v-cad's real module is a drop-in swap with NO route
/// changes.
///
/// Contract (CONFIRMED with v-cad 2026-07-16):
///   - `meshFromSolid(solidText)` takes the run worker's rendered Solid value text (`(: (…) Solid)`)
///     and returns triangle-mesh BUFFERS the route feeds to three.js — a FLAT result (positions +
///     indices at the top level, matching v-cad's Rust cdz-cad mesh output).
///   - Typed error (no throw) so the route renders a parse/mesh failure like a run trap.
///   - v-cad owns manifold-3d wasm INIT inside this module; the route lazy-loads the whole module behind
///     the /cad route, so three + manifold-wasm are code-split off the guide's critical path.

/// The result of meshing a Solid: triangle buffers ready for a three.js BufferGeometry, or a typed error
/// to render (never throws). Flat shape — matches v-cad's confirmed `meshFromSolid` return exactly.
///   - `positions`: flat vertex XYZ, 3 floats per vertex.
///   - `indices`: triangle indices into `positions` (3 per triangle).
///   - `normals`: optional flat per-vertex normals (3 per vertex); the route computes them if omitted.
export type MeshResult =
  | { ok: true; positions: Float32Array; indices: Uint32Array; normals?: Float32Array }
  | { ok: false; error: string };

/// Mesh a rendered Solid value into triangle buffers. STUB: returns a fixed unit cube so the /cad route
/// renders end-to-end before v-cad's real parser/driver lands. v-cad replaces this body (keeping the
/// signature) with: parse `solidText` → manifold-3d CSG → triangulate → buffers.
export async function meshFromSolid(_solidText: string): Promise<MeshResult> {
  return unitCube();
}

/// A unit cube centered at the origin (8 verts, 12 triangles) — the stub mesh + a useful fallback shape.
function unitCube(): MeshResult {
  // prettier-ignore
  const positions = new Float32Array([
    -1, -1, -1,   1, -1, -1,   1, 1, -1,  -1, 1, -1, // back face (z = -1)
    -1, -1,  1,   1, -1,  1,   1, 1,  1,  -1, 1,  1, // front face (z = +1)
  ]);
  // prettier-ignore
  const indices = new Uint32Array([
    0, 1, 2,  0, 2, 3, // back
    4, 6, 5,  4, 7, 6, // front
    4, 5, 1,  4, 1, 0, // bottom
    3, 2, 6,  3, 6, 7, // top
    4, 0, 3,  4, 3, 7, // left
    1, 5, 6,  1, 6, 2, // right
  ]);
  return { ok: true, positions, indices };
}
