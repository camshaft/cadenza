/// Contract tests for the CAD mesh boundary (guide/src/cad). These pin the interface the /cad route
/// consumes so v-cad's real parser/driver is a drop-in swap, and validate the stub cube's geometry is
/// well-formed (a mesh with an out-of-range index would crash three.js). Run with `npm run test:unit`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { meshFromSolid, type MeshResult } from "./index.ts";

test("meshFromSolid returns an ok result with flat mesh buffers (stub)", async () => {
  const r: MeshResult = await meshFromSolid("(: (cube) Solid)");
  assert.equal(r.ok, true);
  if (r.ok) {
    assert.ok(r.positions instanceof Float32Array);
    assert.ok(r.indices instanceof Uint32Array);
    assert.ok(r.positions.length > 0 && r.positions.length % 3 === 0, "positions are XYZ triples");
    assert.ok(r.indices.length > 0 && r.indices.length % 3 === 0, "indices are triangles");
  }
});

test("the stub cube's indices are all within the vertex range (no OOB → no three.js crash)", async () => {
  const r = await meshFromSolid("");
  assert.ok(r.ok);
  if (r.ok) {
    const vertexCount = r.positions.length / 3;
    for (const idx of r.indices) {
      assert.ok(idx >= 0 && idx < vertexCount, `index ${idx} out of range [0, ${vertexCount})`);
    }
  }
});
