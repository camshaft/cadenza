/// Unit tests for /cad's mesh-download glue (`download.ts` — `encodeMesh`). Verifies the STL/3MF byte
/// production + filename/MIME wiring over a tiny triangle mesh, so a regression in the format routing or the
/// v-cad serializer interface trips here (the download BUTTON path itself is exercised headlessly). Run with
/// `npm run test:unit`.

import test from "node:test";
import assert from "node:assert/strict";
import { encodeMesh } from "./download.ts";

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
