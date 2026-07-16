/// Contract + driver tests for guide/src/cad (v-cad's CAD mesh module). These exercise the REAL parser +
/// manifold-3d driver over the exact `Solidr` render grammar (Rational `n/d` coords), pin the flat
/// MeshResult contract the /cad route consumes, and validate mesh well-formedness (an out-of-range index
/// would crash three.js). Run with `npm run test:unit`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { meshFromSolid, type MeshResult } from "./index.ts";

// The exact grammar a program built on implementation/cad's Solidr model renders to (via cdz run).
const CUBE = "(: (Cuber (: (tuple 2/1 2/1 2/1) Vec3r)) Solidr)";
const PLATE = "(: (Differencer (Cuber (: (tuple 50/1 30/1 5/1) Vec3r)) (Spherer 127/20)) Solidr)";

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
  const r = await meshFromSolid("(: (Spherer 127/20) Solidr)");
  assert.equal(r.ok, true); // a valid rational radius meshes fine
});

test("a malformed solid returns a typed error, never throws", async () => {
  const r = await meshFromSolid("(: (Torusr 1/1) Solidr)");
  assert.equal(r.ok, false);
  if (!r.ok) assert.match(r.error, /unknown Solidr constructor/);
});

test("a zero-denominator rational is a typed error", async () => {
  const r = await meshFromSolid("(: (Spherer 1/0) Solidr)");
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
  const r = await meshFromSolid("(: (Spherer nan) Solidr)");
  assert.equal(r.ok, true, "a `nan` radius must parse (not a typed parse error)");
});

test("an empty solid meshes to zero triangles (matches the native Manifold::empty())", async () => {
  // Emptyr → no geometry, mirroring the Rust cdz-cad driver. Pins that the browser path agrees (0 triangles),
  // not degenerate geometry from the zero-size-cube empty idiom.
  const r = await meshFromSolid("(: (Emptyr unit) Solidr)");
  assert.equal(r.ok, true);
  if (r.ok) assert.equal(r.indices.length, 0, "an empty solid has no triangles");
});
