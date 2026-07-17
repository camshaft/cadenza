/// Unit tests for the browser 3MF writer (guide/src/cad/threemf.ts). Exercise the real `meshFromSolid`
/// mesh → 3MF, and verify the container is a valid OPC zip with the three required parts + a unit-declared
/// model. Run with `npm run test:unit`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { unzipSync, strFromU8 } from "fflate";
import { meshFromSolid } from "./index.ts";
import { meshTo3mf } from "./threemf.ts";

const CUBE = "(: (Cuber (: (tuple 2/1 2/1 2/1) Vec3r)) Solidr)";

test("a cube meshes then serializes to a valid 3MF zip with the three required OPC parts", async () => {
  const r = await meshFromSolid(CUBE);
  assert.ok(r.ok);
  if (r.ok) {
    const bytes = meshTo3mf(r);
    // A .3mf is a zip — unzip it and check the three required package parts are present at their exact paths.
    const parts = unzipSync(bytes);
    assert.ok(parts["[Content_Types].xml"], "content types part present");
    assert.ok(parts["_rels/.rels"], "package relationships part present");
    assert.ok(parts["3D/3dmodel.model"], "the model part is at the required /3D/3dmodel.model path");
  }
});

test("the 3MF model part is well-formed XML declaring the mesh vertices + triangles", async () => {
  const r = await meshFromSolid(CUBE);
  assert.ok(r.ok);
  if (r.ok) {
    const parts = unzipSync(meshTo3mf(r));
    const model = strFromU8(parts["3D/3dmodel.model"]);
    assert.match(model, /<model/, "opens a <model> element");
    assert.match(model, /<mesh>/, "carries a <mesh>");
    assert.match(model, /<vertices>/, "carries vertices");
    assert.match(model, /<triangles>/, "carries triangles");
    // A cube is 8 vertices, 12 triangles — pin the triangle count via the number of <triangle rows.
    const triangles = (model.match(/<triangle /g) || []).length;
    assert.equal(triangles, 12, "a cube is 12 triangles in the 3MF model");
  }
});

test("the exported 3MF declares its unit (meter by default, matching the exact-meter model)", async () => {
  const r = await meshFromSolid(CUBE);
  assert.ok(r.ok);
  if (r.ok) {
    const model = strFromU8(unzipSync(meshTo3mf(r))["3D/3dmodel.model"]);
    assert.match(model, /unit="meter"/, "the model declares unit=meter by default");
  }
});

test("a caller-chosen unit (millimeter) is honored in the exported 3MF", async () => {
  const r = await meshFromSolid(CUBE);
  assert.ok(r.ok);
  if (r.ok) {
    const model = strFromU8(unzipSync(meshTo3mf(r, "millimeter"))["3D/3dmodel.model"]);
    assert.match(model, /unit="millimeter"/, "the chosen unit is declared");
  }
});
