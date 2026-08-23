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

test("Union(Empty, cube) meshes the cube — an Empty base must NOT annihilate the boolean (snowflake blank regression)", async () => {
  // REGRESSION: the empty case must be a PROPER manifold empty that composes, not a zero-size cube (which
  // annihilates booleans: cube([0,0,0]).add(realCube) === 0 tris). The snowflake folds from an Empty base
  // (Union(Empty, body)), so a bad empty zeroed the WHOLE model → blank canvas. This pins that an Empty
  // accumulator in a union contributes nothing but LEAVES the real operand intact (identity element).
  const withEmptyBase = await meshFromSolid(
    "(: (Union (Empty unit) (Cube (: (tuple 4/1 4/1 4/1) Vec3))) Solid)",
  );
  const bareCube = await meshFromSolid("(: (Cube (: (tuple 4/1 4/1 4/1) Vec3)) Solid)");
  assert.equal(withEmptyBase.ok, true);
  assert.equal(bareCube.ok, true);
  if (withEmptyBase.ok && bareCube.ok) {
    assert.ok(withEmptyBase.indices.length > 0, "Union(Empty, cube) must have geometry (Empty is union's identity, not an annihilator)");
    assert.equal(withEmptyBase.indices.length, bareCube.indices.length, "Union(Empty, cube) meshes exactly the cube");
  }
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

// ── tessellation resolution (OpenSCAD-`$fn`-style): the preview-quality segment count ────────────────
// Increment 1 of the cascading-resolution feature: meshFromSolid takes an optional `segments` count that
// threads through the whole mesh walk to every curved leaf (sphere/cylinder/circle) + the revolve/Bézier
// sweep. It's a MESH hint (the exact model is unchanged), so raising it refines the tessellation only. These
// pin: (a) more segments → more triangles on a curved primitive; (b) it CASCADES through transforms/booleans
// to nested curved leaves; (c) the count is clamped to a closable loop (≥3) and defaults to 32.

test("more segments → a finer sphere mesh (the $fn quality knob refines tessellation)", async () => {
  const coarse = await meshFromSolid("(: (Sphere 5/1) Solid)", 8);
  const fine = await meshFromSolid("(: (Sphere 5/1) Solid)", 64);
  assert.ok(coarse.ok && fine.ok);
  if (coarse.ok && fine.ok) {
    assert.ok(
      fine.indices.length > coarse.indices.length,
      `a 64-segment sphere (${fine.indices.length / 3} tris) must be finer than an 8-segment one (${coarse.indices.length / 3} tris)`,
    );
  }
});

test("resolution CASCADES through transforms + booleans to a nested curved leaf", async () => {
  // A sphere buried under a translate inside a union with a cube: the segment count must reach it (the hint
  // threads through every mesh-walk arm), so the whole model gets finer with a higher count.
  const model = "(: (Union (Cube (: (tuple 2/1 2/1 2/1) Vec3)) (Translate (: (tuple 10/1 0/1 0/1) Vec3) (Sphere 5/1))) Solid)";
  const coarse = await meshFromSolid(model, 8);
  const fine = await meshFromSolid(model, 64);
  assert.ok(coarse.ok && fine.ok);
  if (coarse.ok && fine.ok) {
    assert.ok(fine.indices.length > coarse.indices.length, "the nested sphere refines → the cascade reaches it");
  }
});

test("segment count below the minimum is clamped (does not throw or empty the mesh)", async () => {
  // A slider/argument below 3 can't close a curved loop; the driver clamps up to MIN_SEGMENTS rather than
  // erroring. 0 and 1 must both mesh to the SAME thing as the clamp floor (3), never a crash or empty.
  const zero = await meshFromSolid("(: (Sphere 5/1) Solid)", 0);
  const floor = await meshFromSolid("(: (Sphere 5/1) Solid)", 3);
  assert.ok(zero.ok && floor.ok, "a below-minimum count must still mesh");
  if (zero.ok && floor.ok) {
    assert.ok(zero.indices.length > 0, "a clamped-up count meshes real geometry, not empty");
    assert.equal(zero.indices.length, floor.indices.length, "0 segments clamps to the same mesh as the floor (3)");
  }
});

test("a fractional/NaN segment count is handled (floored / defaulted), never poisons the mesh", async () => {
  // A slider could hand a fractional value; a bad computation could hand NaN. The floor'd fractional count
  // meshes like its integer part; a NaN count falls back to the default (32) rather than producing NaN geometry.
  const frac = await meshFromSolid("(: (Sphere 5/1) Solid)", 16.9);
  const int16 = await meshFromSolid("(: (Sphere 5/1) Solid)", 16);
  const nan = await meshFromSolid("(: (Sphere 5/1) Solid)", NaN);
  const def = await meshFromSolid("(: (Sphere 5/1) Solid)");
  assert.ok(frac.ok && int16.ok && nan.ok && def.ok);
  if (frac.ok && int16.ok) assert.equal(frac.indices.length, int16.indices.length, "16.9 floors to 16");
  if (nan.ok && def.ok) assert.equal(nan.indices.length, def.indices.length, "a NaN count falls back to the default (32)");
});

// ── P-D: extrude / revolve / path profiles (the browser twin of cdz-cad's mesh.rs P-D cases) ──────────

test("meshFromSolid extrudes a Rect profile into a prism (12 triangles)", async () => {
  // (ExtrudeLinear (Rect (: (tuple 4/1 2/1) Vec2R)) 6/1) — a 4×2 rectangle lifted to height 6 → a box.
  const r = await meshFromSolid("(: (ExtrudeLinear (Rect (: (tuple 4/1 2/1) Vec2R)) 6/1) SolidR)");
  assert.equal(r.ok, true, "an extruded rect meshes");
  if (r.ok) {
    assert.ok(r.indices.length > 0, "the extruded rect has geometry");
    assert.equal(r.indices.length / 3, 12, "an extruded rectangle is a 12-triangle box");
  }
});

test("meshFromSolid extrudes a Circle profile into a cylinder (curved walls)", async () => {
  const r = await meshFromSolid("(: (ExtrudeLinear (Circle 3/1) 5/1) SolidR)");
  assert.equal(r.ok, true, "an extruded circle meshes");
  if (r.ok) assert.ok(r.indices.length / 3 > 12, "an extruded disc has curved-wall triangles");
});

test("meshFromSolid revolves a profile into a swept solid", async () => {
  // a rect revolved 360° about y, offset in x so the sweep encloses volume.
  const r = await meshFromSolid(
    "(: (Translate (: (tuple 5/1 0/1 0/1) Vec3) (Revolve (Rect (: (tuple 2/1 4/1) Vec2R)) 360/1)) SolidR)",
  );
  assert.equal(r.ok, true, "a revolved profile meshes");
  if (r.ok) assert.ok(r.indices.length > 0, "the revolved profile has geometry");
});

test("meshFromSolid extrudes a PathProfile (line + cubic-Bézier spline) into a curved part", async () => {
  // a triangular path outline (0,0)→(4,0)→(2,3) extruded — the path samples to a polygon (line = vertex).
  const tri = await meshFromSolid(
    "(: (ExtrudeLinear (PathProfile (: (list (MoveToAbs (: (tuple 0/1 0/1) Vec2R)) (LineToAbs (: (tuple 4/1 0/1) Vec2R)) (LineToAbs (: (tuple 2/1 3/1) Vec2R))) PathR)) 3/1) SolidR)",
  );
  assert.equal(tri.ok, true, "an extruded triangle path meshes");
  if (tri.ok) assert.ok(tri.indices.length > 0, "the extruded triangle has geometry");
  // a SPLINE outline (a cubic Bézier) — sampled to many polygon points → a smooth curved wall.
  const spline = await meshFromSolid(
    "(: (ExtrudeLinear (PathProfile (: (list (MoveToAbs (: (tuple 0/1 0/1) Vec2R)) (LineToAbs (: (tuple 8/1 0/1) Vec2R)) (CubicToAbs (: (tuple 0/1 0/1) Vec2R) (: (tuple 8/1 10/1) Vec2R) (: (tuple 0/1 10/1) Vec2R))) PathR)) 2/1) SolidR)",
  );
  assert.equal(spline.ok, true, "an extruded cubic-Bézier spline path meshes");
  if (spline.ok) assert.ok(spline.indices.length / 3 > 12, "a spline outline samples to a curved (many-tri) wall");
});

// ── winding consistency: an extruded solid must be all-outward-wound (no inverted faces) ──────────────
// Regression for the operator's "Extrude.Linear extrudes one side, leaves others flat": manifold-3d's
// extrude(h, …, center=true) INVERTS the winding of some faces (verified: 8 outward + 4 inward on a square
// prism), so those faces render dark/one-sided. The fix is extrude(h)+translate (consistently outward, like
// a cube). This pins that every triangle of an extruded solid winds OUTWARD (its face normal points away
// from the solid's centroid) — the tri-count test alone missed the inverted faces.

/// Count triangles whose winding-normal points outward (away from the mesh centroid) vs inward.
function windingBalance(positions: Float32Array, indices: Uint32Array): { outward: number; inward: number } {
  let cx = 0, cy = 0, cz = 0;
  const n = positions.length / 3;
  for (let i = 0; i < positions.length; i += 3) {
    cx += positions[i];
    cy += positions[i + 1];
    cz += positions[i + 2];
  }
  cx /= n;
  cy /= n;
  cz /= n;
  let outward = 0, inward = 0;
  for (let t = 0; t < indices.length; t += 3) {
    const a = indices[t] * 3, b = indices[t + 1] * 3, c = indices[t + 2] * 3;
    const ux = positions[b] - positions[a], uy = positions[b + 1] - positions[a + 1], uz = positions[b + 2] - positions[a + 2];
    const vx = positions[c] - positions[a], vy = positions[c + 1] - positions[a + 1], vz = positions[c + 2] - positions[a + 2];
    const nx = uy * vz - uz * vy, ny = uz * vx - ux * vz, nz = ux * vy - uy * vx;
    const gx = (positions[a] + positions[b] + positions[c]) / 3 - cx;
    const gy = (positions[a + 1] + positions[b + 1] + positions[c + 1]) / 3 - cy;
    const gz = (positions[a + 2] + positions[b + 2] + positions[c + 2]) / 3 - cz;
    if (nx * gx + ny * gy + nz * gz > 0) outward++;
    else inward++;
  }
  return { outward, inward };
}

test("an extruded prism is all-outward-wound (no inverted faces → no 'one side flat')", async () => {
  const r = await meshFromSolid("(: (ExtrudeLinear (Rect (: (tuple 4/1 4/1) Vec2R)) 4/1) SolidR)");
  assert.equal(r.ok, true);
  if (r.ok) {
    const { outward, inward } = windingBalance(r.positions, r.indices);
    assert.equal(inward, 0, `an extruded prism must have NO inward-wound faces (got ${inward} inward, ${outward} outward)`);
    assert.ok(outward > 0, "the prism has geometry");
  }
});

test("a cube is all-outward-wound (baseline for the winding check)", async () => {
  const r = await meshFromSolid("(: (Cube (: (tuple 4/1 4/1 4/1) Vec3)) Solid)");
  assert.equal(r.ok, true);
  if (r.ok) assert.equal(windingBalance(r.positions, r.indices).inward, 0, "a cube has no inward faces");
});

// ── Rotate / Mirror (the browser twin of cdz-cad's mesh.rs rotate/mirror cases) ──────────────────────

test("a rotated cube still meshes a watertight 12-triangle box (rotate = trig at the manifold leaf)", async () => {
  const r = await meshFromSolid(
    "(: (Rotate (: (tuple 0/1 0/1 45/1) Vec3) (Cube (: (tuple 2/1 2/1 2/1) Vec3))) Solid)",
  );
  assert.equal(r.ok, true, "a rotated cube meshes");
  if (r.ok) assert.equal(r.indices.length / 3, 12, "a rotated cube is still 12 triangles");
});

test("a mirrored cube still meshes a 12-triangle box (mirror across an axis plane)", async () => {
  const r = await meshFromSolid(
    "(: (Mirror (: (tuple 1/1 0/1 0/1) Vec3) (Translate (: (tuple 5/1 0/1 0/1) Vec3) (Cube (: (tuple 2/1 2/1 2/1) Vec3)))) Solid)",
  );
  assert.equal(r.ok, true, "a mirrored cube meshes");
  if (r.ok) assert.equal(r.indices.length / 3, 12, "a mirrored cube is still 12 triangles");
});

test("a 6-fold rotational array (Union of rotated bars) meshes more geometry than one bar", async () => {
  const bar = "(Translate (: (tuple 5/1 0/1 0/1) Vec3) (Cube (: (tuple 8/1 1/1 1/1) Vec3)))";
  const one = await meshFromSolid(`(: ${bar} Solid)`);
  const two = await meshFromSolid(
    `(: (Union ${bar} (Rotate (: (tuple 0/1 0/1 60/1) Vec3) ${bar})) Solid)`,
  );
  assert.equal(one.ok, true);
  assert.equal(two.ok, true);
  if (one.ok && two.ok) {
    assert.ok(
      two.indices.length > one.indices.length,
      "the 6-fold-style union of a bar with its 60-degree-rotated copy has more geometry than one bar",
    );
  }
});

// ── grammar completeness: parse EVERY lower-emittable constructor in one solid ────────────────────────
// Twin of v-cad's native-parser completeness guard (cdz-cad Rust, batch #103): the native driver pins that
// a sexpr touching every SolidR head + every Profile + every PathSeg parses, so a model `lower` arm added
// (or removed) without a matching parser arm fails LOUDLY instead of silently rendering nothing — the
// RENDER-BLANK class (the empty-Solid/"snowflake blank" family). This is the browser twin over index.ts's
// parser: parseNode declines an unknown/removed head with `unknown Solid constructor` (→ a typed error,
// never a throw), so if any of the 13 SolidR heads / 3 Profiles / 6 PathSegs loses its arm, this all-in-one
// solid stops meshing and the test goes red. The exact model↔driver grammar lives in
// implementation/cad/src/exact.cdz (SolidR / ProfileR / PathSegR).
const ALL_CONSTRUCTORS =
  "(: (Union" +
  " (Difference (Intersection (Cube (: (tuple 4/1 4/1 4/1) Vec3)) (Sphere 3/1)) (Cylinder 2/1 1/1))" +
  " (Union" +
  "  (Translate (: (tuple 10/1 0/1 0/1) Vec3) (Scale (: (tuple 1/1 1/1 1/1) Vec3)" +
  "   (Rotate (: (tuple 0/1 0/1 45/1) Vec3) (Mirror (: (tuple 1/1 0/1 0/1) Vec3) (Cube (: (tuple 2/1 2/1 2/1) Vec3))))))" +
  "  (Union (ExtrudeLinear (Rect (: (tuple 4/1 2/1) Vec2R)) 6/1)" +
  "   (Union (ExtrudeLinear (Circle 3/1) 5/1)" +
  "    (Union (Revolve (Rect (: (tuple 2/1 4/1) Vec2R)) 360/1)" +
  "     (Union" +
  "      (ExtrudeLinear (PathProfile (: (list" +
  "       (MoveToAbs (: (tuple 0/1 0/1) Vec2R)) (LineToAbs (: (tuple 4/1 0/1) Vec2R))" +
  "       (CubicToAbs (: (tuple 4/1 4/1) Vec2R) (: (tuple 5/1 1/1) Vec2R) (: (tuple 5/1 3/1) Vec2R))" +
  "       (MoveToRel (: (tuple 1/1 1/1) Vec2R)) (LineToRel (: (tuple 2/1 0/1) Vec2R))" +
  "       (CubicToRel (: (tuple 0/1 2/1) Vec2R) (: (tuple 1/1 0/1) Vec2R) (: (tuple 1/1 1/1) Vec2R)))" +
  "       PathR)) 3/1)" +
  "      (Empty unit)))))))" +
  " Solid)";

test("the parser handles EVERY render-grammar constructor (a removed head → render-blank regression)", async () => {
  // Every SolidR head (13: Empty, Cube, Sphere, Cylinder, Union, Difference, Intersection, Translate,
  // Scale, Rotate, Mirror, ExtrudeLinear, Revolve), every Profile (Rect/Circle/PathProfile via Extrude+
  // Revolve), and every PathSeg (MoveToAbs/Rel, LineToAbs/Rel, CubicToAbs/Rel) appear in this one solid.
  const r = await meshFromSolid(ALL_CONSTRUCTORS);
  assert.equal(
    r.ok,
    true,
    r.ok ? "" : `an all-constructors solid must parse + mesh, got error: ${r.error}`,
  );
  if (r.ok) {
    assert.ok(r.indices.length > 0 && r.indices.length % 3 === 0, "the all-constructors solid meshes to triangles");
  }
});
