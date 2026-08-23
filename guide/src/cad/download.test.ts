/// Unit tests for /cad's mesh-download glue (`download.ts` — `encodeMesh`). Verifies the STL/3MF byte
/// production + filename/MIME wiring over a tiny triangle mesh, so a regression in the format routing or the
/// v-cad serializer interface trips here (the download BUTTON path itself is exercised headlessly). Run with
/// `npm run test:unit`.

import test from "node:test";
import assert from "node:assert/strict";
import { encodeMesh } from "./download.ts";
import { meshFromSolid } from "./index.ts";

// A single triangle — the minimal mesh both serializers accept (positions = 3 verts × xyz, indices = 1 tri).
const TRI = {
  positions: new Float32Array([0, 0, 0, 1, 0, 0, 0, 1, 0]),
  indices: new Uint32Array([0, 1, 2]),
};

test("encodeMesh STL: bytes, MIME, filename", () => {
  const { bytes, mime, filename } = encodeMesh(TRI, "stl");
  assert.ok(bytes instanceof Uint8Array && bytes.length > 0, "produces STL bytes");
  // Binary STL: 80-byte header + 4-byte triangle count + 50 bytes/triangle = 134 for one triangle.
  assert.equal(bytes.length, 134, "binary STL length for 1 triangle (80 + 4 + 50)");
  assert.equal(mime, "model/stl");
  assert.equal(filename, "cad-model.stl");
});

test("encodeMesh 3MF: bytes, MIME, filename (default unit)", () => {
  const { bytes, mime, filename } = encodeMesh(TRI, "3mf");
  assert.ok(bytes instanceof Uint8Array && bytes.length > 0, "produces 3MF bytes");
  // 3MF is a zip container — starts with the local-file-header magic "PK\x03\x04".
  assert.equal(bytes[0], 0x50, "PK zip magic byte 0");
  assert.equal(bytes[1], 0x4b, "PK zip magic byte 1");
  assert.equal(mime, "model/3mf");
  assert.equal(filename, "cad-model.3mf");
});

test("encodeMesh 3MF: unit is threaded (a different unit changes the bytes)", () => {
  const mm = encodeMesh(TRI, "3mf", "millimeter").bytes;
  const m = encodeMesh(TRI, "3mf", "meter").bytes;
  // The unit is declared in the model XML inside the container, so the byte streams differ.
  assert.notDeepEqual([...mm], [...m], "unit label changes the serialized 3MF");
});

test("encodeMesh: STL and 3MF are distinct encodings", () => {
  const stl = encodeMesh(TRI, "stl").bytes;
  const tmf = encodeMesh(TRI, "3mf").bytes;
  assert.notEqual(stl.length, tmf.length, "STL (raw binary) and 3MF (zip) differ in size");
});

// ── increment 3: the export honors the chosen tessellation resolution (end-to-end) ────────────────────
// /cad exports the mesh AT the preview quality: the download serializes the current meshed result, which the
// Quality slider re-meshes at its segment count (CadPage keeps `lastMesh` in sync with the slider). So a finer
// resolution must produce an exported file with more triangles — "what you see is what you download". These
// pin that end-to-end (meshFromSolid(segments) → encodeMesh) so a future refactor can't silently decouple the
// exported resolution from the chosen one. The binary-STL triangle count lives at byte offset 80 (u32 LE);
// 3MF is a zip, so we compare the STL count directly.

/// The triangle count an encoded binary STL declares in its header (u32 LE at offset 80).
function stlHeaderTriCount(mesh: { positions: Float32Array; indices: Uint32Array }): number {
  const { bytes } = encodeMesh(mesh, "stl");
  return new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength).getUint32(80, true);
}

test("STL export honors the chosen resolution — a finer mesh exports more triangles", async () => {
  const coarse = await meshFromSolid("(: (Sphere 5/1) Solid)", 8);
  const fine = await meshFromSolid("(: (Sphere 5/1) Solid)", 64);
  assert.ok(coarse.ok && fine.ok, "both resolutions mesh");
  if (coarse.ok && fine.ok) {
    const cTris = stlHeaderTriCount(coarse);
    const fTris = stlHeaderTriCount(fine);
    // the STL header count matches the meshed triangle count (the export serializes exactly what was meshed)…
    assert.equal(cTris, coarse.indices.length / 3, "STL header triangle count matches the coarse mesh");
    assert.equal(fTris, fine.indices.length / 3, "STL header triangle count matches the fine mesh");
    // …and a higher resolution yields a larger exported file (the slider value flows into the download).
    assert.ok(fTris > cTris, `a finer export has more triangles in the STL (${fTris} > ${cTris})`);
  }
});
