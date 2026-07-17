/// Unit tests for the browser binary-STL writer (guide/src/cad/stl.ts). Exercise the real
/// `meshFromSolid` mesh → binary STL, and pin the byte layout so it stays compatible with the native
/// cdz-cad writer. Run with `npm run test:unit`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { meshFromSolid } from "./index.ts";
import { meshToBinaryStl } from "./stl.ts";

const CUBE = "(: (Cuber (: (tuple 2/1 2/1 2/1) Vec3r)) Solidr)";

test("a cube meshes then serializes to a well-formed binary STL (84 + 50/tri bytes, 12 triangles)", async () => {
  const r = await meshFromSolid(CUBE);
  assert.ok(r.ok);
  if (r.ok) {
    const stl = meshToBinaryStl(r);
    const triCount = r.indices.length / 3;
    assert.equal(triCount, 12, "a cube is 12 triangles");
    // 80 header + 4 count + 50 bytes/triangle.
    assert.equal(stl.byteLength, 84 + triCount * 50, "binary STL is 84 + 50/tri bytes");
    // The u32 little-endian triangle count at offset 80 matches.
    const view = new DataView(stl.buffer, stl.byteOffset, stl.byteLength);
    assert.equal(view.getUint32(80, true), triCount, "the STL triangle count field matches the mesh");
  }
});

test("the 80-byte header does NOT begin with 'solid' (else a reader parses it as ASCII STL)", async () => {
  const r = await meshFromSolid(CUBE);
  assert.ok(r.ok);
  if (r.ok) {
    const stl = meshToBinaryStl(r);
    const head = new TextDecoder().decode(stl.slice(0, 5)).toLowerCase();
    assert.notEqual(head, "solid", "binary STL header must not start with 'solid'");
  }
});

test("an empty mesh serializes to a valid header-only STL (0 triangles, 84 bytes)", () => {
  // The Emptyr / degenerate case: no triangles → just the 84-byte preamble with a zero count.
  const empty = { positions: new Float32Array(0), indices: new Uint32Array(0) };
  const stl = meshToBinaryStl(empty);
  assert.equal(stl.byteLength, 84, "empty STL is header (80) + count (4)");
  const view = new DataView(stl.buffer, stl.byteOffset, stl.byteLength);
  assert.equal(view.getUint32(80, true), 0, "zero triangles");
});

test("a face normal is unit-length for a non-degenerate triangle", async () => {
  // Pull the first triangle's stored normal out of the serialized buffer and check it is normalized
  // (magnitude 1) — a cube's faces are non-degenerate, so the winding normal is a unit vector.
  const r = await meshFromSolid(CUBE);
  assert.ok(r.ok);
  if (r.ok) {
    const stl = meshToBinaryStl(r);
    const view = new DataView(stl.buffer, stl.byteOffset, stl.byteLength);
    const nx = view.getFloat32(84, true);
    const ny = view.getFloat32(88, true);
    const nz = view.getFloat32(92, true);
    const mag = Math.hypot(nx, ny, nz);
    assert.ok(Math.abs(mag - 1) < 1e-5, `first face normal should be unit-length, got ${mag}`);
  }
});
