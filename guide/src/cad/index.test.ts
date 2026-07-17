/// Contract + driver tests for guide/src/cad (v-cad's CAD mesh module). These exercise the REAL parser +
/// manifold-3d driver over the exact `Solid` render grammar (Rational `n/d` coords), pin the flat
/// MeshResult contract the /cad route consumes, and validate mesh well-formedness (an out-of-range index
/// would crash three.js). Run with `npm run test:unit`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { meshFromSolid, type MeshResult } from "./index.ts";

// The exact grammar a program built on implementation/cad's Solid model renders to (via cdz run).
const CUBE = "(: (Cube (: (tuple 2/1 2/1 2/1) Vec3)) Solid)";
const PLATE = "(: (Difference (Cube (: (tuple 50/1 30/1 5/1) Vec3)) (Sphere 127/20)) Solid)";

test("meshFromSolid returns flat mesh buffers for a cube (real manifold-3d mesh)", async () => {
  const r: MeshResult = await meshFromSolid(CUBE);
  assert.equal(r.ok, true);
  if (r.ok) {
    assert.ok(r.positions instanceof Float32Array);
    assert.ok(r.indices instanceof Uint32Array);
    assert.ok(r.positions.length > 0 && r.positions.length % 3 === 0, "positions are XYZ triples");
    assert.ok(r.indices.length > 0 && r.indices.length % 3 === 0, "indices are triangles");
    assert.equal(r.indices.length / 3, 12, "a cube meshes to 12 triangles");
  }
});

test("mesh indices are all within the vertex range (no OOB → no three.js crash)", async () => {
  const r = await meshFromSolid(PLATE);
  assert.ok(r.ok);
  if (r.ok) {
    const vertexCount = r.positions.length / 3;
    for (const idx of r.indices) {
      assert.ok(idx >= 0 && idx < vertexCount, `index ${idx} out of range [0, ${vertexCount})`);
    }
  }
});

test("a difference (cube minus sphere) carves geometry — more than a bare cube", async () => {
  const r = await meshFromSolid(PLATE);
  assert.ok(r.ok);
  if (r.ok) assert.ok(r.indices.length / 3 > 12, "the difference adds triangles");
});

test("Rational n/d coordinates are read exactly (127/20 = 6.35, not a parse error)", async () => {
  const r = await meshFromSolid("(: (Sphere 127/20) Solid)");
  assert.equal(r.ok, true); // a valid rational radius meshes fine
});

test("a malformed solid returns a typed error, never throws", async () => {
  const r = await meshFromSolid("(: (Torusr 1/1) Solid)");
  assert.equal(r.ok, false);
  if (!r.ok) assert.match(r.error, /unknown Solid constructor/);
});

test("a zero-denominator rational is a typed error", async () => {
  const r = await meshFromSolid("(: (Sphere 1/0) Solid)");
  assert.equal(r.ok, false);
});

test("empty input is a typed error (not a throw)", async () => {
  const r = await meshFromSolid("");
  assert.equal(r.ok, false);
});

test("a lowercase `nan` radius parses to NaN, not a parse error (Cadenza renders NaN lowercase)", async () => {
  // Cadenza's value form spells a NaN float `nan` (lowercase); the parser must accept it (case-insensitively),
  // not throw "expected a rational" — PR#459 regression guard. A NaN dimension is pathological but must not
  // crash the parse: it flows to manifold as NaN (which produces empty/degenerate geometry, handled downstream).
  const r = await meshFromSolid("(: (Sphere nan) Solid)");
  assert.equal(r.ok, true, "a `nan` radius must parse (not a typed parse error)");
});

test("an empty solid meshes to zero triangles (matches the native Manifold::empty())", async () => {
  // Empty → no geometry, mirroring the Rust cdz-cad driver. Pins that the browser path agrees (0 triangles),
  // not degenerate geometry from the zero-size-cube empty idiom.
  const r = await meshFromSolid("(: (Empty unit) Solid)");
  assert.equal(r.ok, true);
  if (r.ok) assert.equal(r.indices.length, 0, "an empty solid has no triangles");
});

test("a negative-dimension cube meshes to zero triangles (cross-surface: matches native + exact model)", async () => {
  // Cross-surface consistency guard: the exact model (exact.cdz) normalizes a negative-dimension box, the native
  // cdz-cad driver meshes it empty, and manifold documents that any negative dimension → an empty Manifold. This
  // pins that the BROWSER driver agrees — a negative-size Cube meshes to NOTHING (never degenerate geometry that
  // would crash three.js), so all three surfaces treat a pathological negative dimension the same way.
  const r = await meshFromSolid("(: (Cube (: (tuple -2/1 2/1 2/1) Vec3)) Solid)");
  assert.equal(r.ok, true, "a negative-dimension cube must parse + mesh (not throw)");
  if (r.ok) assert.equal(r.indices.length, 0, "a negative-dimension cube has no triangles");
});
