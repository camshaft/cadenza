/// Unit tests for scalar result formatting by static type — the fix for a whole-number Float scalar
/// losing its `.0` in the browser run path. The critical guard: ONLY Float types get the forced
/// decimal; sized ints / Qty.value / Bool (also JS `number`s) must be left untouched (the issue's
/// documented unsoundness of a value-only heuristic). Run with `npm run test:unit`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { formatScalarByType, resultTypeOf } from "./scalarFormat.ts";

test("a whole-number Float gets a forced .0", () => {
  assert.equal(formatScalarByType("5", "Float64"), "5.0");
  assert.equal(formatScalarByType("5", "Float32"), "5.0");
  assert.equal(formatScalarByType("-3", "Float64"), "-3.0");
});

test("a Float that already reads as fractional is left alone", () => {
  assert.equal(formatScalarByType("4.5", "Float64"), "4.5");
  assert.equal(formatScalarByType("1e-9", "Float64"), "1e-9");
  assert.equal(formatScalarByType("0.3000000000000004", "Float64"), "0.3000000000000004");
});

test("NON-float types are never decorated (the unsoundness guard)", () => {
  // sized ints, Qty.value, and Bool all arrive as JS `number` too — must NOT get a `.0`.
  assert.equal(formatScalarByType("2", "UInt8"), "2"); // UInt8.wrap 258 → 2
  assert.equal(formatScalarByType("3000", "Qty"), "3000"); // a Qty.value read-back
  assert.equal(formatScalarByType("8", "Int64"), "8");
  assert.equal(formatScalarByType("42", "Int32"), "42");
  assert.equal(formatScalarByType("1", "Bool"), "1");
});

test("unknown / missing type leaves the value unchanged", () => {
  assert.equal(formatScalarByType("5", null), "5");
  assert.equal(formatScalarByType("5", undefined), "5");
  assert.equal(formatScalarByType("5", ""), "5");
});

test("a padded value is trimmed before the .0 is appended (no preserved whitespace)", () => {
  assert.equal(formatScalarByType(" 5", "Float64"), "5.0");
  assert.equal(formatScalarByType("5 ", "Float64"), "5.0");
});

test("isFloatType is ANCHORED — a type merely CONTAINING Float* is not a float scalar", () => {
  // The bare export types are what fire the .0…
  assert.equal(formatScalarByType("5", " Float64 "), "5.0"); // tab-padded from the query split
  // …but a compound type string that only CONTAINS Float64 must NOT (it renders via the compound path
  // anyway, but the type check should be honest).
  assert.equal(formatScalarByType("5", "(Tuple Float64 Int64)"), "5");
  assert.equal(formatScalarByType("5", "Float64Thing"), "5"); // not the exact type
});

test("resultTypeOf prefers main, else the sole export, else null", () => {
  assert.equal(resultTypeOf("main\tFloat64\n"), "Float64");
  assert.equal(resultTypeOf("helper\tInt64\nmain\tFloat32\n"), "Float32"); // main wins
  assert.equal(resultTypeOf("only\tUInt8\n"), "UInt8"); // sole export
  assert.equal(resultTypeOf("a\tInt64\nb\tFloat64\n"), null); // ambiguous, no main
  assert.equal(resultTypeOf(""), null); // no exports
  assert.equal(resultTypeOf("\n\n"), null); // blank lines
});

test("end-to-end: a whole Float export formats to N.0, a sized-int export stays bare", () => {
  const floatExports = "main\tFloat64\n";
  assert.equal(formatScalarByType("5", resultTypeOf(floatExports)), "5.0");
  const uintExports = "main\tUInt8\n";
  assert.equal(formatScalarByType("2", resultTypeOf(uintExports)), "2");
});
